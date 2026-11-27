use pebble_core::{Attachment, Message, MessageSummary, PebbleError, Result};
use rusqlite::{params, OptionalExtension, Row};
use std::collections::{HashMap, HashSet};

use crate::pending_ops::PendingMailOpStatus;
use crate::{Store, SQL_IN_CHUNK_SIZE};

mod mutate;
mod query;
mod remote;

#[cfg(test)]
mod list_perf_tests;
#[cfg(test)]
mod remote_id_scope_tests;
#[cfg(test)]
mod sql_batching_tests;
#[cfg(test)]
mod thread_listing_tests;
#[cfg(test)]
mod tombstone_tests;

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
    row_to_message_at(row, 0)
}

/// [`row_to_message`] for a SELECT whose message columns start at `base`.
fn row_to_message_at(row: &Row, base: usize) -> rusqlite::Result<Message> {
    let to_json: String = row.get(base + 11)?;
    let cc_json: String = row.get(base + 12)?;
    let bcc_json: String = row.get(base + 13)?;
    let has_attachments: i32 = row.get(base + 16)?;
    let is_read: i32 = row.get(base + 17)?;
    let is_starred: i32 = row.get(base + 18)?;
    let is_draft: i32 = row.get(base + 19)?;
    let is_deleted: i32 = row.get(base + 22)?;

    Ok(Message {
        id: row.get(base)?,
        account_id: row.get(base + 1)?,
        remote_id: row.get(base + 2)?,
        message_id_header: row.get(base + 3)?,
        in_reply_to: row.get(base + 4)?,
        references_header: row.get(base + 5)?,
        thread_id: row.get(base + 6)?,
        subject: row.get(base + 7)?,
        snippet: row.get(base + 8)?,
        from_address: row.get(base + 9)?,
        from_name: row.get(base + 10)?,
        to_list: serde_json::from_str(&to_json).unwrap_or_default(),
        cc_list: serde_json::from_str(&cc_json).unwrap_or_default(),
        bcc_list: serde_json::from_str(&bcc_json).unwrap_or_default(),
        body_text: row.get(base + 14)?,
        body_html_raw: row.get(base + 15)?,
        has_attachments: has_attachments != 0,
        is_read: is_read != 0,
        is_starred: is_starred != 0,
        is_draft: is_draft != 0,
        date: row.get(base + 20)?,
        remote_version: row.get(base + 21)?,
        is_deleted: is_deleted != 0,
        deleted_at: row.get(base + 23)?,
        created_at: row.get(base + 24)?,
        updated_at: row.get(base + 25)?,
    })
}

/// Full message column list. Bodies live in `message_bodies` (V21) so that
/// list/summary reads never touch body overflow pages; always pair this with
/// [`MSG_FROM`].
const MSG_SELECT: &str = "m.id, m.account_id, m.remote_id, m.message_id_header, m.in_reply_to, \
     m.references_header, m.thread_id, m.subject, m.snippet, m.from_address, \
     m.from_name, m.to_list, m.cc_list, m.bcc_list, \
     COALESCE(b.body_text, ''), COALESCE(b.body_html_raw, ''), \
     m.has_attachments, m.is_read, m.is_starred, m.is_draft, \
     m.date, m.remote_version, m.is_deleted, m.deleted_at, m.created_at, m.updated_at";

/// FROM clause matching [`MSG_SELECT`].
const MSG_FROM: &str = "messages m LEFT JOIN message_bodies b ON b.message_id = m.id";

/// Column list for list queries (excludes body_text and body_html_raw).
/// Expects the `messages` table to be aliased as `m`.
const MSG_SUMMARY_SELECT: &str =
    "m.id, m.account_id, m.remote_id, m.message_id_header, m.in_reply_to, \
     m.references_header, m.thread_id, m.subject, m.snippet, m.from_address, \
     m.from_name, m.to_list, m.cc_list, m.bcc_list, \
     m.has_attachments, m.is_read, m.is_starred, m.is_draft, \
     m.date, m.remote_version, m.is_deleted, m.deleted_at, m.created_at, m.updated_at";

/// Keyset cursor for folder message pages: the `(date, id)` of the last
/// message already shown. The next page contains strictly older messages.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct MessageCursor {
    pub date: i64,
    pub id: String,
}

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
           has_attachments = ?15,
           is_read = ?16,
           is_starred = ?17,
           is_draft = ?18,
           date = ?19,
           remote_version = ?20,
           is_deleted = ?21,
           deleted_at = ?22,
           created_at = ?23,
           updated_at = ?24
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
    if updated == 0 {
        insert_message_row(conn, msg, &to_json, &cc_json, &bcc_json)?;
    } else {
        write_message_body(conn, msg)?;
        record_correspondents(conn, msg)?;
    }
    Ok(())
}

/// INSERT a brand-new message row plus its body and autocomplete entries.
fn insert_message_row(
    conn: &rusqlite::Connection,
    msg: &Message,
    to_json: &str,
    cc_json: &str,
    bcc_json: &str,
) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO messages (id, account_id, remote_id, message_id_header, in_reply_to,
         references_header, thread_id, subject, snippet, from_address, from_name,
         to_list, cc_list, bcc_list,
         has_attachments, is_read, is_starred, is_draft,
         date, remote_version, is_deleted, deleted_at, created_at, updated_at)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14, ?15, ?16, ?17, ?18,
                 ?19, ?20, ?21, ?22, ?23, ?24)",
    )?
    .execute(params![
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
    ])?;
    write_message_body(conn, msg)?;
    record_correspondents(conn, msg)?;
    Ok(())
}

fn write_message_body(conn: &rusqlite::Connection, msg: &Message) -> Result<()> {
    conn.prepare_cached(
        "INSERT INTO message_bodies (message_id, body_text, body_html_raw) VALUES (?1, ?2, ?3)
         ON CONFLICT(message_id) DO UPDATE SET
             body_text = excluded.body_text,
             body_html_raw = excluded.body_html_raw",
    )?
    .execute(params![msg.id, msg.body_text, msg.body_html_raw])?;
    Ok(())
}

/// Maintain `message_correspondents` (autocomplete source) for a live message.
/// Mirrors `contacts::add_recent_candidate`: the most recent interaction wins
/// the display address/name; an older interaction only fills a missing name.
fn record_correspondents(conn: &rusqlite::Connection, msg: &Message) -> Result<()> {
    if msg.is_deleted {
        return Ok(());
    }
    let mut stmt = conn.prepare_cached(
        "INSERT INTO message_correspondents
             (account_id, normalized_address, address, name, last_interaction_at)
         VALUES (?1, ?2, ?3, ?4, ?5)
         ON CONFLICT(account_id, normalized_address) DO UPDATE SET
             address = CASE WHEN excluded.last_interaction_at > last_interaction_at
                            THEN excluded.address ELSE address END,
             name = CASE WHEN excluded.last_interaction_at > last_interaction_at
                         THEN excluded.name
                         WHEN name IS NULL THEN excluded.name
                         ELSE name END,
             last_interaction_at = MAX(last_interaction_at, excluded.last_interaction_at)",
    )?;
    let mut upsert = |name: Option<&str>, address: &str| -> Result<()> {
        let address = address.trim();
        if !address.contains('@') {
            return Ok(());
        }
        let name = name.map(str::trim).filter(|value| !value.is_empty());
        stmt.execute(params![
            msg.account_id,
            address.to_lowercase(),
            address,
            name,
            msg.date
        ])?;
        Ok(())
    };
    upsert(Some(msg.from_name.as_str()), &msg.from_address)?;
    for address in msg.to_list.iter().chain(&msg.cc_list).chain(&msg.bcc_list) {
        upsert(address.name.as_deref(), &address.address)?;
    }
    Ok(())
}

/// Append `items` as `?N` placeholders starting at `start`.
fn placeholders(start: usize, count: usize) -> String {
    (start..start + count)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

/// SQL for one folder's newest-first page of message ids, read straight off
/// `idx_mf_folder_date` (partial on `is_deleted = 0`).
const FOLDER_PAGE_SQL: &str = "SELECT message_id, date FROM message_folders
     WHERE folder_id = ?1 AND is_deleted = 0
     ORDER BY date DESC, message_id DESC
     LIMIT ?2 OFFSET ?3";
const FOLDER_PAGE_AFTER_SQL: &str = "SELECT message_id, date FROM message_folders
     WHERE folder_id = ?1 AND is_deleted = 0 AND (date, message_id) < (?4, ?5)
     ORDER BY date DESC, message_id DESC
     LIMIT ?2 OFFSET ?3";
/// Unread-only variants: same index walk, skipping read rows.
const FOLDER_UNREAD_PAGE_SQL: &str = "SELECT message_id, date FROM message_folders
     WHERE folder_id = ?1 AND is_deleted = 0 AND is_read = 0
     ORDER BY date DESC, message_id DESC
     LIMIT ?2 OFFSET ?3";
const FOLDER_UNREAD_PAGE_AFTER_SQL: &str = "SELECT message_id, date FROM message_folders
     WHERE folder_id = ?1 AND is_deleted = 0 AND is_read = 0 AND (date, message_id) < (?4, ?5)
     ORDER BY date DESC, message_id DESC
     LIMIT ?2 OFFSET ?3";

fn folder_page(
    conn: &rusqlite::Connection,
    folder_id: &str,
    cursor: Option<&MessageCursor>,
    limit: u32,
    offset: u32,
    unread_only: bool,
) -> Result<Vec<(String, i64)>> {
    let map = |row: &Row| Ok((row.get::<_, String>(0)?, row.get::<_, i64>(1)?));
    let (page_sql, after_sql) = if unread_only {
        (FOLDER_UNREAD_PAGE_SQL, FOLDER_UNREAD_PAGE_AFTER_SQL)
    } else {
        (FOLDER_PAGE_SQL, FOLDER_PAGE_AFTER_SQL)
    };
    let rows = match cursor {
        Some(cursor) => conn
            .prepare_cached(after_sql)?
            .query_map(
                params![folder_id, limit, offset, cursor.date, cursor.id],
                map,
            )?
            .collect::<rusqlite::Result<Vec<_>>>()?,
        None => conn
            .prepare_cached(page_sql)?
            .query_map(params![folder_id, limit, offset], map)?
            .collect::<rusqlite::Result<Vec<_>>>()?,
    };
    Ok(rows)
}

/// Resolve a page of message ids (newest first) across `folder_ids`.
/// Each folder contributes at most `offset + limit` rows from its index; the
/// small union is merged in memory, deduplicating multi-folder messages.
fn page_message_ids(
    conn: &rusqlite::Connection,
    folder_ids: &[String],
    cursor: Option<&MessageCursor>,
    limit: u32,
    offset: u32,
    unread_only: bool,
) -> Result<Vec<String>> {
    if folder_ids.len() == 1 {
        return Ok(
            folder_page(conn, &folder_ids[0], cursor, limit, offset, unread_only)?
                .into_iter()
                .map(|(id, _)| id)
                .collect(),
        );
    }
    let window = limit.saturating_add(offset);
    let mut merged: HashMap<String, i64> = HashMap::new();
    let mut seen_folders = HashSet::new();
    for folder_id in folder_ids {
        if !seen_folders.insert(folder_id.as_str()) {
            continue;
        }
        for (id, date) in folder_page(conn, folder_id, cursor, window, 0, unread_only)? {
            merged.insert(id, date);
        }
    }
    let mut rows: Vec<(String, i64)> = merged.into_iter().collect();
    rows.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
    Ok(rows
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .map(|(id, _)| id)
        .collect())
}

/// Fetch summaries for `ids` (primary-key lookups only) in the given order.
fn fetch_summaries_ordered(
    conn: &rusqlite::Connection,
    ids: &[String],
) -> Result<Vec<MessageSummary>> {
    if ids.is_empty() {
        return Ok(Vec::new());
    }
    let mut by_id = HashMap::with_capacity(ids.len());
    for chunk in ids.chunks(500) {
        let sql = format!(
            "SELECT {MSG_SUMMARY_SELECT} FROM messages m WHERE m.id IN ({})",
            placeholders(1, chunk.len())
        );
        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt.query_map(
            rusqlite::params_from_iter(chunk.iter()),
            row_to_message_summary,
        )?;
        for row in rows {
            let summary = row?;
            by_id.insert(summary.id.clone(), summary);
        }
    }
    Ok(ids.iter().filter_map(|id| by_id.remove(id)).collect())
}

/// Walk one folder newest-first and collect the first `want` distinct
/// `(account_id, thread key)` pairs with their most recent date in that folder. The walk is a covering
/// scan of `idx_mf_folder_date` that stops as soon as enough threads are
/// found, so page cost is proportional to the page, not the folder.
/// With `unread_only`, see [`folder_unread_thread_keys`].
fn folder_thread_keys(
    conn: &rusqlite::Connection,
    folder_id: &str,
    want: usize,
    unread_only: bool,
) -> Result<Vec<((String, String), i64)>> {
    if unread_only {
        return folder_unread_thread_keys(conn, folder_id, want);
    }
    let mut stmt = conn.prepare_cached(
        "SELECT m.account_id, mf.thread_key, mf.date
         FROM message_folders mf
         JOIN messages m ON m.id = mf.message_id
         WHERE mf.folder_id = ?1 AND mf.is_deleted = 0
         ORDER BY mf.date DESC, mf.message_id DESC",
    )?;
    let mut rows = stmt.query(params![folder_id])?;
    let mut seen = HashSet::new();
    let mut keys = Vec::new();
    while keys.len() < want {
        let Some(row) = rows.next()? else { break };
        let key: (String, String) = (row.get(0)?, row.get(1)?);
        let date: i64 = row.get(2)?;
        if seen.insert(key.clone()) {
            keys.push((key, date));
        }
    }
    Ok(keys)
}

/// Threads with an unread message in this folder, ranked like the unfiltered
/// list: by the thread's newest message in the folder (the date its row
/// shows), not by its newest unread one. Reads only the unread rows
/// (`idx_mf_folder_unread_date`) plus one indexed MAX per thread
/// (`idx_mf_folder_thread`), so cost follows the unread count.
fn folder_unread_thread_keys(
    conn: &rusqlite::Connection,
    folder_id: &str,
    want: usize,
) -> Result<Vec<((String, String), i64)>> {
    let mut unread = conn.prepare_cached(
        "SELECT DISTINCT m.account_id, mf.thread_key
         FROM message_folders mf
         JOIN messages m ON m.id = mf.message_id
         WHERE mf.folder_id = ?1 AND mf.is_deleted = 0 AND mf.is_read = 0",
    )?;
    let keys = unread
        .query_map(params![folder_id], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    let mut latest = conn.prepare_cached(
        "SELECT MAX(date) FROM message_folders
         WHERE folder_id = ?1 AND thread_key = ?2 AND is_deleted = 0",
    )?;
    let mut ranked = Vec::with_capacity(keys.len());
    for key in keys {
        let date: Option<i64> = latest.query_row(params![folder_id, key.1], |row| row.get(0))?;
        ranked.push((key, date.unwrap_or_default()));
    }
    ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
    ranked.truncate(want);
    Ok(ranked)
}

#[derive(Default)]
struct ThreadAccumulator {
    account_id: String,
    subject: String,
    last_date: i64,
    snippet: String,
    message_ids: HashSet<String>,
    unread: u32,
    starred: bool,
    has_attachments: bool,
    participants: Vec<String>,
    /// Lower-cased sender addresses already in `participants`.
    participant_addresses: HashSet<String>,
}

impl Store {
    pub fn insert_message(&self, msg: &Message, folder_ids: &[String]) -> Result<()> {
        self.with_write(|conn| {
            let to_json = serde_json::to_string(&msg.to_list)
                .map_err(|e| PebbleError::Storage(e.to_string()))?;
            let cc_json = serde_json::to_string(&msg.cc_list)
                .map_err(|e| PebbleError::Storage(e.to_string()))?;
            let bcc_json = serde_json::to_string(&msg.bcc_list)
                .map_err(|e| PebbleError::Storage(e.to_string()))?;

            let tx = conn.unchecked_transaction()?;
            insert_message_row(&tx, msg, &to_json, &cc_json, &bcc_json)?;

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
}
