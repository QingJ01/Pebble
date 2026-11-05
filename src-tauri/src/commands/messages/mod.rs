pub mod flags;
pub mod lifecycle;
pub mod provider_dispatch;
pub mod query;
pub mod rendering;

// Shared helpers used by flags and lifecycle submodules.

use crate::commands::encrypted_store::load_account_auth_data;
use crate::commands::network::{
    account_proxy_mode_from_auth_value, resolve_mail_proxy_from_mode, AccountProxyMode,
};
use crate::commands::oauth::ensure_account_oauth_auth;
use crate::state::AppState;
use pebble_core::{FolderRole, Message, PebbleError};
use pebble_crypto::CryptoService;
use pebble_mail::{GmailProvider, ImapConfig, ImapProvider, OutlookProvider, Pop3Config};
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

pub(super) fn remote_delete_is_already_absent(error: &PebbleError) -> bool {
    let message = error.to_string().to_ascii_lowercase();
    message.contains("status 404")
        || message.contains("404 not found")
        || message.contains("message not found")
        || message.contains("no such message")
}

pub(super) fn classify_remote_delete_result(
    result: std::result::Result<(), PebbleError>,
    permanent: bool,
) -> std::result::Result<(), PebbleError> {
    match result {
        Err(error) if permanent && remote_delete_is_already_absent(&error) => Ok(()),
        result => result,
    }
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
    let auth = ensure_account_oauth_auth(state, account_id, "gmail").await?;
    GmailProvider::new_with_proxy(auth.tokens.access_token, auth.proxy)
}

pub(super) async fn connect_outlook(
    state: &AppState,
    account_id: &str,
) -> std::result::Result<OutlookProvider, PebbleError> {
    let auth = ensure_account_oauth_auth(state, account_id, "outlook").await?;
    OutlookProvider::new_with_proxy(auth.tokens.access_token, account_id.to_string(), auth.proxy)
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
pub(crate) fn refresh_search_documents(
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
    let (mut config, proxy_mode): (ImapConfig, AccountProxyMode) = if let Some(decrypted) =
        load_account_auth_data(crypto, store, account_id)?
    {
        let value: serde_json::Value = serde_json::from_slice(&decrypted)
            .map_err(|e| PebbleError::Internal(format!("Failed to parse config: {e}")))?;
        let proxy_mode = account_proxy_mode_from_auth_value(&value);
        let config = serde_json::from_value(value.get("imap").cloned().unwrap_or(value.clone()))
            .map_err(|e| {
                PebbleError::Internal(format!("Failed to deserialize IMAP config: {e}"))
            })?;
        (config, proxy_mode)
    } else {
        // Legacy path: IMAP config used to live inline in sync_state.
        let sync_state = store
            .get_sync_state(account_id)?
            .ok_or_else(|| PebbleError::Internal(format!("No config for account {account_id}")))?;
        let imap_value = sync_state.imap.ok_or_else(|| {
            PebbleError::Internal(format!("No IMAP config for account {account_id}"))
        })?;
        let config = serde_json::from_value(imap_value).map_err(|e| {
            PebbleError::Internal(format!("Failed to deserialize IMAP config: {e}"))
        })?;
        (config, AccountProxyMode::Inherit)
    };

    config.proxy = resolve_mail_proxy_from_mode(crypto, store, proxy_mode, config.proxy)?;

    Ok(config)
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

pub(super) fn load_pop3_config(
    store: &Store,
    crypto: &CryptoService,
    account_id: &str,
) -> std::result::Result<Pop3Config, PebbleError> {
    let (imap_config, proxy_mode) = if let Some(decrypted) =
        load_account_auth_data(crypto, store, account_id)?
    {
        let value: serde_json::Value = serde_json::from_slice(&decrypted)
            .map_err(|e| PebbleError::Internal(format!("Failed to parse config: {e}")))?;
        let proxy_mode = account_proxy_mode_from_auth_value(&value);
        let config: ImapConfig = serde_json::from_value(
            value.get("imap").cloned().unwrap_or(value.clone()),
        )
        .map_err(|e| PebbleError::Internal(format!("Failed to deserialize POP3 config: {e}")))?;
        (config, proxy_mode)
    } else {
        return Err(PebbleError::Internal(format!(
            "No POP3 config for account {account_id}"
        )));
    };

    let proxy = resolve_mail_proxy_from_mode(crypto, store, proxy_mode, imap_config.proxy)?;
    Ok(Pop3Config {
        host: imap_config.host,
        port: imap_config.port,
        username: imap_config.username,
        password: imap_config.password,
        security: imap_config.security,
        accept_invalid_certs: imap_config.accept_invalid_certs,
        proxy,
    })
}

/// Find the folder with a given role for an account.
pub(super) fn find_folder_by_role(
    state: &AppState,
    account_id: &str,
    role: FolderRole,
) -> std::result::Result<pebble_core::Folder, PebbleError> {
    state
        .store
        .find_folder_by_role(account_id, role.clone())?
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
