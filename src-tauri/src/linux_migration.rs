//! One-time fixes for the Linux rename: the package and its launcher file are
//! now `pebble-mail` / `Pebble Mail.desktop` (see tauri.linux.conf.json).

use std::path::{Path, PathBuf};
use std::process::Command;

const MAILTO: &str = "x-scheme-handler/mailto";
const OLD_DESKTOP: &str = "Pebble.desktop";
const NEW_DESKTOP: &str = "Pebble Mail.desktop";

/// The first mailto handler in a mimeapps.list's `[Default Applications]` section.
fn mailto_default(mimeapps: &str) -> Option<&str> {
    let mut in_defaults = false;
    for line in mimeapps.lines().map(str::trim) {
        if line.starts_with('[') {
            in_defaults = line == "[Default Applications]";
            continue;
        }
        let value = line
            .strip_prefix(MAILTO)
            .and_then(|rest| rest.trim_start().strip_prefix('='));
        if let (true, Some(value)) = (in_defaults, value) {
            return value.split(';').map(str::trim).find(|v| !v.is_empty());
        }
    }
    None
}

fn user_mimeapps_list() -> Option<PathBuf> {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|dir| !dir.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| Path::new(&home).join(".config")))?;
    Some(config.join("mimeapps.list"))
}

/// Point a mailto default that still names the old launcher file at the new
/// one. Reads mimeapps.list directly: `xdg-mime query` skips an entry whose
/// file no longer exists (GNOME reports a fallback app instead).
// shortcut: only the user's mimeapps.list, not desktop-specific ones
// (gnome-mimeapps.list…); extend if someone's default lives there.
pub fn migrate_mailto_default() {
    if let Some(path) = user_mimeapps_list() {
        migrate_mailto_default_in(&path, || Command::new("xdg-mime"));
    }
}

fn migrate_mailto_default_in(mimeapps: &Path, xdg_mime: impl FnOnce() -> Command) -> bool {
    let Ok(contents) = std::fs::read_to_string(mimeapps) else {
        return false;
    };
    if mailto_default(&contents) != Some(OLD_DESKTOP) {
        return false;
    }
    match xdg_mime().args(["default", NEW_DESKTOP, MAILTO]).status() {
        Ok(status) if status.success() => {
            tracing::info!("Moved the mailto default from {OLD_DESKTOP} to {NEW_DESKTOP}");
            true
        }
        Ok(status) => {
            tracing::warn!("xdg-mime exited with {status} while moving the mailto default");
            false
        }
        Err(e) => {
            tracing::warn!("Could not run xdg-mime to move the mailto default: {e}");
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_the_mailto_default_only_from_default_applications() {
        let list = "[Added Associations]\nx-scheme-handler/mailto=Other.desktop;\n\n\
                    [Default Applications]\ntext/html=firefox.desktop\n\
                    x-scheme-handler/mailto = Pebble.desktop;thunderbird.desktop;\n";
        assert_eq!(mailto_default(list), Some("Pebble.desktop"));
        assert_eq!(
            mailto_default("[Added Associations]\nx-scheme-handler/mailto=Pebble.desktop\n"),
            None
        );
        assert_eq!(
            mailto_default("[Default Applications]\nx-scheme-handler/mailtox=Pebble.desktop\n"),
            None
        );
    }

    /// Runs the real xdg-mime against a scratch config directory; skipped
    /// where xdg-utils isn't installed.
    #[test]
    fn moves_only_an_old_pebble_default() {
        if Command::new("xdg-mime").arg("--version").output().is_err() {
            return;
        }
        let dir = std::env::temp_dir().join(format!("pebble-mimeapps-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let list = dir.join("mimeapps.list");
        let xdg_mime = || {
            let mut command = Command::new("xdg-mime");
            command.env("XDG_CONFIG_HOME", &dir);
            command
        };

        std::fs::write(
            &list,
            "[Default Applications]\nx-scheme-handler/mailto=thunderbird.desktop\n",
        )
        .unwrap();
        assert!(!migrate_mailto_default_in(&list, xdg_mime));
        assert_eq!(
            mailto_default(&std::fs::read_to_string(&list).unwrap()),
            Some("thunderbird.desktop")
        );

        std::fs::write(
            &list,
            "[Default Applications]\nx-scheme-handler/mailto=Pebble.desktop\n",
        )
        .unwrap();
        assert!(migrate_mailto_default_in(&list, xdg_mime));
        assert_eq!(
            mailto_default(&std::fs::read_to_string(&list).unwrap()),
            Some(NEW_DESKTOP)
        );

        std::fs::remove_dir_all(&dir).unwrap();
    }
}
