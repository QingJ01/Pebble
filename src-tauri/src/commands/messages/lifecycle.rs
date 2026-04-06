use crate::state::AppState;
use pebble_core::traits::LabelProvider;
use pebble_core::{FolderRole, PebbleError, ProviderType};
use tauri::State;
use tracing::{info, warn};

use super::{
    connect_gmail, connect_imap, find_folder_by_role, find_message_folder,
    refresh_search_document, remove_search_documents,
};

/// Returns "archived" or "unarchived" so the frontend can show the correct toast.
#[tauri::command]
pub async fn archive_message(
    state: State<'_, AppState>,
    message_id: String,
) -> std::result::Result<String, PebbleError> {
    let msg = state.store.get_message(&message_id)?
        .ok_or_else(|| PebbleError::Internal(format!("Message not found: {message_id}")))?;
    let provider_type = state
        .store
        .get_account(&msg.account_id)?
        .map(|account| account.provider)
        .ok_or_else(|| PebbleError::Internal(format!("Account not found: {}", msg.account_id)))?;

    let source_folder = find_message_folder(&state, &message_id, &msg.account_id)?;
    // If the message is already in an archive folder, unarchive it (move to inbox)
    if source_folder.role == Some(FolderRole::Archive) {
        info!("Message {} already in archive, restoring to inbox", message_id);
        let inbox = find_folder_by_role(&state, &msg.account_id, FolderRole::Inbox)?;
        if matches!(provider_type, ProviderType::Gmail) {
            if let Ok(provider) = connect_gmail(&state, &msg.account_id).await {
                if let Err(e) = provider.modify_labels(&msg.remote_id, &["INBOX".to_string()], &[]).await {
                    warn!("Gmail unarchive failed: {e}");
                }
            }
        }
        state.store.move_message_to_folder(&message_id, &inbox.id)?;
        refresh_search_document(&state, &message_id)?;
        return Ok("unarchived".to_string());
    }

    // Try to find Archive folder; if not available, just soft-delete locally
    match find_folder_by_role(&state, &msg.account_id, FolderRole::Archive) {
        Ok(archive_folder) => {
            let is_local = archive_folder.remote_id.starts_with("__local_");
            match provider_type {
                ProviderType::Gmail => {
                    if let Ok(provider) = connect_gmail(&state, &msg.account_id).await {
                        if let Err(e) = provider.modify_labels(&msg.remote_id, &[], &["INBOX".to_string()]).await {
                            warn!("Gmail archive failed: {e}");
                        }
                    }
                }
                ProviderType::Imap | ProviderType::Outlook => {
                    // Move on IMAP server (only if archive folder exists on server)
                    if !is_local {
                        let uid: u32 = msg.remote_id.parse()
                            .map_err(|_| PebbleError::Internal("Invalid remote_id (not a UID)".to_string()))?;
                        match connect_imap(&state, &msg.account_id).await {
                            Ok(imap) => {
                                let result = imap.move_message(&source_folder.remote_id, uid, &archive_folder.remote_id).await;
                                let _ = imap.disconnect().await;
                                if let Err(e) = result {
                                    tracing::warn!("IMAP move to archive failed: {e}");
                                } else {
                                    info!("Archived message {} (UID {}) from {} to {}", message_id, uid, source_folder.name, archive_folder.name);
                                }
                            }
                            Err(e) => {
                                tracing::warn!("IMAP connect failed for archive: {e}");
                            }
                        }
                    }
                }
            }
            // Move locally to archive folder so user can see it there
            state.store.move_message_to_folder(&message_id, &archive_folder.id)?;
            refresh_search_document(&state, &message_id)?;
            Ok("archived".to_string())
        }
        Err(_) => {
            // No archive folder — soft-delete as fallback
            info!("No archive folder found, soft-deleting message {} locally", message_id);
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
    let msg = state.store.get_message(&message_id)?
        .ok_or_else(|| PebbleError::Internal(format!("Message not found: {message_id}")))?;
    let provider_type = state
        .store
        .get_account(&msg.account_id)?
        .map(|account| account.provider)
        .ok_or_else(|| PebbleError::Internal(format!("Account not found: {}", msg.account_id)))?;

    let source_folder = find_message_folder(&state, &message_id, &msg.account_id)?;

    match provider_type {
        ProviderType::Gmail => {
            if source_folder.role == Some(FolderRole::Trash) {
                if let Ok(provider) = connect_gmail(&state, &msg.account_id).await {
                    if let Err(e) = provider.delete_message_permanently(&msg.remote_id).await {
                        warn!("Gmail permanent delete failed: {e}");
                    }
                }
            } else if let Ok(provider) = connect_gmail(&state, &msg.account_id).await {
                if let Err(e) = provider.trash_message(&msg.remote_id).await {
                    warn!("Gmail trash move failed: {e}");
                } else {
                    info!("Moved Gmail message {} to Trash on server", message_id);
                }
            }
        }
        ProviderType::Imap | ProviderType::Outlook => {
            // Try IMAP operations but don't block local deletion on failure
            if let Ok(uid) = msg.remote_id.parse::<u32>() {
                match connect_imap(&state, &msg.account_id).await {
                    Ok(imap) => {
                        match find_folder_by_role(&state, &msg.account_id, FolderRole::Trash) {
                            Ok(ref trash_folder) if trash_folder.id != source_folder.id => {
                                if let Err(e) = imap.move_message(&source_folder.remote_id, uid, &trash_folder.remote_id).await {
                                    tracing::warn!("IMAP move to trash failed: {e}");
                                } else {
                                    info!("Moved message {} to Trash on server", message_id);
                                }
                            }
                            _ => {
                                if let Err(e) = imap.delete_message(&source_folder.remote_id, uid).await {
                                    tracing::warn!("IMAP delete failed: {e}");
                                } else {
                                    info!("Permanently deleted message {} (UID {})", message_id, uid);
                                }
                            }
                        }
                        let _ = imap.disconnect().await;
                    }
                    Err(e) => {
                        tracing::warn!("IMAP connect failed for delete: {e}");
                    }
                }
            }
        }
    }

    // Move locally to trash folder if it exists, otherwise soft-delete
    match find_folder_by_role(&state, &msg.account_id, FolderRole::Trash) {
        Ok(trash_folder) if trash_folder.id != source_folder.id => {
            state.store.move_message_to_folder(&message_id, &trash_folder.id)?;
        }
        _ => {
            state.store.soft_delete_message(&message_id)?;
        }
    }

    refresh_search_document(&state, &message_id)?;
    Ok(())
}

#[tauri::command]
pub async fn restore_message(
    state: State<'_, AppState>,
    message_id: String,
) -> std::result::Result<(), PebbleError> {
    let msg = state.store.get_message(&message_id)?
        .ok_or_else(|| PebbleError::Internal(format!("Message not found: {message_id}")))?;
    let provider_type = state
        .store
        .get_account(&msg.account_id)?
        .map(|account| account.provider)
        .ok_or_else(|| PebbleError::Internal(format!("Account not found: {}", msg.account_id)))?;

    let inbox = find_folder_by_role(&state, &msg.account_id, FolderRole::Inbox)?;

    // Get source folder before moving locally
    let source_folder = find_message_folder(&state, &message_id, &msg.account_id).ok();

    // Move locally to inbox
    state.store.move_message_to_folder(&message_id, &inbox.id)?;

    match provider_type {
        ProviderType::Gmail => {
            if let Some(ref src) = source_folder {
                if let Ok(provider) = connect_gmail(&state, &msg.account_id).await {
                    let result = if src.role == Some(FolderRole::Trash) {
                        provider.untrash_message(&msg.remote_id).await
                    } else {
                        provider.modify_labels(&msg.remote_id, &["INBOX".to_string()], &[]).await
                    };
                    if let Err(e) = result {
                        warn!("Gmail restore failed: {e}");
                    }
                }
            }
        }
        ProviderType::Imap | ProviderType::Outlook => {
            // Try to move on IMAP server too (skip for local-only folders)
            if let Ok(uid) = msg.remote_id.parse::<u32>() {
                if let Some(ref src) = source_folder {
                    let is_local = src.remote_id.starts_with("__local_");
                    if !is_local && src.id != inbox.id {
                        if let Ok(imap) = connect_imap(&state, &msg.account_id).await {
                            let _ = imap.move_message(&src.remote_id, uid, &inbox.remote_id).await;
                            let _ = imap.disconnect().await;
                        }
                    }
                }
            }
        }
    }

    refresh_search_document(&state, &message_id)?;
    info!("Restored message {} to inbox", message_id);
    Ok(())
}

#[tauri::command]
pub async fn empty_trash(
    state: State<'_, AppState>,
    account_id: String,
) -> std::result::Result<u32, PebbleError> {
    let trash = find_folder_by_role(&state, &account_id, FolderRole::Trash)?;
    let messages = state.store.list_messages_by_folder(&trash.id, 10000, 0)?;
    let provider_type = state
        .store
        .get_account(&account_id)?
        .map(|account| account.provider)
        .ok_or_else(|| PebbleError::Internal(format!("Account not found: {account_id}")))?;

    if messages.is_empty() {
        return Ok(0);
    }

    let count = messages.len() as u32;

    match provider_type {
        ProviderType::Gmail => {
            if let Ok(provider) = connect_gmail(&state, &account_id).await {
                for msg in &messages {
                    let _ = provider.delete_message_permanently(&msg.remote_id).await;
                }
            }
        }
        ProviderType::Imap | ProviderType::Outlook => {
            // Try to permanently delete on IMAP server
            if let Ok(imap) = connect_imap(&state, &account_id).await {
                for msg in &messages {
                    if let Ok(uid) = msg.remote_id.parse::<u32>() {
                        let _ = imap.delete_message(&trash.remote_id, uid).await;
                    }
                }
                let _ = imap.disconnect().await;
            }
        }
    }

    // Permanently delete locally (hard delete, not soft delete + purge)
    let ids: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
    state.store.hard_delete_messages(&ids)?;
    remove_search_documents(&state, &ids)?;

    info!("Emptied trash: {} messages permanently deleted", count);
    Ok(count)
}
