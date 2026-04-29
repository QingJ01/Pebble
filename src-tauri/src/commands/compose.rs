use std::path::{Path, PathBuf};

use crate::commands::attachments::stage_local_attachment_records;
use crate::commands::messages::refresh_search_document;
use crate::commands::oauth::ensure_account_oauth_tokens;
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

pub(crate) fn save_outgoing_message_locally(
    store: &Store,
    account: &Account,
    outgoing: &OutgoingMessage,
    state: LocalOutgoingState,
    attachments_dir: &Path,
) -> std::result::Result<Message, PebbleError> {
    let folder = ensure_local_outgoing_folder(store, &account.id, state)?;
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
        from_address: account.email.clone(),
        from_name: account.display_name.clone(),
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

    store.replace_message_with_attachments(&message, &[folder.id], &attachments)?;
    Ok(message)
}

fn save_outgoing_message_and_refresh_search(
    state: &AppState,
    account: &Account,
    outgoing: &OutgoingMessage,
    outgoing_state: LocalOutgoingState,
    attachments_dir: &Path,
) -> std::result::Result<Message, PebbleError> {
    let message = save_outgoing_message_locally(
        &state.store,
        account,
        outgoing,
        outgoing_state,
        attachments_dir,
    )?;
    if let Err(e) = refresh_search_document(state, &message.id) {
        warn!("Failed to index outgoing message {}: {e}", message.id);
    }
    Ok(message)
}

pub(crate) fn outgoing_message_from_stored(
    message: &Message,
    attachment_paths: Vec<String>,
) -> OutgoingMessage {
    OutgoingMessage {
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
    let encrypted = store.get_auth_data(account_id)?.ok_or_else(|| {
        PebbleError::Internal(format!("No auth data found for account {account_id}"))
    })?;
    let decrypted = crypto.decrypt(&encrypted)?;
    let config: serde_json::Value = serde_json::from_slice(&decrypted)
        .map_err(|e| PebbleError::Internal(format!("Failed to parse decrypted config: {e}")))?;

    serde_json::from_value(
        config
            .get("smtp")
            .cloned()
            .ok_or_else(|| PebbleError::Internal("No SMTP config in auth data".to_string()))?,
    )
    .map_err(|e| PebbleError::Internal(format!("Failed to deserialize SMTP config: {e}")))
}

pub(crate) async fn send_imap_smtp_message(
    state: &AppState,
    account: &Account,
    outgoing: &OutgoingMessage,
) -> std::result::Result<(), PebbleError> {
    let smtp_config = load_smtp_config(&state.store, &state.crypto, &account.id)?;

    let sender = SmtpSender::new(
        smtp_config.host,
        smtp_config.port,
        smtp_config.username,
        smtp_config.password,
        smtp_config.security,
        smtp_config.proxy,
    );

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
            &account.email,
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

fn should_queue_send_failure(error: &PebbleError) -> bool {
    matches!(error, PebbleError::Network(_))
}

fn queue_failed_send(
    state: &AppState,
    account: &Account,
    outgoing: &OutgoingMessage,
    error: &PebbleError,
) -> std::result::Result<String, PebbleError> {
    let message = save_outgoing_message_locally(
        &state.store,
        account,
        outgoing,
        LocalOutgoingState::Queued,
        &state.attachments_dir,
    )?;
    let op_id = state.store.insert_pending_mail_op(
        &account.id,
        &message.id,
        "send",
        &serde_json::json!({
            "provider_account_id": account.id,
            "remote_id": message.remote_id,
            "op": "send",
            "payload": {
                "queued_due_to": error.to_string(),
            },
        })
        .to_string(),
    )?;
    if let Err(e) = refresh_search_document(state, &message.id) {
        warn!(
            "Failed to index queued outgoing message {}: {e}",
            message.id
        );
    }
    Ok(op_id)
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
    let attachment_paths = if raw_paths.is_empty() {
        raw_paths
    } else {
        validate_attachment_paths(&raw_paths, &state.attachments_dir)?
    };
    let account = state
        .store
        .get_account(&account_id)?
        .ok_or_else(|| PebbleError::Internal(format!("Account not found: {account_id}")))?;

    let outgoing = OutgoingMessage {
        to: parse_recipients(to.clone()),
        cc: parse_recipients(cc.clone()),
        bcc: parse_recipients(bcc.clone()),
        subject: subject.clone(),
        body_text: body_text.clone(),
        body_html: body_html.clone(),
        in_reply_to: in_reply_to.clone(),
        attachment_paths: attachment_paths.clone(),
    };

    if matches!(
        account.provider,
        ProviderType::Gmail | ProviderType::Outlook
    ) {
        let provider_name = match account.provider {
            ProviderType::Gmail => "gmail",
            ProviderType::Outlook => "outlook",
            _ => unreachable!(),
        };
        let tokens = ensure_account_oauth_tokens(&state, &account_id, provider_name).await?;
        let result = match account.provider {
            ProviderType::Gmail => {
                let provider = GmailProvider::new(tokens.access_token);
                provider.send_message(&outgoing).await
            }
            ProviderType::Outlook => {
                let provider = OutlookProvider::new(tokens.access_token, account_id);
                provider.send_message(&outgoing).await
            }
            _ => unreachable!(),
        };
        if let Err(e) = result {
            if should_queue_send_failure(&e) {
                queue_failed_send(&state, &account, &outgoing, &e)?;
                let _ = app.emit(events::MAIL_PENDING_OPS_CHANGED, ());
                return Ok(());
            }
            return Err(e);
        }
        return Ok(());
    }

    match send_imap_smtp_message(&state, &account, &outgoing).await {
        Ok(()) => {
            save_outgoing_message_and_refresh_search(
                &state,
                &account,
                &outgoing,
                LocalOutgoingState::Sent,
                &state.attachments_dir,
            )?;
            Ok(())
        }
        Err(e) if should_queue_send_failure(&e) => {
            queue_failed_send(&state, &account, &outgoing, &e)?;
            let _ = app.emit(events::MAIL_PENDING_OPS_CHANGED, ());
            Ok(())
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::messages::refresh_search_document_with_store;
    use pebble_core::{now_timestamp, Account, FolderRole};
    use pebble_search::TantivySearch;
    use pebble_store::Store;

    fn test_account() -> Account {
        Account {
            id: "account-1".to_string(),
            email: "sender@example.com".to_string(),
            display_name: "Sender".to_string(),
            provider: ProviderType::Imap,
            created_at: now_timestamp(),
            updated_at: now_timestamp(),
        }
    }

    fn outgoing_message() -> OutgoingMessage {
        OutgoingMessage {
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
    fn failed_send_is_saved_to_local_outbox_folder_for_retry() {
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
}
