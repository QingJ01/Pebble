//! V28: local MCP connections (#107, M4).
//!
//! - `mcp_clients`: paired MCP clients. Only a SHA-256 hash of each
//!   client's token is kept; `grants_json` maps account id → access level.
//! - `mcp_audit`: one row per tool call: who, which tool, which accounts,
//!   how many results, the outcome. Never mail content, tokens or codes.
//! - `mcp_settings`: whether Pebble listens for MCP connections (off by
//!   default).

use super::set_schema_version;
use pebble_core::Result;
use rusqlite::Connection;

pub(super) fn apply_v28(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS mcp_clients (
             id TEXT PRIMARY KEY,
             name TEXT NOT NULL,
             token_hash TEXT NOT NULL UNIQUE,
             grants_json TEXT NOT NULL DEFAULT '{}',
             created_at INTEGER NOT NULL,
             last_used_at INTEGER,
             revoked_at INTEGER
         );
         CREATE TABLE IF NOT EXISTS mcp_audit (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             client_id TEXT NOT NULL,
             tool TEXT NOT NULL,
             accounts TEXT NOT NULL DEFAULT '',
             results INTEGER NOT NULL DEFAULT 0,
             outcome TEXT NOT NULL,
             created_at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_mcp_audit_client ON mcp_audit(client_id, created_at);
         CREATE TABLE IF NOT EXISTS mcp_settings (
             id INTEGER PRIMARY KEY CHECK (id = 1),
             enabled INTEGER NOT NULL DEFAULT 0
         );",
    )?;
    set_schema_version(&tx, 28)?;
    tx.commit()?;
    Ok(())
}
