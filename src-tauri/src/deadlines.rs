//! Deadlines found in mail (#107, M3b).
//!
//! When new mail asks for something by a date ("by Friday 5pm",
//! "请于12月5日前回复"), Pebble notes it in the conversation. Before it is
//! due, and if the user hasn't replied yet, Pebble reminds them once. The
//! daily briefing lists what is due in the next few days.
//!
//! Deadlines come from [`pebble_notify::deadline::find_deadline`] (rules,
//! every account) or from the model when model-assisted notifications are
//! on and the rules found none.

use chrono::{Local, NaiveDateTime, TimeZone};
use pebble_core::{Account, Message};
use pebble_store::agent::{AgentEvent, NewAgentEvent};
use pebble_store::Store;
use serde_json::{json, Value};
use tracing::warn;

const HOUR: i64 = 60 * 60;
/// Remind a day ahead for deadlines that were set well in advance…
const LONG_LEAD: i64 = 24 * HOUR;
/// …and a few hours ahead for short ones.
const SHORT_LEAD: i64 = 3 * HOUR;
/// Deadlines further out than this when the mail arrived count as long.
const LONG_NOTICE: i64 = 36 * HOUR;
/// Past due by more than this, a deadline is settled as expired.
const EXPIRE_AFTER: i64 = 24 * HOUR;
/// The briefing looks this far ahead.
pub const BRIEFING_AHEAD: i64 = 3 * 24 * HOUR;
const SNIPPET_CHARS: usize = 140;

pub fn dedupe_key(message_id: &str) -> String {
    format!("deadline:{message_id}")
}

/// Local wall-clock time of a Unix timestamp.
pub fn local_naive(ts: i64) -> NaiveDateTime {
    Local
        .timestamp_opt(ts, 0)
        .single()
        .map(|dt| dt.naive_local())
        .unwrap_or_default()
}

/// Unix time of a local wall-clock time.
pub fn local_ts(naive: NaiveDateTime) -> Option<i64> {
    Local
        .from_local_datetime(&naive)
        .earliest()
        .map(|dt| dt.timestamp())
}

fn sender_label(message: &Message) -> String {
    if message.from_name.trim().is_empty() {
        message.from_address.clone()
    } else {
        message.from_name.trim().to_string()
    }
}

fn body_text(message: &Message) -> String {
    if message.body_text.trim().is_empty() {
        pebble_core::strip_html_for_snippet(&message.body_html_raw)
    } else {
        message.body_text.clone()
    }
}

/// Note a deadline for `message` in the conversation (once per message).
/// Returns `true` when it was new.
pub fn record(
    store: &Store,
    message: &Message,
    due: NaiveDateTime,
    all_day: bool,
    phrase: Option<&str>,
    source: &str,
    now: i64,
) -> bool {
    let Some(due_ts) = local_ts(due) else {
        return false;
    };
    let event = NewAgentEvent {
        kind: "deadline".into(),
        dedupe_key: dedupe_key(&message.id),
        account_id: Some(message.account_id.clone()),
        message_id: Some(message.id.clone()),
        thread_id: message.thread_id.clone(),
        payload: json!({
            "from": sender_label(message),
            "fromAddress": message.from_address,
            "subject": message.subject,
            "snippet": message.snippet.chars().take(SNIPPET_CHARS).collect::<String>(),
            "receivedAt": message.date,
            "due": due_ts,
            "allDay": all_day,
            "phrase": phrase,
            "source": source,
        }),
        // The mail's own notification (if any) already interrupted; the
        // reminder before the deadline is what notifies.
        push: false,
        ..Default::default()
    };
    match store.insert_agent_event(&event, now) {
        Ok(inserted) => inserted.is_some(),
        Err(e) => {
            warn!("Failed to record deadline: {e}");
            false
        }
    }
}

/// Look for a deadline in new mail with the rules. Returns `true` when one
/// was recorded.
pub fn detect(store: &Store, message: &Message, now: i64) -> bool {
    let received = local_naive(message.date);
    let Some(found) =
        pebble_notify::deadline::find_deadline(&message.subject, &body_text(message), received)
    else {
        return false;
    };
    record(
        store,
        message,
        found.due,
        found.all_day,
        Some(&found.phrase),
        "rule",
        now,
    )
}

fn due_of(event: &AgentEvent) -> Option<i64> {
    event.payload["due"].as_i64()
}

/// The user handled it: replied in the thread (or the mail is gone).
fn handled(store: &Store, event: &AgentEvent, accounts: &[Account]) -> bool {
    let Some(message_id) = event.message_id.as_deref() else {
        return true;
    };
    let Some(account) = accounts
        .iter()
        .find(|a| Some(a.id.as_str()) == event.account_id.as_deref())
    else {
        return true;
    };
    store
        .replied_by_after(message_id, &account.email)
        .unwrap_or(false)
}

/// When to remind about a deadline recorded at `created_at`.
pub fn remind_at(due: i64, created_at: i64) -> i64 {
    let lead = if due - created_at > LONG_NOTICE {
        LONG_LEAD
    } else {
        SHORT_LEAD
    };
    due - lead
}

/// Settle handled and long-past deadlines, and remind once about the ones
/// coming up. Returns `true` when the conversation changed.
pub fn tick(store: &Store, now: i64) -> bool {
    let accounts = store.list_accounts().unwrap_or_default();
    let mut changed = false;
    for event in store.list_agent_events(500, None).unwrap_or_default() {
        if event.kind != "deadline" || event.resolution.is_some() {
            continue;
        }
        let Some(due) = due_of(&event) else {
            continue;
        };
        if handled(store, &event, &accounts) {
            changed |= store
                .resolve_agent_event(&event.id, &json!({ "action": "answered", "at": now }), now)
                .is_ok();
            continue;
        }
        if now > due + EXPIRE_AFTER {
            changed |= store
                .resolve_agent_event(&event.id, &json!({ "action": "expired", "at": now }), now)
                .is_ok();
            continue;
        }
        if now < remind_at(due, event.created_at) || now > due {
            continue;
        }
        let Some(message_id) = event.message_id.clone() else {
            continue;
        };
        if store
            .get_snoozed_message(&message_id)
            .ok()
            .flatten()
            .is_some()
        {
            continue;
        }
        let soon = NewAgentEvent {
            kind: "deadline_soon".into(),
            dedupe_key: format!("deadline_soon:{message_id}"),
            account_id: event.account_id.clone(),
            message_id: Some(message_id),
            thread_id: event.thread_id.clone(),
            payload: event.payload.clone(),
            push: true,
            ..Default::default()
        };
        if let Ok(Some(_)) = store.insert_agent_event(&soon, now) {
            changed = true;
        }
    }
    changed
}

/// Open deadlines due between now (minus a grace period) and `ahead`,
/// soonest first, for the briefing and the side column.
pub fn upcoming(store: &Store, now: i64, ahead: i64, limit: usize) -> Vec<Value> {
    let mut items: Vec<(i64, Value)> = store
        .list_agent_events(500, None)
        .unwrap_or_default()
        .into_iter()
        .filter(|event| event.kind == "deadline" && event.resolution.is_none())
        .filter_map(|event| {
            let due = due_of(&event)?;
            (due >= now - 12 * HOUR && due <= now + ahead).then(|| {
                (
                    due,
                    json!({
                        "eventId": event.id,
                        "messageId": event.message_id,
                        "accountId": event.account_id,
                        "from": event.payload.get("from"),
                        "fromAddress": event.payload.get("fromAddress"),
                        "subject": event.payload.get("subject"),
                        "due": due,
                        "allDay": event.payload.get("allDay"),
                    }),
                )
            })
        })
        .collect();
    items.sort_by_key(|(due, _)| *due);
    items.into_iter().take(limit).map(|(_, v)| v).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_core::{EmailAddress, ProviderType};

    fn store_with(account_email: &str) -> (Store, Account) {
        let store = Store::open_in_memory().unwrap();
        let account = Account {
            id: "a1".into(),
            email: account_email.into(),
            display_name: "Me".into(),
            color: None,
            account_label: None,
            provider_display_name: None,
            provider: ProviderType::Imap,
            created_at: 0,
            updated_at: 0,
        };
        store.insert_account(&account).unwrap();
        (store, account)
    }

    fn mail(id: &str, from: &str, body: &str, date: i64) -> Message {
        Message {
            id: id.into(),
            account_id: "a1".into(),
            remote_id: id.into(),
            message_id_header: None,
            in_reply_to: None,
            references_header: None,
            thread_id: Some("t1".into()),
            subject: "Budget".into(),
            snippet: body.into(),
            from_address: from.into(),
            from_name: String::new(),
            to_list: vec![EmailAddress {
                name: None,
                address: "me@x.example".into(),
            }],
            cc_list: vec![],
            bcc_list: vec![],
            body_text: body.into(),
            body_html_raw: String::new(),
            has_attachments: false,
            is_read: false,
            is_starred: false,
            is_draft: false,
            date,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    /// A local Wednesday 10:00, as Unix time.
    fn wednesday() -> i64 {
        local_ts(
            chrono::NaiveDate::from_ymd_opt(2026, 12, 2)
                .unwrap()
                .and_hms_opt(10, 0, 0)
                .unwrap(),
        )
        .unwrap()
    }

    #[test]
    fn reminds_once_before_due_and_settles_when_answered() {
        let (store, _) = store_with("me@x.example");
        let sent = wednesday();
        let ask = mail(
            "m1",
            "dana@x.example",
            "Please send the numbers by Friday.",
            sent,
        );
        store.insert_message(&ask, &[]).unwrap();
        assert!(detect(&store, &ask, sent));
        assert!(!detect(&store, &ask, sent), "once per message");

        let event = &store.list_agent_events(10, None).unwrap()[0];
        let due = event.payload["due"].as_i64().unwrap();
        assert_eq!(local_naive(due).format("%a %H:%M").to_string(), "Fri 18:00");
        assert_eq!(remind_at(due, sent), due - LONG_LEAD);

        assert!(!tick(&store, due - LONG_LEAD - 60), "too early");
        assert!(tick(&store, due - LONG_LEAD + 60));
        assert!(!tick(&store, due - LONG_LEAD + 120), "only one reminder");
        let kinds: Vec<_> = store
            .list_agent_events(10, None)
            .unwrap()
            .into_iter()
            .map(|e| e.kind)
            .collect();
        assert!(kinds.contains(&"deadline_soon".to_string()));
        assert_eq!(upcoming(&store, sent, BRIEFING_AHEAD, 5).len(), 1);

        // Another note from Dana doesn't settle it; the user's reply does.
        store
            .insert_message(&mail("m2", "dana@x.example", "Reminder!", sent + 60), &[])
            .unwrap();
        tick(&store, due - 60);
        assert!(upcoming(&store, sent, BRIEFING_AHEAD, 5).len() == 1);
        store
            .insert_message(&mail("m3", "me@x.example", "Here you go", sent + 120), &[])
            .unwrap();
        assert!(tick(&store, due - 30));
        assert!(upcoming(&store, sent, BRIEFING_AHEAD, 5).is_empty());
    }

    #[test]
    fn expires_unanswered_deadlines_and_ignores_mail_without_one() {
        let (store, _) = store_with("me@x.example");
        let sent = wednesday();
        let plain = mail("m0", "dana@x.example", "See you Friday!", sent);
        assert!(!detect(&store, &plain, sent));
        let ask = mail("m1", "dana@x.example", "请在明天下班前回复", sent);
        store.insert_message(&ask, &[]).unwrap();
        assert!(detect(&store, &ask, sent));
        let due = store.list_agent_events(10, None).unwrap()[0].payload["due"]
            .as_i64()
            .unwrap();
        assert_eq!(remind_at(due, sent), due - SHORT_LEAD, "short notice");
        assert!(tick(&store, due + EXPIRE_AFTER + 1));
        assert!(upcoming(&store, due, BRIEFING_AHEAD, 5).is_empty());
    }
}
