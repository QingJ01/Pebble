use crate::state::AppState;
use pebble_core::{FolderRole, Message, PebbleError, ProviderType};
use tauri::State;
use tracing::{info, warn};

use super::provider_dispatch::{
    archives_without_folder, delete_op, folder_change, move_is_local_only,
    remote_ids_are_folder_scoped,
};
use super::{
    apply_remote_or_queue, find_folder_by_role, find_message_folder, refresh_search_document,
    remove_search_documents, RemoteMutationOutcome, RemoteSession,
};
use pebble_store::pending_ops::MailOp;

/// Load a message and its account's provider type, surfacing a clear error
/// if either is missing. Four lifecycle commands share this preamble.
fn resolve_message_context(
    state: &AppState,
    message_id: &str,
) -> std::result::Result<(Message, ProviderType), PebbleError> {
    let msg = state
        .store
        .get_message(message_id)?
        .ok_or_else(|| PebbleError::Internal(format!("Message not found: {message_id}")))?;
    let provider_type = state
        .store
        .get_account(&msg.account_id)?
        .map(|account| account.provider)
        .ok_or_else(|| PebbleError::Internal(format!("Account not found: {}", msg.account_id)))?;
    Ok((msg, provider_type))
}

fn finalize_permanent_local_delete(
    state: &AppState,
    message_id: &str,
) -> std::result::Result<(), PebbleError> {
    let ids = [message_id.to_string()];
    state.store.hard_delete_messages(&ids)?;
    remove_search_documents(state, &ids)
}

fn is_local_folder(folder: &pebble_core::Folder) -> bool {
    folder.remote_id.starts_with("__local_")
}

/// Returns "archived" or "unarchived" so the frontend can show the correct toast.
#[tauri::command]
pub async fn archive_message(
    state: State<'_, AppState>,
    message_id: String,
) -> std::result::Result<String, PebbleError> {
    let (msg, provider_type) = resolve_message_context(&state, &message_id)?;

    let source_folder = find_message_folder(&state, &message_id, &msg.account_id)?;
    if source_folder.role == Some(FolderRole::Archive) {
        info!(
            "Message {} already in archive, restoring to inbox",
            message_id
        );
        let inbox = find_folder_by_role(&state, &msg.account_id, FolderRole::Inbox)?;
        let local_only = is_local_folder(&source_folder) || is_local_folder(&inbox);
        let op = MailOp::Unarchive(folder_change(Some(&source_folder), Some(&inbox)));
        apply_remote_or_queue(&state, &msg, &provider_type, op, local_only).await?;

        state.store.move_message_to_folder(&message_id, &inbox.id)?;
        refresh_search_document(&state, &message_id)?;
        return Ok("unarchived".to_string());
    }

    // Try to find Archive folder; if not available, just soft-delete locally
    match find_folder_by_role(&state, &msg.account_id, FolderRole::Archive) {
        Ok(archive_folder) => {
            let op = MailOp::Archive(folder_change(Some(&source_folder), Some(&archive_folder)));
            let local_only = is_local_folder(&archive_folder);
            apply_remote_or_queue(&state, &msg, &provider_type, op, local_only).await?;

            state
                .store
                .move_message_to_folder(&message_id, &archive_folder.id)?;
            refresh_search_document(&state, &message_id)?;
            Ok("archived".to_string())
        }
        Err(_) => {
            if archives_without_folder(&provider_type) {
                let op = MailOp::Archive(folder_change(Some(&source_folder), None));
                apply_remote_or_queue(&state, &msg, &provider_type, op, false).await?;
            }

            info!(
                "No archive folder found, soft-deleting message {} locally",
                message_id
            );
            state.store.soft_delete_message(&message_id)?;
            refresh_search_document(&state, &message_id)?;
            Ok("archived".to_string())
        }
    }
}

#[tauri::command]
pub async fn delete_message(
    state: State<'_, AppState>,
    message_id: String,
) -> std::result::Result<(), PebbleError> {
    let (msg, provider_type) = resolve_message_context(&state, &message_id)?;

    if crate::commands::drafts::delete_local_only_draft(&state, &msg)? {
        return Ok(());
    }

    let source_folder = find_message_folder(&state, &message_id, &msg.account_id)?;
    let trash_folder = find_folder_by_role(&state, &msg.account_id, FolderRole::Trash).ok();
    let is_permanent = source_folder.role == Some(FolderRole::Trash);

    // A folder-scoped id cannot be addressed from a device-only folder, and
    // a device-only trash keeps the deleted copy locally.
    let local_only = remote_ids_are_folder_scoped(&provider_type)
        && (is_local_folder(&source_folder)
            || (!is_permanent && trash_folder.as_ref().is_some_and(is_local_folder)));
    let op = delete_op(Some(&source_folder), trash_folder.as_ref(), is_permanent);
    apply_remote_or_queue(&state, &msg, &provider_type, op, local_only).await?;

    if is_permanent {
        state
            .store
            .hard_delete_messages(std::slice::from_ref(&message_id))?;
        remove_search_documents(&state, std::slice::from_ref(&message_id))?;
    } else if let Some(trash_folder) = trash_folder {
        if trash_folder.id != source_folder.id {
            state
                .store
                .move_message_to_folder(&message_id, &trash_folder.id)?;
            refresh_search_document(&state, &message_id)?;
        } else {
            state.store.soft_delete_message(&message_id)?;
            refresh_search_document(&state, &message_id)?;
        }
    } else {
        state.store.soft_delete_message(&message_id)?;
        refresh_search_document(&state, &message_id)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn restore_message(
    state: State<'_, AppState>,
    message_id: String,
) -> std::result::Result<(), PebbleError> {
    let (msg, provider_type) = resolve_message_context(&state, &message_id)?;

    let inbox = find_folder_by_role(&state, &msg.account_id, FolderRole::Inbox)?;

    let source_folder = find_message_folder(&state, &message_id, &msg.account_id).ok();

    let local_only = remote_ids_are_folder_scoped(&provider_type)
        && source_folder.as_ref().is_none_or(|src| {
            src.id == inbox.id || is_local_folder(src) || is_local_folder(&inbox)
        });
    let op = MailOp::Restore(folder_change(source_folder.as_ref(), Some(&inbox)));
    apply_remote_or_queue(&state, &msg, &provider_type, op, local_only).await?;

    state.store.move_message_to_folder(&message_id, &inbox.id)?;
    refresh_search_document(&state, &message_id)?;
    info!("Restored message {} to inbox", message_id);
    Ok(())
}

#[tauri::command]
pub async fn move_to_folder(
    state: State<'_, AppState>,
    message_id: String,
    target_folder_id: String,
) -> std::result::Result<(), PebbleError> {
    let (msg, provider_type) = resolve_message_context(&state, &message_id)?;

    let source_folder = find_message_folder(&state, &message_id, &msg.account_id)?;
    if source_folder.id == target_folder_id {
        return Ok(());
    }

    // Look up target folder to get its remote_id
    let target_folders = state.store.list_folders(&msg.account_id)?;
    let target_folder = target_folders
        .iter()
        .find(|f| f.id == target_folder_id)
        .ok_or_else(|| {
            PebbleError::Internal(format!("Target folder not found: {target_folder_id}"))
        })?;

    let local_only = move_is_local_only(&provider_type, &source_folder, target_folder);
    let op = MailOp::MoveToFolder(folder_change(Some(&source_folder), Some(target_folder)));
    apply_remote_or_queue(&state, &msg, &provider_type, op, local_only).await?;

    state
        .store
        .move_message_to_folder(&message_id, &target_folder_id)?;
    refresh_search_document(&state, &message_id)?;
    info!(
        "Moved message {} to folder {} ({})",
        message_id, target_folder.name, target_folder_id
    );
    Ok(())
}

#[tauri::command]
pub async fn empty_trash(
    state: State<'_, AppState>,
    account_id: String,
) -> std::result::Result<u32, PebbleError> {
    let trash = find_folder_by_role(&state, &account_id, FolderRole::Trash)?;
    let provider_type = state
        .store
        .get_account(&account_id)?
        .map(|account| account.provider)
        .ok_or_else(|| PebbleError::Internal(format!("Account not found: {account_id}")))?;

    let mut total_deleted: u32 = 0;
    const PAGE_SIZE: u32 = 500;

    // Materialize a stable command-start snapshot before the first remote
    // mutation. Deleting acknowledged rows below therefore cannot shift the
    // pagination window and skip later messages.
    let messages = collect_paginated(PAGE_SIZE, |limit, offset| {
        state
            .store
            .list_messages_by_folder(&trash.id, limit, offset)
    })?;

    if is_local_folder(&trash) {
        let ids: Vec<String> = messages.iter().map(|message| message.id.clone()).collect();
        if !ids.is_empty() {
            state.store.hard_delete_messages(&ids)?;
            remove_search_documents(&state, &ids)?;
            total_deleted = ids.len() as u32;
        }
    } else {
        let session = RemoteSession::open(&state, &account_id, &provider_type).await;
        let result = async {
            for summary in &messages {
                let Some(msg) = state.store.get_message(&summary.id)? else {
                    continue;
                };
                let msg = &msg;
                let op = delete_op(Some(&trash), None, true);
                let applied = session.apply(&state, msg, op).await?;
                match applied.outcome {
                    RemoteMutationOutcome::Applied | RemoteMutationOutcome::LocalOnly => {
                        finalize_permanent_local_delete(&state, &msg.id)?;
                        total_deleted += 1;
                    }
                    RemoteMutationOutcome::Queued | RemoteMutationOutcome::QueuedLocalCommit => {
                        // Offline: every message is queued. A failing server
                        // stops the pass at the first failure.
                        if session.is_connected() {
                            warn!(
                                "Permanent delete failed for {}: {}",
                                msg.id,
                                applied.error.as_deref().unwrap_or_default()
                            );
                            break;
                        }
                    }
                }
            }
            Ok::<(), PebbleError>(())
        }
        .await;
        session.close().await;
        result?;
    }

    info!(
        "Emptied trash: {} messages permanently deleted",
        total_deleted
    );
    Ok(total_deleted)
}

fn collect_paginated<T>(
    page_size: u32,
    mut load_page: impl FnMut(u32, u32) -> std::result::Result<Vec<T>, PebbleError>,
) -> std::result::Result<Vec<T>, PebbleError> {
    debug_assert!(page_size > 0);
    let mut items = Vec::new();
    let mut offset = 0_u32;
    loop {
        let page = load_page(page_size, offset)?;
        let page_len = page.len() as u32;
        items.extend(page);
        if page_len < page_size {
            break;
        }
        offset = offset.saturating_add(page_len);
    }
    Ok(items)
}

#[cfg(test)]
mod remote_mutation_tests {
    use super::*;
    use crate::commands::messages::{
        remote_delete_is_already_absent, remote_mutation_allows_local_commit,
    };

    #[test]
    fn trash_snapshot_visits_every_page_without_reloading_failed_items() {
        let source = ["m1", "m2", "m3", "m4", "m5"];
        let mut offsets = Vec::new();

        let snapshot = collect_paginated(2, |limit, offset| {
            offsets.push(offset);
            Ok(source
                .iter()
                .skip(offset as usize)
                .take(limit as usize)
                .copied()
                .collect::<Vec<_>>())
        })
        .unwrap();

        assert_eq!(snapshot, source);
        assert_eq!(offsets, vec![0, 2, 4]);
    }

    #[test]
    fn permanent_delete_treats_remote_absence_as_idempotent_success() {
        assert!(remote_delete_is_already_absent(&PebbleError::Network(
            "Failed to permanently delete message (status 404 Not Found)".to_string()
        )));
        assert!(remote_delete_is_already_absent(&PebbleError::Sync(
            "NO no such message".to_string()
        )));
        assert!(!remote_delete_is_already_absent(&PebbleError::Network(
            "status 503 Service Unavailable".to_string()
        )));
    }

    #[test]
    fn remote_mutation_allows_local_commit_after_remote_ack_local_only_or_offline_queue() {
        assert!(remote_mutation_allows_local_commit(
            RemoteMutationOutcome::Applied
        ));
        assert!(remote_mutation_allows_local_commit(
            RemoteMutationOutcome::LocalOnly
        ));
        assert!(remote_mutation_allows_local_commit(
            RemoteMutationOutcome::QueuedLocalCommit
        ));
        assert!(!remote_mutation_allows_local_commit(
            RemoteMutationOutcome::Queued
        ));
    }
}
