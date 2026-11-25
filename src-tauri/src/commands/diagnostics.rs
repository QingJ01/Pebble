use regex::Regex;
use serde::Serialize;
use std::fs;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use tauri::{AppHandle, Manager};

/// Log files are rotated daily as `pebble.YYYY-MM-DD.log`.
pub const LOG_FILE_PREFIX: &str = "pebble";
pub const LOG_FILE_SUFFIX: &str = "log";
/// Number of daily log files kept on disk.
pub const LOG_MAX_FILES: usize = 7;
/// Pre-rotation log file name, still read if no rotated file exists yet.
const LEGACY_LOG_FILE_NAME: &str = "pebble.log";
const DEFAULT_LOG_MAX_BYTES: u64 = 64 * 1024;
const MAX_LOG_MAX_BYTES: u64 = 1024 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AppLogSnapshot {
    pub path: String,
    pub content: String,
    pub truncated: bool,
}

pub fn app_log_dir(app_data_dir: &Path) -> PathBuf {
    app_data_dir.join("logs")
}

/// The most recent log file: the newest rotated `pebble.<date>.log`, or the
/// legacy `pebble.log` when no rotated file exists.
fn current_log_path(log_dir: &Path) -> PathBuf {
    let prefix = format!("{LOG_FILE_PREFIX}.");
    let suffix = format!(".{LOG_FILE_SUFFIX}");
    let newest_rotated = fs::read_dir(log_dir).ok().and_then(|entries| {
        entries
            .filter_map(|entry| entry.ok())
            .filter_map(|entry| entry.file_name().into_string().ok())
            .filter(|name| {
                name != LEGACY_LOG_FILE_NAME && name.starts_with(&prefix) && name.ends_with(&suffix)
            })
            // Date-stamped names sort chronologically.
            .max()
    });
    log_dir.join(newest_rotated.unwrap_or_else(|| LEGACY_LOG_FILE_NAME.to_string()))
}

fn email_pattern() -> &'static Regex {
    static EMAIL: OnceLock<Regex> = OnceLock::new();
    EMAIL.get_or_init(|| {
        Regex::new(r"[A-Za-z0-9._%+\-]+@[A-Za-z0-9\-]+(?:\.[A-Za-z0-9\-]+)+")
            .expect("email pattern is valid")
    })
}

/// Remove personal data from log text before it leaves the app: email
/// addresses and the user's home directory path.
fn redact_log_text(text: &str, home: Option<&Path>) -> String {
    let mut redacted = text.to_string();
    if let Some(home) = home {
        let home = home.to_string_lossy();
        let home = home.trim_end_matches(['/', '\\']);
        if home.len() > 1 {
            redacted = redacted.replace(home, "~");
            let alternate = if home.contains('\\') {
                home.replace('\\', "/")
            } else {
                home.replace('/', "\\")
            };
            redacted = redacted.replace(&alternate, "~");
        }
    }
    email_pattern()
        .replace_all(&redacted, "<email>")
        .into_owned()
}

fn read_log_tail(path: &Path, max_bytes: u64) -> Result<AppLogSnapshot, String> {
    let path_display = path.display().to_string();
    let Ok(metadata) = fs::metadata(path) else {
        return Ok(AppLogSnapshot {
            path: path_display,
            content: String::new(),
            truncated: false,
        });
    };

    let file_len = metadata.len();
    let truncated = file_len > max_bytes;
    let start = if truncated { file_len - max_bytes } else { 0 };
    let mut file = fs::File::open(path).map_err(|e| format!("Failed to open app log: {e}"))?;
    file.seek(SeekFrom::Start(start))
        .map_err(|e| format!("Failed to seek app log: {e}"))?;

    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)
        .map_err(|e| format!("Failed to read app log: {e}"))?;

    Ok(AppLogSnapshot {
        path: path_display,
        content: String::from_utf8_lossy(&bytes).into_owned(),
        truncated,
    })
}

fn redact_snapshot(snapshot: AppLogSnapshot, home: Option<&Path>) -> AppLogSnapshot {
    AppLogSnapshot {
        path: redact_log_text(&snapshot.path, home),
        content: redact_log_text(&snapshot.content, home),
        truncated: snapshot.truncated,
    }
}

#[tauri::command]
pub fn read_app_log(app: AppHandle, max_bytes: Option<u64>) -> Result<AppLogSnapshot, String> {
    let app_data_dir = crate::profile::require_managed_app_data_dir(&app)?;
    let max_bytes = max_bytes
        .unwrap_or(DEFAULT_LOG_MAX_BYTES)
        .clamp(1, MAX_LOG_MAX_BYTES);
    let snapshot = read_log_tail(&current_log_path(&app_log_dir(&app_data_dir)), max_bytes)?;
    let home = app.path().home_dir().ok();
    Ok(redact_snapshot(snapshot, home.as_deref()))
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::PathBuf;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_log_path() -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("time should be available")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "pebble-diagnostics-test-{}-{nonce}.log",
            std::process::id()
        ))
    }

    #[test]
    fn read_log_tail_returns_only_recent_bytes_when_file_is_large() {
        let path = temp_log_path();
        fs::write(&path, "alpha\nbeta\ngamma\n").expect("test log should be writable");

        let snapshot = super::read_log_tail(&path, 11).expect("tail should be readable");

        assert_eq!(snapshot.content, "beta\ngamma\n");
        assert_eq!(snapshot.path, path.display().to_string());
        assert!(snapshot.truncated);

        let _ = fs::remove_file(path);
    }

    #[test]
    fn redaction_removes_emails_and_home_dir() {
        let home = std::path::Path::new("/Users/alice");
        let text = "sync alice.smith+tag@example.co.uk at /Users/alice/Library/x and C:\\Users\\bob ok bob@host.io";
        let redacted = super::redact_log_text(text, Some(home));

        assert!(!redacted.contains("alice.smith"));
        assert!(!redacted.contains("example.co.uk"));
        assert!(!redacted.contains("bob@host.io"));
        assert!(!redacted.contains("/Users/alice"));
        assert!(redacted.contains("~/Library/x"));
        assert_eq!(redacted.matches("<email>").count(), 2);
    }

    #[test]
    fn current_log_path_prefers_newest_rotated_file() {
        let dir = temp_log_path();
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(super::current_log_path(&dir), dir.join("pebble.log"));

        fs::write(dir.join("pebble.log"), "legacy").unwrap();
        fs::write(dir.join("pebble.2026-11-24.log"), "old").unwrap();
        fs::write(dir.join("pebble.2026-11-25.log"), "new").unwrap();
        fs::write(dir.join("other.2027-01-01.log"), "x").unwrap();

        assert_eq!(
            super::current_log_path(&dir),
            dir.join("pebble.2026-11-25.log")
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn read_log_tail_returns_empty_snapshot_when_file_is_missing() {
        let path = temp_log_path();

        let snapshot = super::read_log_tail(&path, 128).expect("missing log should not error");

        assert_eq!(snapshot.content, "");
        assert_eq!(snapshot.path, path.display().to_string());
        assert!(!snapshot.truncated);
    }
}
