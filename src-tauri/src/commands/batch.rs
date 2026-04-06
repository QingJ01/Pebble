use crate::state::AppState;
use pebble_core::PebbleError;
use tauri::State;
use tracing::{info, warn};

#[tauri::command]
pub async fn batch_archive(
    state: State<'_, AppState>,
    message_ids: Vec<String>,
) -> std::result::Result<u32, PebbleError> {
    let mut success_count: u32 = 0;

    for message_id in &message_ids {
        match archive_single(&state, message_id) {
            Ok(()) => success_count += 1,
            Err(e) => warn!("Failed to archive message {}: {}", message_id, e),
        }
    }

    info!(
        "Batch archive: {}/{} messages archived",
        success_count,
        message_ids.len()
    );
    Ok(success_count)
}

#[tauri::command]
pub async fn batch_delete(
    state: State<'_, AppState>,
    message_ids: Vec<String>,
) -> std::result::Result<u32, PebbleError> {
    let mut success_count: u32 = 0;

    for message_id in &message_ids {
        match state.store.soft_delete_message(message_id) {
            Ok(()) => success_count += 1,
            Err(e) => warn!("Failed to delete message {}: {}", message_id, e),
        }
    }

    info!(
        "Batch delete: {}/{} messages deleted",
        success_count,
        message_ids.len()
    );
    Ok(success_count)
}

#[tauri::command]
pub async fn batch_mark_read(
    state: State<'_, AppState>,
    message_ids: Vec<String>,
    is_read: bool,
) -> std::result::Result<u32, PebbleError> {
    let mut success_count: u32 = 0;

    for message_id in &message_ids {
        match state
            .store
            .update_message_flags(message_id, Some(is_read), None)
        {
            Ok(()) => success_count += 1,
            Err(e) => warn!("Failed to mark message {} read={}: {}", message_id, is_read, e),
        }
    }

    info!(
        "Batch mark_read({}): {}/{} messages updated",
        is_read,
        success_count,
        message_ids.len()
    );
    Ok(success_count)
}

/// Archive a single message locally (move to archive folder or soft-delete).
fn archive_single(state: &AppState, message_id: &str) -> std::result::Result<(), PebbleError> {
    let msg = state
        .store
        .get_message(message_id)?
        .ok_or_else(|| PebbleError::Internal(format!("Message not found: {message_id}")))?;

    let folders = state.store.list_folders(&msg.account_id)?;
    let archive_folder = folders
        .iter()
        .find(|f| f.role == Some(pebble_core::FolderRole::Archive));

    match archive_folder {
        Some(folder) => {
            state
                .store
                .move_message_to_folder(message_id, &folder.id)?;
        }
        None => {
            // No archive folder, fall back to soft-delete
            state.store.soft_delete_message(message_id)?;
        }
    }

    Ok(())
}
