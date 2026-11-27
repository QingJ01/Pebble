use pebble_core::{build_snippet, PebbleError, Result};
use rusqlite::{Connection, OptionalExtension};
use std::collections::HashSet;

#[cfg(test)]
mod tests;
mod v01_v10;
mod v11_v16;
mod v17_v22;
mod v21;
mod v23;

pub(crate) const CURRENT_VERSION: u32 = 23;
/// How many pre-migration backups to keep next to the database file.
const MIGRATION_BACKUPS_TO_KEEP: usize = 2;
// Keep in sync with src/lib/accountColors.ts and src-tauri/src/account_colors.rs
// (tests/lib/account-color-presets-sync.test.ts checks all three).
const ACCOUNT_COLOR_PRESETS: [&str; 12] = [
    "#0ea5e9", "#10b981", "#8b5cf6", "#ec4899", "#65a30d", "#6366f1", "#14b8a6", "#c026d3",
    "#3b82f6", "#64748b", "#06b6d4", "#1e40af",
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

fn column_exists(conn: &Connection, table: &str, column: &str) -> Result<bool> {
    let mut stmt = conn.prepare("SELECT name FROM pragma_table_info(?1)")?;
    let mut rows = stmt.query([table])?;
    while let Some(row) = rows.next()? {
        let name: String = row.get(0)?;
        if name.eq_ignore_ascii_case(column) {
            return Ok(true);
        }
    }
    Ok(false)
}

/// Path of the main database file, or `None` for in-memory databases.
fn database_file_path(conn: &Connection) -> Option<std::path::PathBuf> {
    let path = conn.path()?;
    if path.is_empty() {
        return None;
    }
    let path = std::path::PathBuf::from(path);
    path.is_file().then_some(path)
}

/// Snapshot the database with `VACUUM INTO` before applying migrations to an
/// existing database, keeping the newest [`MIGRATION_BACKUPS_TO_KEEP`] copies.
/// Migrations such as V21 rewrite large tables; a consistent copy makes a
/// failed or buggy upgrade recoverable by hand.
fn backup_before_migration(conn: &Connection, from_version: u32) -> Result<()> {
    let Some(db_path) = database_file_path(conn) else {
        return Ok(());
    };
    let Some(file_name) = db_path.file_name().and_then(|n| n.to_str()) else {
        return Ok(());
    };
    let dir = db_path
        .parent()
        .map(std::path::Path::to_path_buf)
        .unwrap_or_default();
    let stamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    let prefix = format!("{file_name}.v{from_version}-");
    let backup_path = dir.join(format!("{prefix}{stamp}.bak"));
    conn.execute("VACUUM INTO ?1", [backup_path.to_string_lossy().as_ref()])
        .map_err(|e| {
            PebbleError::Storage(format!(
                "Refusing to migrate database from v{from_version}: backup to {} failed: {e}",
                backup_path.display()
            ))
        })?;
    tracing::info!(
        "Database backed up to {} before migrating from v{from_version}",
        backup_path.display()
    );

    // Prune older backups of this database file (any source version).
    let backup_prefix = format!("{file_name}.v");
    if let Ok(entries) = std::fs::read_dir(&dir) {
        let mut backups: Vec<(std::time::SystemTime, std::path::PathBuf)> = entries
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_str()
                    .is_some_and(|n| n.starts_with(&backup_prefix) && n.ends_with(".bak"))
            })
            .filter_map(|entry| {
                let modified = entry.metadata().and_then(|m| m.modified()).ok()?;
                Some((modified, entry.path()))
            })
            .collect();
        backups.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.cmp(&a.1)));
        for (_, stale) in backups.into_iter().skip(MIGRATION_BACKUPS_TO_KEEP) {
            if let Err(e) = std::fs::remove_file(&stale) {
                tracing::warn!(
                    "Failed to prune old migration backup {}: {e}",
                    stale.display()
                );
            }
        }
    }
    Ok(())
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
    run_migrations_up_to(conn, CURRENT_VERSION)
}

/// [`run_migrations`], reporting progress after each schema version.
pub fn run_migrations_with_progress(
    conn: &Connection,
    progress: &mut dyn FnMut(MigrationProgress),
) -> Result<()> {
    run_migrations_reporting(conn, CURRENT_VERSION, progress)
}

/// How far an upgrade has got: `done` of `total` schema versions applied.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MigrationProgress {
    pub done: u32,
    pub total: u32,
}

/// Schema version of the database at `path` when it exists and is older
/// than this build (so opening it will run migrations), else `None`. A new
/// or empty database returns `None`: creating it is quick.
pub fn pending_migration_from(path: &std::path::Path) -> Result<Option<u32>> {
    if !path.exists() {
        return Ok(None);
    }
    let conn = Connection::open_with_flags(
        path,
        rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY | rusqlite::OpenFlags::SQLITE_OPEN_NO_MUTEX,
    )?;
    let version = get_schema_version(&conn);
    Ok((version > 0 && version < CURRENT_VERSION).then_some(version))
}

/// Apply migrations up to `target` (tests use this to build historical
/// schemas; production always targets [`CURRENT_VERSION`]). Versions up to
/// 20 are always applied.
fn run_migrations_up_to(conn: &Connection, target: u32) -> Result<()> {
    run_migrations_reporting(conn, target, &mut |_| {})
}

fn run_migrations_reporting(
    conn: &Connection,
    target: u32,
    progress: &mut dyn FnMut(MigrationProgress),
) -> Result<()> {
    // auto_vacuum can only be chosen before the first table exists; new
    // databases reclaim space incrementally (see Store::run_maintenance).
    let has_tables: bool = conn.query_row(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table')",
        [],
        |row| row.get(0),
    )?;
    if !has_tables {
        conn.execute_batch("PRAGMA auto_vacuum = INCREMENTAL;")?;
    }
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch("PRAGMA synchronous=NORMAL;")?;

    conn.execute_batch("PRAGMA foreign_keys=ON;")?;

    conn.execute_batch("PRAGMA busy_timeout=5000;")?;

    let version = get_schema_version(conn);
    let total = target.saturating_sub(version);
    let mut step = |applied: u32| {
        if total > 0 && applied > version {
            progress(MigrationProgress {
                done: (applied - version).min(total),
                total,
            });
        }
    };
    if version > CURRENT_VERSION {
        return Err(PebbleError::Storage(format!(
            "Database schema version {version} is newer than this version of Pebble supports \
             ({CURRENT_VERSION}). Please update Pebble; the database was left untouched."
        )));
    }
    // A staged migration that stopped part-way was backed up before its
    // first stage; resuming must not snapshot the half-migrated file over
    // the older good backups.
    if version > 0 && version < target && !v21::migration_in_progress(conn)? {
        backup_before_migration(conn, version)?;
    }

    // Each migration is wrapped in a transaction so that the DDL and version
    // update are atomic; a crash mid-migration won't leave an inconsistent state.

    if version < 1 {
        v01_v10::apply_v1(conn)?;
        step(1);
    }

    if version < 2 {
        v01_v10::apply_v2(conn)?;
        step(2);
    }

    if version < 3 {
        v01_v10::apply_v3(conn)?;
        step(3);
    }

    if version < 4 {
        v01_v10::apply_v4(conn)?;
        step(4);
    }

    if version < 5 {
        v01_v10::apply_v5(conn)?;
        step(5);
    }

    // V6: search_pending table for crash-recovery of the search index
    if version < 6 {
        v01_v10::apply_v6(conn)?;
        step(6);
    }

    if version < 7 {
        v01_v10::apply_v7(conn)?;
        step(7);
    }

    if version < 8 {
        v01_v10::apply_v8(conn)?;
        step(8);
    }

    if version < 9 {
        v01_v10::apply_v9(conn)?;
        step(9);
    }

    if version < 10 {
        v01_v10::apply_v10(conn)?;
        step(10);
    }

    if version < 11 {
        v11_v16::apply_v11(conn)?;
        step(11);
    }

    if version < 12 {
        v11_v16::apply_v12(conn)?;
        step(12);
    }

    // V13: rebuild snippets to strip leaked HTML/CSS from previews
    if version < 13 {
        v11_v16::apply_v13(conn)?;
        step(13);
    }

    // V14: enforce at most one LIVE row per (account_id, remote_id). The legacy
    // idx_messages_account_remote index was non-unique, so a soft-delete followed
    // by a re-sync that re-fetched the same remote_id inserted a second row
    // (tombstone + new live copy). Deduplicate existing live rows first, then
    // replace the index with a partial UNIQUE index scoped to is_deleted = 0 so
    // tombstones (is_deleted = 1) can still coexist with a fresh live copy.
    if version < 14 {
        v11_v16::apply_v14(conn)?;
        step(14);
    }

    // V15: local-only drafts use an empty remote_id. They are distinct records,
    // so the remote-message uniqueness constraint must not include them.
    if version < 15 {
        v11_v16::apply_v15(conn)?;
        step(15);
    }

    // V16: IMAP UIDs are scoped to one mailbox, while OAuth/POP provider IDs
    // remain account-wide. SQLite cannot express that relationship with one
    // messages-table index, so provider-aware triggers enforce both scopes.
    if version < 16 {
        v11_v16::apply_v16(conn)?;
        step(16);
    }

    // V17: profile-level address book and hidden recent-contact suggestions.
    if version < 17 {
        v17_v22::apply_v17(conn)?;
        step(17);
    }

    if version < 18 {
        v17_v22::apply_v18(conn)?;
        step(18);
    }

    if version < 19 {
        v17_v22::apply_v19(conn)?;
        step(19);
    }

    if version < 20 {
        v17_v22::apply_v20(conn)?;
        step(20);
    }

    // V21: storage layout for fast list pages (staged and resumable).
    if version < 21 && target >= 21 {
        v21::apply_v21(conn)?;
        step(21);
    }

    // V22: foreign keys for pending_mail_ops.account_id and labels.rule_id.
    // Table rebuilds must run with enforcement off (dropping `labels` would
    // otherwise cascade into message_labels), mirroring V12.
    if version < 22 && target >= 22 {
        v17_v22::apply_v22(conn)?;
        step(22);
    }

    // V23: partial date index for unread-only list pages.
    if version < 23 && target >= 23 {
        v23::apply_v23(conn)?;
        step(23);
    }

    Ok(())
}
