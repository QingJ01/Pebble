use super::*;

impl Store {
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

    /// Change mailbox membership and its mailbox-local UID atomically.
    pub fn move_message_to_folder_with_remote_id(
        &self,
        message_id: &str,
        target_folder_id: &str,
        new_remote_id: &str,
    ) -> Result<()> {
        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute(
                "DELETE FROM message_folders WHERE message_id = ?1",
                params![message_id],
            )?;
            let changed = tx.execute(
                "UPDATE messages SET remote_id = ?1, is_deleted = 0, deleted_at = NULL,
                 updated_at = ?2 WHERE id = ?3",
                params![new_remote_id, pebble_core::now_timestamp(), message_id],
            )?;
            if changed == 0 {
                return Err(PebbleError::Internal(format!(
                    "Message not found for remote_id update: {message_id}"
                )));
            }
            tx.execute(
                "INSERT INTO message_folders (message_id, folder_id) VALUES (?1, ?2)",
                params![message_id, target_folder_id],
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
}

impl Store {
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
            let tx = conn.unchecked_transaction()?;
            let now = pebble_core::now_timestamp();
            for chunk in message_ids.chunks(SQL_IN_CHUNK_SIZE) {
                let placeholders: Vec<String> = (0..chunk.len())
                    .map(|i| format!("?{}", i + 2))
                    .collect();
                let sql = format!(
                    "UPDATE messages SET is_deleted = 1, deleted_at = ?1, updated_at = ?1 WHERE id IN ({})",
                    placeholders.join(", ")
                );
                let mut param_values: Vec<Box<dyn rusqlite::types::ToSql>> = Vec::with_capacity(chunk.len() + 1);
                param_values.push(Box::new(now));
                for id in chunk {
                    param_values.push(Box::new(id.clone()));
                }
                let params: Vec<&dyn rusqlite::types::ToSql> = param_values.iter().map(|v| v.as_ref()).collect();
                tx.execute(&sql, params.as_slice())?;
            }
            tx.commit()?;
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
}
