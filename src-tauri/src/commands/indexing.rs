//! Search indexing + rule-application pipeline.
//!
//! Receives newly stored messages from the sync worker, indexes them in
//! Tantivy, and applies rule-engine actions. Split out of `sync_cmd.rs`
//! so the sync lifecycle and the indexing pipeline can evolve independently.

use crate::commands::pending_mail_ops::queue_pending_mail_op;
use crate::events;
use crate::state::AppState;
use pebble_core::{FolderRole, PebbleError};
use pebble_rules::RuleEngine;
use pebble_search::TantivySearch;
use pebble_store::Store;
use serde_json::json;
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tokio::sync::mpsc;
use tracing::{error, info, warn};

/// Rebuild the search index from all messages in the store.
///
/// Iterates messages per account (not per folder) so that a Gmail message
/// tagged with multiple labels is indexed exactly once, with all of its
/// folder IDs attached in a single call.
pub fn do_reindex(store: &Store, search: &TantivySearch) -> std::result::Result<u32, PebbleError> {
    search.clear_index()?;

    let accounts = store.list_accounts()?;
    let mut count: u32 = 0;
    let batch_size = 200u32;

    for account in &accounts {
        let mut offset = 0u32;
        loop {
            let messages = store.list_full_messages_by_account(&account.id, batch_size, offset)?;
            if messages.is_empty() {
                break;
            }

            let ids: Vec<String> = messages.iter().map(|m| m.id.clone()).collect();
            let folder_map = store.get_message_folder_ids_batch(&ids)?;

            let batch: Vec<_> = messages
                .iter()
                .map(|msg| {
                    let folder_ids = folder_map.get(&msg.id).cloned().unwrap_or_default();
                    (msg.clone(), folder_ids)
                })
                .collect();
            let batch_len = batch.len() as u32;
            if let Err(e) = search.index_messages_batch(&batch) {
                warn!("Failed to index batch of {} messages: {}", batch_len, e);
            } else {
                count += batch_len;
            }

            offset += messages.len() as u32;
            if (messages.len() as u32) < batch_size {
                break;
            }
        }
    }

    search.commit()?;
    info!("Reindexed {} messages", count);
    Ok(count)
}

/// Receive newly stored messages from the sync worker and index them for search.
/// Also emits `mail:new` events to notify the frontend, and applies rule engine actions.
/// Batches messages and commits periodically for efficiency.
fn new_mail_event_payload(stored: &pebble_mail::StoredMessage) -> serde_json::Value {
    serde_json::json!({
        "account_id": stored.message.account_id,
        "message_id": stored.message.id,
        "folder_ids": stored.folder_ids,
        "thread_id": stored.message.thread_id,
        "subject": stored.message.subject,
        "from": stored.message.from_address,
        "received_at": stored.message.date,
    })
}

fn should_send_new_mail_notification(
    store: &Store,
    stored: &pebble_mail::StoredMessage,
) -> pebble_core::Result<bool> {
    if !stored.notify || stored.message.is_deleted || stored.message.is_draft {
        return Ok(false);
    }

    let folder_ids: HashSet<&str> = stored.folder_ids.iter().map(String::as_str).collect();
    if folder_ids.is_empty() {
        return Ok(false);
    }

    let folders = store.list_folders(&stored.message.account_id)?;
    Ok(folders.iter().any(|folder| {
        folder.role == Some(FolderRole::Inbox) && folder_ids.contains(folder.id.as_str())
    }))
}

fn new_mail_notification_body(stored: &pebble_mail::StoredMessage) -> String {
    let sender = if stored.message.from_name.trim().is_empty() {
        stored.message.from_address.trim()
    } else {
        stored.message.from_name.trim()
    };
    let subject = stored.message.subject.trim();

    match (sender.is_empty(), subject.is_empty()) {
        (true, true) => "New message".to_string(),
        (true, false) => subject.to_string(),
        (false, true) => sender.to_string(),
        (false, false) => format!("{sender}: {subject}"),
    }
}

fn maybe_send_new_mail_notification(
    app: &tauri::AppHandle,
    store: &Store,
    stored: &pebble_mail::StoredMessage,
) {
    let notifications_enabled = app
        .try_state::<AppState>()
        .is_some_and(|state| state.notifications_enabled.load(Ordering::SeqCst));

    if !notifications_enabled {
        return;
    }

    match should_send_new_mail_notification(store, stored) {
        Ok(true) => {
            if let Err(e) = app
                .notification()
                .builder()
                .title("Pebble - New Mail")
                .body(&new_mail_notification_body(stored))
                .show()
            {
                warn!("Failed to show new mail notification: {e}");
            }
        }
        Ok(false) => {}
        Err(e) => warn!(
            "Failed to evaluate new mail notification eligibility for message {}: {e}",
            stored.message.id
        ),
    }
}

pub async fn index_new_messages(
    search: &Arc<TantivySearch>,
    store: &Arc<Store>,
    rx: &mut mpsc::UnboundedReceiver<pebble_mail::StoredMessage>,
    app: Option<tauri::AppHandle>,
) {
    const COMMIT_BATCH_SIZE: u32 = 20;
    const COMMIT_IDLE_SECS: u64 = 2;

    // Rules are reloaded at each batch boundary so edits made mid-sync take
    // effect within ~20 messages (or ~2s idle) rather than waiting for the
    // next full sync session.
    let load_engine = |store: &Arc<Store>| -> Option<RuleEngine> {
        match store.list_rules() {
            Ok(rules) if !rules.is_empty() => Some(RuleEngine::new(&rules)),
            Ok(_) => None,
            Err(e) => {
                warn!("Failed to load rules: {e}");
                None
            }
        }
    };
    let mut engine = load_engine(store);
    if let Some(ref e) = engine {
        info!("Rule engine loaded with {} rules", e.rule_count());
    }

    let mut pending = 0u32;
    loop {
        let stored = match tokio::time::timeout(
            tokio::time::Duration::from_secs(COMMIT_IDLE_SECS),
            rx.recv(),
        )
        .await
        {
            Ok(Some(stored)) => stored,
            Ok(None) => break,
            Err(_) => {
                if pending > 0 {
                    if let Err(e) = search.commit() {
                        error!("Failed to commit search index after idle flush: {}", e);
                    }
                    pending = 0;
                }
                // Idle — take the opportunity to refresh rules.
                engine = load_engine(store);
                continue;
            }
        };

        if let Some(ref app) = app {
            let _ = app.emit(
                events::MAIL_NEW,
                new_mail_event_payload(&stored),
            );
            maybe_send_new_mail_notification(app, store, &stored);
        }

        if let Some(ref engine) = engine {
            let actions = engine.evaluate(&stored.message);
            for action in actions {
                if let Err(e) = apply_rule_action(store, &stored.message.account_id, &stored.message.id, &action) {
                    warn!("Rule action failed for message {}: {e}", stored.message.id);
                }
            }
        }

        let message_id = stored.message.id.clone();
        let latest_message = match store.get_message(&message_id) {
            Ok(message) => message,
            Err(e) => {
                warn!("Failed to reload message {} before indexing: {}", message_id, e);
                continue;
            }
        };

        match latest_message {
            Some(message) if !message.is_deleted => {
                let folder_ids = match store.get_message_folder_ids(&message_id) {
                    Ok(folder_ids) => folder_ids,
                    Err(e) => {
                        warn!("Failed to load folders for indexed message {}: {}", message_id, e);
                        continue;
                    }
                };

                if folder_ids.is_empty() {
                    if let Err(e) = search.remove_message(&message_id) {
                        warn!("Failed to remove folderless search document {}: {}", message_id, e);
                        continue;
                    }
                } else if let Err(e) = search.index_message(&message, &folder_ids) {
                    warn!("Failed to index message {}: {}", message_id, e);
                    continue;
                }
            }
            Some(_) | None => {
                if let Err(e) = search.remove_message(&message_id) {
                    warn!("Failed to remove stale search document {}: {}", message_id, e);
                    continue;
                }
            }
        }
        pending += 1;

        if pending >= COMMIT_BATCH_SIZE {
            if let Err(e) = search.commit() {
                error!("Failed to commit search index: {}", e);
            }
            pending = 0;
            engine = load_engine(store);
        }
    }

    if pending > 0 {
        if let Err(e) = search.commit() {
            error!("Failed to commit search index on close: {}", e);
        }
    }
}

/// Apply a single rule action to a message.
fn apply_rule_action(
    store: &Store,
    account_id: &str,
    message_id: &str,
    action: &pebble_rules::types::RuleAction,
) -> pebble_core::Result<()> {
    use pebble_rules::types::RuleAction;
    match action {
        RuleAction::MarkRead => {
            if queue_remote_rule_action(store, account_id, message_id, action)? {
                info!("Rule: queued remote mark-read for message {}", message_id);
                return Ok(());
            }
            store.update_message_flags(message_id, Some(true), None)?;
            info!("Rule: marked message {} as read", message_id);
        }
        RuleAction::Archive => {
            if queue_remote_rule_action(store, account_id, message_id, action)? {
                info!("Rule: queued remote archive for message {}", message_id);
                return Ok(());
            }
            if let Some(archive_folder) = store.find_folder_by_role(account_id, pebble_core::FolderRole::Archive)? {
                store.move_message_to_folder(message_id, &archive_folder.id)?;
                info!("Rule: archived message {} to folder {}", message_id, archive_folder.name);
            } else {
                store.soft_delete_message(message_id)?;
                info!("Rule: archived (soft-deleted) message {} (no archive folder)", message_id);
            }
        }
        RuleAction::AddLabel(label) => {
            store.add_label(message_id, label)?;
            info!("Rule: added label '{}' to message {}", label, message_id);
        }
        RuleAction::MoveToFolder(folder_name) => {
            if queue_remote_rule_action(store, account_id, message_id, action)? {
                info!(
                    "Rule: queued remote move for message {} to folder '{}'",
                    message_id, folder_name
                );
                return Ok(());
            }
            if let Some(target_folder) = store.find_folder_by_name(account_id, folder_name)? {
                store.move_message_to_folder(message_id, &target_folder.id)?;
                info!("Rule: moved message {} to folder '{}'", message_id, target_folder.name);
            } else {
                warn!("Rule: target folder '{}' not found for account {}", folder_name, account_id);
            }
        }
        RuleAction::SetKanbanColumn(column) => {
            let now = pebble_core::now_timestamp();
            let card = pebble_core::KanbanCard {
                message_id: message_id.to_string(),
                column: column.clone(),
                position: 0,
                created_at: now,
                updated_at: now,
            };
            store.upsert_kanban_card(&card)?;
            info!("Rule: added message {} to kanban column {:?}", message_id, column);
        }
    }
    Ok(())
}

fn queue_remote_rule_action(
    store: &Store,
    account_id: &str,
    message_id: &str,
    action: &pebble_rules::types::RuleAction,
) -> pebble_core::Result<bool> {
    use crate::commands::gmail_labels::gmail_move_label_delta;
    use pebble_core::{FolderRole, ProviderType};
    use pebble_rules::types::RuleAction;

    let Some(account) = store.get_account(account_id)? else {
        return Ok(false);
    };
    let Some(message) = store.get_message(message_id)? else {
        return Ok(false);
    };
    let source_folder = store
        .get_message_folder_ids(message_id)?
        .into_iter()
        .next()
        .and_then(|folder_id| {
            store
                .list_folders(account_id)
                .ok()?
                .into_iter()
                .find(|folder| folder.id == folder_id)
        });

    match action {
        RuleAction::MarkRead => {
            if account.provider == ProviderType::Imap
                && source_folder
                    .as_ref()
                    .is_some_and(|folder| folder.remote_id.starts_with("__local_"))
            {
                return Ok(false);
            }

            let mut payload = json!({
                "is_read": true,
                "is_starred": null,
            });
            if account.provider == ProviderType::Gmail {
                payload["add_labels"] = json!([]);
                payload["remove_labels"] = json!(["UNREAD"]);
            }
            if let Some(folder) = source_folder.as_ref() {
                payload["folder_remote_id"] = json!(folder.remote_id);
            }
            queue_pending_mail_op(store, &message, "update_flags", payload)?;
            Ok(true)
        }
        RuleAction::Archive => {
            let archive_folder = store.find_folder_by_role(account_id, FolderRole::Archive)?;
            if let Some(archive) = archive_folder.as_ref() {
                if archive.remote_id.starts_with("__local_") {
                    return Ok(false);
                }
            } else if account.provider != ProviderType::Gmail {
                return Ok(false);
            }

            let mut payload = json!({
                "source_folder_id": source_folder.as_ref().map(|folder| folder.id.as_str()),
                "source_folder_remote_id": source_folder.as_ref().map(|folder| folder.remote_id.as_str()),
                "target_folder_id": archive_folder.as_ref().map(|folder| folder.id.as_str()),
                "target_folder_remote_id": archive_folder.as_ref().map(|folder| folder.remote_id.as_str()),
            });
            if account.provider == ProviderType::Gmail {
                payload["add_labels"] = json!([]);
                payload["remove_labels"] = json!(["INBOX"]);
            }
            queue_pending_mail_op(store, &message, "archive", payload)?;
            Ok(true)
        }
        RuleAction::MoveToFolder(folder_name) => {
            let Some(target_folder) = store.find_folder_by_name(account_id, folder_name)? else {
                return Ok(false);
            };
            if target_folder.remote_id.starts_with("__local_") {
                return Ok(false);
            }

            let mut payload = json!({
                "source_folder_id": source_folder.as_ref().map(|folder| folder.id.as_str()),
                "source_folder_remote_id": source_folder.as_ref().map(|folder| folder.remote_id.as_str()),
                "target_folder_id": target_folder.id.as_str(),
                "target_folder_remote_id": target_folder.remote_id.as_str(),
            });
            if account.provider == ProviderType::Gmail {
                let delta = gmail_move_label_delta(
                    source_folder
                        .as_ref()
                        .map(|folder| folder.remote_id.as_str()),
                    &target_folder.remote_id,
                    target_folder.role,
                );
                payload["add_labels"] = json!(delta.add_labels);
                payload["remove_labels"] = json!(delta.remove_labels);
            }
            queue_pending_mail_op(store, &message, "move_to_folder", payload)?;
            Ok(true)
        }
        RuleAction::AddLabel(_) | RuleAction::SetKanbanColumn(_) => Ok(false),
    }
}

#[cfg(test)]
mod rule_writeback_tests {
    use super::{apply_rule_action, new_mail_event_payload, should_send_new_mail_notification};
    use pebble_core::*;
    use pebble_rules::types::RuleAction;
    use pebble_store::pending_ops::PendingMailOpStatus;
    use pebble_store::Store;
    use serde_json::Value;

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

    #[test]
    fn new_mail_event_payload_includes_folder_and_thread_contract() {
        let message = Message {
            id: "message-1".to_string(),
            account_id: "account-1".to_string(),
            remote_id: "remote-1".to_string(),
            message_id_header: None,
            in_reply_to: None,
            references_header: None,
            thread_id: Some("thread-1".to_string()),
            subject: "Hello".to_string(),
            snippet: "snippet".to_string(),
            from_address: "sender@example.com".to_string(),
            from_name: "Sender".to_string(),
            to_list: vec![],
            cc_list: vec![],
            bcc_list: vec![],
            body_text: String::new(),
            body_html_raw: String::new(),
            has_attachments: false,
            is_read: false,
            is_starred: false,
            is_draft: false,
            date: 1_700_000_000,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: 1_700_000_000,
            updated_at: 1_700_000_000,
        };
        let stored = pebble_mail::StoredMessage {
            message,
            folder_ids: vec!["folder-inbox".to_string()],
            notify: true,
        };

        let payload = new_mail_event_payload(&stored);

        assert_eq!(payload["account_id"], "account-1");
        assert_eq!(payload["message_id"], "message-1");
        assert_eq!(payload["folder_ids"], serde_json::json!(["folder-inbox"]));
        assert_eq!(payload["thread_id"], "thread-1");
        assert_eq!(payload["subject"], "Hello");
        assert_eq!(payload["from"], "sender@example.com");
        assert_eq!(payload["received_at"], 1_700_000_000);
    }

    fn test_folder(account_id: &str) -> Folder {
        Folder {
            id: new_id(),
            account_id: account_id.to_string(),
            remote_id: "INBOX".to_string(),
            name: "Inbox".to_string(),
            folder_type: FolderType::Folder,
            role: Some(FolderRole::Inbox),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 0,
        }
    }

    fn test_label(account_id: &str, remote_id: &str, name: &str) -> Folder {
        Folder {
            id: new_id(),
            account_id: account_id.to_string(),
            remote_id: remote_id.to_string(),
            name: name.to_string(),
            folder_type: FolderType::Label,
            role: None,
            parent_id: None,
            color: None,
            is_system: false,
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
    fn initial_sync_messages_do_not_trigger_new_mail_notifications() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let folder = test_folder(&account.id);
        store.insert_folder(&folder).unwrap();
        let message = test_message(&account.id);
        let stored = pebble_mail::StoredMessage {
            message,
            folder_ids: vec![folder.id],
            notify: false,
        };

        assert!(!should_send_new_mail_notification(&store, &stored).unwrap());
    }

    #[test]
    fn realtime_inbox_messages_trigger_new_mail_notifications() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let folder = test_folder(&account.id);
        store.insert_folder(&folder).unwrap();
        let message = test_message(&account.id);
        let stored = pebble_mail::StoredMessage {
            message,
            folder_ids: vec![folder.id],
            notify: true,
        };

        assert!(should_send_new_mail_notification(&store, &stored).unwrap());
    }

    #[test]
    fn rule_mark_read_for_remote_account_queues_pending_op_before_local_commit() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let folder = test_folder(&account.id);
        store.insert_folder(&folder).unwrap();
        let message = test_message(&account.id);
        store.insert_message(&message, &[folder.id]).unwrap();

        apply_rule_action(&store, &account.id, &message.id, &RuleAction::MarkRead).unwrap();

        let reloaded = store.get_message(&message.id).unwrap().unwrap();
        assert!(!reloaded.is_read);
        let ops = store.list_pending_mail_ops(&account.id).unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].op_type, "update_flags");
        assert_eq!(ops[0].status, PendingMailOpStatus::Pending);
    }

    #[test]
    fn rule_move_to_folder_from_gmail_label_removes_source_label() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let source = test_label(&account.id, "Label_A", "Label A");
        let target = test_label(&account.id, "Label_B", "Label B");
        store.insert_folder(&source).unwrap();
        store.insert_folder(&target).unwrap();
        let message = test_message(&account.id);
        store.insert_message(&message, &[source.id]).unwrap();

        apply_rule_action(
            &store,
            &account.id,
            &message.id,
            &RuleAction::MoveToFolder("Label B".to_string()),
        )
        .unwrap();

        let ops = store.list_pending_mail_ops(&account.id).unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].op_type, "move_to_folder");
        let payload: Value = serde_json::from_str(&ops[0].payload_json).unwrap();
        let payload = &payload["payload"];
        assert_eq!(
            payload["add_labels"],
            serde_json::json!(["Label_B"]),
            "payload: {payload}"
        );
        assert_eq!(
            payload["remove_labels"],
            serde_json::json!(["Label_A"]),
            "payload: {payload}"
        );
    }
}
