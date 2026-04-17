//! Search indexing + rule-application pipeline.
//!
//! Receives newly stored messages from the sync worker, indexes them in
//! Tantivy, and applies rule-engine actions. Split out of `sync_cmd.rs`
//! so the sync lifecycle and the indexing pipeline can evolve independently.

use crate::events;
use pebble_core::PebbleError;
use pebble_rules::RuleEngine;
use pebble_search::TantivySearch;
use pebble_store::Store;
use std::sync::Arc;
use tauri::Emitter;
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
                serde_json::json!({
                    "account_id": stored.message.account_id,
                    "message_id": stored.message.id,
                    "subject": stored.message.subject,
                    "from": stored.message.from_address,
                }),
            );
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
            store.update_message_flags(message_id, Some(true), None)?;
            info!("Rule: marked message {} as read", message_id);
        }
        RuleAction::Archive => {
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
        // TODO: MoveToFolder only updates the local DB; it does not issue an
        // IMAP MOVE or Gmail label change, so the remote mailbox stays unchanged.
        // A future provider-aware action layer should propagate the move upstream.
        RuleAction::MoveToFolder(folder_name) => {
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
