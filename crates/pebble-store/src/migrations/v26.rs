//! V26: chat with the assistant (#107, M3).
//!
//! One continuous conversation, interleaved in the UI with the agent's own
//! messages (`agent_events`). The payload (text, the mail it used, draft
//! proposals) is encrypted by the app before it is stored here.

use super::set_schema_version;
use pebble_core::Result;
use rusqlite::Connection;

pub(super) fn apply_v26(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS agent_chat (
             id TEXT PRIMARY KEY,
             role TEXT NOT NULL CHECK (role IN ('user', 'assistant')),
             run_id TEXT,
             status TEXT NOT NULL DEFAULT 'done' CHECK (status IN ('done', 'error', 'cancelled')),
             payload BLOB NOT NULL,
             created_at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_agent_chat_created ON agent_chat(created_at);",
    )?;
    set_schema_version(&tx, 26)?;
    tx.commit()?;
    Ok(())
}
