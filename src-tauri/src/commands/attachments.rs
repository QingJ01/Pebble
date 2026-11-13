use crate::state::AppState;
use pebble_core::{Attachment, PebbleError};
use std::path::{Path, PathBuf};
use tauri::{Emitter, State};

#[path = "attachment_preview.rs"]
mod attachment_preview;
use attachment_preview::{open_attachment_source, AttachmentPreview};

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

    Ok(())
}

fn copy_attachment_file_safely<F>(
    source: &Path,
    save_path: &Path,
    on_progress: F,
) -> Result<PathBuf, PebbleError>
where
    F: FnMut(u64, u64),
{
    let src_file = std::fs::File::open(source)
        .map_err(|e| PebbleError::Internal(format!("Failed to open source: {e}")))?;
    copy_open_attachment_safely(src_file, save_path, on_progress)
}

fn copy_open_attachment_safely<F>(
    src_file: std::fs::File,
    save_path: &Path,
    on_progress: F,
) -> Result<PathBuf, PebbleError>
where
    F: FnMut(u64, u64),
{
    let total_bytes = src_file
        .metadata()
        .map_err(|e| PebbleError::Internal(format!("Failed to read file metadata: {e}")))?
        .len();

    copy_reader_to_target(src_file, total_bytes, save_path, on_progress)
}

fn copy_reader_to_target<R, F>(
    source: R,
    total_bytes: u64,
    save_path: &Path,
    mut on_progress: F,
) -> Result<PathBuf, PebbleError>
where
    R: std::io::Read,
    F: FnMut(u64, u64),
{
    use std::io::{Read, Write};
    let mut src_file = source.take(total_bytes);

    let (actual_save_path, mut dst_file) = create_unique_target(save_path)?;

    let mut buf = [0u8; 8192];
    let mut bytes_copied: u64 = 0;
    let copy_result: std::result::Result<(), PebbleError> = (|| {
        on_progress(0, total_bytes);
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
        if bytes_copied != total_bytes {
            return Err(PebbleError::Validation(
                "Attachment changed while downloading. Retry the download.".to_string(),
            ));
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
    for source in source_paths {
        let source_path = Path::new(source);
        let source_metadata = match std::fs::metadata(source_path) {
            Ok(metadata) => metadata,
            Err(error) => {
                cleanup_staged_attachment_records(&records);
                return Err(PebbleError::Internal(format!(
                    "Attachment source file not available: {source} ({error})"
                )));
            }
        };
        if !source_metadata.is_file() {
            cleanup_staged_attachment_records(&records);
            return Err(PebbleError::Validation(format!(
                "Attachment source is not a file: {source}"
            )));
        }

        let filename = source_path
            .file_name()
            .and_then(|value| value.to_str())
            .map(sanitize_stored_filename)
            .unwrap_or_else(|| "attachment".to_string());
        let attachment_dir = message_dir.join(pebble_core::new_id());
        if let Err(error) = std::fs::create_dir_all(&attachment_dir) {
            cleanup_staged_attachment_records(&records);
            return Err(PebbleError::Internal(format!(
                "Failed to create local attachment directory {}: {error}",
                attachment_dir.display()
            )));
        }
        let target = attachment_dir.join(&filename);
        let staged_path =
            match copy_attachment_file_safely(source_path, &target, |_copied, _total| {}) {
                Ok(path) => path,
                Err(error) => {
                    let _ = std::fs::remove_dir_all(&attachment_dir);
                    cleanup_staged_attachment_records(&records);
                    return Err(error);
                }
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

pub(crate) fn cleanup_staged_attachment_records(records: &[Attachment]) {
    for record in records {
        let Some(path) = record.local_path.as_deref().map(Path::new) else {
            continue;
        };
        if let Err(error) = std::fs::remove_file(path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(
                    "Failed to remove staged attachment {}: {error}",
                    path.display()
                );
            }
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
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

    let source = open_attachment_source(&state.attachments_dir, &att)?;

    let att_id = attachment_id.clone();
    // Use spawn_blocking to avoid blocking the async executor on large files
    let actual_path_result = tokio::task::spawn_blocking(move || {
        let save_path = std::path::Path::new(&save_to);
        copy_open_attachment_safely(source, save_path, |bytes_copied, total_bytes| {
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

#[tauri::command]
pub async fn preview_attachment(
    state: State<'_, AppState>,
    attachment_id: String,
) -> Result<AttachmentPreview, PebbleError> {
    let attachment = state
        .store
        .get_attachment(&attachment_id)?
        .ok_or_else(|| PebbleError::Validation("Attachment not found".to_string()))?;
    attachment_preview::preview(state.attachments_dir.clone(), attachment).await
}

#[derive(serde::Serialize, Debug)]
pub struct AttachmentSaveResult {
    pub attachment_id: String,
    pub filename: String,
    pub saved_path: Option<String>,
    pub error: Option<String>,
}

fn save_attachments_to_directory<F>(
    root: &Path,
    directory: &Path,
    attachments: Vec<Attachment>,
    mut on_progress: F,
) -> Result<Vec<AttachmentSaveResult>, PebbleError>
where
    F: FnMut(&str, u64, u64),
{
    let directory = directory.canonicalize().map_err(|_| {
        PebbleError::Validation("Save directory is unavailable. Choose another folder.".to_string())
    })?;
    if !directory.is_dir() || attachments.len() > 1000 {
        return Err(PebbleError::Validation(
            "Invalid save directory or too many attachments (maximum 1000)".to_string(),
        ));
    }
    let mut results = Vec::new();
    for attachment in attachments.into_iter().filter(|a| !a.is_inline) {
        let filename = sanitize_stored_filename(&attachment.filename);
        // Leave room for a duplicate suffix on filesystems with a 255-byte name limit.
        let mut end = filename.len().min(200);
        while !filename.is_char_boundary(end) {
            end -= 1;
        }
        let filename = filename[..end].trim_end_matches(['.', ' ']);
        let target = directory.join(filename);
        let result = (|| {
            validate_save_path(&target.to_string_lossy())?;
            let source = open_attachment_source(root, &attachment)?;
            copy_open_attachment_safely(source, &target, |copied, total| {
                on_progress(&attachment.id, copied, total)
            })
        })();
        let (saved_path, error) = match result {
            Ok(path) => (Some(path.to_string_lossy().into_owned()), None),
            Err(error) => (None, Some(error.to_string())),
        };
        results.push(AttachmentSaveResult {
            attachment_id: attachment.id,
            filename: attachment.filename,
            saved_path,
            error,
        });
    }
    Ok(results)
}

#[tauri::command]
pub async fn save_all_attachments(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    message_id: String,
) -> Result<Option<Vec<AttachmentSaveResult>>, PebbleError> {
    use tauri_plugin_dialog::DialogExt;

    let attachments = state.store.list_attachments_by_message(&message_id)?;
    let root = state.attachments_dir.clone();
    tokio::task::spawn_blocking(move || {
        let Some(directory) = app.dialog().file().blocking_pick_folder() else {
            return Ok(None);
        };
        let directory = directory
            .into_path()
            .map_err(|_| PebbleError::Validation("Choose a local save directory".to_string()))?;
        let results =
            save_attachments_to_directory(&root, &directory, attachments, |id, copied, total| {
                let _ = app.emit(
                    "attachment:download-progress",
                    serde_json::json!({
                        "attachment_id": id, "bytes_copied": copied, "total_bytes": total,
                    }),
                );
            })?;
        Ok(Some(results))
    })
    .await
    .map_err(|_| {
        PebbleError::Internal(
            "Save All could not finish. Check the destination folder and retry.".to_string(),
        )
    })?
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(root: &Path, filename: &str, contents: Option<&[u8]>, inline: bool) -> Attachment {
        let id = pebble_core::new_id();
        let path = root.join(&id);
        if let Some(contents) = contents {
            std::fs::write(&path, contents).unwrap();
        }
        Attachment {
            id,
            message_id: "message".into(),
            filename: filename.into(),
            mime_type: "application/octet-stream".into(),
            size: 0,
            local_path: Some(path.to_string_lossy().into_owned()),
            content_id: None,
            is_inline: inline,
        }
    }

    #[test]
    fn save_all_handles_duplicates_existing_files_bad_names_and_partial_failures() {
        let base = std::env::temp_dir().join(format!("pebble-save-all-{}", pebble_core::new_id()));
        let root = base.join("attachments");
        let output = base.join("output");
        std::fs::create_dir_all(&root).unwrap();
        std::fs::create_dir(&output).unwrap();
        std::fs::write(output.join("report.txt"), b"existing").unwrap();
        let attachments = vec![
            record(&root, "report.txt", Some(b"first"), false),
            record(&root, "report.txt", Some(b"second"), false),
            record(&root, "missing.txt", None, false),
            record(&root, "../..\\NUL.txt", Some(b"reserved"), false),
            record(&root, "inline.png", Some(b"inline"), true),
            record(&root, &"测".repeat(120), Some(b"long name"), false),
        ];
        let mut progress = Vec::new();
        let result =
            save_attachments_to_directory(&root, &output, attachments, |id, copied, total| {
                progress.push((id.to_owned(), copied, total))
            })
            .unwrap();
        assert_eq!(result.len(), 5, "ordinary attachments only");
        assert_eq!(result.iter().filter(|r| r.saved_path.is_some()).count(), 4);
        assert!(result[2].error.is_some());
        assert!(result[2].saved_path.is_none());
        assert_eq!(
            std::fs::read(output.join("report.txt")).unwrap(),
            b"existing"
        );
        assert_eq!(
            std::fs::read(output.join("report (1).txt")).unwrap(),
            b"first"
        );
        assert_eq!(
            std::fs::read(output.join("report (2).txt")).unwrap(),
            b"second"
        );
        assert_eq!(
            std::fs::read(output.join("attachment")).unwrap(),
            b"reserved"
        );
        assert!(!output.join("missing.txt").exists());
        assert!(!output.join("inline.png").exists());
        assert_eq!(std::fs::read_dir(&output).unwrap().count(), 5);
        assert!(progress
            .iter()
            .any(|(_, copied, total)| copied == total && *total > 0));
        assert!(save_attachments_to_directory(
            &root,
            &output.join("absent"),
            Vec::new(),
            |_, _, _| {}
        )
        .is_err());
        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn failed_or_truncated_copy_removes_only_its_new_target() {
        struct BrokenRead(bool);
        impl std::io::Read for BrokenRead {
            fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
                if self.0 {
                    return Err(std::io::Error::other("simulated read failure"));
                }
                self.0 = true;
                buffer[0] = 1;
                Ok(1)
            }
        }
        let base =
            std::env::temp_dir().join(format!("pebble-copy-failure-{}", pebble_core::new_id()));
        std::fs::create_dir(&base).unwrap();
        let target = base.join("keep.txt");
        std::fs::write(&target, b"existing").unwrap();
        assert!(copy_reader_to_target(BrokenRead(false), 10, &target, |_, _| {}).is_err());
        assert!(
            copy_reader_to_target(std::io::Cursor::new(b"short"), 10, &target, |_, _| {}).is_err()
        );
        assert_eq!(std::fs::read_dir(&base).unwrap().count(), 1);
        assert_eq!(std::fs::read(target).unwrap(), b"existing");
        let _ = std::fs::remove_dir_all(base);
    }

    #[cfg(unix)]
    #[test]
    fn target_symlink_is_never_followed_or_overwritten() {
        let base = std::env::temp_dir().join(format!("pebble-copy-link-{}", pebble_core::new_id()));
        std::fs::create_dir(&base).unwrap();
        let original = base.join("original.txt");
        let target = base.join("link.txt");
        std::fs::write(&original, b"keep").unwrap();
        std::os::unix::fs::symlink(&original, &target).unwrap();
        let actual =
            copy_reader_to_target(std::io::Cursor::new(b"new"), 3, &target, |_, _| {}).unwrap();
        assert_eq!(std::fs::read(&original).unwrap(), b"keep");
        assert_eq!(actual.file_name().unwrap(), "link (1).txt");
        assert!(std::fs::symlink_metadata(target)
            .unwrap()
            .file_type()
            .is_symlink());
        let _ = std::fs::remove_dir_all(base);
    }

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

    #[test]
    fn stage_local_attachment_records_keeps_duplicate_filenames_distinct() {
        let unique = pebble_core::new_id();
        let base = std::env::temp_dir().join(format!("pebble-attachment-duplicates-{unique}"));
        let first_dir = base.join("first");
        let second_dir = base.join("second");
        let attachments_root = base.join("attachments");
        std::fs::create_dir_all(&first_dir).unwrap();
        std::fs::create_dir_all(&second_dir).unwrap();
        let first = first_dir.join("report.txt");
        let second = second_dir.join("report.txt");
        std::fs::write(&first, b"first").unwrap();
        std::fs::write(&second, b"second").unwrap();

        let records = stage_local_attachment_records(
            &attachments_root,
            "message-1",
            &[
                first.to_string_lossy().to_string(),
                second.to_string_lossy().to_string(),
            ],
        )
        .unwrap();

        assert_eq!(records.len(), 2);
        assert_eq!(records[0].filename, "report.txt");
        assert_eq!(records[1].filename, "report.txt");
        assert_ne!(records[0].local_path, records[1].local_path);
        assert_eq!(
            std::fs::read(records[0].local_path.as_ref().unwrap()).unwrap(),
            b"first"
        );
        assert_eq!(
            std::fs::read(records[1].local_path.as_ref().unwrap()).unwrap(),
            b"second"
        );

        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn stage_local_attachment_records_cleans_partial_copy_on_error() {
        let unique = pebble_core::new_id();
        let base = std::env::temp_dir().join(format!("pebble-attachment-rollback-{unique}"));
        let source_dir = base.join("source");
        let attachments_root = base.join("attachments");
        std::fs::create_dir_all(&source_dir).unwrap();
        let valid = source_dir.join("valid.txt");
        std::fs::write(&valid, b"payload").unwrap();
        let missing = source_dir.join("missing.txt");

        let result = stage_local_attachment_records(
            &attachments_root,
            "message-1",
            &[
                valid.to_string_lossy().to_string(),
                missing.to_string_lossy().to_string(),
            ],
        );

        assert!(result.is_err());
        let message_dir = attachments_root.join("message-1");
        let remaining_files = std::fs::read_dir(&message_dir)
            .unwrap()
            .flat_map(|entry| std::fs::read_dir(entry.unwrap().path()).unwrap())
            .count();
        assert_eq!(remaining_files, 0);

        let _ = std::fs::remove_dir_all(base);
    }
}
