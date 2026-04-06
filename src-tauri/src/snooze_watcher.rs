use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::sync::mpsc::Receiver;
use std::time::{Duration, Instant};

use pebble_store::Store;
use tauri::{Emitter, Manager};
use tauri_plugin_notification::NotificationExt;
use tracing::{debug, error, info, warn};

use crate::events;
use crate::state::AppState;

pub async fn run_snooze_watcher(
    store: Arc<Store>,
    app_handle: tauri::AppHandle,
    stop_rx: Receiver<()>,
) {
    let interval = Duration::from_secs(30);
    let mut last_purge = Instant::now();
    const PURGE_INTERVAL: Duration = Duration::from_secs(3600); // 1 hour
    const TOMBSTONE_MAX_AGE_SECS: i64 = 30 * 24 * 3600; // 30 days

    loop {
        // Check for stop signal (non-blocking) — also stop on channel disconnect
        match stop_rx.try_recv() {
            Ok(()) | Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                debug!("Snooze watcher stopping");
                break;
            }
            Err(std::sync::mpsc::TryRecvError::Empty) => {}
        }

        tokio::time::sleep(interval).await;

        let now = pebble_core::now_timestamp();

        // Run SQLite operations in spawn_blocking to avoid blocking the Tokio runtime
        let store_clone = store.clone();
        let due_result = tokio::task::spawn_blocking(move || {
            store_clone.get_due_snoozed(now)
        }).await;

        match due_result {
            Ok(Ok(due)) => {
                for snoozed in due {
                    debug!("Unsnoozing message {}", snoozed.message_id);
                    let store_clone = store.clone();
                    let msg_id = snoozed.message_id.clone();
                    if let Err(e) = tokio::task::spawn_blocking(move || {
                        store_clone.unsnooze_message(&msg_id)
                    }).await.unwrap_or_else(|e| Err(pebble_core::PebbleError::Internal(e.to_string()))) {
                        error!("Failed to unsnooze message {}: {e}", snoozed.message_id);
                    }
                    let _ = app_handle.emit(events::MAIL_UNSNOOZED, serde_json::json!({
                        "message_id": snoozed.message_id,
                        "return_to": snoozed.return_to,
                    }));

                    // Send OS notification if enabled
                    let should_notify = app_handle
                        .try_state::<AppState>()
                        .map(|s| s.notifications_enabled.load(Ordering::SeqCst))
                        .unwrap_or(true);
                    if should_notify {
                        let store_clone = store.clone();
                        let msg_id = snoozed.message_id.clone();
                        let body = match tokio::task::spawn_blocking(move || {
                            store_clone.get_message(&msg_id)
                        }).await {
                            Ok(Ok(Some(msg))) => {
                                if msg.from_name.is_empty() {
                                    msg.subject.clone()
                                } else {
                                    format!("{}: {}", msg.from_name, msg.subject)
                                }
                            }
                            _ => snoozed.message_id.clone(),
                        };
                        let _ = app_handle
                            .notification()
                            .builder()
                            .title("Pebble - Snoozed Message")
                            .body(&body)
                            .show();
                    }
                }
            }
            Ok(Err(e)) => warn!("Snooze watcher error: {e}"),
            Err(e) => warn!("Snooze watcher task error: {e}"),
        }

        if last_purge.elapsed() >= PURGE_INTERVAL {
            let store_clone = store.clone();
            match tokio::task::spawn_blocking(move || {
                store_clone.purge_old_tombstones(TOMBSTONE_MAX_AGE_SECS)
            }).await {
                Ok(Ok(count)) if count > 0 => info!("Purged {} old tombstone messages", count),
                Ok(Ok(_)) => {}
                Ok(Err(e)) => warn!("Tombstone purge error: {e}"),
                Err(e) => warn!("Tombstone purge task error: {e}"),
            }
            last_purge = Instant::now();
        }
    }
}
