//! Search indexing + rule-application pipeline.
//!
//! Receives newly stored messages from the sync worker, indexes them in
//! Tantivy, and applies rule-engine actions. Split out of `sync_cmd.rs`
//! so the sync lifecycle and the indexing pipeline can evolve independently.

use crate::commands::messages::queue_mail_op;
use crate::commands::notifications;
use crate::events;
use crate::state::AppState;
use pebble_core::{FolderRole, PebbleError};
use pebble_rules::RuleEngine;
use pebble_search::TantivySearch;
use pebble_store::Store;
use serde_json::json;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::Ordering;
use std::sync::Arc;
use tauri::{Emitter, Manager};
use tokio::sync::mpsc;
use tracing::{error, info, warn};

#[cfg(any(
    windows,
    all(not(windows), not(target_os = "linux"), not(target_os = "macos"))
))]
use tauri_plugin_notification::NotificationExt;

#[cfg(any(target_os = "linux", windows))]
const NOTIFICATION_OPEN_ACTION: &str = "open";

/// Serialises "read latest DB state, then write the search index" sections
/// between the live indexer, the full rebuild and other removers, so a stale
/// snapshot can never overwrite a newer one (e.g. the rebuild re-adding a
/// message the indexer just removed). Hold it only around short batches.
static SEARCH_WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn search_write_guard() -> std::sync::MutexGuard<'static, ()> {
    SEARCH_WRITE_LOCK
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// Rebuild the search index from all messages in the store.
///
/// Walks live messages once each in rowid order (`rowid > cursor`, never
/// OFFSET), so every batch is a cheap range seek and Gmail messages with
/// several labels are indexed exactly once with all folder IDs attached.
///
/// Safe to run while sync workers are active: messages committed before the
/// clear are visited by the walk, and anything stored afterwards is indexed
/// by the live indexer; each batch holds [`search_write_guard`] so the two
/// never interleave a read-then-write.
pub fn do_reindex(store: &Store, search: &TantivySearch) -> std::result::Result<u32, PebbleError> {
    const BATCH_SIZE: u32 = 500;
    const COMMIT_EVERY: u32 = 5_000;

    {
        let _guard = search_write_guard();
        search.clear_index()?;
    }

    let mut count: u32 = 0;
    let mut since_commit: u32 = 0;
    let mut after_rowid = 0i64;
    loop {
        let _guard = search_write_guard();
        let messages = store.list_full_messages_after_rowid(after_rowid, BATCH_SIZE)?;
        let Some((last_rowid, _)) = messages.last() else {
            break;
        };
        after_rowid = *last_rowid;

        let ids: Vec<String> = messages.iter().map(|(_, m)| m.id.clone()).collect();
        let mut folder_map = store.get_message_folder_ids_batch(&ids)?;
        let batch: Vec<_> = messages
            .into_iter()
            .map(|(_, msg)| {
                let folder_ids = folder_map.remove(&msg.id).unwrap_or_default();
                (msg, folder_ids)
            })
            .collect();
        let batch_len = batch.len() as u32;
        if let Err(e) = search.index_messages_batch(&batch) {
            warn!("Failed to index batch of {} messages: {}", batch_len, e);
        } else {
            count += batch_len;
            since_commit += batch_len;
        }
        if since_commit >= COMMIT_EVERY {
            // Periodic commits keep search usable during long rebuilds.
            search.commit()?;
            since_commit = 0;
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

#[cfg(test)]
fn should_send_new_mail_notification(
    store: &Store,
    stored: &pebble_mail::StoredMessage,
) -> pebble_core::Result<bool> {
    should_notify_with_cache(store, &mut InboxFolderCache::default(), stored)
}

/// Inbox folder ids per account, loaded once per flush instead of calling
/// `list_folders` for every stored message.
#[derive(Default)]
struct InboxFolderCache {
    inboxes: HashMap<String, HashSet<String>>,
}

impl InboxFolderCache {
    fn inboxes(
        &mut self,
        store: &Store,
        account_id: &str,
    ) -> pebble_core::Result<&HashSet<String>> {
        if !self.inboxes.contains_key(account_id) {
            let ids = store
                .list_folders(account_id)?
                .into_iter()
                .filter(|folder| folder.role == Some(FolderRole::Inbox))
                .map(|folder| folder.id)
                .collect();
            self.inboxes.insert(account_id.to_string(), ids);
        }
        Ok(&self.inboxes[account_id])
    }
}

fn should_notify_with_cache(
    store: &Store,
    cache: &mut InboxFolderCache,
    stored: &pebble_mail::StoredMessage,
) -> pebble_core::Result<bool> {
    if !stored.notify || stored.message.is_deleted || stored.message.is_draft {
        return Ok(false);
    }
    if stored.folder_ids.is_empty() {
        return Ok(false);
    }
    let inboxes = cache.inboxes(store, &stored.message.account_id)?;
    Ok(stored.folder_ids.iter().any(|id| inboxes.contains(id)))
}

fn new_mail_notification_body(
    stored: &pebble_mail::StoredMessage,
    labels: &notifications::NotificationLabels,
) -> String {
    let sender = if stored.message.from_name.trim().is_empty() {
        stored.message.from_address.trim()
    } else {
        stored.message.from_name.trim()
    };
    let subject = stored.message.subject.trim();

    match (sender.is_empty(), subject.is_empty()) {
        (true, true) => labels.new_message_fallback.clone(),
        (true, false) => subject.to_string(),
        (false, true) => sender.to_string(),
        (false, false) => format!("{sender}: {subject}"),
    }
}

fn notification_open_payload(account_id: &str, message_id: &str) -> serde_json::Value {
    serde_json::json!({
        "account_id": account_id,
        "message_id": message_id,
    })
}

#[cfg(any(target_os = "linux", windows))]
fn is_notification_open_action(action: &str) -> bool {
    matches!(action, "default" | NOTIFICATION_OPEN_ACTION)
}

fn open_message_from_notification(app: &tauri::AppHandle, account_id: &str, message_id: &str) {
    notifications::clear_attention_indicator(app);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.show();
        let _ = window.set_focus();
    }
    let _ = app.emit(
        events::MAIL_NOTIFICATION_OPEN,
        notification_open_payload(account_id, message_id),
    );
}

#[cfg(any(
    windows,
    all(not(windows), not(target_os = "linux"), not(target_os = "macos"))
))]
fn show_default_new_mail_notification(app: &tauri::AppHandle, body: &str) -> Result<(), String> {
    let labels = notifications::notification_labels();
    app.notification()
        .builder()
        .title(&labels.new_mail_title)
        .body(body)
        .show()
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "linux")]
fn show_linux_new_mail_notification(
    app: &tauri::AppHandle,
    body: &str,
    account_id: &str,
    message_id: &str,
) -> Result<(), String> {
    notifications::ensure_notification_environment(app)?;
    let labels = notifications::notification_labels();

    let mut notification = notify_rust::Notification::new();
    notification
        .summary(&labels.new_mail_title)
        .body(body)
        .appname("Pebble")
        .auto_icon()
        .action("default", &labels.open_action);

    let handle = notification.show().map_err(|e| e.to_string())?;
    let app_handle = app.clone();
    let account_id = account_id.to_string();
    let message_id = message_id.to_string();

    std::thread::Builder::new()
        .name("pebble-notification-open".to_string())
        .spawn(move || {
            handle.wait_for_action(|action| {
                if is_notification_open_action(action) {
                    open_message_from_notification(&app_handle, &account_id, &message_id);
                }
            });
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "macos")]
fn is_macos_notification_open_response(
    response: &mac_notification_sys::NotificationResponse,
) -> bool {
    matches!(
        response,
        mac_notification_sys::NotificationResponse::Click
            | mac_notification_sys::NotificationResponse::ActionButton(_)
    )
}

#[cfg(target_os = "macos")]
fn show_macos_new_mail_notification(
    app: &tauri::AppHandle,
    body: &str,
    account_id: &str,
    message_id: &str,
) -> Result<(), String> {
    notifications::ensure_notification_environment(app)?;

    let app_handle = app.clone();
    let account_id = account_id.to_string();
    let message_id = message_id.to_string();
    let body = body.to_string();
    let labels = notifications::notification_labels();
    let app_id = if tauri::is_dev() {
        "com.apple.Terminal".to_string()
    } else {
        app.config().identifier.clone()
    };

    std::thread::Builder::new()
        .name("pebble-notification-open".to_string())
        .spawn(move || {
            let _ = mac_notification_sys::set_application(&app_id);
            let mut notification = mac_notification_sys::Notification::new();
            notification
                .title(&labels.new_mail_title)
                .message(&body)
                .main_button(mac_notification_sys::MainButton::SingleAction(
                    &labels.open_action,
                ))
                .wait_for_click(true);

            match notification.send() {
                Ok(response) if is_macos_notification_open_response(&response) => {
                    open_message_from_notification(&app_handle, &account_id, &message_id);
                }
                Ok(_) => {}
                Err(e) => warn!("Failed to show clickable macOS notification: {e}"),
            }
        })
        .map(|_| ())
        .map_err(|e| e.to_string())
}

#[cfg(windows)]
fn show_windows_new_mail_notification(
    app: &tauri::AppHandle,
    body: &str,
    account_id: &str,
    message_id: &str,
) -> Result<(), String> {
    notifications::ensure_notification_environment(app)?;
    let labels = notifications::notification_labels();
    let app_handle = app.clone();
    let account_id = account_id.to_string();
    let message_id = message_id.to_string();
    tauri_winrt_notification::Toast::new(&notifications::windows_notification_app_id(app))
        .title(&labels.new_mail_title)
        .text1(body)
        .duration(tauri_winrt_notification::Duration::Short)
        .add_button(&labels.open_action, NOTIFICATION_OPEN_ACTION)
        .on_activated(move |action| {
            if action
                .as_deref()
                .map(is_notification_open_action)
                .unwrap_or(true)
            {
                open_message_from_notification(&app_handle, &account_id, &message_id);
            }
            Ok(())
        })
        .show()
        .map_err(|e| format!("{e:?}"))
}

fn show_new_mail_notification(
    app: &tauri::AppHandle,
    body: &str,
    account_id: &str,
    message_id: &str,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        match show_windows_new_mail_notification(app, body, account_id, message_id) {
            Ok(()) => Ok(()),
            Err(e) => {
                warn!("Failed to show clickable Windows notification, falling back: {e}");
                show_default_new_mail_notification(app, body)
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        show_linux_new_mail_notification(app, body, account_id, message_id)
    }

    #[cfg(target_os = "macos")]
    {
        show_macos_new_mail_notification(app, body, account_id, message_id)
    }

    #[cfg(all(not(windows), not(target_os = "linux"), not(target_os = "macos")))]
    {
        let _ = (account_id, message_id);
        show_default_new_mail_notification(app, body)
    }
}

/// At most this many individual notifications per account per flush; a
/// larger burst (catch-up after sleep, bulk arrival) becomes one summary.
const MAX_INDIVIDUAL_NOTIFICATIONS: usize = 3;

/// A notification-eligible message, trimmed to what the toast needs.
#[derive(Debug, Clone)]
struct NotifyItem {
    account_id: String,
    message_id: String,
    body: String,
    date: i64,
}

/// Decide which toasts to show for one flush: per account, up to
/// [`MAX_INDIVIDUAL_NOTIFICATIONS`] individual toasts, otherwise a single
/// summary that opens the newest message. Returns `(body, account, message)`.
fn plan_notifications(
    items: Vec<NotifyItem>,
    labels: &notifications::NotificationLabels,
) -> Vec<(String, String, String)> {
    let mut by_account: Vec<(String, Vec<NotifyItem>)> = Vec::new();
    for item in items {
        match by_account
            .iter_mut()
            .find(|(account, _)| *account == item.account_id)
        {
            Some((_, list)) => list.push(item),
            None => by_account.push((item.account_id.clone(), vec![item])),
        }
    }
    let mut toasts = Vec::new();
    for (account_id, mut list) in by_account {
        if list.len() <= MAX_INDIVIDUAL_NOTIFICATIONS {
            toasts.extend(
                list.into_iter()
                    .map(|item| (item.body, item.account_id, item.message_id)),
            );
        } else {
            list.sort_by_key(|item| item.date);
            let newest = list.last().expect("non-empty");
            toasts.push((
                labels.summary(list.len(), &newest.body),
                account_id,
                newest.message_id.clone(),
            ));
        }
    }
    toasts
}

fn send_planned_notifications(app: &tauri::AppHandle, items: Vec<NotifyItem>) {
    if items.is_empty() {
        return;
    }
    let notifications_enabled = app
        .try_state::<AppState>()
        .is_some_and(|state| state.notifications_enabled.load(Ordering::SeqCst));
    if !notifications_enabled {
        return;
    }
    // The user is already looking at Pebble: the list refresh is the signal.
    if notifications::main_window_has_focus(app) {
        return;
    }
    let labels = notifications::notification_labels();
    for (body, account_id, message_id) in plan_notifications(items, &labels) {
        if let Err(e) = show_new_mail_notification(app, &body, &account_id, &message_id) {
            warn!("Failed to show new mail notification: {e}");
        }
    }
    notifications::mark_attention_indicator(app);
}

/// Replay crash-recovery search operations. Only operations that were applied
/// successfully are eligible for clearing, and their markers remain durable
/// until the Tantivy commit itself succeeds.
pub(crate) fn recover_pending_search_operations(
    store: &Store,
    search: &TantivySearch,
) -> pebble_core::Result<usize> {
    let pending = store.list_search_pending()?;
    let mut applied_ids = Vec::with_capacity(pending.len());
    for (message_id, operation) in pending {
        let result = if operation == "remove" {
            search.remove_message(&message_id)
        } else {
            match store.get_message(&message_id) {
                Ok(Some(message)) if !message.is_deleted => {
                    match store.get_message_folder_ids(&message_id) {
                        Ok(folder_ids) if !folder_ids.is_empty() => {
                            search.index_message(&message, &folder_ids)
                        }
                        Ok(_) => search.remove_message(&message_id),
                        Err(error) => Err(error),
                    }
                }
                Ok(_) => search.remove_message(&message_id),
                Err(error) => Err(error),
            }
        };
        match result {
            Ok(()) => applied_ids.push(message_id),
            Err(error) => warn!(
                "Failed to recover search operation for message {}: {}",
                message_id, error
            ),
        }
    }

    if applied_ids.is_empty() {
        return Ok(0);
    }
    search.commit()?;
    store.clear_search_pending(&applied_ids)?;
    Ok(applied_ids.len())
}

/// Flush policy for the live indexer: commit after this many documents or
/// this long after the first unflushed message, whichever comes first.
const FLUSH_MAX_DOCS: usize = 1_000;
const FLUSH_MAX_DELAY: std::time::Duration = std::time::Duration::from_secs(2);

/// Result of processing one flush window on a blocking thread.
#[derive(Default)]
struct FlushOutcome {
    /// One coalesced `mail:new` payload per account.
    events: Vec<serde_json::Value>,
    notifications: Vec<NotifyItem>,
}

/// Receive newly stored messages from the sync worker, apply rules, index
/// them for search and notify the frontend.
///
/// Messages are processed in flush windows (≤ [`FLUSH_MAX_DOCS`] or
/// [`FLUSH_MAX_DELAY`]); each window does one batched DB read, one Tantivy
/// commit, and emits at most one `mail:new` per account (same event name and
/// payload shape as before; `folder_ids` is the union for that account).
/// All store/index work runs on the blocking pool.
pub async fn index_new_messages(
    search: &Arc<TantivySearch>,
    store: &Arc<Store>,
    rx: &mut mpsc::UnboundedReceiver<pebble_mail::StoredMessage>,
    app: Option<tauri::AppHandle>,
) {
    let mut engine: Option<Arc<RuleEngine>> = None;
    let mut rules_fingerprint: Option<String> = None;
    let mut reconciliation_pending_ids: Vec<String> = Vec::new();
    let mut closed = false;

    while !closed {
        let Some(first) = rx.recv().await else {
            break;
        };
        let mut batch = vec![first];
        let deadline = tokio::time::Instant::now() + FLUSH_MAX_DELAY;
        while batch.len() < FLUSH_MAX_DOCS {
            match tokio::time::timeout_at(deadline, rx.recv()).await {
                Ok(Some(stored)) => batch.push(stored),
                Ok(None) => {
                    closed = true;
                    break;
                }
                Err(_) => break,
            }
        }

        // Reload rules only when the rules' content changed (see
        // `pebble_store::rules::rules_fingerprint`).
        match store
            .with_read_async(pebble_store::rules::rules_fingerprint)
            .await
        {
            Ok(fingerprint) if rules_fingerprint.as_deref() != Some(fingerprint.as_str()) => {
                let store = Arc::clone(store);
                match tokio::task::spawn_blocking(move || store.list_rules()).await {
                    Ok(Ok(rules)) => {
                        engine = (!rules.is_empty()).then(|| Arc::new(RuleEngine::new(&rules)));
                        if let Some(ref e) = engine {
                            info!("Rule engine loaded with {} rules", e.rule_count());
                        }
                        rules_fingerprint = Some(fingerprint);
                    }
                    Ok(Err(e)) => warn!("Failed to load rules: {e}"),
                    Err(e) => warn!("Failed to load rules: {e}"),
                }
            }
            Ok(_) => {}
            Err(e) => warn!("Failed to check rules for changes: {e}"),
        }

        let search_for_batch = Arc::clone(search);
        let store_for_batch = Arc::clone(store);
        let engine_for_batch = engine.clone();
        let mut pending_ids = std::mem::take(&mut reconciliation_pending_ids);
        let result = tokio::task::spawn_blocking(move || {
            let outcome = process_flush_window(
                &store_for_batch,
                &search_for_batch,
                engine_for_batch.as_deref(),
                batch,
                &mut pending_ids,
            );
            (outcome, pending_ids)
        })
        .await;
        let outcome = match result {
            Ok((outcome, pending_ids)) => {
                reconciliation_pending_ids = pending_ids;
                outcome
            }
            Err(e) => {
                error!("Search indexing task failed: {e}");
                continue;
            }
        };

        if let Some(ref app) = app {
            for payload in outcome.events {
                let _ = app.emit(events::MAIL_NEW, payload);
            }
            send_planned_notifications(app, outcome.notifications);
        }
    }
}

/// Apply rules, refresh the search documents of a flush window from the
/// latest DB state (one batched read), and commit once.
fn process_flush_window(
    store: &Store,
    search: &TantivySearch,
    engine: Option<&RuleEngine>,
    batch: Vec<pebble_mail::StoredMessage>,
    reconciliation_pending_ids: &mut Vec<String>,
) -> FlushOutcome {
    let mut outcome = FlushOutcome::default();
    let mut inbox_cache = InboxFolderCache::default();
    let labels = notifications::notification_labels();
    // account_id -> (newest stored message, union of folder ids)
    let mut per_account: Vec<(String, usize, Vec<String>)> = Vec::new();

    for (idx, stored) in batch.iter().enumerate() {
        if stored.reconciliation {
            continue;
        }
        let account_id = &stored.message.account_id;
        match per_account
            .iter_mut()
            .find(|(account, _, _)| account == account_id)
        {
            Some((_, newest, folders)) => {
                if stored.message.date >= batch[*newest].message.date {
                    *newest = idx;
                }
                for folder_id in &stored.folder_ids {
                    if !folders.contains(folder_id) {
                        folders.push(folder_id.clone());
                    }
                }
            }
            None => per_account.push((account_id.clone(), idx, stored.folder_ids.clone())),
        }

        match should_notify_with_cache(store, &mut inbox_cache, stored) {
            Ok(true) => outcome.notifications.push(NotifyItem {
                account_id: account_id.clone(),
                message_id: stored.message.id.clone(),
                body: new_mail_notification_body(stored, &labels),
                date: stored.message.date,
            }),
            Ok(false) => {}
            Err(e) => warn!(
                "Failed to evaluate new mail notification eligibility for message {}: {e}",
                stored.message.id
            ),
        }

        if let Some(engine) = engine {
            for action in engine.evaluate(&stored.message) {
                if let Err(e) = apply_rule_action(store, account_id, &stored.message.id, &action) {
                    warn!("Rule action failed for message {}: {e}", stored.message.id);
                }
            }
        }
    }

    for (_, newest, folders) in per_account {
        let mut payload = new_mail_event_payload(&batch[newest]);
        payload["folder_ids"] = json!(folders);
        outcome.events.push(payload);
    }

    let mut ids: Vec<String> = Vec::with_capacity(batch.len());
    let mut seen = HashSet::new();
    for stored in &batch {
        if seen.insert(stored.message.id.as_str()) {
            ids.push(stored.message.id.clone());
        }
    }

    {
        let _guard = search_write_guard();
        let refreshed = store.get_messages_batch(&ids).and_then(|messages| {
            store
                .get_message_folder_ids_batch(&ids)
                .map(|folders| (messages, folders))
        });
        let (messages, mut folder_map) = match refreshed {
            Ok(value) => value,
            Err(e) => {
                warn!(
                    "Failed to reload {} messages before indexing: {e}",
                    ids.len()
                );
                return outcome;
            }
        };
        let mut live: HashMap<String, pebble_core::Message> = messages
            .into_iter()
            .filter(|message| !message.is_deleted)
            .map(|message| (message.id.clone(), message))
            .collect();
        let mut docs = Vec::new();
        let mut applied = HashSet::new();
        for id in &ids {
            let folder_ids = folder_map.remove(id).unwrap_or_default();
            match live.remove(id) {
                Some(message) if !folder_ids.is_empty() => docs.push((message, folder_ids)),
                _ => match search.remove_message(id) {
                    Ok(()) => {
                        applied.insert(id.clone());
                    }
                    Err(e) => warn!("Failed to remove stale search document {id}: {e}"),
                },
            }
        }
        if !docs.is_empty() {
            match search.index_messages_batch(&docs) {
                Ok(()) => applied.extend(docs.iter().map(|(message, _)| message.id.clone())),
                Err(e) => warn!("Failed to index batch of {} messages: {e}", docs.len()),
            }
        }

        for stored in &batch {
            if stored.reconciliation
                && applied.contains(&stored.message.id)
                && !reconciliation_pending_ids.contains(&stored.message.id)
            {
                reconciliation_pending_ids.push(stored.message.id.clone());
            }
        }

        if let Err(e) =
            commit_search_batch_and_clear_recovery(search, store, reconciliation_pending_ids)
        {
            // Documents stay buffered in the writer and markers are kept;
            // the next flush retries the commit.
            error!("Failed to commit search index: {e}");
        }
    }
    outcome
}

fn commit_search_batch_and_clear_recovery(
    search: &TantivySearch,
    store: &Store,
    reconciliation_pending_ids: &mut Vec<String>,
) -> pebble_core::Result<()> {
    finalize_recovery_markers_after_commit(store, reconciliation_pending_ids, || search.commit())
}

fn finalize_recovery_markers_after_commit(
    store: &Store,
    reconciliation_pending_ids: &mut Vec<String>,
    commit: impl FnOnce() -> pebble_core::Result<()>,
) -> pebble_core::Result<()> {
    commit()?;
    if !reconciliation_pending_ids.is_empty() {
        store.clear_search_pending(reconciliation_pending_ids)?;
        reconciliation_pending_ids.clear();
    }
    Ok(())
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
            if let Some(archive_folder) =
                store.find_folder_by_role(account_id, pebble_core::FolderRole::Archive)?
            {
                store.move_message_to_folder(message_id, &archive_folder.id)?;
                info!(
                    "Rule: archived message {} to folder {}",
                    message_id, archive_folder.name
                );
            } else {
                store.soft_delete_message(message_id)?;
                info!(
                    "Rule: archived (soft-deleted) message {} (no archive folder)",
                    message_id
                );
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
                info!(
                    "Rule: moved message {} to folder '{}'",
                    message_id, target_folder.name
                );
            } else {
                warn!(
                    "Rule: target folder '{}' not found for account {}",
                    folder_name, account_id
                );
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
            info!(
                "Rule: added message {} to kanban column {:?}",
                message_id, column
            );
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
    use crate::commands::messages::provider_dispatch::{
        archives_without_folder, folder_change, remote_ids_are_folder_scoped,
    };
    use pebble_core::{FolderRole, ProviderType};
    use pebble_rules::types::RuleAction;
    use pebble_store::pending_ops::{FlagsOp, MailOp};

    let Some(account) = store.get_account(account_id)? else {
        return Ok(false);
    };
    let Some(message) = store.get_message(message_id)? else {
        return Ok(false);
    };
    if account.provider == ProviderType::Pop3 {
        return Ok(false);
    }
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

    let op = match action {
        RuleAction::MarkRead => {
            if remote_ids_are_folder_scoped(&account.provider)
                && source_folder
                    .as_ref()
                    .is_some_and(|folder| folder.remote_id.starts_with("__local_"))
            {
                return Ok(false);
            }
            MailOp::UpdateFlags(FlagsOp {
                is_read: Some(true),
                is_starred: None,
                folder_remote_id: source_folder
                    .as_ref()
                    .map(|folder| folder.remote_id.clone()),
                ..FlagsOp::default()
            })
        }
        RuleAction::Archive => {
            let archive_folder = store.find_folder_by_role(account_id, FolderRole::Archive)?;
            if let Some(archive) = archive_folder.as_ref() {
                if archive.remote_id.starts_with("__local_") {
                    return Ok(false);
                }
            } else if !archives_without_folder(&account.provider) {
                return Ok(false);
            }
            MailOp::Archive(folder_change(
                source_folder.as_ref(),
                archive_folder.as_ref(),
            ))
        }
        RuleAction::MoveToFolder(folder_name) => {
            let Some(target_folder) = store.find_folder_by_name(account_id, folder_name)? else {
                return Ok(false);
            };
            if target_folder.remote_id.starts_with("__local_") {
                return Ok(false);
            }
            let mut change = folder_change(source_folder.as_ref(), Some(&target_folder));
            if account.provider == ProviderType::Gmail {
                // Record the exact label edit, decided against the folders
                // as they are now.
                let delta = gmail_move_label_delta(
                    source_folder
                        .as_ref()
                        .map(|folder| folder.remote_id.as_str()),
                    &target_folder.remote_id,
                    target_folder.role.clone(),
                );
                change.add_labels = Some(delta.add_labels);
                change.remove_labels = Some(delta.remove_labels);
            }
            MailOp::MoveToFolder(change)
        }
        RuleAction::AddLabel(_) | RuleAction::SetKanbanColumn(_) => return Ok(false),
    };
    // The rule's local change is applied when the op replays. If the op
    // cancels an opposite queued op instead, apply it locally right away.
    let outcome = queue_mail_op(store, &message, op, false, None)?;
    Ok(outcome != crate::commands::messages::RemoteMutationOutcome::LocalOnly)
}

#[cfg(test)]
mod rule_writeback_tests {
    #[cfg(any(target_os = "linux", windows))]
    use super::is_notification_open_action;
    use super::{
        apply_rule_action, finalize_recovery_markers_after_commit, new_mail_event_payload,
        notification_open_payload, plan_notifications, process_flush_window,
        recover_pending_search_operations, should_send_new_mail_notification, NotifyItem,
    };
    use pebble_core::*;
    use pebble_rules::types::RuleAction;
    use pebble_search::TantivySearch;
    use pebble_store::pending_ops::PendingMailOpStatus;
    use pebble_store::Store;
    use serde_json::Value;

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
            reconciliation: false,
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
    fn reconciliation_marker_survives_failed_search_commit() {
        let store = Store::open_in_memory().unwrap();
        let message_id = "reconcile-message".to_string();
        store
            .add_search_pending(std::slice::from_ref(&message_id), "remove")
            .unwrap();
        let mut ids = vec![message_id.clone()];

        let result = finalize_recovery_markers_after_commit(&store, &mut ids, || {
            Err(PebbleError::Internal("injected commit failure".to_string()))
        });

        assert!(result.is_err());
        assert_eq!(ids, vec![message_id.clone()]);
        assert_eq!(
            store.list_search_pending().unwrap(),
            vec![(message_id, "remove".to_string())]
        );
    }

    #[test]
    fn startup_search_recovery_commits_before_clearing_markers() {
        let store = Store::open_in_memory().unwrap();
        let search = TantivySearch::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let folder = test_folder(&account.id);
        store.insert_folder(&folder).unwrap();
        let message = test_message(&account.id);
        store
            .insert_message(&message, std::slice::from_ref(&folder.id))
            .unwrap();
        store
            .add_search_pending(std::slice::from_ref(&message.id), "index")
            .unwrap();

        assert_eq!(
            recover_pending_search_operations(&store, &search).unwrap(),
            1
        );
        assert!(store.list_search_pending().unwrap().is_empty());
        assert_eq!(search.search("Test", 10).unwrap().len(), 1);
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
            reconciliation: false,
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
            reconciliation: false,
        };

        assert!(should_send_new_mail_notification(&store, &stored).unwrap());
    }

    #[test]
    fn notification_bursts_collapse_to_one_summary_per_account() {
        let item = |account: &str, id: &str, date: i64| NotifyItem {
            account_id: account.to_string(),
            message_id: id.to_string(),
            body: format!("body {id}"),
            date,
        };
        let mut items: Vec<NotifyItem> =
            (0..50).map(|i| item("bulk", &format!("b{i}"), i)).collect();
        items.push(item("quiet", "q1", 5));
        items.push(item("quiet", "q2", 6));
        let toasts = plan_notifications(
            items,
            &crate::commands::notifications::NotificationLabels::default(),
        );
        assert_eq!(toasts.len(), 3);
        assert_eq!(toasts[0].1, "bulk");
        assert_eq!(toasts[0].2, "b49", "summary opens the newest message");
        assert!(toasts[0].0.starts_with("50 new messages"));
        assert_eq!(toasts[1].2, "q1");
        assert_eq!(toasts[2].2, "q2");
    }

    #[test]
    fn flush_window_coalesces_mail_new_and_indexes_once() {
        let store = Store::open_in_memory().unwrap();
        let search = TantivySearch::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let folder = test_folder(&account.id);
        store.insert_folder(&folder).unwrap();
        let mut batch = Vec::new();
        for i in 0..30 {
            let mut message = test_message(&account.id);
            message.remote_id = format!("r{i}");
            message.date = 1_000 + i;
            store
                .insert_message(&message, std::slice::from_ref(&folder.id))
                .unwrap();
            batch.push(pebble_mail::StoredMessage {
                message,
                folder_ids: vec![folder.id.clone()],
                notify: false,
                reconciliation: false,
            });
        }
        let newest = batch.last().unwrap().message.id.clone();
        let mut pending = Vec::new();
        let outcome = process_flush_window(&store, &search, None, batch, &mut pending);
        assert_eq!(
            outcome.events.len(),
            1,
            "one mail:new per account per flush"
        );
        assert_eq!(outcome.events[0]["account_id"], account.id.as_str());
        assert_eq!(outcome.events[0]["message_id"], newest.as_str());
        assert!(
            outcome.notifications.is_empty(),
            "initial sync must not notify"
        );
        assert_eq!(search.doc_count(), 30);
    }

    #[test]
    fn notification_open_payload_identifies_clicked_message_account() {
        let payload = notification_open_payload("account-2", "message-1");

        assert_eq!(payload["account_id"], "account-2");
        assert_eq!(payload["message_id"], "message-1");
    }

    #[cfg(any(target_os = "linux", windows))]
    #[test]
    fn notification_open_action_accepts_desktop_clicks_only() {
        assert!(is_notification_open_action("default"));
        assert!(is_notification_open_action("open"));
        assert!(!is_notification_open_action("__closed"));
        assert!(!is_notification_open_action("dismissed"));
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
    fn rule_mark_read_for_pop3_account_commits_locally_without_pending_op() {
        let store = Store::open_in_memory().unwrap();
        let mut account = test_account();
        account.provider = ProviderType::Pop3;
        store.insert_account(&account).unwrap();
        let folder = test_folder(&account.id);
        store.insert_folder(&folder).unwrap();
        let message = test_message(&account.id);
        store.insert_message(&message, &[folder.id]).unwrap();

        apply_rule_action(&store, &account.id, &message.id, &RuleAction::MarkRead).unwrap();

        let reloaded = store.get_message(&message.id).unwrap().unwrap();
        assert!(reloaded.is_read);
        let ops = store.list_pending_mail_ops(&account.id).unwrap();
        assert!(ops.is_empty());
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
