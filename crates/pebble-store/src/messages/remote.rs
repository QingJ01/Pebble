use super::*;

impl Store {
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
            let mut result = HashSet::new();
            for chunk in remote_ids.chunks(SQL_IN_CHUNK_SIZE) {
                let placeholders: Vec<String> = (0..chunk.len())
                    .map(|i| format!("?{}", i + 2))
                    .collect();
                let sql = format!(
                    "SELECT remote_id FROM messages WHERE account_id = ?1 AND remote_id IN ({}) AND is_deleted = 0",
                    placeholders.join(", ")
                );
                let mut stmt = conn.prepare(&sql)?;
                let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::with_capacity(chunk.len() + 1);
                params_vec.push(Box::new(account_id.to_string()));
                for rid in chunk {
                    params_vec.push(Box::new(rid.clone()));
                }
                let param_refs: Vec<&dyn rusqlite::types::ToSql> = params_vec.iter().map(|p| p.as_ref()).collect();
                let rows = stmt.query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))?;
                for row in rows {
                    result.insert(row?);
                }
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
            let mut result = HashSet::new();
            for chunk in remote_ids.chunks(SQL_IN_CHUNK_SIZE) {
                let placeholders: Vec<String> =
                    (0..chunk.len()).map(|i| format!("?{}", i + 3)).collect();
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
                    Vec::with_capacity(chunk.len() + 2);
                params_vec.push(Box::new(account_id.to_string()));
                params_vec.push(Box::new(folder_id.to_string()));
                for rid in chunk {
                    params_vec.push(Box::new(rid.clone()));
                }
                let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                    params_vec.iter().map(|p| p.as_ref()).collect();
                let rows = stmt.query_map(param_refs.as_slice(), |row| row.get::<_, String>(0))?;

                for row in rows {
                    result.insert(row?);
                }
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
            let mut result = HashMap::new();
            for chunk in remote_ids.chunks(SQL_IN_CHUNK_SIZE) {
                let placeholders: Vec<String> = (0..chunk.len())
                    .map(|i| format!("?{}", i + 2))
                    .collect();
                let sql = format!(
                    "SELECT remote_id, id FROM messages WHERE account_id = ?1 AND remote_id IN ({}) AND is_deleted = 0",
                    placeholders.join(", ")
                );
                let mut stmt = conn.prepare(&sql)?;
                let mut params_vec: Vec<Box<dyn rusqlite::types::ToSql>> =
                    Vec::with_capacity(chunk.len() + 1);
                params_vec.push(Box::new(account_id.to_string()));
                for remote_id in chunk {
                    params_vec.push(Box::new(remote_id.clone()));
                }
                let param_refs: Vec<&dyn rusqlite::types::ToSql> =
                    params_vec.iter().map(|p| p.as_ref()).collect();
                let rows = stmt.query_map(param_refs.as_slice(), |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;

                for row in rows {
                    let (remote_id, message_id) = row?;
                    result.insert(remote_id, message_id);
                }
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
            let mut result = HashMap::new();
            for chunk in remote_ids.chunks(SQL_IN_CHUNK_SIZE) {
                let first_remote_param = if folder_id.is_some() { 3 } else { 2 };
                let placeholders: Vec<String> = (0..chunk.len())
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
                    Vec::with_capacity(chunk.len() + 2);
                values.push(Box::new(account_id.to_string()));
                if let Some(folder_id) = folder_id {
                    values.push(Box::new(folder_id.to_string()));
                }
                for remote_id in chunk {
                    values.push(Box::new(remote_id.clone()));
                }
                let params: Vec<&dyn rusqlite::types::ToSql> =
                    values.iter().map(|value| value.as_ref()).collect();
                let mut stmt = conn.prepare(&sql)?;
                let rows = stmt.query_map(params.as_slice(), |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
                for row in rows {
                    let (remote_id, message_id) = row?;
                    result.insert(remote_id, message_id);
                }
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
                    "SELECT {MSG_SELECT}
                     FROM {MSG_FROM}
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
}
