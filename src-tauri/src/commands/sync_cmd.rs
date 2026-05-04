use crate::commands::indexing;
use crate::commands::oauth::{
    build_oauth_token_refresher, decode_oauth_account_tokens, gmail_oauth_config,
    outlook_oauth_config,
};
use crate::events;
use crate::realtime::{RealtimeMode, RealtimeStatusPayload, SyncTrigger};
use crate::state::{AppState, SyncHandle};
use pebble_core::{PebbleError, ProviderType};
use pebble_mail::{
    GmailProvider, GmailSyncWorker, ImapMailProvider, OutlookProvider, OutlookSyncWorker,
    SyncConfig, SyncRuntimeStatus, SyncWorker,
};
use pebble_store::Store;
use std::collections::HashSet;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tauri::{Emitter, State};
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};

fn spawn_sync_start_placeholder(
    stop_rx: watch::Receiver<bool>,
    trigger_rx: mpsc::UnboundedReceiver<SyncTrigger>,
) -> tokio::task::JoinHandle<()> {
    tokio::spawn(async move {
        let _keepalive = (stop_rx, trigger_rx);
        std::future::pending::<()>().await;
    })
}

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
        let (placeholder_tx, placeholder_rx) = watch::channel(false);
        let (placeholder_trigger_tx, placeholder_trigger_rx) = mpsc::unbounded_channel();
        let placeholder_task = spawn_sync_start_placeholder(placeholder_rx, placeholder_trigger_rx);
        handles.insert(
            account_id.clone(),
            SyncHandle {
                stop_tx: placeholder_tx,
                trigger_tx: placeholder_trigger_tx,
                task: placeholder_task,
            },
        );
    }

    // Look up account to determine provider type.
    // On any failure below, remove the placeholder we reserved above.
    let account = match state.store.get_account(&account_id) {
        Ok(Some(a)) => a,
        Ok(None) => {
            let mut handles = state.sync_handles.lock().await;
            if let Some(handle) = handles.remove(&account_id) {
                handle.task.abort();
            }
            return Err(PebbleError::Internal(format!(
                "Account not found: {account_id}"
            )));
        }
        Err(e) => {
            let mut handles = state.sync_handles.lock().await;
            if let Some(handle) = handles.remove(&account_id) {
                handle.task.abort();
            }
            return Err(e);
        }
    };

    let provider_for_errors = account.provider.clone();
    let account_id_for_errors = account_id.clone();
    let store = Arc::clone(&state.store);
    let attachments_dir = state.attachments_dir.clone();
    let (stop_tx, stop_rx) = watch::channel(false);
    let (trigger_tx, trigger_rx) = mpsc::unbounded_channel();

    let (error_tx, mut error_rx) = mpsc::unbounded_channel();
    let app_handle = app.clone();
    tokio::spawn(async move {
        while let Some(sync_error) = error_rx.recv().await {
            let _ = app_handle.emit(events::MAIL_ERROR, &sync_error);
            emit_realtime_status(
                &app_handle,
                realtime_status_payload(
                    &account_id_for_errors,
                    &provider_for_errors,
                    realtime_error_mode(&sync_error),
                    None,
                    None,
                    Some(sync_error.message.clone()),
                ),
            );
        }
    });

    let (progress_tx, mut progress_rx) = mpsc::unbounded_channel();
    let app_for_sync_progress = app.clone();
    tokio::spawn(async move {
        while let Some(sync_progress) = progress_rx.recv().await {
            let _ = app_for_sync_progress.emit(events::MAIL_SYNC_PROGRESS, &sync_progress);
        }
    });

    // Channel for newly stored messages — used to populate the search index and emit events
    let (message_tx, mut message_rx) = mpsc::unbounded_channel();
    let search = Arc::clone(&state.search);
    let store_for_rules = Arc::clone(&state.store);
    let app_for_index = app.clone();
    tokio::spawn(async move {
        indexing::index_new_messages(
            &search,
            &store_for_rules,
            &mut message_rx,
            Some(app_for_index),
        )
        .await;
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
        trigger_rx,
        error_tx,
        progress_tx,
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
            if let Some(handle) = handles.remove(&account_id) {
                handle.task.abort();
            }
            return Err(e);
        }
    };

    // Replace the placeholder with the real sync handle.
    {
        let mut handles = state.sync_handles.lock().await;
        if let Some(previous) = handles.insert(
            account_id,
            SyncHandle {
                stop_tx,
                trigger_tx,
                task,
            },
        ) {
            previous.task.abort();
        }
    }

    Ok(())
}

fn provider_slug(provider: &ProviderType) -> &'static str {
    match provider {
        ProviderType::Imap => "imap",
        ProviderType::Gmail => "gmail",
        ProviderType::Outlook => "outlook",
    }
}

fn now_timestamp_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

fn realtime_status_payload(
    account_id: &str,
    provider: &ProviderType,
    mode: RealtimeMode,
    last_success_at: Option<i64>,
    next_retry_at: Option<i64>,
    message: Option<String>,
) -> RealtimeStatusPayload {
    RealtimeStatusPayload {
        account_id: account_id.to_string(),
        mode,
        provider: provider_slug(provider).to_string(),
        last_success_at,
        next_retry_at,
        message,
    }
}

fn manual_realtime_status_payload(
    account_id: &str,
    provider: &ProviderType,
) -> RealtimeStatusPayload {
    realtime_status_payload(
        account_id,
        provider,
        RealtimeMode::Manual,
        None,
        None,
        Some("Manual only".to_string()),
    )
}

fn realtime_error_mode(sync_error: &pebble_mail::SyncError) -> RealtimeMode {
    let text = format!(
        "{} {}",
        sync_error.error_type.to_ascii_lowercase(),
        sync_error.message.to_ascii_lowercase()
    );
    if text.contains("auth")
        || text.contains("token")
        || text.contains("unauthorized")
        || text.contains("401")
    {
        RealtimeMode::AuthRequired
    } else if text.contains("offline") || text.contains("network") {
        RealtimeMode::Offline
    } else if text.contains("circuit") || text.contains("backoff") {
        RealtimeMode::Backoff
    } else {
        RealtimeMode::Error
    }
}

fn emit_realtime_status(app: &tauri::AppHandle, payload: RealtimeStatusPayload) {
    let _ = app.emit(events::MAIL_REALTIME_STATUS, payload);
}

fn polling_status_message(config: &SyncConfig) -> String {
    if config.manual_only() {
        "Manual only".to_string()
    } else {
        format!("Polling every {}s", config.poll_interval_secs)
    }
}

fn realtime_preference_poll_interval(mode: &str) -> std::result::Result<u64, PebbleError> {
    match mode {
        "realtime" => Ok(3),
        "balanced" => Ok(15),
        "battery" => Ok(60),
        "manual" => Ok(0),
        other => Err(PebbleError::Validation(format!(
            "Invalid realtime preference: {other}"
        ))),
    }
}

fn imap_initial_realtime_mode(config: &SyncConfig) -> RealtimeMode {
    if config.manual_only() {
        RealtimeMode::Manual
    } else {
        RealtimeMode::Polling
    }
}

fn imap_capability_realtime_mode(config: &SyncConfig, supports_idle: bool) -> RealtimeMode {
    if config.manual_only() {
        RealtimeMode::Manual
    } else if supports_idle {
        RealtimeMode::Realtime
    } else {
        RealtimeMode::Polling
    }
}

#[derive(Debug, Default)]
struct RealtimePreferenceStartSummary {
    started_count: usize,
    failures: Vec<(String, String)>,
}

impl RealtimePreferenceStartSummary {
    fn record_start_result(
        &mut self,
        account_id: &str,
        result: std::result::Result<(), PebbleError>,
    ) {
        match result {
            Ok(()) => self.started_count += 1,
            Err(e) => self.failures.push((account_id.to_string(), e.to_string())),
        }
    }

    fn into_command_result(self) -> std::result::Result<(), PebbleError> {
        if self.failures.is_empty() {
            return Ok(());
        }

        let failures = self
            .failures
            .iter()
            .map(|(account_id, error)| format!("{account_id}: {error}"))
            .collect::<Vec<_>>()
            .join("; ");
        Err(PebbleError::Internal(format!(
            "Realtime preference applied with {} account start failure(s); {} account(s) started; failures: {}",
            self.failures.len(),
            self.started_count,
            failures
        )))
    }
}

/// Build and spawn the provider-specific sync task.
///
/// Extracted so that any `?` propagation (token decode, config parse, etc.)
/// returns `Err` to the caller, which can then remove the placeholder entry
/// from `sync_handles` before propagating the error.
#[allow(clippy::too_many_arguments)]
fn build_sync_task(
    state: &AppState,
    store: Arc<Store>,
    attachments_dir: std::path::PathBuf,
    stop_rx: watch::Receiver<bool>,
    trigger_rx: mpsc::UnboundedReceiver<SyncTrigger>,
    error_tx: mpsc::UnboundedSender<pebble_mail::SyncError>,
    progress_tx: mpsc::UnboundedSender<pebble_mail::SyncProgress>,
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
            let tokens = match decode_oauth_account_tokens(state, &account_id_clone) {
                Ok(tokens) => tokens,
                Err(e) => {
                    emit_realtime_status(
                        &app_for_progress,
                        realtime_status_payload(
                            &account_id_clone,
                            &ProviderType::Gmail,
                            RealtimeMode::AuthRequired,
                            None,
                            None,
                            Some(e.to_string()),
                        ),
                    );
                    return Err(e);
                }
            };
            let expires_at = tokens.expires_at;
            let provider = Arc::new(GmailProvider::new_with_proxy(
                tokens.access_token.clone(),
                tokens.proxy.clone(),
            )?);
            let refresher = build_oauth_token_refresher(
                gmail_oauth_config(),
                tokens.refresh_token,
                tokens.access_token,
                Arc::clone(&state.crypto),
                Arc::clone(&state.store),
                account_id_clone.clone(),
            );

            tokio::spawn(async move {
                let mut config = SyncConfig::default();
                if let Some(interval) = poll_interval_secs {
                    config.poll_interval_secs = interval;
                }
                emit_realtime_status(
                    &app_for_progress,
                    realtime_status_payload(
                        &account_id_for_progress,
                        &ProviderType::Gmail,
                        RealtimeMode::Polling,
                        Some(now_timestamp_secs()),
                        None,
                        Some(polling_status_message(&config)),
                    ),
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
                .with_progress_tx(progress_tx)
                .with_token_refresher(refresher, expires_at);
                worker.run(config, Some(trigger_rx)).await;
                let _ = app_for_progress.emit(
                    events::MAIL_SYNC_COMPLETE,
                    serde_json::json!({ "account_id": &account_id_for_progress }),
                );
                info!("Gmail sync task completed for account {}", account_id_clone);
            })
        }
        ProviderType::Outlook => {
            // --- Outlook: Graph API over HTTPS ---
            let tokens = match decode_oauth_account_tokens(state, &account_id_clone) {
                Ok(tokens) => tokens,
                Err(e) => {
                    emit_realtime_status(
                        &app_for_progress,
                        realtime_status_payload(
                            &account_id_clone,
                            &ProviderType::Outlook,
                            RealtimeMode::AuthRequired,
                            None,
                            None,
                            Some(e.to_string()),
                        ),
                    );
                    return Err(e);
                }
            };
            let expires_at = tokens.expires_at;
            let provider = Arc::new(OutlookProvider::new_with_proxy(
                tokens.access_token.clone(),
                account_id_clone.clone(),
                tokens.proxy.clone(),
            )?);
            let refresher = build_oauth_token_refresher(
                outlook_oauth_config(),
                tokens.refresh_token,
                tokens.access_token,
                Arc::clone(&state.crypto),
                Arc::clone(&state.store),
                account_id_clone.clone(),
            );

            tokio::spawn(async move {
                let mut config = SyncConfig::default();
                if let Some(interval) = poll_interval_secs {
                    config.poll_interval_secs = interval;
                }
                emit_realtime_status(
                    &app_for_progress,
                    realtime_status_payload(
                        &account_id_for_progress,
                        &ProviderType::Outlook,
                        RealtimeMode::Polling,
                        Some(now_timestamp_secs()),
                        None,
                        Some(polling_status_message(&config)),
                    ),
                );
                let worker = OutlookSyncWorker::new(
                    account_id_clone.clone(),
                    provider,
                    store,
                    attachments_dir,
                )
                .with_error_tx(error_tx)
                .with_message_tx(message_tx)
                .with_progress_tx(progress_tx)
                .with_token_refresher(refresher, expires_at);
                worker.run(config, stop_rx, Some(trigger_rx)).await;
                let _ = app_for_progress.emit(
                    events::MAIL_SYNC_COMPLETE,
                    serde_json::json!({ "account_id": &account_id_for_progress }),
                );
                info!(
                    "Outlook sync task completed for account {}",
                    account_id_clone
                );
            })
        }
        ProviderType::Imap => {
            // --- IMAP path ---
            let imap_config = match crate::commands::messages::load_imap_config(
                &state.store,
                &state.crypto,
                &account_id_clone,
            ) {
                Ok(config) => config,
                Err(e) => {
                    emit_realtime_status(
                        &app_for_progress,
                        realtime_status_payload(
                            &account_id_clone,
                            &ProviderType::Imap,
                            RealtimeMode::Error,
                            None,
                            None,
                            Some(e.to_string()),
                        ),
                    );
                    return Err(e);
                }
            };

            let provider = Arc::new(ImapMailProvider::new(imap_config));
            tokio::spawn(async move {
                let mut config = SyncConfig::default();
                if let Some(interval) = poll_interval_secs {
                    config.poll_interval_secs = interval;
                }
                emit_realtime_status(
                    &app_for_progress,
                    realtime_status_payload(
                        &account_id_for_progress,
                        &ProviderType::Imap,
                        imap_initial_realtime_mode(&config),
                        Some(now_timestamp_secs()),
                        None,
                        Some(polling_status_message(&config)),
                    ),
                );
                let (runtime_status_tx, mut runtime_status_rx) = mpsc::unbounded_channel();
                let app_for_runtime_status = app_for_progress.clone();
                let account_id_for_runtime_status = account_id_for_progress.clone();
                let config_for_runtime_status = config.clone();
                tokio::spawn(async move {
                    while let Some(status) = runtime_status_rx.recv().await {
                        let supports_idle = matches!(status, SyncRuntimeStatus::ImapIdleAvailable);
                        emit_realtime_status(
                            &app_for_runtime_status,
                            realtime_status_payload(
                                &account_id_for_runtime_status,
                                &ProviderType::Imap,
                                imap_capability_realtime_mode(
                                    &config_for_runtime_status,
                                    supports_idle,
                                ),
                                Some(now_timestamp_secs()),
                                None,
                                if supports_idle {
                                    None
                                } else {
                                    Some(polling_status_message(&config_for_runtime_status))
                                },
                            ),
                        );
                    }
                });
                let worker = SyncWorker::new(
                    account_id_clone.clone(),
                    provider,
                    store,
                    stop_rx,
                    attachments_dir,
                )
                .with_error_tx(error_tx)
                .with_message_tx(message_tx)
                .with_progress_tx(progress_tx)
                .with_runtime_status_tx(runtime_status_tx);
                worker.run(config, Some(trigger_rx)).await;
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
pub async fn trigger_sync(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    account_id: String,
    reason: String,
) -> std::result::Result<(), PebbleError> {
    let trigger = SyncTrigger::from_reason(&reason);
    let should_start_one_shot = {
        let mut handles = state.sync_handles.lock().await;
        match handles.get(&account_id) {
            Some(handle) if handle.task.is_finished() => {
                handles.remove(&account_id);
                true
            }
            Some(handle) => {
                let send_failed = handle.trigger_tx.send(trigger).is_err();
                if send_failed {
                    warn!(
                        "Sync trigger channel was already closed for account {}",
                        account_id
                    );
                    handles.remove(&account_id);
                }
                should_drop_trigger_handle(false, send_failed)
            }
            None => true,
        }
    };

    if should_start_one_shot {
        start_sync_inner(&app, &state, account_id, Some(0)).await?;
    }
    Ok(())
}

#[tauri::command]
pub async fn stop_sync(
    state: State<'_, AppState>,
    account_id: String,
) -> std::result::Result<(), PebbleError> {
    stop_sync_inner(&state, &account_id).await;
    Ok(())
}

async fn stop_sync_inner(state: &AppState, account_id: &str) {
    let mut handles = state.sync_handles.lock().await;
    if let Some(handle) = handles.remove(account_id) {
        if should_send_stop_signal_to_handle(handle.task.is_finished()) {
            if let Err(e) = handle.stop_tx.send(true) {
                warn!(
                    "Sync stop channel was already closed for account {}: {}",
                    account_id, e
                );
            }
            handle.task.abort();
        }
    }
}

fn should_send_stop_signal_to_handle(task_finished: bool) -> bool {
    !task_finished
}

fn should_drop_trigger_handle(task_finished: bool, send_failed: bool) -> bool {
    task_finished || send_failed
}

#[tauri::command]
pub async fn set_realtime_preference(
    app: tauri::AppHandle,
    state: State<'_, AppState>,
    mode: String,
) -> std::result::Result<(), PebbleError> {
    let poll_interval_secs = realtime_preference_poll_interval(&mode)?;
    let accounts = state.store.list_accounts()?;
    let running_account_ids = {
        let handles = state.sync_handles.lock().await;
        handles.keys().cloned().collect::<Vec<_>>()
    };

    for account_id in running_account_ids {
        stop_sync_inner(&state, &account_id).await;
    }

    if poll_interval_secs == 0 {
        for account in accounts {
            emit_realtime_status(
                &app,
                manual_realtime_status_payload(&account.id, &account.provider),
            );
        }
        return Ok(());
    }

    let account_ids = accounts
        .into_iter()
        .map(|account| account.id)
        .collect::<Vec<_>>();

    let mut start_summary = RealtimePreferenceStartSummary::default();
    for account_id in account_ids {
        let result =
            start_sync_inner(&app, &state, account_id.clone(), Some(poll_interval_secs)).await;
        if let Err(e) = &result {
            warn!("Failed to apply realtime preference to account {account_id}: {e}");
        }
        start_summary.record_start_result(&account_id, result);
    }

    if !start_summary.failures.is_empty() {
        warn!(
            "Realtime preference applied with {} account start failure(s); {} account(s) started",
            start_summary.failures.len(),
            start_summary.started_count
        );
    }

    start_summary.into_command_result()
}

/// Rebuild the search index from all messages currently in the store.
#[tauri::command]
pub async fn reindex_search(state: State<'_, AppState>) -> std::result::Result<u32, PebbleError> {
    let store = Arc::clone(&state.store);
    let search = Arc::clone(&state.search);

    tokio::task::spawn_blocking(move || indexing::do_reindex(&store, &search))
        .await
        .map_err(|e| PebbleError::Internal(format!("Reindex task failed: {e}")))?
}

#[allow(dead_code)]
#[derive(Default)]
struct TriggerCoalescer {
    pending: HashSet<String>,
}

#[allow(dead_code)]
impl TriggerCoalescer {
    fn mark_pending(&mut self, account_id: &str) -> bool {
        self.pending.insert(account_id.to_string())
    }

    fn clear_pending(&mut self, account_id: &str) {
        self.pending.remove(account_id);
    }
}

#[cfg(test)]
mod trigger_tests {
    use super::*;

    #[test]
    fn coalesces_duplicate_realtime_triggers_for_same_account() {
        let mut state = TriggerCoalescer::default();

        assert!(state.mark_pending("account-1"));
        assert!(!state.mark_pending("account-1"));
        state.clear_pending("account-1");
        assert!(state.mark_pending("account-1"));
    }

    #[test]
    fn realtime_status_payload_uses_provider_mode_contract() {
        let payload = realtime_status_payload(
            "account-1",
            &ProviderType::Imap,
            crate::realtime::RealtimeMode::Realtime,
            Some(1_700_000_000),
            None,
            None,
        );

        let json = serde_json::to_value(payload).unwrap();

        assert_eq!(json["account_id"], "account-1");
        assert_eq!(json["provider"], "imap");
        assert_eq!(json["mode"], "realtime");
        assert_eq!(json["last_success_at"], 1_700_000_000);
        assert!(json["next_retry_at"].is_null());
        assert!(json["message"].is_null());
    }

    #[test]
    fn realtime_preference_maps_to_backend_poll_interval() {
        assert_eq!(realtime_preference_poll_interval("realtime").unwrap(), 3);
        assert_eq!(realtime_preference_poll_interval("balanced").unwrap(), 15);
        assert_eq!(realtime_preference_poll_interval("battery").unwrap(), 60);
        assert_eq!(realtime_preference_poll_interval("manual").unwrap(), 0);
        assert!(realtime_preference_poll_interval("turbo").is_err());
    }

    #[test]
    fn manual_preference_status_payload_reports_manual_mode() {
        let payload = manual_realtime_status_payload("account-1", &ProviderType::Gmail);

        let json = serde_json::to_value(payload).unwrap();

        assert_eq!(json["account_id"], "account-1");
        assert_eq!(json["provider"], "gmail");
        assert_eq!(json["mode"], "manual");
        assert_eq!(json["message"], "Manual only");
    }

    #[test]
    fn realtime_preference_start_summary_keeps_successes_after_account_failure() {
        let mut summary = RealtimePreferenceStartSummary::default();

        summary.record_start_result(
            "bad-account",
            Err(PebbleError::Internal("No auth data".to_string())),
        );
        summary.record_start_result("good-account", Ok(()));

        assert_eq!(summary.started_count, 1);
        assert_eq!(summary.failures.len(), 1);
        let err = summary
            .into_command_result()
            .expect_err("partial realtime preference failures should be visible to the UI");
        assert!(err.to_string().contains("bad-account"));
        assert!(err.to_string().contains("1 account(s) started"));
    }

    #[tokio::test]
    async fn sync_start_placeholder_keeps_slot_reserved_until_replaced() {
        let (_stop_tx, stop_rx) = watch::channel(false);
        let (trigger_tx, trigger_rx) = mpsc::unbounded_channel();
        let handle = spawn_sync_start_placeholder(stop_rx, trigger_rx);
        tokio::task::yield_now().await;

        assert!(!handle.is_finished());
        assert!(trigger_tx.send(SyncTrigger::Manual).is_ok());
        handle.abort();
    }

    #[test]
    fn finished_sync_handle_does_not_need_stop_signal() {
        assert!(!should_send_stop_signal_to_handle(true));
        assert!(should_send_stop_signal_to_handle(false));
    }

    #[test]
    fn trigger_handle_is_dropped_when_finished_or_channel_closed() {
        assert!(should_drop_trigger_handle(true, false));
        assert!(should_drop_trigger_handle(false, true));
        assert!(!should_drop_trigger_handle(false, false));
    }

    #[test]
    fn imap_initial_status_is_polling_until_idle_is_confirmed() {
        let config = SyncConfig {
            poll_interval_secs: 10,
            ..Default::default()
        };

        assert_eq!(imap_initial_realtime_mode(&config), RealtimeMode::Polling);
    }

    #[test]
    fn imap_capability_status_reports_realtime_only_when_idle_is_available() {
        let config = SyncConfig {
            poll_interval_secs: 10,
            ..Default::default()
        };

        assert_eq!(
            imap_capability_realtime_mode(&config, true),
            RealtimeMode::Realtime
        );
        assert_eq!(
            imap_capability_realtime_mode(&config, false),
            RealtimeMode::Polling
        );
    }
}
