//! The agent's own initiative (#107): follow-up deadlines, mail that waits
//! for the user's reply, and the daily briefing.
//!
//! Two stages, both rule-based:
//! 1. **Findings** become messages in the conversation (`agent_events`).
//!    Mail waiting for a reply is scored ([`pebble_notify::awaiting`]);
//!    only a recent, clearly personal ask asks for a desktop notification.
//! 2. **Delivery** ([`pebble_notify::delivery`]) decides which pending
//!    messages become a desktop notification now: never while Pebble is in
//!    front or notifications are off, after quiet hours rather than during
//!    them, at most a few a day, spaced out, and coalesced into one.

use std::collections::HashMap;
use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Duration;

use pebble_core::{now_timestamp, Account, Message};
use pebble_notify::awaiting::{
    awaiting_score, request_strength, AwaitingFeatures, AWAITING_THRESHOLD,
};
use pebble_notify::delivery::{
    briefing_due, plan_delivery, DeliveryContext, DeliveryLimits, PendingPush, PushKind,
};
use pebble_store::agent::{AgentEvent, NewAgentEvent};
use pebble_store::Store;
use serde_json::json;
use tauri::{AppHandle, Emitter, Manager};
use tracing::warn;

use crate::commands::notifications::{fill, main_window_has_focus, notification_labels};
use crate::events;
use crate::smart_notify::{load_prefs, local_minute_of_day, NotifyPrefsRecord};
use crate::state::AppState;

const TICK: Duration = Duration::from_secs(60);
/// First inbox scan after startup (lets the first sync land).
const FIRST_SCAN_AFTER: Duration = Duration::from_secs(2 * 60);
/// How often the inbox is scanned for mail awaiting a reply.
const AWAITING_SCAN_EVERY: Duration = Duration::from_secs(15 * 60);
/// Mail older than this is no longer considered.
const AWAITING_MAX_AGE: i64 = 14 * 24 * 60 * 60;
/// Only mail that crossed the waiting threshold recently may interrupt;
/// older findings (first run, back from a trip) go to the conversation and
/// the briefing only.
const AWAITING_PUSH_MAX_AGE: i64 = 3 * 24 * 60 * 60;
/// New findings per scan; the rest wait for the next scan.
const AWAITING_MAX_PER_SCAN: usize = 8;
const EVENT_RETENTION: i64 = 90 * 24 * 60 * 60;
const SNIPPET_CHARS: usize = 140;

/// One message found waiting for the user's reply.
#[derive(Debug, Clone)]
pub struct AwaitingReply {
    pub message: Message,
    pub score: f32,
}

fn sender_label(message: &Message) -> String {
    if message.from_name.trim().is_empty() {
        message.from_address.clone()
    } else {
        message.from_name.trim().to_string()
    }
}

/// Mail in `account` that someone sent the user, that asks for something,
/// and that nobody has answered for the configured time. Scored; skips
/// automated mail, muted and snoozed messages, and ones already raised.
pub fn find_awaiting_replies(
    store: &Store,
    account: &Account,
    prefs: &NotifyPrefsRecord,
    now: i64,
) -> Vec<AwaitingReply> {
    let until = now - i64::from(prefs.awaiting_reply_hours.max(1)) * 3600;
    let since = now - AWAITING_MAX_AGE;
    let ids = match store.awaiting_reply_candidates(&account.id, &account.email, since, until, 200)
    {
        Ok(ids) => ids,
        Err(e) => {
            warn!("Awaiting-reply scan failed for {}: {e}", account.id);
            return Vec::new();
        }
    };
    let messages = store.get_messages_batch(&ids).unwrap_or_default();
    let email = account.email.trim().to_lowercase();
    let mut found = Vec::new();
    for message in messages {
        if store
            .agent_event_exists(&format!("awaiting:{}", message.id))
            .unwrap_or(true)
        {
            continue;
        }
        if store
            .get_snoozed_message(&message.id)
            .ok()
            .flatten()
            .is_some()
        {
            continue;
        }
        let in_to = message
            .to_list
            .iter()
            .any(|addr| addr.address.trim().eq_ignore_ascii_case(&email));
        let in_cc = message
            .cc_list
            .iter()
            .any(|addr| addr.address.trim().eq_ignore_ascii_case(&email));
        if !in_to && !in_cc {
            continue;
        }
        let signals = store.message_signals(&message.id).unwrap_or_default();
        if signals.auto_submitted
            || signals.bulk
            || signals.provider_low_priority
            || pebble_notify::is_automated_sender(&message.from_address, &message.from_name)
        {
            continue;
        }
        let pref = store
            .sender_pref_for(&account.id, &message.from_address)
            .ok()
            .flatten();
        if pref.as_deref() == Some("mute") {
            continue;
        }
        let vip = pref.as_deref() == Some("vip")
            || store
                .get_contact_by_email(&message.from_address)
                .ok()
                .flatten()
                .is_some_and(|contact| contact.is_favorite);
        let features = AwaitingFeatures {
            request: Some(request_strength(&message.subject, &message.body_text)),
            sent_count: store
                .sent_count_to(&account.id, &message.from_address)
                .unwrap_or(0),
            direct: in_to,
            only_recipient: in_to && message.to_list.len() == 1 && message.cc_list.is_empty(),
            vip,
            age_hours: ((now - message.date).max(0) as f32) / 3600.0,
        };
        let score = awaiting_score(&features);
        if score >= AWAITING_THRESHOLD {
            found.push(AwaitingReply { message, score });
        }
    }
    // Most pressing first, then newest.
    found.sort_by(|a, b| {
        b.score
            .partial_cmp(&a.score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then(b.message.date.cmp(&a.message.date))
    });
    found.truncate(AWAITING_MAX_PER_SCAN);
    found
}

/// Record each finding as a message in the conversation. Recent ones ask
/// for a desktop notification. Returns how many were new.
pub fn record_awaiting_replies(store: &Store, found: &[AwaitingReply], now: i64) -> usize {
    let mut recorded = 0;
    for item in found {
        let message = &item.message;
        let snippet: String = message.snippet.chars().take(SNIPPET_CHARS).collect();
        let event = NewAgentEvent {
            kind: "awaiting_reply".into(),
            dedupe_key: format!("awaiting:{}", message.id),
            account_id: Some(message.account_id.clone()),
            message_id: Some(message.id.clone()),
            thread_id: message.thread_id.clone(),
            payload: json!({
                "from": sender_label(message),
                "fromAddress": message.from_address,
                "subject": message.subject,
                "snippet": snippet,
                "receivedAt": message.date,
                "score": item.score,
            }),
            push: now - message.date <= AWAITING_PUSH_MAX_AGE,
            ..Default::default()
        };
        match store.insert_agent_event(&event, now) {
            Ok(Some(_)) => recorded += 1,
            Ok(None) => {}
            Err(e) => warn!("Failed to record awaiting-reply nudge: {e}"),
        }
    }
    recorded
}

/// A person's reply to `watch` that is already stored (bots excluded).
fn stored_reply(store: &Store, watch: &pebble_store::agent::ThreadWatch) -> Option<Message> {
    store
        .stored_replies_for_watch(watch)
        .unwrap_or_default()
        .into_iter()
        .find(|m| !pebble_notify::is_automated_sender(&m.from_address, &m.from_name))
}

/// Complete open follow-ups whose reply is already stored but was missed
/// (it came while the follow-up was paused, or a sync stored it quietly).
/// Runs at the scan cadence. Returns how many.
pub fn complete_answered_follow_ups(store: &Store, now: i64) -> usize {
    let mut completed = 0;
    for watch in store.open_thread_watches().unwrap_or_default() {
        if watch.status != "active" {
            continue;
        }
        if let Some(reply) = stored_reply(store, &watch) {
            if crate::smart_notify::complete_watch(store, &watch, &reply, now, true) {
                completed += 1;
            }
        }
    }
    completed
}

/// Follow-ups whose deadline passed without a reply. Returns how many.
pub fn record_overdue_follow_ups(store: &Store, now: i64) -> usize {
    let overdue = store.overdue_thread_watches(now).unwrap_or_default();
    let mut recorded = 0;
    for watch in overdue {
        // The answer may already be here (it came while the follow-up was
        // paused, or was filed away): complete it instead of nudging.
        if let Some(reply) = stored_reply(store, &watch) {
            crate::smart_notify::complete_watch(store, &watch, &reply, now, true);
            recorded += 1;
            continue;
        }
        let link = store.watch_link_message(&watch).ok().flatten();
        let sent = link
            .as_deref()
            .and_then(|id| store.get_message(id).ok().flatten());
        let event = NewAgentEvent {
            kind: "watch_overdue".into(),
            dedupe_key: format!(
                "watch_overdue:{}:{}",
                watch.id,
                watch.deadline.unwrap_or_default()
            ),
            account_id: Some(watch.account_id.clone()),
            message_id: link.clone(),
            thread_id: watch.thread_id.clone(),
            watch_id: Some(watch.id.clone()),
            conversation_id: watch.conversation_id.clone(),
            payload: json!({
                "title": watch.title,
                "createdAt": watch.created_at,
                "deadline": watch.deadline,
                "participants": watch.participants,
                "anchorMessageId": watch.anchor_message_id,
                "subject": sent.as_ref().map(|m| m.subject.clone()),
                "snippet": sent.as_ref().map(|m| m.snippet.chars().take(SNIPPET_CHARS).collect::<String>()),
                "sentAt": sent.as_ref().map(|m| m.date),
            }),
            push: true,
        };
        if let Err(e) = store.insert_agent_event(&event, now) {
            warn!("Failed to record overdue follow-up: {e}");
            continue;
        }
        if let Err(e) = store.mark_thread_watch_nudged(&watch.id, now) {
            warn!("Failed to mark follow-up nudged: {e}");
        }
        recorded += 1;
    }
    recorded
}

/// "Remind me tomorrow" answers that are due: if the mail is still
/// unanswered, raise it again (once). Returns how many.
pub fn record_later_reminders(store: &Store, now: i64) -> usize {
    let mut recorded = 0;
    for event in store.list_agent_events(500, None).unwrap_or_default() {
        if event.kind != "awaiting_reply" {
            continue;
        }
        let Some(resolution) = &event.resolution else {
            continue;
        };
        if resolution["action"] != "later" || resolution["until"].as_i64().unwrap_or(i64::MAX) > now
        {
            continue;
        }
        let Some(message_id) = &event.message_id else {
            continue;
        };
        if store.message_answered_or_gone(message_id).unwrap_or(true) {
            continue;
        }
        let mut payload = event.payload.clone();
        payload["again"] = json!(true);
        let again = NewAgentEvent {
            kind: "awaiting_reply".into(),
            dedupe_key: format!("awaiting_again:{message_id}:{}", resolution["until"]),
            account_id: event.account_id.clone(),
            message_id: event.message_id.clone(),
            thread_id: event.thread_id.clone(),
            payload,
            push: true,
            ..Default::default()
        };
        if let Ok(Some(_)) = store.insert_agent_event(&again, now) {
            recorded += 1;
        }
    }
    recorded
}

/// Mark "waiting for your reply" messages whose mail has since been
/// answered (or deleted) as settled. Returns how many.
pub fn settle_answered(store: &Store, now: i64) -> usize {
    let mut settled = 0;
    for event in store.list_agent_events(500, None).unwrap_or_default() {
        if event.kind != "awaiting_reply" || event.resolution.is_some() {
            continue;
        }
        let Some(message_id) = &event.message_id else {
            continue;
        };
        if store.message_answered_or_gone(message_id).unwrap_or(false)
            && store
                .resolve_agent_event(&event.id, &json!({ "action": "answered", "at": now }), now)
                .is_ok()
        {
            settled += 1;
        }
    }
    settled
}

/// The daily briefing's content.
pub fn build_briefing(store: &Store, now: i64) -> serde_json::Value {
    let day = 24 * 60 * 60;
    let important_unread = store.count_unread_interrupts(now - day).unwrap_or(0);
    let important_ids = store
        .list_unread_interrupts(now - day, 5)
        .unwrap_or_default();
    let important: Vec<serde_json::Value> = store
        .get_messages_batch(&important_ids)
        .unwrap_or_default()
        .into_iter()
        .map(|message| {
            json!({
                "messageId": message.id,
                "accountId": message.account_id,
                "from": sender_label(&message),
                "fromAddress": message.from_address,
                "subject": message.subject,
                "receivedAt": message.date,
            })
        })
        .collect();
    let awaiting: Vec<serde_json::Value> = store
        .list_agent_events(200, None)
        .unwrap_or_default()
        .into_iter()
        .filter(|event| event.kind == "awaiting_reply" && event.resolution.is_none())
        .filter(|event| {
            event
                .message_id
                .as_deref()
                .and_then(|id| store.get_message(id).ok().flatten())
                .is_some_and(|message| !message.is_deleted)
        })
        .take(10)
        .map(|event| {
            json!({
                "messageId": event.message_id,
                "accountId": event.account_id,
                "from": event.payload.get("from"),
                "fromAddress": event.payload.get("fromAddress"),
                "subject": event.payload.get("subject"),
                "receivedAt": event.payload.get("receivedAt"),
            })
        })
        .collect();
    let watches = store.open_thread_watches().unwrap_or_default();
    let overdue = watches
        .iter()
        .filter(|watch| {
            watch.status == "active" && watch.deadline.is_some_and(|deadline| deadline <= now)
        })
        .count();
    let digest = store
        .list_unseen_digest(500)
        .map(|rows| rows.len())
        .unwrap_or(0);
    let deadlines = crate::deadlines::upcoming(store, now, crate::deadlines::BRIEFING_AHEAD, 5);
    json!({
        "deadlines": deadlines,
        "importantUnread": important_unread,
        "important": important,
        "awaitingReply": awaiting,
        "followUpCount": watches.len(),
        "overdueCount": overdue,
        "digestCount": digest,
    })
}

/// Whether a briefing has anything worth interrupting for.
pub fn briefing_has_news(payload: &serde_json::Value) -> bool {
    let count = |key: &str| payload[key].as_u64().unwrap_or(0);
    count("importantUnread") > 0
        || payload["awaitingReply"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
        || count("overdueCount") > 0
        || payload["deadlines"]
            .as_array()
            .is_some_and(|items| !items.is_empty())
}

/// Produce today's briefing if it is due. Returns `true` when one was added.
pub fn maybe_record_briefing(
    store: &Store,
    prefs: &NotifyPrefsRecord,
    local_date: &str,
    minute: u16,
    now: i64,
) -> bool {
    if !prefs.briefing_enabled {
        return false;
    }
    let key = format!("briefing:{local_date}");
    let already = store.agent_event_exists(&key).unwrap_or(true);
    if !briefing_due(minute, prefs.briefing_minute, already) {
        return false;
    }
    let payload = build_briefing(store, now);
    let push = briefing_has_news(&payload);
    let event = NewAgentEvent {
        kind: "briefing".into(),
        dedupe_key: key,
        payload,
        push,
        ..Default::default()
    };
    matches!(store.insert_agent_event(&event, now), Ok(Some(_)))
}

fn push_kind(event: &AgentEvent) -> PushKind {
    match event.kind.as_str() {
        "briefing" => PushKind::Briefing,
        "deadline_soon" => PushKind::Urgent,
        _ => PushKind::Normal,
    }
}

/// Text for one desktop notification covering `events`.
pub fn notification_text(
    events: &[&AgentEvent],
    labels: &crate::commands::notifications::NotificationLabels,
) -> String {
    let str_of =
        |event: &AgentEvent, key: &str| event.payload[key].as_str().unwrap_or_default().to_string();
    match events {
        [] => String::new(),
        [one] => match one.kind.as_str() {
            "awaiting_reply" => fill(
                &labels.awaiting_one,
                &[
                    ("from", &str_of(one, "from")),
                    ("subject", &str_of(one, "subject")),
                ],
            ),
            "watch_overdue" => fill(&labels.watch_overdue, &[("title", &str_of(one, "title"))]),
            "deadline_soon" => fill(
                &labels.deadline_soon,
                &[
                    ("subject", &str_of(one, "subject")),
                    ("time", &due_text(one.payload["due"].as_i64().unwrap_or(0))),
                ],
            ),
            "briefing" => {
                let count = |key: &str| match &one.payload[key] {
                    serde_json::Value::Array(items) => items.len().to_string(),
                    value => value.as_u64().unwrap_or(0).to_string(),
                };
                fill(
                    &labels.briefing_body,
                    &[
                        ("important", &count("importantUnread")),
                        ("awaiting", &count("awaitingReply")),
                        ("followups", &count("followUpCount")),
                    ],
                )
            }
            _ => str_of(one, "what"),
        },
        many if many.iter().all(|event| event.kind == "awaiting_reply") => {
            fill(&labels.awaiting_many, &[("n", &many.len().to_string())])
        }
        many => fill(&labels.agent_many, &[("n", &many.len().to_string())]),
    }
}

/// "18:00" today, otherwise "12/04 18:00" (local).
fn due_text(due: i64) -> String {
    let due = crate::deadlines::local_naive(due);
    if due.date() == chrono::Local::now().date_naive() {
        due.format("%H:%M").to_string()
    } else {
        due.format("%m/%d %H:%M").to_string()
    }
}

/// Unix time of the most recent local midnight.
fn local_midnight() -> i64 {
    use chrono::{Local, TimeZone};
    let today = Local::now().date_naive();
    today
        .and_hms_opt(0, 0, 0)
        .and_then(|midnight| Local.from_local_datetime(&midnight).earliest())
        .map(|dt| dt.timestamp())
        .unwrap_or_else(|| now_timestamp() - 86_400)
}

fn local_date() -> String {
    chrono::Local::now().format("%Y-%m-%d").to_string()
}

/// Turn pending messages into desktop notifications per the delivery plan.
fn deliver(app: &AppHandle, store: &Store, prefs: &NotifyPrefsRecord, now: i64) {
    let pending = store.pending_agent_pushes().unwrap_or_default();
    if pending.is_empty() {
        return;
    }
    let (pushes_today, last_push_at) = store
        .agent_push_stats(local_midnight())
        .unwrap_or((0, None));
    let ctx = DeliveryContext {
        now,
        local_minute_of_day: local_minute_of_day(),
        notifications_enabled: app
            .try_state::<AppState>()
            .is_some_and(|state| state.notifications_enabled.load(Ordering::SeqCst)),
        window_focused: main_window_has_focus(app),
        quiet_hours: prefs.quiet_hours,
        pushes_today,
        last_push_at,
    };
    let queue: Vec<PendingPush> = pending
        .iter()
        .map(|event| PendingPush {
            id: event.id.clone(),
            kind: push_kind(event),
            created_at: event.created_at,
        })
        .collect();
    let plan = plan_delivery(&queue, &ctx, &DeliveryLimits::default());
    if !plan.in_app.is_empty() {
        if let Err(e) = store.set_agent_push_state(&plan.in_app, false, now) {
            warn!("Failed to update agent push state: {e}");
        }
    }
    let by_id: HashMap<&str, &AgentEvent> = pending
        .iter()
        .map(|event| (event.id.as_str(), event))
        .collect();
    let labels = notification_labels();
    for group in &plan.notify {
        let events: Vec<&AgentEvent> = group
            .iter()
            .filter_map(|id| by_id.get(id.as_str()).copied())
            .collect();
        let body = notification_text(&events, &labels);
        let result = match events.as_slice() {
            [one] if one.message_id.is_some() && one.account_id.is_some() => {
                crate::commands::indexing::show_agent_notification(
                    app,
                    &labels.agent_title,
                    &body,
                    one.account_id.as_deref().unwrap_or_default(),
                    one.message_id.as_deref().unwrap_or_default(),
                )
            }
            _ => crate::commands::notifications::show_desktop_notification(
                app,
                &labels.agent_title,
                &body,
            ),
        };
        if let Err(e) = result {
            warn!("Failed to show agent notification: {e}");
        }
        // Mark as pushed even if the OS refused: never retry in a loop.
        if let Err(e) = store.set_agent_push_state(group, true, now) {
            warn!("Failed to update agent push state: {e}");
        }
    }
}

/// One pass: findings (scan when `scan` is set), briefing, then delivery.
async fn tick(app: &AppHandle, store: &Arc<Store>, scan: bool) {
    let store_for_findings = Arc::clone(store);
    let changed = tokio::task::spawn_blocking(move || {
        let store = store_for_findings;
        let now = now_timestamp();
        let prefs = load_prefs(&store, "");
        let mut changed = record_overdue_follow_ups(&store, now) > 0;
        changed |= record_later_reminders(&store, now) > 0;
        changed |= settle_answered(&store, now) > 0;
        changed |= crate::deadlines::tick(&store, now);
        if scan {
            changed |= complete_answered_follow_ups(&store, now) > 0;
        }
        if scan && prefs.awaiting_reply_enabled {
            for account in store.list_accounts().unwrap_or_default() {
                let found = find_awaiting_replies(&store, &account, &prefs, now);
                changed |= record_awaiting_replies(&store, &found, now) > 0;
            }
        }
        changed |= maybe_record_briefing(&store, &prefs, &local_date(), local_minute_of_day(), now);
        changed
    })
    .await
    .unwrap_or(false);
    if changed {
        let _ = app.emit(events::AGENT_EVENTS_CHANGED, ());
    }
    let app = app.clone();
    let store = Arc::clone(store);
    let _ = tokio::task::spawn_blocking(move || {
        let prefs = load_prefs(&store, "");
        deliver(&app, &store, &prefs, now_timestamp());
    })
    .await;
}

/// Run the proactive worker until the app exits.
pub async fn run(app: AppHandle, store: Arc<Store>) {
    tokio::time::sleep(Duration::from_secs(45)).await;
    let started = tokio::time::Instant::now();
    let mut last_scan: Option<tokio::time::Instant> = None;
    let mut last_prune: Option<tokio::time::Instant> = None;
    loop {
        let due = started.elapsed() >= FIRST_SCAN_AFTER
            && last_scan.is_none_or(|at| at.elapsed() >= AWAITING_SCAN_EVERY);
        if due {
            last_scan = Some(tokio::time::Instant::now());
        }
        tick(&app, &store, due).await;
        if last_prune.is_none_or(|at| at.elapsed() >= Duration::from_secs(24 * 60 * 60)) {
            last_prune = Some(tokio::time::Instant::now());
            let store = Arc::clone(&store);
            let _ = tokio::task::spawn_blocking(move || {
                store.prune_agent_events(now_timestamp() - EVENT_RETENTION)
            })
            .await;
        }
        tokio::time::sleep(TICK).await;
    }
}

#[cfg(test)]
mod tests;
