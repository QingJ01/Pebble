use pebble_core::{Message, MessageSummary, PebbleError, Result};
use rusqlite::{params, OptionalExtension, Row};
use std::collections::HashMap;

use crate::Store;

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
    ) -> Result<Vec<(String, String, bool, bool, i64)>> {
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

    /// Get all (message_id_header → thread_id) mappings for an account.
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
    use crate::Store;
    use pebble_core::*;

    fn make_account() -> Account {
        let now = now_timestamp();
        Account {
            id: new_id(),
            email: "imap@example.com".to_string(),
            display_name: "IMAP".to_string(),
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
        store.insert_message(&msg, &[inbox.id.clone()]).unwrap();

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
}

#[cfg(test)]
mod tombstone_tests {
    use crate::Store;
    use pebble_core::*;

    fn setup_store_with_message(is_deleted: bool, deleted_at: Option<i64>) -> (Store, String) {
        let store = Store::open_in_memory().unwrap();
        let now = now_timestamp();
        let account = Account {
            id: new_id(),
            email: "test@example.com".to_string(),
            display_name: "Test".to_string(),
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
        store.insert_message(&msg, &[folder.id.clone()]).unwrap();
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
        let one_day_ago = pebble_core::now_timestamp() - (1 * 24 * 3600);
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
            id: new_id(),
            email: "test@example.com".to_string(),
            display_name: "Test".to_string(),
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
        store.insert_message(&m1, &[folder_id.clone()]).unwrap();
        store.insert_message(&m2, &[folder_id.clone()]).unwrap();
        store.insert_message(&m3, &[folder_id.clone()]).unwrap();

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
}
