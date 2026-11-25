//! Thread-id bookkeeping for stored messages (re-parenting and re-threading).

use pebble_core::Result;
use rusqlite::params;

use crate::Store;

/// The threading-relevant headers of one stored message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadHeaderRow {
    pub id: String,
    pub message_id_header: Option<String>,
    pub in_reply_to: Option<String>,
    pub references_header: Option<String>,
    pub thread_id: Option<String>,
    pub date: i64,
}

impl Store {
    /// Threading headers of every message of an account (tombstones included,
    /// matching the account-wide scope of [`Store::merge_thread_aliases`]).
    pub fn list_thread_headers(&self, account_id: &str) -> Result<Vec<ThreadHeaderRow>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id, message_id_header, in_reply_to, references_header, thread_id, date
                 FROM messages WHERE account_id = ?1",
            )?;
            let rows = stmt
                .query_map(params![account_id], |row| {
                    Ok(ThreadHeaderRow {
                        id: row.get(0)?,
                        message_id_header: row.get(1)?,
                        in_reply_to: row.get(2)?,
                        references_header: row.get(3)?,
                        thread_id: row.get(4)?,
                        date: row.get(5)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// Apply thread merges in order inside one write transaction: every
    /// message of the account filed under one of a merge's aliases moves to
    /// its thread id. Returns the ids of the messages that changed.
    pub fn merge_thread_aliases(
        &self,
        account_id: &str,
        merges: &[(String, Vec<String>)],
    ) -> Result<Vec<String>> {
        if merges.iter().all(|(_, aliases)| aliases.is_empty()) {
            return Ok(Vec::new());
        }
        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()?;
            let mut changed = Vec::new();
            {
                let mut stmt = tx.prepare(
                    "UPDATE messages SET thread_id = ?1
                     WHERE account_id = ?2 AND thread_id = ?3
                     RETURNING id",
                )?;
                for (thread_id, aliases) in merges {
                    for alias in aliases {
                        let ids = stmt
                            .query_map(params![thread_id, account_id, alias], |row| {
                                row.get::<_, String>(0)
                            })?
                            .collect::<rusqlite::Result<Vec<_>>>()?;
                        changed.extend(ids);
                    }
                }
            }
            tx.commit()?;
            changed.sort();
            changed.dedup();
            Ok(changed)
        })
    }

    /// Re-file stored messages under the given thread ids. Each entry is
    /// `(local_message_id, thread_id)`. Returns the local ids that changed.
    pub fn assign_thread_ids(
        &self,
        account_id: &str,
        assignments: &[(String, String)],
    ) -> Result<Vec<String>> {
        if assignments.is_empty() {
            return Ok(Vec::new());
        }
        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()?;
            let mut changed = Vec::new();
            {
                let mut stmt = tx.prepare(
                    "UPDATE messages SET thread_id = ?1
                     WHERE id = ?2 AND account_id = ?3
                       AND (thread_id IS NULL OR thread_id <> ?1)",
                )?;
                for (message_id, thread_id) in assignments {
                    if stmt.execute(params![thread_id, message_id, account_id])? > 0 {
                        changed.push(message_id.clone());
                    }
                }
            }
            tx.commit()?;
            Ok(changed)
        })
    }
}
