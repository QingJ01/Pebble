//! V23: index for unread-only list pages (#75).
//!
//! The unread filter pages a folder newest-first while skipping read rows.
//! `idx_mf_folder_date` would walk every read row to find them, and
//! `idx_mf_folder_unread` (folder only) would need a sort, so add a partial
//! index over unread rows in date order. `thread_key` makes it covering for
//! the thread-list walk too.

use super::{set_schema_version, table_exists};
use pebble_core::Result;
use rusqlite::Connection;

pub(super) fn apply_v23(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    // Partial test schemas may lack the table; there is nothing to index then.
    if table_exists(&tx, "message_folders")? {
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_mf_folder_unread_date
             ON message_folders(folder_id, date, message_id, thread_key)
             WHERE is_read = 0 AND is_deleted = 0;",
        )?;
    }
    set_schema_version(&tx, 23)?;
    tx.commit()?;
    Ok(())
}
