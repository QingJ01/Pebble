use crate::state::AppState;
use pebble_core::traits::{FolderProvider, LabelProvider};
use pebble_core::{FolderRole, Message, PebbleError, ProviderType};
use tauri::State;
use tracing::{info, warn};

use super::{
    connect_gmail, connect_imap, connect_outlook, find_folder_by_role, find_message_folder,
    refresh_search_document, remove_search_documents,
};
use super::provider_dispatch::{ConnectedProvider, parse_imap_uid};

/// Load a message and its account's provider type, surfacing a clear error
/// if either is missing. Four lifecycle commands share this preamble.
fn resolve_message_context(
    state: &AppState,
    message_id: &str,
) -> std::result::Result<(Message, ProviderType), PebbleError> {
    let msg = state.store.get_message(message_id)?
        .ok_or_else(|| PebbleError::Internal(format!("Message not found: {message_id}")))?;
    let provider_type = state
        .store
        .get_account(&msg.account_id)?
        .map(|account| account.provider)
        .ok_or_else(|| PebbleError::Internal(format!("Account not found: {}", msg.account_id)))?;
    Ok((msg, provider_type))
}

/// Returns "archived" or "unarchived" so the frontend can show the correct toast.
#[tauri::command]
pub async fn archive_message(
    state: State<'_, AppState>,
    message_id: String,
) -> std::result::Result<String, PebbleError> {
    let (msg, provider_type) = resolve_message_context(&state, &message_id)?;

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
                ProviderType::Outlook => {
                    if !is_local {
                        match connect_outlook(&state, &msg.account_id).await {
                            Ok(provider) => {
                                if let Err(e) = provider.move_message(&msg.remote_id, &archive_folder.remote_id).await {
                                    warn!("Outlook move to archive failed: {e}");
                                } else {
                                    info!("Archived Outlook message {} from {} to {}", message_id, source_folder.name, archive_folder.name);
                                }
                            }
                            Err(e) => warn!("Outlook connect failed for archive: {e}"),
                        }
                    }
                }
                ProviderType::Imap => {
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
    let (msg, provider_type) = resolve_message_context(&state, &message_id)?;

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
        ProviderType::Outlook => {
            match connect_outlook(&state, &msg.account_id).await {
                Ok(provider) => {
                    if source_folder.role == Some(FolderRole::Trash) {
                        if let Err(e) = provider.delete_message_permanently(&msg.remote_id).await {
                            warn!("Outlook permanent delete failed: {e}");
                        }
                    } else if let Err(e) = provider.trash_message(&msg.remote_id).await {
                        warn!("Outlook trash failed: {e}");
                    } else {
                        info!("Moved Outlook message {} to Trash on server", message_id);
                    }
                }
                Err(e) => warn!("Outlook connect failed for delete: {e}"),
            }
        }
        ProviderType::Imap => {
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
    let (msg, provider_type) = resolve_message_context(&state, &message_id)?;

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
        ProviderType::Outlook => {
            if let Ok(provider) = connect_outlook(&state, &msg.account_id).await {
                if let Err(e) = provider.restore_message(&msg.remote_id).await {
                    warn!("Outlook restore failed: {e}");
                }
            }
        }
        ProviderType::Imap => {
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
    let target_folder = target_folders.iter()
        .find(|f| f.id == target_folder_id)
        .ok_or_else(|| PebbleError::Internal(format!("Target folder not found: {target_folder_id}")))?;

    let is_local_target = target_folder.remote_id.starts_with("__local_");

    // TODO(FC-5): add full remote-move support for Gmail label semantics
    match provider_type {
        ProviderType::Outlook => {
            if !is_local_target {
                match connect_outlook(&state, &msg.account_id).await {
                    Ok(provider) => {
                        if let Err(e) = provider.move_message(&msg.remote_id, &target_folder.remote_id).await {
                            warn!("Outlook move_to_folder failed: {e}");
                        } else {
                            info!("Moved Outlook message {} to folder {}", message_id, target_folder.name);
                        }
                    }
                    Err(e) => warn!("Outlook connect failed for move_to_folder: {e}"),
                }
            }
        }
        ProviderType::Imap => {
            if !is_local_target {
                if let Ok(uid) = msg.remote_id.parse::<u32>() {
                    match connect_imap(&state, &msg.account_id).await {
                        Ok(imap) => {
                            let result = imap.move_message(&source_folder.remote_id, uid, &target_folder.remote_id).await;
                            let _ = imap.disconnect().await;
                            if let Err(e) = result {
                                warn!("IMAP move_to_folder failed: {e}");
                            } else {
                                info!("Moved IMAP message {} (UID {}) to folder {}", message_id, uid, target_folder.name);
                            }
                        }
                        Err(e) => warn!("IMAP connect failed for move_to_folder: {e}"),
                    }
                }
            }
        }
        ProviderType::Gmail => {
            // Gmail uses labels; moving to a non-inbox folder means removing INBOX label
            if let Ok(provider) = connect_gmail(&state, &msg.account_id).await {
                if target_folder.role == Some(pebble_core::FolderRole::Spam) {
                    if let Err(e) = provider.modify_labels(&msg.remote_id, &["SPAM".to_string()], &["INBOX".to_string()]).await {
                        warn!("Gmail mark as spam failed: {e}");
                    }
                } else if !is_local_target {
                    // Generic move: strip INBOX, add target label
                    if let Err(e) = provider.modify_labels(&msg.remote_id, &[target_folder.remote_id.clone()], &["INBOX".to_string()]).await {
                        warn!("Gmail move_to_folder failed: {e}");
                    }
                }
            }
        }
    }

    state.store.move_message_to_folder(&message_id, &target_folder_id)?;
    refresh_search_document(&state, &message_id)?;
    info!("Moved message {} to folder {} ({})", message_id, target_folder.name, target_folder_id);
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

    let conn = ConnectedProvider::connect(&state, &account_id, &provider_type).await.ok();

    let mut total_deleted: u32 = 0;
    const PAGE_SIZE: u32 = 500;

    loop {
        let messages = state.store.list_messages_by_folder(&trash.id, PAGE_SIZE, 0)?;
        if messages.is_empty() {
            break;
        }

        if let Some(ref conn) = conn {
            match conn {
                ConnectedProvider::Gmail(provider) => {
                    for msg in &messages {
                        let _ = provider.delete_message_permanently(&msg.remote_id).await;
                    }
                }
                ConnectedProvider::Outlook(provider) => {
                    for msg in &messages {
                        let _ = provider.delete_message_permanently(&msg.remote_id).await;
                    }
                }
                ConnectedProvider::Imap(imap) => {
                    for msg in &messages {
                        if let Ok(uid) = parse_imap_uid(&msg.remote_id) {
                            let _ = imap.delete_message(&trash.remote_id, uid).await;
                        }
                    }
                }
            }
        }

        let ids: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
        let batch_count = ids.len() as u32;
        state.store.hard_delete_messages(&ids)?;
        remove_search_documents(&state, &ids)?;
        total_deleted += batch_count;

        if batch_count < PAGE_SIZE {
            break;
        }
    }

    if let Some(conn) = conn {
        conn.disconnect().await;
    }

    info!("Emptied trash: {} messages permanently deleted", total_deleted);
    Ok(total_deleted)
}
