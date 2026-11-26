//! V21: storage layout for fast list pages.
//!
//! * Move `body_text` / `body_html_raw` out of `messages` into
//!   `message_bodies`, so list/summary reads never walk body overflow pages.
//!   The emptied columns stay on `messages`: `ALTER TABLE ... DROP COLUMN`
//!   rewrites the whole table in one transaction (the V21 benchmark measured
//!   a WAL of about one page per message), while two empty strings per row
//!   cost one header byte each.
//! * Denormalise `date`, `is_read`, `is_deleted` and the thread key into
//!   `message_folders` (kept in sync by triggers, so every writer — including
//!   ones outside messages.rs — stays correct) and index
//!   `(folder_id, date, message_id, thread_key) WHERE is_deleted = 0` so a
//!   folder page is an index range scan with no sort.
//! * Partial indexes for unread counts and tombstone purges.
//! * `message_correspondents`: per-account recent addresses for autocomplete,
//!   replacing `%x%` LIKE scans across every message's recipient JSON.
//!
//! The rewrite is proportional to the mailbox size, so it does not run as one
//! transaction. It is split into stages, and the stages that touch every row
//! work in rowid batches with one transaction each. The finished stage and
//! the last committed rowid are recorded in `schema_migration_progress`, so a
//! crash, a kill or a failure resumes where it stopped instead of redoing (or
//! holding a lock over) the whole mailbox. Each batch commit lets SQLite's WAL
//! auto-checkpoint run, and every finished stage truncates the WAL, so the WAL
//! stays around one batch in size instead of growing to the whole rewrite.
//! `user_version` becomes 21 only after the last stage.

use super::*;

const V21: i64 = 21;

#[cfg(test)]
thread_local! {
    /// Test hook: (stage, n) makes the stage fail after committing n batches
    /// (n = 0 fails before its first change).
    pub(super) static FAIL_AFTER_BATCHES: std::cell::Cell<Option<(i64, u32)>> =
        const { std::cell::Cell::new(None) };
    /// Test hook: overrides every batch size.
    pub(super) static BATCH_OVERRIDE: std::cell::Cell<Option<i64>> =
        const { std::cell::Cell::new(None) };
}

/// Test-only: the stage currently running (-1 when idle), for benchmarks
/// that attribute WAL growth to a stage from another thread.
#[cfg(test)]
pub(super) static CURRENT_STAGE: std::sync::atomic::AtomicI64 =
    std::sync::atomic::AtomicI64::new(-1);

fn batch_size(default: i64) -> i64 {
    #[cfg(test)]
    if let Some(size) = BATCH_OVERRIDE.with(|cell| cell.get()) {
        return size;
    }
    default
}

#[allow(unused_variables)]
fn injected_failure(stage: Stage, committed_batches: u32) -> Result<()> {
    #[cfg(test)]
    if FAIL_AFTER_BATCHES.with(|cell| cell.get()) == Some((stage as i64, committed_batches)) {
        return Err(PebbleError::Storage(format!(
            "injected failure in {stage:?} after {committed_batches} batches"
        )));
    }
    Ok(())
}

/// Messages copied per body batch. Bodies are the bulk of the database, so
/// this bounds both the transaction and the WAL growth per commit.
const BODY_BATCH: i64 = 2_000;
/// `message_folders` rows denormalised per batch (small fixed-width rows).
const FOLDER_BATCH: i64 = 20_000;
/// Messages scanned per correspondents batch (each one expands its JSON
/// recipient lists).
const CORRESPONDENT_BATCH: i64 = 5_000;

/// The ordered stages. Their numbers are persisted: append, never reorder.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum Stage {
    Setup = 0,
    CopyBodies = 1,
    DenormaliseFolders = 2,
    FolderTriggersAndIndexes = 3,
    Correspondents = 4,
    Done = 5,
}

impl Stage {
    fn from_i64(value: i64) -> Result<Self> {
        Ok(match value {
            0 => Self::Setup,
            1 => Self::CopyBodies,
            2 => Self::DenormaliseFolders,
            3 => Self::FolderTriggersAndIndexes,
            4 => Self::Correspondents,
            5 => Self::Done,
            other => {
                return Err(PebbleError::Storage(format!(
                    "Migration V21: unknown stage {other} in schema_migration_progress"
                )))
            }
        })
    }

    fn next(self) -> Self {
        match self {
            Self::Setup => Self::CopyBodies,
            Self::CopyBodies => Self::DenormaliseFolders,
            Self::DenormaliseFolders => Self::FolderTriggersAndIndexes,
            Self::FolderTriggersAndIndexes => Self::Correspondents,
            Self::Correspondents | Self::Done => Self::Done,
        }
    }
}

/// Whether an earlier run of a staged migration stopped part-way. Such a
/// database was already backed up before its first stage ran.
pub(super) fn migration_in_progress(conn: &Connection) -> Result<bool> {
    if !table_exists(conn, "schema_migration_progress")? {
        return Ok(false);
    }
    Ok(conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM schema_migration_progress)",
        [],
        |row| row.get(0),
    )?)
}

fn ensure_progress_table(conn: &Connection) -> Result<()> {
    conn.execute_batch(
        "CREATE TABLE IF NOT EXISTS schema_migration_progress (
             migration INTEGER PRIMARY KEY,
             stage INTEGER NOT NULL,
             cursor INTEGER NOT NULL DEFAULT 0
         );",
    )?;
    Ok(())
}

fn load_progress(conn: &Connection) -> Result<(Stage, i64)> {
    let row: Option<(i64, i64)> = conn
        .query_row(
            "SELECT stage, cursor FROM schema_migration_progress WHERE migration = ?1",
            [V21],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .optional()?;
    match row {
        Some((stage, cursor)) => Ok((Stage::from_i64(stage)?, cursor)),
        None => Ok((Stage::Setup, 0)),
    }
}

fn save_progress(conn: &Connection, stage: Stage, cursor: i64) -> Result<()> {
    conn.execute(
        "INSERT INTO schema_migration_progress (migration, stage, cursor)
         VALUES (?1, ?2, ?3)
         ON CONFLICT(migration) DO UPDATE SET stage = excluded.stage, cursor = excluded.cursor",
        rusqlite::params![V21, stage as i64, cursor],
    )?;
    Ok(())
}

/// Best effort: return the WAL to zero length between stages.
fn truncate_wal(conn: &Connection) {
    if let Err(e) = conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(())) {
        tracing::debug!("Migration V21: WAL checkpoint skipped: {e}");
    }
}

fn storage_err(stage: Stage, e: impl std::fmt::Display) -> PebbleError {
    PebbleError::Storage(format!("Migration V21 failed in stage {stage:?}: {e}"))
}

/// Run a single-transaction stage and advance the recorded progress with it.
fn run_stage(
    conn: &Connection,
    stage: Stage,
    body: impl FnOnce(&Connection) -> Result<()>,
) -> Result<()> {
    injected_failure(stage, 0).map_err(|e| storage_err(stage, e))?;
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| storage_err(stage, e))?;
    ensure_progress_table(&tx).map_err(|e| storage_err(stage, e))?;
    body(&tx).map_err(|e| storage_err(stage, e))?;
    save_progress(&tx, stage.next(), 0).map_err(|e| storage_err(stage, e))?;
    tx.commit().map_err(|e| storage_err(stage, e))?;
    truncate_wal(conn);
    Ok(())
}

/// Walk `table` in rowid order from `cursor`, `batch` rows per transaction,
/// calling `body(tx, low_exclusive, high_inclusive)` and committing the new
/// cursor with each batch. Advances to the next stage when the table is done.
fn run_batched_stage(
    conn: &Connection,
    stage: Stage,
    table: &str,
    batch: i64,
    mut cursor: i64,
    mut body: impl FnMut(&Connection, i64, i64) -> Result<()>,
) -> Result<()> {
    let upper_sql = format!(
        "SELECT MAX(rowid) FROM (SELECT rowid FROM {table} WHERE rowid > ?1 ORDER BY rowid LIMIT ?2)"
    );
    let total: i64 = conn
        .query_row(
            &format!("SELECT COUNT(*) FROM {table} WHERE rowid > ?1"),
            [cursor],
            |row| row.get(0),
        )
        .map_err(|e| storage_err(stage, e))?;
    let batch = batch_size(batch);
    let mut done = 0i64;
    let mut committed = 0u32;
    loop {
        injected_failure(stage, committed).map_err(|e| storage_err(stage, e))?;
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| storage_err(stage, e))?;
        let high: Option<i64> = tx
            .query_row(&upper_sql, rusqlite::params![cursor, batch], |row| {
                row.get(0)
            })
            .map_err(|e| storage_err(stage, e))?;
        let Some(high) = high else {
            save_progress(&tx, stage.next(), 0).map_err(|e| storage_err(stage, e))?;
            tx.commit().map_err(|e| storage_err(stage, e))?;
            break;
        };
        body(&tx, cursor, high).map_err(|e| storage_err(stage, e))?;
        save_progress(&tx, stage, high).map_err(|e| storage_err(stage, e))?;
        tx.commit().map_err(|e| storage_err(stage, e))?;
        committed += 1;
        done = (done + batch).min(total);
        cursor = high;
        tracing::info!("Migration V21 {stage:?}: {done}/{total} rows");
    }
    truncate_wal(conn);
    Ok(())
}

pub(super) fn apply_v21(conn: &Connection) -> Result<()> {
    if !table_exists(conn, "messages")? {
        // Nothing to rewrite (tests build partial historical schemas).
        let tx = conn.unchecked_transaction()?;
        set_schema_version(&tx, 21)?;
        tx.commit()?;
        return Ok(());
    }

    let (mut stage, cursor) = if table_exists(conn, "schema_migration_progress")? {
        load_progress(conn)?
    } else {
        (Stage::Setup, 0)
    };
    if stage != Stage::Setup {
        tracing::info!("Migration V21: resuming at stage {stage:?} (cursor {cursor})");
    }
    let mut cursor = cursor;

    while stage != Stage::Done {
        tracing::info!("Migration V21: stage {stage:?}");
        #[cfg(test)]
        CURRENT_STAGE.store(stage as i64, std::sync::atomic::Ordering::Relaxed);
        match stage {
            Stage::Setup => run_stage(conn, stage, setup)?,
            Stage::CopyBodies => {
                let has_text = column_exists(conn, "messages", "body_text")?;
                let has_html = column_exists(conn, "messages", "body_html_raw")?;
                if has_text || has_html {
                    copy_bodies(conn, cursor, has_text, has_html)?;
                } else {
                    run_stage(conn, stage, |_| Ok(()))?;
                }
            }
            Stage::DenormaliseFolders => {
                if table_exists(conn, "message_folders")? {
                    denormalise_folders(conn, cursor)?;
                } else {
                    run_stage(conn, stage, |_| Ok(()))?;
                }
            }
            Stage::FolderTriggersAndIndexes => run_stage(conn, stage, folder_triggers_and_indexes)?,
            Stage::Correspondents => {
                if can_backfill_correspondents(conn)? {
                    backfill_correspondents(conn, cursor)?;
                } else {
                    run_stage(conn, stage, |_| Ok(()))?;
                }
            }
            Stage::Done => unreachable!(),
        }
        (stage, cursor) = load_progress(conn)?;
    }

    let tx = conn
        .unchecked_transaction()
        .map_err(|e| PebbleError::Storage(format!("Migration V21 finish failed: {e}")))?;
    // Pebble always declared the body columns `NOT NULL DEFAULT ''`, so new
    // inserts that omit them keep working. A column without a default would
    // reject those inserts; drop it (its rows were blanked, so this is the
    // small rewrite).
    for column in ["body_text", "body_html_raw"] {
        let needs_value: bool = tx
            .query_row(
                "SELECT \"notnull\" = 1 AND dflt_value IS NULL
                 FROM pragma_table_info('messages') WHERE name = ?1",
                [column],
                |row| row.get(0),
            )
            .optional()?
            .unwrap_or(false);
        if needs_value {
            tx.execute_batch(&format!("ALTER TABLE messages DROP COLUMN {column};"))?;
        }
    }
    tx.execute(
        "DELETE FROM schema_migration_progress WHERE migration = ?1",
        [V21],
    )?;
    set_schema_version(&tx, 21)?;
    tx.commit()
        .map_err(|e| PebbleError::Storage(format!("Migration V21 commit failed: {e}")))?;
    Ok(())
}

/// Schema additions that are cheap regardless of mailbox size.
fn setup(tx: &Connection) -> Result<()> {
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS message_bodies (
             message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
             body_text TEXT NOT NULL DEFAULT '',
             body_html_raw TEXT NOT NULL DEFAULT ''
         );",
    )?;

    if column_exists(tx, "messages", "deleted_at")? && column_exists(tx, "messages", "is_deleted")?
    {
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_messages_tombstones
                 ON messages(deleted_at) WHERE is_deleted = 1;",
        )?;
    }

    if table_exists(tx, "message_folders")? {
        tx.execute_batch(
            "DELETE FROM message_folders
             WHERE NOT EXISTS (SELECT 1 FROM messages m WHERE m.id = message_folders.message_id);",
        )?;
        for (column, ddl) in [
            (
                "date",
                "ALTER TABLE message_folders ADD COLUMN date INTEGER NOT NULL DEFAULT 0;",
            ),
            (
                "is_read",
                "ALTER TABLE message_folders ADD COLUMN is_read INTEGER NOT NULL DEFAULT 0;",
            ),
            (
                "is_deleted",
                "ALTER TABLE message_folders ADD COLUMN is_deleted INTEGER NOT NULL DEFAULT 0;",
            ),
            (
                "thread_key",
                "ALTER TABLE message_folders ADD COLUMN thread_key TEXT NOT NULL DEFAULT '';",
            ),
        ] {
            if !column_exists(tx, "message_folders", column)? {
                tx.execute_batch(ddl)?;
            }
        }
    }

    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS message_correspondents (
             account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
             normalized_address TEXT NOT NULL,
             address TEXT NOT NULL,
             name TEXT,
             last_interaction_at INTEGER NOT NULL,
             PRIMARY KEY (account_id, normalized_address)
         ) WITHOUT ROWID;
         CREATE INDEX IF NOT EXISTS idx_message_correspondents_recent
             ON message_correspondents(account_id, last_interaction_at);",
    )?;
    Ok(())
}

/// Copy bodies into `message_bodies` and blank them in `messages` in the same
/// batch transaction, so list reads stop touching their overflow pages and
/// the freed pages are reused by the next batch's copies.
fn copy_bodies(conn: &Connection, cursor: i64, has_text: bool, has_html: bool) -> Result<()> {
    let text_expr = if has_text {
        "COALESCE(body_text, '')"
    } else {
        "''"
    };
    let html_expr = if has_html {
        "COALESCE(body_html_raw, '')"
    } else {
        "''"
    };
    let copy = format!(
        "INSERT OR IGNORE INTO message_bodies (message_id, body_text, body_html_raw)
         SELECT id, {text_expr}, {html_expr} FROM messages WHERE rowid > ?1 AND rowid <= ?2"
    );
    let mut blank_columns = Vec::new();
    if has_text {
        blank_columns.push("body_text = ''");
    }
    if has_html {
        blank_columns.push("body_html_raw = ''");
    }
    let blank = format!(
        "UPDATE messages SET {} WHERE rowid > ?1 AND rowid <= ?2",
        blank_columns.join(", ")
    );
    run_batched_stage(
        conn,
        Stage::CopyBodies,
        "messages",
        BODY_BATCH,
        cursor,
        |tx, low, high| {
            tx.execute(&copy, [low, high])?;
            tx.execute(&blank, [low, high])?;
            Ok(())
        },
    )
}

/// Source expressions for the denormalised `message_folders` columns.
fn denorm_sources(conn: &Connection) -> Result<(String, String, String, String)> {
    let expr = |column: &str, fallback: &str| -> Result<String> {
        Ok(if column_exists(conn, "messages", column)? {
            format!("m.{column}")
        } else {
            fallback.to_string()
        })
    };
    let thread_key = if column_exists(conn, "messages", "thread_id")? {
        "COALESCE(m.thread_id, m.id)".to_string()
    } else {
        "m.id".to_string()
    };
    Ok((
        expr("date", "0")?,
        expr("is_read", "0")?,
        expr("is_deleted", "0")?,
        thread_key,
    ))
}

fn denormalise_folders(conn: &Connection, cursor: i64) -> Result<()> {
    let (date, is_read, is_deleted, thread_key) = denorm_sources(conn)?;
    let sql = format!(
        "UPDATE message_folders
         SET (date, is_read, is_deleted, thread_key) = (
             SELECT {date}, {is_read}, {is_deleted}, {thread_key}
             FROM messages m WHERE m.id = message_folders.message_id
         )
         WHERE rowid > ?1 AND rowid <= ?2"
    );
    run_batched_stage(
        conn,
        Stage::DenormaliseFolders,
        "message_folders",
        FOLDER_BATCH,
        cursor,
        |tx, low, high| {
            tx.execute(&sql, [low, high])?;
            Ok(())
        },
    )
}

fn folder_triggers_and_indexes(tx: &Connection) -> Result<()> {
    if !table_exists(tx, "message_folders")? {
        return Ok(());
    }
    let (date, is_read, is_deleted, thread_key) = denorm_sources(tx)?;
    tx.execute_batch(&format!(
        "DROP TRIGGER IF EXISTS trg_message_folders_denorm_insert;
         CREATE TRIGGER trg_message_folders_denorm_insert
         AFTER INSERT ON message_folders
         BEGIN
             UPDATE message_folders
             SET (date, is_read, is_deleted, thread_key) = (
                 SELECT {date}, {is_read}, {is_deleted}, {thread_key}
                 FROM messages m WHERE m.id = NEW.message_id
             )
             WHERE message_id = NEW.message_id AND folder_id = NEW.folder_id
               AND EXISTS (SELECT 1 FROM messages m WHERE m.id = NEW.message_id);
         END;

         DROP TRIGGER IF EXISTS trg_message_folders_denorm_relink;
         CREATE TRIGGER trg_message_folders_denorm_relink
         AFTER UPDATE OF message_id ON message_folders
         BEGIN
             UPDATE message_folders
             SET (date, is_read, is_deleted, thread_key) = (
                 SELECT {date}, {is_read}, {is_deleted}, {thread_key}
                 FROM messages m WHERE m.id = NEW.message_id
             )
             WHERE message_id = NEW.message_id AND folder_id = NEW.folder_id
               AND EXISTS (SELECT 1 FROM messages m WHERE m.id = NEW.message_id);
         END;"
    ))?;

    let has_all_sources = column_exists(tx, "messages", "date")?
        && column_exists(tx, "messages", "is_read")?
        && column_exists(tx, "messages", "is_deleted")?
        && column_exists(tx, "messages", "thread_id")?;
    if has_all_sources {
        tx.execute_batch(
            "DROP TRIGGER IF EXISTS trg_messages_denorm_folders;
             CREATE TRIGGER trg_messages_denorm_folders
             AFTER UPDATE OF date, is_read, is_deleted, thread_id ON messages
             WHEN OLD.date IS NOT NEW.date
               OR OLD.is_read IS NOT NEW.is_read
               OR OLD.is_deleted IS NOT NEW.is_deleted
               OR OLD.thread_id IS NOT NEW.thread_id
             BEGIN
                 UPDATE message_folders
                 SET date = NEW.date,
                     is_read = NEW.is_read,
                     is_deleted = NEW.is_deleted,
                     thread_key = COALESCE(NEW.thread_id, NEW.id)
                 WHERE message_id = NEW.id;
             END;",
        )?;
    }

    tx.execute_batch(
        "DROP INDEX IF EXISTS idx_message_folders_folder_id;
         CREATE INDEX IF NOT EXISTS idx_mf_folder_date
             ON message_folders(folder_id, date, message_id, thread_key)
             WHERE is_deleted = 0;
         CREATE INDEX IF NOT EXISTS idx_mf_folder_thread
             ON message_folders(folder_id, thread_key, date, message_id)
             WHERE is_deleted = 0;
         CREATE INDEX IF NOT EXISTS idx_mf_folder_unread
             ON message_folders(folder_id)
             WHERE is_read = 0 AND is_deleted = 0;",
    )?;
    Ok(())
}

fn can_backfill_correspondents(conn: &Connection) -> Result<bool> {
    if !table_exists(conn, "accounts")? {
        return Ok(false);
    }
    for column in [
        "account_id",
        "from_name",
        "from_address",
        "to_list",
        "cc_list",
        "bcc_list",
        "date",
        "is_deleted",
    ] {
        if !column_exists(conn, "messages", column)? {
            return Ok(false);
        }
    }
    Ok(true)
}

/// Upsert each batch's senders and recipients. The conflict clause keeps the
/// newest address spelling/name and the latest date whatever order rows (or
/// batches) arrive in, so no global sort is needed.
fn backfill_correspondents(conn: &Connection, cursor: i64) -> Result<()> {
    const SQL: &str = "WITH batch AS (
             SELECT account_id, from_name, from_address, to_list, cc_list, bcc_list, date
             FROM messages
             WHERE rowid > ?1 AND rowid <= ?2 AND is_deleted = 0
               AND account_id IN (SELECT id FROM accounts)
         ),
         raw(account_id, name, address, date) AS (
             SELECT account_id, from_name, from_address, date FROM batch
             UNION ALL
             SELECT b.account_id, json_extract(j.value, '$.name'),
                    json_extract(j.value, '$.address'), b.date
             FROM batch b,
                  json_each(CASE WHEN json_valid(b.to_list) THEN b.to_list ELSE '[]' END) j
             UNION ALL
             SELECT b.account_id, json_extract(j.value, '$.name'),
                    json_extract(j.value, '$.address'), b.date
             FROM batch b,
                  json_each(CASE WHEN json_valid(b.cc_list) THEN b.cc_list ELSE '[]' END) j
             UNION ALL
             SELECT b.account_id, json_extract(j.value, '$.name'),
                    json_extract(j.value, '$.address'), b.date
             FROM batch b,
                  json_each(CASE WHEN json_valid(b.bcc_list) THEN b.bcc_list ELSE '[]' END) j
         )
         INSERT INTO message_correspondents
             (account_id, normalized_address, address, name, last_interaction_at)
         SELECT account_id, lower(trim(address)), trim(address),
                NULLIF(trim(COALESCE(name, '')), ''), date
         FROM raw
         WHERE typeof(address) = 'text' AND trim(address) LIKE '%_@_%'
         ON CONFLICT(account_id, normalized_address) DO UPDATE SET
             address = CASE WHEN excluded.last_interaction_at > last_interaction_at
                            THEN excluded.address ELSE address END,
             name = CASE WHEN excluded.last_interaction_at > last_interaction_at
                         THEN COALESCE(excluded.name, name)
                         WHEN name IS NULL THEN excluded.name
                         ELSE name END,
             last_interaction_at = MAX(last_interaction_at, excluded.last_interaction_at)";
    run_batched_stage(
        conn,
        Stage::Correspondents,
        "messages",
        CORRESPONDENT_BATCH,
        cursor,
        |tx, low, high| {
            tx.execute(SQL, [low, high])?;
            Ok(())
        },
    )
}
