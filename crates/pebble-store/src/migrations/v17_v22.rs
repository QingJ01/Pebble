use super::*;

pub(super) fn apply_v17(conn: &Connection) -> Result<()> {
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
    Ok(())
}

pub(super) fn apply_v18(conn: &Connection) -> Result<()> {
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
    Ok(())
}

pub(super) fn apply_v19(conn: &Connection) -> Result<()> {
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
    set_schema_version(&tx, 19)?;
    tx.commit()?;
    Ok(())
}

pub(super) fn apply_v20(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(include_str!("../mail_archive_schema.sql"))?;
    set_schema_version(&tx, 20)?;
    tx.commit()?;
    Ok(())
}

pub(super) fn apply_v22(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA foreign_keys=OFF;")
        .map_err(|e| PebbleError::Storage(format!("Migration V22 disable FK failed: {e}")))?;
    let migration_result = (|| -> Result<()> {
        let tx = conn
            .unchecked_transaction()
            .map_err(|e| PebbleError::Storage(format!("Migration V22 begin failed: {e}")))?;
        migrate_v22_foreign_keys(&tx)
            .map_err(|e| PebbleError::Storage(format!("Migration V22 failed: {e}")))?;
        set_schema_version(&tx, 22)?;
        tx.commit()
            .map_err(|e| PebbleError::Storage(format!("Migration V22 commit failed: {e}")))?;
        Ok(())
    })();
    let enable_foreign_keys_result = conn
        .execute_batch("PRAGMA foreign_keys=ON;")
        .map_err(|e| PebbleError::Storage(format!("Migration V22 enable FK failed: {e}")));
    migration_result?;
    enable_foreign_keys_result?;
    Ok(())
}

pub(super) const PENDING_MAIL_OPS_COLUMNS: [&str; 11] = [
    "id",
    "account_id",
    "message_id",
    "op_type",
    "payload_json",
    "status",
    "attempts",
    "last_error",
    "created_at",
    "updated_at",
    "next_retry_at",
];

/// V22: add the missing foreign keys by rebuilding the (small) child tables.
///
/// * `pending_mail_ops.account_id -> accounts(id) ON DELETE CASCADE`.
/// * `labels.rule_id -> rules(id) ON DELETE SET NULL` (a label outlives the
///   rule that created it).
///
/// Deliberately NOT foreign-keyed (see the store-perf report):
/// * `pending_mail_ops.message_id` — ops carry the provider identity in their
///   payload and must outlive the local row (`delete_permanent` and send
///   finalisation remove the message while the op is still being settled).
/// * `search_pending.message_id` — `remove` markers are written for messages
///   that were just hard-deleted, precisely so crash recovery can drop them
///   from the index.
/// * `mail_archives.account_id` — archives intentionally survive account
///   deletion (see mail_archive_schema.sql).
///
/// Orphans are cleaned first, and only the rebuilt tables are FK-checked so
/// unrelated legacy damage cannot brick startup.
pub(super) fn migrate_v22_foreign_keys(tx: &Connection) -> Result<()> {
    if table_exists(tx, "pending_mail_ops")? && table_exists(tx, "accounts")? {
        let complete = PENDING_MAIL_OPS_COLUMNS
            .iter()
            .map(|column| column_exists(tx, "pending_mail_ops", column))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .all(|exists| exists);
        if complete {
            let columns = PENDING_MAIL_OPS_COLUMNS.join(", ");
            tx.execute_batch(&format!(
                "DELETE FROM pending_mail_ops
                 WHERE account_id NOT IN (SELECT id FROM accounts);
                 CREATE TABLE pending_mail_ops_v22 (
                     id TEXT PRIMARY KEY,
                     account_id TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
                     message_id TEXT NOT NULL,
                     op_type TEXT NOT NULL,
                     payload_json TEXT NOT NULL,
                     status TEXT NOT NULL CHECK(status IN ('pending', 'in_progress', 'failed', 'done')),
                     attempts INTEGER NOT NULL DEFAULT 0,
                     last_error TEXT,
                     created_at INTEGER NOT NULL,
                     updated_at INTEGER NOT NULL,
                     next_retry_at INTEGER
                 );
                 INSERT INTO pending_mail_ops_v22 (rowid, {columns})
                     SELECT rowid, {columns} FROM pending_mail_ops ORDER BY rowid;
                 DROP TABLE pending_mail_ops;
                 ALTER TABLE pending_mail_ops_v22 RENAME TO pending_mail_ops;
                 CREATE INDEX IF NOT EXISTS idx_pending_mail_ops_account_status
                     ON pending_mail_ops(account_id, status, updated_at);
                 CREATE INDEX IF NOT EXISTS idx_pending_mail_ops_retry
                     ON pending_mail_ops(status, next_retry_at, updated_at);
                 CREATE INDEX IF NOT EXISTS idx_pending_mail_ops_message
                     ON pending_mail_ops(message_id);"
            ))?;
            ensure_no_fk_violations(tx, "pending_mail_ops")?;
        }
    }

    if table_exists(tx, "labels")? && table_exists(tx, "rules")? {
        let complete = ["id", "name", "color", "is_system", "rule_id"]
            .iter()
            .map(|column| column_exists(tx, "labels", column))
            .collect::<Result<Vec<_>>>()?
            .into_iter()
            .all(|exists| exists);
        if complete {
            tx.execute_batch(
                "UPDATE labels SET rule_id = NULL
                 WHERE rule_id IS NOT NULL AND rule_id NOT IN (SELECT id FROM rules);
                 CREATE TABLE labels_v22 (
                     id TEXT PRIMARY KEY,
                     name TEXT NOT NULL,
                     color TEXT NOT NULL DEFAULT '#808080',
                     is_system INTEGER NOT NULL DEFAULT 0,
                     rule_id TEXT REFERENCES rules(id) ON DELETE SET NULL
                 );
                 INSERT INTO labels_v22 (id, name, color, is_system, rule_id)
                     SELECT id, name, color, is_system, rule_id FROM labels;
                 DROP TABLE labels;
                 ALTER TABLE labels_v22 RENAME TO labels;
                 CREATE INDEX IF NOT EXISTS idx_labels_name ON labels(name);",
            )?;
            ensure_no_fk_violations(tx, "labels")?;
            if table_exists(tx, "message_labels")? && table_exists(tx, "messages")? {
                tx.execute_batch(
                    "DELETE FROM message_labels
                     WHERE label_id NOT IN (SELECT id FROM labels)
                        OR message_id NOT IN (SELECT id FROM messages);",
                )?;
                ensure_no_fk_violations(tx, "message_labels")?;
            }
        }
    }
    Ok(())
}

pub(super) fn ensure_no_fk_violations(tx: &Connection, table: &str) -> Result<()> {
    let violation = tx
        .query_row(
            &format!("PRAGMA foreign_key_check({table})"),
            [],
            |_| Ok(()),
        )
        .optional()?;
    if violation.is_some() {
        return Err(PebbleError::Storage(format!(
            "foreign key violations remain in {table}"
        )));
    }
    Ok(())
}
