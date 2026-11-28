//! V25: follow-ups and the agent activity feed (#107, M2).
//!
//! - `thread_watches`: "tell me when they reply" / "remind me if there's no
//!   reply by …". A watch matches by thread id, or by normalised subject plus
//!   the people we are waiting for (replies to freshly sent mail arrive
//!   before the sent copy is threaded).
//! - `agent_events`: what the agent tells the user on its own initiative:
//!   follow-up replies, overdue follow-ups, mail awaiting the user's reply,
//!   daily briefings, deadlines. `dedupe_key` keeps each event single;
//!   `push_state` tracks whether it still waits for a desktop notification
//!   (`pending`), got one (`pushed`) or stays in the app (`in_app`);
//!   `resolution_json` records the user's answer to it ("wait 2 days").

use super::set_schema_version;
use pebble_core::Result;
use rusqlite::Connection;

pub(super) fn apply_v25(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS thread_watches (
             id TEXT PRIMARY KEY,
             account_id TEXT NOT NULL,
             thread_id TEXT,
             subject_key TEXT NOT NULL DEFAULT '',
             participants_json TEXT NOT NULL DEFAULT '[]',
             anchor_message_id TEXT,
             title TEXT NOT NULL DEFAULT '',
             status TEXT NOT NULL CHECK (status IN ('active', 'paused', 'done', 'cancelled')),
             source TEXT NOT NULL DEFAULT 'ui',
             conversation_id TEXT,
             created_at INTEGER NOT NULL,
             deadline INTEGER,
             nudged_at INTEGER,
             matched_message_id TEXT,
             matched_at INTEGER,
             updated_at INTEGER NOT NULL
         );
         CREATE INDEX IF NOT EXISTS idx_thread_watches_active
             ON thread_watches(account_id, status);
         CREATE INDEX IF NOT EXISTS idx_thread_watches_deadline
             ON thread_watches(deadline) WHERE status = 'active' AND deadline IS NOT NULL;

         CREATE TABLE IF NOT EXISTS agent_events (
             id TEXT PRIMARY KEY,
             kind TEXT NOT NULL,
             dedupe_key TEXT NOT NULL UNIQUE,
             account_id TEXT,
             message_id TEXT,
             thread_id TEXT,
             watch_id TEXT,
             conversation_id TEXT,
             payload_json TEXT NOT NULL DEFAULT '{}',
             created_at INTEGER NOT NULL,
             read_at INTEGER,
             dismissed_at INTEGER,
             push_state TEXT NOT NULL DEFAULT 'in_app'
                 CHECK (push_state IN ('pending', 'pushed', 'in_app')),
             pushed_at INTEGER,
             resolution_json TEXT
         );
         CREATE INDEX IF NOT EXISTS idx_agent_events_pending
             ON agent_events(push_state) WHERE push_state = 'pending';
         CREATE INDEX IF NOT EXISTS idx_agent_events_pushed
             ON agent_events(pushed_at) WHERE pushed_at IS NOT NULL;
         CREATE INDEX IF NOT EXISTS idx_agent_events_created ON agent_events(created_at);
         CREATE INDEX IF NOT EXISTS idx_agent_events_unread
             ON agent_events(read_at, dismissed_at, created_at);",
    )?;
    set_schema_version(&tx, 25)?;
    tx.commit()?;
    Ok(())
}
