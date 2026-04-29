pub mod flags;
pub mod lifecycle;
pub mod provider_dispatch;
pub mod query;
pub mod rendering;

// Shared helpers used by flags and lifecycle submodules.

use crate::commands::oauth::ensure_account_oauth_tokens;
use crate::state::AppState;
use pebble_core::{FolderRole, Message, PebbleError};
use pebble_crypto::CryptoService;
use pebble_mail::{GmailProvider, ImapConfig, ImapProvider, OutlookProvider};
use pebble_search::TantivySearch;
use pebble_store::Store;
use serde_json::json;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum RemoteMutationOutcome {
    Applied,
    Queued,
    QueuedLocalCommit,
    LocalOnly,
    #[allow(dead_code)]
    Failed,
}

pub(super) fn remote_mutation_allows_local_commit(outcome: RemoteMutationOutcome) -> bool {
    matches!(
        outcome,
        RemoteMutationOutcome::Applied
            | RemoteMutationOutcome::QueuedLocalCommit
            | RemoteMutationOutcome::LocalOnly
    )
}

pub(super) fn queue_pending_remote_op(
    state: &AppState,
    message: &Message,
    op_type: &str,
    payload: serde_json::Value,
    error: &str,
) -> std::result::Result<RemoteMutationOutcome, PebbleError> {
    let payload = json!({
        "provider_account_id": message.account_id,
        "remote_id": message.remote_id,
        "op": op_type,
        "payload": payload,
    });
    let op_id = state.store.insert_pending_mail_op(
        &message.account_id,
        &message.id,
        op_type,
        &payload.to_string(),
    )?;
    state.store.mark_pending_mail_op_failed(&op_id, error)?;
    Ok(RemoteMutationOutcome::Queued)
}

pub(super) fn queue_pending_remote_op_for_local_commit(
    state: &AppState,
    message: &Message,
    op_type: &str,
    payload: serde_json::Value,
    error: &str,
) -> std::result::Result<RemoteMutationOutcome, PebbleError> {
    let outcome = queue_pending_remote_op(state, message, op_type, payload, error)?;
    debug_assert_eq!(outcome, RemoteMutationOutcome::Queued);
    Ok(RemoteMutationOutcome::QueuedLocalCommit)
}

pub(super) fn queued_remote_error(op_type: &str, error: &str) -> PebbleError {
    PebbleError::Network(format!(
        "Remote {op_type} failed and was queued for retry: {error}"
    ))
}

pub(super) async fn connect_gmail(
    state: &AppState,
    account_id: &str,
) -> std::result::Result<GmailProvider, PebbleError> {
    let tokens = ensure_account_oauth_tokens(state, account_id, "gmail").await?;
    Ok(GmailProvider::new(tokens.access_token))
}

pub(super) async fn connect_outlook(
    state: &AppState,
    account_id: &str,
) -> std::result::Result<OutlookProvider, PebbleError> {
    let tokens = ensure_account_oauth_tokens(state, account_id, "outlook").await?;
    Ok(OutlookProvider::new(
        tokens.access_token,
        account_id.to_string(),
    ))
}

pub(crate) fn refresh_search_document(
    state: &AppState,
    message_id: &str,
) -> std::result::Result<(), PebbleError> {
    refresh_search_document_with_store(&state.store, &state.search, message_id)
}

pub(crate) fn refresh_search_document_with_store(
    store: &Store,
    search: &TantivySearch,
    message_id: &str,
) -> std::result::Result<(), PebbleError> {
    let ids = vec![message_id.to_string()];
    store.add_search_pending(&ids, "index")?;

    match store.get_message(message_id)? {
        Some(message) if !message.is_deleted => {
            let folder_ids = store.get_message_folder_ids(message_id)?;
            if folder_ids.is_empty() {
                search.remove_message(message_id)?;
            } else {
                search.index_message(&message, &folder_ids)?;
            }
        }
        Some(_) | None => {
            search.remove_message(message_id)?;
        }
    }

    search.commit()?;
    store.clear_search_pending(&ids)?;
    Ok(())
}

pub(super) fn remove_search_documents(
    state: &AppState,
    message_ids: &[String],
) -> std::result::Result<(), PebbleError> {
    if message_ids.is_empty() {
        return Ok(());
    }
    state.store.add_search_pending(message_ids, "remove")?;
    for message_id in message_ids {
        state.search.remove_message(message_id)?;
    }
    state.search.commit()?;
    state.store.clear_search_pending(message_ids)?;
    Ok(())
}

/// Refresh multiple search documents with a single index commit at the end.
/// Use this instead of calling `refresh_search_document` in a loop: one commit
/// for N messages is dramatically cheaper than N commits (segment flush +
/// reader reload per doc).
pub(super) fn refresh_search_documents(
    state: &AppState,
    message_ids: &[String],
) -> std::result::Result<(), PebbleError> {
    if message_ids.is_empty() {
        return Ok(());
    }
    state.store.add_search_pending(message_ids, "index")?;
    for message_id in message_ids {
        match state.store.get_message(message_id)? {
            Some(message) if !message.is_deleted => {
                let folder_ids = state.store.get_message_folder_ids(message_id)?;
                if folder_ids.is_empty() {
                    state.search.remove_message(message_id)?;
                } else {
                    state.search.index_message(&message, &folder_ids)?;
                }
            }
            Some(_) | None => {
                state.search.remove_message(message_id)?;
            }
        }
    }
    state.search.commit()?;
    state.store.clear_search_pending(message_ids)?;
    Ok(())
}

/// Extract the IMAP config for an account (without connecting). Takes
/// `&Store`/`&CryptoService` rather than `&AppState` so it's callable from
/// inside `spawn_blocking` closures that only hold cloned `Arc`s.
pub(super) fn load_imap_config(
    store: &Store,
    crypto: &CryptoService,
    account_id: &str,
) -> std::result::Result<ImapConfig, PebbleError> {
    if let Some(encrypted) = store.get_auth_data(account_id)? {
        let decrypted = crypto.decrypt(&encrypted)?;
        let value: serde_json::Value = serde_json::from_slice(&decrypted)
            .map_err(|e| PebbleError::Internal(format!("Failed to parse config: {e}")))?;
        serde_json::from_value(value.get("imap").cloned().unwrap_or(value.clone()))
            .map_err(|e| PebbleError::Internal(format!("Failed to deserialize IMAP config: {e}")))
    } else {
        // Legacy path: IMAP config used to live inline in sync_state.
        let sync_state = store
            .get_sync_state(account_id)?
            .ok_or_else(|| PebbleError::Internal(format!("No config for account {account_id}")))?;
        let imap_value = sync_state.imap.ok_or_else(|| {
            PebbleError::Internal(format!("No IMAP config for account {account_id}"))
        })?;
        serde_json::from_value(imap_value)
            .map_err(|e| PebbleError::Internal(format!("Failed to deserialize IMAP config: {e}")))
    }
}

/// Resolve an IMAP connection from the account's auth data.
pub(super) async fn connect_imap(
    state: &AppState,
    account_id: &str,
) -> std::result::Result<ImapProvider, PebbleError> {
    let imap_config = load_imap_config(&state.store, &state.crypto, account_id)?;
    let provider = ImapProvider::new(imap_config);
    provider.connect().await?;
    Ok(provider)
}

/// Find the folder with a given role for an account.
pub(super) fn find_folder_by_role(
    state: &AppState,
    account_id: &str,
    role: FolderRole,
) -> std::result::Result<pebble_core::Folder, PebbleError> {
    let folders = state.store.list_folders(account_id)?;
    folders
        .into_iter()
        .find(|f| f.role == Some(role.clone()))
        .ok_or_else(|| PebbleError::Internal(format!("No {:?} folder found", role)))
}

/// Find the folder containing a given message (via the message_folders junction table).
pub(super) fn find_message_folder(
    state: &AppState,
    message_id: &str,
    account_id: &str,
) -> std::result::Result<pebble_core::Folder, PebbleError> {
    let folder_ids = state.store.get_message_folder_ids(message_id)?;
    if folder_ids.is_empty() {
        return Err(PebbleError::Internal(
            "Message not found in any folder".to_string(),
        ));
    }
    let folders = state.store.list_folders(account_id)?;
    // Return the first matching folder (prefer inbox-like folders)
    for fid in &folder_ids {
        if let Some(folder) = folders.iter().find(|f| &f.id == fid) {
            return Ok(folder.clone());
        }
    }
    Err(PebbleError::Internal(
        "Message folder not found".to_string(),
    ))
}
