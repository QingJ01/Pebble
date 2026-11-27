//! V24: smart notifications (#107, M1).
//!
//! - `message_signals`: bulk / auto-reply markers captured at sync time.
//! - `sent_recipients`: addresses each account has written to, so triage
//!   can tell people the user knows from strangers. Backfilled from stored
//!   mail sent by the account.
//! - `sender_prefs`: VIP / muted senders (an address or `@domain`), per
//!   account or for all accounts (`account_id = ''`).
//! - `notify_prefs`: per-account notification preferences as JSON
//!   (`account_id = ''` holds the defaults: `all` for existing installs,
//!   `smart` for new ones).
//! - `notification_log`: one row per notified-or-not new message; the
//!   unique key makes delivery idempotent across retries and restarts.

use super::{column_exists, set_schema_version, table_exists};
use pebble_core::Result;
use rusqlite::Connection;

pub(super) fn apply_v24(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS message_signals (
             message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
             bits INTEGER NOT NULL
         ) WITHOUT ROWID;

         CREATE TABLE IF NOT EXISTS sent_recipients (
             account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
             normalized_address TEXT NOT NULL,
             sent_count INTEGER NOT NULL,
             last_sent_at INTEGER NOT NULL,
             PRIMARY KEY (account_id, normalized_address)
         ) WITHOUT ROWID;

         CREATE TABLE IF NOT EXISTS sender_prefs (
             account_id TEXT NOT NULL DEFAULT '',
             pattern TEXT NOT NULL,
             kind TEXT NOT NULL CHECK (kind IN ('vip', 'mute')),
             created_at INTEGER NOT NULL,
             PRIMARY KEY (account_id, pattern)
         ) WITHOUT ROWID;

         CREATE TABLE IF NOT EXISTS notify_prefs (
             account_id TEXT PRIMARY KEY,
             prefs_json TEXT NOT NULL,
             updated_at INTEGER NOT NULL
         ) WITHOUT ROWID;

         CREATE TABLE IF NOT EXISTS notification_log (
             account_id TEXT NOT NULL,
             message_key TEXT NOT NULL,
             message_id TEXT NOT NULL,
             tier TEXT NOT NULL,
             reason TEXT NOT NULL,
             why TEXT,
             delivered INTEGER NOT NULL DEFAULT 0,
             created_at INTEGER NOT NULL,
             seen_at INTEGER,
             PRIMARY KEY (account_id, message_key)
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS idx_notification_log_created
             ON notification_log(created_at);
         CREATE INDEX IF NOT EXISTS idx_notification_log_digest
             ON notification_log(account_id, tier, seen_at, created_at);",
    )?;

    // Backfill who each account has written to. Partial test schemas may
    // lack these tables; there is nothing to backfill then.
    let can_backfill = table_exists(&tx, "messages")?
        && table_exists(&tx, "accounts")?
        && ["from_address", "to_list", "cc_list", "date", "is_deleted"]
            .iter()
            .try_fold(true, |ok, column| {
                Ok::<_, pebble_core::PebbleError>(ok && column_exists(&tx, "messages", column)?)
            })?;
    if can_backfill {
        tx.execute_batch(
            "INSERT INTO sent_recipients (account_id, normalized_address, sent_count, last_sent_at)
             SELECT account_id, address, COUNT(*), MAX(date) FROM (
                 SELECT m.account_id AS account_id,
                        lower(trim(json_extract(r.value, '$.address'))) AS address,
                        m.date AS date
                 FROM messages m
                 JOIN accounts a ON a.id = m.account_id
                      AND lower(trim(m.from_address)) = lower(trim(a.email)),
                      json_each(CASE WHEN json_valid(m.to_list) THEN m.to_list ELSE '[]' END) r
                 WHERE m.is_deleted = 0
                 UNION ALL
                 SELECT m.account_id,
                        lower(trim(json_extract(r.value, '$.address'))),
                        m.date
                 FROM messages m
                 JOIN accounts a ON a.id = m.account_id
                      AND lower(trim(m.from_address)) = lower(trim(a.email)),
                      json_each(CASE WHEN json_valid(m.cc_list) THEN m.cc_list ELSE '[]' END) r
                 WHERE m.is_deleted = 0
             )
             WHERE address LIKE '%@%'
             GROUP BY account_id, address
             ON CONFLICT(account_id, normalized_address) DO NOTHING;",
        )?;
    }

    // Default notification mode: installs that already have accounts keep
    // today's behaviour (every new inbox message notifies); new installs
    // start with smart triage.
    let has_accounts = table_exists(&tx, "accounts")?
        && tx.query_row("SELECT EXISTS(SELECT 1 FROM accounts)", [], |row| {
            row.get::<_, bool>(0)
        })?;
    let mode = if has_accounts { "all" } else { "smart" };
    tx.execute(
        "INSERT OR IGNORE INTO notify_prefs (account_id, prefs_json, updated_at) VALUES ('', ?1, 0)",
        [format!(r#"{{"mode":"{mode}"}}"#)],
    )?;

    set_schema_version(&tx, 24)?;
    tx.commit()?;
    Ok(())
}
