use pebble_core::{Attachment, Message, MessageSummary, PebbleError, Result};
use rusqlite::{params, OptionalExtension, Row};
use std::collections::{HashMap, HashSet};

use crate::pending_ops::PendingMailOpStatus;
use crate::Store;

pub type FolderRemoteMessageState = (String, String, bool, bool, i64);

#[derive(Debug)]
pub struct ImapFolderSnapshotMessage {
    pub message: Message,
    pub attachments: Vec<Attachment>,
}

#[derive(Debug)]
pub struct ImapFolderReconcileResult {
    pub indexed_message_ids: Vec<String>,
    pub removed_messages: Vec<Message>,
    pub unreferenced_attachment_paths: Vec<String>,
}

/// Maps a row to a Message. Column order must match the SELECT lists used below.
///
/// Expected column indices:
/// 0=id, 1=account_id, 2=remote_id, 3=message_id_header, 4=in_reply_to,
/// 5=references_header, 6=thread_id, 7=subject, 8=snippet, 9=from_address,
/// 10=from_name, 11=to_list, 12=cc_list, 13=bcc_list,
/// 14=body_text, 15=body_html_raw,
/// 16=has_attachments, 17=is_read, 18=is_starred, 19=is_draft,
/// 20=date, 21=remote_version, 22=is_deleted, 23=deleted_at, 24=created_at, 25=updated_at
fn row_to_message(row: &Row) -> rusqlite::Result<Message> {
    let to_json: String = row.get(11)?;
    let cc_json: String = row.get(12)?;
    let bcc_json: String = row.get(13)?;
    let has_attachments: i32 = row.get(16)?;
    let is_read: i32 = row.get(17)?;
    let is_starred: i32 = row.get(18)?;
    let is_draft: i32 = row.get(19)?;
    let is_deleted: i32 = row.get(22)?;

    Ok(Message {
        id: row.get(0)?,
        account_id: row.get(1)?,
        remote_id: row.get(2)?,
        message_id_header: row.get(3)?,
        in_reply_to: row.get(4)?,
        references_header: row.get(5)?,
        thread_id: row.get(6)?,
        subject: row.get(7)?,
        snippet: row.get(8)?,
        from_address: row.get(9)?,
        from_name: row.get(10)?,
        to_list: serde_json::from_str(&to_json).unwrap_or_default(),
        cc_list: serde_json::from_str(&cc_json).unwrap_or_default(),
        bcc_list: serde_json::from_str(&bcc_json).unwrap_or_default(),
        body_text: row.get(14)?,
        body_html_raw: row.get(15)?,
        has_attachments: has_attachments != 0,
        is_read: is_read != 0,
        is_starred: is_starred != 0,
        is_draft: is_draft != 0,
        date: row.get(20)?,
        remote_version: row.get(21)?,
        is_deleted: is_deleted != 0,
        deleted_at: row.get(23)?,
        created_at: row.get(24)?,
        updated_at: row.get(25)?,
    })
}

const MSG_SELECT: &str = "id, account_id, remote_id, message_id_header, in_reply_to, \
     references_header, thread_id, subject, snippet, from_address, \
     from_name, to_list, cc_list, bcc_list, \
     body_text, body_html_raw, \
     has_attachments, is_read, is_starred, is_draft, \
     date, remote_version, is_deleted, deleted_at, created_at, updated_at";

/// Column list for list queries (excludes body_text and body_html_raw).
const MSG_SUMMARY_SELECT: &str = "id, account_id, remote_id, message_id_header, in_reply_to, \
     references_header, thread_id, subject, snippet, from_address, \
     from_name, to_list, cc_list, bcc_list, \
     has_attachments, is_read, is_starred, is_draft, \
     date, remote_version, is_deleted, deleted_at, created_at, updated_at";

/// Maps a row to a MessageSummary (no body fields).
///
/// Expected column indices:
/// 0=id, 1=account_id, 2=remote_id, 3=message_id_header, 4=in_reply_to,
/// 5=references_header, 6=thread_id, 7=subject, 8=snippet, 9=from_address,
/// 10=from_name, 11=to_list, 12=cc_list, 13=bcc_list,
/// 14=has_attachments, 15=is_read, 16=is_starred, 17=is_draft,
/// 18=date, 19=remote_version, 20=is_deleted, 21=deleted_at, 22=created_at, 23=updated_at
fn row_to_message_summary(row: &Row) -> rusqlite::Result<MessageSummary> {
    let to_json: String = row.get(11)?;
    let cc_json: String = row.get(12)?;
    let bcc_json: String = row.get(13)?;
    let has_attachments: i32 = row.get(14)?;
    let is_read: i32 = row.get(15)?;
    let is_starred: i32 = row.get(16)?;
    let is_draft: i32 = row.get(17)?;
    let is_deleted: i32 = row.get(20)?;

    Ok(MessageSummary {
        id: row.get(0)?,
        account_id: row.get(1)?,
        remote_id: row.get(2)?,
        message_id_header: row.get(3)?,
        in_reply_to: row.get(4)?,
        references_header: row.get(5)?,
        thread_id: row.get(6)?,
        subject: row.get(7)?,
        snippet: row.get(8)?,
        from_address: row.get(9)?,
        from_name: row.get(10)?,
        to_list: serde_json::from_str(&to_json).unwrap_or_default(),
        cc_list: serde_json::from_str(&cc_json).unwrap_or_default(),
        bcc_list: serde_json::from_str(&bcc_json).unwrap_or_default(),
        has_attachments: has_attachments != 0,
        is_read: is_read != 0,
        is_starred: is_starred != 0,
        is_draft: is_draft != 0,
        date: row.get(18)?,
        remote_version: row.get(19)?,
        is_deleted: is_deleted != 0,
        deleted_at: row.get(21)?,
        created_at: row.get(22)?,
        updated_at: row.get(23)?,
    })
}

fn upsert_message_with_conn(conn: &rusqlite::Connection, msg: &Message) -> Result<()> {
    let to_json =
        serde_json::to_string(&msg.to_list).map_err(|e| PebbleError::Storage(e.to_string()))?;
    let cc_json =
        serde_json::to_string(&msg.cc_list).map_err(|e| PebbleError::Storage(e.to_string()))?;
    let bcc_json =
        serde_json::to_string(&msg.bcc_list).map_err(|e| PebbleError::Storage(e.to_string()))?;

    // An INSERT ... ON CONFLICT(id) statement still runs BEFORE INSERT
    // triggers before SQLite switches to its UPDATE branch. The provider-aware
    // remote-id uniqueness triggers therefore see the row being updated as a
    // duplicate of itself. Update known local identities first, and only use
    // INSERT for genuinely new messages.
    let updated = conn.execute(
        "UPDATE messages SET
           account_id = ?2,
           remote_id = ?3,
           message_id_header = ?4,
           in_reply_to = ?5,
           references_header = ?6,
           thread_id = ?7,
           subject = ?8,
           snippet = ?9,
           from_address = ?10,
           from_name = ?11,
           to_list = ?12,
           cc_list = ?13,
           bcc_list = ?14,
           body_text = ?15,
           body_html_raw = ?16,
           has_attachments = ?17,
           is_read = ?18,
           is_starred = ?19,
           is_draft = ?20,
           date = ?21,
           remote_version = ?22,
           is_deleted = ?23,
           deleted_at = ?24,
           created_at = ?25,
           updated_at = ?26
         WHERE id = ?1",
        params![
            msg.id,
            msg.account_id,
            msg.remote_id,
            msg.message_id_header,
            msg.in_reply_to,
            msg.references_header,
            msg.thread_id,
            msg.subject,
            msg.snippet,
            msg.from_address,
            msg.from_name,
            &to_json,
            &cc_json,
            &bcc_json,
            msg.body_text,
            msg.body_html_raw,
            msg.has_attachments as i32,
            msg.is_read as i32,
            msg.is_starred as i32,
            msg.is_draft as i32,
            msg.date,
            msg.remote_version,
            msg.is_deleted as i32,
            msg.deleted_at,
            msg.created_at,
            msg.updated_at,
        ],
    )?;
    if updated != 0 {
        return Ok(());
    }

    conn.execute(
        "INSERT INTO messages (id, account_id, remote_id, message_id_header, in_reply_to,
         references_header, thread_id, subject, snippet, from_address, from_name,
         to_list, cc_list, bcc_list, body_text, body_html_raw,
         has_attachments, is_read, is_starred, is_draft,
         date, remote_version, is_deleted, deleted_at, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,?21, ?22, ?23, ?24, ?25, ?26)",
        params![
            msg.id,
            msg.account_id,
            msg.remote_id,
            msg.message_id_header,
            msg.in_reply_to,
            msg.references_header,
            msg.thread_id,
            msg.subject,
            msg.snippet,
            msg.from_address,
            msg.from_name,
            to_json,
            cc_json,
            bcc_json,
            msg.body_text,
            msg.body_html_raw,
            msg.has_attachments as i32,
            msg.is_read as i32,
            msg.is_starred as i32,
            msg.is_draft as i32,
            msg.date,
            msg.remote_version,
            msg.is_deleted as i32,
            msg.deleted_at,
            msg.created_at,
            msg.updated_at,
        ],
    )?;
    Ok(())
}

impl Store {
    pub fn insert_message(&self, msg: &Message, folder_ids: &[String]) -> Result<()> {
        self.with_write(|conn| {
            let to_json = serde_json::to_string(&msg.to_list).map_err(|e| PebbleError::Storage(e.to_string()))?;
            let cc_json = serde_json::to_string(&msg.cc_list).map_err(|e| PebbleError::Storage(e.to_string()))?;
            let bcc_json = serde_json::to_string(&msg.bcc_list).map_err(|e| PebbleError::Storage(e.to_string()))?;

            let tx = conn.unchecked_transaction()?;

            tx.execute(
                "INSERT INTO messages (id, account_id, remote_id, message_id_header, in_reply_to,
                 references_header, thread_id, subject, snippet, from_address, from_name,
                 to_list, cc_list, bcc_list, body_text, body_html_raw,
                 has_attachments, is_read, is_starred, is_draft,
                 date, remote_version, is_deleted, deleted_at, created_at, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11,?12, ?13, ?14, ?15, ?16, ?17, ?18, ?19, ?20,?21, ?22, ?23, ?24, ?25, ?26)",
                params![
                    msg.id,
                    msg.account_id,
                    msg.remote_id,
                    msg.message_id_header,
                    msg.in_reply_to,
                    msg.references_header,
                    msg.thread_id,
                    msg.subject,
                    msg.snippet,
                    msg.from_address,
                    msg.from_name,
                    to_json,
                    cc_json,
                    bcc_json,
                    msg.body_text,
                    msg.body_html_raw,
                    msg.has_attachments as i32,
                    msg.is_read as i32,
                    msg.is_starred as i32,
                    msg.is_draft as i32,
                    msg.date,
                    msg.remote_version,
                    msg.is_deleted as i32,
                    msg.deleted_at,
                    msg.created_at,
                    msg.updated_at,
                ],
            )?;

            for folder_id in folder_ids {
                tx.execute(
                    "INSERT INTO message_folders (message_id, folder_id) VALUES (?1, ?2)",
                    params![msg.id, folder_id],
                )?;
            }

            tx.commit()?;
            Ok(())
        })
    }

    pub fn replace_message_with_attachments(
        &self,
        msg: &Message,
        folder_ids: &[String],
        attachments: &[Attachment],
    ) -> Result<()> {
        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()?;
            upsert_message_with_conn(&tx, msg)?;

            tx.execute(
                "DELETE FROM message_folders WHERE message_id = ?1",
                params![msg.id],
            )?;
            tx.execute(
                "DELETE FROM attachments WHERE message_id = ?1",
                params![msg.id],
            )?;

            for folder_id in folder_ids {
                tx.execute(
                    "INSERT INTO message_folders (message_id, folder_id) VALUES (?1, ?2)",
                    params![msg.id, folder_id],
                )?;
            }

            for attachment in attachments {
                tx.execute(
                    "INSERT INTO attachments (id, message_id, filename, mime_type, size, local_path, content_id, is_inline)
                    VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        attachment.id,
                        msg.id,
                        attachment.filename,
                        attachment.mime_type,
                        attachment.size,
                        attachment.local_path,
                        attachment.content_id,
                        attachment.is_inline as i32,
                    ],
                )?;
            }

            tx.commit()?;
            Ok(())
        })
    }

    /// Atomically persist an outgoing message, its attachment records, and
    /// an in-progress send operation before any remote send is attempted.
    pub fn prepare_outgoing_send(
        &self,
        msg: &Message,
        folder_ids: &[String],
        attachments: &[Attachment],
        payload_json: &str,
    ) -> Result<String> {
        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()?;
            upsert_message_with_conn(&tx, msg)?;

            for folder_id in folder_ids {
                tx.execute(
                    "INSERT INTO message_folders (message_id, folder_id) VALUES (?1, ?2)",
                    params![msg.id, folder_id],
                )?;
            }
            for attachment in attachments {
                tx.execute(
                    "INSERT INTO attachments (id, message_id, filename, mime_type, size, local_path, content_id, is_inline)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    params![
                        attachment.id,
                        msg.id,
                        attachment.filename,
                        attachment.mime_type,
                        attachment.size,
                        attachment.local_path,
                        attachment.content_id,
                        attachment.is_inline as i32,
                    ],
                )?;
            }

            let op_id = pebble_core::new_id();
            let now = pebble_core::now_timestamp();
            tx.execute(
                "INSERT INTO pending_mail_ops
                    (id, account_id, message_id, op_type, payload_json, status, attempts,
                     last_error, created_at, updated_at, next_retry_at)
                 VALUES (?1, ?2, ?3, 'send', ?4, ?5, 0, NULL, ?6, ?6, NULL)",
                params![
                    op_id,
                    msg.account_id,
                    msg.id,
                    payload_json,
                    PendingMailOpStatus::InProgress.as_str(),
                    now,
                ],
            )?;
            tx.commit()?;
            Ok(op_id)
        })
    }

    /// Finalize a remotely acknowledged send without exposing an intermediate
    /// state where the message is in Sent but its operation remains active.
    pub fn complete_outgoing_send(
        &self,
        message_id: &str,
        op_id: &str,
        sent_folder_id: Option<&str>,
    ) -> Result<()> {
        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()?;
            let updated = if let Some(sent_folder_id) = sent_folder_id {
                tx.execute(
                    "DELETE FROM message_folders WHERE message_id = ?1",
                    params![message_id],
                )?;
                tx.execute(
                    "INSERT INTO message_folders (message_id, folder_id) VALUES (?1, ?2)",
                    params![message_id, sent_folder_id],
                )?;
                tx.execute(
                    "UPDATE pending_mail_ops
                     SET status = ?1, updated_at = ?2, next_retry_at = NULL
                     WHERE id = ?3 AND message_id = ?4 AND op_type = 'send'",
                    params![
                        PendingMailOpStatus::Done.as_str(),
                        pebble_core::now_timestamp(),
                        op_id,
                        message_id,
                    ],
                )?
            } else {
                let deleted = tx.execute(
                    "DELETE FROM pending_mail_ops
                     WHERE id = ?1 AND message_id = ?2 AND op_type = 'send'",
                    params![op_id, message_id],
                )?;
                if deleted == 1 {
                    tx.execute(
                        "DELETE FROM message_folders WHERE message_id = ?1",
                        params![message_id],
                    )?;
                    tx.execute("DELETE FROM messages WHERE id = ?1", params![message_id])?;
                }
                deleted
            };
            if updated != 1 {
                return Err(PebbleError::Storage(format!(
                    "Prepared send operation not found: {op_id}"
                )));
            }
            tx.commit()?;
            Ok(())
        })
    }

    /// Remove a prepared send after a known pre-delivery failure. The pending
    /// operation is not foreign-keyed to messages, so both rows are deleted in
    /// the same transaction.
    pub fn discard_prepared_outgoing_send(&self, message_id: &str, op_id: &str) -> Result<()> {
        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "DELETE FROM pending_mail_ops WHERE id = ?1 AND message_id = ?2 AND op_type = 'send'",
                params![op_id, message_id],
            )?;
            tx.execute(
                "DELETE FROM message_folders WHERE message_id = ?1",
                params![message_id],
            )?;
            tx.execute("DELETE FROM messages WHERE id = ?1", params![message_id])?;
            tx.commit()?;
            Ok(())
        })
    }

    pub fn list_starred_messages(
        &self,
        account_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<MessageSummary>> {
        self.with_read(|conn| {
            let sql = format!(
                "SELECT m.{} FROM messages m
                 WHERE m.account_id = ?1 AND m.is_starred = 1 AND m.is_deleted = 0
                 ORDER BY m.date DESC
                 LIMIT ?2 OFFSET ?3",
                MSG_SUMMARY_SELECT.replace(", ", ", m.")
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows =
                stmt.query_map(params![account_id, limit, offset], row_to_message_summary)?;
            let mut messages = Vec::new();
            for row in rows {
                messages.push(row?);
            }
            Ok(messages)
        })
    }

    pub fn list_messages_by_folder(
        &self,
        folder_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<MessageSummary>> {
        self.with_read(|conn| {
            let sql = format!(
                "SELECT m.{} FROM messages m
                 JOIN message_folders mf ON m.id = mf.message_id
                 WHERE mf.folder_id = ?1 AND m.is_deleted = 0
                 ORDER BY m.date DESC
                 LIMIT ?2 OFFSET ?3",
                MSG_SUMMARY_SELECT.replace(", ", ", m.")
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![folder_id, limit, offset], row_to_message_summary)?;
            let mut messages = Vec::new();
            for row in rows {
                messages.push(row?);
            }
            Ok(messages)
        })
    }

    /// List full messages by folder (includes body fields). Used for search re-indexing.
    pub fn list_full_messages_by_folder(
        &self,
        folder_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Message>> {
        self.with_read(|conn| {
            let sql = format!(
                "SELECT m.{} FROM messages m
                 JOIN message_folders mf ON m.id = mf.message_id
                 WHERE mf.folder_id = ?1 AND m.is_deleted = 0
                 ORDER BY m.date DESC
                 LIMIT ?2 OFFSET ?3",
                MSG_SELECT.replace(", ", ", m.")
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![folder_id, limit, offset], row_to_message)?;
            let mut messages = Vec::new();
            for row in rows {
                messages.push(row?);
            }
            Ok(messages)
        })
    }

    /// List full messages for an entire account, paginated by `(date DESC, id)`.
    ///
    /// Unlike [`list_full_messages_by_folder`], each message is returned at most
    /// once, even if it lives in multiple folders (e.g. Gmail labels). Intended
    /// for operations that must visit each stored message exactly once, such as
    /// search reindexing.
    pub fn list_full_messages_by_account(
        &self,
        account_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Message>> {
        self.with_read(|conn| {
            let sql = format!(
                "SELECT {} FROM messages
                 WHERE account_id = ?1 AND is_deleted = 0
                 ORDER BY date DESC, id ASC
                 LIMIT ?2 OFFSET ?3",
                MSG_SELECT,
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![account_id, limit, offset], row_to_message)?;
            let mut messages = Vec::new();
            for row in rows {
                messages.push(row?);
            }
            Ok(messages)
        })
    }

    /// Fetch folder IDs for a batch of message IDs in a single query.
    ///
    /// Returns a map of `message_id -> Vec<folder_id>`. Messages with no
    /// folder membership are absent from the map (callers should default
    /// to an empty slice).
    pub fn get_message_folder_ids_batch(
        &self,
        message_ids: &[String],
    ) -> Result<HashMap<String, Vec<String>>> {
        if message_ids.is_empty() {
            return Ok(HashMap::new());
        }
        self.with_read(|conn| {
            let placeholders: Vec<String> =
                (1..=message_ids.len()).map(|i| format!("?{}", i)).collect();
            let sql = format!(
                "SELECT message_id, folder_id FROM message_folders
                 WHERE message_id IN ({})",
                placeholders.join(", "),
            );
            let mut stmt = conn.prepare(&sql)?;
            let param_values: Vec<&dyn rusqlite::types::ToSql> = message_ids
                .iter()
                .map(|s| s as &dyn rusqlite::types::ToSql)
                .collect();
            let rows = stmt.query_map(param_values.as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut out: HashMap<String, Vec<String>> = HashMap::new();
            for row in rows {
                let (mid, fid) = row?;
                out.entry(mid).or_default().push(fid);
            }
            Ok(out)
        })
    }

    /// List messages across multiple folders.
    pub fn list_messages_by_folders(
        &self,
        folder_ids: &[String],
        limit: u32,
        offset: u32,
    ) -> Result<Vec<MessageSummary>> {
        if folder_ids.is_empty() {
            return Ok(Vec::new());
        }
        if folder_ids.len() == 1 {
            return self.list_messages_by_folder(&folder_ids[0], limit, offset);
        }
        self.with_read(|conn| {
            let placeholders: Vec<String> =
                (1..=folder_ids.len()).map(|i| format!("?{}", i)).collect();
            let sql = format!(
                "SELECT DISTINCT m.{} FROM messages m
                 JOIN message_folders mf ON m.id = mf.message_id
                 WHERE mf.folder_id IN ({}) AND m.is_deleted = 0
                 ORDER BY m.date DESC
                 LIMIT ?{} OFFSET ?{}",
                MSG_SUMMARY_SELECT.replace(", ", ", m."),
                placeholders.join(", "),
                folder_ids.len() + 1,
                folder_ids.len() + 2,
            );
            let mut stmt = conn.prepare(&sql)?;

            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            for fid in folder_ids {
                param_values.push(Box::new(fid.clone()));
            }
            param_values.push(Box::new(limit));
            param_values.push(Box::new(offset));

            let params_ref: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(|v| v.as_ref()).collect();
            let rows = stmt.query_map(params_ref.as_slice(), row_to_message_summary)?;
            let mut messages = Vec::new();
            for row in rows {
                messages.push(row?);
            }
            Ok(messages)
        })
    }

    pub fn get_message(&self, id: &str) -> Result<Option<Message>> {
        self.with_read(|conn| {
            let sql = format!("SELECT {MSG_SELECT} FROM messages WHERE id = ?1");
            let result = conn
                .query_row(&sql, params![id], row_to_message)
                .optional()?;
            Ok(result)
        })
    }

    pub fn get_messages_batch(&self, ids: &[String]) -> Result<Vec<Message>> {
        if ids.is_empty() {
            return Ok(Vec::new());
        }

        self.with_read(|conn| {
            let placeholders: Vec<String> = (1..=ids.len()).map(|i| format!("?{i}")).collect();
            let sql = format!(
                "SELECT {MSG_SELECT} FROM messages WHERE id IN ({})",
                placeholders.join(", ")
            );
            let mut stmt = conn.prepare(&sql)?;

            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> =
                Vec::with_capacity(ids.len());
            for id in ids {
                param_values.push(Box::new(id.clone()));
            }
            let params: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(|v| v.as_ref()).collect();

            let rows = stmt.query_map(params.as_slice(), row_to_message)?;

            let mut by_id = HashMap::new();
            for row in rows {
                let message = row?;
                by_id.insert(message.id.clone(), message);
            }

            let mut ordered = Vec::with_capacity(ids.len());
            for id in ids {
                if let Some(message) = by_id.remove(id) {
                    ordered.push(message);
                }
            }
            Ok(ordered)
        })
    }

    pub fn update_message_flags(
        &self,
        id: &str,
        is_read: Option<bool>,
        is_starred: Option<bool>,
    ) -> Result<()> {
        self.with_write(|conn| {
            let mut sets = Vec::new();
            let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();

            if let Some(read) = is_read {
                sets.push(format!("is_read = ?{}", values.len() + 1));
                values.push(Box::new(read as i32));
            }
            if let Some(starred) = is_starred {
                sets.push(format!("is_starred = ?{}", values.len() + 1));
                values.push(Box::new(starred as i32));
            }

            if sets.is_empty() {
                return Ok(());
            }

            let now = pebble_core::now_timestamp();
            sets.push(format!("updated_at = ?{}", values.len() + 1));
            values.push(Box::new(now));

            let id_idx = values.len() + 1;
            values.push(Box::new(id.to_string()));

            let sql = format!(
                "UPDATE messages SET {} WHERE id = ?{}",
                sets.join(", "),
                id_idx
            );
            let params: Vec<&dyn rusqlite::types::ToSql> =
                values.iter().map(|v| v.as_ref()).collect();
            conn.execute(&sql, params.as_slice())?;

            Ok(())
        })
    }

    /// Move a message from its current folder(s) to a target folder.
    /// Clears any soft-delete flag so the message is visible in the new folder.
    pub fn move_message_to_folder(&self, message_id: &str, target_folder_id: &str) -> Result<()> {
        self.with_write(|conn| {
            let now = pebble_core::now_timestamp();
            let tx = conn.unchecked_transaction()?;

            // Remove all existing folder associations
            tx.execute(
                "DELETE FROM message_folders WHERE message_id = ?1",
                params![message_id],
            )?;

            // Insert into target folder
            tx.execute(
                "INSERT INTO message_folders (message_id, folder_id) VALUES (?1, ?2)",
                params![message_id, target_folder_id],
            )?;

            // Clear soft-delete flag so message is visible
            tx.execute(
                "UPDATE messages SET is_deleted = 0, deleted_at = NULL, updated_at = ?1 WHERE id = ?2",
                params![now, message_id],
            )?;

            tx.commit()?;
            Ok(())
        })
    }

    pub fn update_remote_id(&self, message_id: &str, new_remote_id: &str) -> Result<()> {
        self.with_write(|conn| {
            let now = pebble_core::now_timestamp();
            let changed = conn.execute(
                "UPDATE messages SET remote_id = ?1, updated_at = ?2 WHERE id = ?3",
                params![new_remote_id, now, message_id],
            )?;
            if changed == 0 {
                return Err(PebbleError::Internal(format!(
                    "Message not found for remote_id update: {message_id}"
                )));
            }
            Ok(())
        })
    }

    pub fn add_message_to_folder(&self, message_id: &str, folder_id: &str) -> Result<()> {
        self.with_write(|conn| {
            let now = pebble_core::now_timestamp();
            conn.execute(
                "INSERT OR IGNORE INTO message_folders (message_id, folder_id) VALUES (?1, ?2)",
                params![message_id, folder_id],
            )?;
            conn.execute(
                "UPDATE messages SET is_deleted = 0, deleted_at = NULL, updated_at = ?1 WHERE id = ?2",
                params![now, message_id],
            )?;
            Ok(())
        })
    }

    pub fn remove_message_from_folder(&self, message_id: &str, folder_id: &str) -> Result<()> {
        self.with_write(|conn| {
            let now = pebble_core::now_timestamp();
            let tx = conn.unchecked_transaction()?;

            tx.execute(
                "DELETE FROM message_folders WHERE message_id = ?1 AND folder_id = ?2",
                params![message_id, folder_id],
            )?;

            let remaining: i64 = tx
                .query_row(
                    "SELECT COUNT(*) FROM message_folders WHERE message_id = ?1",
                    params![message_id],
                    |row| row.get(0),
                )?;

            if remaining == 0 {
                tx.execute(
                    "UPDATE messages SET is_deleted = 1, deleted_at = ?1, updated_at = ?1 WHERE id = ?2",
                    params![now, message_id],
                )?;
            } else {
                tx.execute(
                    "UPDATE messages SET updated_at = ?1 WHERE id = ?2",
                    params![now, message_id],
                )?;
            }

            tx.commit()?;
            Ok(())
        })
    }

    pub fn soft_delete_message(&self, id: &str) -> Result<()> {
        self.with_write(|conn| {
            let now = pebble_core::now_timestamp();
            conn.execute(
                "UPDATE messages SET is_deleted = 1, deleted_at = ?1, updated_at = ?1 WHERE id = ?2",
                params![now, id],
            )?;
            Ok(())
        })
    }

    /// Check whether a message with the given `remote_id` exists for this account.
    pub fn has_message_by_remote_id(&self, account_id: &str, remote_id: &str) -> Result<bool> {
        self.with_read(|conn| {
            let count: i64 = conn
                .query_row(
                    "SELECT COUNT(*) FROM messages WHERE account_id = ?1 AND remote_id = ?2 AND is_deleted = 0",
                    params![account_id, remote_id],
                    |row| row.get(0),
                )?;
            Ok(count > 0)
        })
    }

    /// Find a local message ID by its remote (Gmail/IMAP) ID.
    pub fn find_message_id_by_remote(
        &self,
        account_id: &str,
        remote_id: &str,
    ) -> Result<Option<String>> {
        self.with_read(|conn| {
            let result = conn
                .query_row(
                    "SELECT id FROM messages WHERE account_id = ?1 AND remote_id = ?2 AND is_deleted = 0",
                    params![account_id, remote_id],
                    |row| row.get(0),
                );
            match result {
                Ok(id) => Ok(Some(id)),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(PebbleError::Storage(e.to_string())),
            }
        })
    }

    /// Bulk-check which remote IDs already exist for an account.
    /// Returns a HashSet of remote_id strings that are already stored.
    pub fn get_existing_remote_ids(
        &self,
        account_id: &str,
        remote_ids: &[String],
    ) -> Result<std::collections::HashSet<String>> {
        use std::collections::HashSet;
        if remote_ids.is_empty() {
            return Ok(HashSet::new());
        }
        self.with_read(|conn| {
            let placeholders: Vec<String> = (0..remote_ids.len())
                .map(|i| format!("?{}", i + 2))
                .collect();
            let sql = format!(
                "SELECT remote_id FROM messages WHERE account_id = ?1 AND remote_id IN ({}) AND is_deleted = 0",
                placeholders.join(", ")
            );
            let mut stmt = conn.prepare(&sql)?;
            let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::with_capacity(remote_ids.len() + 1);
            params_vec.push(Box::new(account_id.to_string()));
            for rid in remote_ids {
                params_vec.push(Box::new(rid.clone()));
            }
            let param_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
            let rows = stmt.query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))?;
            let mut result = HashSet::new();
            for row in rows {
                result.insert(row?);
            }
            Ok(result)
        })
    }

    /// Bulk-check which remote IDs already exist for an account inside one folder.
    /// IMAP UIDs are only unique within a mailbox, so callers must use this
    /// instead of account-wide lookup when syncing IMAP folders.
    pub fn get_existing_remote_ids_in_folder(
        &self,
        account_id: &str,
        folder_id: &str,
        remote_ids: &[String],
    ) -> Result<std::collections::HashSet<String>> {
        use std::collections::HashSet;
        if remote_ids.is_empty() {
            return Ok(HashSet::new());
        }

        self.with_read(|conn| {
            let placeholders: Vec<String> = (0..remote_ids.len())
                .map(|i| format!("?{}", i + 3))
                .collect();
            let sql = format!(
                "SELECT DISTINCT m.remote_id
                 FROM messages m
                 JOIN message_folders mf ON m.id = mf.message_id
                 WHERE m.account_id = ?1
                   AND mf.folder_id = ?2
                   AND m.remote_id IN ({})
                   AND m.is_deleted = 0",
                placeholders.join(", ")
            );
            let mut stmt = conn.prepare(&sql)?;
            let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> =
                Vec::with_capacity(remote_ids.len() + 2);
            params_vec.push(Box::new(account_id.to_string()));
            params_vec.push(Box::new(folder_id.to_string()));
            for rid in remote_ids {
                params_vec.push(Box::new(rid.clone()));
            }
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params_vec.iter().map(|p| p.as_ref()).collect();
            let rows = stmt.query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))?;

            let mut result = HashSet::new();
            for row in rows {
                result.insert(row?);
            }
            Ok(result)
        })
    }

    pub fn get_existing_message_map_by_remote_ids(
        &self,
        account_id: &str,
        remote_ids: &[String],
    ) -> Result<HashMap<String, String>> {
        if remote_ids.is_empty() {
            return Ok(HashMap::new());
        }

        self.with_read(|conn| {
            let placeholders: Vec<String> = (0..remote_ids.len())
                .map(|i| format!("?{}", i + 2))
                .collect();
            let sql = format!(
                "SELECT remote_id, id FROM messages WHERE account_id = ?1 AND remote_id IN ({}) AND is_deleted = 0",
                placeholders.join(", ")
            );
            let mut stmt = conn
                .prepare(&sql)?;
            let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> =
                Vec::with_capacity(remote_ids.len() + 1);
            params_vec.push(Box::new(account_id.to_string()));
            for remote_id in remote_ids {
                params_vec.push(Box::new(remote_id.clone()));
            }
            let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                params_vec.iter().map(|p| p.as_ref()).collect();
            let rows = stmt
                .query_map(param_refs.as_slice(), |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;

            let mut result = HashMap::new();
            for row in rows {
                let (remote_id, message_id) =
                    row?;
                result.insert(remote_id, message_id);
            }
            Ok(result)
        })
    }

    /// Find legacy half-written rows that claim attachments but have no
    /// attachment records. Callers must force-refetch these instead of
    /// treating their remote IDs as complete sync hits.
    pub fn get_incomplete_attachment_message_map_by_remote_ids(
        &self,
        account_id: &str,
        folder_id: Option<&str>,
        remote_ids: &[String],
    ) -> Result<HashMap<String, String>> {
        if remote_ids.is_empty() {
            return Ok(HashMap::new());
        }

        self.with_read(|conn| {
            let first_remote_param = if folder_id.is_some() { 3 } else { 2 };
            let placeholders: Vec<String> = (0..remote_ids.len())
                .map(|index| format!("?{}", index + first_remote_param))
                .collect();
            let folder_join = if folder_id.is_some() {
                "JOIN message_folders mf ON mf.message_id = m.id"
            } else {
                ""
            };
            let folder_filter = if folder_id.is_some() {
                "AND mf.folder_id = ?2"
            } else {
                ""
            };
            let sql = format!(
                "SELECT m.remote_id, m.id
                 FROM messages m
                 {folder_join}
                 WHERE m.account_id = ?1
                   {folder_filter}
                   AND m.remote_id IN ({})
                   AND m.is_deleted = 0
                   AND m.has_attachments = 1
                   AND NOT EXISTS (
                       SELECT 1 FROM attachments a WHERE a.message_id = m.id
                   )",
                placeholders.join(", ")
            );
            let mut values: Vec<Box<dyn rusqlite::types::ToSql>> =
                Vec::with_capacity(remote_ids.len() + 2);
            values.push(Box::new(account_id.to_string()));
            if let Some(folder_id) = folder_id {
                values.push(Box::new(folder_id.to_string()));
            }
            for remote_id in remote_ids {
                values.push(Box::new(remote_id.clone()));
            }
            let params: Vec<&dyn rusqlite::types::ToSql> =
                values.iter().map(|value| value.as_ref()).collect();
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params.as_slice(), |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut result = HashMap::new();
            for row in rows {
                let (remote_id, message_id) = row?;
                result.insert(remote_id, message_id);
            }
            Ok(result)
        })
    }

    /// Atomically replace one IMAP mailbox after UIDVALIDITY changes.
    ///
    /// A unique, non-empty Message-ID lets an unchanged single-mailbox
    /// message reuse its local ID, preserving user-owned relations. Missing
    /// or ambiguous Message-IDs never inherit local state. Search recovery
    /// work and obsolete attachment discovery are committed with the data.
    pub fn reconcile_imap_folder_uidvalidity(
        &self,
        account_id: &str,
        folder_id: &str,
        mut fresh: Vec<ImapFolderSnapshotMessage>,
    ) -> Result<ImapFolderReconcileResult> {
        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()?;
            let old_messages = {
                let sql = format!(
                    "SELECT m.{MSG_SELECT}
                     FROM messages m
                     JOIN message_folders mf ON mf.message_id = m.id
                     WHERE m.account_id = ?1
                       AND mf.folder_id = ?2
                       AND m.remote_id != ''
                       AND m.remote_id NOT GLOB '*[^0-9]*'"
                );
                let mut stmt = tx.prepare(&sql)?;
                let rows = stmt.query_map(params![account_id, folder_id], row_to_message)?;
                let mut messages = Vec::new();
                for row in rows {
                    messages.push(row?);
                }
                messages
            };
            let old_attachment_paths = {
                let mut stmt = tx.prepare(
                    "SELECT DISTINCT a.local_path
                     FROM attachments a
                     JOIN messages m ON m.id = a.message_id
                     JOIN message_folders mf ON mf.message_id = m.id
                     WHERE m.account_id = ?1
                       AND mf.folder_id = ?2
                       AND m.remote_id != ''
                       AND m.remote_id NOT GLOB '*[^0-9]*'
                       AND a.local_path IS NOT NULL",
                )?;
                let rows = stmt.query_map(params![account_id, folder_id], |row| {
                    row.get::<_, String>(0)
                })?;
                let mut paths = Vec::new();
                for row in rows {
                    paths.push(row?);
                }
                paths
            };

            let mut old_header_counts: HashMap<String, usize> = HashMap::new();
            let mut old_by_header: HashMap<String, Message> = HashMap::new();
            for message in &old_messages {
                if let Some(header) = message
                    .message_id_header
                    .as_deref()
                    .filter(|header| !header.is_empty())
                {
                    *old_header_counts.entry(header.to_string()).or_default() += 1;
                    old_by_header.insert(header.to_string(), message.clone());
                }
            }
            let mut fresh_header_counts: HashMap<String, usize> = HashMap::new();
            for entry in &fresh {
                if let Some(header) = entry
                    .message
                    .message_id_header
                    .as_deref()
                    .filter(|header| !header.is_empty())
                {
                    *fresh_header_counts.entry(header.to_string()).or_default() += 1;
                }
            }

            let mut matched_old_ids = HashSet::new();
            for entry in &mut fresh {
                let Some(header) = entry
                    .message
                    .message_id_header
                    .as_deref()
                    .filter(|header| !header.is_empty())
                else {
                    continue;
                };
                if old_header_counts.get(header) != Some(&1)
                    || fresh_header_counts.get(header) != Some(&1)
                {
                    continue;
                }
                let Some(old) = old_by_header.get(header) else {
                    continue;
                };
                let folder_count: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM message_folders WHERE message_id = ?1",
                    params![old.id],
                    |row| row.get(0),
                )?;
                if folder_count != 1 {
                    continue;
                }

                matched_old_ids.insert(old.id.clone());
                entry.message.id = old.id.clone();
                entry.message.is_read = old.is_read;
                entry.message.is_starred = old.is_starred;
                entry.message.created_at = old.created_at;
                for attachment in &mut entry.attachments {
                    attachment.message_id = old.id.clone();
                }
            }

            let now = pebble_core::now_timestamp();
            let mut indexed_message_ids = Vec::new();
            let mut removed_messages = Vec::new();
            // Release the old mailbox-scoped UID namespace before applying
            // any updates. This also handles valid UID swaps (A:1/B:2 ->
            // A:2/B:1) without tripping the same-folder uniqueness trigger.
            for old in &old_messages {
                tx.execute(
                    "DELETE FROM message_folders WHERE message_id = ?1 AND folder_id = ?2",
                    params![old.id, folder_id],
                )?;
            }
            for old in &old_messages {
                if matched_old_ids.contains(&old.id) {
                    continue;
                }
                let remaining_folder_count: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM message_folders WHERE message_id = ?1",
                    params![old.id],
                    |row| row.get(0),
                )?;
                if remaining_folder_count > 0 {
                    indexed_message_ids.push(old.id.clone());
                    tx.execute(
                        "INSERT OR REPLACE INTO search_pending
                             (message_id, operation, created_at)
                         VALUES (?1, 'index', ?2)",
                        params![old.id, now],
                    )?;
                } else {
                    tx.execute(
                        "DELETE FROM pending_mail_ops WHERE message_id = ?1",
                        params![old.id],
                    )?;
                    tx.execute("DELETE FROM messages WHERE id = ?1", params![old.id])?;
                    removed_messages.push(old.clone());
                    tx.execute(
                        "INSERT OR REPLACE INTO search_pending
                             (message_id, operation, created_at)
                         VALUES (?1, 'remove', ?2)",
                        params![old.id, now],
                    )?;
                }
            }

            for entry in &fresh {
                upsert_message_with_conn(&tx, &entry.message)?;
                tx.execute(
                    "INSERT OR IGNORE INTO message_folders (message_id, folder_id)
                     VALUES (?1, ?2)",
                    params![entry.message.id, folder_id],
                )?;
                tx.execute(
                    "DELETE FROM attachments WHERE message_id = ?1",
                    params![entry.message.id],
                )?;
                for attachment in &entry.attachments {
                    tx.execute(
                        "INSERT INTO attachments
                             (id, message_id, filename, mime_type, size,
                              local_path, content_id, is_inline)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        params![
                            attachment.id,
                            entry.message.id,
                            attachment.filename,
                            attachment.mime_type,
                            attachment.size,
                            attachment.local_path,
                            attachment.content_id,
                            attachment.is_inline as i32,
                        ],
                    )?;
                }
                indexed_message_ids.push(entry.message.id.clone());
                tx.execute(
                    "INSERT OR REPLACE INTO search_pending
                         (message_id, operation, created_at)
                     VALUES (?1, 'index', ?2)",
                    params![entry.message.id, now],
                )?;
            }

            tx.execute(
                "DELETE FROM sync_failures WHERE account_id = ?1 AND folder_id = ?2",
                params![account_id, folder_id],
            )?;

            let mut unreferenced_paths = Vec::new();
            for path in old_attachment_paths {
                let still_referenced: bool = tx.query_row(
                    "SELECT EXISTS(SELECT 1 FROM attachments WHERE local_path = ?1)",
                    params![path],
                    |row| row.get(0),
                )?;
                if !still_referenced {
                    unreferenced_paths.push(path);
                }
            }

            tx.commit()?;
            Ok(ImapFolderReconcileResult {
                indexed_message_ids,
                removed_messages,
                unreferenced_attachment_paths: unreferenced_paths,
            })
        })
    }

    /// Get the maximum remote_id (interpreted as integer) for messages in a folder.
    pub fn get_max_remote_id(&self, account_id: &str, folder_id: &str) -> Result<Option<String>> {
        self.with_read(|conn| {
            let result: Option<i64> = conn.query_row(
                "SELECT MAX(CAST(m.remote_id AS INTEGER))
                     FROM messages m
                     JOIN message_folders mf ON m.id = mf.message_id
                     WHERE m.account_id = ?1 AND mf.folder_id = ?2 AND m.is_deleted = 0",
                params![account_id, folder_id],
                |row| row.get(0),
            )?;
            Ok(result.map(|v| v.to_string()))
        })
    }

    /// List (message_id, remote_id, is_read, is_starred, updated_at) for non-deleted messages in a folder.
    pub fn list_remote_ids_by_folder(
        &self,
        account_id: &str,
        folder_id: &str,
    ) -> Result<Vec<FolderRemoteMessageState>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT m.id, m.remote_id, m.is_read, m.is_starred, m.updated_at
                 FROM messages m
                 JOIN message_folders mf ON m.id = mf.message_id
                 WHERE m.account_id = ?1 AND mf.folder_id = ?2 AND m.is_deleted = 0",
            )?;
            let rows = stmt.query_map(params![account_id, folder_id], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i32>(2)? != 0,
                    row.get::<_, i32>(3)? != 0,
                    row.get::<_, i64>(4)?,
                ))
            })?;
            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
        })
    }

    pub fn list_cached_remote_ids_by_account(&self, account_id: &str) -> Result<Vec<String>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT DISTINCT remote_id FROM messages
                 WHERE account_id = ?1 AND is_deleted = 0 AND remote_id != ''",
            )?;
            let rows = stmt.query_map(params![account_id], |row| row.get::<_, String>(0))?;
            let mut remote_ids = Vec::new();
            for row in rows {
                remote_ids.push(row?);
            }
            Ok(remote_ids)
        })
    }

    /// Get the folder IDs that contain a given message.
    pub fn get_message_folder_ids(&self, message_id: &str) -> Result<Vec<String>> {
        self.with_read(|conn| {
            let mut stmt =
                conn.prepare("SELECT folder_id FROM message_folders WHERE message_id = ?1")?;
            let rows = stmt.query_map(params![message_id], |row| row.get::<_, String>(0))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            Ok(ids)
        })
    }

    /// Batch update flags for multiple messages in a transaction.
    pub fn bulk_update_flags(
        &self,
        changes: &[(String, Option<bool>, Option<bool>)],
    ) -> Result<()> {
        if changes.is_empty() {
            return Ok(());
        }
        self.with_write(|conn| {
            let now = pebble_core::now_timestamp();
            let tx = conn.unchecked_transaction()?;

            for (msg_id, is_read, is_starred) in changes {
                let mut sets = Vec::new();
                let mut values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
                if let Some(read) = is_read {
                    sets.push(format!("is_read = ?{}", values.len() + 1));
                    values.push(Box::new(*read as i32));
                }
                if let Some(starred) = is_starred {
                    sets.push(format!("is_starred = ?{}", values.len() + 1));
                    values.push(Box::new(*starred as i32));
                }
                if sets.is_empty() {
                    continue;
                }
                sets.push(format!("updated_at = ?{}", values.len() + 1));
                values.push(Box::new(now));
                let id_idx = values.len() + 1;
                values.push(Box::new(msg_id.clone()));
                let sql = format!(
                    "UPDATE messages SET {} WHERE id = ?{}",
                    sets.join(", "),
                    id_idx
                );
                let params: Vec<&dyn rusqlite::types::ToSql> =
                    values.iter().map(|v| v.as_ref()).collect();
                tx.execute(&sql, params.as_slice())?;
            }

            tx.commit()?;
            Ok(())
        })
    }

    /// Batch soft-delete multiple messages.
    pub fn bulk_soft_delete(&self, message_ids: &[String]) -> Result<()> {
        if message_ids.is_empty() {
            return Ok(());
        }
        self.with_write(|conn| {
            let now = pebble_core::now_timestamp();
            let placeholders: Vec<String> = (0..message_ids.len())
                .map(|i| format!("?{}", i + 2))
                .collect();
            let sql = format!(
                "UPDATE messages SET is_deleted = 1, deleted_at = ?1, updated_at = ?1 WHERE id IN ({})",
                placeholders.join(", ")
            );
            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::with_capacity(message_ids.len() + 1);
            param_values.push(Box::new(now));
            for id in message_ids {
                param_values.push(Box::new(id.clone()));
            }
            let params: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|v| v.as_ref()).collect();
            conn.execute(&sql, params.as_slice())?;
            Ok(())
        })
    }

    /// Physically delete messages and their folder associations immediately.
    pub fn hard_delete_messages(&self, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()?;

            // Batch delete using IN clause for better performance
            for chunk in ids.chunks(100) {
                let placeholders: String = chunk
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", i + 1))
                    .collect::<Vec<_>>()
                    .join(",");

                let sql_folders = format!(
                    "DELETE FROM message_folders WHERE message_id IN ({})",
                    placeholders
                );
                let sql_messages = format!("DELETE FROM messages WHERE id IN ({})", placeholders);

                let params: Vec<&dyn rusqlite::types::ToSql> = chunk
                    .iter()
                    .map(|id| id as &dyn rusqlite::types::ToSql)
                    .collect();

                tx.execute(&sql_folders, params.as_slice())?;
                tx.execute(&sql_messages, params.as_slice())?;
            }

            tx.commit()?;
            Ok(())
        })
    }

    /// Physically delete messages that were soft-deleted more than `older_than_secs` seconds ago.
    /// Returns the number of purged messages.
    pub fn purge_old_tombstones(&self, older_than_secs: i64) -> Result<u32> {
        self.with_write(|conn| {
            let cutoff = pebble_core::now_timestamp() - older_than_secs;
            let count = conn.execute(
                "DELETE FROM messages WHERE is_deleted = 1 AND deleted_at IS NOT NULL AND deleted_at < ?1",
                params![cutoff],
            )?;
            Ok(count as u32)
        })
    }

    /// Return all message IDs belonging to an account (including soft-deleted).
    pub fn list_message_ids_by_account(&self, account_id: &str) -> Result<Vec<String>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare("SELECT id FROM messages WHERE account_id = ?1")?;
            let rows = stmt.query_map(params![account_id], |row| row.get(0))?;
            let mut ids = Vec::new();
            for row in rows {
                ids.push(row?);
            }
            Ok(ids)
        })
    }

    /// List all messages in a thread, ordered chronologically.
    pub fn list_messages_by_thread(&self, thread_id: &str) -> Result<Vec<Message>> {
        self.with_read(|conn| {
            let sql = format!(
                "SELECT {} FROM messages WHERE thread_id = ?1 AND is_deleted = 0 ORDER BY date ASC",
                MSG_SELECT
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![thread_id], row_to_message)?;
            let mut messages = Vec::new();
            for row in rows {
                messages.push(row?);
            }
            Ok(messages)
        })
    }

    /// List thread summaries for a folder, ordered by most recent message.
    ///
    /// The `max_date` subquery is scoped to the target folder so we aggregate
    /// only over messages that actually live in this folder. This avoids a
    /// full-table thread scan and also ensures the snippet reflects the most
    /// recent message *in this folder* (not a possibly-newer message that sits
    /// in a different folder, which previously produced an empty snippet).
    pub fn list_threads_by_folder(
        &self,
        folder_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<pebble_core::ThreadSummary>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(
                "WITH thread_participants AS (
                        SELECT thread_id,
                               GROUP_CONCAT(from_address, '||') AS participants
                        FROM (
                            SELECT DISTINCT m3.thread_id, m3.from_address
                            FROM messages m3
                            JOIN message_folders mf3 ON m3.id = mf3.message_id
                            WHERE mf3.folder_id = ?1
                              AND m3.is_deleted = 0
                              AND m3.thread_id IS NOT NULL
                        )
                        GROUP BY thread_id
                     )
                     SELECT
                        m.thread_id,
                        MAX(m.subject) as subject,
                        MAX(CASE WHEN m.date = max_date.md THEN m.snippet ELSE '' END) as snippet,
                        MAX(m.date) as last_date,
                        COUNT(*) as message_count,
                        SUM(CASE WHEN m.is_read = 0 THEN 1 ELSE 0 END) as unread_count,
                        MAX(m.is_starred) as is_starred,
                        COALESCE(tp.participants, '') as participants,
                        MAX(m.has_attachments) as has_attachments
                     FROM messages m
                     JOIN message_folders mf ON m.id = mf.message_id
                     JOIN (
                        SELECT m2.thread_id, MAX(m2.date) as md
                        FROM messages m2
                        JOIN message_folders mf2 ON m2.id = mf2.message_id
                        WHERE mf2.folder_id = ?1
                          AND m2.is_deleted = 0
                          AND m2.thread_id IS NOT NULL
                        GROUP BY m2.thread_id
                     ) max_date ON m.thread_id = max_date.thread_id
                     LEFT JOIN thread_participants tp ON m.thread_id = tp.thread_id
                     WHERE mf.folder_id = ?1 AND m.is_deleted = 0 AND m.thread_id IS NOT NULL
                     GROUP BY m.thread_id
                     ORDER BY last_date DESC
                     LIMIT ?2 OFFSET ?3",
            )?;

            let rows = stmt.query_map(params![folder_id, limit, offset], |row| {
                let participants_str: String = row.get(7)?;
                let participants: Vec<String> = participants_str
                    .split("||")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                let is_starred: i32 = row.get(6)?;
                let has_attachments: i32 = row.get(8)?;
                Ok(pebble_core::ThreadSummary {
                    thread_id: row.get(0)?,
                    subject: row.get(1)?,
                    snippet: row.get(2)?,
                    last_date: row.get(3)?,
                    message_count: row.get::<_, i64>(4)? as u32,
                    unread_count: row.get::<_, i64>(5)? as u32,
                    is_starred: is_starred != 0,
                    participants,
                    has_attachments: has_attachments != 0,
                })
            })?;

            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
        })
    }

    /// List thread summaries across multiple folders, ordered by most recent
    /// selected message. Messages that are present in more than one selected
    /// folder are counted once.
    pub fn list_threads_by_folders(
        &self,
        folder_ids: &[String],
        limit: u32,
        offset: u32,
    ) -> Result<Vec<pebble_core::ThreadSummary>> {
        if folder_ids.is_empty() {
            return Ok(Vec::new());
        }
        if folder_ids.len() == 1 {
            return self.list_threads_by_folder(&folder_ids[0], limit, offset);
        }

        self.with_read(|conn| {
            let placeholders: Vec<String> =
                (1..=folder_ids.len()).map(|i| format!("?{}", i)).collect();
            let sql = format!(
                "WITH selected_messages AS (
                        SELECT DISTINCT
                               m.id,
                               m.thread_id,
                               m.subject,
                               m.snippet,
                               m.date,
                               m.is_read,
                               m.is_starred,
                               m.from_address,
                               m.has_attachments
                        FROM messages m
                        JOIN message_folders mf ON m.id = mf.message_id
                        WHERE mf.folder_id IN ({})
                          AND m.is_deleted = 0
                          AND m.thread_id IS NOT NULL
                     ),
                     thread_participants AS (
                        SELECT thread_id,
                               GROUP_CONCAT(from_address, '||') AS participants
                        FROM (
                            SELECT DISTINCT thread_id, from_address
                            FROM selected_messages
                        )
                        GROUP BY thread_id
                     ),
                     max_date AS (
                        SELECT thread_id, MAX(date) AS md
                        FROM selected_messages
                        GROUP BY thread_id
                     )
                     SELECT
                        sm.thread_id,
                        MAX(sm.subject) AS subject,
                        MAX(CASE WHEN sm.date = max_date.md THEN sm.snippet ELSE '' END) AS snippet,
                        MAX(sm.date) AS last_date,
                        COUNT(*) AS message_count,
                        SUM(CASE WHEN sm.is_read = 0 THEN 1 ELSE 0 END) AS unread_count,
                        MAX(sm.is_starred) AS is_starred,
                        COALESCE(tp.participants, '') AS participants,
                        MAX(sm.has_attachments) AS has_attachments
                     FROM selected_messages sm
                     JOIN max_date ON sm.thread_id = max_date.thread_id
                     LEFT JOIN thread_participants tp ON sm.thread_id = tp.thread_id
                     GROUP BY sm.thread_id
                     ORDER BY last_date DESC
                     LIMIT ?{} OFFSET ?{}",
                placeholders.join(", "),
                folder_ids.len() + 1,
                folder_ids.len() + 2,
            );
            let mut stmt = conn.prepare(&sql)?;

            let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
            for fid in folder_ids {
                param_values.push(Box::new(fid.clone()));
            }
            param_values.push(Box::new(limit));
            param_values.push(Box::new(offset));
            let params_ref: Vec<&dyn rusqlite::types::ToSql> =
                param_values.iter().map(|v| v.as_ref()).collect();

            let rows = stmt.query_map(params_ref.as_slice(), |row| {
                let participants_str: String = row.get(7)?;
                let participants: Vec<String> = participants_str
                    .split("||")
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                let is_starred: i32 = row.get(6)?;
                let has_attachments: i32 = row.get(8)?;
                Ok(pebble_core::ThreadSummary {
                    thread_id: row.get(0)?,
                    subject: row.get(1)?,
                    snippet: row.get(2)?,
                    last_date: row.get(3)?,
                    message_count: row.get::<_, i64>(4)? as u32,
                    unread_count: row.get::<_, i64>(5)? as u32,
                    is_starred: is_starred != 0,
                    participants,
                    has_attachments: has_attachments != 0,
                })
            })?;

            let mut results = Vec::new();
            for row in rows {
                results.push(row?);
            }
            Ok(results)
        })
    }

    /// Get all message-id to thread-id mappings for an account.
    /// Returns a HashMap for O(1) lookup during thread computation.
    pub fn get_thread_mappings(
        &self,
        account_id: &str,
    ) -> Result<std::collections::HashMap<String, String>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT message_id_header, thread_id
                     FROM messages
                     WHERE account_id = ?1
                       AND message_id_header IS NOT NULL
                       AND thread_id IS NOT NULL
                       AND is_deleted = 0",
            )?;
            let rows = stmt.query_map(params![account_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
            })?;
            let mut results = std::collections::HashMap::new();
            for row in rows {
                let (mid, tid) = row?;
                results.insert(mid, tid);
            }
            Ok(results)
        })
    }

    /// Get (message_id_header → thread_id) mappings only for the given ref IDs.
    /// Used by sync to avoid loading the full account mapping on every batch.
    pub fn get_thread_mappings_for_refs(
        &self,
        account_id: &str,
        ref_ids: &[String],
    ) -> Result<std::collections::HashMap<String, String>> {
        if ref_ids.is_empty() {
            return Ok(std::collections::HashMap::new());
        }
        self.with_read(|conn| {
            let mut results = std::collections::HashMap::new();
            for chunk in ref_ids.chunks(500) {
                let placeholders: String = chunk
                    .iter()
                    .enumerate()
                    .map(|(i, _)| format!("?{}", i + 2))
                    .collect::<Vec<_>>()
                    .join(",");
                let sql = format!(
                    "SELECT message_id_header, thread_id FROM messages \
                     WHERE account_id = ?1 \
                       AND message_id_header IN ({placeholders}) \
                       AND thread_id IS NOT NULL \
                       AND is_deleted = 0"
                );
                let mut stmt = conn.prepare(&sql)?;
                let mut params: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::new();
                params.push(Box::new(account_id.to_string()));
                for id in chunk {
                    params.push(Box::new(id.clone()));
                }
                let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                    params.iter().map(|p| p.as_ref()).collect();
                let rows = stmt.query_map(param_refs.as_slice(), |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
                for row in rows {
                    let (mid, tid) = row?;
                    results.insert(mid, tid);
                }
            }
            Ok(results)
        })
    }

    /// Count total non-deleted messages across all accounts.
    pub fn count_all_messages(&self) -> Result<u64> {
        self.with_read(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*) FROM messages WHERE is_deleted = 0",
                [],
                |row| row.get(0),
            )?;
            Ok(count as u64)
        })
    }

    /// Count unread messages per folder for an account.
    pub fn get_folder_unread_counts(
        &self,
        account_id: &str,
    ) -> Result<std::collections::HashMap<String, u32>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT mf.folder_id, COUNT(*)
                 FROM messages m
                 JOIN message_folders mf ON m.id = mf.message_id
                 WHERE m.account_id = ?1 AND m.is_read = 0 AND m.is_deleted = 0
                 GROUP BY mf.folder_id",
            )?;
            let rows = stmt.query_map(params![account_id], |row| {
                Ok((row.get::<_, String>(0)?, row.get::<_, u32>(1)?))
            })?;
            let mut counts = std::collections::HashMap::new();
            for row in rows {
                let (fid, count) = row?;
                counts.insert(fid, count);
            }
            Ok(counts)
        })
    }
}

#[cfg(test)]
mod remote_id_scope_tests {
    use super::ImapFolderSnapshotMessage;
    use crate::Store;
    use pebble_core::*;
    use std::collections::HashMap;

    fn make_account() -> Account {
        let now = now_timestamp();
        Account {
            account_label: None,
            provider_display_name: None,
            id: new_id(),
            email: "imap@example.com".to_string(),
            display_name: "IMAP".to_string(),
            color: None,
            provider: ProviderType::Imap,
            created_at: now,
            updated_at: now,
        }
    }

    fn make_folder(account_id: &str, remote_id: &str, role: FolderRole, sort_order: i32) -> Folder {
        Folder {
            id: new_id(),
            account_id: account_id.to_string(),
            remote_id: remote_id.to_string(),
            name: remote_id.to_string(),
            folder_type: FolderType::Folder,
            role: Some(role),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order,
        }
    }

    fn make_message(account_id: &str, remote_id: &str) -> Message {
        let now = now_timestamp();
        Message {
            id: new_id(),
            account_id: account_id.to_string(),
            remote_id: remote_id.to_string(),
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
    fn existing_remote_ids_are_scoped_by_folder_for_imap() {
        let store = Store::open_in_memory().unwrap();
        let account = make_account();
        store.insert_account(&account).unwrap();

        let inbox = make_folder(&account.id, "INBOX", FolderRole::Inbox, 0);
        let sent = make_folder(&account.id, "Sent", FolderRole::Sent, 1);
        store.insert_folder(&inbox).unwrap();
        store.insert_folder(&sent).unwrap();

        let msg = make_message(&account.id, "123");
        store
            .insert_message(&msg, std::slice::from_ref(&inbox.id))
            .unwrap();

        let remote_ids = vec!["123".to_string()];
        let inbox_matches = store
            .get_existing_remote_ids_in_folder(&account.id, &inbox.id, &remote_ids)
            .unwrap();
        let sent_matches = store
            .get_existing_remote_ids_in_folder(&account.id, &sent.id, &remote_ids)
            .unwrap();

        assert!(inbox_matches.contains("123"));
        assert!(!sent_matches.contains("123"));
    }

    #[test]
    fn cached_remote_ids_include_folderless_messages_and_are_account_scoped() {
        let store = Store::open_in_memory().unwrap();
        let mut account = make_account();
        account.provider = ProviderType::Gmail;
        let mut other_account = make_account();
        other_account.provider = ProviderType::Gmail;
        store.insert_account(&account).unwrap();
        store.insert_account(&other_account).unwrap();
        let inbox = make_folder(&account.id, "INBOX", FolderRole::Inbox, 0);
        store.insert_folder(&inbox).unwrap();

        let folderless = make_message(&account.id, "folderless");
        let in_folder = make_message(&account.id, "in-folder");
        let mut deleted = make_message(&account.id, "deleted");
        deleted.is_deleted = true;
        let empty_remote_id = make_message(&account.id, "");
        let other_account_message = make_message(&other_account.id, "other-account");

        store.insert_message(&folderless, &[]).unwrap();
        store
            .insert_message(&in_folder, std::slice::from_ref(&inbox.id))
            .unwrap();
        store.insert_message(&deleted, &[]).unwrap();
        store.insert_message(&empty_remote_id, &[]).unwrap();
        store.insert_message(&other_account_message, &[]).unwrap();

        let mut remote_ids = store
            .list_cached_remote_ids_by_account(&account.id)
            .unwrap();
        remote_ids.sort();
        assert_eq!(
            remote_ids,
            vec!["folderless".to_string(), "in-folder".to_string()]
        );
    }

    #[test]
    fn numeric_imap_uids_can_repeat_in_different_folders() {
        let store = Store::open_in_memory().unwrap();
        let account = make_account();
        let inbox = make_folder(&account.id, "INBOX", FolderRole::Inbox, 0);
        let sent = make_folder(&account.id, "Sent", FolderRole::Sent, 1);
        let inbox_message = make_message(&account.id, "123");
        let mut sent_message = make_message(&account.id, "123");
        sent_message.subject = "different mailbox message".to_string();

        store.insert_account(&account).unwrap();
        store.insert_folder(&inbox).unwrap();
        store.insert_folder(&sent).unwrap();
        store
            .insert_message(&inbox_message, std::slice::from_ref(&inbox.id))
            .unwrap();
        store
            .insert_message(&sent_message, std::slice::from_ref(&sent.id))
            .expect("the same IMAP UID in another mailbox must be a distinct message");

        let inbox_state = store
            .list_remote_ids_by_folder(&account.id, &inbox.id)
            .unwrap();
        let sent_state = store
            .list_remote_ids_by_folder(&account.id, &sent.id)
            .unwrap();
        assert_eq!(inbox_state.len(), 1);
        assert_eq!(sent_state.len(), 1);
        assert_ne!(inbox_state[0].0, sent_state[0].0);
    }

    #[test]
    fn non_imap_numeric_remote_ids_remain_account_wide_unique() {
        let store = Store::open_in_memory().unwrap();
        let mut account = make_account();
        account.provider = ProviderType::Gmail;
        let inbox = make_folder(&account.id, "INBOX", FolderRole::Inbox, 0);
        let sent = make_folder(&account.id, "Sent", FolderRole::Sent, 1);
        let inbox_message = make_message(&account.id, "123");
        let sent_message = make_message(&account.id, "123");

        store.insert_account(&account).unwrap();
        store.insert_folder(&inbox).unwrap();
        store.insert_folder(&sent).unwrap();
        store
            .insert_message(&inbox_message, std::slice::from_ref(&inbox.id))
            .unwrap();
        let duplicate = store.insert_message(&sent_message, std::slice::from_ref(&sent.id));

        assert!(duplicate.is_err());
    }

    #[test]
    fn imap_uid_is_rejected_when_repeated_in_the_same_folder() {
        let store = Store::open_in_memory().unwrap();
        let account = make_account();
        let inbox = make_folder(&account.id, "INBOX", FolderRole::Inbox, 0);
        let first = make_message(&account.id, "123");
        let second = make_message(&account.id, "123");

        store.insert_account(&account).unwrap();
        store.insert_folder(&inbox).unwrap();
        store
            .insert_message(&first, std::slice::from_ref(&inbox.id))
            .unwrap();
        let duplicate = store.insert_message(&second, std::slice::from_ref(&inbox.id));

        assert!(duplicate.is_err());
        assert!(store.get_message(&second.id).unwrap().is_none());
    }

    #[test]
    fn incomplete_attachment_rows_are_detected_for_forced_refetch() {
        let store = Store::open_in_memory().unwrap();
        let account = make_account();
        let inbox = make_folder(&account.id, "INBOX", FolderRole::Inbox, 0);
        store.insert_account(&account).unwrap();
        store.insert_folder(&inbox).unwrap();
        let mut message = make_message(&account.id, "42");
        message.has_attachments = true;
        store
            .insert_message(&message, std::slice::from_ref(&inbox.id))
            .unwrap();

        let remote_ids = vec![message.remote_id.clone()];
        let incomplete = store
            .get_incomplete_attachment_message_map_by_remote_ids(
                &account.id,
                Some(&inbox.id),
                &remote_ids,
            )
            .unwrap();
        assert_eq!(incomplete.get("42"), Some(&message.id));

        store
            .insert_attachment(&Attachment {
                id: new_id(),
                message_id: message.id.clone(),
                filename: "complete.bin".to_string(),
                mime_type: "application/octet-stream".to_string(),
                size: 1,
                local_path: None,
                content_id: None,
                is_inline: false,
            })
            .unwrap();
        assert!(store
            .get_incomplete_attachment_message_map_by_remote_ids(&account.id, None, &remote_ids,)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn update_remote_id_reports_missing_message() {
        let store = Store::open_in_memory().unwrap();

        let err = store
            .update_remote_id("missing-message", "new-remote-id")
            .expect_err("updating a missing message must fail");

        assert!(
            matches!(err, PebbleError::Internal(message) if message.contains("missing-message"))
        );
    }

    #[test]
    fn reconciling_imap_uidvalidity_preserves_only_unambiguous_stable_identity() {
        let store = Store::open_in_memory().unwrap();
        let account = make_account();
        let inbox = make_folder(&account.id, "INBOX", FolderRole::Inbox, 0);
        let sent = make_folder(&account.id, "Sent", FolderRole::Sent, 1);
        let mut stable = make_message(&account.id, "42");
        stable.message_id_header = Some("<stable@example.com>".to_string());
        stable.is_read = true;
        stable.is_starred = true;
        let mut different = make_message(&account.id, "50");
        different.message_id_header = Some("<old@example.com>".to_string());
        let mut shared = make_message(&account.id, "43");
        shared.message_id_header = Some("<shared@example.com>".to_string());
        let sent_only = make_message(&account.id, "44");

        store.insert_account(&account).unwrap();
        store.insert_folder(&inbox).unwrap();
        store.insert_folder(&sent).unwrap();
        store
            .insert_message(&stable, std::slice::from_ref(&inbox.id))
            .unwrap();
        store
            .insert_message(&different, std::slice::from_ref(&inbox.id))
            .unwrap();
        store
            .insert_message(&shared, &[inbox.id.clone(), sent.id.clone()])
            .unwrap();
        store
            .insert_message(&sent_only, std::slice::from_ref(&sent.id))
            .unwrap();
        store.add_label(&stable.id, "Keep").unwrap();
        store
            .upsert_kanban_card(&KanbanCard {
                message_id: stable.id.clone(),
                column: KanbanColumn::Waiting,
                position: 1,
                created_at: 1,
                updated_at: 1,
            })
            .unwrap();
        store
            .snooze_message(&SnoozedMessage {
                message_id: stable.id.clone(),
                snoozed_at: 1,
                unsnoozed_at: 2,
                return_to: "inbox".to_string(),
            })
            .unwrap();
        store.add_label(&different.id, "Must not inherit").unwrap();
        store
            .upsert_sync_failure(&account.id, &inbox.id, "42", "imap", "old generation")
            .unwrap();
        store
            .upsert_sync_failure(&account.id, &sent.id, "44", "imap", "other folder")
            .unwrap();
        store
            .insert_pending_mail_op(&account.id, &different.id, "mark_read", "{}")
            .unwrap();
        store
            .insert_pending_mail_op(&account.id, &shared.id, "mark_read", "{}")
            .unwrap();

        let mut fresh_stable = make_message(&account.id, "99");
        fresh_stable.message_id_header = Some("<stable@example.com>".to_string());
        fresh_stable.subject = "fresh stable".to_string();
        let fresh_stable_generated_id = fresh_stable.id.clone();
        let mut same_uid_different_identity = make_message(&account.id, "50");
        same_uid_different_identity.message_id_header = Some("<new@example.com>".to_string());
        let different_new_id = same_uid_different_identity.id.clone();

        let result = store
            .reconcile_imap_folder_uidvalidity(
                &account.id,
                &inbox.id,
                vec![
                    ImapFolderSnapshotMessage {
                        message: fresh_stable,
                        attachments: Vec::new(),
                    },
                    ImapFolderSnapshotMessage {
                        message: same_uid_different_identity,
                        attachments: Vec::new(),
                    },
                ],
            )
            .unwrap();

        assert!(result.unreferenced_attachment_paths.is_empty());
        assert!(store
            .get_message(&fresh_stable_generated_id)
            .unwrap()
            .is_none());
        let reconciled_stable = store.get_message(&stable.id).unwrap().unwrap();
        assert_eq!(reconciled_stable.remote_id, "99");
        assert_eq!(reconciled_stable.subject, "fresh stable");
        assert!(reconciled_stable.is_read);
        assert!(reconciled_stable.is_starred);
        assert_eq!(store.get_message_labels(&stable.id).unwrap().len(), 1);
        assert!(store
            .list_kanban_cards(None)
            .unwrap()
            .iter()
            .any(|card| card.message_id == stable.id));
        assert!(store.get_snoozed_message(&stable.id).unwrap().is_some());

        assert!(store.get_message(&different.id).unwrap().is_none());
        assert!(store.get_message(&different_new_id).unwrap().is_some());
        assert!(store
            .get_message_labels(&different_new_id)
            .unwrap()
            .is_empty());
        assert!(store.get_message(&shared.id).unwrap().is_some());
        assert_eq!(
            store.get_message_folder_ids(&shared.id).unwrap(),
            vec![sent.id.clone()]
        );
        assert!(store.get_message(&sent_only.id).unwrap().is_some());
        assert!(!store
            .has_sync_failures_for_folder(&account.id, &inbox.id)
            .unwrap());
        assert!(store
            .has_sync_failures_for_folder(&account.id, &sent.id)
            .unwrap());
        let pending_ops = store.list_pending_mail_ops(&account.id).unwrap();
        assert_eq!(pending_ops.len(), 1);
        assert_eq!(pending_ops[0].message_id, shared.id);

        let pending: HashMap<String, String> =
            store.list_search_pending().unwrap().into_iter().collect();
        assert_eq!(pending.get(&stable.id).map(String::as_str), Some("index"));
        assert_eq!(
            pending.get(&different.id).map(String::as_str),
            Some("remove")
        );
        assert_eq!(
            pending.get(&different_new_id).map(String::as_str),
            Some("index")
        );
        assert_eq!(pending.get(&shared.id).map(String::as_str), Some("index"));
        assert!(result
            .removed_messages
            .iter()
            .any(|message| message.id == different.id));
    }

    #[test]
    fn failed_imap_uidvalidity_reconcile_keeps_the_old_generation_intact() {
        let store = Store::open_in_memory().unwrap();
        let account = make_account();
        let inbox = make_folder(&account.id, "INBOX", FolderRole::Inbox, 0);
        let sent = make_folder(&account.id, "Sent", FolderRole::Sent, 1);
        store.insert_account(&account).unwrap();
        store.insert_folder(&inbox).unwrap();
        store.insert_folder(&sent).unwrap();

        let mut old = make_message(&account.id, "42");
        old.message_id_header = Some("<stable@example.com>".to_string());
        old.subject = "old generation".to_string();
        store
            .insert_message(&old, std::slice::from_ref(&inbox.id))
            .unwrap();
        store.add_label(&old.id, "Keep").unwrap();
        store
            .upsert_sync_failure(&account.id, &inbox.id, "42", "imap", "keep until commit")
            .unwrap();

        let blocker = make_message(&account.id, "77");
        let duplicate_attachment_id = new_id();
        store
            .replace_message_with_attachments(
                &blocker,
                std::slice::from_ref(&sent.id),
                &[Attachment {
                    id: duplicate_attachment_id.clone(),
                    message_id: blocker.id.clone(),
                    filename: "blocker.bin".to_string(),
                    mime_type: "application/octet-stream".to_string(),
                    size: 1,
                    local_path: None,
                    content_id: None,
                    is_inline: false,
                }],
            )
            .unwrap();

        let mut fresh = make_message(&account.id, "99");
        fresh.message_id_header = old.message_id_header.clone();
        fresh.subject = "fresh generation".to_string();
        let result = store.reconcile_imap_folder_uidvalidity(
            &account.id,
            &inbox.id,
            vec![ImapFolderSnapshotMessage {
                message: fresh,
                attachments: vec![Attachment {
                    id: duplicate_attachment_id,
                    message_id: "temporary-id".to_string(),
                    filename: "new.bin".to_string(),
                    mime_type: "application/octet-stream".to_string(),
                    size: 2,
                    local_path: None,
                    content_id: None,
                    is_inline: false,
                }],
            }],
        );
        assert!(result.is_err());

        let retained = store.get_message(&old.id).unwrap().unwrap();
        assert_eq!(retained.remote_id, "42");
        assert_eq!(retained.subject, "old generation");
        assert_eq!(
            store.get_message_folder_ids(&old.id).unwrap(),
            vec![inbox.id.clone()]
        );
        assert_eq!(store.get_message_labels(&old.id).unwrap().len(), 1);
        assert!(store
            .has_sync_failures_for_folder(&account.id, &inbox.id)
            .unwrap());
        assert!(store.list_search_pending().unwrap().is_empty());
    }

    #[test]
    fn imap_uidvalidity_reconcile_allows_stable_messages_to_swap_uids() {
        let store = Store::open_in_memory().unwrap();
        let account = make_account();
        let inbox = make_folder(&account.id, "INBOX", FolderRole::Inbox, 0);
        store.insert_account(&account).unwrap();
        store.insert_folder(&inbox).unwrap();

        let mut old_a = make_message(&account.id, "1");
        old_a.message_id_header = Some("<a@example.com>".to_string());
        let mut old_b = make_message(&account.id, "2");
        old_b.message_id_header = Some("<b@example.com>".to_string());
        store
            .insert_message(&old_a, std::slice::from_ref(&inbox.id))
            .unwrap();
        store
            .insert_message(&old_b, std::slice::from_ref(&inbox.id))
            .unwrap();

        let mut fresh_a = make_message(&account.id, "2");
        fresh_a.message_id_header = old_a.message_id_header.clone();
        let mut fresh_b = make_message(&account.id, "1");
        fresh_b.message_id_header = old_b.message_id_header.clone();
        store
            .reconcile_imap_folder_uidvalidity(
                &account.id,
                &inbox.id,
                vec![
                    ImapFolderSnapshotMessage {
                        message: fresh_a,
                        attachments: Vec::new(),
                    },
                    ImapFolderSnapshotMessage {
                        message: fresh_b,
                        attachments: Vec::new(),
                    },
                ],
            )
            .unwrap();

        assert_eq!(
            store.get_message(&old_a.id).unwrap().unwrap().remote_id,
            "2"
        );
        assert_eq!(
            store.get_message(&old_b.id).unwrap().unwrap().remote_id,
            "1"
        );
        assert_eq!(
            store.get_message_folder_ids(&old_a.id).unwrap(),
            vec![inbox.id.clone()]
        );
        assert_eq!(
            store.get_message_folder_ids(&old_b.id).unwrap(),
            vec![inbox.id]
        );
    }

    #[test]
    fn replace_message_updates_non_imap_row_without_triggering_self_duplicate() {
        let store = Store::open_in_memory().unwrap();
        let mut account = make_account();
        account.provider = ProviderType::Outlook;
        account.email = "outlook@example.com".to_string();
        store.insert_account(&account).unwrap();
        let inbox = make_folder(&account.id, "Inbox", FolderRole::Inbox, 0);
        store.insert_folder(&inbox).unwrap();

        let message = make_message(&account.id, "graph-message-id");
        store
            .replace_message_with_attachments(&message, std::slice::from_ref(&inbox.id), &[])
            .unwrap();

        let mut updated = message.clone();
        updated.subject = "Updated by delta sync".to_string();
        store
            .replace_message_with_attachments(&updated, std::slice::from_ref(&inbox.id), &[])
            .unwrap();

        assert_eq!(
            store.get_message(&message.id).unwrap().unwrap().subject,
            "Updated by delta sync"
        );
    }

    #[test]
    fn replace_message_with_attachments_replaces_old_attachment_set() {
        let store = Store::open_in_memory().unwrap();
        let account = make_account();
        store.insert_account(&account).unwrap();

        let drafts = make_folder(&account.id, "Drafts", FolderRole::Drafts, 0);
        let archive = make_folder(&account.id, "Archive", FolderRole::Archive, 1);
        store.insert_folder(&drafts).unwrap();
        store.insert_folder(&archive).unwrap();

        let mut msg = make_message(&account.id, "draft-1");
        msg.is_draft = true;
        msg.has_attachments = true;

        let old_attachment = Attachment {
            id: new_id(),
            message_id: msg.id.clone(),
            filename: "old.pdf".to_string(),
            mime_type: "application/pdf".to_string(),
            size: 10,
            local_path: Some("C:\\tmp\\old.pdf".to_string()),
            content_id: None,
            is_inline: false,
        };
        store
            .replace_message_with_attachments(
                &msg,
                std::slice::from_ref(&drafts.id),
                &[old_attachment],
            )
            .unwrap();
        store.add_label(&msg.id, "Important").unwrap();
        store
            .upsert_kanban_card(&KanbanCard {
                message_id: msg.id.clone(),
                column: KanbanColumn::Waiting,
                position: 7,
                created_at: 1,
                updated_at: 2,
            })
            .unwrap();
        store
            .snooze_message(&SnoozedMessage {
                message_id: msg.id.clone(),
                snoozed_at: 3,
                unsnoozed_at: 4,
                return_to: "archive".to_string(),
            })
            .unwrap();

        let mut updated = msg.clone();
        updated.subject = "Updated draft".to_string();
        let new_attachment = Attachment {
            id: new_id(),
            message_id: updated.id.clone(),
            filename: "new.pdf".to_string(),
            mime_type: "application/pdf".to_string(),
            size: 20,
            local_path: Some("C:\\tmp\\new.pdf".to_string()),
            content_id: None,
            is_inline: false,
        };
        store
            .replace_message_with_attachments(
                &updated,
                std::slice::from_ref(&archive.id),
                &[new_attachment],
            )
            .unwrap();

        let fetched = store.get_message(&updated.id).unwrap().unwrap();
        assert_eq!(fetched.subject, "Updated draft");
        assert!(fetched.has_attachments);

        let attachments = store.list_attachments_by_message(&updated.id).unwrap();
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].filename, "new.pdf");
        assert_eq!(
            store.get_message_folder_ids(&updated.id).unwrap(),
            vec![archive.id]
        );
        assert_eq!(store.get_message_labels(&updated.id).unwrap().len(), 1);
        assert_eq!(store.list_kanban_cards(None).unwrap().len(), 1);
        assert!(store.get_snoozed_message(&updated.id).unwrap().is_some());
    }

    #[test]
    fn replace_message_with_attachments_rolls_back_all_state_on_insert_failure() {
        let store = Store::open_in_memory().unwrap();
        let account = make_account();
        store.insert_account(&account).unwrap();
        let drafts = make_folder(&account.id, "Drafts", FolderRole::Drafts, 0);
        let archive = make_folder(&account.id, "Archive", FolderRole::Archive, 1);
        store.insert_folder(&drafts).unwrap();
        store.insert_folder(&archive).unwrap();

        let mut original = make_message(&account.id, "draft-original");
        original.subject = "Original".to_string();
        let old_attachment = Attachment {
            id: new_id(),
            message_id: original.id.clone(),
            filename: "old.pdf".to_string(),
            mime_type: "application/pdf".to_string(),
            size: 10,
            local_path: Some("C:\\tmp\\old.pdf".to_string()),
            content_id: None,
            is_inline: false,
        };
        store
            .replace_message_with_attachments(
                &original,
                std::slice::from_ref(&drafts.id),
                std::slice::from_ref(&old_attachment),
            )
            .unwrap();
        store.add_label(&original.id, "Keep").unwrap();
        store
            .upsert_kanban_card(&KanbanCard {
                message_id: original.id.clone(),
                column: KanbanColumn::Todo,
                position: 1,
                created_at: 1,
                updated_at: 1,
            })
            .unwrap();
        store
            .snooze_message(&SnoozedMessage {
                message_id: original.id.clone(),
                snoozed_at: 1,
                unsnoozed_at: 2,
                return_to: "inbox".to_string(),
            })
            .unwrap();

        let blocker = make_message(&account.id, "blocker");
        let duplicate_id = new_id();
        store
            .replace_message_with_attachments(
                &blocker,
                std::slice::from_ref(&archive.id),
                &[Attachment {
                    id: duplicate_id.clone(),
                    message_id: blocker.id.clone(),
                    filename: "blocker.pdf".to_string(),
                    mime_type: "application/pdf".to_string(),
                    size: 1,
                    local_path: None,
                    content_id: None,
                    is_inline: false,
                }],
            )
            .unwrap();

        let mut attempted = original.clone();
        attempted.subject = "Must roll back".to_string();
        let failure = store.replace_message_with_attachments(
            &attempted,
            std::slice::from_ref(&archive.id),
            &[Attachment {
                id: duplicate_id,
                message_id: attempted.id.clone(),
                filename: "new.pdf".to_string(),
                mime_type: "application/pdf".to_string(),
                size: 20,
                local_path: None,
                content_id: None,
                is_inline: false,
            }],
        );
        assert!(failure.is_err());

        assert_eq!(
            store.get_message(&original.id).unwrap().unwrap().subject,
            "Original"
        );
        assert_eq!(
            store.get_message_folder_ids(&original.id).unwrap(),
            vec![drafts.id]
        );
        let attachments = store.list_attachments_by_message(&original.id).unwrap();
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].id, old_attachment.id);
        assert_eq!(store.get_message_labels(&original.id).unwrap().len(), 1);
        assert_eq!(store.list_kanban_cards(None).unwrap().len(), 1);
        assert!(store.get_snoozed_message(&original.id).unwrap().is_some());
    }

    #[test]
    fn prepared_send_rolls_back_message_and_attachments_when_op_insert_fails() {
        let store = Store::open_in_memory().unwrap();
        let account = make_account();
        store.insert_account(&account).unwrap();
        let outbox = make_folder(&account.id, "Outbox", FolderRole::Inbox, 0);
        store.insert_folder(&outbox).unwrap();
        let mut message = make_message(&account.id, "local-outbox-rollback");
        message.has_attachments = true;
        let attachment = Attachment {
            id: new_id(),
            message_id: message.id.clone(),
            filename: "evidence.txt".to_string(),
            mime_type: "text/plain".to_string(),
            size: 8,
            local_path: Some("C:\\tmp\\evidence.txt".to_string()),
            content_id: None,
            is_inline: false,
        };
        store
            .with_write(|conn| {
                conn.execute_batch(
                    "CREATE TRIGGER fail_prepared_send_op
                     BEFORE INSERT ON pending_mail_ops
                     WHEN NEW.op_type = 'send'
                     BEGIN
                         SELECT RAISE(ABORT, 'injected pending-op failure');
                     END;",
                )?;
                Ok(())
            })
            .unwrap();

        let result = store.prepare_outgoing_send(
            &message,
            std::slice::from_ref(&outbox.id),
            std::slice::from_ref(&attachment),
            r#"{"op":"send","payload":{}}"#,
        );

        assert!(result.is_err());
        assert!(store.get_message(&message.id).unwrap().is_none());
        assert!(store
            .list_attachments_by_message(&message.id)
            .unwrap()
            .is_empty());
        assert!(store.list_pending_mail_ops(&account.id).unwrap().is_empty());
    }
}

#[cfg(test)]
mod tombstone_tests {
    use crate::Store;
    use pebble_core::*;

    fn setup_store_with_message(is_deleted: bool, deleted_at: Option<i64>) -> (Store, String) {
        let store = Store::open_in_memory().unwrap();
        let now = now_timestamp();
        let account = Account {
            account_label: None,
            provider_display_name: None,
            id: new_id(),
            email: "test@example.com".to_string(),
            display_name: "Test".to_string(),
            color: None,
            provider: ProviderType::Imap,
            created_at: now,
            updated_at: now,
        };
        store.insert_account(&account).unwrap();
        let folder = Folder {
            id: new_id(),
            account_id: account.id.clone(),
            remote_id: "INBOX".to_string(),
            name: "Inbox".to_string(),
            folder_type: FolderType::Folder,
            role: Some(FolderRole::Inbox),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 0,
        };
        store.insert_folder(&folder).unwrap();
        let msg = Message {
            id: new_id(),
            account_id: account.id.clone(),
            remote_id: "999".to_string(),
            message_id_header: None,
            in_reply_to: None,
            references_header: None,
            thread_id: None,
            subject: "Test".to_string(),
            snippet: "test".to_string(),
            from_address: "a@b.com".to_string(),
            from_name: "A".to_string(),
            to_list: vec![],
            cc_list: vec![],
            bcc_list: vec![],
            body_text: "body".to_string(),
            body_html_raw: "<p>body</p>".to_string(),
            has_attachments: false,
            is_read: false,
            is_starred: false,
            is_draft: false,
            date: now,
            remote_version: None,
            is_deleted,
            deleted_at,
            created_at: now,
            updated_at: now,
        };
        store
            .insert_message(&msg, std::slice::from_ref(&folder.id))
            .unwrap();
        (store, msg.id)
    }

    #[test]
    fn test_purge_old_tombstone() {
        let thirty_one_days_ago = pebble_core::now_timestamp() - (31 * 24 * 3600);
        let (store, msg_id) = setup_store_with_message(true, Some(thirty_one_days_ago));
        let purged = store.purge_old_tombstones(30 * 24 * 3600).unwrap();
        assert_eq!(purged, 1);
        // Verify message is physically gone
        let fetched = store.get_message(&msg_id).unwrap();
        assert!(fetched.is_none());
    }

    #[test]
    fn test_recent_tombstone_not_purged() {
        let one_day_ago = pebble_core::now_timestamp() - (24 * 3600);
        let (store, msg_id) = setup_store_with_message(true, Some(one_day_ago));
        let purged = store.purge_old_tombstones(30 * 24 * 3600).unwrap();
        assert_eq!(purged, 0);
        let fetched = store.get_message(&msg_id).unwrap();
        assert!(fetched.is_some());
    }

    #[test]
    fn test_non_deleted_message_not_purged() {
        let (store, msg_id) = setup_store_with_message(false, None);
        let purged = store.purge_old_tombstones(30 * 24 * 3600).unwrap();
        assert_eq!(purged, 0);
        let fetched = store.get_message(&msg_id).unwrap();
        assert!(fetched.is_some());
    }
}

#[cfg(test)]
mod thread_listing_tests {
    use crate::Store;
    use pebble_core::*;

    fn seed_account_and_folder(store: &Store) -> (String, String) {
        let now = now_timestamp();
        let account = Account {
            account_label: None,
            provider_display_name: None,
            id: new_id(),
            email: "test@example.com".to_string(),
            display_name: "Test".to_string(),
            color: None,
            provider: ProviderType::Imap,
            created_at: now,
            updated_at: now,
        };
        store.insert_account(&account).unwrap();
        let folder = Folder {
            id: new_id(),
            account_id: account.id.clone(),
            remote_id: "INBOX".to_string(),
            name: "Inbox".to_string(),
            folder_type: FolderType::Folder,
            role: Some(FolderRole::Inbox),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 0,
        };
        store.insert_folder(&folder).unwrap();
        (account.id, folder.id)
    }

    fn make_msg(account_id: &str, thread_id: &str, from: &str, date: i64) -> Message {
        Message {
            id: new_id(),
            account_id: account_id.to_string(),
            remote_id: new_id(),
            message_id_header: None,
            in_reply_to: None,
            references_header: None,
            thread_id: Some(thread_id.to_string()),
            subject: "Thread subject".to_string(),
            snippet: format!("snippet-{date}"),
            from_address: from.to_string(),
            from_name: String::new(),
            to_list: vec![],
            cc_list: vec![],
            bcc_list: vec![],
            body_text: "body".to_string(),
            body_html_raw: String::new(),
            has_attachments: false,
            is_read: false,
            is_starred: false,
            is_draft: false,
            date,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: date,
            updated_at: date,
        }
    }

    #[test]
    fn list_threads_aggregates_distinct_participants() {
        let store = Store::open_in_memory().unwrap();
        let (account_id, folder_id) = seed_account_and_folder(&store);
        let thread_id = new_id();

        let base = now_timestamp();
        // Three messages in same thread, two distinct senders (alice appears twice).
        let m1 = make_msg(&account_id, &thread_id, "alice@example.com", base - 200);
        let m2 = make_msg(&account_id, &thread_id, "bob@example.com", base - 100);
        let m3 = make_msg(&account_id, &thread_id, "alice@example.com", base);
        store
            .insert_message(&m1, std::slice::from_ref(&folder_id))
            .unwrap();
        store
            .insert_message(&m2, std::slice::from_ref(&folder_id))
            .unwrap();
        store
            .insert_message(&m3, std::slice::from_ref(&folder_id))
            .unwrap();

        let threads = store
            .list_threads_by_folder(&folder_id, 50, 0)
            .expect("list_threads_by_folder should succeed with distinct participants");
        assert_eq!(threads.len(), 1);
        let t = &threads[0];
        assert_eq!(t.message_count, 3);
        assert_eq!(t.unread_count, 3);
        let mut parts = t.participants.clone();
        parts.sort();
        assert_eq!(
            parts,
            vec![
                "alice@example.com".to_string(),
                "bob@example.com".to_string()
            ]
        );
    }

    #[test]
    fn list_threads_by_folders_counts_messages_once_across_selected_folders() {
        let store = Store::open_in_memory().unwrap();
        let (account_id, inbox_id) = seed_account_and_folder(&store);
        let archive = Folder {
            id: new_id(),
            account_id: account_id.clone(),
            remote_id: "Archive".to_string(),
            name: "Archive".to_string(),
            folder_type: FolderType::Folder,
            role: Some(FolderRole::Archive),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 1,
        };
        store.insert_folder(&archive).unwrap();

        let thread_id = new_id();
        let base = now_timestamp();
        let m1 = make_msg(&account_id, &thread_id, "alice@example.com", base - 60);
        let m2 = make_msg(&account_id, &thread_id, "bob@example.com", base);
        let m1_folder_ids = [inbox_id.clone(), archive.id.clone()];
        store.insert_message(&m1, &m1_folder_ids).unwrap();
        store
            .insert_message(&m2, std::slice::from_ref(&archive.id))
            .unwrap();

        let threads = store
            .list_threads_by_folders(&[inbox_id, archive.id], 50, 0)
            .expect("list_threads_by_folders should aggregate selected folders");

        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].message_count, 2);
        assert_eq!(threads[0].snippet, format!("snippet-{base}"));
    }
}
