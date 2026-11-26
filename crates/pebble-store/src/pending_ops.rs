use pebble_core::{PebbleError, Result};
use rusqlite::{params, OptionalExtension};
use std::collections::HashSet;

use crate::Store;

mod typed;
pub use typed::{
    DeleteOp, FlagsOp, FolderChangeOp, MailOp, MailOpEnvelope, ManagedMoveOp, PreviousLocalState,
    SendOp,
};

pub const MAX_PENDING_MAIL_OP_ATTEMPTS: i64 = 8;
const BASE_RETRY_DELAY_SECS: i64 = 60;
const MAX_RETRY_DELAY_SECS: i64 = 60 * 60;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingMailOpStatus {
    Pending,
    InProgress,
    Failed,
    Done,
}

impl PendingMailOpStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Pending => "pending",
            Self::InProgress => "in_progress",
            Self::Failed => "failed",
            Self::Done => "done",
        }
    }
}

fn status_from_str(value: &str) -> Result<PendingMailOpStatus> {
    match value {
        "pending" => Ok(PendingMailOpStatus::Pending),
        "in_progress" => Ok(PendingMailOpStatus::InProgress),
        "failed" => Ok(PendingMailOpStatus::Failed),
        "done" => Ok(PendingMailOpStatus::Done),
        other => Err(PebbleError::Storage(format!(
            "Invalid pending mail op status: {other}"
        ))),
    }
}

fn pending_mail_retry_delay_secs(attempts: i64) -> i64 {
    let exponent = attempts.saturating_sub(1).clamp(0, 10) as u32;
    BASE_RETRY_DELAY_SECS
        .saturating_mul(2_i64.saturating_pow(exponent))
        .min(MAX_RETRY_DELAY_SECS)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingMailOp {
    pub id: String,
    pub account_id: String,
    pub message_id: String,
    pub op_type: String,
    pub payload_json: String,
    pub status: PendingMailOpStatus,
    pub attempts: i64,
    pub last_error: Option<String>,
    pub created_at: i64,
    pub updated_at: i64,
    pub next_retry_at: Option<i64>,
}

impl PendingMailOp {
    /// Decode this row's typed op and envelope markers.
    pub fn envelope(&self) -> Result<MailOpEnvelope> {
        MailOpEnvelope::decode(&self.op_type, &self.payload_json)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingMailOpsSummary {
    pub pending_count: i64,
    pub in_progress_count: i64,
    pub failed_count: i64,
    pub total_active_count: i64,
    pub last_error: Option<String>,
    pub updated_at: Option<i64>,
}

/// Result of queueing a pending mail op with supersession rules applied.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingMailOpEnqueue {
    /// A new row was inserted.
    Inserted(String),
    /// The op was folded into the latest not-yet-running op for the message.
    Merged(String),
    /// The new op exactly undoes the latest not-yet-running op, so both were
    /// cancelled. Nothing remains queued for the remote.
    Cancelled { cancelled_op_id: String },
}

/// What happened to local optimistic state when an op was abandoned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingMailOpCompensation {
    /// Local state was never changed (or was already compensated).
    NotNeeded,
    /// Local state was restored from the previous state recorded in the payload.
    Reverted,
    /// Local state may differ from the server and must be refreshed from it.
    NeedsResync,
}

/// Ops that are still active and may run (or are running) before later ops
/// on the same message. Exhausted/stopped ops no longer hold the queue.
fn active_blocking_condition(alias: &str, max_param: &str) -> String {
    format!(
        "({alias}.status IN ('pending', 'in_progress') \
         OR ({alias}.status = 'failed' AND {alias}.attempts < {max_param}))"
    )
}

/// Whether `next` exactly undoes `previous`, so both can be dropped. The op
/// types alone are not enough: unarchive always targets the Inbox, so an
/// archive from Sent or a custom folder followed by an unarchive is a move,
/// not a no-op. Cancel only when the recorded endpoints mirror each other
/// (folder ids for folder-based providers, label edits for Gmail).
fn is_archive_inverse(
    previous_type: &str,
    previous: &MailOpEnvelope,
    next_type: &str,
    next_payload: &str,
) -> bool {
    if !matches!(
        (previous_type, next_type),
        ("archive", "unarchive") | ("unarchive", "archive")
    ) {
        return false;
    }
    let Some(next) = decode_envelope(next_type, next_payload) else {
        return false;
    };
    let (MailOp::Archive(a) | MailOp::Unarchive(a)) = &previous.op else {
        return false;
    };
    let (MailOp::Archive(b) | MailOp::Unarchive(b)) = &next.op else {
        return false;
    };

    let folders_mirror = match (
        a.source_folder_id.as_deref(),
        a.target_id(),
        b.source_folder_id.as_deref(),
        b.target_id(),
    ) {
        (Some(a_src), Some(a_dst), Some(b_src), Some(b_dst)) => a_src == b_dst && a_dst == b_src,
        _ => false,
    };
    let labels_mirror = match (a.labels(), b.labels()) {
        (Some((a_add, a_remove)), Some((b_add, b_remove))) => {
            let set = |labels: &[String]| labels.iter().cloned().collect::<HashSet<_>>();
            !(a_add.is_empty() && a_remove.is_empty())
                && set(a_add) == set(b_remove)
                && set(a_remove) == set(b_add)
        }
        _ => false,
    };
    folders_mirror || labels_mirror
}

/// Decode a stored envelope for bookkeeping. Undecodable rows are treated
/// as legacy rows: never rewritten, and abandoned with a resync.
fn decode_envelope(op_type: &str, payload_json: &str) -> Option<MailOpEnvelope> {
    MailOpEnvelope::decode(op_type, payload_json).ok()
}

/// Fold a later `update_flags` op into an earlier one so only the final flag
/// state is replayed. Returns `None` when the payloads can't be merged safely.
fn merge_update_flags_envelopes(previous: &str, next: &str) -> Option<String> {
    let previous = decode_envelope("update_flags", previous)?;
    let next = decode_envelope("update_flags", next)?;
    if previous.remote_id != next.remote_id {
        return None;
    }
    let (MailOp::UpdateFlags(prev_flags), MailOp::UpdateFlags(next_flags)) =
        (&previous.op, &next.op)
    else {
        return None;
    };
    // Label-only payloads can't be re-derived from booleans; keep them separate.
    if !prev_flags.has_flag_change() || !next_flags.has_flag_change() {
        return None;
    }
    let merged_flags = FlagsOp {
        is_read: next_flags.is_read.or(prev_flags.is_read),
        is_starred: next_flags.is_starred.or(prev_flags.is_starred),
        // Derived from the merged booleans at replay time.
        add_labels: None,
        remove_labels: None,
        folder_remote_id: next_flags
            .folder_remote_id
            .clone()
            .or_else(|| prev_flags.folder_remote_id.clone()),
    };

    let mut merged = next.clone();
    merged.op = MailOp::UpdateFlags(merged_flags);
    // The earliest local commit captured the true pre-change state.
    if previous.local_committed == Some(true) {
        if let Some(state) = previous.previous_state.clone() {
            merged.previous_state = Some(state);
        }
    }
    merged.local_committed = match (previous.local_committed, next.local_committed) {
        (Some(a), Some(b)) => Some(a || b),
        _ => None,
    };
    if merged.local_committed.is_none() {
        merged.previous_state = None;
    }
    Some(merged.to_json())
}

/// One op that was abandoned and the local-state compensation applied for it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AbandonedPendingMailOp {
    pub id: String,
    pub account_id: String,
    pub message_id: String,
    pub compensation: PendingMailOpCompensation,
}

fn compensation_marker(compensation: &PendingMailOpCompensation) -> &'static str {
    match compensation {
        PendingMailOpCompensation::NotNeeded => "not_needed",
        PendingMailOpCompensation::Reverted => "reverted",
        PendingMailOpCompensation::NeedsResync => "needs_resync",
    }
}

/// Restore the local optimistic change made for an abandoned op, using the
/// previous state recorded in its payload. Runs on the caller's transaction.
fn compensate_pending_mail_op_on(
    conn: &rusqlite::Connection,
    id: &str,
) -> Result<Option<AbandonedPendingMailOp>> {
    use serde_json::Value;
    let Some((account_id, message_id, op_type, payload_json, created_at, rowid)) = conn
        .query_row(
            "SELECT account_id, message_id, op_type, payload_json, created_at, rowid
             FROM pending_mail_ops WHERE id = ?1",
            params![id],
            |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, i64>(4)?,
                    row.get::<_, i64>(5)?,
                ))
            },
        )
        .optional()?
    else {
        return Ok(None);
    };
    let mut envelope: Value = serde_json::from_str(&payload_json).unwrap_or(Value::Null);
    let typed = decode_envelope(&op_type, &payload_json);
    let abandoned = |compensation| AbandonedPendingMailOp {
        id: id.to_string(),
        account_id: account_id.clone(),
        message_id: message_id.clone(),
        compensation,
    };
    if envelope.get("compensated").is_some() {
        return Ok(Some(abandoned(PendingMailOpCompensation::NotNeeded)));
    }

    let compensation = match typed {
        // Sends keep their own outbox/needs-attention UX; a remote receipt
        // means the remote already matches the local commit.
        Some(ref typed) if matches!(typed.op, MailOp::Send(_)) || typed.remote_succeeded => {
            PendingMailOpCompensation::NotNeeded
        }
        _ if op_type == "send"
            || envelope.get("remote_succeeded").and_then(Value::as_bool) == Some(true) =>
        {
            PendingMailOpCompensation::NotNeeded
        }
        Some(typed) => match typed.local_committed {
            Some(false) => PendingMailOpCompensation::NotNeeded,
            Some(true) => {
                let later_active: bool = conn.query_row(
                    "SELECT EXISTS(
                        SELECT 1 FROM pending_mail_ops
                        WHERE message_id = ?1 AND id != ?2 AND status != 'done'
                          AND (created_at > ?3 OR (created_at = ?3 AND rowid > ?4)))",
                    params![message_id, id, created_at, rowid],
                    |row| row.get(0),
                )?;
                match typed.previous_state.as_ref() {
                    Some(previous) if !later_active => {
                        if revert_local_state(conn, &message_id, &typed.op, previous)? {
                            PendingMailOpCompensation::Reverted
                        } else {
                            PendingMailOpCompensation::NeedsResync
                        }
                    }
                    _ => PendingMailOpCompensation::NeedsResync,
                }
            }
            // Legacy payloads don't say whether local state was changed.
            None => PendingMailOpCompensation::NeedsResync,
        },
        None => PendingMailOpCompensation::NeedsResync,
    };

    if let Some(object) = envelope.as_object_mut() {
        object.insert(
            "compensated".to_string(),
            Value::String(compensation_marker(&compensation).to_string()),
        );
        conn.execute(
            "UPDATE pending_mail_ops SET payload_json = ?1 WHERE id = ?2",
            params![envelope.to_string(), id],
        )?;
    }
    Ok(Some(abandoned(compensation)))
}

/// Returns `false` when the previous state can't be applied (message gone,
/// an op without a local revert, or recorded folders no longer exist).
fn revert_local_state(
    conn: &rusqlite::Connection,
    message_id: &str,
    op: &MailOp,
    previous: &PreviousLocalState,
) -> Result<bool> {
    let exists: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM messages WHERE id = ?1)",
        params![message_id],
        |row| row.get(0),
    )?;
    if !exists {
        return Ok(false);
    }
    let now = pebble_core::now_timestamp();
    match op {
        MailOp::UpdateFlags(flags) => {
            let is_read = flags.is_read.and(previous.is_read);
            let is_starred = flags.is_starred.and(previous.is_starred);
            if is_read.is_none() && is_starred.is_none() {
                return Ok(false);
            }
            conn.execute(
                "UPDATE messages
                 SET is_read = COALESCE(?1, is_read),
                     is_starred = COALESCE(?2, is_starred),
                     updated_at = ?3
                 WHERE id = ?4",
                params![is_read, is_starred, now, message_id],
            )?;
            Ok(true)
        }
        MailOp::Archive(_)
        | MailOp::Unarchive(_)
        | MailOp::Restore(_)
        | MailOp::Delete(_)
        | MailOp::MoveToFolder(_) => {
            let folder_ids = previous.folder_ids.clone().unwrap_or_default();
            if folder_ids.is_empty() {
                return Ok(false);
            }
            for folder_id in &folder_ids {
                let folder_exists: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM folders WHERE id = ?1)",
                    params![folder_id],
                    |row| row.get(0),
                )?;
                if !folder_exists {
                    return Ok(false);
                }
            }
            conn.execute(
                "DELETE FROM message_folders WHERE message_id = ?1",
                params![message_id],
            )?;
            for folder_id in &folder_ids {
                conn.execute(
                    "INSERT OR IGNORE INTO message_folders (message_id, folder_id) VALUES (?1, ?2)",
                    params![message_id, folder_id],
                )?;
            }
            conn.execute(
                "UPDATE messages
                 SET is_deleted = ?1,
                     deleted_at = ?2,
                     remote_id = COALESCE(?3, remote_id),
                     updated_at = ?4
                 WHERE id = ?5",
                params![
                    previous.is_deleted.unwrap_or(false),
                    previous.deleted_at,
                    previous.remote_id,
                    now,
                    message_id
                ],
            )?;
            Ok(true)
        }
        MailOp::DeletePermanent(_) | MailOp::ManagedMove(_) | MailOp::Send(_) => Ok(false),
    }
}

impl Store {
    pub fn insert_pending_mail_op(
        &self,
        account_id: &str,
        message_id: &str,
        op_type: &str,
        payload_json: &str,
    ) -> Result<String> {
        self.with_write(|conn| {
            let id = pebble_core::new_id();
            let now = pebble_core::now_timestamp();
            conn.execute(
                "INSERT INTO pending_mail_ops
                    (id, account_id, message_id, op_type, payload_json, status, attempts, last_error, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, NULL, ?7, ?7)",
                params![
                    id,
                    account_id,
                    message_id,
                    op_type,
                    payload_json,
                    PendingMailOpStatus::Pending.as_str(),
                    now,
                ],
            )?;
            Ok(id)
        })
    }

    /// Queue a typed op; see [`Store::enqueue_pending_mail_op`].
    pub fn enqueue_mail_op(
        &self,
        account_id: &str,
        message_id: &str,
        envelope: &MailOpEnvelope,
    ) -> Result<PendingMailOpEnqueue> {
        self.enqueue_pending_mail_op(
            account_id,
            message_id,
            envelope.op_type(),
            &envelope.to_json(),
        )
    }

    /// Queue a pending op, collapsing it with the latest not-yet-running op on
    /// the same message when the newer op supersedes it:
    /// - `update_flags` after `update_flags` keeps only the final flag state;
    /// - `archive` after `unarchive` (or vice versa) cancels both when the
    ///   recorded folders (or Gmail label edits) exactly reverse each other.
    pub fn enqueue_pending_mail_op(
        &self,
        account_id: &str,
        message_id: &str,
        op_type: &str,
        payload_json: &str,
    ) -> Result<PendingMailOpEnqueue> {
        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()?;
            let now = pebble_core::now_timestamp();
            let latest = tx
                .query_row(
                    "SELECT id, op_type, status, payload_json, attempts
                     FROM pending_mail_ops
                     WHERE account_id = ?1 AND message_id = ?2 AND status != 'done'
                     ORDER BY created_at DESC, rowid DESC
                     LIMIT 1",
                    params![account_id, message_id],
                    |row| {
                        Ok((
                            row.get::<_, String>(0)?,
                            row.get::<_, String>(1)?,
                            row.get::<_, String>(2)?,
                            row.get::<_, String>(3)?,
                            row.get::<_, i64>(4)?,
                        ))
                    },
                )
                .optional()?;

            if let Some((latest_id, latest_type, latest_status, latest_payload, attempts)) = latest
            {
                // Running, remotely-applied, or abandoned (and possibly already
                // compensated) ops are history; never rewrite them. Rows that
                // can't be decoded are never folded either.
                let not_started = latest_status != PendingMailOpStatus::InProgress.as_str()
                    && attempts < MAX_PENDING_MAIL_OP_ATTEMPTS
                    && decode_envelope(&latest_type, &latest_payload).is_some_and(|envelope| {
                        !envelope.remote_succeeded && envelope.compensated.is_none()
                    });
                if not_started && latest_type == "update_flags" && op_type == "update_flags" {
                    if let Some(merged) = merge_update_flags_envelopes(&latest_payload, payload_json)
                    {
                        tx.execute(
                            "UPDATE pending_mail_ops
                             SET payload_json = ?1, status = ?2, attempts = 0,
                                 last_error = NULL, next_retry_at = NULL, updated_at = ?3
                             WHERE id = ?4",
                            params![
                                merged,
                                PendingMailOpStatus::Pending.as_str(),
                                now,
                                latest_id
                            ],
                        )?;
                        tx.commit()?;
                        return Ok(PendingMailOpEnqueue::Merged(latest_id));
                    }
                }
                let cancels = not_started
                    && decode_envelope(&latest_type, &latest_payload).is_some_and(|previous| {
                        is_archive_inverse(&latest_type, &previous, op_type, payload_json)
                    });
                if cancels {
                    tx.execute(
                        "UPDATE pending_mail_ops
                         SET status = ?1, last_error = NULL, next_retry_at = NULL, updated_at = ?2
                         WHERE id = ?3",
                        params![PendingMailOpStatus::Done.as_str(), now, latest_id],
                    )?;
                    tx.commit()?;
                    return Ok(PendingMailOpEnqueue::Cancelled {
                        cancelled_op_id: latest_id,
                    });
                }
            }

            let id = pebble_core::new_id();
            tx.execute(
                "INSERT INTO pending_mail_ops
                    (id, account_id, message_id, op_type, payload_json, status, attempts, last_error, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, NULL, ?7, ?7)",
                params![
                    id,
                    account_id,
                    message_id,
                    op_type,
                    payload_json,
                    PendingMailOpStatus::Pending.as_str(),
                    now,
                ],
            )?;
            tx.commit()?;
            Ok(PendingMailOpEnqueue::Inserted(id))
        })
    }

    pub fn list_pending_mail_ops(&self, account_id: &str) -> Result<Vec<PendingMailOp>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, account_id, message_id, op_type, payload_json, status,
                        attempts, last_error, created_at, updated_at, next_retry_at
                 FROM pending_mail_ops
                 WHERE account_id = ?1
                 ORDER BY created_at ASC, rowid ASC",
            )?;
            let rows = stmt.query_map(params![account_id], |row| {
                let status: String = row.get(5)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    status,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, Option<i64>>(10)?,
                ))
            })?;

            let mut ops = Vec::new();
            for row in rows {
                let (
                    id,
                    account_id,
                    message_id,
                    op_type,
                    payload_json,
                    status,
                    attempts,
                    last_error,
                    created_at,
                    updated_at,
                    next_retry_at,
                ) = row?;
                ops.push(PendingMailOp {
                    id,
                    account_id,
                    message_id,
                    op_type,
                    payload_json,
                    status: status_from_str(&status)?,
                    attempts,
                    last_error,
                    created_at,
                    updated_at,
                    next_retry_at,
                });
            }
            Ok(ops)
        })
    }

    pub fn list_active_pending_mail_ops(
        &self,
        account_id: Option<&str>,
        limit: i64,
    ) -> Result<Vec<PendingMailOp>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, account_id, message_id, op_type, payload_json, status,
                        attempts, last_error, created_at, updated_at, next_retry_at
                 FROM pending_mail_ops
                 WHERE status != 'done'
                   AND (?1 IS NULL OR account_id = ?1)
                 ORDER BY updated_at DESC
                 LIMIT ?2",
            )?;
            let rows = stmt.query_map(params![account_id, limit.max(1)], |row| {
                let status: String = row.get(5)?;
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, String>(3)?,
                    row.get::<_, String>(4)?,
                    status,
                    row.get::<_, i64>(6)?,
                    row.get::<_, Option<String>>(7)?,
                    row.get::<_, i64>(8)?,
                    row.get::<_, i64>(9)?,
                    row.get::<_, Option<i64>>(10)?,
                ))
            })?;

            let mut ops = Vec::new();
            for row in rows {
                let (
                    id,
                    account_id,
                    message_id,
                    op_type,
                    payload_json,
                    status,
                    attempts,
                    last_error,
                    created_at,
                    updated_at,
                    next_retry_at,
                ) = row?;
                ops.push(PendingMailOp {
                    id,
                    account_id,
                    message_id,
                    op_type,
                    payload_json,
                    status: status_from_str(&status)?,
                    attempts,
                    last_error,
                    created_at,
                    updated_at,
                    next_retry_at,
                });
            }
            Ok(ops)
        })
    }

    /// Ops ready to replay, in creation order. An op is never returned while an
    /// earlier op on the same message is still active (pending, running, or
    /// failed with retries left), so per-message replay order matches the
    /// order the user made the changes in.
    pub fn list_retryable_pending_mail_ops(&self, limit: i64) -> Result<Vec<PendingMailOp>> {
        self.with_read(|conn| {
            let sql = format!(
                "SELECT id, account_id, message_id, op_type, payload_json, status,
                        attempts, last_error, created_at, updated_at, next_retry_at
                 FROM pending_mail_ops
                 WHERE (status = 'pending'
                    OR (
                        status = 'failed'
                        AND attempts < ?2
                        AND (next_retry_at IS NULL OR next_retry_at <= ?3)
                    ))
                   AND NOT EXISTS (
                       SELECT 1 FROM pending_mail_ops claimed
                       WHERE claimed.message_id = pending_mail_ops.message_id
                         AND claimed.status = 'in_progress'
                   )
                   AND NOT EXISTS (
                       SELECT 1 FROM pending_mail_ops earlier
                       WHERE earlier.message_id = pending_mail_ops.message_id
                         AND earlier.id != pending_mail_ops.id
                         AND (earlier.created_at < pending_mail_ops.created_at
                              OR (earlier.created_at = pending_mail_ops.created_at
                                  AND earlier.rowid < pending_mail_ops.rowid))
                         AND {blocking}
                   )
                 ORDER BY created_at ASC, rowid ASC
                 LIMIT ?1",
                blocking = active_blocking_condition("earlier", "?2"),
            );
            let mut stmt = conn.prepare(&sql)?;
            let now = pebble_core::now_timestamp();
            let rows =
                stmt.query_map(params![limit, MAX_PENDING_MAIL_OP_ATTEMPTS, now], |row| {
                    let status: String = row.get(5)?;
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                        row.get::<_, String>(3)?,
                        row.get::<_, String>(4)?,
                        status,
                        row.get::<_, i64>(6)?,
                        row.get::<_, Option<String>>(7)?,
                        row.get::<_, i64>(8)?,
                        row.get::<_, i64>(9)?,
                        row.get::<_, Option<i64>>(10)?,
                    ))
                })?;

            let mut ops = Vec::new();
            for row in rows {
                let (
                    id,
                    account_id,
                    message_id,
                    op_type,
                    payload_json,
                    status,
                    attempts,
                    last_error,
                    created_at,
                    updated_at,
                    next_retry_at,
                ) = row?;
                ops.push(PendingMailOp {
                    id,
                    account_id,
                    message_id,
                    op_type,
                    payload_json,
                    status: status_from_str(&status)?,
                    attempts,
                    last_error,
                    created_at,
                    updated_at,
                    next_retry_at,
                });
            }
            Ok(ops)
        })
    }

    pub fn pending_mail_ops_summary(
        &self,
        account_id: Option<&str>,
    ) -> Result<PendingMailOpsSummary> {
        self.with_read(|conn| {
            let (pending_count, in_progress_count, failed_count, updated_at): (
                i64,
                i64,
                i64,
                Option<i64>,
            ) = conn.query_row(
                "SELECT
                    COALESCE(SUM(CASE WHEN status = 'pending' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status = 'in_progress' THEN 1 ELSE 0 END), 0),
                    COALESCE(SUM(CASE WHEN status = 'failed' THEN 1 ELSE 0 END), 0),
                    MAX(updated_at)
                 FROM pending_mail_ops
                 WHERE status != 'done'
                   AND (?1 IS NULL OR account_id = ?1)",
                params![account_id],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )?;

            let last_error = conn
                .query_row(
                    "SELECT last_error
                     FROM pending_mail_ops
                     WHERE status = 'failed'
                       AND last_error IS NOT NULL
                       AND (?1 IS NULL OR account_id = ?1)
                     ORDER BY updated_at DESC
                     LIMIT 1",
                    params![account_id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?;

            Ok(PendingMailOpsSummary {
                pending_count,
                in_progress_count,
                failed_count,
                total_active_count: pending_count + in_progress_count + failed_count,
                last_error,
                updated_at,
            })
        })
    }

    pub fn mark_pending_mail_op_in_progress(&self, id: &str) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "UPDATE pending_mail_ops
                 SET status = ?1,
                     updated_at = ?2
                 WHERE id = ?3",
                params![
                    PendingMailOpStatus::InProgress.as_str(),
                    pebble_core::now_timestamp(),
                    id,
                ],
            )?;
            Ok(())
        })
    }

    pub fn reset_in_progress_pending_mail_ops(&self) -> Result<()> {
        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()?;
            let interrupted_sends = {
                let mut stmt = tx.prepare(
                    "SELECT id, payload_json
                     FROM pending_mail_ops
                     WHERE status = ?1 AND op_type = 'send'",
                )?;
                let rows = stmt.query_map(
                    params![PendingMailOpStatus::InProgress.as_str()],
                    |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)),
                )?;
                rows.collect::<std::result::Result<Vec<_>, _>>()?
            };
            let now = pebble_core::now_timestamp();
            for (id, payload_json) in interrupted_sends {
                let remote_succeeded = serde_json::from_str::<serde_json::Value>(&payload_json)
                    .ok()
                    .and_then(|payload| {
                        payload
                            .get("remote_succeeded")
                            .and_then(serde_json::Value::as_bool)
                    })
                    .unwrap_or(false);
                if !remote_succeeded {
                    tx.execute(
                        "UPDATE pending_mail_ops
                         SET status = ?1,
                             attempts = ?2,
                             last_error = ?3,
                             updated_at = ?4,
                             next_retry_at = NULL
                         WHERE id = ?5",
                        params![
                            PendingMailOpStatus::Failed.as_str(),
                            MAX_PENDING_MAIL_OP_ATTEMPTS,
                            "Send outcome is unknown after interruption; it was not retried automatically to avoid a duplicate. Check Sent before dismissing or sending again.",
                            now,
                            id,
                        ],
                    )?;
                }
            }
            tx.execute(
                "UPDATE pending_mail_ops
                 SET status = ?1,
                     updated_at = ?2,
                     next_retry_at = NULL
                 WHERE status = ?3",
                params![
                    PendingMailOpStatus::Pending.as_str(),
                    now,
                    PendingMailOpStatus::InProgress.as_str(),
                ],
            )?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn mark_pending_mail_op_remote_succeeded(&self, id: &str) -> Result<()> {
        self.with_write(|conn| {
            let payload_json = conn
                .query_row(
                    "SELECT payload_json FROM pending_mail_ops WHERE id = ?1",
                    params![id],
                    |row| row.get::<_, String>(0),
                )
                .optional()?
                .ok_or_else(|| PebbleError::Storage(format!("Pending mail op not found: {id}")))?;
            let mut payload: serde_json::Value =
                serde_json::from_str(&payload_json).map_err(|error| {
                    PebbleError::Storage(format!("Invalid pending op payload: {error}"))
                })?;
            let object = payload.as_object_mut().ok_or_else(|| {
                PebbleError::Storage("Pending op payload must be a JSON object".to_string())
            })?;
            object.insert(
                "remote_succeeded".to_string(),
                serde_json::Value::Bool(true),
            );
            conn.execute(
                "UPDATE pending_mail_ops
                 SET payload_json = ?1, updated_at = ?2
                 WHERE id = ?3",
                params![payload.to_string(), pebble_core::now_timestamp(), id],
            )?;
            Ok(())
        })
    }

    pub fn mark_pending_mail_op_outcome_unknown(&self, id: &str, error: &str) -> Result<()> {
        self.mark_pending_mail_op_stopped(id, error)
    }

    /// Keep the operation visible, but require manual intervention before another attempt.
    pub fn mark_pending_mail_op_stopped(&self, id: &str, error: &str) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "UPDATE pending_mail_ops
                 SET status = ?1,
                     attempts = ?2,
                     last_error = ?3,
                     updated_at = ?4,
                     next_retry_at = NULL
                 WHERE id = ?5",
                params![
                    PendingMailOpStatus::Failed.as_str(),
                    MAX_PENDING_MAIL_OP_ATTEMPTS,
                    error,
                    pebble_core::now_timestamp(),
                    id,
                ],
            )?;
            Ok(())
        })
    }

    pub fn mark_pending_mail_op_failed(&self, id: &str, error: &str) -> Result<()> {
        self.with_write(|conn| {
            let current_attempts = conn
                .query_row(
                    "SELECT attempts FROM pending_mail_ops WHERE id = ?1",
                    params![id],
                    |row| row.get::<_, i64>(0),
                )
                .optional()?
                .unwrap_or(0);
            let attempts = current_attempts + 1;
            let now = pebble_core::now_timestamp();
            let next_retry_at = if attempts < MAX_PENDING_MAIL_OP_ATTEMPTS {
                Some(now + pending_mail_retry_delay_secs(attempts))
            } else {
                None
            };
            conn.execute(
                "UPDATE pending_mail_ops
                 SET status = ?1,
                     attempts = attempts + 1,
                     last_error = ?2,
                     updated_at = ?3,
                     next_retry_at = ?4
                 WHERE id = ?5",
                params![
                    PendingMailOpStatus::Failed.as_str(),
                    error,
                    now,
                    next_retry_at,
                    id,
                ],
            )?;
            Ok(())
        })
    }

    pub fn dismiss_failed_pending_mail_ops(&self, account_id: Option<&str>) -> Result<u64> {
        Ok(self
            .dismiss_failed_pending_mail_ops_with_compensation(account_id)?
            .len() as u64)
    }

    /// Dismiss failed ops and compensate the local optimistic state each one
    /// left behind. Ops are processed newest-first so that reverting several
    /// ops on one message unwinds back to the oldest recorded state.
    pub fn dismiss_failed_pending_mail_ops_with_compensation(
        &self,
        account_id: Option<&str>,
    ) -> Result<Vec<AbandonedPendingMailOp>> {
        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()?;
            let ids = {
                let mut stmt = tx.prepare(
                    "SELECT id FROM pending_mail_ops
                     WHERE status = 'failed'
                       AND (?1 IS NULL OR account_id = ?1)
                     ORDER BY created_at DESC, rowid DESC",
                )?;
                let rows = stmt.query_map(params![account_id], |row| row.get::<_, String>(0))?;
                rows.collect::<std::result::Result<Vec<_>, _>>()?
            };
            let now = pebble_core::now_timestamp();
            let mut abandoned = Vec::with_capacity(ids.len());
            for id in ids {
                if let Some(op) = compensate_pending_mail_op_on(&tx, &id)? {
                    abandoned.push(op);
                }
                tx.execute(
                    "UPDATE pending_mail_ops
                     SET status = ?1,
                         last_error = NULL,
                         updated_at = ?2,
                         next_retry_at = NULL
                     WHERE id = ?3",
                    params![PendingMailOpStatus::Done.as_str(), now, id],
                )?;
            }
            tx.commit()?;
            Ok(abandoned)
        })
    }

    /// Compensate local optimistic state for an op the worker gave up on
    /// (retries exhausted or a permanent error). Idempotent: an op that was
    /// already compensated reports `NotNeeded`.
    pub fn compensate_abandoned_pending_mail_op(
        &self,
        id: &str,
    ) -> Result<Option<AbandonedPendingMailOp>> {
        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()?;
            let result = compensate_pending_mail_op_on(&tx, id)?;
            tx.commit()?;
            Ok(result)
        })
    }

    pub fn retry_failed_send(&self, id: &str) -> Result<()> {
        self.with_write(|conn| {
            let updated = conn.execute(
                "UPDATE pending_mail_ops SET status = 'pending', attempts = 0,
                 last_error = NULL, next_retry_at = ?1, updated_at = ?1
                 WHERE id = ?2 AND status = 'failed' AND op_type = 'send'
               AND EXISTS (SELECT 1 FROM messages WHERE messages.id = pending_mail_ops.message_id AND messages.is_deleted = 0)",
                params![pebble_core::now_timestamp(), id],
            )?;
            if updated != 1 {
                return Err(pebble_core::PebbleError::Validation("Failed send is no longer available for retry".into()));
            }
            Ok(())
        })
    }

    pub fn mark_pending_mail_op_done(&self, id: &str) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "UPDATE pending_mail_ops
                 SET status = ?1,
                     last_error = NULL,
                     updated_at = ?2,
                     next_retry_at = NULL
                 WHERE id = ?3",
                params![
                    PendingMailOpStatus::Done.as_str(),
                    pebble_core::now_timestamp(),
                    id,
                ],
            )?;
            Ok(())
        })
    }

    #[cfg(test)]
    pub fn force_pending_mail_op_retry_now_for_test(&self, id: &str) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "UPDATE pending_mail_ops SET next_retry_at = ?1 WHERE id = ?2",
                params![pebble_core::now_timestamp() - 1, id],
            )?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stopped_send_only_retries_after_explicit_request_and_preserves_receipt() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let message = test_message(&account.id);
        store.insert_message(&message, &[]).unwrap();
        let id = store
            .insert_pending_mail_op(&account.id, &message.id, "send", "{}")
            .unwrap();
        store
            .mark_pending_mail_op_outcome_unknown(&id, "Check Sent")
            .unwrap();
        assert!(store
            .list_retryable_pending_mail_ops(10)
            .unwrap()
            .is_empty());
        store.retry_failed_send(&id).unwrap();
        let queued = store.list_retryable_pending_mail_ops(10).unwrap();
        assert_eq!(queued.len(), 1);
        assert_eq!(queued[0].id, id);
        assert_eq!(queued[0].attempts, 0);
        assert!(store.retry_failed_send(&id).is_err());
        store.mark_pending_mail_op_remote_succeeded(&id).unwrap();
        store
            .mark_pending_mail_op_stopped(&id, "Local finalize failed")
            .unwrap();
        store.retry_failed_send(&id).unwrap();
        let queued = store.list_retryable_pending_mail_ops(10).unwrap();
        assert_eq!(
            serde_json::from_str::<serde_json::Value>(&queued[0].payload_json).unwrap()
                ["remote_succeeded"],
            true
        );
    }

    fn envelope(op: &str, payload: serde_json::Value) -> String {
        serde_json::json!({ "remote_id": "remote-123", "op": op, "payload": payload }).to_string()
    }

    fn committed_envelope(
        op: &str,
        payload: serde_json::Value,
        previous: serde_json::Value,
    ) -> String {
        serde_json::json!({
            "remote_id": "remote-123",
            "op": op,
            "payload": payload,
            "local_committed": true,
            "previous_state": previous,
        })
        .to_string()
    }

    fn seeded_store() -> (Store, Account, Folder, Message) {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let folder = test_folder(&account.id);
        store.insert_folder(&folder).unwrap();
        let message = test_message(&account.id);
        store
            .insert_message(&message, std::slice::from_ref(&folder.id))
            .unwrap();
        (store, account, folder, message)
    }

    #[test]
    fn earlier_failing_op_replays_before_later_op_on_same_message() {
        let (store, account, _folder, message) = seeded_store();
        let first = store
            .insert_pending_mail_op(&account.id, &message.id, "archive", "{}")
            .unwrap();
        // A fails twice; each failure bumps updated_at.
        store
            .mark_pending_mail_op_failed(&first, "offline")
            .unwrap();
        store
            .force_pending_mail_op_retry_now_for_test(&first)
            .unwrap();
        store
            .mark_pending_mail_op_failed(&first, "offline")
            .unwrap();
        let second = store
            .insert_pending_mail_op(&account.id, &message.id, "move_to_folder", "{}")
            .unwrap();

        // While A waits for its backoff, B must not overtake it.
        assert!(store
            .list_retryable_pending_mail_ops(10)
            .unwrap()
            .is_empty());

        store
            .force_pending_mail_op_retry_now_for_test(&first)
            .unwrap();
        let ids: Vec<_> = store
            .list_retryable_pending_mail_ops(10)
            .unwrap()
            .into_iter()
            .map(|op| op.id)
            .collect();
        assert_eq!(ids, vec![first.clone()]);

        store.mark_pending_mail_op_done(&first).unwrap();
        let ids: Vec<_> = store
            .list_retryable_pending_mail_ops(10)
            .unwrap()
            .into_iter()
            .map(|op| op.id)
            .collect();
        assert_eq!(ids, vec![second]);
    }

    #[test]
    fn exhausted_op_does_not_block_later_ops_forever() {
        let (store, account, _folder, message) = seeded_store();
        let first = store
            .insert_pending_mail_op(&account.id, &message.id, "archive", "{}")
            .unwrap();
        store
            .mark_pending_mail_op_stopped(&first, "gave up")
            .unwrap();
        let second = store
            .insert_pending_mail_op(&account.id, &message.id, "move_to_folder", "{}")
            .unwrap();
        let ids: Vec<_> = store
            .list_retryable_pending_mail_ops(10)
            .unwrap()
            .into_iter()
            .map(|op| op.id)
            .collect();
        assert_eq!(ids, vec![second]);
    }

    #[test]
    fn enqueue_collapses_flag_toggles_to_final_state() {
        let (store, account, _folder, message) = seeded_store();
        let first = store
            .enqueue_pending_mail_op(
                &account.id,
                &message.id,
                "update_flags",
                &committed_envelope(
                    "update_flags",
                    serde_json::json!({"is_read": true, "is_starred": null,
                        "add_labels": [], "remove_labels": ["UNREAD"]}),
                    serde_json::json!({"is_read": false, "is_starred": false}),
                ),
            )
            .unwrap();
        let PendingMailOpEnqueue::Inserted(first_id) = first else {
            panic!("expected insert, got {first:?}");
        };
        store
            .mark_pending_mail_op_failed(&first_id, "offline")
            .unwrap();

        let second = store
            .enqueue_pending_mail_op(
                &account.id,
                &message.id,
                "update_flags",
                &committed_envelope(
                    "update_flags",
                    serde_json::json!({"is_read": false, "is_starred": true}),
                    serde_json::json!({"is_read": true, "is_starred": false}),
                ),
            )
            .unwrap();
        assert_eq!(second, PendingMailOpEnqueue::Merged(first_id.clone()));

        let ops = store.list_pending_mail_ops(&account.id).unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].status, PendingMailOpStatus::Pending);
        assert_eq!(ops[0].attempts, 0);
        let payload: serde_json::Value = serde_json::from_str(&ops[0].payload_json).unwrap();
        assert_eq!(payload["payload"]["is_read"], false);
        assert_eq!(payload["payload"]["is_starred"], true);
        assert!(payload["payload"].get("remove_labels").is_none());
        // The oldest recorded pre-change state is what a revert must restore.
        assert_eq!(payload["previous_state"]["is_read"], false);
    }

    #[test]
    fn enqueue_cancels_archive_then_unarchive_when_neither_started() {
        let (store, account, _folder, message) = seeded_store();
        let to_archive = serde_json::json!({
            "source_folder_id": "inbox", "source_folder_remote_id": "INBOX",
            "target_folder_id": "archive", "target_folder_remote_id": "Archive",
        });
        let back_to_inbox = serde_json::json!({
            "source_folder_id": "archive", "source_folder_remote_id": "Archive",
            "target_folder_id": "inbox", "target_folder_remote_id": "INBOX",
        });
        let PendingMailOpEnqueue::Inserted(archive) = store
            .enqueue_pending_mail_op(
                &account.id,
                &message.id,
                "archive",
                &envelope("archive", to_archive.clone()),
            )
            .unwrap()
        else {
            panic!("expected insert");
        };
        let result = store
            .enqueue_pending_mail_op(
                &account.id,
                &message.id,
                "unarchive",
                &envelope("unarchive", back_to_inbox.clone()),
            )
            .unwrap();
        assert_eq!(
            result,
            PendingMailOpEnqueue::Cancelled {
                cancelled_op_id: archive.clone()
            }
        );
        assert!(store
            .list_active_pending_mail_ops(Some(&account.id), 10)
            .unwrap()
            .is_empty());

        // A running archive is not cancelled; the unarchive queues behind it.
        let PendingMailOpEnqueue::Inserted(running) = store
            .enqueue_pending_mail_op(
                &account.id,
                &message.id,
                "archive",
                &envelope("archive", serde_json::json!({})),
            )
            .unwrap()
        else {
            panic!("expected insert");
        };
        store.mark_pending_mail_op_in_progress(&running).unwrap();
        assert!(matches!(
            store
                .enqueue_pending_mail_op(
                    &account.id,
                    &message.id,
                    "unarchive",
                    &envelope("unarchive", serde_json::json!({}))
                )
                .unwrap(),
            PendingMailOpEnqueue::Inserted(_)
        ));
    }

    #[test]
    fn archive_then_unarchive_to_a_different_folder_is_not_cancelled() {
        let (store, account, _folder, message) = seeded_store();
        // Archived from Sent; unarchive always goes to the Inbox.
        store
            .enqueue_pending_mail_op(
                &account.id,
                &message.id,
                "archive",
                &envelope(
                    "archive",
                    serde_json::json!({
                        "source_folder_id": "sent", "target_folder_id": "archive",
                    }),
                ),
            )
            .unwrap();
        let result = store
            .enqueue_pending_mail_op(
                &account.id,
                &message.id,
                "unarchive",
                &envelope(
                    "unarchive",
                    serde_json::json!({
                        "source_folder_id": "archive", "target_folder_id": "inbox",
                    }),
                ),
            )
            .unwrap();
        assert!(matches!(result, PendingMailOpEnqueue::Inserted(_)));
        assert_eq!(
            store
                .list_active_pending_mail_ops(Some(&account.id), 10)
                .unwrap()
                .len(),
            2
        );

        // Without recorded endpoints the ops are kept too.
        let (store, account, _folder, message) = seeded_store();
        store
            .enqueue_pending_mail_op(
                &account.id,
                &message.id,
                "archive",
                &envelope("archive", serde_json::json!({})),
            )
            .unwrap();
        assert!(matches!(
            store
                .enqueue_pending_mail_op(
                    &account.id,
                    &message.id,
                    "unarchive",
                    &envelope("unarchive", serde_json::json!({})),
                )
                .unwrap(),
            PendingMailOpEnqueue::Inserted(_)
        ));
    }

    #[test]
    fn gmail_label_edits_cancel_only_when_they_mirror() {
        let (store, account, _folder, message) = seeded_store();
        store
            .enqueue_pending_mail_op(
                &account.id,
                &message.id,
                "archive",
                &envelope("archive", serde_json::json!({ "remove_labels": ["INBOX"] })),
            )
            .unwrap();
        assert!(matches!(
            store
                .enqueue_pending_mail_op(
                    &account.id,
                    &message.id,
                    "unarchive",
                    &envelope("unarchive", serde_json::json!({ "add_labels": ["INBOX"] })),
                )
                .unwrap(),
            PendingMailOpEnqueue::Cancelled { .. }
        ));

        let (store, account, _folder, message) = seeded_store();
        store
            .enqueue_pending_mail_op(
                &account.id,
                &message.id,
                "archive",
                &envelope(
                    "archive",
                    serde_json::json!({ "remove_labels": ["INBOX", "IMPORTANT"] }),
                ),
            )
            .unwrap();
        assert!(matches!(
            store
                .enqueue_pending_mail_op(
                    &account.id,
                    &message.id,
                    "unarchive",
                    &envelope("unarchive", serde_json::json!({ "add_labels": ["INBOX"] })),
                )
                .unwrap(),
            PendingMailOpEnqueue::Inserted(_)
        ));
    }

    #[test]
    fn dismiss_reverts_recorded_flag_and_folder_changes() {
        let (store, account, inbox, message) = seeded_store();
        let mut archive = test_folder(&account.id);
        archive.id = new_id();
        archive.remote_id = "Archive".into();
        archive.role = Some(FolderRole::Archive);
        store.insert_folder(&archive).unwrap();

        // Optimistic local commits: archived and marked read.
        let PendingMailOpEnqueue::Inserted(archive_op) = store
            .enqueue_pending_mail_op(
                &account.id,
                &message.id,
                "archive",
                &committed_envelope(
                    "archive",
                    serde_json::json!({"target_folder_id": archive.id}),
                    serde_json::json!({"folder_ids": [inbox.id], "is_deleted": false,
                        "is_read": false, "is_starred": false}),
                ),
            )
            .unwrap()
        else {
            panic!("expected insert");
        };
        store
            .mark_pending_mail_op_failed(&archive_op, "offline")
            .unwrap();
        store
            .move_message_to_folder(&message.id, &archive.id)
            .unwrap();
        let PendingMailOpEnqueue::Inserted(flag_op) = store
            .enqueue_pending_mail_op(
                &account.id,
                &message.id,
                "update_flags",
                &committed_envelope(
                    "update_flags",
                    serde_json::json!({"is_read": true}),
                    serde_json::json!({"folder_ids": [archive.id], "is_read": false,
                        "is_starred": false, "is_deleted": false}),
                ),
            )
            .unwrap()
        else {
            panic!("expected insert");
        };
        store
            .mark_pending_mail_op_failed(&flag_op, "offline")
            .unwrap();
        store
            .update_message_flags(&message.id, Some(true), None)
            .unwrap();

        let abandoned = store
            .dismiss_failed_pending_mail_ops_with_compensation(Some(&account.id))
            .unwrap();
        assert_eq!(abandoned.len(), 2);
        assert!(abandoned
            .iter()
            .all(|op| op.compensation == PendingMailOpCompensation::Reverted));
        let restored = store.get_message(&message.id).unwrap().unwrap();
        assert!(!restored.is_read);
        assert_eq!(
            store.get_message_folder_ids(&message.id).unwrap(),
            vec![inbox.id]
        );
        // Compensation happens once.
        assert_eq!(
            store
                .compensate_abandoned_pending_mail_op(&archive_op)
                .unwrap()
                .unwrap()
                .compensation,
            PendingMailOpCompensation::NotNeeded
        );
    }

    #[test]
    fn abandoned_op_without_previous_state_requests_resync() {
        let (store, account, _folder, message) = seeded_store();
        let legacy = store
            .insert_pending_mail_op(&account.id, &message.id, "archive", "{}")
            .unwrap();
        let uncommitted = store
            .insert_pending_mail_op(
                &account.id,
                &message.id,
                "update_flags",
                &serde_json::json!({"local_committed": false, "payload": {"is_read": true}})
                    .to_string(),
            )
            .unwrap();
        assert_eq!(
            store
                .compensate_abandoned_pending_mail_op(&legacy)
                .unwrap()
                .unwrap()
                .compensation,
            PendingMailOpCompensation::NeedsResync
        );
        assert_eq!(
            store
                .compensate_abandoned_pending_mail_op(&uncommitted)
                .unwrap()
                .unwrap()
                .compensation,
            PendingMailOpCompensation::NotNeeded
        );
    }

    #[test]
    fn deleting_account_clears_queued_ops_and_search_work() {
        let (store, account, _folder, message) = seeded_store();
        store
            .insert_pending_mail_op(&account.id, &message.id, "archive", "{}")
            .unwrap();
        store
            .add_search_pending(std::slice::from_ref(&message.id), "index")
            .unwrap();
        store.delete_account(&account.id).unwrap();
        assert!(store.list_pending_mail_ops(&account.id).unwrap().is_empty());
        assert!(store.list_search_pending().unwrap().is_empty());
    }

    #[test]
    fn manual_send_retry_rejects_non_send_or_missing_message() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let message = test_message(&account.id);
        store.insert_message(&message, &[]).unwrap();
        let id = store
            .insert_pending_mail_op(&account.id, &message.id, "delete", "{}")
            .unwrap();
        store.mark_pending_mail_op_stopped(&id, "Failed").unwrap();
        assert!(store.retry_failed_send(&id).is_err());
        assert!(store.retry_failed_send("missing").is_err());
        let send = store
            .insert_pending_mail_op(&account.id, &message.id, "send", "{}")
            .unwrap();
        store.mark_pending_mail_op_stopped(&send, "Failed").unwrap();
        store
            .bulk_soft_delete(std::slice::from_ref(&message.id))
            .unwrap();
        assert!(store.retry_failed_send(&send).is_err());
    }

    #[test]
    fn restore_cannot_attach_new_credentials_to_legacy_cached_mail() {
        use crate::cloud_sync::{RestoredAuthData, RestoredPrivateData};
        let store = Store::open_in_memory().unwrap();
        let mut account = test_account();
        account.provider = ProviderType::Outlook;
        store.insert_account(&account).unwrap();
        let message = test_message(&account.id);
        store.insert_message(&message, &[]).unwrap();
        let backup = store.export_settings().unwrap();
        store
            .import_settings_with_private_data(
                &backup,
                RestoredPrivateData {
                    auth_data: vec![RestoredAuthData {
                        account_id: account.id.clone(),
                        provider: "outlook".into(),
                        encrypted: b"other-mailbox-token".to_vec(),
                    }],
                    ..Default::default()
                },
            )
            .unwrap();
        assert!(store.get_auth_data(&account.id).unwrap().is_none());
        assert_eq!(
            store.get_message(&message.id).unwrap().unwrap().subject,
            message.subject
        );
    }

    #[test]
    fn stopped_sender_operation_stays_visible_without_becoming_retryable() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let message = test_message(&account.id);
        store.insert_message(&message, &[]).unwrap();
        let op = store
            .insert_pending_mail_op(&account.id, &message.id, "send", r#"{"op":"send"}"#)
            .unwrap();
        store
            .mark_pending_mail_op_stopped(&op, "Sender identity needs attention; no mail sent")
            .unwrap();
        let active = store
            .list_active_pending_mail_ops(Some(&account.id), 10)
            .unwrap();
        assert_eq!(active.len(), 1);
        assert_eq!(active[0].status, PendingMailOpStatus::Failed);
        assert_eq!(active[0].attempts, MAX_PENDING_MAIL_OP_ATTEMPTS);
        assert!(store
            .list_retryable_pending_mail_ops(10)
            .unwrap()
            .is_empty());
    }
    use pebble_core::*;

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

    #[test]
    fn pending_mail_ops_insert_list_mark_failed_and_done() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let folder = test_folder(&account.id);
        store.insert_folder(&folder).unwrap();
        let message = test_message(&account.id);
        store.insert_message(&message, &[folder.id]).unwrap();

        let op_id = store
            .insert_pending_mail_op(&account.id, &message.id, "flag", r#"{"is_read":true}"#)
            .unwrap();

        let ops = store.list_pending_mail_ops(&account.id).unwrap();
        assert_eq!(ops.len(), 1);
        assert_eq!(ops[0].id, op_id);
        assert_eq!(ops[0].status, PendingMailOpStatus::Pending);
        assert_eq!(ops[0].attempts, 0);

        store
            .mark_pending_mail_op_failed(&op_id, "remote unavailable")
            .unwrap();
        let failed = store.list_pending_mail_ops(&account.id).unwrap();
        assert_eq!(failed[0].status, PendingMailOpStatus::Failed);
        assert_eq!(failed[0].attempts, 1);
        assert_eq!(failed[0].last_error.as_deref(), Some("remote unavailable"));

        store.mark_pending_mail_op_done(&op_id).unwrap();
        let done = store.list_pending_mail_ops(&account.id).unwrap();
        assert_eq!(done[0].status, PendingMailOpStatus::Done);
    }

    #[test]
    fn pending_mail_ops_retryable_list_and_summary_ignore_done_ops() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let folder = test_folder(&account.id);
        store.insert_folder(&folder).unwrap();
        let message = test_message(&account.id);
        store.insert_message(&message, &[folder.id]).unwrap();

        let pending_id = store
            .insert_pending_mail_op(&account.id, &message.id, "archive", "{}")
            .unwrap();
        let failed_id = store
            .insert_pending_mail_op(&account.id, &message.id, "update_flags", "{}")
            .unwrap();
        let done_id = store
            .insert_pending_mail_op(&account.id, &message.id, "delete", "{}")
            .unwrap();

        store
            .mark_pending_mail_op_failed(&failed_id, "network unavailable")
            .unwrap();
        store.mark_pending_mail_op_done(&done_id).unwrap();

        let retryable = store.list_retryable_pending_mail_ops(10).unwrap();
        let retryable_ids: Vec<_> = retryable.iter().map(|op| op.id.as_str()).collect();
        assert_eq!(retryable_ids, vec![pending_id.as_str()]);

        store.mark_pending_mail_op_in_progress(&pending_id).unwrap();
        let summary = store.pending_mail_ops_summary(Some(&account.id)).unwrap();
        assert_eq!(summary.pending_count, 0);
        assert_eq!(summary.in_progress_count, 1);
        assert_eq!(summary.failed_count, 1);
        assert_eq!(summary.total_active_count, 2);
        assert_eq!(summary.last_error.as_deref(), Some("network unavailable"));

        let retryable_after_claim = store.list_retryable_pending_mail_ops(10).unwrap();
        assert!(retryable_after_claim.is_empty());
        let failed = store.list_pending_mail_ops(&account.id).unwrap();
        let failed = failed.iter().find(|op| op.id == failed_id).unwrap();
        assert!(failed.next_retry_at.unwrap() > now_timestamp());
    }

    #[test]
    fn failed_pending_mail_ops_stop_retrying_after_max_attempts() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let folder = test_folder(&account.id);
        store.insert_folder(&folder).unwrap();
        let message = test_message(&account.id);
        store.insert_message(&message, &[folder.id]).unwrap();

        let op_id = store
            .insert_pending_mail_op(&account.id, &message.id, "archive", "{}")
            .unwrap();
        for attempt in 0..MAX_PENDING_MAIL_OP_ATTEMPTS {
            store
                .mark_pending_mail_op_failed(&op_id, &format!("failure {attempt}"))
                .unwrap();
            store
                .force_pending_mail_op_retry_now_for_test(&op_id)
                .unwrap();
        }

        assert!(store
            .list_retryable_pending_mail_ops(10)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn active_pending_mail_ops_list_filters_done_ops_accounts_and_limit() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let folder = test_folder(&account.id);
        store.insert_folder(&folder).unwrap();
        let message = test_message(&account.id);
        store.insert_message(&message, &[folder.id]).unwrap();

        let other_account = test_account();
        store.insert_account(&other_account).unwrap();
        let other_folder = test_folder(&other_account.id);
        store.insert_folder(&other_folder).unwrap();
        let other_message = test_message(&other_account.id);
        store
            .insert_message(&other_message, &[other_folder.id])
            .unwrap();

        let pending_id = store
            .insert_pending_mail_op(&account.id, &message.id, "archive", "{}")
            .unwrap();
        let failed_id = store
            .insert_pending_mail_op(&account.id, &message.id, "update_flags", "{}")
            .unwrap();
        let done_id = store
            .insert_pending_mail_op(&account.id, &message.id, "delete", "{}")
            .unwrap();
        let other_id = store
            .insert_pending_mail_op(&other_account.id, &other_message.id, "archive", "{}")
            .unwrap();

        store
            .mark_pending_mail_op_failed(&failed_id, "network unavailable")
            .unwrap();
        store.mark_pending_mail_op_done(&done_id).unwrap();

        let account_ops = store
            .list_active_pending_mail_ops(Some(&account.id), 50)
            .unwrap();
        let account_op_ids: Vec<_> = account_ops.iter().map(|op| op.id.as_str()).collect();
        assert_eq!(account_ops.len(), 2);
        assert!(account_op_ids.contains(&pending_id.as_str()));
        assert!(account_op_ids.contains(&failed_id.as_str()));
        assert!(!account_op_ids.contains(&done_id.as_str()));
        assert!(!account_op_ids.contains(&other_id.as_str()));
        assert!(account_ops
            .iter()
            .all(|op| op.status != PendingMailOpStatus::Done));

        let limited_ops = store.list_active_pending_mail_ops(None, 1).unwrap();
        assert_eq!(limited_ops.len(), 1);
        assert_ne!(limited_ops[0].status, PendingMailOpStatus::Done);
    }

    #[test]
    fn pending_mail_ops_can_reset_stuck_in_progress_ops() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let folder = test_folder(&account.id);
        store.insert_folder(&folder).unwrap();
        let message = test_message(&account.id);
        store.insert_message(&message, &[folder.id]).unwrap();

        let op_id = store
            .insert_pending_mail_op(&account.id, &message.id, "archive", "{}")
            .unwrap();
        store.mark_pending_mail_op_in_progress(&op_id).unwrap();

        store.reset_in_progress_pending_mail_ops().unwrap();

        let retryable = store.list_retryable_pending_mail_ops(10).unwrap();
        assert_eq!(retryable.len(), 1);
        assert_eq!(retryable[0].id, op_id);
        assert_eq!(retryable[0].status, PendingMailOpStatus::Pending);
    }

    #[test]
    fn interrupted_send_without_remote_receipt_is_not_retried_automatically() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let folder = test_folder(&account.id);
        store.insert_folder(&folder).unwrap();
        let message = test_message(&account.id);
        store.insert_message(&message, &[folder.id]).unwrap();

        let archive_id = store
            .insert_pending_mail_op(&account.id, &message.id, "archive", "{}")
            .unwrap();
        let unknown_send_id = store
            .insert_pending_mail_op(
                &account.id,
                &message.id,
                "send",
                r#"{"op":"send","payload":{}}"#,
            )
            .unwrap();
        let confirmed_send_id = store
            .insert_pending_mail_op(
                &account.id,
                &message.id,
                "send",
                r#"{"op":"send","payload":{}}"#,
            )
            .unwrap();
        store
            .mark_pending_mail_op_remote_succeeded(&confirmed_send_id)
            .unwrap();
        for id in [&archive_id, &unknown_send_id, &confirmed_send_id] {
            store.mark_pending_mail_op_in_progress(id).unwrap();
        }

        store.reset_in_progress_pending_mail_ops().unwrap();

        let ops = store.list_pending_mail_ops(&account.id).unwrap();
        let unknown_send = ops.iter().find(|op| op.id == unknown_send_id).unwrap();
        assert_eq!(unknown_send.status, PendingMailOpStatus::Failed);
        assert_eq!(unknown_send.attempts, MAX_PENDING_MAIL_OP_ATTEMPTS);
        assert!(unknown_send
            .last_error
            .as_deref()
            .is_some_and(|error| error.contains("outcome is unknown")));
        assert_eq!(unknown_send.next_retry_at, None);

        let retryable_ids: Vec<_> = store
            .list_retryable_pending_mail_ops(10)
            .unwrap()
            .into_iter()
            .map(|op| op.id)
            .collect();
        // Ops on one message replay in creation order: the archive runs first,
        // the stopped send no longer holds the queue, and the confirmed send
        // finalizes once the archive is done.
        assert_eq!(retryable_ids, vec![archive_id.clone()]);
        store.mark_pending_mail_op_done(&archive_id).unwrap();
        let retryable_ids: Vec<_> = store
            .list_retryable_pending_mail_ops(10)
            .unwrap()
            .into_iter()
            .map(|op| op.id)
            .collect();
        assert_eq!(retryable_ids, vec![confirmed_send_id]);
        assert!(!retryable_ids.contains(&unknown_send_id));
    }

    #[test]
    fn pending_send_records_remote_success_before_local_finalize() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let folder = test_folder(&account.id);
        store.insert_folder(&folder).unwrap();
        let message = test_message(&account.id);
        store.insert_message(&message, &[folder.id]).unwrap();
        let op_id = store
            .insert_pending_mail_op(
                &account.id,
                &message.id,
                "send",
                r#"{"op":"send","payload":{}}"#,
            )
            .unwrap();

        store.mark_pending_mail_op_remote_succeeded(&op_id).unwrap();

        let op = store.list_pending_mail_ops(&account.id).unwrap().remove(0);
        let payload: serde_json::Value = serde_json::from_str(&op.payload_json).unwrap();
        assert_eq!(payload["remote_succeeded"], true);
    }

    #[test]
    fn unknown_send_outcome_is_visible_but_never_retryable() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let folder = test_folder(&account.id);
        store.insert_folder(&folder).unwrap();
        let message = test_message(&account.id);
        store.insert_message(&message, &[folder.id]).unwrap();
        let op_id = store
            .insert_pending_mail_op(&account.id, &message.id, "send", "{}")
            .unwrap();
        store.mark_pending_mail_op_in_progress(&op_id).unwrap();

        store
            .mark_pending_mail_op_outcome_unknown(
                &op_id,
                "Send outcome is unknown; check Sent before sending again.",
            )
            .unwrap();

        assert!(store
            .list_retryable_pending_mail_ops(10)
            .unwrap()
            .is_empty());
        let op = store.list_pending_mail_ops(&account.id).unwrap().remove(0);
        assert_eq!(op.status, PendingMailOpStatus::Failed);
        assert_eq!(op.attempts, MAX_PENDING_MAIL_OP_ATTEMPTS);
        assert!(op.last_error.unwrap().contains("check Sent"));
        let summary = store.pending_mail_ops_summary(Some(&account.id)).unwrap();
        assert_eq!(summary.failed_count, 1);

        assert_eq!(
            store
                .dismiss_failed_pending_mail_ops(Some(&account.id))
                .unwrap(),
            1
        );
    }
}

impl Store {
    /// Return the subset of `message_ids` (local message ids) belonging to
    /// `account_id` that still have an active queued mail operation: one that
    /// is `pending`, `in_progress`, or `failed` but still retryable.
    ///
    /// Sync paths use this to avoid overwriting optimistic local changes
    /// (flags, folder membership) with stale server state before the queued
    /// operation has been written back to the provider.
    pub fn message_ids_with_active_pending_ops(
        &self,
        account_id: &str,
        message_ids: &[String],
    ) -> Result<std::collections::HashSet<String>> {
        let mut active = std::collections::HashSet::new();
        if message_ids.is_empty() {
            return Ok(active);
        }
        self.with_read(|conn| {
            for chunk in message_ids.chunks(500) {
                let placeholders = (0..chunk.len())
                    .map(|index| format!("?{}", index + 3))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT DISTINCT message_id FROM pending_mail_ops
                     WHERE account_id = ?1
                       AND (status IN ('pending', 'in_progress')
                            OR (status = 'failed' AND attempts < ?2))
                       AND message_id IN ({placeholders})"
                );
                let mut stmt = conn.prepare(&sql)?;
                let mut values: Vec<&dyn rusqlite::ToSql> = Vec::with_capacity(chunk.len() + 2);
                values.push(&account_id);
                values.push(&MAX_PENDING_MAIL_OP_ATTEMPTS);
                for id in chunk {
                    values.push(id);
                }
                let rows = stmt.query_map(values.as_slice(), |row| row.get::<_, String>(0))?;
                for row in rows {
                    active.insert(row?);
                }
            }
            Ok(())
        })?;
        Ok(active)
    }
}

#[cfg(test)]
mod active_pending_ops_tests {
    use super::*;
    use pebble_core::{new_id, now_timestamp, Account, Message, ProviderType};

    fn test_account() -> Account {
        let now = now_timestamp();
        Account {
            account_label: None,
            provider_display_name: None,
            id: new_id(),
            email: "active-ops@example.com".to_string(),
            display_name: "Active Ops".to_string(),
            color: None,
            provider: ProviderType::Imap,
            created_at: now,
            updated_at: now,
        }
    }

    fn test_message(account_id: &str) -> Message {
        let now = now_timestamp();
        let id = new_id();
        Message {
            id: id.clone(),
            account_id: account_id.to_string(),
            remote_id: format!("remote-{id}"),
            message_id_header: None,
            in_reply_to: None,
            references_header: None,
            thread_id: None,
            subject: "Test".to_string(),
            snippet: String::new(),
            from_address: "from@example.com".to_string(),
            from_name: "From".to_string(),
            to_list: vec![],
            cc_list: vec![],
            bcc_list: vec![],
            body_text: String::new(),
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
    fn active_pending_ops_cover_pending_in_progress_and_retryable_failures() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let mut ids = Vec::new();
        for _ in 0..5 {
            let message = test_message(&account.id);
            store.insert_message(&message, &[]).unwrap();
            ids.push(message.id);
        }
        store
            .insert_pending_mail_op(&account.id, &ids[0], "update_flags", "{}")
            .unwrap();
        let in_progress = store
            .insert_pending_mail_op(&account.id, &ids[1], "move", "{}")
            .unwrap();
        store
            .mark_pending_mail_op_in_progress(&in_progress)
            .unwrap();
        let retryable = store
            .insert_pending_mail_op(&account.id, &ids[2], "update_flags", "{}")
            .unwrap();
        store
            .mark_pending_mail_op_failed(&retryable, "offline")
            .unwrap();
        let exhausted = store
            .insert_pending_mail_op(&account.id, &ids[3], "update_flags", "{}")
            .unwrap();
        store
            .mark_pending_mail_op_stopped(&exhausted, "gave up")
            .unwrap();

        let active = store
            .message_ids_with_active_pending_ops(&account.id, &ids)
            .unwrap();
        assert!(active.contains(&ids[0]));
        assert!(active.contains(&ids[1]));
        assert!(active.contains(&ids[2]));
        assert!(!active.contains(&ids[3]));
        assert!(!active.contains(&ids[4]));

        assert!(store
            .message_ids_with_active_pending_ops("other-account", &ids)
            .unwrap()
            .is_empty());
        assert!(store
            .message_ids_with_active_pending_ops(&account.id, &[])
            .unwrap()
            .is_empty());
    }
}
