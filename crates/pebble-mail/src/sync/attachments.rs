use super::*;

/// Sanitize a filename to prevent path traversal attacks.
/// Removes path separators, `..` sequences, and trims leading dots/spaces.
pub(crate) fn sanitize_filename(name: &str) -> String {
    fn is_windows_reserved(stem: &str) -> bool {
        let upper = stem.trim().to_ascii_uppercase();
        matches!(
            upper.as_str(),
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

    // Take only the last component if there are path separators
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
    // Reject if the component is exactly ".."
    if base == ".." || base == "." {
        return "unnamed_attachment".to_string();
    }
    // Remove `..` sequences repeatedly until none remain, then strip unsafe chars.
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

    // Windows disallows names ending with dots/spaces and hidden/path-like prefixes are unsafe.
    let trimmed = sanitized
        .trim()
        .trim_matches(|c: char| c == '.' || c == ' ');
    if trimmed.is_empty() {
        return "unnamed_attachment".to_string();
    }

    let stem = trimmed.split('.').next().unwrap_or(trimmed);
    if is_windows_reserved(stem) {
        return "unnamed_attachment".to_string();
    }

    trimmed.to_string()
}

/// Write attachments to disk and record them in the store.
///
/// Takes ownership of `attachments` so that each buffer can be freed the
/// moment it has been flushed — we don't keep every attachment's bytes
/// live in memory until the whole function returns. Writes use a buffered
/// writer with 64 KiB chunks so the working set stays bounded.
pub(super) fn write_attachment_bytes<W: std::io::Write>(
    writer: &mut W,
    data: &[u8],
    file_path: &Path,
) -> Result<()> {
    const CHUNK_SIZE: usize = 64 * 1024;

    for chunk in data.chunks(CHUNK_SIZE) {
        writer.write_all(chunk).map_err(|e| {
            PebbleError::Storage(format!(
                "Failed to write attachment file {}: {e}",
                file_path.display()
            ))
        })?;
    }
    writer.flush().map_err(|e| {
        PebbleError::Storage(format!(
            "Failed to flush attachment file {}: {e}",
            file_path.display()
        ))
    })
}

#[cfg(test)]
pub(crate) fn persist_message_attachments(
    store: &Store,
    attachments_root: &Path,
    message_id: &str,
    attachments: Vec<AttachmentData>,
) -> Result<()> {
    let staged = stage_message_attachments(attachments_root, message_id, attachments)?;
    for attachment in &staged {
        if let Err(e) = store.insert_attachment(attachment) {
            cleanup_staged_attachment_files(&staged);
            return Err(e);
        }
    }

    Ok(())
}

pub(crate) fn stage_message_attachments(
    attachments_root: &Path,
    message_id: &str,
    attachments: Vec<AttachmentData>,
) -> Result<Vec<pebble_core::Attachment>> {
    const CHUNK_SIZE: usize = 64 * 1024;

    let mut written_paths = Vec::new();
    let mut staged = Vec::new();
    for att_data in attachments.into_iter() {
        let att_dir = attachments_root.join(message_id);
        if let Err(e) = std::fs::create_dir_all(&att_dir) {
            cleanup_attachment_files(&written_paths);
            return Err(PebbleError::Storage(format!(
                "Failed to create attachment directory {}: {e}",
                att_dir.display()
            )));
        }

        let safe_filename = sanitize_filename(&att_data.meta.filename);
        let mut file_path = att_dir.join(&safe_filename);
        let mut counter = 1u32;
        while file_path.exists() {
            let stem = std::path::Path::new(&safe_filename)
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy();
            let ext = std::path::Path::new(&safe_filename)
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            file_path = att_dir.join(format!("{stem}_{counter}{ext}"));
            counter += 1;
        }
        let file = match std::fs::File::create(&file_path) {
            Ok(file) => file,
            Err(e) => {
                cleanup_attachment_files(&written_paths);
                return Err(PebbleError::Storage(format!(
                    "Failed to create attachment file {}: {e}",
                    file_path.display()
                )));
            }
        };
        let mut writer = std::io::BufWriter::with_capacity(CHUNK_SIZE, file);

        let AttachmentData { meta, data } = att_data;
        let write_result = write_attachment_bytes(&mut writer, &data, &file_path);
        // Release the attachment buffer as soon as bytes are flushed to the
        // buffered writer, before we touch the store — callers often invoke us
        // in a tight loop where peak memory matters.
        drop(data);

        if let Err(e) = write_result {
            drop(writer);
            let _ = std::fs::remove_file(&file_path);
            cleanup_attachment_files(&written_paths);
            return Err(e);
        }
        drop(writer);

        let attachment = pebble_core::Attachment {
            id: new_id(),
            message_id: message_id.to_string(),
            filename: meta.filename,
            mime_type: meta.mime_type,
            size: meta.size as i64,
            local_path: Some(file_path.to_string_lossy().to_string()),
            content_id: meta.content_id,
            is_inline: meta.is_inline,
        };
        written_paths.push(file_path);
        staged.push(attachment);
    }

    Ok(staged)
}

pub(super) fn cleanup_attachment_files(paths: &[PathBuf]) {
    for path in paths {
        let _ = std::fs::remove_file(path);
    }
}

pub(crate) fn cleanup_staged_attachment_files(attachments: &[pebble_core::Attachment]) {
    for attachment in attachments {
        if let Some(path) = attachment.local_path.as_deref() {
            let _ = std::fs::remove_file(path);
        }
    }
}

#[cfg(test)]
pub(super) async fn finish_attachment_task(
    task: tokio::task::JoinHandle<Result<()>>,
) -> Result<()> {
    task.await
        .map_err(|e| PebbleError::Internal(format!("Attachment persistence task failed: {e}")))?
}

/// Async wrapper that offloads attachment I/O to a blocking thread.
pub(crate) async fn stage_message_attachments_async(
    attachments_root: PathBuf,
    message_id: String,
    attachments: Vec<AttachmentData>,
) -> Result<Vec<pebble_core::Attachment>> {
    if attachments.is_empty() {
        return Ok(Vec::new());
    }
    let task = tokio::task::spawn_blocking(move || {
        stage_message_attachments(&attachments_root, &message_id, attachments)
    });
    task.await
        .map_err(|e| PebbleError::Internal(format!("Attachment staging task failed: {e}")))?
}

pub(crate) async fn store_new_message_with_attachments_atomically(
    store: Arc<Store>,
    attachments_root: PathBuf,
    message: Message,
    folder_ids: Vec<String>,
    attachment_data: Vec<AttachmentData>,
) -> Result<()> {
    let staged =
        stage_message_attachments_async(attachments_root, message.id.clone(), attachment_data)
            .await?;
    let staged_for_store = staged.clone();
    let result = blocking_store(&store, move |store| {
        store.replace_message_with_attachments(&message, &folder_ids, &staged_for_store)
    })
    .await;
    if let Err(error) = result {
        cleanup_staged_attachment_files(&staged);
        return Err(error);
    }
    Ok(())
}

pub(crate) fn remote_message_requires_fetch(
    already_exists: bool,
    has_incomplete_attachments: bool,
) -> bool {
    !already_exists || has_incomplete_attachments
}

pub(crate) fn preserve_local_state_for_attachment_repair(
    store: &Store,
    message: &mut Message,
    folder_ids: &mut Vec<String>,
) -> Result<()> {
    let existing = store.get_message(&message.id)?.ok_or_else(|| {
        PebbleError::Internal(format!(
            "Incomplete attachment message disappeared before repair: {}",
            message.id
        ))
    })?;
    message.is_read = existing.is_read;
    message.is_starred = existing.is_starred;
    message.created_at = existing.created_at;
    for folder_id in store.get_message_folder_ids(&message.id)? {
        if !folder_ids.contains(&folder_id) {
            folder_ids.push(folder_id);
        }
    }
    Ok(())
}
