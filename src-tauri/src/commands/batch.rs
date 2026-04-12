use crate::state::AppState;
use super::messages::{refresh_search_document, find_folder_by_role, find_message_folder};
use super::messages::provider_dispatch::{ConnectedProvider, parse_imap_uid};
use pebble_core::traits::{FolderProvider, LabelProvider};
use pebble_core::{FolderRole, Message, PebbleError, ProviderType};
use pebble_store::Store;
use std::collections::HashMap;
use tauri::State;
use tracing::{info, warn};

/// Group messages by account_id and resolve their provider type.
/// Uses a batch query to avoid N+1 individual lookups.
fn group_by_account(
    store: &Store,
    message_ids: &[String],
) -> std::result::Result<HashMap<String, (ProviderType, Vec<Message>)>, PebbleError> {
    let messages = store.get_messages_batch(message_ids)?;
    let mut groups: HashMap<String, (ProviderType, Vec<Message>)> = HashMap::new();
    for msg in messages {
        let provider = store
            .get_account(&msg.account_id)?
            .map(|a| a.provider)
            .unwrap_or(ProviderType::Imap);
        groups
            .entry(msg.account_id.clone())
            .or_insert_with(|| (provider, Vec::new()))
            .1
            .push(msg);
    }
    Ok(groups)
}

#[tauri::command]
pub async fn batch_archive(
    state: State<'_, AppState>,
    message_ids: Vec<String>,
) -> std::result::Result<u32, PebbleError> {
    if message_ids.is_empty() {
        return Ok(0);
    }

    let store = state.store.clone();
    let ids = message_ids.clone();
    let groups = tokio::task::spawn_blocking(move || {
        group_by_account(&store, &ids)
    })
    .await
    .map_err(|e| PebbleError::Internal(format!("Task join error: {e}")))??;
    let mut success_count: u32 = 0;
    let mut archived_ids = Vec::new();

    for (account_id, (provider_type, messages)) in &groups {
        let archive_folder = find_folder_by_role(&state, account_id, FolderRole::Archive).ok();

        // Remote sync — connect once per account, operate, disconnect.
        // For Outlook/IMAP we need a usable (non-local) archive folder on the
        // server; skip the connection entirely when there isn't one.
        let has_remote_archive = archive_folder
            .as_ref()
            .is_some_and(|af| !af.remote_id.starts_with("__local_"));
        let needs_connection = matches!(provider_type, ProviderType::Gmail) || has_remote_archive;

        if needs_connection {
            if let Ok(conn) = ConnectedProvider::connect(&state, account_id, provider_type).await {
                match &conn {
                    ConnectedProvider::Gmail(provider) => {
                        for msg in messages {
                            if let Err(e) = provider.modify_labels(&msg.remote_id, &[], &["INBOX".to_string()]).await {
                                warn!("Gmail batch archive failed for {}: {e}", msg.id);
                            }
                        }
                    }
                    ConnectedProvider::Outlook(provider) => {
                        let Some(af) = archive_folder.as_ref() else {
                            continue;
                        };
                        for msg in messages {
                            if let Err(e) = provider.move_message(&msg.remote_id, &af.remote_id).await {
                                warn!("Outlook batch archive failed for {}: {e}", msg.id);
                            }
                        }
                    }
                    ConnectedProvider::Imap(imap) => {
                        let Some(af) = archive_folder.as_ref() else {
                            continue;
                        };
                        for msg in messages {
                            if let Ok(uid) = parse_imap_uid(&msg.remote_id) {
                                if let Ok(src) = find_message_folder(&state, &msg.id, account_id) {
                                    if let Err(e) = imap.move_message(&src.remote_id, uid, &af.remote_id).await {
                                        warn!("IMAP batch archive failed for {}: {e}", msg.id);
                                    }
                                }
                            }
                        }
                    }
                }
                conn.disconnect().await;
            }
        }

        // Local store update
        for msg in messages {
            let result = match &archive_folder {
                Some(af) => state.store.move_message_to_folder(&msg.id, &af.id),
                None => state.store.soft_delete_message(&msg.id),
            };
            match result {
                Ok(()) => {
                    success_count += 1;
                    archived_ids.push(msg.id.clone());
                }
                Err(e) => warn!("Failed to archive message {}: {e}", msg.id),
            }
        }
    }

    // Update search index for archived messages (refresh, not remove)
    for id in &archived_ids {
        if let Err(e) = refresh_search_document(&state, id) {
            warn!("Failed to refresh search document for archived message {id}: {e}");
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
    if message_ids.is_empty() {
        return Ok(0);
    }

    let store = state.store.clone();
    let ids = message_ids.clone();
    let groups = tokio::task::spawn_blocking(move || {
        group_by_account(&store, &ids)
    })
    .await
    .map_err(|e| PebbleError::Internal(format!("Task join error: {e}")))??;

    // Track which messages were successfully deleted remotely
    let mut deleted_ids: Vec<String> = Vec::new();

    // Remote sync — connect once per account, operate, disconnect
    for (account_id, (provider_type, messages)) in &groups {
        if let Ok(conn) = ConnectedProvider::connect(&state, account_id, provider_type).await {
            match &conn {
                ConnectedProvider::Gmail(provider) => {
                    for msg in messages {
                        match provider.trash_message(&msg.remote_id).await {
                            Ok(_) => deleted_ids.push(msg.id.clone()),
                            Err(e) => warn!("Gmail batch delete failed for {}: {e}", msg.id),
                        }
                    }
                }
                ConnectedProvider::Outlook(provider) => {
                    for msg in messages {
                        match provider.trash_message(&msg.remote_id).await {
                            Ok(_) => deleted_ids.push(msg.id.clone()),
                            Err(e) => warn!("Outlook batch delete failed for {}: {e}", msg.id),
                        }
                    }
                }
                ConnectedProvider::Imap(imap) => {
                    if let Ok(trash_folder) = find_folder_by_role(&state, account_id, FolderRole::Trash) {
                        for msg in messages {
                            if let Ok(uid) = parse_imap_uid(&msg.remote_id) {
                                if let Ok(src) = find_message_folder(&state, &msg.id, account_id) {
                                    if src.id != trash_folder.id {
                                        match imap.move_message(&src.remote_id, uid, &trash_folder.remote_id).await {
                                            Ok(_) => deleted_ids.push(msg.id.clone()),
                                            Err(e) => warn!("IMAP batch delete failed for {}: {e}", msg.id),
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            conn.disconnect().await;
        }
    }

    // Local bulk soft-delete — only messages that were successfully deleted remotely
    state.store.bulk_soft_delete(&deleted_ids)?;
    let success_count = deleted_ids.len() as u32;

    // Update search index — remove deleted messages
    for id in &deleted_ids {
        if let Err(e) = state.search.remove_message(id) {
            warn!("Failed to remove deleted message {id} from search index: {e}");
        }
    }
    if let Err(e) = state.search.commit() {
        warn!("Failed to commit search index after batch delete: {e}");
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
    if message_ids.is_empty() {
        return Ok(0);
    }

    let store = state.store.clone();
    let ids = message_ids.clone();
    let groups = tokio::task::spawn_blocking(move || {
        group_by_account(&store, &ids)
    })
    .await
    .map_err(|e| PebbleError::Internal(format!("Task join error: {e}")))??;

    // Remote sync — connect once per account, operate, disconnect
    for (account_id, (provider_type, messages)) in &groups {
        if let Ok(conn) = ConnectedProvider::connect(&state, account_id, provider_type).await {
            match &conn {
                ConnectedProvider::Gmail(provider) => {
                    let (add, remove) = if is_read {
                        (vec![], vec!["UNREAD".to_string()])
                    } else {
                        (vec!["UNREAD".to_string()], vec![])
                    };
                    for msg in messages {
                        if let Err(e) = provider.modify_labels(&msg.remote_id, &add, &remove).await {
                            warn!("Gmail batch mark_read failed for {}: {e}", msg.id);
                        }
                    }
                }
                ConnectedProvider::Outlook(provider) => {
                    for msg in messages {
                        if let Err(e) = provider.update_read_status(&msg.remote_id, is_read).await {
                            warn!("Outlook batch mark_read failed for {}: {e}", msg.id);
                        }
                    }
                }
                ConnectedProvider::Imap(imap) => {
                    for msg in messages {
                        if let Ok(uid) = parse_imap_uid(&msg.remote_id) {
                            if let Ok(folder) = find_message_folder(&state, &msg.id, account_id) {
                                if let Err(e) = imap.set_flags(&folder.remote_id, uid, Some(is_read), None).await {
                                    warn!("IMAP batch mark_read failed for {}: {e}", msg.id);
                                }
                            }
                        }
                    }
                }
            }
            conn.disconnect().await;
        }
    }

    // Local bulk flag update
    let changes: Vec<(String, Option<bool>, Option<bool>)> = message_ids
        .iter()
        .map(|id| (id.clone(), Some(is_read), None))
        .collect();
    state.store.bulk_update_flags(&changes)?;
    let success_count = message_ids.len() as u32;

    info!(
        "Batch mark_read({}): {}/{} messages updated",
        is_read,
        success_count,
        message_ids.len()
    );
    Ok(success_count)
}
