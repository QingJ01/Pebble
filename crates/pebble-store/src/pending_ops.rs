use pebble_core::{PebbleError, Result};
use rusqlite::params;

use crate::Store;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PendingMailOpStatus {
    Pending,
    InProgress,
    Failed,
    Done,
}

impl PendingMailOpStatus {
    fn as_str(&self) -> &'static str {
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
                        attempts, last_error, created_at, updated_at
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
                });
            }
            Ok(ops)
        })
    }

    pub fn mark_pending_mail_op_failed(&self, id: &str, error: &str) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "UPDATE pending_mail_ops
                 SET status = ?1,
                     attempts = attempts + 1,
                     last_error = ?2,
                     updated_at = ?3
                 WHERE id = ?4",
                params![
                    PendingMailOpStatus::Failed.as_str(),
                    error,
                    pebble_core::now_timestamp(),
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
                     updated_at = ?2
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
}
