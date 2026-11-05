use crate::commands::attachments::cleanup_staged_attachment_records;
use crate::{events, state::AppState};
use pebble_core::traits::{FolderProvider, LabelProvider};
use pebble_core::{FolderRole, Message, PebbleError, ProviderType};
use pebble_store::pending_ops::{PendingMailOp, PendingMailOpsSummary};
use pebble_store::Store;
use serde::Serialize;
use serde_json::Value;
use tauri::{Emitter, Manager, State};
use tracing::{debug, warn};

use super::compose::{self, LocalOutgoingState};
use super::messages::{
    classify_remote_delete_result, connect_gmail, connect_imap, connect_outlook,
    find_folder_by_role, find_message_folder, imap_move_tracked, refresh_search_document,
    remove_search_documents,
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
            Err(ReplayPendingMailOpError::Retryable(ref e)) if is_permanent_error(e) => {
                state.store.mark_pending_mail_op_done(&op.id)?;
                warn!(
                    "Pending mail op {} permanently failed (non-retryable): {e}",
                    op.id
                );
                completed += 1;
            }
            Err(ReplayPendingMailOpError::RemoteSendOutcomeUnknown(e)) => {
                let error = compose::send_outcome_unknown_message(&e);
                // If this write also fails, leave the row in_progress. Startup
                // recovery treats interrupted sends as outcome-unknown instead
                // of ever putting them back on the automatic retry queue.
                state
                    .store
                    .mark_pending_mail_op_outcome_unknown(&op.id, &error)?;
                warn!("Pending mail op {} has an unknown send outcome: {e}", op.id);
            }
            Err(ReplayPendingMailOpError::Retryable(e)) => {
                state
                    .store
                    .mark_pending_mail_op_failed(&op.id, &e.to_string())?;
                warn!("Pending mail op {} retry failed: {e}", op.id);
            }
            Err(ReplayPendingMailOpError::SenderIdentityBlocked(e)) => {
                state.store.mark_pending_mail_op_stopped(
                    &op.id,
                    &format!("Sender identity requires attention; no message was sent. {e}"),
                )?;
            }
        }
    }

    if changed {
        emit_pending_ops_changed(app);
    }
    Ok(completed)
}

#[derive(Debug)]
enum ReplayPendingMailOpError {
    SenderIdentityBlocked(PebbleError),
    Retryable(PebbleError),
    RemoteSendOutcomeUnknown(PebbleError),
}

impl From<PebbleError> for ReplayPendingMailOpError {
    fn from(error: PebbleError) -> Self {
        Self::Retryable(error)
    }
}

fn classify_remote_send_receipt_write(
    result: std::result::Result<(), PebbleError>,
) -> std::result::Result<(), ReplayPendingMailOpError> {
    result.map_err(ReplayPendingMailOpError::RemoteSendOutcomeUnknown)
}

fn classify_remote_send_call(
    result: std::result::Result<(), PebbleError>,
) -> std::result::Result<(), ReplayPendingMailOpError> {
    match result {
        Err(error @ PebbleError::Network(_)) => {
            Err(ReplayPendingMailOpError::RemoteSendOutcomeUnknown(error))
        }
        Err(error) => Err(ReplayPendingMailOpError::Retryable(error)),
        Ok(()) => Ok(()),
    }
}

fn is_permanent_error(e: &PebbleError) -> bool {
    matches!(e, PebbleError::UnsupportedProvider(_))
}

#[tauri::command]
pub fn dismiss_failed_pending_mail_ops(
    state: State<'_, AppState>,
    app: tauri::AppHandle,
    account_id: Option<String>,
) -> std::result::Result<u64, PebbleError> {
    let dismissed = state
        .store
        .dismiss_failed_pending_mail_ops(account_id.as_deref())?;
    if dismissed > 0 {
        emit_pending_ops_changed(Some(&app));
    }
    Ok(dismissed)
}

fn emit_pending_ops_changed(app: Option<&tauri::AppHandle>) {
    if let Some(app) = app {
        let _ = app.emit(events::MAIL_PENDING_OPS_CHANGED, ());
    }
}

async fn replay_pending_mail_op(
    state: &AppState,
    op: &PendingMailOp,
) -> std::result::Result<(), ReplayPendingMailOpError> {
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
            if !remote_side_effect_already_applied(op)? {
                replay_remote_send(state, account.provider.clone(), &account, &message).await?;
                classify_remote_send_receipt_write(
                    state.store.mark_pending_mail_op_remote_succeeded(&op.id),
                )?;
            }
        }
        other => {
            return Err(ReplayPendingMailOpError::Retryable(PebbleError::Internal(
                format!("Unsupported pending mail op type: {other}"),
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
        ProviderType::Pop3 => Ok(()),
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
            let new_remote_id = connect_outlook(state, &message.account_id)
                .await?
                .move_message(&message.remote_id, &target_remote_id)
                .await?;
            state.store.update_remote_id(&message.id, &new_remote_id)?;
            Ok(())
        }
        ProviderType::Imap => {
            let source_remote_id = source_folder_remote_id(state, message, payload)?;
            let target_remote_id =
                target_folder_remote_id(state, message, payload, FolderRole::Archive)?;
            let uid = parse_uid(message)?;
            let imap = connect_imap(state, &message.account_id).await?;
            let result = imap_move_tracked(
                state,
                &imap,
                message,
                &source_remote_id,
                uid,
                &target_remote_id,
            )
            .await;
            let _ = imap.disconnect().await;
            result
        }
        ProviderType::Pop3 => Ok(()),
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
            let new_remote_id =
                if let Some(target_remote_id) = string_field(payload, "target_folder_remote_id") {
                    connect_outlook(state, &message.account_id)
                        .await?
                        .move_message(&message.remote_id, &target_remote_id)
                        .await?
                } else {
                    connect_outlook(state, &message.account_id)
                        .await?
                        .restore_message(&message.remote_id)
                        .await?
                };
            state.store.update_remote_id(&message.id, &new_remote_id)?;
            Ok(())
        }
        ProviderType::Imap => {
            let source_remote_id = source_folder_remote_id(state, message, payload)?;
            let target_remote_id =
                target_folder_remote_id(state, message, payload, FolderRole::Inbox)?;
            let uid = parse_uid(message)?;
            let imap = connect_imap(state, &message.account_id).await?;
            let result = imap_move_tracked(
                state,
                &imap,
                message,
                &source_remote_id,
                uid,
                &target_remote_id,
            )
            .await;
            let _ = imap.disconnect().await;
            result
        }
        ProviderType::Pop3 => Ok(()),
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
            let result = if permanent {
                provider
                    .delete_message_permanently(&message.remote_id)
                    .await
            } else {
                provider.trash_message(&message.remote_id).await
            };
            classify_remote_delete_result(result, permanent)
        }
        ProviderType::Outlook => {
            let provider = connect_outlook(state, &message.account_id).await?;
            if permanent {
                classify_remote_delete_result(
                    provider
                        .delete_message_permanently(&message.remote_id)
                        .await,
                    true,
                )
            } else {
                let new_remote_id = provider.trash_message(&message.remote_id).await?;
                state.store.update_remote_id(&message.id, &new_remote_id)?;
                Ok(())
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
                    imap.move_message_with_dedup(
                        &source_remote_id,
                        uid,
                        &trash_remote_id,
                        message.message_id_header.as_deref(),
                    )
                    .await
                }
            } else {
                imap.delete_message(&source_remote_id, uid).await
            };
            let _ = imap.disconnect().await;
            classify_remote_delete_result(result, permanent)
        }
        ProviderType::Pop3 => Ok(()),
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
            let new_remote_id = connect_outlook(state, &message.account_id)
                .await?
                .move_message(&message.remote_id, &target_remote_id)
                .await?;
            state.store.update_remote_id(&message.id, &new_remote_id)?;
            Ok(())
        }
        ProviderType::Imap => {
            let source_remote_id = source_folder_remote_id(state, message, payload)?;
            let target_remote_id =
                target_folder_remote_id(state, message, payload, FolderRole::Inbox)?;
            let uid = parse_uid(message)?;
            let imap = connect_imap(state, &message.account_id).await?;
            let result = imap_move_tracked(
                state,
                &imap,
                message,
                &source_remote_id,
                uid,
                &target_remote_id,
            )
            .await;
            let _ = imap.disconnect().await;
            result
        }
        ProviderType::Pop3 => Ok(()),
    }
}

async fn replay_remote_send(
    state: &AppState,
    _provider_type: ProviderType,
    account: &pebble_core::Account,
    message: &pebble_core::Message,
) -> std::result::Result<(), ReplayPendingMailOpError> {
    let attachment_paths = state
        .store
        .list_attachments_by_message(&message.id)?
        .into_iter()
        .filter_map(|attachment| attachment.local_path)
        .collect::<Vec<_>>();
    let outgoing = compose::outgoing_message_from_stored(message, attachment_paths);
    pebble_mail::sender::sender_mailbox(&outgoing.from)
        .map_err(ReplayPendingMailOpError::SenderIdentityBlocked)?;
    // Preparation errors are pre-dispatch and must never become outcome-unknown.
    let (transport, verified_from) = compose::prepare_send_transport(state, account)
        .await
        .map_err(|error| {
            if matches!(error, PebbleError::Validation(_)) {
                ReplayPendingMailOpError::SenderIdentityBlocked(error)
            } else {
                ReplayPendingMailOpError::Retryable(error)
            }
        })?;
    replay_prepared_send(&outgoing, &verified_from, || transport.send(&outgoing)).await
}

async fn replay_prepared_send<F, Fut>(
    outgoing: &pebble_core::traits::OutgoingMessage,
    verified_from: &pebble_core::EmailAddress,
    send: F,
) -> std::result::Result<(), ReplayPendingMailOpError>
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = std::result::Result<(), PebbleError>>,
{
    if !outgoing
        .from
        .address
        .eq_ignore_ascii_case(&verified_from.address)
    {
        return Err(ReplayPendingMailOpError::SenderIdentityBlocked(PebbleError::Validation(
            "The queued message belongs to a different mailbox. Review it and compose a new message if needed.".into(),
        )));
    }
    classify_remote_send_call(send().await)
}

fn apply_pending_local_commit(
    store: &Store,
    op: &PendingMailOp,
) -> std::result::Result<(), PebbleError> {
    let payload = op_payload(op)?;
    let placeholder_attachments =
        if op.op_type == "send" && compose::send_finalize_deletes_placeholder(&payload) {
            store.list_attachments_by_message(&op.message_id)?
        } else {
            Vec::new()
        };
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
            if compose::send_finalize_deletes_placeholder(&payload) {
                store.complete_outgoing_send(&op.message_id, &op.id, None)?;
            } else {
                let sent_folder_id = string_field(&payload, "sent_folder_id")
                    .map(Ok)
                    .unwrap_or_else(|| {
                        compose::ensure_local_outgoing_folder(
                            store,
                            &op.account_id,
                            LocalOutgoingState::Sent,
                        )
                        .map(|folder| folder.id)
                    })?;
                store.complete_outgoing_send(&op.message_id, &op.id, Some(&sent_folder_id))?;
            }
        }
        other => {
            return Err(PebbleError::Internal(format!(
                "Unsupported pending mail op type: {other}"
            )));
        }
    }
    cleanup_staged_attachment_records(&placeholder_attachments);
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

fn remote_side_effect_already_applied(
    op: &PendingMailOp,
) -> std::result::Result<bool, PebbleError> {
    let value: Value = serde_json::from_str(&op.payload_json)
        .map_err(|e| PebbleError::Internal(format!("Invalid pending op payload: {e}")))?;
    Ok(value
        .get("remote_succeeded")
        .and_then(Value::as_bool)
        .unwrap_or(false))
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
    use super::{
        apply_pending_local_commit, classify_remote_send_call, classify_remote_send_receipt_write,
        is_permanent_error, remote_side_effect_already_applied, ReplayPendingMailOpError,
    };
    use crate::commands::compose;
    use crate::commands::messages::classify_remote_delete_result;
    use pebble_core::*;
    use pebble_store::Store;

    #[tokio::test]
    async fn queued_send_stops_before_dispatch_after_mailbox_address_changes() {
        for provider in [ProviderType::Imap, ProviderType::Pop3] {
            let store = Store::open_in_memory().unwrap();
            let mut account = test_account();
            account.provider = provider;
            store.insert_account(&account).unwrap();
            let mut message = test_message(&account.id);
            message.from_address = account.email.clone();
            message.from_name = "Original sender".into();
            store.insert_message(&message, &[]).unwrap();
            store
                .update_account_details(&account.id, "other@example.com", "New sender", None, None)
                .unwrap();
            let changed = store.get_account(&account.id).unwrap().unwrap();
            let stored = store.get_message(&message.id).unwrap().unwrap();
            let outgoing = compose::outgoing_message_from_stored(&stored, vec![]);
            let calls = std::cell::Cell::new(0);
            let result =
                super::replay_prepared_send(&outgoing, &changed.sender_identity(), || async {
                    calls.set(calls.get() + 1);
                    Ok(())
                })
                .await;
            assert!(matches!(
                result,
                Err(ReplayPendingMailOpError::SenderIdentityBlocked(_))
            ));
            assert_eq!(calls.get(), 0);
        }
    }

    #[tokio::test]
    async fn queued_send_keeps_original_name_when_only_name_changes() {
        let mut account = test_account();
        account.provider = ProviderType::Imap;
        let mut message = test_message(&account.id);
        message.from_address = account.email.to_uppercase();
        message.from_name = "Original sender".into();
        account.display_name = "New sender".into();
        let outgoing = compose::outgoing_message_from_stored(&message, vec![]);
        let calls = std::cell::Cell::new(0);
        super::replay_prepared_send(&outgoing, &account.sender_identity(), || async {
            calls.set(calls.get() + 1);
            assert_eq!(outgoing.from.name.as_deref(), Some("Original sender"));
            Ok(())
        })
        .await
        .unwrap();
        assert_eq!(calls.get(), 1);
    }

    fn test_account() -> Account {
        let now = now_timestamp();
        Account {
            account_label: None,
            provider_display_name: None,
            id: new_id(),
            email: "test@example.com".to_string(),
            display_name: "Test".to_string(),
            color: None,
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
    fn authentication_errors_remain_retryable_after_reauthentication() {
        assert!(!is_permanent_error(&PebbleError::Auth(
            "token expired".to_string()
        )));
        assert!(is_permanent_error(&PebbleError::UnsupportedProvider(
            "unknown".to_string()
        )));
    }

    #[test]
    fn remote_send_receipt_write_failure_is_never_classified_as_retryable() {
        let result =
            classify_remote_send_receipt_write(Err(PebbleError::Storage("disk full".to_string())));
        assert!(matches!(
            result,
            Err(ReplayPendingMailOpError::RemoteSendOutcomeUnknown(_))
        ));
    }

    #[test]
    fn network_send_call_failure_is_never_classified_as_retryable() {
        let result = classify_remote_send_call(Err(PebbleError::Network(
            "connection closed after DATA".to_string(),
        )));
        assert!(matches!(
            result,
            Err(ReplayPendingMailOpError::RemoteSendOutcomeUnknown(_))
        ));

        let rejected = classify_remote_send_call(Err(PebbleError::Auth(
            "server rejected the sender".to_string(),
        )));
        assert!(matches!(
            rejected,
            Err(ReplayPendingMailOpError::Retryable(_))
        ));
    }

    #[test]
    fn pending_permanent_delete_treats_remote_absence_as_success() {
        let absent = classify_remote_delete_result(
            Err(PebbleError::Network(
                "Failed to permanently delete message (status 404 Not Found)".to_string(),
            )),
            true,
        );
        assert!(absent.is_ok());

        let non_permanent = classify_remote_delete_result(
            Err(PebbleError::Network("status 404 Not Found".to_string())),
            false,
        );
        assert!(non_permanent.is_err());
    }

    #[test]
    fn completed_remote_send_is_not_replayed_when_local_finalize_retries() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let inbox = test_folder(&account.id, FolderRole::Inbox, "INBOX", "Inbox");
        store.insert_folder(&inbox).unwrap();
        let message = test_message(&account.id);
        store.insert_message(&message, &[inbox.id]).unwrap();
        let op_id = store
            .insert_pending_mail_op(
                &account.id,
                &message.id,
                "send",
                r#"{"op":"send","payload":{}}"#,
            )
            .unwrap();
        let before = store.list_pending_mail_ops(&account.id).unwrap().remove(0);
        assert!(!remote_side_effect_already_applied(&before).unwrap());

        store.mark_pending_mail_op_remote_succeeded(&op_id).unwrap();
        let after = store.list_pending_mail_ops(&account.id).unwrap().remove(0);
        assert!(remote_side_effect_already_applied(&after).unwrap());
    }

    #[test]
    fn remote_succeeded_api_send_recovery_deletes_local_placeholder() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let outbox = test_folder(&account.id, FolderRole::Inbox, "__local_outbox__", "Outbox");
        store.insert_folder(&outbox).unwrap();
        let message = test_message(&account.id);
        store
            .insert_message(&message, std::slice::from_ref(&outbox.id))
            .unwrap();
        let attachment_dir = std::env::temp_dir().join(format!("pebble-api-replay-{}", new_id()));
        std::fs::create_dir_all(&attachment_dir).unwrap();
        let attachment_path = attachment_dir.join("placeholder.txt");
        std::fs::write(&attachment_path, b"placeholder").unwrap();
        store
            .insert_attachment(&Attachment {
                id: new_id(),
                message_id: message.id.clone(),
                filename: "placeholder.txt".to_string(),
                mime_type: "text/plain".to_string(),
                size: 11,
                local_path: Some(attachment_path.to_string_lossy().into_owned()),
                content_id: None,
                is_inline: false,
            })
            .unwrap();
        let op_id = store
            .insert_pending_mail_op(
                &account.id,
                &message.id,
                "send",
                &serde_json::json!({
                    "op": "send",
                    "payload": {
                        "local_finalize": compose::SEND_FINALIZE_DELETE_PLACEHOLDER,
                    }
                })
                .to_string(),
            )
            .unwrap();
        store.mark_pending_mail_op_in_progress(&op_id).unwrap();
        store.mark_pending_mail_op_remote_succeeded(&op_id).unwrap();

        // Crash after the remote receipt but before local finalize. Startup
        // safely requeues because remote_succeeded prevents another send.
        store.reset_in_progress_pending_mail_ops().unwrap();
        let op = store.list_pending_mail_ops(&account.id).unwrap().remove(0);
        assert!(remote_side_effect_already_applied(&op).unwrap());
        apply_pending_local_commit(&store, &op).unwrap();

        assert!(store.get_message(&message.id).unwrap().is_none());
        assert!(store.list_pending_mail_ops(&account.id).unwrap().is_empty());
        assert!(!attachment_path.exists());
        assert!(!attachment_dir.exists());
    }

    #[test]
    fn remote_succeeded_smtp_send_recovery_moves_placeholder_to_sent() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let outbox = test_folder(&account.id, FolderRole::Inbox, "__local_outbox__", "Outbox");
        let sent = test_folder(&account.id, FolderRole::Sent, "__local_sent__", "Sent");
        store.insert_folder(&outbox).unwrap();
        store.insert_folder(&sent).unwrap();
        let message = test_message(&account.id);
        store
            .insert_message(&message, std::slice::from_ref(&outbox.id))
            .unwrap();
        let op_id = store
            .insert_pending_mail_op(
                &account.id,
                &message.id,
                "send",
                &serde_json::json!({
                    "op": "send",
                    "payload": {
                        "local_finalize": compose::SEND_FINALIZE_MOVE_TO_SENT,
                        "sent_folder_id": sent.id,
                    }
                })
                .to_string(),
            )
            .unwrap();
        store.mark_pending_mail_op_in_progress(&op_id).unwrap();
        store.mark_pending_mail_op_remote_succeeded(&op_id).unwrap();
        store.reset_in_progress_pending_mail_ops().unwrap();
        let op = store.list_pending_mail_ops(&account.id).unwrap().remove(0);

        apply_pending_local_commit(&store, &op).unwrap();

        assert!(store.get_message(&message.id).unwrap().is_some());
        assert_eq!(
            store.get_message_folder_ids(&message.id).unwrap(),
            vec![sent.id]
        );
        let op = store.list_pending_mail_ops(&account.id).unwrap().remove(0);
        assert_eq!(op.status.as_str(), "done");
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
