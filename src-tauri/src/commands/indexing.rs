//! Search indexing + rule-application pipeline.
//!
//! Receives newly stored messages from the sync worker, indexes them in
//! Tantivy, and applies rule-engine actions. Split out of `sync_cmd.rs`
//! so the sync lifecycle and the indexing pipeline can evolve independently.

use crate::commands::messages::queue_mail_op;
use crate::commands::notifications;
use crate::events;
use crate::smart_notify;
use crate::state::AppState;
#[cfg(test)]
use pebble_core::FolderRole;
use pebble_core::PebbleError;
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
#[cfg(test)]
#[derive(Default)]
struct InboxFolderCache {
    inboxes: HashMap<String, HashSet<String>>,
}

#[cfg(test)]
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

#[cfg(test)]
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
fn show_default_new_mail_notification(
    app: &tauri::AppHandle,
    title: &str,
    body: &str,
) -> Result<(), String> {
    app.notification()
        .builder()
        .title(title)
        .body(body)
        .show()
        .map_err(|e| e.to_string())
}

#[cfg(target_os = "linux")]
fn show_linux_new_mail_notification(
    app: &tauri::AppHandle,
    title: &str,
    body: &str,
    account_id: &str,
    message_id: &str,
) -> Result<(), String> {
    notifications::ensure_notification_environment(app)?;
    let labels = notifications::notification_labels();

    let mut notification = notify_rust::Notification::new();
    notification
        .summary(title)
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
    title: &str,
    body: &str,
    account_id: &str,
    message_id: &str,
) -> Result<(), String> {
    notifications::ensure_notification_environment(app)?;

    let app_handle = app.clone();
    let account_id = account_id.to_string();
    let message_id = message_id.to_string();
    let body = body.to_string();
    let title = title.to_string();
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
                .title(&title)
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
    title: &str,
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
        .title(title)
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
    let labels = notifications::notification_labels();
    show_titled_notification(app, &labels.new_mail_title, body, account_id, message_id)
}

/// A notification from the agent (#107) that opens a message when clicked.
pub(crate) fn show_agent_notification(
    app: &tauri::AppHandle,
    title: &str,
    body: &str,
    account_id: &str,
    message_id: &str,
) -> Result<(), String> {
    show_titled_notification(app, title, body, account_id, message_id)
}

fn show_titled_notification(
    app: &tauri::AppHandle,
    title: &str,
    body: &str,
    account_id: &str,
    message_id: &str,
) -> Result<(), String> {
    #[cfg(windows)]
    {
        match show_windows_new_mail_notification(app, title, body, account_id, message_id) {
            Ok(()) => Ok(()),
            Err(e) => {
                warn!("Failed to show clickable Windows notification, falling back: {e}");
                show_default_new_mail_notification(app, title, body)
            }
        }
    }

    #[cfg(target_os = "linux")]
    {
        show_linux_new_mail_notification(app, title, body, account_id, message_id)
    }

    #[cfg(target_os = "macos")]
    {
        show_macos_new_mail_notification(app, title, body, account_id, message_id)
    }

    #[cfg(all(not(windows), not(target_os = "linux"), not(target_os = "macos")))]
    {
        let _ = (account_id, message_id);
        show_default_new_mail_notification(app, title, body)
    }
}

/// At most this many individual notifications per account per flush; a
/// larger burst (catch-up after sleep, bulk arrival) becomes one summary.
const MAX_INDIVIDUAL_NOTIFICATIONS: usize = 3;

/// A notification-eligible message, trimmed to what the toast needs.
#[derive(Debug, Clone)]
pub(crate) struct NotifyItem {
    pub(crate) account_id: String,
    pub(crate) message_id: String,
    /// Decision-log key; marked delivered once shown.
    pub(crate) message_key: String,
    pub(crate) body: String,
    pub(crate) date: i64,
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

pub(crate) fn send_planned_notifications(app: &tauri::AppHandle, items: Vec<NotifyItem>) {
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
    let delivered: Vec<(String, String)> = items
        .iter()
        .map(|item| (item.account_id.clone(), item.message_key.clone()))
        .collect();
    let labels = notifications::notification_labels();
    for (body, account_id, message_id) in plan_notifications(items, &labels) {
        if let Err(e) = show_new_mail_notification(app, &body, &account_id, &message_id) {
            warn!("Failed to show new mail notification: {e}");
        }
    }
    notifications::mark_attention_indicator(app);
    if let Some(state) = app.try_state::<AppState>() {
        for (account_id, key) in delivered {
            if let Err(e) = state.store.mark_notification_delivered(&account_id, &key) {
                warn!("Failed to mark notification delivered: {e}");
            }
        }
    }
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
    /// Every new message decided in this window (logged once each).
    triaged: Vec<smart_notify::Triaged>,
    /// A follow-up completed or a deadline was found (the feed changed).
    agent_events_changed: bool,
    /// Messages held back for model-assisted triage.
    model_candidates: Vec<crate::model_triage::ModelCandidate>,
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
            if !outcome.triaged.is_empty() {
                let _ = app.emit(events::NOTIFY_CHANGED, ());
            }
            if outcome.agent_events_changed {
                let _ = app.emit(events::AGENT_EVENTS_CHANGED, ());
            }
            send_planned_notifications(app, outcome.notifications);
            if !outcome.model_candidates.is_empty() {
                tauri::async_runtime::spawn(crate::model_triage::refine(
                    app.clone(),
                    outcome.model_candidates,
                ));
            }
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
    let mut triage = smart_notify::TriageCache::default();
    let minute_of_day = smart_notify::local_minute_of_day();
    let now = pebble_core::now_timestamp();
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

        // Rules run first so the notification decision sees what they did
        // (a rule that marks read or files the message away must not notify).
        let actions = engine
            .map(|engine| engine.evaluate(&stored.message))
            .unwrap_or_default();
        for action in &actions {
            if let Err(e) = apply_rule_action(store, account_id, &stored.message.id, action) {
                warn!("Rule action failed for message {}: {e}", stored.message.id);
            }
        }

        if let Err(e) = store.record_message_signals(&stored.message.id, stored.signals) {
            warn!(
                "Failed to record signals for message {}: {e}",
                stored.message.id
            );
        }
        if let Some(account) = triage.account(store, account_id) {
            if smart_notify::is_from_account(&stored.message, &account) {
                if let Err(e) = store.note_sent_recipients(&stored.message) {
                    warn!(
                        "Failed to note recipients of sent message {}: {e}",
                        stored.message.id
                    );
                }
            }
        }

        if !stored.notify {
            // First sync, backfill and repairs are never notified or logged.
            continue;
        }
        let from_self = triage
            .account(store, account_id)
            .is_some_and(|account| smart_notify::is_from_account(&stored.message, &account));
        let watch = triage.matching_watch(store, &stored.message, from_self);
        let facts = smart_notify::gather_facts(
            store,
            &mut triage,
            stored,
            smart_notify::RuleEffect::from_actions(&actions),
            watch.is_some(),
        );
        let prefs = triage.prefs(store, account_id);
        let decision = pebble_notify::decide(&facts, &prefs.to_prefs(), minute_of_day);
        let triaged = smart_notify::Triaged {
            account_id: account_id.clone(),
            message_id: stored.message.id.clone(),
            message_key: pebble_store::notify::notification_message_key(&stored.message),
            decision,
        };
        if triaged.decision.tier == pebble_notify::Tier::Silent
            && matches!(triaged.decision.reason, pebble_notify::Reason::Backfill)
        {
            continue;
        }
        // A person's reply completes the follow-up whatever the notification
        // decision (a list header, a server filter or a rule may keep it
        // from notifying); auto-replies and bots don't.
        let machine_reply = stored.signals.auto_submitted
            || pebble_notify::is_automated_sender(
                &stored.message.from_address,
                &stored.message.from_name,
            );
        if let (Some(watch), false) = (&watch, machine_reply) {
            if smart_notify::complete_watch(store, watch, &stored.message, now) {
                outcome.agent_events_changed = true;
            }
            triage.forget_watch(account_id, &watch.id);
        }
        if !smart_notify::record(store, &triaged, now) {
            continue;
        }
        // Deadlines in mail that reaches the user (not bulk: "offer ends
        // Friday" is not a deadline).
        let has_deadline = triaged.decision.tier != pebble_notify::Tier::Silent
            && !matches!(triaged.decision.reason, pebble_notify::Reason::Bulk)
            && crate::deadlines::detect(store, &stored.message, now);
        outcome.agent_events_changed |= has_deadline;
        let notify_item = NotifyItem {
            account_id: account_id.clone(),
            message_id: stored.message.id.clone(),
            message_key: triaged.message_key.clone(),
            body: new_mail_notification_body(stored, &labels),
            date: stored.message.date,
        };
        let model_assisted = triaged.decision.model_candidate
            && prefs.model_assist
            && triage
                .account(store, account_id)
                .is_some_and(|account| smart_notify::model_assist_allowed(&account));
        if model_assisted {
            outcome
                .model_candidates
                .push(crate::model_triage::ModelCandidate {
                    triaged: triaged.clone(),
                    prefs: prefs.clone(),
                    message: stored.message.clone(),
                    to_me: facts.to_me,
                    known_contact: facts.sender_is_favorite || facts.sender_known,
                    times_user_wrote: store
                        .sent_count_to(account_id, &stored.message.from_address)
                        .unwrap_or(0),
                    automated: pebble_notify::is_automated(&facts),
                    notify: notify_item,
                    has_deadline,
                });
        } else if triaged.decision.tier == pebble_notify::Tier::Interrupt {
            outcome.notifications.push(notify_item);
        }
        outcome.triaged.push(triaged);
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
            signals: Default::default(),
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
            signals: Default::default(),
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
            signals: Default::default(),
        };

        assert!(should_send_new_mail_notification(&store, &stored).unwrap());
    }

    #[test]
    fn notification_bursts_collapse_to_one_summary_per_account() {
        let item = |account: &str, id: &str, date: i64| NotifyItem {
            account_id: account.to_string(),
            message_id: id.to_string(),
            message_key: format!("id:{id}"),
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
                signals: Default::default(),
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

    fn new_inbox_message(
        store: &Store,
        account: &Account,
        folder: &Folder,
        from: &str,
    ) -> pebble_mail::StoredMessage {
        let mut message = test_message(&account.id);
        message.remote_id = new_id();
        message.message_id_header = Some(format!("<{}@example.com>", message.remote_id));
        message.from_address = from.to_string();
        message.to_list = vec![pebble_core::EmailAddress {
            name: None,
            address: account.email.clone(),
        }];
        store
            .insert_message(&message, std::slice::from_ref(&folder.id))
            .unwrap();
        pebble_mail::StoredMessage {
            message,
            folder_ids: vec![folder.id.clone()],
            notify: true,
            reconciliation: false,
            signals: Default::default(),
        }
    }

    fn triage_setup(mode: &str) -> (Store, TantivySearch, Account, Folder) {
        let store = Store::open_in_memory().unwrap();
        let search = TantivySearch::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let folder = test_folder(&account.id);
        store.insert_folder(&folder).unwrap();
        store
            .set_notify_prefs_json("", &format!(r#"{{"mode":"{mode}"}}"#), 1)
            .unwrap();
        (store, search, account, folder)
    }

    #[test]
    fn rules_run_before_the_notification_decision() {
        let (store, search, account, folder) = triage_setup("all");
        let rule = pebble_core::Rule {
            id: "r".into(),
            name: "read".into(),
            priority: 0,
            conditions: r#"{"operator":"and","conditions":[{"field":"from","op":"contains","value":"robot"}]}"#.into(),
            actions: r#"[{"type":"MarkRead"}]"#.into(),
            is_enabled: true,
            created_at: 0,
            updated_at: 0,
        };
        let engine = pebble_rules::RuleEngine::new(&[rule]);
        let robot = new_inbox_message(&store, &account, &folder, "robot@example.com");
        let person = new_inbox_message(&store, &account, &folder, "ann@example.com");
        let person_id = person.message.id.clone();
        let outcome = process_flush_window(
            &store,
            &search,
            Some(&engine),
            vec![robot, person],
            &mut Vec::new(),
        );
        let notified: Vec<_> = outcome
            .notifications
            .iter()
            .map(|n| n.message_id.clone())
            .collect();
        assert_eq!(
            notified,
            vec![person_id],
            "a rule that marks read must not notify"
        );
        let log = store.list_notification_log(10).unwrap();
        assert!(log
            .iter()
            .any(|row| row.reason == "rule_read" && row.tier == "silent"));
    }

    #[test]
    fn muted_senders_and_repeats_are_not_notified() {
        let (store, search, account, folder) = triage_setup("all");
        store
            .set_sender_pref("", "@noisy.example", "mute", 1)
            .unwrap();
        let muted = new_inbox_message(&store, &account, &folder, "a@noisy.example");
        let person = new_inbox_message(&store, &account, &folder, "ann@example.com");
        let repeat = person.clone();
        let outcome =
            process_flush_window(&store, &search, None, vec![muted, person], &mut Vec::new());
        assert_eq!(outcome.notifications.len(), 1);
        let again = process_flush_window(&store, &search, None, vec![repeat], &mut Vec::new());
        assert!(
            again.notifications.is_empty(),
            "the same message is notified at most once"
        );
        assert!(again.triaged.is_empty());
    }

    #[test]
    fn smart_mode_sends_strangers_to_the_digest_and_known_people_through() {
        let (store, search, account, folder) = triage_setup("smart");
        let mut sent = test_message(&account.id);
        sent.remote_id = "sent".into();
        sent.from_address = account.email.clone();
        sent.to_list = vec![pebble_core::EmailAddress {
            name: None,
            address: "ann@example.com".into(),
        }];
        store.insert_message(&sent, &[]).unwrap();
        store.note_sent_recipients(&sent).unwrap();

        let known = new_inbox_message(&store, &account, &folder, "ann@example.com");
        let stranger = new_inbox_message(&store, &account, &folder, "zed@example.com");
        let mut newsletter = new_inbox_message(&store, &account, &folder, "news@shop.example");
        newsletter.signals.bulk = true;
        let known_id = known.message.id.clone();
        let outcome = process_flush_window(
            &store,
            &search,
            None,
            vec![known, stranger, newsletter],
            &mut Vec::new(),
        );
        let notified: Vec<_> = outcome
            .notifications
            .iter()
            .map(|n| n.message_id.clone())
            .collect();
        assert_eq!(notified, vec![known_id]);
        assert_eq!(store.list_unseen_digest(10).unwrap().len(), 2);
    }

    #[test]
    fn mail_from_the_account_is_remembered_as_sent_and_not_notified() {
        let (store, search, account, folder) = triage_setup("all");
        let mut own = new_inbox_message(&store, &account, &folder, "TEST@example.com");
        own.message.to_list = vec![pebble_core::EmailAddress {
            name: None,
            address: "bob@example.com".into(),
        }];
        let outcome = process_flush_window(&store, &search, None, vec![own], &mut Vec::new());
        assert!(outcome.notifications.is_empty());
        assert_eq!(
            store.sent_count_to(&account.id, "bob@example.com").unwrap(),
            1
        );
    }

    #[test]
    fn a_reply_on_a_followed_thread_completes_the_follow_up_and_notifies() {
        let (store, search, account, folder) = triage_setup("important_only");
        let watch = store
            .create_thread_watch(
                &pebble_store::agent::NewThreadWatch {
                    account_id: account.id.clone(),
                    thread_id: Some("thread-1".into()),
                    subject: "Lease renewal".into(),
                    ..Default::default()
                },
                1,
            )
            .unwrap();
        let mut auto = new_inbox_message(&store, &account, &folder, "landlord@homes.example");
        auto.message.thread_id = Some("thread-1".into());
        auto.signals.auto_submitted = true;
        let mut reply = new_inbox_message(&store, &account, &folder, "landlord@homes.example");
        reply.message.thread_id = Some("thread-1".into());
        let reply_id = reply.message.id.clone();

        let outcome = process_flush_window(&store, &search, None, vec![auto], &mut Vec::new());
        assert!(
            outcome.notifications.is_empty(),
            "an out-of-office reply is not the answer"
        );
        assert_eq!(
            store.get_thread_watch(&watch.id).unwrap().unwrap().status,
            "active"
        );

        let outcome = process_flush_window(&store, &search, None, vec![reply], &mut Vec::new());
        assert_eq!(outcome.notifications.len(), 1);
        assert!(outcome.agent_events_changed);
        let done = store.get_thread_watch(&watch.id).unwrap().unwrap();
        assert_eq!(
            (done.status.as_str(), done.matched_message_id.as_deref()),
            ("done", Some(reply_id.as_str()))
        );
        let events = store.list_agent_events(10, None).unwrap();
        assert_eq!(events[0].kind, "watch_reply");
    }

    #[test]
    fn a_reply_through_a_mailing_list_or_filed_away_still_completes_the_follow_up() {
        let (store, search, account, folder) = triage_setup("important_only");
        let watch = store
            .create_thread_watch(
                &pebble_store::agent::NewThreadWatch {
                    account_id: account.id.clone(),
                    thread_id: Some("thread-2".into()),
                    subject: "Team plan".into(),
                    ..Default::default()
                },
                1,
            )
            .unwrap();
        let mut reply = new_inbox_message(&store, &account, &folder, "colleague@corp.example");
        reply.message.thread_id = Some("thread-2".into());
        reply.signals.bulk = true;
        process_flush_window(&store, &search, None, vec![reply], &mut Vec::new());
        assert_eq!(
            store.get_thread_watch(&watch.id).unwrap().unwrap().status,
            "done"
        );
    }
}
