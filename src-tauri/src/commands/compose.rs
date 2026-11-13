use std::path::{Path, PathBuf};

use crate::commands::attachments::{
    cleanup_staged_attachment_records, sanitize_stored_filename, stage_local_attachment_records,
};
use crate::commands::encrypted_store::load_account_auth_data;
use crate::commands::messages::refresh_search_document;
use crate::commands::network::{account_proxy_mode_from_auth_value, resolve_mail_proxy_from_mode};
use crate::commands::oauth::ensure_account_oauth_auth;
use crate::{events, state::AppState};
use pebble_core::traits::{MailTransport, OutgoingMessage};
use pebble_core::{
    new_id, now_timestamp, Account, EmailAddress, Folder, FolderRole, FolderType, Message,
    PebbleError, ProviderType,
};
use pebble_crypto::CryptoService;
use pebble_mail::{smtp::SmtpSender, SmtpConfig};
use pebble_mail::{GmailProvider, OutlookProvider};
use pebble_store::Store;
use tauri::{Emitter, State};
use tracing::warn;

/// Validate that all attachment paths are within allowed directories.
pub(crate) fn validate_attachment_paths(
    paths: &[String],
    attachments_dir: &std::path::Path,
) -> std::result::Result<Vec<String>, PebbleError> {
    let mut allowed_dirs: Vec<PathBuf> = vec![attachments_dir.to_path_buf()];

    // Add user home subdirectories (Documents, Downloads, Desktop) and temp dir
    if let Ok(home) = std::env::var("USERPROFILE").or_else(|_| std::env::var("HOME")) {
        let home = PathBuf::from(home);
        for sub in &["Documents", "Downloads", "Desktop"] {
            let dir = home.join(sub);
            if dir.exists() {
                allowed_dirs.push(dir);
            }
        }
    }
    if let Ok(tmp) = std::env::temp_dir().canonicalize() {
        allowed_dirs.push(tmp);
    }

    // Canonicalize allowed dirs for consistent comparison
    let allowed_dirs: Vec<PathBuf> = allowed_dirs
        .into_iter()
        .filter_map(|d| std::fs::canonicalize(&d).ok())
        .collect();

    let mut validated = Vec::with_capacity(paths.len());
    for raw_path in paths {
        let canonical = std::fs::canonicalize(raw_path).map_err(|e| {
            PebbleError::Internal(format!("Attachment path not found: {raw_path} ({e})"))
        })?;

        let is_allowed = allowed_dirs.iter().any(|dir| canonical.starts_with(dir));
        if !is_allowed {
            return Err(PebbleError::Internal(format!(
                "Attachment path is outside allowed directories: {}",
                canonical.display()
            )));
        }
        validated.push(canonical.to_string_lossy().into_owned());
    }
    Ok(validated)
}

fn parse_recipients(addresses: Vec<String>) -> Vec<EmailAddress> {
    addresses
        .into_iter()
        .map(|address| EmailAddress {
            name: None,
            address: address.trim().to_string(),
        })
        .filter(|address| !address.address.is_empty())
        .collect()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LocalOutgoingState {
    Sent,
    Queued,
}

fn local_outgoing_folder_spec(
    state: LocalOutgoingState,
) -> (&'static str, &'static str, Option<FolderRole>, i32) {
    match state {
        LocalOutgoingState::Sent => ("__local_sent__", "Sent", Some(FolderRole::Sent), 2),
        LocalOutgoingState::Queued => ("__local_outbox__", "Outbox", None, 3),
    }
}

pub(crate) fn ensure_local_outgoing_folder(
    store: &Store,
    account_id: &str,
    state: LocalOutgoingState,
) -> std::result::Result<Folder, PebbleError> {
    if state == LocalOutgoingState::Sent {
        if let Some(folder) = store.find_folder_by_role(account_id, FolderRole::Sent)? {
            return Ok(folder);
        }
    }

    let (remote_id, name, role, sort_order) = local_outgoing_folder_spec(state);
    if let Some(folder) = store.find_folder_by_name(account_id, name)? {
        return Ok(folder);
    }

    let folder = Folder {
        id: new_id(),
        account_id: account_id.to_string(),
        remote_id: remote_id.to_string(),
        name: name.to_string(),
        folder_type: FolderType::Folder,
        role,
        parent_id: None,
        color: None,
        is_system: true,
        sort_order,
    };
    let id = store.insert_folder(&folder)?;
    Ok(Folder { id, ..folder })
}

#[cfg(test)]
fn save_outgoing_message_locally(
    store: &Store,
    account: &Account,
    outgoing: &OutgoingMessage,
    state: LocalOutgoingState,
    attachments_dir: &Path,
) -> std::result::Result<Message, PebbleError> {
    let folder = ensure_local_outgoing_folder(store, &account.id, state)?;
    let (message, attachments) = stage_outgoing_message(account, outgoing, state, attachments_dir)?;

    if let Err(error) = store.replace_message_with_attachments(
        &message,
        std::slice::from_ref(&folder.id),
        &attachments,
    ) {
        cleanup_staged_attachment_records(&attachments);
        return Err(error);
    }
    Ok(message)
}

fn stage_outgoing_message(
    account: &Account,
    outgoing: &OutgoingMessage,
    state: LocalOutgoingState,
    attachments_dir: &Path,
) -> std::result::Result<(Message, Vec<pebble_core::Attachment>), PebbleError> {
    let now = now_timestamp();
    let id = new_id();
    let prefix = match state {
        LocalOutgoingState::Sent => "local-sent",
        LocalOutgoingState::Queued => "local-outbox",
    };
    let attachments =
        stage_local_attachment_records(attachments_dir, &id, &outgoing.attachment_paths)?;
    let message = Message {
        id: id.clone(),
        account_id: account.id.clone(),
        remote_id: format!("{prefix}-{id}"),
        message_id_header: Some(format!("<{id}@pebble.local>")),
        in_reply_to: outgoing.in_reply_to.clone(),
        references_header: outgoing.in_reply_to.clone(),
        thread_id: None,
        subject: outgoing.subject.clone(),
        snippet: outgoing.body_text.chars().take(200).collect(),
        from_address: outgoing.from.address.clone(),
        from_name: outgoing.from.name.clone().unwrap_or_default(),
        to_list: outgoing.to.clone(),
        cc_list: outgoing.cc.clone(),
        bcc_list: outgoing.bcc.clone(),
        body_text: outgoing.body_text.clone(),
        body_html_raw: outgoing.body_html.clone().unwrap_or_default(),
        has_attachments: !attachments.is_empty(),
        is_read: true,
        is_starred: false,
        is_draft: false,
        date: now,
        remote_version: None,
        is_deleted: false,
        deleted_at: None,
        created_at: now,
        updated_at: now,
    };

    Ok((message, attachments))
}

pub(crate) fn outgoing_message_from_stored(
    message: &Message,
    attachment_paths: Vec<String>,
) -> OutgoingMessage {
    OutgoingMessage {
        from: EmailAddress {
            name: (!message.from_name.is_empty()).then(|| message.from_name.clone()),
            address: message.from_address.clone(),
        },
        to: message.to_list.clone(),
        cc: message.cc_list.clone(),
        bcc: message.bcc_list.clone(),
        subject: message.subject.clone(),
        body_text: message.body_text.clone(),
        body_html: if message.body_html_raw.is_empty() {
            None
        } else {
            Some(message.body_html_raw.clone())
        },
        in_reply_to: message.in_reply_to.clone(),
        attachment_paths,
    }
}

pub(crate) fn load_smtp_config(
    store: &Store,
    crypto: &CryptoService,
    account_id: &str,
) -> std::result::Result<SmtpConfig, PebbleError> {
    let decrypted = load_account_auth_data(crypto, store, account_id)?.ok_or_else(|| {
        PebbleError::Internal(format!("No auth data found for account {account_id}"))
    })?;
    let config: serde_json::Value = serde_json::from_slice(&decrypted)
        .map_err(|e| PebbleError::Internal(format!("Failed to parse decrypted config: {e}")))?;

    let mut smtp_config: SmtpConfig = serde_json::from_value(
        config
            .get("smtp")
            .cloned()
            .ok_or_else(|| PebbleError::Internal("No SMTP config in auth data".to_string()))?,
    )
    .map_err(|e| PebbleError::Internal(format!("Failed to deserialize SMTP config: {e}")))?;

    let proxy_mode = account_proxy_mode_from_auth_value(&config);
    smtp_config.proxy = resolve_mail_proxy_from_mode(crypto, store, proxy_mode, smtp_config.proxy)?;

    Ok(smtp_config)
}

async fn send_smtp_message(
    sender: &SmtpSender,
    outgoing: &OutgoingMessage,
) -> std::result::Result<(), PebbleError> {
    let to = outgoing
        .to
        .iter()
        .map(|addr| addr.address.clone())
        .collect::<Vec<_>>();
    let cc = outgoing
        .cc
        .iter()
        .map(|addr| addr.address.clone())
        .collect::<Vec<_>>();
    let bcc = outgoing
        .bcc
        .iter()
        .map(|addr| addr.address.clone())
        .collect::<Vec<_>>();

    sender
        .send(
            &outgoing.from,
            &to,
            &cc,
            &bcc,
            &outgoing.subject,
            &outgoing.body_text,
            outgoing.body_html.as_deref(),
            outgoing.in_reply_to.as_deref(),
            &outgoing.attachment_paths,
        )
        .await
}

fn send_call_outcome_is_unknown(error: &PebbleError) -> bool {
    matches!(error, PebbleError::Network(_))
}

pub(super) fn send_outcome_unknown_message(error: &PebbleError) -> String {
    format!(
        "Remote send outcome is unknown: {error}. Check Sent before dismissing or sending again."
    )
}

struct PreparedOutgoingSend {
    message: Message,
    attachments: Vec<pebble_core::Attachment>,
    op_id: String,
    sent_folder_id: Option<String>,
    delete_placeholder_after_send: bool,
}

pub(super) const SEND_FINALIZE_DELETE_PLACEHOLDER: &str = "delete_placeholder";
pub(super) const SEND_FINALIZE_MOVE_TO_SENT: &str = "move_to_sent";

pub(super) fn send_finalize_deletes_placeholder(payload: &serde_json::Value) -> bool {
    payload
        .get("local_finalize")
        .and_then(serde_json::Value::as_str)
        == Some(SEND_FINALIZE_DELETE_PLACEHOLDER)
}

fn prepare_outgoing_send_locally(
    state: &AppState,
    account: &Account,
    outgoing: &OutgoingMessage,
) -> std::result::Result<PreparedOutgoingSend, PebbleError> {
    let outbox =
        ensure_local_outgoing_folder(&state.store, &account.id, LocalOutgoingState::Queued)?;
    let delete_placeholder_after_send = matches!(
        account.provider,
        ProviderType::Gmail | ProviderType::Outlook
    );
    let sent_folder_id = if delete_placeholder_after_send {
        None
    } else {
        Some(ensure_local_outgoing_folder(&state.store, &account.id, LocalOutgoingState::Sent)?.id)
    };
    let (message, attachments) = stage_outgoing_message(
        account,
        outgoing,
        LocalOutgoingState::Queued,
        &state.attachments_dir,
    )?;
    let payload = serde_json::json!({
        "provider_account_id": message.account_id,
        "remote_id": message.remote_id,
        "op": "send",
        "payload": {
            "local_finalize": if delete_placeholder_after_send {
                SEND_FINALIZE_DELETE_PLACEHOLDER
            } else {
                SEND_FINALIZE_MOVE_TO_SENT
            },
            "sent_folder_id": sent_folder_id,
        },
    });
    let op_id = match state.store.prepare_outgoing_send(
        &message,
        std::slice::from_ref(&outbox.id),
        &attachments,
        &payload.to_string(),
    ) {
        Ok(op_id) => op_id,
        Err(error) => {
            cleanup_staged_attachment_records(&attachments);
            return Err(error);
        }
    };
    if let Err(e) = refresh_search_document(state, &message.id) {
        warn!(
            "Failed to index prepared outgoing message {}: {e}",
            message.id
        );
    }
    Ok(PreparedOutgoingSend {
        message,
        attachments,
        op_id,
        sent_folder_id,
        delete_placeholder_after_send,
    })
}

fn accept_post_dispatch_transition(
    result: std::result::Result<(), PebbleError>,
    op_id: &str,
    phase: &str,
) -> std::result::Result<(), PebbleError> {
    if let Err(error) = result {
        warn!(
            "Send operation {op_id} could not persist its {phase} transition: {error}; leaving the durable in-progress row for conservative startup recovery"
        );
    }
    Ok(())
}

fn definite_send_failure_command_result(
    provider_error: PebbleError,
    cleanup_error: Option<&PebbleError>,
    op_id: &str,
) -> std::result::Result<(), PebbleError> {
    if let Some(cleanup_error) = cleanup_error {
        warn!(
            "Send operation {op_id} had a definite provider failure, but local cleanup also failed: {cleanup_error}; preserving the original provider error so the draft remains retryable"
        );
    }
    Err(provider_error)
}

pub(crate) enum PreparedSendTransport {
    Gmail(GmailProvider),
    Outlook(OutlookProvider),
    Smtp(SmtpSender),
}

impl PreparedSendTransport {
    pub(crate) async fn send(
        &self,
        outgoing: &OutgoingMessage,
    ) -> std::result::Result<(), PebbleError> {
        match self {
            Self::Gmail(provider) => provider.send_message(outgoing).await,
            Self::Outlook(provider) => provider.send_message(outgoing).await,
            Self::Smtp(sender) => send_smtp_message(sender, outgoing).await,
        }
    }
}

pub(crate) async fn prepare_send_transport(
    state: &AppState,
    account: &Account,
) -> std::result::Result<(PreparedSendTransport, EmailAddress), PebbleError> {
    let mut from = account.sender_identity();
    pebble_mail::sender::sender_mailbox(&from)?;
    match account.provider {
        ProviderType::Gmail => {
            let auth = ensure_account_oauth_auth(state, &account.id, "gmail").await?;
            let identity = super::oauth::fetch_mailbox_identity("gmail", &auth).await?;
            state.store.apply_verified_oauth_identity(
                &account.id,
                &account.email,
                &identity,
                false,
            )?;
            from.address = identity.email;
            Ok((
                PreparedSendTransport::Gmail(GmailProvider::new_with_proxy(
                    auth.tokens.access_token,
                    auth.proxy,
                )?),
                from,
            ))
        }
        ProviderType::Outlook => {
            let auth = ensure_account_oauth_auth(state, &account.id, "outlook").await?;
            let identity = super::oauth::fetch_mailbox_identity("outlook", &auth).await?;
            state.store.apply_verified_oauth_identity(
                &account.id,
                &account.email,
                &identity,
                false,
            )?;
            from = EmailAddress {
                name: identity.display_name,
                address: identity.email,
            };
            Ok((
                PreparedSendTransport::Outlook(OutlookProvider::new_with_proxy(
                    auth.tokens.access_token,
                    account.id.clone(),
                    auth.proxy,
                )?),
                from,
            ))
        }
        ProviderType::Imap | ProviderType::Pop3 => {
            let smtp_config = load_smtp_config(&state.store, &state.crypto, &account.id)?;
            Ok((
                PreparedSendTransport::Smtp(SmtpSender::new(
                    smtp_config.host,
                    smtp_config.port,
                    smtp_config.username,
                    smtp_config.password,
                    smtp_config.security,
                    smtp_config.accept_invalid_certs,
                    smtp_config.proxy,
                )),
                from,
            ))
        }
    }
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn send_email(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    to: Vec<String>,
    cc: Vec<String>,
    bcc: Vec<String>,
    subject: String,
    body_text: String,
    body_html: Option<String>,
    in_reply_to: Option<String>,
    attachment_paths: Option<Vec<String>>,
) -> std::result::Result<(), PebbleError> {
    let raw_paths = attachment_paths.unwrap_or_default();
    dispatch_send_email(
        &app,
        &state,
        account_id,
        to,
        cc,
        bcc,
        subject,
        body_text,
        body_html,
        in_reply_to,
        raw_paths,
    )
    .await
    .map(|_| ())
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum SendDispatchOutcome {
    Accepted,
    Review,
}

#[allow(clippy::too_many_arguments)]
pub(crate) async fn dispatch_send_email(
    app: &tauri::AppHandle,
    state: &AppState,
    account_id: String,
    to: Vec<String>,
    cc: Vec<String>,
    bcc: Vec<String>,
    subject: String,
    body_text: String,
    body_html: Option<String>,
    in_reply_to: Option<String>,
    raw_paths: Vec<String>,
) -> std::result::Result<SendDispatchOutcome, PebbleError> {
    let attachment_paths = if raw_paths.is_empty() {
        raw_paths
    } else {
        validate_attachment_paths(&raw_paths, &state.attachments_dir)?
    };
    let account = state
        .store
        .get_account(&account_id)?
        .ok_or_else(|| PebbleError::Internal(format!("Account not found: {account_id}")))?;

    let (transport, from) = prepare_send_transport(state, &account).await?;
    let outgoing = OutgoingMessage {
        from,
        to: parse_recipients(to.clone()),
        cc: parse_recipients(cc.clone()),
        bcc: parse_recipients(bcc.clone()),
        subject: subject.clone(),
        body_text: body_text.clone(),
        body_html: body_html.clone(),
        in_reply_to: in_reply_to.clone(),
        attachment_paths: attachment_paths.clone(),
    };

    // Resolve credentials/configuration before creating an in-progress send.
    // Errors here are known pre-dispatch failures and remain safe to return.
    let prepared = prepare_outgoing_send_locally(state, &account, &outgoing)?;
    let durable_attachment_paths = prepared
        .attachments
        .iter()
        .filter_map(|attachment| attachment.local_path.clone())
        .collect();
    let durable_outgoing =
        outgoing_message_from_stored(&prepared.message, durable_attachment_paths);

    match transport.send(&durable_outgoing).await {
        Ok(()) => {
            if let Err(receipt_error) = state
                .store
                .mark_pending_mail_op_remote_succeeded(&prepared.op_id)
            {
                let transition = state.store.mark_pending_mail_op_outcome_unknown(
                    &prepared.op_id,
                    &send_outcome_unknown_message(&receipt_error),
                );
                let _ = app.emit(events::MAIL_PENDING_OPS_CHANGED, ());
                accept_post_dispatch_transition(transition, &prepared.op_id, "outcome-unknown")?;
                return Ok(SendDispatchOutcome::Review);
            }
            if let Err(finalize_error) = state.store.complete_outgoing_send(
                &prepared.message.id,
                &prepared.op_id,
                prepared.sent_folder_id.as_deref(),
            ) {
                // The remote receipt is durable, so this retry only performs
                // the local Sent-folder commit and can never resend the mail.
                let transition = state
                    .store
                    .mark_pending_mail_op_failed(&prepared.op_id, &finalize_error.to_string());
                let _ = app.emit(events::MAIL_PENDING_OPS_CHANGED, ());
                accept_post_dispatch_transition(
                    transition,
                    &prepared.op_id,
                    "local-finalize retry",
                )?;
                return Ok(SendDispatchOutcome::Accepted);
            }
            if prepared.delete_placeholder_after_send {
                cleanup_staged_attachment_records(&prepared.attachments);
            }
            if let Err(error) = refresh_search_document(state, &prepared.message.id) {
                warn!(
                    "Failed to index sent outgoing message {}: {error}",
                    prepared.message.id
                );
            }
            let _ = app.emit(events::MAIL_PENDING_OPS_CHANGED, ());
            Ok(SendDispatchOutcome::Accepted)
        }
        Err(error) if send_call_outcome_is_unknown(&error) => {
            let transition = state.store.mark_pending_mail_op_outcome_unknown(
                &prepared.op_id,
                &send_outcome_unknown_message(&error),
            );
            let _ = app.emit(events::MAIL_PENDING_OPS_CHANGED, ());
            accept_post_dispatch_transition(transition, &prepared.op_id, "outcome-unknown")?;
            Ok(SendDispatchOutcome::Review)
        }
        Err(error) => {
            let transition = state
                .store
                .discard_prepared_outgoing_send(&prepared.message.id, &prepared.op_id);
            if transition.is_ok() {
                cleanup_staged_attachment_records(&prepared.attachments);
                if let Err(index_error) = refresh_search_document(state, &prepared.message.id) {
                    warn!(
                        "Failed to remove discarded outgoing message {} from search: {index_error}",
                        prepared.message.id
                    );
                }
            }
            let _ = app.emit(events::MAIL_PENDING_OPS_CHANGED, ());
            definite_send_failure_command_result(
                error,
                transition.as_ref().err(),
                &prepared.op_id,
            )?;
            Ok(SendDispatchOutcome::Review)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replay_preserves_sender_snapshot_after_account_rename() {
        let store = Store::open_in_memory().unwrap();
        let mut account = test_account();
        account.account_label = Some("Private account label".into());
        store.insert_account(&account).unwrap();
        let attachments_dir = temp_attachments_dir("pebble-sender-snapshot");
        let stored = save_outgoing_message_locally(
            &store,
            &account,
            &outgoing_message(),
            LocalOutgoingState::Queued,
            &attachments_dir,
        )
        .unwrap();
        store
            .update_account_details(
                &account.id,
                "new@example.com",
                "New name",
                None,
                Some("Renamed label"),
            )
            .unwrap();
        let replay = outgoing_message_from_stored(&stored, vec![]);
        assert_eq!(replay.from.address, "sender@example.com");
        assert_eq!(replay.from.name.as_deref(), Some("Sender"));
        assert_eq!(stored.from_name, "Sender");
    }
    use crate::commands::messages::refresh_search_document_with_store;
    use pebble_core::{now_timestamp, Account, FolderRole};
    use pebble_search::TantivySearch;
    use pebble_store::pending_ops::{PendingMailOpStatus, MAX_PENDING_MAIL_OP_ATTEMPTS};
    use pebble_store::Store;

    fn test_account() -> Account {
        Account {
            account_label: None,
            provider_display_name: None,
            id: "account-1".to_string(),
            email: "sender@example.com".to_string(),
            display_name: "Sender".to_string(),
            color: None,
            provider: ProviderType::Imap,
            created_at: now_timestamp(),
            updated_at: now_timestamp(),
        }
    }

    fn outgoing_message() -> OutgoingMessage {
        OutgoingMessage {
            from: pebble_core::EmailAddress {
                name: Some("Sender".into()),
                address: "sender@example.com".into(),
            },
            to: vec![EmailAddress {
                name: None,
                address: "to@example.com".to_string(),
            }],
            cc: vec![],
            bcc: vec![],
            subject: "Queued subject".to_string(),
            body_text: "Plain body".to_string(),
            body_html: Some("<p>HTML body</p>".to_string()),
            in_reply_to: Some("<parent@example.com>".to_string()),
            attachment_paths: vec![],
        }
    }

    fn temp_attachments_dir(prefix: &str) -> std::path::PathBuf {
        std::env::temp_dir().join(format!("{prefix}-{}", new_id()))
    }

    fn prepare_test_send(
        store: &Store,
        account: &Account,
        attachments_dir: &Path,
    ) -> (
        Message,
        Vec<pebble_core::Attachment>,
        String,
        Folder,
        Folder,
    ) {
        let outbox =
            ensure_local_outgoing_folder(store, &account.id, LocalOutgoingState::Queued).unwrap();
        let sent =
            ensure_local_outgoing_folder(store, &account.id, LocalOutgoingState::Sent).unwrap();
        let (message, attachments) = stage_outgoing_message(
            account,
            &outgoing_message(),
            LocalOutgoingState::Queued,
            attachments_dir,
        )
        .unwrap();
        let op_id = store
            .prepare_outgoing_send(
                &message,
                std::slice::from_ref(&outbox.id),
                &attachments,
                &serde_json::json!({
                    "op": "send",
                    "payload": {
                        "local_finalize": SEND_FINALIZE_MOVE_TO_SENT,
                        "sent_folder_id": sent.id,
                    }
                })
                .to_string(),
            )
            .unwrap();
        (message, attachments, op_id, outbox, sent)
    }

    #[test]
    fn sent_message_is_saved_to_local_sent_folder_after_send_success() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let attachments_dir = temp_attachments_dir("pebble-sent-attachments");

        let saved = save_outgoing_message_locally(
            &store,
            &account,
            &outgoing_message(),
            LocalOutgoingState::Sent,
            &attachments_dir,
        )
        .unwrap();

        let sent = store
            .find_folder_by_role(&account.id, FolderRole::Sent)
            .unwrap()
            .expect("sent folder should be created");
        let folder_ids = store.get_message_folder_ids(&saved.id).unwrap();
        let reloaded = store.get_message(&saved.id).unwrap().unwrap();

        assert_eq!(folder_ids, vec![sent.id]);
        assert_eq!(reloaded.from_address, account.email);
        assert_eq!(reloaded.subject, "Queued subject");
        assert!(reloaded.remote_id.starts_with("local-sent-"));
        assert!(reloaded.is_read);
        assert!(!reloaded.is_draft);

        let _ = std::fs::remove_dir_all(attachments_dir);
    }

    #[test]
    fn failed_send_is_saved_to_local_outbox_folder() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let attachments_dir = temp_attachments_dir("pebble-outbox-attachments");

        let saved = save_outgoing_message_locally(
            &store,
            &account,
            &outgoing_message(),
            LocalOutgoingState::Queued,
            &attachments_dir,
        )
        .unwrap();

        let outbox = store
            .find_folder_by_name(&account.id, "Outbox")
            .unwrap()
            .expect("outbox folder should be created");
        let folder_ids = store.get_message_folder_ids(&saved.id).unwrap();

        assert_eq!(outbox.remote_id, "__local_outbox__");
        assert!(outbox.role.is_none());
        assert_eq!(folder_ids, vec![outbox.id]);
        assert!(saved.remote_id.starts_with("local-outbox-"));

        let _ = std::fs::remove_dir_all(attachments_dir);
    }

    #[test]
    fn network_send_failure_is_recorded_for_manual_review_not_retry() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let attachments_dir = temp_attachments_dir("pebble-unknown-send-attachments");
        let (_message, _attachments, op_id, _outbox, _sent) =
            prepare_test_send(&store, &account, &attachments_dir);
        let network_error = PebbleError::Network("connection closed after DATA".to_string());

        store
            .mark_pending_mail_op_outcome_unknown(
                &op_id,
                &send_outcome_unknown_message(&network_error),
            )
            .unwrap();

        let op = store.list_pending_mail_ops(&account.id).unwrap().remove(0);
        assert_eq!(op.status, PendingMailOpStatus::Failed);
        assert_eq!(op.attempts, MAX_PENDING_MAIL_OP_ATTEMPTS);
        assert!(op
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("Check Sent")));
        assert!(store
            .list_retryable_pending_mail_ops(10)
            .unwrap()
            .is_empty());

        let _ = std::fs::remove_dir_all(attachments_dir);
    }

    #[test]
    fn crash_after_dispatch_before_response_becomes_manual_review_on_startup() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let attachments_dir = temp_attachments_dir("pebble-interrupted-send");
        let (message, _attachments, op_id, outbox, _sent) =
            prepare_test_send(&store, &account, &attachments_dir);

        // Inject a process boundary after remote dispatch: no response or
        // remote_succeeded receipt was persisted, so the operation is still
        // in_progress when startup recovery runs.
        store.reset_in_progress_pending_mail_ops().unwrap();

        let op = store
            .list_pending_mail_ops(&account.id)
            .unwrap()
            .into_iter()
            .find(|op| op.id == op_id)
            .unwrap();
        assert_eq!(op.status, PendingMailOpStatus::Failed);
        assert_eq!(op.attempts, MAX_PENDING_MAIL_OP_ATTEMPTS);
        assert!(op
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("outcome is unknown")));
        assert!(store
            .list_retryable_pending_mail_ops(10)
            .unwrap()
            .is_empty());
        assert!(store.get_message(&message.id).unwrap().is_some());
        assert_eq!(
            store.get_message_folder_ids(&message.id).unwrap(),
            vec![outbox.id]
        );

        let _ = std::fs::remove_dir_all(attachments_dir);
    }

    #[test]
    fn successful_prepared_send_finishes_in_sent_without_active_operation() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let attachments_dir = temp_attachments_dir("pebble-successful-prepared-send");
        let (message, _attachments, op_id, outbox, sent) =
            prepare_test_send(&store, &account, &attachments_dir);

        store.mark_pending_mail_op_remote_succeeded(&op_id).unwrap();
        store
            .complete_outgoing_send(&message.id, &op_id, Some(&sent.id))
            .unwrap();

        assert_eq!(
            store.get_message_folder_ids(&message.id).unwrap(),
            vec![sent.id]
        );
        assert_ne!(
            store.get_message_folder_ids(&message.id).unwrap(),
            vec![outbox.id]
        );
        let op = store.list_pending_mail_ops(&account.id).unwrap().remove(0);
        assert_eq!(op.status, PendingMailOpStatus::Done);
        assert!(store
            .list_active_pending_mail_ops(None, 10)
            .unwrap()
            .is_empty());

        let _ = std::fs::remove_dir_all(attachments_dir);
    }

    #[test]
    fn api_send_finalize_deletes_placeholder_to_avoid_sync_duplicates() {
        let store = Store::open_in_memory().unwrap();
        let mut account = test_account();
        account.provider = ProviderType::Gmail;
        store.insert_account(&account).unwrap();
        let attachments_dir = temp_attachments_dir("pebble-api-send-placeholder");
        let outbox =
            ensure_local_outgoing_folder(&store, &account.id, LocalOutgoingState::Queued).unwrap();
        let (message, attachments) = stage_outgoing_message(
            &account,
            &outgoing_message(),
            LocalOutgoingState::Queued,
            &attachments_dir,
        )
        .unwrap();
        let op_id = store
            .prepare_outgoing_send(
                &message,
                std::slice::from_ref(&outbox.id),
                &attachments,
                &serde_json::json!({
                    "op": "send",
                    "payload": {
                        "local_finalize": SEND_FINALIZE_DELETE_PLACEHOLDER,
                    }
                })
                .to_string(),
            )
            .unwrap();

        store.mark_pending_mail_op_remote_succeeded(&op_id).unwrap();
        store
            .complete_outgoing_send(&message.id, &op_id, None)
            .unwrap();

        assert!(store.get_message(&message.id).unwrap().is_none());
        assert!(store
            .list_attachments_by_message(&message.id)
            .unwrap()
            .is_empty());
        assert!(store.list_pending_mail_ops(&account.id).unwrap().is_empty());
        cleanup_staged_attachment_records(&attachments);
        let _ = std::fs::remove_dir_all(attachments_dir);
    }

    #[test]
    fn post_dispatch_transition_write_failure_is_accepted_not_returned_for_retry() {
        let result = accept_post_dispatch_transition(
            Err(PebbleError::Storage(
                "injected transition failure".to_string(),
            )),
            "send-op",
            "outcome-unknown",
        );
        assert!(result.is_ok());
    }

    #[test]
    fn definite_provider_failure_stays_retryable_when_local_cleanup_fails() {
        let provider_error = PebbleError::Auth("provider rejected sender".to_string());
        let cleanup_error = PebbleError::Storage("injected cleanup failure".to_string());
        let result =
            definite_send_failure_command_result(provider_error, Some(&cleanup_error), "send-op");
        assert!(matches!(result, Err(PebbleError::Auth(_))));
    }

    #[test]
    fn locally_saved_outgoing_message_can_be_added_to_search_index() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let attachments_dir = temp_attachments_dir("pebble-outgoing-search");
        let search = TantivySearch::open_in_memory().unwrap();

        let saved = save_outgoing_message_locally(
            &store,
            &account,
            &outgoing_message(),
            LocalOutgoingState::Sent,
            &attachments_dir,
        )
        .unwrap();
        refresh_search_document_with_store(&store, &search, &saved.id).unwrap();

        let hits = search.search("Queued", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].message_id, saved.id);

        let _ = std::fs::remove_dir_all(attachments_dir);
    }

    #[test]
    fn locally_saved_outgoing_message_uses_staged_attachment_copy() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let base = temp_attachments_dir("pebble-outgoing-stage");
        let source_dir = base.join("source");
        let attachments_dir = base.join("attachments");
        std::fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("report.txt");
        std::fs::write(&source, b"payload").unwrap();

        let mut outgoing = outgoing_message();
        outgoing.attachment_paths = vec![source.to_string_lossy().to_string()];

        let saved = save_outgoing_message_locally(
            &store,
            &account,
            &outgoing,
            LocalOutgoingState::Queued,
            &attachments_dir,
        )
        .unwrap();
        let attachments = store.list_attachments_by_message(&saved.id).unwrap();

        assert_eq!(attachments.len(), 1);
        let staged_path = attachments[0].local_path.as_ref().unwrap();
        assert_ne!(staged_path.as_str(), source.to_string_lossy().as_ref());
        assert!(std::path::Path::new(staged_path).starts_with(&attachments_dir));
        assert_eq!(std::fs::read(staged_path).unwrap(), b"payload");

        std::fs::remove_file(&source).unwrap();
        assert_eq!(std::fs::read(staged_path).unwrap(), b"payload");

        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn compose_attachment_upload_stages_browser_file_bytes_under_attachments_dir() {
        let attachments_dir = temp_attachments_dir("pebble-compose-upload");
        let staged = stage_compose_attachment_bytes(
            &attachments_dir,
            "..\\quarterly:report?.txt",
            b"payload",
        )
        .unwrap();

        let canonical_attachments_dir = attachments_dir.canonicalize().unwrap();
        assert!(staged.starts_with(&canonical_attachments_dir));
        assert_eq!(std::fs::read(&staged).unwrap(), b"payload");
        assert_eq!(
            staged.file_name().and_then(|name| name.to_str()),
            Some("quarterly_report_.txt")
        );

        let _ = std::fs::remove_dir_all(attachments_dir);
    }

    #[test]
    fn staged_browser_attachment_preserves_original_filename_in_local_record() {
        let base = temp_attachments_dir("pebble-compose-upload-name");
        let attachments_dir = base.join("attachments");
        let staged =
            stage_compose_attachment_bytes(&attachments_dir, "report.pdf", b"payload").unwrap();

        let records = stage_local_attachment_records(
            &attachments_dir,
            "message-1",
            &[staged.to_string_lossy().to_string()],
        )
        .unwrap();

        assert_eq!(records.len(), 1);
        assert_eq!(records[0].filename, "report.pdf");

        let _ = std::fs::remove_dir_all(base);
    }

    #[test]
    fn compose_staging_cleanup_deletes_only_the_staged_file_and_empty_directory() {
        let attachments_dir = temp_attachments_dir("pebble-compose-cleanup");
        let staged =
            stage_compose_attachment_bytes(&attachments_dir, "report.pdf", b"payload").unwrap();
        let staged_parent = staged.parent().unwrap().to_path_buf();

        cleanup_staged_compose_attachment_path(&attachments_dir, &staged).unwrap();

        assert!(!staged.exists());
        assert!(!staged_parent.exists());
        assert!(attachments_dir.join("compose_staging").exists());
        let _ = std::fs::remove_dir_all(attachments_dir);
    }

    #[test]
    fn compose_staging_cleanup_rejects_files_outside_the_staging_directory() {
        let base = temp_attachments_dir("pebble-compose-cleanup-outside");
        let attachments_dir = base.join("attachments");
        let outside = base.join("keep.txt");
        std::fs::create_dir_all(&attachments_dir).unwrap();
        std::fs::write(&outside, b"keep").unwrap();

        let result = cleanup_staged_compose_attachment_path(&attachments_dir, &outside);

        assert!(result.is_err());
        assert_eq!(std::fs::read(&outside).unwrap(), b"keep");
        let _ = std::fs::remove_dir_all(base);
    }
}

pub(crate) fn stage_compose_attachment_bytes(
    attachments_dir: &Path,
    filename: &str,
    bytes: &[u8],
) -> std::result::Result<PathBuf, PebbleError> {
    let staging_dir = attachments_dir.join("compose_staging");
    std::fs::create_dir_all(&staging_dir).map_err(|e| {
        PebbleError::Internal(format!(
            "Failed to create compose attachment staging directory {}: {e}",
            staging_dir.display()
        ))
    })?;
    let canonical_staging_dir = staging_dir.canonicalize().map_err(|e| {
        PebbleError::Internal(format!(
            "Failed to resolve compose attachment staging directory {}: {e}",
            staging_dir.display()
        ))
    })?;
    let safe_filename = sanitize_stored_filename(filename);
    let staged_dir = canonical_staging_dir.join(new_id());
    std::fs::create_dir_all(&staged_dir).map_err(|e| {
        PebbleError::Internal(format!(
            "Failed to create compose attachment staging directory {}: {e}",
            staged_dir.display()
        ))
    })?;
    let staged_path = staged_dir.join(safe_filename);
    std::fs::write(&staged_path, bytes).map_err(|e| {
        PebbleError::Internal(format!(
            "Failed to stage compose attachment {}: {e}",
            staged_path.display()
        ))
    })?;
    Ok(staged_path)
}

pub(crate) fn cleanup_staged_compose_attachment_path(
    attachments_dir: &Path,
    path: &Path,
) -> std::result::Result<(), PebbleError> {
    let staging_dir = attachments_dir.join("compose_staging");
    let canonical_staging_dir = staging_dir.canonicalize().map_err(|e| {
        PebbleError::Internal(format!(
            "Failed to resolve compose attachment staging directory {}: {e}",
            staging_dir.display()
        ))
    })?;
    let canonical_path = path.canonicalize().map_err(|e| {
        PebbleError::Internal(format!(
            "Failed to resolve staged compose attachment {}: {e}",
            path.display()
        ))
    })?;
    let parent = canonical_path.parent().ok_or_else(|| {
        PebbleError::Validation("Staged compose attachment has no parent directory".to_string())
    })?;
    if !canonical_path.starts_with(&canonical_staging_dir)
        || parent.parent() != Some(canonical_staging_dir.as_path())
        || !canonical_path.is_file()
    {
        return Err(PebbleError::Validation(format!(
            "Path is not a staged compose attachment: {}",
            canonical_path.display()
        )));
    }

    std::fs::remove_file(&canonical_path).map_err(|e| {
        PebbleError::Internal(format!(
            "Failed to remove staged compose attachment {}: {e}",
            canonical_path.display()
        ))
    })?;
    if let Err(error) = std::fs::remove_dir(parent) {
        if error.kind() != std::io::ErrorKind::DirectoryNotEmpty
            && error.kind() != std::io::ErrorKind::NotFound
        {
            return Err(PebbleError::Internal(format!(
                "Failed to remove staged compose attachment directory {}: {error}",
                parent.display()
            )));
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn stage_compose_attachment(
    state: State<'_, AppState>,
    filename: String,
    bytes: Vec<u8>,
) -> std::result::Result<String, PebbleError> {
    let staged = stage_compose_attachment_bytes(&state.attachments_dir, &filename, &bytes)?;
    Ok(staged.to_string_lossy().into_owned())
}

#[tauri::command]
pub async fn cleanup_staged_compose_attachment(
    state: State<'_, AppState>,
    path: String,
) -> std::result::Result<(), PebbleError> {
    cleanup_staged_compose_attachment_path(&state.attachments_dir, Path::new(&path))
}
