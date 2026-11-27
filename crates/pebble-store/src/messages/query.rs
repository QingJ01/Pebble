use super::*;

impl Store {
    pub fn list_starred_messages(
        &self,
        account_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<MessageSummary>> {
        self.with_read(|conn| {
            let sql = format!(
                "SELECT {MSG_SUMMARY_SELECT} FROM messages m
                 WHERE m.account_id = ?1 AND m.is_starred = 1 AND m.is_deleted = 0
                 ORDER BY m.date DESC, m.id DESC
                 LIMIT ?2 OFFSET ?3"
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

    /// Newest-first page of a folder (offset pagination; prefer
    /// [`Store::list_messages_page`] with a cursor for deep pages).
    pub fn list_messages_by_folder(
        &self,
        folder_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<MessageSummary>> {
        self.list_messages_page(
            std::slice::from_ref(&folder_id.to_string()),
            None,
            limit,
            offset,
        )
    }

    /// Newest-first page across one or more folders (a message in several of
    /// them is returned once). With `cursor`, returns messages strictly older
    /// than `(cursor.date, cursor.id)` (keyset pagination, `offset` is then
    /// applied after the cursor and should normally be 0).
    ///
    /// Each folder is read through `idx_mf_folder_date` (a backwards index
    /// range scan with no sort) and only the page's rows are fetched from
    /// `messages`.
    pub fn list_messages_page(
        &self,
        folder_ids: &[String],
        cursor: Option<&MessageCursor>,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<MessageSummary>> {
        self.list_messages_page_filtered(folder_ids, cursor, limit, offset, false)
    }

    /// [`Store::list_messages_page`], optionally keeping only unread messages.
    pub fn list_messages_page_filtered(
        &self,
        folder_ids: &[String],
        cursor: Option<&MessageCursor>,
        limit: u32,
        offset: u32,
        unread_only: bool,
    ) -> Result<Vec<MessageSummary>> {
        if folder_ids.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        self.with_read(|conn| {
            let ids = page_message_ids(conn, folder_ids, cursor, limit, offset, unread_only)?;
            fetch_summaries_ordered(conn, &ids)
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
                "SELECT {MSG_SELECT} FROM message_folders mf
                 JOIN messages m ON m.id = mf.message_id
                 LEFT JOIN message_bodies b ON b.message_id = m.id
                 WHERE mf.folder_id = ?1 AND mf.is_deleted = 0
                 ORDER BY mf.date DESC, mf.message_id DESC
                 LIMIT ?2 OFFSET ?3"
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
    /// once, even if it lives in multiple folders (e.g. Gmail labels).
    pub fn list_full_messages_by_account(
        &self,
        account_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<Message>> {
        self.with_read(|conn| {
            let sql = format!(
                "SELECT {MSG_SELECT} FROM {MSG_FROM}
                 WHERE m.account_id = ?1 AND m.is_deleted = 0
                 ORDER BY m.date DESC, m.id ASC
                 LIMIT ?2 OFFSET ?3",
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

    /// Iterate every live message exactly once, in rowid order, for full
    /// search reindexing. Returns `(rowid, message)` pairs with
    /// `rowid > after_rowid`; pass the last rowid back to continue. Unlike
    /// OFFSET pagination each batch is an O(batch) rowid range seek, and rows
    /// inserted during the walk are appended after the cursor.
    pub fn list_full_messages_after_rowid(
        &self,
        after_rowid: i64,
        limit: u32,
    ) -> Result<Vec<(i64, Message)>> {
        self.with_read(|conn| {
            let sql = format!(
                "SELECT m.rowid, {MSG_SELECT} FROM {MSG_FROM}
                 WHERE m.rowid > ?1 AND m.is_deleted = 0
                 ORDER BY m.rowid
                 LIMIT ?2"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![after_rowid, limit], |row| {
                let rowid: i64 = row.get(0)?;
                let message = row_to_message_at(row, 1)?;
                Ok((rowid, message))
            })?;
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
        let mut seen = HashSet::new();
        let unique_ids: Vec<&String> = message_ids
            .iter()
            .filter(|message_id| seen.insert(*message_id))
            .collect();
        self.with_read(|conn| {
            let mut out: HashMap<String, Vec<String>> = HashMap::new();
            for chunk in unique_ids.chunks(SQL_IN_CHUNK_SIZE) {
                let sql = format!(
                    "SELECT message_id, folder_id FROM message_folders
                     WHERE message_id IN ({})",
                    placeholders(1, chunk.len()),
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
                for row in rows {
                    let (mid, fid) = row?;
                    out.entry(mid).or_default().push(fid);
                }
            }
            Ok(out)
        })
    }

    /// Owning account of each message id, in one query per chunk. Unknown ids
    /// are absent from the map.
    pub fn get_message_account_ids_batch(
        &self,
        message_ids: &[String],
    ) -> Result<HashMap<String, String>> {
        if message_ids.is_empty() {
            return Ok(HashMap::new());
        }
        self.with_read(|conn| {
            let mut out = HashMap::with_capacity(message_ids.len());
            for chunk in message_ids.chunks(SQL_IN_CHUNK_SIZE) {
                let sql = format!(
                    "SELECT id, account_id FROM messages WHERE id IN ({})",
                    placeholders(1, chunk.len()),
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(rusqlite::params_from_iter(chunk.iter()), |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
                for row in rows {
                    let (id, account_id) = row?;
                    out.insert(id, account_id);
                }
            }
            Ok(out)
        })
    }

    /// List message IDs in a thread restricted to the given folders.
    pub fn list_message_ids_by_thread_in_folders(
        &self,
        thread_id: &str,
        folder_ids: &[String],
    ) -> Result<Vec<String>> {
        if folder_ids.is_empty() {
            return Ok(Vec::new());
        }
        let mut seen = HashSet::new();
        let unique_folder_ids: Vec<&String> =
            folder_ids.iter().filter(|id| seen.insert(*id)).collect();
        self.with_read(|conn| {
            let mut out = Vec::new();
            for chunk in unique_folder_ids.chunks(SQL_IN_CHUNK_SIZE) {
                let placeholders: Vec<String> =
                    (3..=chunk.len() + 2).map(|i| format!("?{i}")).collect();
                let sql = format!(
                    "SELECT m.id FROM messages m
                     JOIN message_folders mf ON mf.message_id = m.id
                     WHERE (m.thread_id = ?1 OR (m.thread_id IS NULL AND m.id = ?1))
                       AND mf.folder_id IN ({}) AND m.is_deleted = 0
                     ORDER BY m.date ASC",
                    placeholders.join(", "),
                );
                let mut stmt = conn.prepare(&sql)?;
                let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = vec![
                    Box::new(thread_id.to_string()),
                    Box::new(thread_id.to_string()),
                ];
                for folder_id in chunk {
                    param_values.push(Box::new((*folder_id).clone()));
                }
                let params: Vec<&dyn rusqlite::types::ToSql> =
                    param_values.iter().map(|v| v.as_ref()).collect();
                let rows = stmt.query_map(params.as_slice(), |row| row.get::<_, String>(0))?;
                for row in rows {
                    let id = row?;
                    if !out.contains(&id) {
                        out.push(id);
                    }
                }
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
        self.list_messages_page(folder_ids, None, limit, offset)
    }

    pub fn get_message(&self, id: &str) -> Result<Option<Message>> {
        self.with_read(|conn| {
            let sql = format!("SELECT {MSG_SELECT} FROM {MSG_FROM} WHERE m.id = ?1");
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
            let mut by_id = HashMap::new();
            for chunk in ids.chunks(SQL_IN_CHUNK_SIZE) {
                let sql = format!(
                    "SELECT {MSG_SELECT} FROM {MSG_FROM} WHERE m.id IN ({})",
                    placeholders(1, chunk.len())
                );
                let mut stmt = conn.prepare(&sql)?;
                let rows =
                    stmt.query_map(rusqlite::params_from_iter(chunk.iter()), row_to_message)?;
                for row in rows {
                    let message = row?;
                    by_id.insert(message.id.clone(), message);
                }
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
}

impl Store {
    /// List all messages of one account's thread, ordered chronologically.
    ///
    /// Thread ids are derived from Message-ID headers, so the same
    /// conversation delivered to two local accounts shares a thread id; the
    /// account filter keeps their copies apart. `None` (legacy callers that
    /// cannot resolve the account) returns every account's copy.
    pub fn list_messages_by_thread(
        &self,
        account_id: Option<&str>,
        thread_id: &str,
    ) -> Result<Vec<Message>> {
        self.with_read(|conn| {
            let sql = format!(
                "SELECT {MSG_SELECT} FROM {MSG_FROM}
                 WHERE (m.thread_id = ?1 OR (m.thread_id IS NULL AND m.id = ?1))
                   AND (?2 IS NULL OR m.account_id = ?2)
                   AND m.is_deleted = 0
                 ORDER BY m.date ASC, m.id ASC"
            );
            let mut stmt = conn.prepare(&sql)?;
            let rows = stmt.query_map(params![thread_id, account_id], row_to_message)?;
            let mut messages = Vec::new();
            for row in rows {
                messages.push(row?);
            }
            Ok(messages)
        })
    }

    /// List thread summaries for a folder, ordered by most recent message.
    ///
    /// Aggregates only over messages that actually live in this folder, so
    /// the snippet reflects the most recent message *in this folder*.
    pub fn list_threads_by_folder(
        &self,
        folder_id: &str,
        limit: u32,
        offset: u32,
    ) -> Result<Vec<pebble_core::ThreadSummary>> {
        self.list_threads_by_folders(std::slice::from_ref(&folder_id.to_string()), limit, offset)
    }

    /// List thread summaries across folders, ordered by most recent selected
    /// message. Messages present in more than one selected folder are counted
    /// once; threads are keyed by `(account_id, thread key)` so two accounts'
    /// copies of one conversation stay separate.
    ///
    /// Two phases: (1) walk each folder's date index newest-first until
    /// `offset + limit` distinct threads are known, (2) aggregate only the
    /// page's threads via `idx_mf_folder_thread`.
    pub fn list_threads_by_folders(
        &self,
        folder_ids: &[String],
        limit: u32,
        offset: u32,
    ) -> Result<Vec<pebble_core::ThreadSummary>> {
        self.list_threads_by_folders_filtered(folder_ids, limit, offset, false)
    }

    /// [`Store::list_threads_by_folders`], optionally keeping only threads
    /// with an unread message in the selected folders. Matching threads are
    /// still aggregated in full (all their messages in those folders).
    pub fn list_threads_by_folders_filtered(
        &self,
        folder_ids: &[String],
        limit: u32,
        offset: u32,
        unread_only: bool,
    ) -> Result<Vec<pebble_core::ThreadSummary>> {
        if folder_ids.is_empty() || limit == 0 {
            return Ok(Vec::new());
        }
        let mut unique_folders: Vec<String> = Vec::new();
        for folder_id in folder_ids {
            if !unique_folders.contains(folder_id) {
                unique_folders.push(folder_id.clone());
            }
        }

        self.with_read(|conn| {
            // Phase 1: the page's thread keys.
            let want = limit.saturating_add(offset) as usize;
            let mut latest: HashMap<(String, String), i64> = HashMap::new();
            for folder_id in &unique_folders {
                for (key, date) in folder_thread_keys(conn, folder_id, want, unread_only)? {
                    let entry = latest.entry(key).or_insert(date);
                    *entry = (*entry).max(date);
                }
            }
            let mut ranked: Vec<((String, String), i64)> = latest.into_iter().collect();
            ranked.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| b.0.cmp(&a.0)));
            let page: Vec<(String, String)> = ranked
                .into_iter()
                .skip(offset as usize)
                .take(limit as usize)
                .map(|(key, _)| key)
                .collect();
            if page.is_empty() {
                return Ok(Vec::new());
            }

            // Phase 2: aggregate only those threads.
            let mut thread_keys: Vec<&str> = page.iter().map(|(_, key)| key.as_str()).collect();
            thread_keys.sort_unstable();
            thread_keys.dedup();
            let mut accumulators: HashMap<(String, String), ThreadAccumulator> = page
                .iter()
                .map(|key| {
                    (
                        key.clone(),
                        ThreadAccumulator {
                            account_id: key.0.clone(),
                            ..Default::default()
                        },
                    )
                })
                .collect();
            let sql = format!(
                "SELECT m.account_id, mf.thread_key, m.id, m.subject, m.snippet, m.date,
                        m.is_read, m.is_starred, m.from_address, m.has_attachments, m.from_name
                 FROM message_folders mf
                 JOIN messages m ON m.id = mf.message_id
                 WHERE mf.folder_id IN ({})
                   AND mf.thread_key IN ({})
                   AND mf.is_deleted = 0
                 ORDER BY m.date ASC, m.id ASC",
                placeholders(1, unique_folders.len()),
                placeholders(unique_folders.len() + 1, thread_keys.len()),
            );
            let mut stmt = conn.prepare(&sql)?;
            let params = unique_folders
                .iter()
                .map(String::as_str)
                .chain(thread_keys.iter().copied());
            let mut rows = stmt.query(rusqlite::params_from_iter(params))?;
            while let Some(row) = rows.next()? {
                let key = (row.get::<_, String>(0)?, row.get::<_, String>(1)?);
                let Some(acc) = accumulators.get_mut(&key) else {
                    continue;
                };
                let message_id: String = row.get(2)?;
                if !acc.message_ids.insert(message_id) {
                    continue;
                }
                let subject: String = row.get(3)?;
                let snippet: String = row.get(4)?;
                let date: i64 = row.get(5)?;
                let is_read: bool = row.get::<_, i64>(6)? != 0;
                let is_starred: bool = row.get::<_, i64>(7)? != 0;
                let from_address: String = row.get(8)?;
                let has_attachments: bool = row.get::<_, i64>(9)? != 0;

                // Rows arrive oldest first: the thread's title is the
                // earliest message's subject without reply/forward prefixes,
                // so a later reply that edits the subject cannot rename it.
                if acc.subject.is_empty() {
                    acc.subject = pebble_core::normalize_subject(&subject);
                }
                if acc.message_ids.len() == 1 || date > acc.last_date {
                    acc.last_date = date;
                    acc.snippet = snippet;
                } else if date == acc.last_date && snippet > acc.snippet {
                    acc.snippet = snippet;
                }
                if !is_read {
                    acc.unread += 1;
                }
                acc.starred |= is_starred;
                acc.has_attachments |= has_attachments;
                // Participants are shown as names (like the message list),
                // falling back to the address; one entry per address.
                let from_name: String = row.get::<_, Option<String>>(10)?.unwrap_or_default();
                let from_address = from_address.trim();
                if !from_address.is_empty()
                    && acc
                        .participant_addresses
                        .insert(from_address.to_ascii_lowercase())
                {
                    let name = from_name.trim();
                    acc.participants.push(if name.is_empty() {
                        from_address.to_string()
                    } else {
                        name.to_string()
                    });
                }
            }

            Ok(page
                .into_iter()
                .filter_map(|key| {
                    let acc = accumulators.remove(&key)?;
                    if acc.message_ids.is_empty() {
                        return None;
                    }
                    Some(pebble_core::ThreadSummary {
                        thread_id: key.1,
                        account_id: acc.account_id,
                        subject: acc.subject,
                        snippet: acc.snippet,
                        last_date: acc.last_date,
                        message_count: acc.message_ids.len() as u32,
                        unread_count: acc.unread,
                        is_starred: acc.starred,
                        participants: acc.participants,
                        has_attachments: acc.has_attachments,
                    })
                })
                .collect())
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
            for chunk in ref_ids.chunks(SQL_IN_CHUNK_SIZE) {
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
                 FROM folders f
                 JOIN message_folders mf
                   ON mf.folder_id = f.id AND mf.is_read = 0 AND mf.is_deleted = 0
                 WHERE f.account_id = ?1
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
