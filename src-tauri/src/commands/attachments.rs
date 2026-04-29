use crate::state::AppState;
use pebble_core::{Attachment, PebbleError};
use std::path::{Path, PathBuf};
use tauri::{Emitter, State};

fn is_windows_reserved_name(name: &str) -> bool {
    matches!(
        name.trim().to_ascii_uppercase().as_str(),
        "CON"
            | "PRN"
            | "AUX"
            | "NUL"
            | "COM1"
            | "COM2"
            | "COM3"
            | "COM4"
            | "COM5"
            | "COM6"
            | "COM7"
            | "COM8"
            | "COM9"
            | "LPT1"
            | "LPT2"
            | "LPT3"
            | "LPT4"
            | "LPT5"
            | "LPT6"
            | "LPT7"
            | "LPT8"
            | "LPT9"
    )
}

/// Validate that save_to path is within a safe directory (user's home).
///
/// This is the security gate for attachment writes: it *rejects* suspicious
/// paths rather than normalizing them. The FE helper at `src/lib/sanitizeFilename.ts`
/// covers the complementary UX role of cleaning up suggested defaults; it is
/// not a substitute for this check. Keep the character sets below in sync with
/// the FE when they change.
fn validate_save_path(save_to: &str) -> Result<(), PebbleError> {
    let path = Path::new(save_to);
    let canonical = path
        .parent()
        .and_then(|p| p.canonicalize().ok())
        .ok_or_else(|| PebbleError::Internal("Invalid save directory".to_string()))?;

    // Ensure no path traversal components in the filename
    let filename = path
        .file_name()
        .ok_or_else(|| PebbleError::Internal("No filename specified".to_string()))?;
    let filename_str = filename.to_string_lossy();
    if filename_str.contains("..") || filename_str.contains('/') || filename_str.contains('\\') {
        return Err(PebbleError::Internal(
            "Invalid filename in save path".to_string(),
        ));
    }
    if filename_str.ends_with(' ') || filename_str.ends_with('.') {
        return Err(PebbleError::Validation(
            "Filename cannot end with a dot or space".to_string(),
        ));
    }
    if filename_str
        .chars()
        .any(|c| matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*'))
    {
        return Err(PebbleError::Validation(
            "Filename contains characters unsupported on Windows".to_string(),
        ));
    }
    if filename_str.chars().any(|c| (c as u32) < 0x20) {
        return Err(PebbleError::Validation(
            "Filename contains control characters".to_string(),
        ));
    }
    let stem = Path::new(filename)
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or_default();
    if is_windows_reserved_name(stem) {
        return Err(PebbleError::Validation(
            "Filename is reserved on Windows".to_string(),
        ));
    }

    // Ensure parent directory actually exists and is absolute
    if !canonical.is_absolute() {
        return Err(PebbleError::Internal(
            "Save path must be absolute".to_string(),
        ));
    }

    // Ensure target is within user home directory to prevent writes to system paths
    let home = home_dir()
        .ok_or_else(|| PebbleError::Internal("Cannot determine user home directory".to_string()))?;
    if !canonical.starts_with(&home) {
        return Err(PebbleError::Validation(
            "Save path must be within user home directory".to_string(),
        ));
    }

    Ok(())
}

/// Get the user's home directory.
fn home_dir() -> Option<std::path::PathBuf> {
    #[cfg(target_os = "windows")]
    {
        std::env::var("USERPROFILE")
            .ok()
            .map(std::path::PathBuf::from)
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::env::var("HOME").ok().map(std::path::PathBuf::from)
    }
}

fn copy_attachment_file_safely<F>(
    source: &Path,
    save_path: &Path,
    mut on_progress: F,
) -> Result<PathBuf, PebbleError>
where
    F: FnMut(u64, u64),
{
    use std::io::{Read, Write};

    let mut src_file = std::fs::File::open(source)
        .map_err(|e| PebbleError::Internal(format!("Failed to open source: {e}")))?;
    let total_bytes = src_file
        .metadata()
        .map_err(|e| PebbleError::Internal(format!("Failed to read file metadata: {e}")))?
        .len();

    let (actual_save_path, mut dst_file) = create_unique_target(save_path)?;

    let mut buf = [0u8; 8192];
    let mut bytes_copied: u64 = 0;
    let copy_result: std::result::Result<(), PebbleError> = (|| {
        loop {
            let n = src_file
                .read(&mut buf)
                .map_err(|e| PebbleError::Internal(format!("Read error: {e}")))?;
            if n == 0 {
                break;
            }
            dst_file
                .write_all(&buf[..n])
                .map_err(|e| PebbleError::Internal(format!("Write error: {e}")))?;
            bytes_copied += n as u64;
            on_progress(bytes_copied, total_bytes);
        }
        dst_file
            .sync_all()
            .map_err(|e| PebbleError::Internal(format!("Failed to flush file: {e}")))?;
        Ok(())
    })();

    if let Err(e) = copy_result {
        drop(dst_file);
        let _ = std::fs::remove_file(&actual_save_path);
        return Err(e);
    }

    Ok(actual_save_path)
}

fn create_unique_target(save_path: &Path) -> Result<(PathBuf, std::fs::File), PebbleError> {
    const MAX_UNIQUE_ATTEMPTS: u32 = 1000;

    for attempt in 0..MAX_UNIQUE_ATTEMPTS {
        let candidate = unique_save_path(save_path, attempt);
        // create_new refuses to follow or replace an existing target, including
        // a symlink planted after path validation and before the file is opened.
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&candidate)
        {
            Ok(file) => return Ok((candidate, file)),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                return Err(PebbleError::Internal(format!(
                    "Failed to create target file: {e}"
                )))
            }
        }
    }

    Err(PebbleError::Validation(
        "Could not choose an unused filename for attachment download".to_string(),
    ))
}

fn unique_save_path(save_path: &Path, attempt: u32) -> PathBuf {
    if attempt == 0 {
        return save_path.to_path_buf();
    }

    let parent = save_path.parent().unwrap_or_else(|| Path::new(""));
    let stem = save_path
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or("download");
    let extension = save_path
        .extension()
        .and_then(|value| value.to_str())
        .map(|value| format!(".{value}"))
        .unwrap_or_default();

    parent.join(format!("{stem} ({attempt}){extension}"))
}

pub(crate) fn sanitize_stored_filename(name: &str) -> String {
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    if base == "." || base == ".." {
        return "attachment".to_string();
    }

    let mut cleaned = base.to_string();
    while cleaned.contains("..") {
        cleaned = cleaned.replace("..", ".");
    }

    let sanitized: String = cleaned
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
            _ => c,
        })
        .filter(|c| !c.is_control())
        .collect();
    let trimmed = sanitized
        .trim()
        .trim_matches(|c: char| c == '.' || c == ' ');

    if trimmed.is_empty() {
        return "attachment".to_string();
    }

    let stem = Path::new(trimmed)
        .file_stem()
        .and_then(|value| value.to_str())
        .unwrap_or_default();
    if is_windows_reserved_name(stem) {
        return "attachment".to_string();
    }

    trimmed.to_string()
}

pub(crate) fn stage_local_attachment_records(
    attachments_root: &Path,
    message_id: &str,
    source_paths: &[String],
) -> Result<Vec<Attachment>, PebbleError> {
    if source_paths.is_empty() {
        return Ok(Vec::new());
    }

    let message_dir = attachments_root.join(message_id);
    std::fs::create_dir_all(&message_dir).map_err(|e| {
        PebbleError::Internal(format!(
            "Failed to create local attachment directory {}: {e}",
            message_dir.display()
        ))
    })?;

    let mut records = Vec::with_capacity(source_paths.len());
    let canonical_message_dir = message_dir.canonicalize().map_err(|e| {
        PebbleError::Internal(format!(
            "Failed to resolve local attachment directory {}: {e}",
            message_dir.display()
        ))
    })?;
    for source in source_paths {
        let source_path = Path::new(source);
        let source_metadata = std::fs::metadata(source_path).map_err(|e| {
            PebbleError::Internal(format!(
                "Attachment source file not available: {source} ({e})"
            ))
        })?;
        if !source_metadata.is_file() {
            return Err(PebbleError::Validation(format!(
                "Attachment source is not a file: {source}"
            )));
        }

        let filename = source_path
            .file_name()
            .and_then(|value| value.to_str())
            .map(sanitize_stored_filename)
            .unwrap_or_else(|| "attachment".to_string());
        let canonical_source = source_path.canonicalize().map_err(|e| {
            PebbleError::Internal(format!("Failed to resolve attachment source {source}: {e}"))
        })?;
        let staged_path = if canonical_source.starts_with(&canonical_message_dir) {
            canonical_source
        } else {
            let target = message_dir.join(&filename);
            copy_attachment_file_safely(source_path, &target, |_copied, _total| {})?
        };
        let size = std::fs::metadata(&staged_path)
            .map(|metadata| metadata.len().min(i64::MAX as u64) as i64)
            .unwrap_or(0);

        records.push(Attachment {
            id: pebble_core::new_id(),
            message_id: message_id.to_string(),
            filename,
            mime_type: "application/octet-stream".to_string(),
            size,
            local_path: Some(staged_path.to_string_lossy().to_string()),
            content_id: None,
            is_inline: false,
        });
    }

    Ok(records)
}

#[tauri::command]
pub async fn list_attachments(
    state: State<'_, AppState>,
    message_id: String,
) -> std::result::Result<Vec<Attachment>, PebbleError> {
    state.store.list_attachments_by_message(&message_id)
}

#[tauri::command]
pub async fn get_attachment_path(
    state: State<'_, AppState>,
    attachment_id: String,
) -> std::result::Result<Option<String>, PebbleError> {
    let att = state.store.get_attachment(&attachment_id)?;
    Ok(att.and_then(|a| a.local_path))
}

#[tauri::command]
pub async fn download_attachment(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    attachment_id: String,
    save_to: String,
) -> std::result::Result<String, PebbleError> {
    let att = state
        .store
        .get_attachment(&attachment_id)?
        .ok_or_else(|| PebbleError::Internal("Attachment not found".to_string()))?;
    // Validate save path to prevent path traversal
    validate_save_path(&save_to)?;

    let source = att
        .local_path
        .ok_or_else(|| PebbleError::Internal("Attachment file not available".to_string()))?;

    let att_id = attachment_id.clone();
    // Use spawn_blocking to avoid blocking the async executor on large files
    let actual_path_result = tokio::task::spawn_blocking(move || {
        let source_path = std::path::Path::new(&source);
        let save_path = std::path::Path::new(&save_to);
        copy_attachment_file_safely(source_path, save_path, |bytes_copied, total_bytes| {
            let _ = app.emit(
                "attachment:download-progress",
                serde_json::json!({
                    "attachment_id": att_id,
                    "bytes_copied": bytes_copied,
                    "total_bytes": total_bytes,
                }),
            );
        })
    })
    .await
    .map_err(|e| PebbleError::Internal(format!("Copy task failed: {e}")))?;

    let actual_path = match actual_path_result {
        Ok(path) => path,
        Err(e) => {
            tracing::warn!(
                attachment_id = %attachment_id,
                filename = %att.filename,
                error = %e,
                "Attachment download failed"
            );
            return Err(e);
        }
    };

    tracing::info!(
        attachment_id = %attachment_id,
        filename = %att.filename,
        save_path = %actual_path.display(),
        "Attachment downloaded"
    );

    Ok(actual_path.to_string_lossy().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_attachment_file_safely_uses_unique_target_when_requested_name_exists() {
        let unique = pebble_core::new_id();
        let base = std::env::temp_dir().join(format!("pebble-attachment-copy-{unique}"));
        std::fs::create_dir_all(&base).expect("test dir");
        let source = base.join("source.txt");
        let target = base.join("target.txt");
        std::fs::write(&source, b"new content").expect("source write");
        std::fs::write(&target, b"existing content").expect("target write");

        let actual = copy_attachment_file_safely(&source, &target, |_copied, _total| {})
            .expect("existing targets should get a unique filename");

        assert_eq!(
            std::fs::read(&target).expect("target read"),
            b"existing content"
        );
        assert_ne!(actual, target);
        assert_eq!(
            actual.file_name().and_then(|name| name.to_str()),
            Some("target (1).txt")
        );
        assert_eq!(
            std::fs::read(actual).expect("unique target read"),
            b"new content"
        );

        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn stage_local_attachment_records_copies_sources_into_message_attachment_dir() {
        let unique = pebble_core::new_id();
        let base = std::env::temp_dir().join(format!("pebble-attachment-stage-{unique}"));
        let source_dir = base.join("source");
        let attachments_root = base.join("attachments");
        std::fs::create_dir_all(&source_dir).expect("source dir");
        let source = source_dir.join("report.txt");
        std::fs::write(&source, b"payload").expect("source write");

        let records = stage_local_attachment_records(
            &attachments_root,
            "message-1",
            &[source.to_string_lossy().to_string()],
        )
        .expect("local attachment should be staged");

        assert_eq!(records.len(), 1);
        let staged_path = records[0]
            .local_path
            .as_ref()
            .map(PathBuf::from)
            .expect("record should point at staged file");
        assert_ne!(staged_path, source);
        assert!(staged_path.starts_with(attachments_root.join("message-1")));
        assert_eq!(
            std::fs::read(&staged_path).expect("staged read"),
            b"payload"
        );

        std::fs::remove_file(&source).expect("remove original");
        let target = base.join("downloaded.txt");
        copy_attachment_file_safely(&staged_path, &target, |_copied, _total| {})
            .expect("download should use staged copy, not the original file");
        assert_eq!(std::fs::read(target).expect("downloaded read"), b"payload");

        let _ = std::fs::remove_dir_all(base);
    }
}
