use crate::{events, state::AppState};
use pebble_core::traits::{FolderProvider, LabelProvider, MailTransport};
use pebble_core::{FolderRole, Message, PebbleError, ProviderType};
use pebble_store::pending_ops::{PendingMailOp, PendingMailOpsSummary};
use pebble_store::Store;
use serde::Serialize;
use serde_json::Value;
use tauri::{Emitter, Manager, State};
use tracing::{debug, warn};

use super::compose::{self, LocalOutgoingState};
use super::messages::{
    connect_gmail, connect_imap, connect_outlook, find_folder_by_role, find_message_folder,
    refresh_search_document, remove_search_documents,
};

const WORKER_INTERVAL_SECS: u64 = 30;
const WORKER_BATCH_LIMIT: i64 = 20;

#[derive(Debug, Clone, Serialize)]
pub struct PendingMailOpsSummaryResponse {
    pub pending_count: i64,
    pub in_progress_count: i64,
    pub failed_count: i64,
    pub total_active_count: i64,
    pub last_error: Option<String>,
    pub updated_at: Option<i64>,
}

impl From<PendingMailOpsSummary> for PendingMailOpsSummaryResponse {
    fn from(value: PendingMailOpsSummary) -> Self {
        Self {
            pending_count: value.pending_count,
            in_progress_count: value.in_progress_count,
            failed_count: value.failed_count,
            total_active_count: value.total_active_count,
            last_error: value.last_error,
            updated_at: value.updated_at,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PendingMailOpResponse {
    pub id: String,
    pub account_id: String,
    pub message_id: String,
    pub op_type: String,
    pub status: String,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub next_retry_at: Option<i64>,
}

impl From<PendingMailOp> for PendingMailOpResponse {
    fn from(value: PendingMailOp) -> Self {
        Self {
            id: value.id,
            account_id: value.account_id,
            message_id: value.message_id,
            op_type: value.op_type,
            status: value.status.as_str().to_string(),
            attempts: value.attempts,
            last_error: value.last_error,
            created_at: value.created_at,
            updated_at: value.updated_at,
            next_retry_at: value.next_retry_at,
        }
    }
}

#[tauri::command]
pub fn get_pending_mail_ops_summary(
    state: State<'_, AppState>,
    account_id: Option<String>,
) -> std::result::Result<PendingMailOpsSummaryResponse, PebbleError> {
    state
        .store
        .pending_mail_ops_summary(account_id.as_deref())
        .map(Into::into)
}

#[tauri::command]
pub fn list_pending_mail_ops(
    state: State<'_, AppState>,
    account_id: Option<String>,
    limit: Option<i64>,
) -> std::result::Result<Vec<PendingMailOpResponse>, PebbleError> {
    let limit = limit.unwrap_or(100).clamp(1, 500);
    state
        .store
        .list_active_pending_mail_ops(account_id.as_deref(), limit)
        .map(|ops| ops.into_iter().map(Into::into).collect())
}

pub fn queue_pending_mail_op(
    store: &Store,
    message: &Message,
    op_type: &str,
    payload: Value,
) -> std::result::Result<String, PebbleError> {
    let payload = serde_json::json!({
        "provider_account_id": message.account_id,
        "remote_id": message.remote_id,
        "op": op_type,
        "payload": payload,
    });
    store.insert_pending_mail_op(
        &message.account_id,
        &message.id,
        op_type,
        &payload.to_string(),
    )
}

pub async fn run_pending_mail_ops_worker(app: tauri::AppHandle) {
    let mut interval =
        tokio::time::interval(tokio::time::Duration::from_secs(WORKER_INTERVAL_SECS));
    {
        let state = app.state::<AppState>();
        if let Err(e) = state.store.reset_in_progress_pending_mail_ops() {
            warn!("Failed to reset interrupted pending mail ops: {e}");
        }
    }

    loop {
        interval.tick().await;
        let state = app.state::<AppState>();
        if let Err(e) = process_pending_mail_ops(&state, Some(&app)).await {
            warn!("Pending mail op worker pass failed: {e}");
        }
    }
}

pub async fn process_pending_mail_ops(
    state: &AppState,
    app: Option<&tauri::AppHandle>,
) -> std::result::Result<usize, PebbleError> {
    let ops = state
        .store
        .list_retryable_pending_mail_ops(WORKER_BATCH_LIMIT)?;
    let mut changed = false;
    let mut completed = 0usize;

    for op in ops {
        state.store.mark_pending_mail_op_in_progress(&op.id)?;
        changed = true;

        match replay_pending_mail_op(state, &op).await {
            Ok(()) => {
                state.store.mark_pending_mail_op_done(&op.id)?;
                completed += 1;
            }
            Err(e) => {
                state
                    .store
                    .mark_pending_mail_op_failed(&op.id, &e.to_string())?;
                warn!("Pending mail op {} retry failed: {e}", op.id);
            }
        }
    }

    if changed {
        emit_pending_ops_changed(app);
    }
    Ok(completed)
}

fn emit_pending_ops_changed(app: Option<&tauri::AppHandle>) {
    if let Some(app) = app {
        let _ = app.emit(events::MAIL_PENDING_OPS_CHANGED, ());
    }
}

async fn replay_pending_mail_op(
    state: &AppState,
    op: &PendingMailOp,
) -> std::result::Result<(), PebbleError> {
    let Some(message) = state.store.get_message(&op.message_id)? else {
        debug!("Pending mail op {} skipped because message is gone", op.id);
        return Ok(());
    };
    let Some(account) = state.store.get_account(&op.account_id)? else {
        debug!("Pending mail op {} skipped because account is gone", op.id);
        return Ok(());
    };

    let payload = op_payload(op)?;
    match op.op_type.as_str() {
        "update_flags" => {
            let is_read = optional_bool(&payload, "is_read");
            let is_starred = optional_bool(&payload, "is_starred");
            replay_remote_update_flags(
                state,
                account.provider,
                &message,
                &payload,
                is_read,
                is_starred,
            )
            .await?;
        }
        "archive" => {
            replay_remote_archive(state, account.provider, &message, &payload).await?;
        }
        "unarchive" | "restore" => {
            replay_remote_restore(state, account.provider, &message, &payload).await?;
        }
        "delete" => {
            replay_remote_delete(state, account.provider, &message, &payload, false).await?;
        }
        "delete_permanent" => {
            replay_remote_delete(state, account.provider, &message, &payload, true).await?;
        }
        "move_to_folder" => {
            replay_remote_move_to_folder(state, account.provider, &message, &payload).await?;
        }
        "send" => {
            replay_remote_send(state, account.provider.clone(), &account, &message).await?;
        }
        other => {
            return Err(PebbleError::Internal(format!(
                "Unsupported pending mail op type: {other}"
            )));
        }
    }

    apply_pending_local_commit(&state.store, op)?;
    refresh_after_pending_commit(state, op)?;
    Ok(())
}

async fn replay_remote_update_flags(
    state: &AppState,
    provider_type: ProviderType,
    message: &pebble_core::Message,
    payload: &Value,
    is_read: Option<bool>,
    is_starred: Option<bool>,
) -> std::result::Result<(), PebbleError> {
    match provider_type {
        ProviderType::Gmail => {
            let add = string_array(payload, "add_labels").unwrap_or_else(|| {
                let mut labels = Vec::new();
                if is_read == Some(false) {
                    labels.push("UNREAD".to_string());
                }
                if is_starred == Some(true) {
                    labels.push("STARRED".to_string());
                }
                labels
            });
            let remove = string_array(payload, "remove_labels").unwrap_or_else(|| {
                let mut labels = Vec::new();
                if is_read == Some(true) {
                    labels.push("UNREAD".to_string());
                }
                if is_starred == Some(false) {
                    labels.push("STARRED".to_string());
                }
                labels
            });
            if add.is_empty() && remove.is_empty() {
                return Ok(());
            }
            connect_gmail(state, &message.account_id)
                .await?
                .modify_labels(&message.remote_id, &add, &remove)
                .await
        }
        ProviderType::Outlook => {
            let provider = connect_outlook(state, &message.account_id).await?;
            if let Some(read) = is_read {
                provider
                    .update_read_status(&message.remote_id, read)
                    .await?;
            }
            if let Some(starred) = is_starred {
                provider
                    .update_flag_status(&message.remote_id, starred)
                    .await?;
            }
            Ok(())
        }
        ProviderType::Imap => {
            let folder_remote_id = string_field(payload, "folder_remote_id")
                .or_else(|| {
                    find_message_folder(state, &message.id, &message.account_id)
                        .ok()
                        .map(|folder| folder.remote_id)
                })
                .ok_or_else(|| {
                    PebbleError::Internal("Pending update_flags has no IMAP folder".to_string())
                })?;
            let uid = message
                .remote_id
                .parse::<u32>()
                .map_err(|e| PebbleError::Internal(format!("Invalid IMAP UID: {e}")))?;
            let imap = connect_imap(state, &message.account_id).await?;
            let result = imap
                .set_flags(&folder_remote_id, uid, is_read, is_starred)
                .await;
            let _ = imap.disconnect().await;
            result
        }
    }
}

async fn replay_remote_archive(
    state: &AppState,
    provider_type: ProviderType,
    message: &pebble_core::Message,
    payload: &Value,
) -> std::result::Result<(), PebbleError> {
    match provider_type {
        ProviderType::Gmail => {
            let add = string_array(payload, "add_labels").unwrap_or_default();
            let remove =
                string_array(payload, "remove_labels").unwrap_or_else(|| vec!["INBOX".to_string()]);
            connect_gmail(state, &message.account_id)
                .await?
                .modify_labels(&message.remote_id, &add, &remove)
                .await
        }
        ProviderType::Outlook => {
            let target_remote_id =
                target_folder_remote_id(state, message, payload, FolderRole::Archive)?;
            connect_outlook(state, &message.account_id)
                .await?
                .move_message(&message.remote_id, &target_remote_id)
                .await
        }
        ProviderType::Imap => {
            let source_remote_id = source_folder_remote_id(state, message, payload)?;
            let target_remote_id =
                target_folder_remote_id(state, message, payload, FolderRole::Archive)?;
            let uid = parse_uid(message)?;
            let imap = connect_imap(state, &message.account_id).await?;
            let result = imap
                .move_message(&source_remote_id, uid, &target_remote_id)
                .await;
            let _ = imap.disconnect().await;
            result
        }
    }
}

async fn replay_remote_restore(
    state: &AppState,
    provider_type: ProviderType,
    message: &pebble_core::Message,
    payload: &Value,
) -> std::result::Result<(), PebbleError> {
    match provider_type {
        ProviderType::Gmail => {
            let current_folder = find_message_folder(state, &message.id, &message.account_id).ok();
            let provider = connect_gmail(state, &message.account_id).await?;
            if current_folder
                .as_ref()
                .is_some_and(|folder| folder.role == Some(FolderRole::Trash))
            {
                provider.untrash_message(&message.remote_id).await
            } else {
                provider
                    .modify_labels(&message.remote_id, &["INBOX".to_string()], &[])
                    .await
            }
        }
        ProviderType::Outlook => {
            if let Some(target_remote_id) = string_field(payload, "target_folder_remote_id") {
                connect_outlook(state, &message.account_id)
                    .await?
                    .move_message(&message.remote_id, &target_remote_id)
                    .await
            } else {
                connect_outlook(state, &message.account_id)
                    .await?
                    .restore_message(&message.remote_id)
                    .await
            }
        }
        ProviderType::Imap => {
            let source_remote_id = source_folder_remote_id(state, message, payload)?;
            let target_remote_id =
                target_folder_remote_id(state, message, payload, FolderRole::Inbox)?;
            let uid = parse_uid(message)?;
            let imap = connect_imap(state, &message.account_id).await?;
            let result = imap
                .move_message(&source_remote_id, uid, &target_remote_id)
                .await;
            let _ = imap.disconnect().await;
            result
        }
    }
}

async fn replay_remote_delete(
    state: &AppState,
    provider_type: ProviderType,
    message: &pebble_core::Message,
    payload: &Value,
    permanent: bool,
) -> std::result::Result<(), PebbleError> {
    match provider_type {
        ProviderType::Gmail => {
            let provider = connect_gmail(state, &message.account_id).await?;
            if permanent {
                provider
                    .delete_message_permanently(&message.remote_id)
                    .await
            } else {
                provider.trash_message(&message.remote_id).await
            }
        }
        ProviderType::Outlook => {
            let provider = connect_outlook(state, &message.account_id).await?;
            if permanent {
                provider
                    .delete_message_permanently(&message.remote_id)
                    .await
            } else {
                provider.trash_message(&message.remote_id).await
            }
        }
        ProviderType::Imap => {
            let source_remote_id = source_folder_remote_id(state, message, payload)?;
            let uid = parse_uid(message)?;
            let imap = connect_imap(state, &message.account_id).await?;
            let result = if permanent {
                imap.delete_message(&source_remote_id, uid).await
            } else if let Some(trash_remote_id) = string_field(payload, "trash_folder_remote_id")
                .or_else(|| {
                    find_folder_by_role(state, &message.account_id, FolderRole::Trash)
                        .ok()
                        .map(|folder| folder.remote_id)
                })
            {
                if trash_remote_id == source_remote_id {
                    imap.delete_message(&source_remote_id, uid).await
                } else {
                    imap.move_message(&source_remote_id, uid, &trash_remote_id)
                        .await
                }
            } else {
                imap.delete_message(&source_remote_id, uid).await
            };
            let _ = imap.disconnect().await;
            result
        }
    }
}

async fn replay_remote_move_to_folder(
    state: &AppState,
    provider_type: ProviderType,
    message: &pebble_core::Message,
    payload: &Value,
) -> std::result::Result<(), PebbleError> {
    match provider_type {
        ProviderType::Gmail => {
            let add = string_array(payload, "add_labels").unwrap_or_else(|| {
                string_field(payload, "target_folder_remote_id")
                    .into_iter()
                    .collect()
            });
            let remove =
                string_array(payload, "remove_labels").unwrap_or_else(|| vec!["INBOX".to_string()]);
            connect_gmail(state, &message.account_id)
                .await?
                .modify_labels(&message.remote_id, &add, &remove)
                .await
        }
        ProviderType::Outlook => {
            let target_remote_id =
                target_folder_remote_id(state, message, payload, FolderRole::Inbox)?;
            connect_outlook(state, &message.account_id)
                .await?
                .move_message(&message.remote_id, &target_remote_id)
                .await
        }
        ProviderType::Imap => {
            let source_remote_id = source_folder_remote_id(state, message, payload)?;
            let target_remote_id =
                target_folder_remote_id(state, message, payload, FolderRole::Inbox)?;
            let uid = parse_uid(message)?;
            let imap = connect_imap(state, &message.account_id).await?;
            let result = imap
                .move_message(&source_remote_id, uid, &target_remote_id)
                .await;
            let _ = imap.disconnect().await;
            result
        }
    }
}

async fn replay_remote_send(
    state: &AppState,
    provider_type: ProviderType,
    account: &pebble_core::Account,
    message: &pebble_core::Message,
) -> std::result::Result<(), PebbleError> {
    let attachment_paths = state
        .store
        .list_attachments_by_message(&message.id)?
        .into_iter()
        .filter_map(|attachment| attachment.local_path)
        .collect::<Vec<_>>();
    let outgoing = compose::outgoing_message_from_stored(message, attachment_paths);

    match provider_type {
        ProviderType::Gmail => {
            connect_gmail(state, &account.id)
                .await?
                .send_message(&outgoing)
                .await
        }
        ProviderType::Outlook => {
            connect_outlook(state, &account.id)
                .await?
                .send_message(&outgoing)
                .await
        }
        ProviderType::Imap => compose::send_imap_smtp_message(state, account, &outgoing).await,
    }
}

fn apply_pending_local_commit(
    store: &Store,
    op: &PendingMailOp,
) -> std::result::Result<(), PebbleError> {
    let payload = op_payload(op)?;
    match op.op_type.as_str() {
        "update_flags" => {
            store.update_message_flags(
                &op.message_id,
                optional_bool(&payload, "is_read"),
                optional_bool(&payload, "is_starred"),
            )?;
        }
        "archive" => {
            if let Some(folder_id) = string_field(&payload, "target_folder_id")
                .or_else(|| string_field(&payload, "archive_folder_id"))
                .or_else(|| {
                    store
                        .find_folder_by_role(&op.account_id, FolderRole::Archive)
                        .ok()
                        .flatten()
                        .map(|folder| folder.id)
                })
            {
                store.move_message_to_folder(&op.message_id, &folder_id)?;
            } else {
                store.soft_delete_message(&op.message_id)?;
            }
        }
        "unarchive" | "restore" => {
            let folder_id = string_field(&payload, "target_folder_id")
                .or_else(|| {
                    store
                        .find_folder_by_role(&op.account_id, FolderRole::Inbox)
                        .ok()
                        .flatten()
                        .map(|folder| folder.id)
                })
                .ok_or_else(|| PebbleError::Internal("No restore target folder".to_string()))?;
            store.move_message_to_folder(&op.message_id, &folder_id)?;
        }
        "delete" => {
            if let Some(trash_folder_id) = string_field(&payload, "trash_folder_id") {
                let current = store
                    .get_message_folder_ids(&op.message_id)?
                    .into_iter()
                    .next();
                if current.as_deref() != Some(trash_folder_id.as_str()) {
                    store.move_message_to_folder(&op.message_id, &trash_folder_id)?;
                } else {
                    store.soft_delete_message(&op.message_id)?;
                }
            } else if let Some(trash) =
                store.find_folder_by_role(&op.account_id, FolderRole::Trash)?
            {
                store.move_message_to_folder(&op.message_id, &trash.id)?;
            } else {
                store.soft_delete_message(&op.message_id)?;
            }
        }
        "delete_permanent" => {
            store.hard_delete_messages(std::slice::from_ref(&op.message_id))?;
        }
        "move_to_folder" => {
            let folder_id = string_field(&payload, "target_folder_id")
                .ok_or_else(|| PebbleError::Internal("No move target folder".to_string()))?;
            store.move_message_to_folder(&op.message_id, &folder_id)?;
        }
        "send" => {
            let sent = compose::ensure_local_outgoing_folder(
                store,
                &op.account_id,
                LocalOutgoingState::Sent,
            )?;
            store.move_message_to_folder(&op.message_id, &sent.id)?;
        }
        other => {
            return Err(PebbleError::Internal(format!(
                "Unsupported pending mail op type: {other}"
            )));
        }
    }
    Ok(())
}

fn refresh_after_pending_commit(
    state: &AppState,
    op: &PendingMailOp,
) -> std::result::Result<(), PebbleError> {
    if op.op_type == "delete_permanent" {
        remove_search_documents(state, std::slice::from_ref(&op.message_id))
    } else {
        refresh_search_document(state, &op.message_id)
    }
}

fn op_payload(op: &PendingMailOp) -> std::result::Result<Value, PebbleError> {
    let value: Value = serde_json::from_str(&op.payload_json)
        .map_err(|e| PebbleError::Internal(format!("Invalid pending op payload: {e}")))?;
    Ok(value.get("payload").cloned().unwrap_or(value))
}

fn string_field(payload: &Value, key: &str) -> Option<String> {
    payload
        .get(key)
        .and_then(Value::as_str)
        .map(ToOwned::to_owned)
}

fn optional_bool(payload: &Value, key: &str) -> Option<bool> {
    payload.get(key).and_then(Value::as_bool)
}

fn string_array(payload: &Value, key: &str) -> Option<Vec<String>> {
    payload.get(key).and_then(|value| {
        value.as_array().map(|items| {
            items
                .iter()
                .filter_map(Value::as_str)
                .map(ToOwned::to_owned)
                .collect()
        })
    })
}

fn source_folder_remote_id(
    state: &AppState,
    message: &pebble_core::Message,
    payload: &Value,
) -> std::result::Result<String, PebbleError> {
    string_field(payload, "source_folder_remote_id")
        .or_else(|| {
            find_message_folder(state, &message.id, &message.account_id)
                .ok()
                .map(|folder| folder.remote_id)
        })
        .ok_or_else(|| PebbleError::Internal("No source folder for pending op".to_string()))
}

fn target_folder_remote_id(
    state: &AppState,
    message: &pebble_core::Message,
    payload: &Value,
    fallback_role: FolderRole,
) -> std::result::Result<String, PebbleError> {
    string_field(payload, "target_folder_remote_id")
        .or_else(|| string_field(payload, "archive_folder_remote_id"))
        .or_else(|| string_field(payload, "trash_folder_remote_id"))
        .or_else(|| {
            string_field(payload, "target_folder_id")
                .or_else(|| string_field(payload, "archive_folder_id"))
                .and_then(|folder_id| {
                    state
                        .store
                        .list_folders(&message.account_id)
                        .ok()?
                        .into_iter()
                        .find(|folder| folder.id == folder_id)
                        .map(|folder| folder.remote_id)
                })
        })
        .or_else(|| {
            find_folder_by_role(state, &message.account_id, fallback_role)
                .ok()
                .map(|folder| folder.remote_id)
        })
        .ok_or_else(|| PebbleError::Internal("No target folder for pending op".to_string()))
}

fn parse_uid(message: &pebble_core::Message) -> std::result::Result<u32, PebbleError> {
    message
        .remote_id
        .parse::<u32>()
        .map_err(|e| PebbleError::Internal(format!("Invalid IMAP UID: {e}")))
}

#[cfg(test)]
mod tests {
    use super::apply_pending_local_commit;
    use pebble_core::*;
    use pebble_store::Store;

    fn test_account() -> Account {
        let now = now_timestamp();
        Account {
            id: new_id(),
            email: "test@example.com".to_string(),
            display_name: "Test".to_string(),
            provider: ProviderType::Gmail,
            created_at: now,
            updated_at: now,
        }
    }

    fn test_folder(account_id: &str, role: FolderRole, remote_id: &str, name: &str) -> Folder {
        Folder {
            id: new_id(),
            account_id: account_id.to_string(),
            remote_id: remote_id.to_string(),
            name: name.to_string(),
            folder_type: FolderType::Folder,
            role: Some(role),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 0,
        }
    }

    fn test_message(account_id: &str) -> Message {
        let now = now_timestamp();
        Message {
            id: new_id(),
            account_id: account_id.to_string(),
            remote_id: "remote-123".to_string(),
            message_id_header: None,
            in_reply_to: None,
            references_header: None,
            thread_id: None,
            subject: "Test".to_string(),
            snippet: "test".to_string(),
            from_address: "from@example.com".to_string(),
            from_name: "From".to_string(),
            to_list: vec![],
            cc_list: vec![],
            bcc_list: vec![],
            body_text: "body".to_string(),
            body_html_raw: String::new(),
            has_attachments: false,
            is_read: false,
            is_starred: false,
            is_draft: false,
            date: now,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn local_commit_replays_archived_pending_op_after_remote_success() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let inbox = test_folder(&account.id, FolderRole::Inbox, "INBOX", "Inbox");
        let archive = test_folder(&account.id, FolderRole::Archive, "ARCHIVE", "Archive");
        store.insert_folder(&inbox).unwrap();
        store.insert_folder(&archive).unwrap();
        let message = test_message(&account.id);
        store
            .insert_message(&message, std::slice::from_ref(&inbox.id))
            .unwrap();

        let op_id = store
            .insert_pending_mail_op(
                &account.id,
                &message.id,
                "archive",
                &serde_json::json!({
                    "remote_id": message.remote_id,
                    "op": "archive",
                    "payload": {
                        "source_folder_id": inbox.id,
                        "target_folder_id": archive.id,
                    }
                })
                .to_string(),
            )
            .unwrap();
        let op = store.list_pending_mail_ops(&account.id).unwrap().remove(0);

        apply_pending_local_commit(&store, &op).unwrap();

        let folder_ids = store.get_message_folder_ids(&message.id).unwrap();
        assert_eq!(folder_ids, vec![archive.id]);
        store.mark_pending_mail_op_done(&op_id).unwrap();
    }
}
