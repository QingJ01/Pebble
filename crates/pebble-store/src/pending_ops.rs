use pebble_core::{PebbleError, Result};
use rusqlite::{params, OptionalExtension};

use crate::Store;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingMailOpsSummary {
    pub pending_count: i64,
    pub in_progress_count: i64,
    pub failed_count: i64,
    pub total_active_count: i64,
    pub last_error: Option<String>,
    pub updated_at: Option<i64>,
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

    pub fn list_pending_mail_ops(&self, account_id: &str) -> Result<Vec<PendingMailOp>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, account_id, message_id, op_type, payload_json, status,
                        attempts, last_error, created_at, updated_at, next_retry_at
                 FROM pending_mail_ops
                 WHERE account_id = ?1
                 ORDER BY updated_at ASC",
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

    pub fn list_retryable_pending_mail_ops(&self, limit: i64) -> Result<Vec<PendingMailOp>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, account_id, message_id, op_type, payload_json, status,
                        attempts, last_error, created_at, updated_at, next_retry_at
                 FROM pending_mail_ops
                 WHERE status = 'pending'
                    OR (
                        status = 'failed'
                        AND attempts < ?2
                        AND (next_retry_at IS NULL OR next_retry_at <= ?3)
                    )
                 ORDER BY updated_at ASC
                 LIMIT ?1",
            )?;
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
            conn.execute(
                "UPDATE pending_mail_ops
                 SET status = ?1,
                     updated_at = ?2,
                     next_retry_at = NULL
                 WHERE status = ?3",
                params![
                    PendingMailOpStatus::Pending.as_str(),
                    pebble_core::now_timestamp(),
                    PendingMailOpStatus::InProgress.as_str(),
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
    use pebble_core::*;

    fn test_account() -> Account {
        let now = now_timestamp();
        Account {
            id: new_id(),
            email: "test@example.com".to_string(),
            display_name: "Test".to_string(),
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
}
