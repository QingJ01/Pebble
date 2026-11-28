//! V27: model-assisted notifications (#107, M3b).
//!
//! - `notification_log.confidence`: how sure the model was (no text kept).
//! - `notify_feedback`: the user's "not important" / "should notify"
//!   corrections. Kept on this device; recent ones are examples for the
//!   model, repeated ones suggest a VIP or mute rule.

use super::{column_exists, set_schema_version};
use pebble_core::Result;
use rusqlite::Connection;

pub(super) fn apply_v27(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    if !column_exists(&tx, "notification_log", "confidence")? {
        tx.execute_batch("ALTER TABLE notification_log ADD COLUMN confidence REAL;")?;
    }
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS notify_feedback (
             id INTEGER PRIMARY KEY AUTOINCREMENT,
             account_id TEXT NOT NULL,
             message_id TEXT NOT NULL,
             sender TEXT NOT NULL,
             subject TEXT NOT NULL DEFAULT '',
             important INTEGER NOT NULL,
             created_at INTEGER NOT NULL,
             UNIQUE (account_id, message_id)
         );
         CREATE INDEX IF NOT EXISTS idx_notify_feedback_sender
             ON notify_feedback(account_id, sender);",
    )?;
    set_schema_version(&tx, 27)?;
    tx.commit()?;
    Ok(())
}
