//! Commands for follow-ups and the agent activity feed (#107, M2).

use crate::events;
use crate::smart_notify::is_from_account;
use crate::state::AppState;
use pebble_core::{now_timestamp, PebbleError};
use pebble_store::agent::{AgentEvent, NewThreadWatch, ThreadWatch};
use tauri::{AppHandle, Emitter, State};

type CmdResult<T> = std::result::Result<T, PebbleError>;

/// Follow a thread or an outgoing message.
///
/// - From a received / sent message (`message_id`): waits for a reply in
///   its thread from anyone else (or, for the user's own mail, from its
///   recipients).
/// - For mail being sent (`anchor` = `send:<id>`, no stored message yet):
///   waits for a reply from `participants` with the same subject.
#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn create_thread_watch(
    state: State<'_, AppState>,
    app: AppHandle,
    account_id: String,
    message_id: Option<String>,
    subject: Option<String>,
    participants: Option<Vec<String>>,
    deadline: Option<i64>,
    anchor: Option<String>,
) -> CmdResult<ThreadWatch> {
    let mut input = NewThreadWatch {
        account_id: account_id.clone(),
        subject: subject.unwrap_or_default(),
        participants: participants.unwrap_or_default(),
        anchor_message_id: anchor,
        source: "ui".into(),
        deadline,
        ..Default::default()
    };
    if let Some(message_id) = message_id {
        let message = state
            .store
            .get_message(&message_id)?
            .ok_or_else(|| PebbleError::Validation("Message not found".into()))?;
        if message.account_id != account_id {
            return Err(PebbleError::Validation(
                "Message belongs to another account".into(),
            ));
        }
        let account = state
            .store
            .get_account(&account_id)?
            .ok_or_else(|| PebbleError::Validation("Account not found".into()))?;
        input.thread_id = message.thread_id.clone();
        if input.subject.is_empty() {
            input.subject = message.subject.clone();
        }
        if input.participants.is_empty() && is_from_account(&message, &account) {
            input.participants = message
                .to_list
                .iter()
                .chain(&message.cc_list)
                .map(|addr| addr.address.clone())
                .collect();
        }
        if input.anchor_message_id.is_none() {
            input.anchor_message_id = Some(message.id);
        }
    }
    let watch = state.store.create_thread_watch(&input, now_timestamp())?;
    let _ = app.emit(events::AGENT_EVENTS_CHANGED, ());
    Ok(watch)
}

#[tauri::command]
pub async fn list_thread_watches(state: State<'_, AppState>) -> CmdResult<Vec<ThreadWatch>> {
    // Active and paused, plus anything finished in the last week.
    state
        .store
        .list_thread_watches(now_timestamp() - 7 * 24 * 60 * 60)
}

#[tauri::command]
pub async fn get_thread_watch_for(
    state: State<'_, AppState>,
    account_id: String,
    thread_id: String,
) -> CmdResult<Option<ThreadWatch>> {
    state.store.watch_for_thread(&account_id, &thread_id)
}

/// Pause, resume or cancel a follow-up, or change its deadline
/// (`clear_deadline` removes it).
#[tauri::command]
pub async fn update_thread_watch(
    state: State<'_, AppState>,
    app: AppHandle,
    id: String,
    status: Option<String>,
    deadline: Option<i64>,
    clear_deadline: Option<bool>,
) -> CmdResult<Option<ThreadWatch>> {
    let deadline_change = if clear_deadline.unwrap_or(false) {
        Some(None)
    } else {
        deadline.map(Some)
    };
    let watch = state.store.update_thread_watch(
        &id,
        status.as_deref(),
        deadline_change,
        now_timestamp(),
    )?;
    let _ = app.emit(events::AGENT_EVENTS_CHANGED, ());
    Ok(watch)
}

/// Cancel the follow-up created for a send (used when sending fails).
#[tauri::command]
pub async fn cancel_send_follow_up(
    state: State<'_, AppState>,
    app: AppHandle,
    anchor: String,
) -> CmdResult<usize> {
    let count = state
        .store
        .cancel_thread_watches_by_anchor(&anchor, now_timestamp())?;
    let _ = app.emit(events::AGENT_EVENTS_CHANGED, ());
    Ok(count)
}

#[tauri::command]
pub async fn list_agent_events(
    state: State<'_, AppState>,
    limit: Option<u32>,
    before: Option<i64>,
) -> CmdResult<Vec<AgentEvent>> {
    state.store.list_agent_events(limit.unwrap_or(100), before)
}

#[tauri::command]
pub async fn count_unread_agent_events(state: State<'_, AppState>) -> CmdResult<u32> {
    state.store.count_unread_agent_events()
}

#[tauri::command]
pub async fn mark_agent_events_read(
    state: State<'_, AppState>,
    app: AppHandle,
) -> CmdResult<usize> {
    let count = state.store.mark_agent_events_read(now_timestamp())?;
    let _ = app.emit(events::AGENT_EVENTS_CHANGED, ());
    Ok(count)
}

#[tauri::command]
pub async fn dismiss_agent_event(
    state: State<'_, AppState>,
    app: AppHandle,
    id: String,
) -> CmdResult<()> {
    state.store.dismiss_agent_event(&id, now_timestamp())?;
    let _ = app.emit(events::AGENT_EVENTS_CHANGED, ());
    Ok(())
}

/// Answer one of Pebble's messages with a quick reply and act on it:
///
/// - follow-up overdue: `wait` (`days`, default 2) extends the deadline,
///   `stop` cancels the follow-up;
/// - waiting for your reply: `later` (`days`, default 1) raises it again
///   then if still unanswered, `no_reply` never raises it again.
///
/// The choice is stored on the message so the conversation shows it.
#[tauri::command]
pub async fn resolve_agent_event(
    state: State<'_, AppState>,
    app: AppHandle,
    id: String,
    action: String,
    days: Option<u32>,
) -> CmdResult<Option<AgentEvent>> {
    let now = now_timestamp();
    let event = state
        .store
        .list_agent_events(500, None)?
        .into_iter()
        .find(|event| event.id == id)
        .ok_or_else(|| PebbleError::Validation("Message not found".into()))?;
    let day = 24 * 60 * 60;
    let resolution = match (event.kind.as_str(), action.as_str()) {
        ("watch_overdue", "wait") => {
            let days = days.unwrap_or(2).clamp(1, 30);
            let until = now + i64::from(days) * day;
            if let Some(watch_id) = &event.watch_id {
                state.store.update_thread_watch(
                    watch_id,
                    Some("active"),
                    Some(Some(until)),
                    now,
                )?;
            }
            serde_json::json!({ "action": "wait", "days": days, "until": until, "at": now })
        }
        ("watch_overdue", "stop") => {
            if let Some(watch_id) = &event.watch_id {
                state
                    .store
                    .update_thread_watch(watch_id, Some("cancelled"), None, now)?;
            }
            serde_json::json!({ "action": "stop", "at": now })
        }
        ("awaiting_reply", "later") => {
            let days = days.unwrap_or(1).clamp(1, 30);
            serde_json::json!({ "action": "later", "days": days, "until": now + i64::from(days) * day, "at": now })
        }
        ("awaiting_reply", "no_reply") => serde_json::json!({ "action": "no_reply", "at": now }),
        // A deadline handled ("done") or misread ("not_deadline"). Settling
        // the reminder settles the deadline too, so it isn't raised again.
        ("deadline" | "deadline_soon", "done") | ("deadline", "not_deadline") => {
            let resolution = serde_json::json!({ "action": action, "at": now });
            if let Some(message_id) = &event.message_id {
                for other in state.store.list_agent_events(500, None)? {
                    if other.id != event.id
                        && other.resolution.is_none()
                        && other.message_id.as_deref() == Some(message_id)
                        && matches!(other.kind.as_str(), "deadline" | "deadline_soon")
                    {
                        state
                            .store
                            .resolve_agent_event(&other.id, &resolution, now)?;
                    }
                }
            }
            resolution
        }
        (kind, action) => {
            return Err(PebbleError::Validation(format!(
                "Can't answer {kind} with {action}"
            )));
        }
    };
    let updated = state.store.resolve_agent_event(&id, &resolution, now)?;
    let _ = app.emit(events::AGENT_EVENTS_CHANGED, ());
    Ok(updated)
}
