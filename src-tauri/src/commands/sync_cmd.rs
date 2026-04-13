use crate::commands::oauth::{
    build_oauth_token_refresher, decode_oauth_account_tokens, gmail_oauth_config,
    outlook_oauth_config,
};
use crate::events;
use crate::state::{AppState, SyncHandle};
use pebble_core::{PebbleError, ProviderType};
use pebble_mail::{GmailProvider, GmailSyncWorker, ImapConfig, ImapMailProvider, OutlookProvider, OutlookSyncWorker, SyncConfig, SyncWorker};
use pebble_rules::RuleEngine;
use pebble_search::TantivySearch;
use pebble_store::Store;
use std::sync::Arc;
use tauri::{Emitter, State};
use tokio::sync::{mpsc, watch};
use tracing::{error, info, warn};

#[tauri::command]
pub async fn start_sync(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    poll_interval_secs: Option<u64>,
) -> std::result::Result<String, PebbleError> {
    start_sync_inner(&app, &state, account_id.clone(), poll_interval_secs).await?;
    Ok(format!("Sync started for account {account_id}"))
}

/// Auto-resume sync for all existing accounts on app startup.
pub async fn resume_all_syncs(app: tauri::AppHandle) {
    use tauri::Manager;
    let state: tauri::State<AppState> = app.state();
    let accounts = match state.store.list_accounts() {
        Ok(a) => a,
        Err(e) => {
            warn!("Failed to list accounts for auto-sync: {e}");
            return;
        }
    };

    for account in accounts {
        info!("Auto-resuming sync for account {}", account.id);
        if let Err(e) = start_sync_inner(&app, &state, account.id.clone(), None).await {
            warn!("Failed to auto-resume sync for account {}: {e}", account.id);
        }
    }
}

/// Core sync logic shared by the command and auto-resume.
async fn start_sync_inner(
    app: &tauri::AppHandle,
    state: &AppState,
    account_id: String,
    poll_interval_secs: Option<u64>,
) -> std::result::Result<(), PebbleError> {
    // Atomically check and reserve the slot to prevent two sync workers
    // for the same account from starting concurrently.
    {
        let mut handles = state.sync_handles.lock().await;
        if handles.contains_key(&account_id) {
            return Ok(());
        }
        // Insert a placeholder with a dummy stop channel. The real handle
        // will replace it below. If setup fails, we remove the placeholder.
        let (placeholder_tx, _placeholder_rx) = watch::channel(false);
        let placeholder_task = tokio::spawn(async {});
        handles.insert(account_id.clone(), SyncHandle { stop_tx: placeholder_tx, task: placeholder_task });
    }

    // Look up account to determine provider type.
    // On any failure below, remove the placeholder we reserved above.
    let account = match state.store.get_account(&account_id) {
        Ok(Some(a)) => a,
        Ok(None) => {
            let mut handles = state.sync_handles.lock().await;
            handles.remove(&account_id);
            return Err(PebbleError::Internal(format!("Account not found: {account_id}")));
        }
        Err(e) => {
            let mut handles = state.sync_handles.lock().await;
            handles.remove(&account_id);
            return Err(e);
        }
    };

    let store = Arc::clone(&state.store);
    let attachments_dir = state.attachments_dir.clone();
    let (stop_tx, stop_rx) = watch::channel(false);

    let (error_tx, mut error_rx) = mpsc::unbounded_channel();
    let app_handle = app.clone();
    tokio::spawn(async move {
        while let Some(sync_error) = error_rx.recv().await {
            let _ = app_handle.emit(events::MAIL_ERROR, &sync_error);
        }
    });

    // Channel for newly stored messages — used to populate the search index and emit events
    let (message_tx, mut message_rx) = mpsc::unbounded_channel();
    let search = Arc::clone(&state.search);
    let store_for_rules = Arc::clone(&state.store);
    let app_for_index = app.clone();
    tokio::spawn(async move {
        index_new_messages(&search, &store_for_rules, &mut message_rx, Some(app_for_index)).await;
    });

    let app_for_progress = app.clone();
    let account_id_for_progress = account_id.clone();
    let account_id_clone = account_id.clone();

    // Build the provider-specific task. If this fails (e.g. token decode error,
    // IMAP config parse error), remove the placeholder so the account can retry.
    let task = match build_sync_task(
        state,
        store,
        attachments_dir,
        stop_rx,
        error_tx,
        message_tx,
        app_for_progress,
        account_id_for_progress,
        account_id_clone,
        poll_interval_secs,
        account,
    ) {
        Ok(task) => task,
        Err(e) => {
            let mut handles = state.sync_handles.lock().await;
            handles.remove(&account_id);
            return Err(e);
        }
    };

    // Replace the placeholder with the real sync handle.
    {
        let mut handles = state.sync_handles.lock().await;
        handles.insert(account_id, SyncHandle { stop_tx, task });
    }

    Ok(())
}

/// Build and spawn the provider-specific sync task.
///
/// Extracted so that any `?` propagation (token decode, config parse, etc.)
/// returns `Err` to the caller, which can then remove the placeholder entry
/// from `sync_handles` before propagating the error.
fn build_sync_task(
    state: &AppState,
    store: Arc<Store>,
    attachments_dir: std::path::PathBuf,
    stop_rx: watch::Receiver<bool>,
    error_tx: mpsc::UnboundedSender<pebble_mail::SyncError>,
    message_tx: mpsc::UnboundedSender<pebble_mail::StoredMessage>,
    app_for_progress: tauri::AppHandle,
    account_id_for_progress: String,
    account_id_clone: String,
    poll_interval_secs: Option<u64>,
    account: pebble_core::Account,
) -> std::result::Result<tokio::task::JoinHandle<()>, PebbleError> {
    let task = match account.provider {
        ProviderType::Gmail => {
            // --- Gmail: REST API over HTTPS ---
            let tokens = decode_oauth_account_tokens(state, &account_id_clone)?;
            let expires_at = tokens.expires_at;
            let provider = Arc::new(GmailProvider::new(tokens.access_token.clone()));
            let refresher = build_oauth_token_refresher(
                gmail_oauth_config(),
                tokens.refresh_token,
                tokens.access_token,
                Arc::clone(&state.crypto),
                Arc::clone(&state.store),
                account_id_clone.clone(),
            );

            tokio::spawn(async move {
                let _ = app_for_progress.emit(
                    events::MAIL_SYNC_PROGRESS,
                    serde_json::json!({ "account_id": &account_id_for_progress, "status": "started" }),
                );
                let worker = GmailSyncWorker::new(
                    account_id_clone.clone(),
                    provider,
                    store,
                    stop_rx,
                    attachments_dir,
                )
                .with_error_tx(error_tx)
                .with_message_tx(message_tx)
                .with_token_refresher(refresher, expires_at);
                let mut config = SyncConfig::default();
                if let Some(interval) = poll_interval_secs {
                    config.poll_interval_secs = interval;
                }
                worker.run(config).await;
                let _ = app_for_progress.emit(
                    events::MAIL_SYNC_COMPLETE,
                    serde_json::json!({ "account_id": &account_id_for_progress }),
                );
                info!("Gmail sync task completed for account {}", account_id_clone);
            })
        }
        ProviderType::Outlook => {
            // --- Outlook: Graph API over HTTPS ---
            let tokens = decode_oauth_account_tokens(state, &account_id_clone)?;
            let expires_at = tokens.expires_at;
            let provider = Arc::new(OutlookProvider::new(
                tokens.access_token.clone(),
                account_id_clone.clone(),
            ));
            let refresher = build_oauth_token_refresher(
                outlook_oauth_config(),
                tokens.refresh_token,
                tokens.access_token,
                Arc::clone(&state.crypto),
                Arc::clone(&state.store),
                account_id_clone.clone(),
            );

            tokio::spawn(async move {
                let _ = app_for_progress.emit(
                    events::MAIL_SYNC_PROGRESS,
                    serde_json::json!({ "account_id": &account_id_for_progress, "status": "started" }),
                );
                let worker = OutlookSyncWorker::new(
                    account_id_clone.clone(),
                    provider,
                    store,
                    attachments_dir,
                )
                .with_error_tx(error_tx)
                .with_message_tx(message_tx)
                .with_token_refresher(refresher, expires_at);
                let mut config = SyncConfig::default();
                if let Some(interval) = poll_interval_secs {
                    config.poll_interval_secs = interval;
                }
                worker.run(config, stop_rx).await;
                let _ = app_for_progress.emit(
                    events::MAIL_SYNC_COMPLETE,
                    serde_json::json!({ "account_id": &account_id_for_progress }),
                );
                info!("Outlook sync task completed for account {}", account_id_clone);
            })
        }
        ProviderType::Imap => {
            // --- IMAP path ---
            let imap_config: ImapConfig = if let Some(encrypted) = state.store.get_auth_data(&account_id_clone)? {
                let decrypted = state.crypto.decrypt(&encrypted)?;
                let value: serde_json::Value = serde_json::from_slice(&decrypted)
                    .map_err(|e| PebbleError::Internal(format!("Failed to parse decrypted config: {e}")))?;
                serde_json::from_value(value.get("imap").cloned().unwrap_or(value.clone()))
                    .map_err(|e| PebbleError::Internal(format!("Failed to deserialize IMAP config: {e}")))?
            } else {
                // Legacy path: IMAP config used to live inline in sync_state.
                let sync_state = state
                    .store
                    .get_sync_state(&account_id_clone)?
                    .ok_or_else(|| {
                        PebbleError::Internal(format!("No config found for account {account_id_clone}"))
                    })?;
                let imap_value = sync_state.imap.clone().ok_or_else(|| {
                    PebbleError::Internal(format!(
                        "No IMAP config found for account {account_id_clone}"
                    ))
                })?;
                serde_json::from_value(imap_value).map_err(|e| {
                    PebbleError::Internal(format!("Failed to deserialize IMAP config: {e}"))
                })?
            };

            let provider = Arc::new(ImapMailProvider::new(imap_config));
            tokio::spawn(async move {
                let _ = app_for_progress.emit(
                    events::MAIL_SYNC_PROGRESS,
                    serde_json::json!({ "account_id": &account_id_for_progress, "status": "started" }),
                );
                let worker = SyncWorker::new(account_id_clone.clone(), provider, store, stop_rx, attachments_dir)
                    .with_error_tx(error_tx)
                    .with_message_tx(message_tx);
                let mut config = SyncConfig::default();
                if let Some(interval) = poll_interval_secs {
                    config.poll_interval_secs = interval;
                }
                worker.run(config).await;
                let _ = app_for_progress.emit(
                    events::MAIL_SYNC_COMPLETE,
                    serde_json::json!({ "account_id": &account_id_for_progress }),
                );
                info!("Sync task completed for account {}", account_id_clone);
            })
        }
    };

    Ok(task)
}

#[tauri::command]
pub async fn stop_sync(
    state: State<'_, AppState>,
    account_id: String,
) -> std::result::Result<(), PebbleError> {
    let mut handles = state.sync_handles.lock().await;
    if let Some(handle) = handles.remove(&account_id) {
        if let Err(e) = handle.stop_tx.send(true) {
            error!("Failed to send stop signal for account {}: {}", account_id, e);
        }
        handle.task.abort();
    }
    Ok(())
}

/// Rebuild the search index from all messages in the store (shared logic).
///
/// Iterates messages per account (not per folder) so that a Gmail message
/// tagged with multiple labels is indexed exactly once, with all of its
/// folder IDs attached in a single call. This fixes duplicate-index work
/// and ensures the indexed document's folder list is complete.
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

            for msg in &messages {
                let empty: Vec<String> = Vec::new();
                let folder_ids = folder_map.get(&msg.id).unwrap_or(&empty);
                if let Err(e) = search.index_message(msg, folder_ids) {
                    warn!("Failed to index message {}: {}", msg.id, e);
                } else {
                    count += 1;
                }
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

/// Rebuild the search index from all messages currently in the store.
#[tauri::command]
pub async fn reindex_search(
    state: State<'_, AppState>,
) -> std::result::Result<u32, PebbleError> {
    let store = Arc::clone(&state.store);
    let search = Arc::clone(&state.search);

    tokio::task::spawn_blocking(move || do_reindex(&store, &search))
    .await
    .map_err(|e| PebbleError::Internal(format!("Reindex task failed: {e}")))?
}

/// Receive newly stored messages from the sync worker and index them for search.
/// Also emits `mail:new` events to notify the frontend, and applies rule engine actions.
/// Batches messages and commits periodically for efficiency.
async fn index_new_messages(
    search: &Arc<TantivySearch>,
    store: &Arc<Store>,
    rx: &mut mpsc::UnboundedReceiver<pebble_mail::StoredMessage>,
    app: Option<tauri::AppHandle>,
) {
    const COMMIT_BATCH_SIZE: u32 = 20;
    const COMMIT_IDLE_SECS: u64 = 2;

    // Load rules once at the start of the sync session
    let engine = match store.list_rules() {
        Ok(rules) if !rules.is_empty() => {
            info!("Rule engine loaded with {} rules", rules.len());
            Some(RuleEngine::new(&rules))
        }
        Ok(_) => None,
        Err(e) => {
            warn!("Failed to load rules: {e}");
            None
        }
    };

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
                continue;
            }
        };

        // Emit new mail event to frontend
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

        // Apply rule engine actions
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

        // Commit in batches to avoid excessive I/O
        if pending >= COMMIT_BATCH_SIZE {
            if let Err(e) = search.commit() {
                error!("Failed to commit search index: {}", e);
            }
            pending = 0;
        }
    }

    // Commit any remaining indexed messages when the channel closes
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
