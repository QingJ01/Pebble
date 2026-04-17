use pebble_core::{now_timestamp, PebbleError, Result};

use crate::Store;

impl Store {
    pub fn add_search_pending(&self, message_ids: &[String], operation: &str) -> Result<()> {
        self.with_write(|conn| {
            let now = now_timestamp();
            let mut stmt = conn
                .prepare("INSERT OR REPLACE INTO search_pending (message_id, operation, created_at) VALUES (?1, ?2, ?3)")
                .map_err(|e| PebbleError::Storage(format!("Failed to prepare search_pending insert: {e}")))?;
            for id in message_ids {
                stmt.execute(rusqlite::params![id, operation, now])
                    .map_err(|e| PebbleError::Storage(format!("Failed to insert search_pending: {e}")))?;
            }
            Ok(())
        })
    }

    pub fn list_search_pending(&self) -> Result<Vec<(String, String)>> {
        self.with_read(|conn| {
            let mut stmt = conn
                .prepare("SELECT message_id, operation FROM search_pending ORDER BY created_at")
                .map_err(|e| PebbleError::Storage(format!("Failed to query search_pending: {e}")))?;
            let rows = stmt
                .query_map([], |row| Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?)))
                .map_err(|e| PebbleError::Storage(format!("Failed to read search_pending: {e}")))?;
            let mut result = Vec::new();
            for row in rows {
                result.push(row.map_err(|e| PebbleError::Storage(e.to_string()))?);
            }
            Ok(result)
        })
    }

    pub fn clear_search_pending(&self, message_ids: &[String]) -> Result<()> {
        self.with_write(|conn| {
            let mut stmt = conn
                .prepare("DELETE FROM search_pending WHERE message_id = ?1")
                .map_err(|e| PebbleError::Storage(format!("Failed to prepare search_pending delete: {e}")))?;
            for id in message_ids {
                stmt.execute(rusqlite::params![id])
                    .map_err(|e| PebbleError::Storage(format!("Failed to delete search_pending: {e}")))?;
            }
            Ok(())
        })
    }

    pub fn clear_all_search_pending(&self) -> Result<()> {
        self.with_write(|conn| {
            conn.execute("DELETE FROM search_pending", [])
                .map_err(|e| PebbleError::Storage(format!("Failed to clear search_pending: {e}")))?;
            Ok(())
        })
    }
}
