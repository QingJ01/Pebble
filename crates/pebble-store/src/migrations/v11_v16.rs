use super::*;

pub(super) fn apply_v11(conn: &Connection) -> Result<()> {
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
    Ok(())
}

pub(super) fn apply_v12(conn: &Connection) -> Result<()> {
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
    Ok(())
}

pub(super) fn apply_v13(conn: &Connection) -> Result<()> {
    let tx = conn
        .unchecked_transaction()
        .map_err(|e| PebbleError::Storage(format!("Migration V13 begin failed: {e}")))?;
    rebuild_snippets(&tx)?;
    set_schema_version(&tx, 13)?;
    tx.commit()
        .map_err(|e| PebbleError::Storage(format!("Migration V13 commit failed: {e}")))?;
    Ok(())
}

pub(super) fn apply_v14(conn: &Connection) -> Result<()> {
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
    Ok(())
}

pub(super) fn apply_v15(conn: &Connection) -> Result<()> {
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
    Ok(())
}

pub(super) fn apply_v16(conn: &Connection) -> Result<()> {
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
    Ok(())
}
