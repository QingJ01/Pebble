//! Commands for smart notifications (#107): preferences, VIP / muted
//! senders, the decision log and the digest.

use std::collections::HashMap;

use crate::events;
use crate::smart_notify::{load_prefs, NotifyPrefsRecord};
use crate::state::AppState;
use pebble_core::{now_timestamp, PebbleError};
use pebble_store::notify::{NotificationLogRow, SenderPrefRow};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

type CmdResult<T> = std::result::Result<T, PebbleError>;

/// Defaults plus the accounts that override them.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotifySettings {
    pub defaults: NotifyPrefsRecord,
    pub accounts: HashMap<String, NotifyPrefsRecord>,
}

#[tauri::command]
pub async fn get_notify_settings(state: State<'_, AppState>) -> CmdResult<NotifySettings> {
    let mut accounts = HashMap::new();
    for (account_id, json) in state.store.list_notify_prefs_json()? {
        if account_id.is_empty() {
            continue;
        }
        if let Ok(prefs) = serde_json::from_str::<NotifyPrefsRecord>(&json) {
            accounts.insert(account_id, prefs);
        }
    }
    Ok(NotifySettings {
        defaults: load_prefs(&state.store, ""),
        accounts,
    })
}

/// Save preferences for an account, or the defaults when `account_id` is
/// empty / absent.
#[tauri::command]
pub async fn set_notify_prefs(
    state: State<'_, AppState>,
    app: AppHandle,
    account_id: Option<String>,
    prefs: NotifyPrefsRecord,
) -> CmdResult<()> {
    if let Some(quiet) = prefs.quiet_hours {
        if quiet.start_minute >= 24 * 60 || quiet.end_minute >= 24 * 60 {
            return Err(PebbleError::Validation(
                "Quiet hours must be within a day".into(),
            ));
        }
    }
    if prefs.model_assist {
        if let Some(account) = account_id
            .as_deref()
            .filter(|id| !id.is_empty())
            .and_then(|id| state.store.get_account(id).ok().flatten())
        {
            if !crate::smart_notify::model_assist_allowed(&account) {
                return Err(PebbleError::Validation(
                    "AI help with notifications isn't available for Gmail accounts yet".into(),
                ));
            }
        }
    }
    let json = serde_json::to_string(&prefs).map_err(|e| PebbleError::Internal(e.to_string()))?;
    state.store.set_notify_prefs_json(
        account_id.as_deref().unwrap_or(""),
        &json,
        now_timestamp(),
    )?;
    let _ = app.emit(events::NOTIFY_CHANGED, ());
    Ok(())
}

/// Make an account follow the defaults again.
#[tauri::command]
pub async fn reset_account_notify_prefs(
    state: State<'_, AppState>,
    account_id: String,
) -> CmdResult<()> {
    if account_id.is_empty() {
        return Err(PebbleError::Validation(
            "The defaults cannot be reset".into(),
        ));
    }
    state.store.delete_notify_prefs(&account_id)
}

#[tauri::command]
pub async fn list_sender_prefs(state: State<'_, AppState>) -> CmdResult<Vec<SenderPrefRow>> {
    state.store.list_sender_prefs()
}

/// Mark a sender (address or `@domain`) as VIP or muted, for one account or
/// all of them. Returns the normalised pattern.
#[tauri::command]
pub async fn set_sender_pref(
    state: State<'_, AppState>,
    app: AppHandle,
    account_id: Option<String>,
    pattern: String,
    kind: String,
) -> CmdResult<String> {
    let stored = state.store.set_sender_pref(
        account_id.as_deref().unwrap_or(""),
        &pattern,
        &kind,
        now_timestamp(),
    )?;
    let _ = app.emit(events::NOTIFY_CHANGED, ());
    Ok(stored)
}

#[tauri::command]
pub async fn remove_sender_pref(
    state: State<'_, AppState>,
    app: AppHandle,
    account_id: Option<String>,
    pattern: String,
) -> CmdResult<()> {
    state
        .store
        .remove_sender_pref(account_id.as_deref().unwrap_or(""), &pattern)?;
    let _ = app.emit(events::NOTIFY_CHANGED, ());
    Ok(())
}

/// A logged decision with enough of the message to show it.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct NotificationEntry {
    pub account_id: String,
    pub message_id: String,
    pub tier: String,
    pub reason: String,
    pub why: Option<String>,
    pub delivered: bool,
    pub created_at: i64,
    pub seen: bool,
    pub subject: String,
    pub from_name: String,
    pub from_address: String,
    pub thread_id: Option<String>,
    pub date: i64,
    /// The user's correction: `true` "should notify", `false` "not important".
    pub feedback: Option<bool>,
}

fn with_messages(
    state: &AppState,
    rows: Vec<NotificationLogRow>,
) -> CmdResult<Vec<NotificationEntry>> {
    let ids: Vec<String> = rows.iter().map(|row| row.message_id.clone()).collect();
    let messages: HashMap<String, pebble_core::Message> = state
        .store
        .get_messages_batch(&ids)?
        .into_iter()
        .map(|message| (message.id.clone(), message))
        .collect();
    Ok(rows
        .into_iter()
        .filter_map(|row| {
            let message = messages.get(&row.message_id)?;
            if message.is_deleted {
                return None;
            }
            let feedback = state
                .store
                .notify_feedback_for(&row.account_id, &row.message_id)
                .ok()
                .flatten();
            Some(NotificationEntry {
                account_id: row.account_id,
                message_id: row.message_id,
                tier: row.tier,
                reason: row.reason,
                why: row.why,
                delivered: row.delivered,
                created_at: row.created_at,
                seen: row.seen_at.is_some(),
                subject: message.subject.clone(),
                from_name: message.from_name.clone(),
                from_address: message.from_address.clone(),
                thread_id: message.thread_id.clone(),
                date: message.date,
                feedback,
            })
        })
        .collect())
}

/// Recent decisions ("why was / wasn't I notified"), newest first.
#[tauri::command]
pub async fn list_notification_log(
    state: State<'_, AppState>,
    limit: Option<u32>,
) -> CmdResult<Vec<NotificationEntry>> {
    let rows = state.store.list_notification_log(limit.unwrap_or(100))?;
    with_messages(&state, rows)
}

/// Unseen digest entries, newest first.
#[tauri::command]
pub async fn list_digest(
    state: State<'_, AppState>,
    limit: Option<u32>,
) -> CmdResult<Vec<NotificationEntry>> {
    let rows = state.store.list_unseen_digest(limit.unwrap_or(200))?;
    with_messages(&state, rows)
}

#[tauri::command]
pub async fn mark_digest_seen(state: State<'_, AppState>, app: AppHandle) -> CmdResult<usize> {
    let count = state.store.mark_digest_seen(now_timestamp())?;
    let _ = app.emit(events::NOTIFY_CHANGED, ());
    Ok(count)
}

/// Corrections in one direction after which a sender rule is suggested.
const SUGGEST_AFTER: u32 = 3;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FeedbackResult {
    pub sender: String,
    /// `vip` or `mute` when the user keeps correcting this sender the same
    /// way and has no rule for them yet.
    pub suggest: Option<String>,
}

/// The user says a message was not important (`important = false`) or
/// should have notified (`true`). Kept on this device; recent corrections
/// guide model-assisted notifications.
#[tauri::command]
pub async fn notify_feedback(
    state: State<'_, AppState>,
    app: AppHandle,
    account_id: String,
    message_id: String,
    important: bool,
) -> CmdResult<FeedbackResult> {
    let message = state
        .store
        .get_message(&message_id)?
        .filter(|message| message.account_id == account_id)
        .ok_or_else(|| PebbleError::Validation("Message not found".into()))?;
    let count = state.store.record_notify_feedback(
        &account_id,
        &message_id,
        &message.from_address,
        &message.subject,
        important,
        now_timestamp(),
    )?;
    let has_rule = state
        .store
        .sender_pref_for(&account_id, &message.from_address)?
        .is_some();
    let suggest = (count >= SUGGEST_AFTER && !has_rule)
        .then(|| if important { "vip" } else { "mute" }.to_string());
    let _ = app.emit(events::NOTIFY_CHANGED, ());
    Ok(FeedbackResult {
        sender: message.from_address.trim().to_lowercase(),
        suggest,
    })
}
