use pebble_core::{build_snippet, PebbleError, Result};
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashSet;

const CURRENT_VERSION: u32 = 19;
const ACCOUNT_COLOR_PRESETS: [&str; 12] = [
    "#0ea5e9", "#22c55e", "#f59e0b", "#8b5cf6", "#f43f5e", "#14b8a6", "#6366f1", "#f97316",
    "#06b6d4", "#ec4899", "#84cc16", "#3b82f6",
];

fn get_schema_version(conn: &Connection) -> u32 {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap_or(0)
}

fn set_schema_version(conn: &Connection, version: u32) -> Result<()> {
    conn.pragma_update(None, "user_version", version)
        .map_err(|e| PebbleError::Storage(format!("Failed to set schema version: {e}")))
}

fn table_exists(conn: &Connection, table: &str) -> Result<bool> {
    conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
        [table],
        |row| row.get(0),
    )
    .map_err(Into::into)
}

fn is_valid_account_color(color: &str) -> bool {
    color.len() == 7
        && color.as_bytes()[0] == b'#'
        && color.as_bytes()[1..].iter().all(|b| b.is_ascii_hexdigit())
}

fn derive_account_color(seed: &str) -> String {
    let mut hash = 0u32;
    for byte in seed.bytes() {
        hash = hash.wrapping_mul(31).wrapping_add(byte as u32);
    }
    ACCOUNT_COLOR_PRESETS[(hash as usize) % ACCOUNT_COLOR_PRESETS.len()].to_string()
}

fn backfill_account_colors(conn: &Connection) -> Result<()> {
    let accounts = {
        let mut stmt =
            conn.prepare("SELECT id, color FROM accounts ORDER BY created_at ASC, id ASC")?;
        let rows = stmt.query_map([], |row| {
            Ok((row.get::<_, String>(0)?, row.get::<_, Option<String>>(1)?))
        })?;
        let mut accounts = Vec::new();
        for row in rows {
            accounts.push(row?);
        }
        accounts
    };

    let mut used_colors: HashSet<String> = accounts
        .iter()
        .filter_map(|(_, color)| color.as_deref())
        .filter(|color| is_valid_account_color(color))
        .map(str::to_ascii_lowercase)
        .collect();

    for (id, color) in accounts {
        if color.as_deref().is_some_and(is_valid_account_color) {
            continue;
        }

        let selected = ACCOUNT_COLOR_PRESETS
            .iter()
            .find(|candidate| !used_colors.contains(**candidate))
            .map(|color| (*color).to_string())
            .unwrap_or_else(|| derive_account_color(&id));
        used_colors.insert(selected.clone());
        conn.execute(
            "UPDATE accounts SET color = ?1 WHERE id = ?2",
            rusqlite::params![selected, id],
        )?;
    }

    Ok(())
}

fn accounts_provider_check_allows_pop3(conn: &Connection) -> Result<bool> {
    let sql: String = conn.query_row(
        "SELECT sql FROM sqlite_master WHERE type = 'table' AND name = 'accounts'",
        [],
        |row| row.get(0),
    )?;
    Ok(sql.contains("'pop3'"))
}

fn rebuild_accounts_with_pop3_provider(conn: &Connection) -> Result<()> {
    if conn
        .prepare("SELECT auth_data FROM accounts LIMIT 0")
        .is_err()
    {
        conn.execute_batch("ALTER TABLE accounts ADD COLUMN auth_data BLOB;")
            .map_err(|e| {
                PebbleError::Storage(format!("Migration V12 auth_data column failed: {e}"))
            })?;
    }
    if conn
        .prepare("SELECT sync_state FROM accounts LIMIT 0")
        .is_err()
    {
        conn.execute_batch("ALTER TABLE accounts ADD COLUMN sync_state TEXT;")
            .map_err(|e| {
                PebbleError::Storage(format!("Migration V12 sync_state column failed: {e}"))
            })?;
    }

    conn.execute_batch(
        "CREATE TABLE accounts_new (
            id TEXT PRIMARY KEY,
            email TEXT NOT NULL,
            display_name TEXT NOT NULL DEFAULT '',
            color TEXT,
            provider TEXT NOT NULL CHECK(provider IN ('imap', 'pop3', 'gmail', 'outlook')),
            auth_data BLOB,
            sync_state TEXT,
            created_at INTEGER NOT NULL,
            updated_at INTEGER NOT NULL
        );
        INSERT INTO accounts_new (id, email, display_name, color, provider, auth_data, sync_state, created_at, updated_at)
            SELECT id, email, display_name, color, provider, auth_data, sync_state, created_at, updated_at
            FROM accounts;
        DROP TABLE accounts;
        ALTER TABLE accounts_new RENAME TO accounts;",
    )
    .map_err(|e| PebbleError::Storage(format!("Migration V12 failed: {e}")))?;
    Ok(())
}

fn rebuild_snippets(conn: &Connection) -> Result<()> {
    let mut stmt = match conn.prepare("SELECT id, body_text, body_html_raw FROM messages") {
        Ok(s) => s,
        Err(_) => return Ok(()),
    };
    let rows: Vec<(String, String, String)> = stmt
        .query_map([], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)))
        .map_err(|e| PebbleError::Storage(format!("V13 query failed: {e}")))?
        .filter_map(|r| r.ok())
        .collect();

    let mut update = conn
        .prepare("UPDATE messages SET snippet = ?1 WHERE id = ?2")
        .map_err(|e| PebbleError::Storage(format!("V13 prepare update failed: {e}")))?;
    for (id, body_text, body_html) in &rows {
        let new_snippet = build_snippet(body_text, body_html);
        update
            .execute(rusqlite::params![new_snippet, id])
            .map_err(|e| PebbleError::Storage(format!("V13 update failed: {e}")))?;
    }
    Ok(())
}

pub fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch("PRAGMA synchronous=NORMAL;")?;

    conn.execute_batch("PRAGMA foreign_keys=ON;")?;

    conn.execute_batch("PRAGMA busy_timeout=5000;")?;

    let version = get_schema_version(conn);

    // Each migration is wrapped in a transaction so that the DDL and version
    // update are atomic; a crash mid-migration won't leave an inconsistent state.

    if version < 1 {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| PebbleError::Storage(format!("Migration V1 begin failed: {e}")))?;
        tx.execute_batch(SCHEMA_V1)
            .map_err(|e| PebbleError::Storage(format!("Migration V1 failed: {e}")))?;
        set_schema_version(&tx, 1)?;
        tx.commit()
            .map_err(|e| PebbleError::Storage(format!("Migration V1 commit failed: {e}")))?;
    }

    if version < 2 {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| PebbleError::Storage(format!("Migration V2 begin failed: {e}")))?;
        let has_content_id: bool = tx
            .prepare("SELECT content_id FROM attachments LIMIT 0")
            .is_ok();
        if !has_content_id {
            tx.execute_batch(
                "ALTER TABLE attachments ADD COLUMN content_id TEXT;
                 ALTER TABLE attachments ADD COLUMN is_inline INTEGER NOT NULL DEFAULT 0;",
            )
            .map_err(|e| PebbleError::Storage(format!("Migration V2 failed: {e}")))?;
        }
        set_schema_version(&tx, 2)?;
        tx.commit()
            .map_err(|e| PebbleError::Storage(format!("Migration V2 commit failed: {e}")))?;
    }

    if version < 3 {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| PebbleError::Storage(format!("Migration V3 begin failed: {e}")))?;
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_messages_account_remote ON messages(account_id, remote_id);
             CREATE INDEX IF NOT EXISTS idx_snoozed_unsnoozed_at ON snoozed_messages(unsnoozed_at);
             CREATE UNIQUE INDEX IF NOT EXISTS idx_folders_account_remote ON folders(account_id, remote_id);"
        )
        .map_err(|e| PebbleError::Storage(format!("Migration V3 failed: {e}")))?;
        set_schema_version(&tx, 3)?;
        tx.commit()
            .map_err(|e| PebbleError::Storage(format!("Migration V3 commit failed: {e}")))?;
    }

    if version < 4 {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| PebbleError::Storage(format!("Migration V4 begin failed: {e}")))?;
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_message_folders_folder_id ON message_folders(folder_id);
             CREATE INDEX IF NOT EXISTS idx_messages_account_starred ON messages(account_id, is_starred) WHERE is_starred = 1 AND is_deleted = 0;
             CREATE INDEX IF NOT EXISTS idx_messages_thread_date ON messages(thread_id, date) WHERE thread_id IS NOT NULL AND is_deleted = 0;"
        )
        .map_err(|e| PebbleError::Storage(format!("Migration V4 failed: {e}")))?;
        set_schema_version(&tx, 4)?;
        tx.commit()
            .map_err(|e| PebbleError::Storage(format!("Migration V4 commit failed: {e}")))?;
    }

    if version < 5 {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| PebbleError::Storage(format!("Migration V5 begin failed: {e}")))?;
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_mf_folder_message ON message_folders(folder_id, message_id);",
        )
        .map_err(|e| PebbleError::Storage(format!("Migration V5 failed: {e}")))?;
        set_schema_version(&tx, 5)?;
        tx.commit()
            .map_err(|e| PebbleError::Storage(format!("Migration V5 commit failed: {e}")))?;
    }

    // V6: search_pending table for crash-recovery of the search index
    if version < 6 {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| PebbleError::Storage(format!("Migration V6 begin failed: {e}")))?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS search_pending (
                 message_id TEXT PRIMARY KEY,
                 operation TEXT NOT NULL CHECK(operation IN ('index', 'remove')),
                 created_at INTEGER NOT NULL
             );",
        )
        .map_err(|e| PebbleError::Storage(format!("Migration V6 failed: {e}")))?;
        set_schema_version(&tx, 6)?;
        tx.commit()
            .map_err(|e| PebbleError::Storage(format!("Migration V6 commit failed: {e}")))?;
    }

    if version < 7 {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| PebbleError::Storage(format!("Migration V7 begin failed: {e}")))?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS folder_sync_state (
                 account_id TEXT NOT NULL,
                 folder_id TEXT NOT NULL,
                 state TEXT NOT NULL,
                 updated_at INTEGER NOT NULL,
                 PRIMARY KEY (account_id, folder_id),
                 FOREIGN KEY(account_id) REFERENCES accounts(id) ON DELETE CASCADE,
                 FOREIGN KEY(folder_id) REFERENCES folders(id) ON DELETE CASCADE
             );",
        )
        .map_err(|e| PebbleError::Storage(format!("Migration V7 failed: {e}")))?;
        set_schema_version(&tx, 7)?;
        tx.commit()
            .map_err(|e| PebbleError::Storage(format!("Migration V7 commit failed: {e}")))?;
    }

    if version < 8 {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| PebbleError::Storage(format!("Migration V8 begin failed: {e}")))?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS sync_failures (
                 account_id TEXT NOT NULL,
                 folder_id TEXT NOT NULL,
                 remote_id TEXT NOT NULL,
                 provider TEXT NOT NULL,
                 reason TEXT NOT NULL,
                 attempts INTEGER NOT NULL DEFAULT 1,
                 updated_at INTEGER NOT NULL,
                 PRIMARY KEY (account_id, folder_id, remote_id),
                 FOREIGN KEY(account_id) REFERENCES accounts(id) ON DELETE CASCADE,
                 FOREIGN KEY(folder_id) REFERENCES folders(id) ON DELETE CASCADE
             );
             CREATE INDEX IF NOT EXISTS idx_sync_failures_folder
                 ON sync_failures(account_id, folder_id);",
        )
        .map_err(|e| PebbleError::Storage(format!("Migration V8 failed: {e}")))?;
        set_schema_version(&tx, 8)?;
        tx.commit()
            .map_err(|e| PebbleError::Storage(format!("Migration V8 commit failed: {e}")))?;
    }

    if version < 9 {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| PebbleError::Storage(format!("Migration V9 begin failed: {e}")))?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS pending_mail_ops (
                 id TEXT PRIMARY KEY,
                 account_id TEXT NOT NULL,
                 message_id TEXT NOT NULL,
                 op_type TEXT NOT NULL,
                 payload_json TEXT NOT NULL,
                 status TEXT NOT NULL CHECK(status IN ('pending', 'in_progress', 'failed', 'done')),
                 attempts INTEGER NOT NULL DEFAULT 0,
                 last_error TEXT,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL
             );
             CREATE INDEX IF NOT EXISTS idx_pending_mail_ops_account_status
                 ON pending_mail_ops(account_id, status, updated_at);",
        )
        .map_err(|e| PebbleError::Storage(format!("Migration V9 failed: {e}")))?;
        set_schema_version(&tx, 9)?;
        tx.commit()
            .map_err(|e| PebbleError::Storage(format!("Migration V9 commit failed: {e}")))?;
    }

    if version < 10 {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| PebbleError::Storage(format!("Migration V10 begin failed: {e}")))?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS secure_user_data (
                 key TEXT PRIMARY KEY,
                 value BLOB NOT NULL,
                 updated_at INTEGER NOT NULL
             );
             ALTER TABLE pending_mail_ops ADD COLUMN next_retry_at INTEGER;
             CREATE INDEX IF NOT EXISTS idx_pending_mail_ops_retry
                 ON pending_mail_ops(status, next_retry_at, updated_at);",
        )
        .map_err(|e| PebbleError::Storage(format!("Migration V10 failed: {e}")))?;
        set_schema_version(&tx, 10)?;
        tx.commit()
            .map_err(|e| PebbleError::Storage(format!("Migration V10 commit failed: {e}")))?;
    }

    if version < 11 {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| PebbleError::Storage(format!("Migration V11 begin failed: {e}")))?;
        let has_color: bool = tx.prepare("SELECT color FROM accounts LIMIT 0").is_ok();
        if !has_color {
            tx.execute_batch("ALTER TABLE accounts ADD COLUMN color TEXT;")
                .map_err(|e| PebbleError::Storage(format!("Migration V11 failed: {e}")))?;
        }
        backfill_account_colors(&tx)?;
        set_schema_version(&tx, 11)?;
        tx.commit()
            .map_err(|e| PebbleError::Storage(format!("Migration V11 commit failed: {e}")))?;
    }

    if version < 12 {
        conn.execute_batch("PRAGMA foreign_keys=OFF;")
            .map_err(|e| PebbleError::Storage(format!("Migration V12 disable FK failed: {e}")))?;
        let migration_result = (|| -> Result<()> {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| PebbleError::Storage(format!("Migration V12 begin failed: {e}")))?;
            if !accounts_provider_check_allows_pop3(&tx)? {
                rebuild_accounts_with_pop3_provider(&tx)?;
            }
            let fk_violation = tx
                .query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
                .optional()
                .map_err(|e| PebbleError::Storage(format!("Migration V12 FK check failed: {e}")))?;
            if fk_violation.is_some() {
                return Err(PebbleError::Storage(
                    "Migration V12 introduced foreign key violations".to_string(),
                ));
            }
            set_schema_version(&tx, 12)?;
            tx.commit()
                .map_err(|e| PebbleError::Storage(format!("Migration V12 commit failed: {e}")))?;
            Ok(())
        })();
        let enable_foreign_keys_result = conn
            .execute_batch("PRAGMA foreign_keys=ON;")
            .map_err(|e| PebbleError::Storage(format!("Migration V12 enable FK failed: {e}")));
        migration_result?;
        enable_foreign_keys_result?;
    }

    // V13: rebuild snippets to strip leaked HTML/CSS from previews
    if version < 13 {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| PebbleError::Storage(format!("Migration V13 begin failed: {e}")))?;
        rebuild_snippets(&tx)?;
        set_schema_version(&tx, 13)?;
        tx.commit()
            .map_err(|e| PebbleError::Storage(format!("Migration V13 commit failed: {e}")))?;
    }

    // V14: enforce at most one LIVE row per (account_id, remote_id). The legacy
    // idx_messages_account_remote index was non-unique, so a soft-delete followed
    // by a re-sync that re-fetched the same remote_id inserted a second row
    // (tombstone + new live copy). Deduplicate existing live rows first, then
    // replace the index with a partial UNIQUE index scoped to is_deleted = 0 so
    // tombstones (is_deleted = 1) can still coexist with a fresh live copy.
    if version < 14 {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| PebbleError::Storage(format!("Migration V14 begin failed: {e}")))?;
        // The messages table always exists in production (created in SCHEMA_V1).
        // Guard the dedup/index so the migration is a no-op on the minimal
        // schemas used by migration unit tests that omit messages — and is
        // harmless if a future store variant lacks it.
        if table_exists(&tx, "messages")? {
            tx.execute_batch(
                "CREATE TEMP TABLE v14_message_dedup (
                     loser_id TEXT PRIMARY KEY,
                     winner_id TEXT NOT NULL
                 );",
            )
            .map_err(|e| PebbleError::Storage(format!("Migration V14 failed: {e}")))?;
            if table_exists(&tx, "accounts")? {
                tx.execute_batch(
                    "INSERT INTO v14_message_dedup (loser_id, winner_id)
                     SELECT id, winner_id
                     FROM (
                         SELECT
                             message.id AS id,
                             FIRST_VALUE(message.id) OVER (
                                 PARTITION BY message.account_id, message.remote_id
                                 ORDER BY message.updated_at DESC,
                                          message.created_at DESC,
                                          message.id DESC
                             ) AS winner_id
                         FROM messages AS message
                         JOIN accounts AS account ON account.id = message.account_id
                         WHERE message.is_deleted = 0
                           AND message.remote_id != ''
                           AND account.provider != 'imap'
                     )
                     WHERE id != winner_id;",
                )
                .map_err(|e| PebbleError::Storage(format!("Migration V14 failed: {e}")))?;
            } else {
                // Minimal migration-test schemas do not include accounts, so
                // retain the legacy inference there only. Production always
                // takes the provider-aware branch above.
                tx.execute_batch(
                    "INSERT INTO v14_message_dedup (loser_id, winner_id)
                     SELECT id, winner_id
                     FROM (
                         SELECT
                             id,
                             FIRST_VALUE(id) OVER (
                                 PARTITION BY account_id, remote_id
                                 ORDER BY updated_at DESC, created_at DESC, id DESC
                             ) AS winner_id
                         FROM messages
                         WHERE is_deleted = 0
                           AND remote_id != ''
                           AND remote_id GLOB '*[^0-9]*'
                     )
                     WHERE id != winner_id;",
                )
                .map_err(|e| PebbleError::Storage(format!("Migration V14 failed: {e}")))?;
            }

            if table_exists(&tx, "message_folders")? {
                tx.execute_batch(
                    "INSERT OR IGNORE INTO message_folders (message_id, folder_id)
                     SELECT dedup.winner_id, relation.folder_id
                     FROM message_folders AS relation
                     JOIN v14_message_dedup AS dedup ON dedup.loser_id = relation.message_id;",
                )?;
            }
            if table_exists(&tx, "attachments")? {
                tx.execute_batch(
                    "UPDATE attachments
                     SET message_id = (
                         SELECT winner_id FROM v14_message_dedup
                         WHERE loser_id = attachments.message_id
                     )
                     WHERE message_id IN (SELECT loser_id FROM v14_message_dedup);",
                )?;
            }
            if table_exists(&tx, "message_labels")? {
                tx.execute_batch(
                    "INSERT OR IGNORE INTO message_labels (message_id, label_id)
                     SELECT dedup.winner_id, relation.label_id
                     FROM message_labels AS relation
                     JOIN v14_message_dedup AS dedup ON dedup.loser_id = relation.message_id;",
                )?;
            }
            if table_exists(&tx, "kanban_cards")? {
                tx.execute_batch(
                    "INSERT INTO kanban_cards
                         (message_id, column_name, position, created_at, updated_at)
                     SELECT dedup.winner_id, relation.column_name, relation.position,
                            relation.created_at, relation.updated_at
                     FROM kanban_cards AS relation
                     JOIN v14_message_dedup AS dedup ON dedup.loser_id = relation.message_id
                     WHERE 1
                     ON CONFLICT(message_id) DO UPDATE SET
                         column_name = excluded.column_name,
                         position = excluded.position,
                         created_at = excluded.created_at,
                         updated_at = excluded.updated_at
                     WHERE excluded.updated_at > kanban_cards.updated_at;",
                )?;
            }
            if table_exists(&tx, "snoozed_messages")? {
                tx.execute_batch(
                    "INSERT INTO snoozed_messages
                         (message_id, snoozed_at, unsnoozed_at, return_to)
                     SELECT dedup.winner_id, relation.snoozed_at, relation.unsnoozed_at,
                            relation.return_to
                     FROM snoozed_messages AS relation
                     JOIN v14_message_dedup AS dedup ON dedup.loser_id = relation.message_id
                     WHERE 1
                     ON CONFLICT(message_id) DO UPDATE SET
                         snoozed_at = excluded.snoozed_at,
                         unsnoozed_at = excluded.unsnoozed_at,
                         return_to = excluded.return_to
                     WHERE excluded.snoozed_at > snoozed_messages.snoozed_at;",
                )?;
            }
            if table_exists(&tx, "search_pending")? {
                tx.execute_batch(
                    "INSERT INTO search_pending (message_id, operation, created_at)
                     SELECT dedup.winner_id, 'index', MAX(relation.created_at)
                     FROM search_pending AS relation
                     JOIN v14_message_dedup AS dedup ON dedup.loser_id = relation.message_id
                     GROUP BY dedup.winner_id
                     ON CONFLICT(message_id) DO UPDATE SET
                         operation = 'index',
                         created_at = MAX(search_pending.created_at, excluded.created_at);
                     UPDATE search_pending
                     SET operation = 'index'
                     WHERE message_id IN (SELECT winner_id FROM v14_message_dedup);
                     DELETE FROM search_pending
                     WHERE message_id IN (SELECT loser_id FROM v14_message_dedup);",
                )?;
            }
            if table_exists(&tx, "pending_mail_ops")? {
                tx.execute_batch(
                    "UPDATE pending_mail_ops
                     SET message_id = (
                         SELECT winner_id FROM v14_message_dedup
                         WHERE loser_id = pending_mail_ops.message_id
                     )
                     WHERE message_id IN (SELECT loser_id FROM v14_message_dedup);",
                )?;
            }

            tx.execute_batch(
                "DELETE FROM messages WHERE id IN (SELECT loser_id FROM v14_message_dedup);
                 DROP TABLE v14_message_dedup;
                 DROP INDEX IF EXISTS idx_messages_account_remote;
                 CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_account_remote_unique
                   ON messages(account_id, remote_id)
                   WHERE is_deleted = 0
                     AND remote_id != ''
                     AND remote_id GLOB '*[^0-9]*';",
            )
            .map_err(|e| PebbleError::Storage(format!("Migration V14 failed: {e}")))?;
        }
        set_schema_version(&tx, 14)?;
        tx.commit()
            .map_err(|e| PebbleError::Storage(format!("Migration V14 commit failed: {e}")))?;
    }

    // V15: local-only drafts use an empty remote_id. They are distinct records,
    // so the remote-message uniqueness constraint must not include them.
    if version < 15 {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| PebbleError::Storage(format!("Migration V15 begin failed: {e}")))?;
        if table_exists(&tx, "messages")? {
            tx.execute_batch(
                "DROP INDEX IF EXISTS idx_messages_account_remote_unique;
                 CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_account_remote_unique
                   ON messages(account_id, remote_id)
                   WHERE is_deleted = 0
                     AND remote_id != ''
                     AND remote_id GLOB '*[^0-9]*';",
            )
            .map_err(|e| PebbleError::Storage(format!("Migration V15 failed: {e}")))?;
        }
        set_schema_version(&tx, 15)?;
        tx.commit()
            .map_err(|e| PebbleError::Storage(format!("Migration V15 commit failed: {e}")))?;
    }

    // V16: IMAP UIDs are scoped to one mailbox, while OAuth/POP provider IDs
    // remain account-wide. SQLite cannot express that relationship with one
    // messages-table index, so provider-aware triggers enforce both scopes.
    if version < 16 {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| PebbleError::Storage(format!("Migration V16 begin failed: {e}")))?;
        if table_exists(&tx, "messages")? {
            tx.execute_batch("DROP INDEX IF EXISTS idx_messages_account_remote_unique;")
                .map_err(|e| PebbleError::Storage(format!("Migration V16 failed: {e}")))?;
            if table_exists(&tx, "accounts")? && table_exists(&tx, "message_folders")? {
                // V15's temporary index deliberately left numeric IDs
                // unconstrained so mailbox-scoped IMAP UIDs could coexist.
                // Normalize any duplicates before installing the final,
                // provider-aware constraints. For IMAP, remove only the
                // duplicate mailbox relation and delete/merge the message only
                // when that leaves it without any mailbox relation.
                tx.execute_batch(
                    "CREATE TEMP TABLE v16_message_dedup (
                         loser_id TEXT PRIMARY KEY,
                         winner_id TEXT NOT NULL
                     );
                     INSERT INTO v16_message_dedup (loser_id, winner_id)
                     SELECT id, winner_id
                     FROM (
                         SELECT
                             message.id AS id,
                             FIRST_VALUE(message.id) OVER (
                                 PARTITION BY message.account_id, message.remote_id
                                 ORDER BY message.updated_at DESC,
                                          message.created_at DESC,
                                          message.id DESC
                             ) AS winner_id
                         FROM messages AS message
                         JOIN accounts AS account ON account.id = message.account_id
                         WHERE message.is_deleted = 0
                           AND message.remote_id != ''
                           AND account.provider != 'imap'
                     )
                     WHERE id != winner_id;

                     CREATE TEMP TABLE v16_imap_relation_dedup (
                         loser_id TEXT NOT NULL,
                         winner_id TEXT NOT NULL,
                         folder_id TEXT NOT NULL,
                         PRIMARY KEY (loser_id, folder_id)
                     );
                     INSERT INTO v16_imap_relation_dedup
                         (loser_id, winner_id, folder_id)
                     SELECT message_id, winner_id, folder_id
                     FROM (
                         SELECT
                             message.id AS message_id,
                             relation.folder_id AS folder_id,
                             FIRST_VALUE(message.id) OVER (
                                 PARTITION BY message.account_id,
                                              relation.folder_id,
                                              message.remote_id
                                 ORDER BY message.updated_at DESC,
                                          message.created_at DESC,
                                          message.id DESC
                             ) AS winner_id
                         FROM messages AS message
                         JOIN accounts AS account ON account.id = message.account_id
                         JOIN message_folders AS relation
                           ON relation.message_id = message.id
                         WHERE message.is_deleted = 0
                           AND message.remote_id != ''
                           AND account.provider = 'imap'
                     )
                     WHERE message_id != winner_id;

                     DELETE FROM message_folders
                     WHERE EXISTS (
                         SELECT 1
                         FROM v16_imap_relation_dedup AS dedup
                         WHERE dedup.loser_id = message_folders.message_id
                           AND dedup.folder_id = message_folders.folder_id
                     );

                     INSERT OR IGNORE INTO v16_message_dedup (loser_id, winner_id)
                     SELECT relation.loser_id, MAX(relation.winner_id)
                     FROM v16_imap_relation_dedup AS relation
                     WHERE NOT EXISTS (
                         SELECT 1 FROM message_folders
                         WHERE message_id = relation.loser_id
                     )
                     GROUP BY relation.loser_id;",
                )
                .map_err(|e| PebbleError::Storage(format!("Migration V16 failed: {e}")))?;

                if table_exists(&tx, "message_folders")? {
                    tx.execute_batch(
                        "INSERT OR IGNORE INTO message_folders (message_id, folder_id)
                         SELECT dedup.winner_id, relation.folder_id
                         FROM message_folders AS relation
                         JOIN v16_message_dedup AS dedup
                           ON dedup.loser_id = relation.message_id;",
                    )?;
                }
                if table_exists(&tx, "attachments")? {
                    tx.execute_batch(
                        "UPDATE attachments
                         SET message_id = (
                             SELECT winner_id FROM v16_message_dedup
                             WHERE loser_id = attachments.message_id
                         )
                         WHERE message_id IN (SELECT loser_id FROM v16_message_dedup);",
                    )?;
                }
                if table_exists(&tx, "message_labels")? {
                    tx.execute_batch(
                        "INSERT OR IGNORE INTO message_labels (message_id, label_id)
                         SELECT dedup.winner_id, relation.label_id
                         FROM message_labels AS relation
                         JOIN v16_message_dedup AS dedup
                           ON dedup.loser_id = relation.message_id;",
                    )?;
                }
                if table_exists(&tx, "kanban_cards")? {
                    tx.execute_batch(
                        "INSERT INTO kanban_cards
                             (message_id, column_name, position, created_at, updated_at)
                         SELECT dedup.winner_id, relation.column_name, relation.position,
                                relation.created_at, relation.updated_at
                         FROM kanban_cards AS relation
                         JOIN v16_message_dedup AS dedup
                           ON dedup.loser_id = relation.message_id
                         WHERE 1
                         ON CONFLICT(message_id) DO UPDATE SET
                             column_name = excluded.column_name,
                             position = excluded.position,
                             created_at = excluded.created_at,
                             updated_at = excluded.updated_at
                         WHERE excluded.updated_at > kanban_cards.updated_at;",
                    )?;
                }
                if table_exists(&tx, "snoozed_messages")? {
                    tx.execute_batch(
                        "INSERT INTO snoozed_messages
                             (message_id, snoozed_at, unsnoozed_at, return_to)
                         SELECT dedup.winner_id, relation.snoozed_at,
                                relation.unsnoozed_at, relation.return_to
                         FROM snoozed_messages AS relation
                         JOIN v16_message_dedup AS dedup
                           ON dedup.loser_id = relation.message_id
                         WHERE 1
                         ON CONFLICT(message_id) DO UPDATE SET
                             snoozed_at = excluded.snoozed_at,
                             unsnoozed_at = excluded.unsnoozed_at,
                             return_to = excluded.return_to
                         WHERE excluded.snoozed_at > snoozed_messages.snoozed_at;",
                    )?;
                }
                if table_exists(&tx, "search_pending")? {
                    tx.execute_batch(
                        "INSERT INTO search_pending (message_id, operation, created_at)
                         SELECT dedup.winner_id, 'index', MAX(relation.created_at)
                         FROM search_pending AS relation
                         JOIN v16_message_dedup AS dedup
                           ON dedup.loser_id = relation.message_id
                         GROUP BY dedup.winner_id
                         ON CONFLICT(message_id) DO UPDATE SET
                             operation = 'index',
                             created_at = MAX(search_pending.created_at, excluded.created_at);
                         UPDATE search_pending
                         SET operation = 'index'
                         WHERE message_id IN (SELECT winner_id FROM v16_message_dedup);
                         DELETE FROM search_pending
                         WHERE message_id IN (SELECT loser_id FROM v16_message_dedup);",
                    )?;
                }
                if table_exists(&tx, "pending_mail_ops")? {
                    tx.execute_batch(
                        "UPDATE pending_mail_ops
                         SET message_id = (
                             SELECT winner_id FROM v16_message_dedup
                             WHERE loser_id = pending_mail_ops.message_id
                         )
                         WHERE message_id IN (SELECT loser_id FROM v16_message_dedup);",
                    )?;
                }

                tx.execute_batch(
                    "DELETE FROM messages
                     WHERE id IN (SELECT loser_id FROM v16_message_dedup);
                     DROP TABLE v16_imap_relation_dedup;
                     DROP TABLE v16_message_dedup;",
                )
                .map_err(|e| PebbleError::Storage(format!("Migration V16 failed: {e}")))?;

                tx.execute_batch(
                    "CREATE TRIGGER IF NOT EXISTS trg_messages_non_imap_remote_unique_insert
                     BEFORE INSERT ON messages
                     WHEN NEW.is_deleted = 0
                      AND NEW.remote_id != ''
                      AND COALESCE((SELECT provider FROM accounts WHERE id = NEW.account_id), '') != 'imap'
                      AND EXISTS (
                          SELECT 1 FROM messages existing
                          WHERE existing.account_id = NEW.account_id
                            AND existing.remote_id = NEW.remote_id
                            AND existing.is_deleted = 0
                      )
                     BEGIN
                         SELECT RAISE(ABORT, 'duplicate live non-IMAP remote_id');
                     END;

                     CREATE TRIGGER IF NOT EXISTS trg_messages_non_imap_remote_unique_update
                     BEFORE UPDATE OF account_id, remote_id, is_deleted ON messages
                     WHEN NEW.is_deleted = 0
                      AND NEW.remote_id != ''
                      AND COALESCE((SELECT provider FROM accounts WHERE id = NEW.account_id), '') != 'imap'
                      AND EXISTS (
                          SELECT 1 FROM messages existing
                          WHERE existing.id != OLD.id
                            AND existing.account_id = NEW.account_id
                            AND existing.remote_id = NEW.remote_id
                            AND existing.is_deleted = 0
                      )
                     BEGIN
                         SELECT RAISE(ABORT, 'duplicate live non-IMAP remote_id');
                     END;

                     CREATE TRIGGER IF NOT EXISTS trg_message_folders_imap_uid_unique_insert
                     BEFORE INSERT ON message_folders
                     WHEN COALESCE((
                              SELECT account.provider
                              FROM messages message
                              JOIN accounts account ON account.id = message.account_id
                              WHERE message.id = NEW.message_id
                          ), '') = 'imap'
                      AND COALESCE((SELECT is_deleted FROM messages WHERE id = NEW.message_id), 1) = 0
                      AND COALESCE((SELECT remote_id FROM messages WHERE id = NEW.message_id), '') != ''
                      AND EXISTS (
                          SELECT 1
                          FROM message_folders relation
                          JOIN messages existing ON existing.id = relation.message_id
                          WHERE relation.folder_id = NEW.folder_id
                            AND existing.id != NEW.message_id
                            AND existing.is_deleted = 0
                            AND existing.account_id = (SELECT account_id FROM messages WHERE id = NEW.message_id)
                            AND existing.remote_id = (SELECT remote_id FROM messages WHERE id = NEW.message_id)
                      )
                     BEGIN
                         SELECT RAISE(ABORT, 'duplicate live IMAP UID in folder');
                     END;

                     CREATE TRIGGER IF NOT EXISTS trg_message_folders_imap_uid_unique_update
                     BEFORE UPDATE OF message_id, folder_id ON message_folders
                     WHEN COALESCE((
                              SELECT account.provider
                              FROM messages message
                              JOIN accounts account ON account.id = message.account_id
                              WHERE message.id = NEW.message_id
                          ), '') = 'imap'
                      AND COALESCE((SELECT is_deleted FROM messages WHERE id = NEW.message_id), 1) = 0
                      AND COALESCE((SELECT remote_id FROM messages WHERE id = NEW.message_id), '') != ''
                      AND EXISTS (
                          SELECT 1
                          FROM message_folders relation
                          JOIN messages existing ON existing.id = relation.message_id
                          WHERE relation.folder_id = NEW.folder_id
                            AND existing.id != NEW.message_id
                            AND existing.is_deleted = 0
                            AND existing.account_id = (SELECT account_id FROM messages WHERE id = NEW.message_id)
                            AND existing.remote_id = (SELECT remote_id FROM messages WHERE id = NEW.message_id)
                      )
                     BEGIN
                         SELECT RAISE(ABORT, 'duplicate live IMAP UID in folder');
                     END;

                     CREATE TRIGGER IF NOT EXISTS trg_messages_imap_uid_unique_update
                     BEFORE UPDATE OF account_id, remote_id, is_deleted ON messages
                     WHEN NEW.is_deleted = 0
                      AND NEW.remote_id != ''
                      AND COALESCE((SELECT provider FROM accounts WHERE id = NEW.account_id), '') = 'imap'
                      AND EXISTS (
                          SELECT 1
                          FROM message_folders own_relation
                          JOIN message_folders other_relation
                            ON other_relation.folder_id = own_relation.folder_id
                           AND other_relation.message_id != OLD.id
                          JOIN messages existing ON existing.id = other_relation.message_id
                          WHERE own_relation.message_id = OLD.id
                            AND existing.is_deleted = 0
                            AND existing.account_id = NEW.account_id
                            AND existing.remote_id = NEW.remote_id
                      )
                     BEGIN
                         SELECT RAISE(ABORT, 'duplicate live IMAP UID in folder');
                     END;",
                )
                .map_err(|e| PebbleError::Storage(format!("Migration V16 failed: {e}")))?;
            } else {
                // Preserve lightweight migration-test schemas that omit
                // account/folder metadata. Production never takes this path.
                tx.execute_batch(
                    "CREATE UNIQUE INDEX IF NOT EXISTS idx_messages_account_remote_unique
                       ON messages(account_id, remote_id)
                       WHERE is_deleted = 0
                         AND remote_id != ''
                         AND remote_id GLOB '*[^0-9]*';",
                )
                .map_err(|e| PebbleError::Storage(format!("Migration V16 failed: {e}")))?;
            }

            // This is intentionally non-unique: it keeps lookups by provider
            // ID indexed while the triggers above enforce the correct scope.
            tx.execute_batch(
                "CREATE INDEX IF NOT EXISTS idx_messages_account_remote
                   ON messages(account_id, remote_id);",
            )
            .map_err(|e| PebbleError::Storage(format!("Migration V16 failed: {e}")))?;
        }
        set_schema_version(&tx, 16)?;
        tx.commit()
            .map_err(|e| PebbleError::Storage(format!("Migration V16 commit failed: {e}")))?;
    }

    // V17: profile-level address book and hidden recent-contact suggestions.
    if version < 17 {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| PebbleError::Storage(format!("Migration V17 begin failed: {e}")))?;
        tx.execute_batch(
            "CREATE TABLE contacts (
                 id TEXT PRIMARY KEY,
                 display_name TEXT NOT NULL DEFAULT '',
                 notes TEXT NOT NULL DEFAULT '',
                 is_favorite INTEGER NOT NULL DEFAULT 0 CHECK(is_favorite IN (0, 1)),
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL
             );
             CREATE TABLE contact_emails (
                 id TEXT PRIMARY KEY,
                 contact_id TEXT NOT NULL REFERENCES contacts(id) ON DELETE CASCADE,
                 address TEXT NOT NULL,
                 normalized_address TEXT NOT NULL COLLATE NOCASE,
                 label TEXT NOT NULL DEFAULT 'other'
                     CHECK(label IN ('work', 'personal', 'other')),
                 is_primary INTEGER NOT NULL DEFAULT 0 CHECK(is_primary IN (0, 1)),
                 created_at INTEGER NOT NULL,
                 UNIQUE(normalized_address)
             );
             CREATE INDEX idx_contact_emails_contact
                 ON contact_emails(contact_id);
             CREATE UNIQUE INDEX idx_contact_emails_one_primary
                 ON contact_emails(contact_id) WHERE is_primary = 1;
             CREATE TABLE contact_suggestion_suppressions (
                 normalized_address TEXT PRIMARY KEY COLLATE NOCASE,
                 created_at INTEGER NOT NULL
             );",
        )
        .map_err(|e| PebbleError::Storage(format!("Migration V17 failed: {e}")))?;
        set_schema_version(&tx, 17)?;
        tx.commit()
            .map_err(|e| PebbleError::Storage(format!("Migration V17 commit failed: {e}")))?;
    }

    if version < 18 {
        let tx = conn.unchecked_transaction()?;
        // Lightweight migration fixtures may intentionally omit accounts.
        if table_exists(&tx, "accounts")? {
            tx.execute_batch(
                "ALTER TABLE accounts ADD COLUMN account_label TEXT;
                ALTER TABLE accounts ADD COLUMN provider_display_name TEXT;
                ALTER TABLE accounts ADD COLUMN oauth_subject TEXT;",
            )?;
        }
        set_schema_version(&tx, 18)?;
        tx.commit()?;
    }

    if version < 19 {
        let tx = conn.unchecked_transaction()?;
        tx.execute_batch(
            "CREATE TABLE scheduled_sends (
                id TEXT PRIMARY KEY,
                account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
                kind TEXT NOT NULL CHECK(kind IN ('scheduled', 'undo')),
                due_at INTEGER NOT NULL,
                state TEXT NOT NULL CHECK(state IN ('pending', 'dispatching', 'review', 'draft', 'sent')),
                payload BLOB NOT NULL,
                revision INTEGER NOT NULL DEFAULT 0,
                updated_at INTEGER NOT NULL
             );
             CREATE INDEX scheduled_sends_due ON scheduled_sends(state, due_at);",
        )?;
        set_schema_version(&tx, CURRENT_VERSION)?;
        tx.commit()?;
    }

    Ok(())
}

const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL,
    display_name TEXT NOT NULL DEFAULT '',
    color TEXT,
    provider TEXT NOT NULL CHECK(provider IN ('imap', 'pop3', 'gmail', 'outlook')),
    auth_data BLOB,
    sync_state TEXT,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS folders (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    remote_id TEXT NOT NULL,
    name TEXT NOT NULL,
    folder_type TEXT NOT NULL CHECK(folder_type IN ('folder', 'label', 'category')),
    role TEXT CHECK(role IN ('inbox', 'sent', 'drafts', 'trash', 'archive', 'spam')),
    parent_id TEXT,
    color TEXT,
    is_system INTEGER NOT NULL DEFAULT 0,
    sort_order INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_folders_account ON folders(account_id);

CREATE TABLE IF NOT EXISTS messages (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    remote_id TEXT NOT NULL,
    message_id_header TEXT,
    in_reply_to TEXT,
    references_header TEXT,
    thread_id TEXT,
    subject TEXT NOT NULL DEFAULT '',
    snippet TEXT NOT NULL DEFAULT '',
    from_address TEXT NOT NULL DEFAULT '',
    from_name TEXT NOT NULL DEFAULT '',
    to_list TEXT NOT NULL DEFAULT '[]',
    cc_list TEXT NOT NULL DEFAULT '[]',
    bcc_list TEXT NOT NULL DEFAULT '[]',
    body_text TEXT NOT NULL DEFAULT '',
    body_html_raw TEXT NOT NULL DEFAULT '',
    has_attachments INTEGER NOT NULL DEFAULT 0,
    is_read INTEGER NOT NULL DEFAULT 0,
    is_starred INTEGER NOT NULL DEFAULT 0,
    is_draft INTEGER NOT NULL DEFAULT 0,
    date INTEGER NOT NULL,
    raw_headers TEXT,
    remote_version TEXT,
    is_deleted INTEGER NOT NULL DEFAULT 0,
    deleted_at INTEGER,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_messages_account ON messages(account_id);
CREATE INDEX IF NOT EXISTS idx_messages_thread ON messages(thread_id);
CREATE INDEX IF NOT EXISTS idx_messages_date ON messages(date);
CREATE INDEX IF NOT EXISTS idx_messages_message_id_header ON messages(message_id_header);

CREATE TABLE IF NOT EXISTS message_folders (
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    folder_id TEXT NOT NULL REFERENCES folders(id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, folder_id)
);

CREATE TABLE IF NOT EXISTS attachments (
    id TEXT PRIMARY KEY,
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    filename TEXT NOT NULL DEFAULT '',
    mime_type TEXT NOT NULL DEFAULT '',
    size INTEGER NOT NULL DEFAULT 0,
    local_path TEXT,
    content_id TEXT,
    is_inline INTEGER NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_attachments_message ON attachments(message_id);

CREATE TABLE IF NOT EXISTS labels (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    color TEXT NOT NULL DEFAULT '#808080',
    is_system INTEGER NOT NULL DEFAULT 0,
    rule_id TEXT
);

CREATE TABLE IF NOT EXISTS message_labels (
    message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE,
    label_id TEXT NOT NULL REFERENCES labels(id) ON DELETE CASCADE,
    PRIMARY KEY (message_id, label_id)
);

CREATE TABLE IF NOT EXISTS kanban_cards (
    message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    column_name TEXT NOT NULL CHECK(column_name IN ('todo', 'waiting', 'done')),
    position INTEGER NOT NULL DEFAULT 0,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS snoozed_messages (
    message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
    snoozed_at INTEGER NOT NULL,
    unsnoozed_at INTEGER NOT NULL,
    return_to TEXT NOT NULL DEFAULT 'inbox'
);

CREATE TABLE IF NOT EXISTS trusted_senders (
    account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    email TEXT NOT NULL,
    trust_type TEXT NOT NULL CHECK(trust_type IN ('images', 'all')),
    created_at INTEGER NOT NULL,
    PRIMARY KEY (account_id, email)
);

CREATE TABLE IF NOT EXISTS rules (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    priority INTEGER NOT NULL DEFAULT 0,
    conditions TEXT NOT NULL DEFAULT '{}',
    actions TEXT NOT NULL DEFAULT '[]',
    is_enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);

CREATE TABLE IF NOT EXISTS translate_config (
    id TEXT PRIMARY KEY DEFAULT 'active',
    provider_type TEXT NOT NULL CHECK(provider_type IN ('deeplx', 'deepl', 'generic_api', 'llm')),
    config TEXT NOT NULL DEFAULT '{}',
    is_enabled INTEGER NOT NULL DEFAULT 1,
    created_at INTEGER NOT NULL,
    updated_at INTEGER NOT NULL
);
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn migration_v18_preserves_existing_identity_and_rolls_back_on_failure() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE accounts (id TEXT PRIMARY KEY, email TEXT, display_name TEXT, auth_data BLOB);
             INSERT INTO accounts VALUES ('a', 'old@example.com', 'Original Name', X'1234');
             PRAGMA user_version=17;",
        ).unwrap();
        run_migrations(&conn).unwrap();
        let values: (String, String, Vec<u8>, Option<String>, Option<String>, Option<String>) = conn.query_row(
            "SELECT email, display_name, auth_data, account_label, provider_display_name, oauth_subject FROM accounts WHERE id='a'",
            [], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?, row.get(4)?, row.get(5)?)),
        ).unwrap();
        assert_eq!(
            values,
            (
                "old@example.com".into(),
                "Original Name".into(),
                vec![0x12, 0x34],
                None,
                None,
                None
            )
        );

        let broken = Connection::open_in_memory().unwrap();
        broken.execute_batch("CREATE TABLE accounts (id TEXT, provider_display_name TEXT); PRAGMA user_version=17;").unwrap();
        assert!(run_migrations(&broken).is_err());
        assert_eq!(
            broken
                .pragma_query_value(None, "user_version", |row| row.get::<_, u32>(0))
                .unwrap(),
            17
        );
        assert!(
            broken
                .prepare("SELECT account_label FROM accounts")
                .is_err(),
            "earlier ALTER must roll back when a later ALTER fails"
        );
    }

    #[test]
    fn migration_v17_creates_contact_tables() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA user_version=14;")
            .unwrap();

        run_migrations(&conn).unwrap();

        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, CURRENT_VERSION);

        conn.execute_batch(
            "INSERT INTO contacts
                (id, display_name, notes, is_favorite, created_at, updated_at)
             VALUES ('contact-1', 'Alice', '', 0, 1, 1);
             INSERT INTO contact_emails
                (id, contact_id, address, normalized_address, label, is_primary, created_at)
             VALUES ('email-1', 'contact-1', 'Alice@Example.com', 'alice@example.com', 'work', 1, 1);
             INSERT INTO contact_suggestion_suppressions (normalized_address, created_at)
             VALUES ('hidden@example.com', 1);",
        )
        .expect("V17 contact tables should accept valid rows");

        let duplicate_address = conn.execute(
            "INSERT INTO contact_emails
                (id, contact_id, address, normalized_address, label, is_primary, created_at)
             VALUES ('email-2', 'contact-1', 'ALICE@example.com', 'ALICE@EXAMPLE.COM', 'other', 0, 1)",
            [],
        );
        assert!(
            duplicate_address.is_err(),
            "normalized email addresses must be unique case-insensitively"
        );

        let second_primary = conn.execute(
            "INSERT INTO contact_emails
                (id, contact_id, address, normalized_address, label, is_primary, created_at)
             VALUES ('email-3', 'contact-1', 'other@example.com', 'other@example.com', 'personal', 1, 1)",
            [],
        );
        assert!(
            second_primary.is_err(),
            "a contact must not have more than one primary email"
        );

        conn.execute("DELETE FROM contacts WHERE id = 'contact-1'", [])
            .unwrap();
        let remaining_emails: i64 = conn
            .query_row("SELECT COUNT(*) FROM contact_emails", [], |row| row.get(0))
            .unwrap();
        assert_eq!(
            remaining_emails, 0,
            "contact emails should cascade on delete"
        );
    }

    #[test]
    fn migration_v11_adds_account_color_and_sets_schema_version() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE accounts (
                id TEXT PRIMARY KEY,
                email TEXT NOT NULL,
                display_name TEXT NOT NULL DEFAULT '',
                provider TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            PRAGMA user_version = 10;",
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, CURRENT_VERSION);
        conn.prepare("SELECT color FROM accounts LIMIT 0")
            .expect("accounts.color should exist after V11");
    }

    #[test]
    fn failed_v12_migration_leaves_schema_version_at_v11() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE accounts (
                id TEXT PRIMARY KEY,
                email TEXT NOT NULL,
                display_name TEXT NOT NULL DEFAULT '',
                provider TEXT NOT NULL CHECK(provider IN ('imap', 'gmail', 'outlook')),
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE accounts_new (id TEXT PRIMARY KEY);
            PRAGMA user_version = 10;",
        )
        .unwrap();

        assert!(run_migrations(&conn).is_err());

        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(
            version, 11,
            "completed V11 must not claim later migrations ran"
        );
    }

    #[test]
    fn migration_v14_dedups_live_rows_and_enforces_partial_unique_index() {
        let conn = Connection::open_in_memory().unwrap();
        // Minimal messages shape at the V13 boundary: only the columns V14
        // touches. run_migrations from user_version=13 runs just the V14 block.
        conn.execute_batch(
            "CREATE TABLE messages (
                id TEXT PRIMARY KEY,
                account_id TEXT NOT NULL,
                remote_id TEXT NOT NULL,
                body_text TEXT NOT NULL,
                is_deleted INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE attachments (
                id TEXT PRIMARY KEY,
                message_id TEXT NOT NULL REFERENCES messages(id) ON DELETE CASCADE
            );
            CREATE TABLE message_folders (
                message_id TEXT NOT NULL,
                folder_id TEXT NOT NULL,
                PRIMARY KEY (message_id, folder_id)
            );
            CREATE TABLE kanban_cards (
                message_id TEXT PRIMARY KEY,
                column_name TEXT NOT NULL,
                position INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE snoozed_messages (
                message_id TEXT PRIMARY KEY,
                snoozed_at INTEGER NOT NULL,
                unsnoozed_at INTEGER NOT NULL,
                return_to TEXT NOT NULL
            );
            CREATE TABLE search_pending (
                message_id TEXT PRIMARY KEY,
                operation TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE INDEX idx_messages_account_remote ON messages(account_id, remote_id);
            PRAGMA user_version = 13;",
        )
        .unwrap();

        // Two LIVE duplicates of the same remote message, one tombstone for it,
        // and one unrelated live row.
        conn.execute_batch(
            "INSERT INTO messages
                (id, account_id, remote_id, body_text, is_deleted, created_at, updated_at) VALUES
                ('m-dup-a', 'acct', 'UID:42', 'stale body', 0, 1, 1),
                ('m-dup-b', 'acct', 'UID:42', 'fresh body', 0, 2, 2),
                ('m-tomb',  'acct', 'UID:42', 'tombstone',  1, 1, 1),
                ('m-other', 'acct', 'UID:99', 'other',      0, 1, 1),
                ('m-local-a','acct', '',       'draft one',  0, 1, 1),
                ('m-local-b','acct', '',       'draft two',  0, 2, 2);
             INSERT INTO attachments (id, message_id) VALUES
                ('att-old', 'm-dup-a'),
                ('att-new', 'm-dup-b');
             INSERT INTO message_folders (message_id, folder_id) VALUES
                ('m-dup-a', 'archive'),
                ('m-dup-b', 'inbox');
             INSERT INTO kanban_cards
                (message_id, column_name, position, created_at, updated_at) VALUES
                ('m-dup-a', 'done', 9, 1, 20),
                ('m-dup-b', 'todo', 1, 1, 10);
             INSERT INTO snoozed_messages
                (message_id, snoozed_at, unsnoozed_at, return_to) VALUES
                ('m-dup-a', 30, 60, 'archive'),
                ('m-dup-b', 10, 40, 'inbox');
             INSERT INTO search_pending (message_id, operation, created_at) VALUES
                ('m-dup-a', 'index', 20),
                ('m-dup-b', 'remove', 10);",
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, CURRENT_VERSION);

        // Duplicate live rows collapse to one; the tombstone survives; the
        // unrelated live row is untouched.
        let live_42: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE account_id='acct' AND remote_id='UID:42' AND is_deleted=0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(live_42, 1, "duplicate live rows should be collapsed to one");

        let retained: (String, String) = conn
            .query_row(
                "SELECT id, body_text FROM messages
                 WHERE account_id='acct' AND remote_id='UID:42' AND is_deleted=0",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(retained, ("m-dup-b".to_string(), "fresh body".to_string()));
        let retained_attachments: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM attachments WHERE message_id='m-dup-b'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            retained_attachments, 2,
            "dedup must merge attachments into the winner"
        );
        let retained_folders: Vec<String> = {
            let mut stmt = conn
                .prepare(
                    "SELECT folder_id FROM message_folders
                     WHERE message_id='m-dup-b' ORDER BY folder_id",
                )
                .unwrap();
            stmt.query_map([], |row| row.get(0))
                .unwrap()
                .collect::<std::result::Result<_, _>>()
                .unwrap()
        };
        assert_eq!(
            retained_folders,
            vec!["archive".to_string(), "inbox".to_string()],
            "folder relations are a set and must be unioned during dedup"
        );
        let retained_card: (String, i64, i64) = conn
            .query_row(
                "SELECT column_name, position, updated_at
                 FROM kanban_cards WHERE message_id='m-dup-b'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            retained_card,
            ("done".to_string(), 9, 20),
            "newest kanban relation must survive even when it belongs to the loser"
        );
        let retained_snooze: (i64, i64, String) = conn
            .query_row(
                "SELECT snoozed_at, unsnoozed_at, return_to
                 FROM snoozed_messages WHERE message_id='m-dup-b'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
            )
            .unwrap();
        assert_eq!(
            retained_snooze,
            (30, 60, "archive".to_string()),
            "latest snooze relation must survive even when it belongs to the loser"
        );
        let retained_search: (String, i64) = conn
            .query_row(
                "SELECT operation, created_at
                 FROM search_pending WHERE message_id='m-dup-b'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            retained_search,
            ("index".to_string(), 20),
            "dedup must re-index the winner at the newest pending timestamp"
        );
        let stale_search: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM search_pending WHERE message_id='m-dup-a'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(stale_search, 0, "loser search work must be removed");
        let local_drafts: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE account_id='acct' AND remote_id='' AND is_deleted=0",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(
            local_drafts, 2,
            "local drafts without remote IDs are not duplicates"
        );

        let tomb_survives: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages WHERE id='m-tomb'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(tomb_survives, 1, "tombstone must not be removed by dedup");

        // Partial unique index: a second LIVE row with the same key is rejected,
        // but an additional tombstone (is_deleted=1) is still allowed.
        let dup_live = conn.execute(
            "INSERT INTO messages
                (id, account_id, remote_id, body_text, is_deleted, created_at, updated_at)
             VALUES ('m-extra','acct','UID:42','extra',0,3,3)",
            [],
        );
        assert!(
            dup_live.is_err(),
            "second live row must be rejected by the partial unique index"
        );

        let dup_tomb = conn.execute(
            "INSERT INTO messages
                (id, account_id, remote_id, body_text, is_deleted, created_at, updated_at)
             VALUES ('m-tomb2','acct','UID:42','tombstone 2',1,3,3)",
            [],
        );
        assert!(
            dup_tomb.is_ok(),
            "additional tombstone must be allowed alongside one live row"
        );
        let another_local = conn.execute(
            "INSERT INTO messages
                (id, account_id, remote_id, body_text, is_deleted, created_at, updated_at)
             VALUES ('m-local-c','acct','','draft three',0,3,3)",
            [],
        );
        assert!(
            another_local.is_ok(),
            "multiple local drafts must remain valid"
        );
    }

    #[test]
    fn migration_after_v14_replaces_index_that_blocked_multiple_local_drafts() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE messages (
                id TEXT PRIMARY KEY,
                account_id TEXT NOT NULL,
                remote_id TEXT NOT NULL,
                is_deleted INTEGER NOT NULL DEFAULT 0
            );
            INSERT INTO messages (id, account_id, remote_id, is_deleted)
                VALUES ('draft-a', 'acct', '', 0);
            CREATE UNIQUE INDEX idx_messages_account_remote_unique
                ON messages(account_id, remote_id) WHERE is_deleted = 0;
            PRAGMA user_version = 14;",
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        conn.execute(
            "INSERT INTO messages (id, account_id, remote_id, is_deleted)
             VALUES ('draft-b', 'acct', '', 0)",
            [],
        )
        .expect("the corrected index must allow more than one local draft");
    }

    #[test]
    fn migration_from_v13_preserves_mailbox_scoped_imap_uids() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE accounts (
                id TEXT PRIMARY KEY,
                provider TEXT NOT NULL
            );
            CREATE TABLE messages (
                id TEXT PRIMARY KEY,
                account_id TEXT NOT NULL,
                remote_id TEXT NOT NULL,
                is_deleted INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE message_folders (
                message_id TEXT NOT NULL,
                folder_id TEXT NOT NULL,
                PRIMARY KEY (message_id, folder_id)
            );
            INSERT INTO accounts (id, provider) VALUES
                ('imap-account', 'imap'),
                ('gmail-account', 'gmail');
            INSERT INTO messages
                (id, account_id, remote_id, is_deleted, created_at, updated_at) VALUES
                ('imap-inbox', 'imap-account', '42', 0, 1, 1),
                ('imap-sent', 'imap-account', '42', 0, 1, 1),
                ('gmail-old', 'gmail-account', '7', 0, 1, 1),
                ('gmail-new', 'gmail-account', '7', 0, 2, 2);
            INSERT INTO message_folders (message_id, folder_id) VALUES
                ('imap-inbox', 'folder-inbox'),
                ('imap-sent', 'folder-sent');
            PRAGMA user_version = 13;",
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        let imap_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages
                 WHERE account_id = 'imap-account' AND remote_id = '42'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        let gmail_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages
                 WHERE account_id = 'gmail-account' AND remote_id = '7'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(imap_rows, 2);
        assert_eq!(gmail_rows, 1);

        conn.execute(
            "INSERT INTO messages
                (id, account_id, remote_id, is_deleted, created_at, updated_at)
             VALUES ('imap-archive', 'imap-account', '42', 0, 3, 3)",
            [],
        )
        .unwrap();
        conn.execute(
            "INSERT INTO message_folders (message_id, folder_id)
             VALUES ('imap-archive', 'folder-archive')",
            [],
        )
        .expect("the same UID must remain valid in another IMAP mailbox");

        conn.execute(
            "INSERT INTO messages
                (id, account_id, remote_id, is_deleted, created_at, updated_at)
             VALUES ('imap-inbox-duplicate', 'imap-account', '42', 0, 4, 4)",
            [],
        )
        .unwrap();
        let same_mailbox_duplicate = conn.execute(
            "INSERT INTO message_folders (message_id, folder_id)
             VALUES ('imap-inbox-duplicate', 'folder-inbox')",
            [],
        );
        assert!(same_mailbox_duplicate.is_err());

        let gmail_duplicate = conn.execute(
            "INSERT INTO messages
                (id, account_id, remote_id, is_deleted, created_at, updated_at)
             VALUES ('gmail-duplicate', 'gmail-account', '7', 0, 5, 5)",
            [],
        );
        assert!(gmail_duplicate.is_err());
    }

    #[test]
    fn migration_v16_merges_existing_scoped_duplicates_and_keeps_lookup_indexed() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE accounts (
                id TEXT PRIMARY KEY,
                provider TEXT NOT NULL
            );
            CREATE TABLE messages (
                id TEXT PRIMARY KEY,
                account_id TEXT NOT NULL,
                remote_id TEXT NOT NULL,
                is_deleted INTEGER NOT NULL DEFAULT 0,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE message_folders (
                message_id TEXT NOT NULL,
                folder_id TEXT NOT NULL,
                PRIMARY KEY (message_id, folder_id)
            );
            CREATE TABLE attachments (
                id TEXT PRIMARY KEY,
                message_id TEXT NOT NULL
            );
            CREATE TABLE kanban_cards (
                message_id TEXT PRIMARY KEY,
                column_name TEXT NOT NULL,
                position INTEGER NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE snoozed_messages (
                message_id TEXT PRIMARY KEY,
                snoozed_at INTEGER NOT NULL,
                unsnoozed_at INTEGER NOT NULL,
                return_to TEXT NOT NULL
            );
            CREATE TABLE search_pending (
                message_id TEXT PRIMARY KEY,
                operation TEXT NOT NULL,
                created_at INTEGER NOT NULL
            );
            CREATE TABLE pending_mail_ops (
                id TEXT PRIMARY KEY,
                message_id TEXT NOT NULL
            );
            CREATE UNIQUE INDEX idx_messages_account_remote_unique
                ON messages(account_id, remote_id)
                WHERE is_deleted = 0
                  AND remote_id != ''
                  AND remote_id GLOB '*[^0-9]*';

            INSERT INTO accounts (id, provider) VALUES
                ('imap-account', 'imap'),
                ('gmail-account', 'gmail');
            INSERT INTO messages
                (id, account_id, remote_id, is_deleted, created_at, updated_at) VALUES
                ('imap-old', 'imap-account', '42', 0, 1, 1),
                ('imap-new', 'imap-account', '42', 0, 2, 2),
                ('imap-other-folder', 'imap-account', '42', 0, 3, 3),
                ('gmail-old', 'gmail-account', '7', 0, 1, 1),
                ('gmail-new', 'gmail-account', '7', 0, 2, 2);
            INSERT INTO message_folders (message_id, folder_id) VALUES
                ('imap-old', 'inbox'),
                ('imap-new', 'inbox'),
                ('imap-other-folder', 'sent');
            INSERT INTO attachments (id, message_id) VALUES
                ('attachment-old', 'imap-old');
            INSERT INTO kanban_cards
                (message_id, column_name, position, created_at, updated_at) VALUES
                ('imap-old', 'done', 9, 1, 30),
                ('imap-new', 'todo', 1, 1, 10);
            INSERT INTO snoozed_messages
                (message_id, snoozed_at, unsnoozed_at, return_to) VALUES
                ('imap-old', 30, 60, 'archive'),
                ('imap-new', 10, 40, 'inbox');
            INSERT INTO search_pending (message_id, operation, created_at) VALUES
                ('imap-old', 'index', 20),
                ('imap-new', 'remove', 10);
            INSERT INTO pending_mail_ops (id, message_id) VALUES
                ('pending-old', 'imap-old');
            PRAGMA user_version = 15;",
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        let surviving_imap_ids: Vec<String> = {
            let mut statement = conn
                .prepare(
                    "SELECT id FROM messages
                     WHERE account_id='imap-account' AND remote_id='42'
                     ORDER BY id",
                )
                .unwrap();
            statement
                .query_map([], |row| row.get(0))
                .unwrap()
                .collect::<std::result::Result<_, _>>()
                .unwrap()
        };
        assert_eq!(
            surviving_imap_ids,
            vec!["imap-new".to_string(), "imap-other-folder".to_string()]
        );
        let inbox_owner: String = conn
            .query_row(
                "SELECT message_id FROM message_folders WHERE folder_id='inbox'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(inbox_owner, "imap-new");
        let attachment_owner: String = conn
            .query_row(
                "SELECT message_id FROM attachments WHERE id='attachment-old'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(attachment_owner, "imap-new");
        let pending_owner: String = conn
            .query_row(
                "SELECT message_id FROM pending_mail_ops WHERE id='pending-old'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(pending_owner, "imap-new");
        let card: (String, i64) = conn
            .query_row(
                "SELECT column_name, updated_at FROM kanban_cards WHERE message_id='imap-new'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(card, ("done".to_string(), 30));
        let snooze: (i64, String) = conn
            .query_row(
                "SELECT snoozed_at, return_to FROM snoozed_messages WHERE message_id='imap-new'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(snooze, (30, "archive".to_string()));
        let search: (String, i64) = conn
            .query_row(
                "SELECT operation, created_at FROM search_pending WHERE message_id='imap-new'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(search, ("index".to_string(), 20));
        let gmail_rows: i64 = conn
            .query_row(
                "SELECT COUNT(*) FROM messages
                 WHERE account_id='gmail-account' AND remote_id='7'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(gmail_rows, 1, "numeric non-IMAP IDs remain account-scoped");

        conn.execute(
            "INSERT INTO messages
                (id, account_id, remote_id, is_deleted, created_at, updated_at)
             VALUES ('imap-duplicate', 'imap-account', '42', 0, 4, 4)",
            [],
        )
        .unwrap();
        assert!(conn
            .execute(
                "INSERT INTO message_folders (message_id, folder_id)
                 VALUES ('imap-duplicate', 'inbox')",
                [],
            )
            .is_err());
        conn.execute(
            "INSERT INTO message_folders (message_id, folder_id)
             VALUES ('imap-duplicate', 'archive')",
            [],
        )
        .expect("the same IMAP UID remains valid in a different mailbox");

        let account_remote_index: (String, i64) = conn
            .query_row(
                "SELECT name, \"unique\" FROM pragma_index_list('messages')
                 WHERE name='idx_messages_account_remote'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?)),
            )
            .unwrap();
        assert_eq!(
            account_remote_index,
            ("idx_messages_account_remote".into(), 0)
        );
        let query_plan: String = conn
            .query_row(
                "EXPLAIN QUERY PLAN
                 SELECT id FROM messages
                 WHERE account_id='imap-account' AND remote_id='42'",
                [],
                |row| row.get(3),
            )
            .unwrap();
        assert!(
            query_plan.contains("idx_messages_account_remote"),
            "lookup should use restored account/remote index: {query_plan}"
        );
    }

    #[test]
    fn migration_v11_backfills_existing_account_colors() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "CREATE TABLE accounts (
                id TEXT PRIMARY KEY,
                email TEXT NOT NULL,
                display_name TEXT NOT NULL DEFAULT '',
                provider TEXT NOT NULL,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            INSERT INTO accounts (id, email, display_name, provider, created_at, updated_at)
            VALUES
                ('account-1', 'one@example.com', 'One', 'gmail', 1, 1),
                ('account-2', 'two@example.com', 'Two', 'gmail', 2, 2);
            PRAGMA user_version = 10;",
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        let mut stmt = conn
            .prepare("SELECT color FROM accounts ORDER BY created_at ASC")
            .unwrap();
        let colors = stmt
            .query_map([], |row| row.get::<_, Option<String>>(0))
            .unwrap()
            .collect::<std::result::Result<Vec<_>, _>>()
            .unwrap();
        assert_eq!(
            colors,
            vec![Some("#0ea5e9".to_string()), Some("#22c55e".to_string())]
        );
    }

    #[test]
    fn migration_v12_allows_pop3_provider_without_breaking_foreign_keys() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys=ON;
            CREATE TABLE accounts (
                id TEXT PRIMARY KEY,
                email TEXT NOT NULL,
                display_name TEXT NOT NULL DEFAULT '',
                color TEXT,
                provider TEXT NOT NULL CHECK(provider IN ('imap', 'gmail', 'outlook')),
                auth_data BLOB,
                sync_state TEXT,
                created_at INTEGER NOT NULL,
                updated_at INTEGER NOT NULL
            );
            CREATE TABLE folders (
                id TEXT PRIMARY KEY,
                account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
                remote_id TEXT NOT NULL,
                name TEXT NOT NULL,
                folder_type TEXT NOT NULL CHECK(folder_type IN ('folder', 'label', 'category')),
                role TEXT CHECK(role IN ('inbox', 'sent', 'drafts', 'trash', 'archive', 'spam')),
                parent_id TEXT,
                color TEXT,
                is_system INTEGER NOT NULL DEFAULT 0,
                sort_order INTEGER NOT NULL DEFAULT 0
            );
            INSERT INTO accounts (id, email, display_name, color, provider, created_at, updated_at)
                VALUES ('account-1', 'one@example.com', 'One', '#0ea5e9', 'imap', 1, 1);
            INSERT INTO folders (id, account_id, remote_id, name, folder_type, role, is_system, sort_order)
                VALUES ('folder-1', 'account-1', 'INBOX', 'Inbox', 'folder', 'inbox', 1, 0);
            PRAGMA user_version = 11;",
        )
        .unwrap();

        run_migrations(&conn).unwrap();

        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, CURRENT_VERSION);
        conn.execute(
            "INSERT INTO accounts (id, email, display_name, provider, created_at, updated_at)
                VALUES ('account-2', 'two@example.com', 'Two', 'pop3', 2, 2)",
            [],
        )
        .expect("accounts.provider should accept pop3 after V12");
        let folder_account: String = conn
            .query_row(
                "SELECT account_id FROM folders WHERE id = 'folder-1'",
                [],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(folder_account, "account-1");
        let fk_issue = conn
            .query_row("PRAGMA foreign_key_check", [], |_| Ok(()))
            .optional()
            .unwrap();
        assert!(fk_issue.is_none());
    }

    #[test]
    fn migration_v12_fk_violation_rolls_back_version_and_restores_enforcement() {
        let conn = Connection::open_in_memory().unwrap();
        conn.execute_batch(
            "PRAGMA foreign_keys=OFF;
             CREATE TABLE accounts (
                 id TEXT PRIMARY KEY,
                 email TEXT NOT NULL,
                 display_name TEXT NOT NULL DEFAULT '',
                 color TEXT,
                 provider TEXT NOT NULL CHECK(provider IN ('imap', 'gmail', 'outlook')),
                 auth_data BLOB,
                 sync_state TEXT,
                 created_at INTEGER NOT NULL,
                 updated_at INTEGER NOT NULL
             );
             CREATE TABLE folders (
                 id TEXT PRIMARY KEY,
                 account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE
             );
             INSERT INTO accounts
                 (id, email, display_name, provider, created_at, updated_at)
             VALUES ('account-1', 'one@example.com', 'One', 'imap', 1, 1);
             INSERT INTO folders (id, account_id) VALUES ('orphan', 'missing-account');
             PRAGMA user_version = 11;",
        )
        .unwrap();

        assert!(run_migrations(&conn).is_err());

        let version: u32 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .unwrap();
        assert_eq!(version, 11, "failed FK validation must roll V12 back");
        let foreign_keys: i64 = conn
            .pragma_query_value(None, "foreign_keys", |row| row.get(0))
            .unwrap();
        assert_eq!(foreign_keys, 1, "foreign key enforcement must be restored");
    }
}
