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

pub(super) fn apply_v21(conn: &Connection) -> Result<()> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| PebbleError::Storage(format!("Migration V21 begin failed: {e}")))?;
    migrate_v21_list_layout(&tx)
        .map_err(|e| PebbleError::Storage(format!("Migration V21 failed: {e}")))?;
    set_schema_version(&tx, 21)?;
    tx.commit()
        .map_err(|e| PebbleError::Storage(format!("Migration V21 commit failed: {e}")))?;
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

/// V21:
/// * Move `body_text` / `body_html_raw` out of `messages` into
///   `message_bodies`, so list/summary reads never walk body overflow pages.
/// * Denormalise `date`, `is_read`, `is_deleted` and the thread key into
///   `message_folders` (kept in sync by triggers, so every writer — including
///   ones outside messages.rs — stays correct) and index
///   `(folder_id, date, message_id, thread_key) WHERE is_deleted = 0` so a
///   folder page is an index range scan with no sort.
/// * Partial indexes for unread counts and tombstone purges.
/// * `message_correspondents`: per-account recent addresses for autocomplete,
///   replacing `%x%` LIKE scans across every message's recipient JSON.
pub(super) fn migrate_v21_list_layout(tx: &Connection) -> Result<()> {
    if !table_exists(tx, "messages")? {
        return Ok(());
    }

    // --- bodies -----------------------------------------------------------
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS message_bodies (
             message_id TEXT PRIMARY KEY REFERENCES messages(id) ON DELETE CASCADE,
             body_text TEXT NOT NULL DEFAULT '',
             body_html_raw TEXT NOT NULL DEFAULT ''
         );",
    )?;
    let has_body_text = column_exists(tx, "messages", "body_text")?;
    let has_body_html = column_exists(tx, "messages", "body_html_raw")?;
    if has_body_text || has_body_html {
        let text_expr = if has_body_text {
            "COALESCE(body_text, '')"
        } else {
            "''"
        };
        let html_expr = if has_body_html {
            "COALESCE(body_html_raw, '')"
        } else {
            "''"
        };
        tx.execute_batch(&format!(
            "INSERT OR REPLACE INTO message_bodies (message_id, body_text, body_html_raw)
             SELECT id, {text_expr}, {html_expr} FROM messages;"
        ))?;
        if has_body_text {
            tx.execute_batch("ALTER TABLE messages DROP COLUMN body_text;")?;
        }
        if has_body_html {
            tx.execute_batch("ALTER TABLE messages DROP COLUMN body_html_raw;")?;
        }
    }

    // --- tombstones / unread on messages ------------------------------------
    if column_exists(tx, "messages", "deleted_at")? && column_exists(tx, "messages", "is_deleted")?
    {
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_messages_tombstones
                 ON messages(deleted_at) WHERE is_deleted = 1;",
        )?;
    }

    // --- message_folders denormalisation ----------------------------------
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

        let expr = |column: &str, fallback: &str| -> Result<String> {
            Ok(if column_exists(tx, "messages", column)? {
                format!("m.{column}")
            } else {
                fallback.to_string()
            })
        };
        let date = expr("date", "0")?;
        let is_read = expr("is_read", "0")?;
        let is_deleted = expr("is_deleted", "0")?;
        let thread_key = if column_exists(tx, "messages", "thread_id")? {
            "COALESCE(m.thread_id, m.id)".to_string()
        } else {
            "m.id".to_string()
        };
        tx.execute_batch(&format!(
            "UPDATE message_folders
             SET (date, is_read, is_deleted, thread_key) = (
                 SELECT {date}, {is_read}, {is_deleted}, {thread_key}
                 FROM messages m WHERE m.id = message_folders.message_id
             );

             DROP TRIGGER IF EXISTS trg_message_folders_denorm_insert;
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
    }

    // --- autocomplete correspondents ----------------------------------------
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
    let can_backfill_correspondents = table_exists(tx, "accounts")?
        && [
            "account_id",
            "from_name",
            "from_address",
            "to_list",
            "cc_list",
            "bcc_list",
            "date",
            "is_deleted",
        ]
        .iter()
        .map(|column| column_exists(tx, "messages", column))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .all(|exists| exists);
    if can_backfill_correspondents {
        tx.execute_batch(
            "WITH raw(account_id, name, address, date) AS (
                 SELECT account_id, from_name, from_address, date
                 FROM messages WHERE is_deleted = 0
                 UNION ALL
                 SELECT m.account_id, json_extract(j.value, '$.name'),
                        json_extract(j.value, '$.address'), m.date
                 FROM messages m,
                      json_each(CASE WHEN json_valid(m.to_list) THEN m.to_list ELSE '[]' END) j
                 WHERE m.is_deleted = 0
                 UNION ALL
                 SELECT m.account_id, json_extract(j.value, '$.name'),
                        json_extract(j.value, '$.address'), m.date
                 FROM messages m,
                      json_each(CASE WHEN json_valid(m.cc_list) THEN m.cc_list ELSE '[]' END) j
                 WHERE m.is_deleted = 0
                 UNION ALL
                 SELECT m.account_id, json_extract(j.value, '$.name'),
                        json_extract(j.value, '$.address'), m.date
                 FROM messages m,
                      json_each(CASE WHEN json_valid(m.bcc_list) THEN m.bcc_list ELSE '[]' END) j
                 WHERE m.is_deleted = 0
             ),
             cleaned AS (
                 SELECT account_id,
                        lower(trim(address)) AS normalized_address,
                        trim(address) AS address,
                        NULLIF(trim(COALESCE(name, '')), '') AS name,
                        date
                 FROM raw
                 WHERE typeof(address) = 'text'
                   AND trim(address) LIKE '%_@_%'
                   AND account_id IN (SELECT id FROM accounts)
                 ORDER BY date ASC
             )
             INSERT INTO message_correspondents
                 (account_id, normalized_address, address, name, last_interaction_at)
             SELECT account_id, normalized_address, address, name, date FROM cleaned WHERE 1
             ON CONFLICT(account_id, normalized_address) DO UPDATE SET
                 address = CASE WHEN excluded.last_interaction_at > last_interaction_at
                                THEN excluded.address ELSE address END,
                 name = CASE WHEN excluded.last_interaction_at > last_interaction_at
                             THEN excluded.name
                             WHEN name IS NULL THEN excluded.name
                             ELSE name END,
                 last_interaction_at = MAX(last_interaction_at, excluded.last_interaction_at);",
        )?;
    }
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
