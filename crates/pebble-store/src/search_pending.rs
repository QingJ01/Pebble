use pebble_core::{now_timestamp, PebbleError, Result};

use crate::Store;

impl Store {
    pub fn add_search_pending(&self, message_ids: &[String], operation: &str) -> Result<()> {
        if message_ids.is_empty() {
            return Ok(());
        }
        // One transaction for the whole batch: without it every row is its
        // own autocommit (and WAL fsync) on the single writer connection.
        self.with_write(|conn| {
            let now = now_timestamp();
            let tx = conn.unchecked_transaction()?;
            {
                let mut stmt = tx
                    .prepare_cached("INSERT OR REPLACE INTO search_pending (message_id, operation, created_at) VALUES (?1, ?2, ?3)")
                    .map_err(|e| PebbleError::Storage(format!("Failed to prepare search_pending insert: {e}")))?;
                for id in message_ids {
                    stmt.execute(rusqlite::params![id, operation, now])
                        .map_err(|e| PebbleError::Storage(format!("Failed to insert search_pending: {e}")))?;
                }
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn list_search_pending(&self) -> Result<Vec<(String, String)>> {
        self.with_read(|conn| {
            let mut stmt = conn
                .prepare("SELECT message_id, operation FROM search_pending ORDER BY created_at")
                .map_err(|e| {
                    PebbleError::Storage(format!("Failed to query search_pending: {e}"))
                })?;
            let rows = stmt
                .query_map([], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })
                .map_err(|e| PebbleError::Storage(format!("Failed to read search_pending: {e}")))?;
            let mut result = Vec::new();
            for row in rows {
                result.push(row.map_err(|e| PebbleError::Storage(e.to_string()))?);
            }
            Ok(result)
        })
    }

    pub fn clear_search_pending(&self, message_ids: &[String]) -> Result<()> {
        if message_ids.is_empty() {
            return Ok(());
        }
        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()?;
            for chunk in message_ids.chunks(500) {
                let placeholders = (1..=chunk.len())
                    .map(|i| format!("?{i}"))
                    .collect::<Vec<_>>()
                    .join(", ");
                tx.execute(
                    &format!("DELETE FROM search_pending WHERE message_id IN ({placeholders})"),
                    rusqlite::params_from_iter(chunk.iter()),
                )
                .map_err(|e| {
                    PebbleError::Storage(format!("Failed to delete search_pending: {e}"))
                })?;
            }
            tx.commit()?;
            Ok(())
        })
    }

    pub fn clear_all_search_pending(&self) -> Result<()> {
        self.with_write(|conn| {
            conn.execute("DELETE FROM search_pending", [])
                .map_err(|e| {
                    PebbleError::Storage(format!("Failed to clear search_pending: {e}"))
                })?;
            Ok(())
        })
    }
}
