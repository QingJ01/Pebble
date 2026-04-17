use crate::commands::oauth::{
    build_oauth_token_refresher, decode_oauth_account_tokens, gmail_oauth_config,
    outlook_oauth_config,
};
use crate::events;
use crate::state::{AppState, SyncHandle};
use crate::commands::indexing;
use pebble_core::{PebbleError, ProviderType};
use pebble_mail::{GmailProvider, GmailSyncWorker, ImapMailProvider, OutlookProvider, OutlookSyncWorker, SyncConfig, SyncWorker};
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
    // If an old task has finished, remove its stale entry so a new one can start.
    {
        let mut handles = state.sync_handles.lock().await;
        if let Some(existing) = handles.get(&account_id) {
            if !existing.task.is_finished() {
                return Ok(());
            }
            handles.remove(&account_id);
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
        indexing::index_new_messages(&search, &store_for_rules, &mut message_rx, Some(app_for_index)).await;
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
            let imap_config =
                crate::commands::messages::load_imap_config(&state.store, &state.crypto, &account_id_clone)?;

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

/// Rebuild the search index from all messages currently in the store.
#[tauri::command]
pub async fn reindex_search(
    state: State<'_, AppState>,
) -> std::result::Result<u32, PebbleError> {
    let store = Arc::clone(&state.store);
    let search = Arc::clone(&state.search);

    tokio::task::spawn_blocking(move || indexing::do_reindex(&store, &search))
    .await
    .map_err(|e| PebbleError::Internal(format!("Reindex task failed: {e}")))?
}
