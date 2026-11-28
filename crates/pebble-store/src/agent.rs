//! Follow-ups (thread watches) and the agent activity feed (#107, M2).

use pebble_core::{new_id, normalize_subject, Message, PebbleError, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::Store;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ThreadWatch {
    pub id: String,
    pub account_id: String,
    pub thread_id: Option<String>,
    pub subject_key: String,
    /// Lower-case addresses whose reply we wait for; empty = anyone else.
    pub participants: Vec<String>,
    pub anchor_message_id: Option<String>,
    pub title: String,
    pub status: String,
    pub source: String,
    pub conversation_id: Option<String>,
    pub created_at: i64,
    pub deadline: Option<i64>,
    pub nudged_at: Option<i64>,
    pub matched_message_id: Option<String>,
    pub matched_at: Option<i64>,
    pub updated_at: i64,
    /// The message to open for this follow-up (filled by
    /// [`Store::watch_link_message`]; not stored).
    #[serde(default)]
    pub link_message_id: Option<String>,
}

/// What a new follow-up waits for.
#[derive(Debug, Clone, Default)]
pub struct NewThreadWatch {
    pub account_id: String,
    pub thread_id: Option<String>,
    pub subject: String,
    pub participants: Vec<String>,
    pub anchor_message_id: Option<String>,
    pub source: String,
    pub conversation_id: Option<String>,
    pub deadline: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AgentEvent {
    pub id: String,
    pub kind: String,
    pub dedupe_key: String,
    pub account_id: Option<String>,
    pub message_id: Option<String>,
    pub thread_id: Option<String>,
    pub watch_id: Option<String>,
    pub conversation_id: Option<String>,
    pub payload: serde_json::Value,
    pub created_at: i64,
    pub read_at: Option<i64>,
    pub dismissed_at: Option<i64>,
    /// `pending` / `pushed` / `in_app`.
    pub push_state: String,
    pub pushed_at: Option<i64>,
    /// The user's answer, e.g. `{"action":"snooze","days":2,"at":…}`.
    pub resolution: Option<serde_json::Value>,
}

#[derive(Debug, Clone, Default)]
pub struct NewAgentEvent {
    pub kind: String,
    pub dedupe_key: String,
    pub account_id: Option<String>,
    pub message_id: Option<String>,
    pub thread_id: Option<String>,
    pub watch_id: Option<String>,
    pub conversation_id: Option<String>,
    pub payload: serde_json::Value,
    /// Ask for a desktop notification (`pending`); otherwise in the app only.
    pub push: bool,
}

/// Subject key used to match replies: normalised (Re:/Fwd: stripped) and
/// lower-cased.
pub fn subject_key(subject: &str) -> String {
    normalize_subject(subject).trim().to_lowercase()
}

const WATCH_COLUMNS: &str =
    "id, account_id, thread_id, subject_key, participants_json, anchor_message_id, \
     title, status, source, conversation_id, created_at, deadline, nudged_at, matched_message_id, \
     matched_at, updated_at";

fn row_to_watch(row: &rusqlite::Row) -> rusqlite::Result<ThreadWatch> {
    let participants_json: String = row.get(4)?;
    Ok(ThreadWatch {
        id: row.get(0)?,
        account_id: row.get(1)?,
        thread_id: row.get(2)?,
        subject_key: row.get(3)?,
        participants: serde_json::from_str(&participants_json).unwrap_or_default(),
        anchor_message_id: row.get(5)?,
        title: row.get(6)?,
        status: row.get(7)?,
        source: row.get(8)?,
        conversation_id: row.get(9)?,
        created_at: row.get(10)?,
        deadline: row.get(11)?,
        nudged_at: row.get(12)?,
        matched_message_id: row.get(13)?,
        matched_at: row.get(14)?,
        updated_at: row.get(15)?,
        link_message_id: None,
    })
}

const EVENT_COLUMNS: &str = "id, kind, dedupe_key, account_id, message_id, thread_id, watch_id, \
     conversation_id, payload_json, created_at, read_at, dismissed_at, push_state, pushed_at, resolution_json";

fn row_to_event(row: &rusqlite::Row) -> rusqlite::Result<AgentEvent> {
    let payload: String = row.get(8)?;
    Ok(AgentEvent {
        id: row.get(0)?,
        kind: row.get(1)?,
        dedupe_key: row.get(2)?,
        account_id: row.get(3)?,
        message_id: row.get(4)?,
        thread_id: row.get(5)?,
        watch_id: row.get(6)?,
        conversation_id: row.get(7)?,
        payload: serde_json::from_str(&payload).unwrap_or(serde_json::Value::Null),
        created_at: row.get(9)?,
        read_at: row.get(10)?,
        dismissed_at: row.get(11)?,
        push_state: row.get(12)?,
        pushed_at: row.get(13)?,
        resolution: row
            .get::<_, Option<String>>(14)?
            .and_then(|json| serde_json::from_str(&json).ok()),
    })
}

impl ThreadWatch {
    /// Whether `message` (already known to be from someone else, and not
    /// automated) is the reply this watch waits for.
    pub fn matches(&self, message: &Message) -> bool {
        if self.status != "active" || message.account_id != self.account_id {
            return false;
        }
        if self.anchor_message_id.as_deref() == Some(message.id.as_str()) {
            return false;
        }
        let from = message.from_address.trim().to_lowercase();
        let from_ok = self.participants.is_empty() || self.participants.contains(&from);
        if let (Some(watched), Some(thread)) = (&self.thread_id, &message.thread_id) {
            if watched == thread {
                return from_ok;
            }
        }
        !self.subject_key.is_empty() && from_ok && subject_key(&message.subject) == self.subject_key
    }
}

impl Store {
    pub fn create_thread_watch(&self, input: &NewThreadWatch, now: i64) -> Result<ThreadWatch> {
        if input.thread_id.is_none() && subject_key(&input.subject).is_empty() {
            return Err(PebbleError::Validation(
                "A follow-up needs a thread or a subject".into(),
            ));
        }
        let participants: Vec<String> = input
            .participants
            .iter()
            .map(|p| p.trim().to_lowercase())
            .filter(|p| p.contains('@'))
            .collect();
        let anchor = input.anchor_message_id.clone();
        let watch = ThreadWatch {
            id: new_id(),
            account_id: input.account_id.clone(),
            thread_id: input.thread_id.clone(),
            subject_key: subject_key(&input.subject),
            participants,
            anchor_message_id: input.anchor_message_id.clone(),
            title: normalize_subject(&input.subject).trim().to_string(),
            status: "active".into(),
            source: if input.source.is_empty() {
                "ui".into()
            } else {
                input.source.clone()
            },
            conversation_id: input.conversation_id.clone(),
            created_at: now,
            deadline: input.deadline,
            nudged_at: None,
            matched_message_id: None,
            matched_at: None,
            updated_at: now,
            link_message_id: None,
        };
        self.with_write(|conn| {
            // Re-sending the same outgoing mail replaces its follow-up.
            if let Some(anchor) = &anchor {
                conn.execute(
                    "UPDATE thread_watches SET status = 'cancelled', updated_at = ?2
                     WHERE anchor_message_id = ?1 AND status IN ('active', 'paused')",
                    params![anchor, now],
                )?;
            }
            // One active follow-up per thread: replace the previous one.
            if let Some(thread_id) = &watch.thread_id {
                conn.execute(
                    "UPDATE thread_watches SET status = 'cancelled', updated_at = ?3
                     WHERE account_id = ?1 AND thread_id = ?2 AND status IN ('active', 'paused')",
                    params![watch.account_id, thread_id, now],
                )?;
            }
            conn.execute(
                &format!("INSERT INTO thread_watches ({WATCH_COLUMNS}) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16)"),
                params![
                    watch.id,
                    watch.account_id,
                    watch.thread_id,
                    watch.subject_key,
                    serde_json::to_string(&watch.participants).unwrap_or_else(|_| "[]".into()),
                    watch.anchor_message_id,
                    watch.title,
                    watch.status,
                    watch.source,
                    watch.conversation_id,
                    watch.created_at,
                    watch.deadline,
                    watch.nudged_at,
                    watch.matched_message_id,
                    watch.matched_at,
                    watch.updated_at
                ],
            )?;
            Ok(())
        })?;
        Ok(watch)
    }

    pub fn get_thread_watch(&self, id: &str) -> Result<Option<ThreadWatch>> {
        self.with_read(|conn| {
            Ok(conn
                .query_row(
                    &format!("SELECT {WATCH_COLUMNS} FROM thread_watches WHERE id = ?1"),
                    params![id],
                    row_to_watch,
                )
                .optional()?)
        })
    }

    /// Watches that are active or paused, newest first, plus recently
    /// finished ones (`finished_since`).
    pub fn list_thread_watches(&self, finished_since: i64) -> Result<Vec<ThreadWatch>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {WATCH_COLUMNS} FROM thread_watches
                 WHERE status IN ('active', 'paused') OR updated_at >= ?1
                 ORDER BY CASE status WHEN 'active' THEN 0 WHEN 'paused' THEN 1 ELSE 2 END, created_at DESC
                 LIMIT 200"
            ))?;
            let rows = stmt
                .query_map(params![finished_since], row_to_watch)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn active_thread_watches(&self, account_id: &str) -> Result<Vec<ThreadWatch>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare_cached(&format!(
                "SELECT {WATCH_COLUMNS} FROM thread_watches WHERE account_id = ?1 AND status = 'active'"
            ))?;
            let rows = stmt
                .query_map(params![account_id], row_to_watch)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// The active watch thread for `thread_id`, if any.
    pub fn watch_for_thread(
        &self,
        account_id: &str,
        thread_id: &str,
    ) -> Result<Option<ThreadWatch>> {
        self.with_read(|conn| {
            Ok(conn
                .query_row(
                    &format!(
                        "SELECT {WATCH_COLUMNS} FROM thread_watches
                         WHERE account_id = ?1 AND thread_id = ?2 AND status IN ('active', 'paused')
                         ORDER BY created_at DESC LIMIT 1"
                    ),
                    params![account_id, thread_id],
                    row_to_watch,
                )
                .optional()?)
        })
    }

    /// Change status (`active` / `paused` / `cancelled`) or deadline.
    pub fn update_thread_watch(
        &self,
        id: &str,
        status: Option<&str>,
        deadline: Option<Option<i64>>,
        now: i64,
    ) -> Result<Option<ThreadWatch>> {
        if let Some(status) = status {
            if !matches!(status, "active" | "paused" | "cancelled") {
                return Err(PebbleError::Validation(format!(
                    "Unknown follow-up status: {status}"
                )));
            }
        }
        self.with_write(|conn| {
            if let Some(status) = status {
                conn.execute(
                    "UPDATE thread_watches SET status = ?2, updated_at = ?3
                     WHERE id = ?1 AND status IN ('active', 'paused')",
                    params![id, status, now],
                )?;
            }
            if let Some(deadline) = deadline {
                conn.execute(
                    "UPDATE thread_watches SET deadline = ?2, nudged_at = NULL, updated_at = ?3 WHERE id = ?1",
                    params![id, deadline, now],
                )?;
            }
            Ok(())
        })?;
        self.get_thread_watch(id)
    }

    /// Record the reply that satisfied a watch. Returns `false` if it was
    /// no longer active (already matched elsewhere).
    pub fn complete_thread_watch(&self, id: &str, message_id: &str, now: i64) -> Result<bool> {
        self.with_write(|conn| {
            Ok(conn.execute(
                "UPDATE thread_watches
                 SET status = 'done', matched_message_id = ?2, matched_at = ?3, updated_at = ?3
                 WHERE id = ?1 AND status = 'active'",
                params![id, message_id, now],
            )? == 1)
        })
    }

    /// Active watches past their deadline that have not been nudged yet.
    pub fn overdue_thread_watches(&self, now: i64) -> Result<Vec<ThreadWatch>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {WATCH_COLUMNS} FROM thread_watches
                 WHERE status = 'active' AND deadline IS NOT NULL AND deadline <= ?1 AND nudged_at IS NULL
                 ORDER BY deadline"
            ))?;
            let rows = stmt
                .query_map(params![now], row_to_watch)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn mark_thread_watch_nudged(&self, id: &str, now: i64) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "UPDATE thread_watches SET nudged_at = ?2, updated_at = ?2 WHERE id = ?1",
                params![id, now],
            )?;
            Ok(())
        })
    }

    /// Insert an event unless one with the same `dedupe_key` exists.
    /// Returns the new event, or `None` for a duplicate.
    pub fn insert_agent_event(
        &self,
        input: &NewAgentEvent,
        now: i64,
    ) -> Result<Option<AgentEvent>> {
        let event = AgentEvent {
            id: new_id(),
            kind: input.kind.clone(),
            dedupe_key: input.dedupe_key.clone(),
            account_id: input.account_id.clone(),
            message_id: input.message_id.clone(),
            thread_id: input.thread_id.clone(),
            watch_id: input.watch_id.clone(),
            conversation_id: input.conversation_id.clone(),
            payload: input.payload.clone(),
            created_at: now,
            read_at: None,
            dismissed_at: None,
            push_state: if input.push {
                "pending".into()
            } else {
                "in_app".into()
            },
            pushed_at: None,
            resolution: None,
        };
        let inserted = self.with_write(|conn| {
            Ok(conn.execute(
                &format!(
                    "INSERT OR IGNORE INTO agent_events ({EVENT_COLUMNS})
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, NULL, NULL, ?11, NULL, NULL)"
                ),
                params![
                    event.id,
                    event.kind,
                    event.dedupe_key,
                    event.account_id,
                    event.message_id,
                    event.thread_id,
                    event.watch_id,
                    event.conversation_id,
                    serde_json::to_string(&event.payload).unwrap_or_else(|_| "{}".into()),
                    event.created_at,
                    event.push_state
                ],
            )? == 1)
        })?;
        Ok(inserted.then_some(event))
    }

    pub fn agent_event_exists(&self, dedupe_key: &str) -> Result<bool> {
        self.with_read(|conn| {
            Ok(conn
                .prepare_cached("SELECT 1 FROM agent_events WHERE dedupe_key = ?1")?
                .exists(params![dedupe_key])?)
        })
    }

    /// Feed entries, newest first, skipping dismissed ones.
    pub fn list_agent_events(&self, limit: u32, before: Option<i64>) -> Result<Vec<AgentEvent>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {EVENT_COLUMNS} FROM agent_events
                 WHERE dismissed_at IS NULL AND created_at < ?2
                 ORDER BY created_at DESC LIMIT ?1"
            ))?;
            let rows = stmt
                .query_map(
                    params![limit.min(500), before.unwrap_or(i64::MAX)],
                    row_to_event,
                )?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn count_unread_agent_events(&self) -> Result<u32> {
        self.with_read(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM agent_events WHERE read_at IS NULL AND dismissed_at IS NULL",
                [],
                |row| row.get(0),
            )?;
            Ok(count.clamp(0, u32::MAX as i64) as u32)
        })
    }

    pub fn mark_agent_events_read(&self, now: i64) -> Result<usize> {
        self.with_write(|conn| {
            Ok(conn.execute(
                "UPDATE agent_events SET read_at = ?1 WHERE read_at IS NULL",
                params![now],
            )?)
        })
    }

    pub fn dismiss_agent_event(&self, id: &str, now: i64) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "UPDATE agent_events SET dismissed_at = ?2, read_at = COALESCE(read_at, ?2) WHERE id = ?1",
                params![id, now],
            )?;
            Ok(())
        })
    }

    /// Inbox messages from others, received in `[since, until]`, that are
    /// the latest message of their thread (nobody, including the user,
    /// has written since). Newest first. The caller decides whether each
    /// one actually asks for a reply.
    pub fn awaiting_reply_candidates(
        &self,
        account_id: &str,
        account_email: &str,
        since: i64,
        until: i64,
        limit: u32,
    ) -> Result<Vec<String>> {
        let email = account_email.trim().to_lowercase();
        self.with_read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT m.id FROM messages m
                 WHERE m.account_id = ?1 AND m.is_deleted = 0 AND m.is_draft = 0
                   AND m.date >= ?3 AND m.date <= ?4
                   AND lower(trim(m.from_address)) <> ?2
                   AND EXISTS (
                       SELECT 1 FROM message_folders mf JOIN folders f ON f.id = mf.folder_id
                       WHERE mf.message_id = m.id AND f.role = 'inbox')
                   AND (m.thread_id IS NULL OR NOT EXISTS (
                       SELECT 1 FROM messages later
                       WHERE later.thread_id = m.thread_id AND later.is_deleted = 0
                         AND later.is_draft = 0 AND later.id <> m.id
                         AND (later.date > m.date OR (later.date = m.date AND later.id > m.id))))
                 ORDER BY m.date DESC LIMIT ?5",
            )?;
            let ids = stmt
                .query_map(
                    params![account_id, email, since, until, limit.min(500)],
                    |row| row.get(0),
                )?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(ids)
        })
    }

    /// Whether anyone (the user included) wrote in `thread_id` after
    /// `message_id` (by date), or the message is gone.
    pub fn message_answered_or_gone(&self, message_id: &str) -> Result<bool> {
        self.with_read(|conn| {
            let row: Option<(Option<String>, i64, i64)> = conn
                .query_row(
                    "SELECT thread_id, date, is_deleted FROM messages WHERE id = ?1",
                    params![message_id],
                    |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
                )
                .optional()?;
            let Some((thread_id, date, deleted)) = row else {
                return Ok(true);
            };
            if deleted != 0 {
                return Ok(true);
            }
            let Some(thread_id) = thread_id else {
                return Ok(false);
            };
            Ok(conn
                .prepare_cached(
                    "SELECT 1 FROM messages WHERE thread_id = ?1 AND id <> ?2 AND is_deleted = 0
                       AND is_draft = 0 AND date > ?3 LIMIT 1",
                )?
                .exists(params![thread_id, message_id, date])?)
        })
    }

    /// The message to open for a follow-up: the reply that answered it, the
    /// message it was created from, the newest message of its thread, or
    /// (for mail followed while sending) the account's sent copy with the
    /// same subject to the same people.
    pub fn watch_link_message(&self, watch: &ThreadWatch) -> Result<Option<String>> {
        if let Some(reply) = &watch.matched_message_id {
            return Ok(Some(reply.clone()));
        }
        if let Some(anchor) = watch
            .anchor_message_id
            .as_deref()
            .filter(|a| !a.starts_with("send:"))
        {
            if self.get_message(anchor)?.is_some_and(|m| !m.is_deleted) {
                return Ok(Some(anchor.to_string()));
            }
        }
        self.with_read(|conn| {
            if let Some(thread_id) = &watch.thread_id {
                let newest: Option<String> = conn
                    .query_row(
                        "SELECT id FROM messages WHERE thread_id = ?1 AND account_id = ?2
                           AND is_deleted = 0 AND is_draft = 0 ORDER BY date DESC LIMIT 1",
                        params![thread_id, watch.account_id],
                        |row| row.get(0),
                    )
                    .optional()?;
                if newest.is_some() {
                    return Ok(newest);
                }
            }
            let mut stmt = conn.prepare_cached(
                "SELECT m.id, m.subject, m.to_list, m.cc_list FROM messages m
                 JOIN accounts a ON a.id = m.account_id
                 WHERE m.account_id = ?1 AND m.is_deleted = 0 AND m.is_draft = 0
                   AND lower(trim(m.from_address)) = lower(trim(a.email))
                   AND m.date >= ?2
                 ORDER BY m.date DESC LIMIT 200",
            )?;
            let rows = stmt.query_map(
                params![watch.account_id, watch.created_at - 24 * 60 * 60],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                    ))
                },
            )?;
            for row in rows {
                let (id, subject, to, cc) = row?;
                if subject_key(&subject) != watch.subject_key {
                    continue;
                }
                let recipients = format!("{to}{cc}").to_lowercase();
                if watch.participants.is_empty()
                    || watch
                        .participants
                        .iter()
                        .any(|p| recipients.contains(p.as_str()))
                {
                    return Ok(Some(id));
                }
            }
            Ok(None)
        })
    }

    /// Unread messages that notified (tier interrupt) since `since`, newest first.
    pub fn list_unread_interrupts(&self, since: i64, limit: u32) -> Result<Vec<String>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT l.message_id FROM notification_log l JOIN messages m ON m.id = l.message_id
                 WHERE l.tier = 'interrupt' AND l.created_at >= ?1 AND m.is_read = 0 AND m.is_deleted = 0
                 ORDER BY m.date DESC LIMIT ?2",
            )?;
            let ids = stmt
                .query_map(params![since, limit.min(50)], |row| row.get(0))?
                .collect::<rusqlite::Result<Vec<String>>>()?;
            Ok(ids)
        })
    }

    /// Messages that notified (tier interrupt) since `since` and are still unread.
    pub fn count_unread_interrupts(&self, since: i64) -> Result<u32> {
        self.with_read(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM notification_log l JOIN messages m ON m.id = l.message_id
                 WHERE l.tier = 'interrupt' AND l.created_at >= ?1 AND m.is_read = 0 AND m.is_deleted = 0",
                params![since],
                |row| row.get(0),
            )?;
            Ok(count.clamp(0, u32::MAX as i64) as u32)
        })
    }

    /// Active / paused follow-ups waiting for a reply, oldest first.
    pub fn open_thread_watches(&self) -> Result<Vec<ThreadWatch>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {WATCH_COLUMNS} FROM thread_watches WHERE status IN ('active', 'paused')
                 ORDER BY created_at"
            ))?;
            let rows = stmt
                .query_map([], row_to_watch)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// Cancel follow-ups created for an outgoing send (`anchor`), e.g. when
    /// the send is undone.
    pub fn cancel_thread_watches_by_anchor(&self, anchor: &str, now: i64) -> Result<usize> {
        self.with_write(|conn| {
            Ok(conn.execute(
                "UPDATE thread_watches SET status = 'cancelled', updated_at = ?2
                 WHERE anchor_message_id = ?1 AND status IN ('active', 'paused')",
                params![anchor, now],
            )?)
        })
    }

    /// Events waiting for a desktop notification, oldest first.
    pub fn pending_agent_pushes(&self) -> Result<Vec<AgentEvent>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {EVENT_COLUMNS} FROM agent_events
                 WHERE push_state = 'pending' AND dismissed_at IS NULL ORDER BY created_at"
            ))?;
            let rows = stmt
                .query_map([], row_to_event)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// Mark events as pushed (with `now`) or as in-app only.
    pub fn set_agent_push_state(&self, ids: &[String], pushed: bool, now: i64) -> Result<()> {
        self.with_write(|conn| {
            let mut stmt = conn.prepare_cached(
                "UPDATE agent_events SET push_state = ?2, pushed_at = ?3 WHERE id = ?1",
            )?;
            for id in ids {
                if pushed {
                    stmt.execute(params![id, "pushed", now])?;
                } else {
                    stmt.execute(params![id, "in_app", Option::<i64>::None])?;
                }
            }
            Ok(())
        })
    }

    /// Distinct desktop notifications sent since `since` (events pushed
    /// together count once) and the time of the latest.
    pub fn agent_push_stats(&self, since: i64) -> Result<(u32, Option<i64>)> {
        self.with_read(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(DISTINCT pushed_at) FROM agent_events WHERE pushed_at >= ?1",
                params![since],
                |row| row.get(0),
            )?;
            let last: Option<i64> =
                conn.query_row("SELECT MAX(pushed_at) FROM agent_events", [], |row| {
                    row.get(0)
                })?;
            Ok((count.clamp(0, u32::MAX as i64) as u32, last))
        })
    }

    /// Record the user's answer to an event (and mark it read).
    pub fn resolve_agent_event(
        &self,
        id: &str,
        resolution: &serde_json::Value,
        now: i64,
    ) -> Result<Option<AgentEvent>> {
        let json =
            serde_json::to_string(resolution).map_err(|e| PebbleError::Internal(e.to_string()))?;
        self.with_write(|conn| {
            conn.execute(
                "UPDATE agent_events SET resolution_json = ?2, read_at = COALESCE(read_at, ?3),
                     push_state = CASE WHEN push_state = 'pending' THEN 'in_app' ELSE push_state END
                 WHERE id = ?1",
                params![id, json, now],
            )?;
            Ok(())
        })?;
        self.with_read(|conn| {
            Ok(conn
                .query_row(
                    &format!("SELECT {EVENT_COLUMNS} FROM agent_events WHERE id = ?1"),
                    params![id],
                    row_to_event,
                )
                .optional()?)
        })
    }

    pub fn prune_agent_events(&self, before: i64) -> Result<usize> {
        self.with_write(|conn| {
            Ok(conn.execute(
                "DELETE FROM agent_events WHERE created_at < ?1",
                params![before],
            )?)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_core::EmailAddress;

    fn message(account: &str, from: &str, subject: &str, thread: Option<&str>) -> Message {
        Message {
            id: new_id(),
            account_id: account.into(),
            remote_id: new_id(),
            message_id_header: None,
            in_reply_to: None,
            references_header: None,
            thread_id: thread.map(str::to_string),
            subject: subject.into(),
            snippet: String::new(),
            from_address: from.into(),
            from_name: String::new(),
            to_list: vec![EmailAddress {
                name: None,
                address: "me@example.com".into(),
            }],
            cc_list: vec![],
            bcc_list: vec![],
            body_text: String::new(),
            body_html_raw: String::new(),
            has_attachments: false,
            is_read: false,
            is_starred: false,
            is_draft: false,
            date: 0,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn watches_match_by_thread_or_subject_and_people() {
        let store = Store::open_in_memory().unwrap();
        let by_thread = store
            .create_thread_watch(
                &NewThreadWatch {
                    account_id: "a".into(),
                    thread_id: Some("t1".into()),
                    subject: "Lease renewal".into(),
                    ..Default::default()
                },
                10,
            )
            .unwrap();
        assert!(by_thread.matches(&message(
            "a",
            "landlord@x.example",
            "Something else",
            Some("t1")
        )));
        assert!(!by_thread.matches(&message(
            "b",
            "landlord@x.example",
            "Re: Lease renewal",
            Some("t1")
        )));

        let by_subject = store
            .create_thread_watch(
                &NewThreadWatch {
                    account_id: "a".into(),
                    subject: "Proposal".into(),
                    participants: vec!["Dana@Partner.example".into()],
                    ..Default::default()
                },
                10,
            )
            .unwrap();
        assert!(by_subject.matches(&message("a", "dana@partner.example", "RE: Proposal", None)));
        assert!(!by_subject.matches(&message("a", "other@partner.example", "Re: Proposal", None)));
        assert!(!by_subject.matches(&message("a", "dana@partner.example", "Unrelated", None)));
        assert!(store
            .create_thread_watch(
                &NewThreadWatch {
                    account_id: "a".into(),
                    ..Default::default()
                },
                1
            )
            .is_err());
    }

    #[test]
    fn watch_lifecycle_and_deadlines() {
        let store = Store::open_in_memory().unwrap();
        let input = NewThreadWatch {
            account_id: "a".into(),
            thread_id: Some("t1".into()),
            subject: "Visa".into(),
            deadline: Some(100),
            ..Default::default()
        };
        let first = store.create_thread_watch(&input, 10).unwrap();
        let second = store.create_thread_watch(&input, 11).unwrap();
        assert_eq!(
            store.get_thread_watch(&first.id).unwrap().unwrap().status,
            "cancelled",
            "one follow-up per thread"
        );
        assert_eq!(
            store.watch_for_thread("a", "t1").unwrap().unwrap().id,
            second.id
        );

        assert!(store.overdue_thread_watches(99).unwrap().is_empty());
        assert_eq!(store.overdue_thread_watches(100).unwrap().len(), 1);
        store.mark_thread_watch_nudged(&second.id, 100).unwrap();
        assert!(
            store.overdue_thread_watches(200).unwrap().is_empty(),
            "nudged once"
        );

        store
            .update_thread_watch(&second.id, Some("paused"), None, 120)
            .unwrap();
        assert!(store.active_thread_watches("a").unwrap().is_empty());
        assert!(
            !store.complete_thread_watch(&second.id, "m", 130).unwrap(),
            "paused watches don't complete"
        );
        store
            .update_thread_watch(&second.id, Some("active"), Some(Some(500)), 140)
            .unwrap();
        let resumed = store.get_thread_watch(&second.id).unwrap().unwrap();
        assert_eq!((resumed.deadline, resumed.nudged_at), (Some(500), None));
        assert!(store.complete_thread_watch(&second.id, "m", 150).unwrap());
        assert!(!store.complete_thread_watch(&second.id, "m2", 160).unwrap());
        assert!(store
            .update_thread_watch(&second.id, Some("bogus"), None, 1)
            .is_err());
        assert_eq!(store.list_thread_watches(0).unwrap().len(), 2);
    }

    #[test]
    fn awaiting_reply_candidates_are_unanswered_inbox_mail_from_others() {
        use pebble_core::{Account, Folder, FolderRole, FolderType, ProviderType};
        let store = Store::open_in_memory().unwrap();
        let account = Account {
            id: "a".into(),
            email: "me@example.com".into(),
            display_name: "Me".into(),
            color: None,
            account_label: None,
            provider_display_name: None,
            provider: ProviderType::Imap,
            created_at: 0,
            updated_at: 0,
        };
        store.insert_account(&account).unwrap();
        let inbox = Folder {
            id: "inbox".into(),
            account_id: "a".into(),
            remote_id: "INBOX".into(),
            name: "Inbox".into(),
            folder_type: FolderType::Folder,
            role: Some(FolderRole::Inbox),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 0,
        };
        store.insert_folder(&inbox).unwrap();
        let put = |from: &str, thread: &str, date: i64, folders: &[String]| {
            let mut m = message("a", from, "Question", Some(thread));
            m.date = date;
            store.insert_message(&m, folders).unwrap();
            m.id
        };
        let inbox_ids = vec!["inbox".to_string()];
        let waiting = put("ann@example.com", "t1", 100, &inbox_ids);
        put("bob@example.com", "t2", 100, &inbox_ids);
        put("me@example.com", "t2", 200, &[]);
        put("cat@example.com", "t3", 100, &[]);
        put("dan@example.com", "t4", 900, &inbox_ids);
        put("me@example.com", "t5", 100, &inbox_ids);

        let ids = store
            .awaiting_reply_candidates("a", "Me@Example.com", 0, 500, 10)
            .unwrap();
        assert_eq!(ids, vec![waiting]);
    }

    #[test]
    fn undoing_a_send_cancels_its_follow_up() {
        let store = Store::open_in_memory().unwrap();
        let input = NewThreadWatch {
            account_id: "a".into(),
            subject: "Proposal".into(),
            anchor_message_id: Some("send:job-1".into()),
            ..Default::default()
        };
        let first = store.create_thread_watch(&input, 1).unwrap();
        let second = store.create_thread_watch(&input, 2).unwrap();
        assert_eq!(
            store.get_thread_watch(&first.id).unwrap().unwrap().status,
            "cancelled"
        );
        assert_eq!(
            store
                .cancel_thread_watches_by_anchor("send:job-1", 3)
                .unwrap(),
            1
        );
        assert_eq!(
            store.get_thread_watch(&second.id).unwrap().unwrap().status,
            "cancelled"
        );
        assert!(store.open_thread_watches().unwrap().is_empty());
    }

    #[test]
    fn events_dedupe_and_read_state() {
        let store = Store::open_in_memory().unwrap();
        let event = NewAgentEvent {
            kind: "awaiting_reply".into(),
            dedupe_key: "awaiting:m1".into(),
            message_id: Some("m1".into()),
            payload: serde_json::json!({"days": 2}),
            ..Default::default()
        };
        let created = store.insert_agent_event(&event, 5).unwrap().unwrap();
        assert!(store.insert_agent_event(&event, 6).unwrap().is_none());
        assert!(store.agent_event_exists("awaiting:m1").unwrap());
        assert_eq!(store.count_unread_agent_events().unwrap(), 1);
        store.mark_agent_events_read(7).unwrap();
        assert_eq!(store.count_unread_agent_events().unwrap(), 0);
        assert_eq!(created.push_state, "in_app");
        let pushy = store
            .insert_agent_event(
                &NewAgentEvent {
                    dedupe_key: "awaiting:m2".into(),
                    push: true,
                    ..event.clone()
                },
                6,
            )
            .unwrap()
            .unwrap();
        assert_eq!(store.pending_agent_pushes().unwrap().len(), 1);
        store
            .set_agent_push_state(std::slice::from_ref(&pushy.id), true, 100)
            .unwrap();
        assert!(store.pending_agent_pushes().unwrap().is_empty());
        assert_eq!(store.agent_push_stats(50).unwrap(), (1, Some(100)));
        let resolved = store
            .resolve_agent_event(
                &pushy.id,
                &serde_json::json!({"action": "snooze", "days": 2}),
                120,
            )
            .unwrap()
            .unwrap();
        assert_eq!(resolved.resolution.unwrap()["days"], 2);
        assert_eq!(resolved.read_at, Some(120));
        store.dismiss_agent_event(&pushy.id, 8).unwrap();
        store.dismiss_agent_event(&created.id, 8).unwrap();
        assert!(store.list_agent_events(10, None).unwrap().is_empty());
        assert_eq!(store.prune_agent_events(6).unwrap(), 1);
    }
}
