use crate::events;
use crate::state::{AppState, SyncHandle};
use pebble_core::{PebbleError, ProviderType};
use pebble_mail::{GmailProvider, GmailSyncWorker, ImapConfig, ImapMailProvider, OutlookProvider, SyncConfig, SyncWorker};
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
    // Check if already syncing
    {
        let handles = state.sync_handles.lock().await;
        if handles.contains_key(&account_id) {
            return Ok(());
        }
    }

    // Look up account to determine provider type
    let account = state
        .store
        .get_account(&account_id)?
        .ok_or_else(|| PebbleError::Internal(format!("Account not found: {account_id}")))?;

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

    let task = match account.provider {
        ProviderType::Gmail => {
            // --- Gmail: REST API over HTTPS ---
            let encrypted = state
                .store
                .get_auth_data(&account_id)?
                .ok_or_else(|| PebbleError::Internal("No auth data for Gmail account".into()))?;
            let decrypted = state.crypto.decrypt(&encrypted)?;
            let token_data: serde_json::Value = serde_json::from_slice(&decrypted)
                .map_err(|e| PebbleError::Internal(format!("Failed to parse token data: {e}")))?;
            let access_token = token_data["access_token"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let refresh_token = token_data["refresh_token"]
                .as_str()
                .map(|s| s.to_string());
            let expires_at = token_data["expires_at"].as_i64();

            let provider = Arc::new(GmailProvider::new(access_token));

            // Build token refresher closure
            let crypto = Arc::clone(&state.crypto);
            let store_for_refresh = Arc::clone(&state.store);
            let acct_id_for_refresh = account_id.clone();
            let refresher: pebble_mail::gmail_sync::TokenRefresher = if let Some(rt) = refresh_token {
                Box::new(move || {
                    let rt = rt.clone();
                    let crypto = Arc::clone(&crypto);
                    let store = Arc::clone(&store_for_refresh);
                    let acct_id = acct_id_for_refresh.clone();
                    Box::pin(async move {
                        use pebble_oauth::{OAuthConfig, OAuthManager};
                        let config = OAuthConfig {
                            client_id: option_env!("GOOGLE_CLIENT_ID")
                                .unwrap_or("GOOGLE_CLIENT_ID_PLACEHOLDER").into(),
                            client_secret: None,
                            auth_url: "https://accounts.google.com/o/oauth2/v2/auth".into(),
                            token_url: "https://oauth2.googleapis.com/token".into(),
                            scopes: vec!["https://mail.google.com/".into()],
                            redirect_port: 8756,
                        };
                        let manager = OAuthManager::new(config);
                        let token_pair = manager
                            .refresh_token(&rt)
                            .await
                            .map_err(|e| PebbleError::OAuth(format!("Token refresh failed: {e}")))?;

                        // Persist updated tokens
                        let tokens_json = serde_json::json!({
                            "access_token": token_pair.access_token,
                            "refresh_token": token_pair.refresh_token,
                            "expires_at": token_pair.expires_at,
                            "scopes": token_pair.scopes,
                        });
                        let config_bytes = serde_json::to_vec(&tokens_json)
                            .map_err(|e| PebbleError::Internal(format!("Serialize tokens: {e}")))?;
                        let encrypted = crypto.encrypt(&config_bytes)?;
                        store.set_auth_data(&acct_id, &encrypted)?;

                        Ok(token_pair.access_token)
                    })
                })
            } else {
                // No refresh token — just return current token
                let provider_ref = Arc::clone(&provider);
                Box::new(move || {
                    let p = Arc::clone(&provider_ref);
                    Box::pin(async move {
                        Ok(p.token())
                    })
                })
            };

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
            let encrypted = state
                .store
                .get_auth_data(&account_id)?
                .ok_or_else(|| PebbleError::Internal("No auth data for Outlook account".into()))?;
            let decrypted = state.crypto.decrypt(&encrypted)?;
            let token_data: serde_json::Value = serde_json::from_slice(&decrypted)
                .map_err(|e| PebbleError::Internal(format!("Failed to parse token data: {e}")))?;
            let access_token = token_data["access_token"]
                .as_str()
                .unwrap_or("")
                .to_string();
            let refresh_token = token_data["refresh_token"]
                .as_str()
                .map(|s| s.to_string());
            let expires_at = token_data["expires_at"].as_i64();

            let provider = Arc::new(OutlookProvider::new(access_token, account_id.clone()));

            // Build token refresher closure for Outlook
            let crypto = Arc::clone(&state.crypto);
            let store_for_refresh = Arc::clone(&state.store);
            let acct_id_for_refresh = account_id.clone();
            let provider_for_task = Arc::clone(&provider);

            let refresher: Option<pebble_mail::gmail_sync::TokenRefresher> = refresh_token.map(|rt| {
                let refresher: pebble_mail::gmail_sync::TokenRefresher = Box::new(move || {
                    let rt = rt.clone();
                    let crypto = Arc::clone(&crypto);
                    let store = Arc::clone(&store_for_refresh);
                    let acct_id = acct_id_for_refresh.clone();
                    Box::pin(async move {
                        use pebble_oauth::{OAuthConfig, OAuthManager};
                        let config = OAuthConfig {
                            client_id: std::env::var("MICROSOFT_CLIENT_ID")
                                .unwrap_or_else(|_| "MICROSOFT_CLIENT_ID_PLACEHOLDER".into()),
                            client_secret: std::env::var("MICROSOFT_CLIENT_SECRET").ok(),
                            auth_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize".into(),
                            token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token".into(),
                            scopes: vec![
                                "https://graph.microsoft.com/Mail.ReadWrite".into(),
                                "https://graph.microsoft.com/Mail.Send".into(),
                                "offline_access".into(),
                            ],
                            redirect_port: 8757,
                        };
                        let manager = OAuthManager::new(config);
                        let token_pair = manager
                            .refresh_token(&rt)
                            .await
                            .map_err(|e| PebbleError::OAuth(format!("Outlook token refresh failed: {e}")))?;

                        let tokens_json = serde_json::json!({
                            "access_token": token_pair.access_token,
                            "refresh_token": token_pair.refresh_token,
                            "expires_at": token_pair.expires_at,
                            "scopes": token_pair.scopes,
                        });
                        let config_bytes = serde_json::to_vec(&tokens_json)
                            .map_err(|e| PebbleError::Internal(format!("Serialize tokens: {e}")))?;
                        let encrypted = crypto.encrypt(&config_bytes)?;
                        store.set_auth_data(&acct_id, &encrypted)?;

                        Ok(token_pair.access_token)
                    })
                });
                refresher
            });

            tokio::spawn(async move {
                use pebble_core::traits::{FetchQuery, FolderProvider, MailTransport};
                let mut stop_rx = stop_rx;
                let mut current_expires_at = expires_at;

                let _ = app_for_progress.emit(
                    events::MAIL_SYNC_PROGRESS,
                    serde_json::json!({ "account_id": &account_id_for_progress, "status": "started" }),
                );

                let poll_interval = poll_interval_secs.unwrap_or(300);

                loop {
                    if *stop_rx.borrow() {
                        break;
                    }

                    // Refresh token if needed — update expires_at after successful refresh
                    if let Some(ref refresher_fn) = refresher {
                        let should_refresh = current_expires_at
                            .map(|exp| pebble_core::now_timestamp() >= exp - 60)
                            .unwrap_or(false);
                        if should_refresh {
                            match refresher_fn().await {
                                Ok(new_token) => {
                                    provider_for_task.set_access_token(new_token);
                                    // Update expires_at so we don't refresh again until next expiry
                                    current_expires_at = Some(pebble_core::now_timestamp() + 3600);
                                }
                                Err(e) => {
                                    let _ = error_tx.send(pebble_mail::SyncError {
                                        error_type: "token_refresh".into(),
                                        message: format!("Outlook token refresh failed: {e}"),
                                        timestamp: pebble_core::now_timestamp() as u64,
                                    });
                                }
                            }
                        }
                    }

                    // List folders and fetch messages per folder
                    let folders = match provider_for_task.list_folders().await {
                        Ok(f) => f,
                        Err(e) => {
                            warn!("Outlook folder list failed: {e}");
                            let _ = error_tx.send(pebble_mail::SyncError {
                                error_type: "sync".into(),
                                message: format!("Outlook folder list failed: {e}"),
                                timestamp: pebble_core::now_timestamp() as u64,
                            });
                            vec![]
                        }
                    };

                    for folder in &folders {
                        // Persist folder
                        let _ = store.insert_folder(&folder);

                        let query = FetchQuery {
                            folder_id: folder.remote_id.clone(),
                            limit: Some(50),
                        };
                        match provider_for_task.fetch_messages(&query).await {
                            Ok(result) => {
                                // Deduplicate: only insert messages not already in the store
                                let remote_ids: Vec<String> = result.messages.iter().map(|m| m.remote_id.clone()).collect();
                                let existing = store.get_existing_remote_ids(&account_id_clone, &remote_ids)
                                    .unwrap_or_default();
                                for msg in &result.messages {
                                    if existing.contains(&msg.remote_id) {
                                        continue;
                                    }
                                    let folder_ids = vec![folder.id.clone()];
                                    if let Err(e) = store.insert_message(msg, &folder_ids) {
                                        warn!("Failed to store Outlook message: {e}");
                                    } else {
                                        let _ = message_tx.send(pebble_mail::StoredMessage {
                                            message: msg.clone(),
                                            folder_ids,
                                        });
                                    }
                                }
                            }
                            Err(e) => {
                                warn!("Outlook sync fetch failed for folder {}: {e}", folder.name);
                            }
                        }

                        if *stop_rx.borrow() {
                            break;
                        }
                    }

                    // Emit progress
                    let _ = app_for_progress.emit(
                        events::MAIL_SYNC_PROGRESS,
                        serde_json::json!({ "account_id": &account_id_for_progress, "status": "synced" }),
                    );

                    // Wait for next poll or stop signal
                    let _ = tokio::time::timeout(
                        std::time::Duration::from_secs(poll_interval),
                        stop_rx.changed(),
                    ).await;
                }

                let _ = app_for_progress.emit(
                    events::MAIL_SYNC_COMPLETE,
                    serde_json::json!({ "account_id": &account_id_for_progress }),
                );
                info!("Outlook sync task completed for account {}", account_id_clone);
            })
        }
        ProviderType::Imap => {
            // --- IMAP path ---
            let imap_config: ImapConfig = if let Some(encrypted) = state.store.get_auth_data(&account_id)? {
                let decrypted = state.crypto.decrypt(&encrypted)?;
                let value: serde_json::Value = serde_json::from_slice(&decrypted)
                    .map_err(|e| PebbleError::Internal(format!("Failed to parse decrypted config: {e}")))?;
                serde_json::from_value(value.get("imap").cloned().unwrap_or(value.clone()))
                    .map_err(|e| PebbleError::Internal(format!("Failed to deserialize IMAP config: {e}")))?
            } else {
                let sync_state_json = state
                    .store
                    .get_account_sync_state(&account_id)?
                    .ok_or_else(|| {
                        PebbleError::Internal(format!("No config found for account {account_id}"))
                    })?;
                let value: serde_json::Value = serde_json::from_str(&sync_state_json)
                    .map_err(|e| PebbleError::Internal(format!("Failed to parse sync state: {e}")))?;
                if let Some(imap_value) = value.get("imap") {
                    serde_json::from_value(imap_value.clone())
                        .map_err(|e| PebbleError::Internal(format!("Failed to deserialize IMAP config: {e}")))?
                } else {
                    serde_json::from_value(value)
                        .map_err(|e| PebbleError::Internal(format!("Failed to deserialize IMAP config: {e}")))?
                }
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

    {
        let mut handles = state.sync_handles.lock().await;
        handles.insert(account_id, SyncHandle { stop_tx, task });
    }

    Ok(())
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
pub fn do_reindex(store: &Store, search: &TantivySearch) -> std::result::Result<u32, PebbleError> {
    search.clear_index()?;

    let accounts = store.list_accounts()?;
    let mut count: u32 = 0;

    for account in &accounts {
        let folders = store.list_folders(&account.id)?;
        for folder in &folders {
            let mut offset = 0u32;
            let batch_size = 200u32;
            loop {
                let messages = store.list_full_messages_by_folder(&folder.id, batch_size, offset)?;
                if messages.is_empty() {
                    break;
                }
                for msg in &messages {
                    if let Err(e) = search.index_message(msg, &[folder.id.clone()]) {
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
