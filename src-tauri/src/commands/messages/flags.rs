use crate::commands::batch::{batch_mark_read_detailed, BatchMarkReadResult};
use crate::state::AppState;
use pebble_core::{Message, PebbleError, ProviderType};
use pebble_store::pending_ops::{FlagsOp, MailOp};
use tauri::State;

use super::provider_dispatch::remote_ids_are_folder_scoped;
use super::{apply_remote_or_queue, load_imap_config};

/// What the remote writeback needs, resolved from the local DB.
struct WritebackTarget {
    msg: Message,
    provider_type: ProviderType,
    /// Server folder holding the message, for folder-scoped ids (IMAP).
    folder_remote_id: Option<String>,
}

#[tauri::command]
pub async fn update_message_flags(
    state: State<'_, AppState>,
    message_id: String,
    is_read: Option<bool>,
    is_starred: Option<bool>,
) -> std::result::Result<(), PebbleError> {
    // 1. Resolve DB state off the Tokio runtime before attempting remote writeback.
    let store = state.store.clone();
    let crypto = state.crypto.clone();
    let msg_id = message_id.clone();

    let target = tokio::task::spawn_blocking(
        move || -> std::result::Result<Option<WritebackTarget>, PebbleError> {
            let Some(msg) = store.get_message(&msg_id)? else {
                return Ok(None);
            };
            let provider_type = store
                .get_account(&msg.account_id)?
                .map(|account| account.provider)
                .unwrap_or(ProviderType::Imap);
            if matches!(provider_type, ProviderType::Pop3) {
                return Ok(None);
            }
            if !remote_ids_are_folder_scoped(&provider_type) {
                return Ok(Some(WritebackTarget {
                    msg,
                    provider_type,
                    folder_remote_id: None,
                }));
            }
            // Local-only folders (e.g. `__local_archive__`) do not exist on
            // the server, so they can never be a writeback target.
            let folder_ids = store.get_message_folder_ids(&msg_id)?;
            let folders = store.list_folders(&msg.account_id)?;
            let folder = folder_ids
                .iter()
                .filter_map(|fid| folders.iter().find(|f| &f.id == fid))
                .find(|f| !f.remote_id.starts_with("__local_"))
                .cloned();
            // Missing config means the account has no IMAP writeback target;
            // the flag change is then valid locally only.
            let has_config = load_imap_config(&store, &crypto, &msg.account_id).is_ok();
            Ok(folder.filter(|_| has_config).map(|folder| WritebackTarget {
                msg,
                provider_type,
                folder_remote_id: Some(folder.remote_id),
            }))
        },
    )
    .await
    .map_err(|e| PebbleError::Internal(format!("Task join error: {e}")))??;

    // 2. Remote writeback. Local flags are committed only after remote ack,
    //    when queued while offline, or when there is no remote target.
    if let Some(target) = target {
        let op = MailOp::UpdateFlags(FlagsOp {
            is_read,
            is_starred,
            folder_remote_id: target.folder_remote_id,
            ..FlagsOp::default()
        });
        let nothing_to_write = is_read.is_none() && is_starred.is_none();
        apply_remote_or_queue(
            &state,
            &target.msg,
            &target.provider_type,
            op,
            nothing_to_write,
        )
        .await?;
    }

    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        store.update_message_flags(&message_id, is_read, is_starred)
    })
    .await
    .map_err(|e| PebbleError::Internal(format!("Task join error: {e}")))??;

    Ok(())
}

#[tauri::command]
pub async fn update_thread_read_flags_in_folders(
    state: State<'_, AppState>,
    thread_id: String,
    folder_ids: Vec<String>,
    is_read: bool,
) -> std::result::Result<BatchMarkReadResult, PebbleError> {
    let store = state.store.clone();
    let ids = tokio::task::spawn_blocking(move || {
        store.list_message_ids_by_thread_in_folders(&thread_id, &folder_ids)
    })
    .await
    .map_err(|e| PebbleError::Internal(format!("Task join error: {e}")))??;

    let mut result = BatchMarkReadResult {
        updated: 0,
        queued: 0,
        failed: 0,
    };
    for chunk in ids.chunks(1000) {
        let chunk_result = batch_mark_read_detailed(state.clone(), chunk.to_vec(), is_read).await?;
        result.updated += chunk_result.updated;
        result.queued += chunk_result.queued;
        result.failed += chunk_result.failed;
    }
    Ok(result)
}
