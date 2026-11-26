use super::*;

#[test]
fn v19_archive_upgrade_is_sequential_preserves_queue_and_is_transactional() {
    let conn = Connection::open_in_memory().unwrap();
    run_migrations(&conn).unwrap();
    conn.execute_batch("DROP TABLE mail_archive_fts; DROP TABLE mail_archive_messages; DROP TABLE mail_archive_folders; DROP TABLE mail_archives; PRAGMA user_version=19;").unwrap();
    conn.execute_batch("INSERT INTO accounts (id,email,provider,created_at,updated_at) VALUES ('a','a@example.com','imap',1,1); INSERT INTO scheduled_sends VALUES ('job','a','scheduled',200,'pending',X'0102',0,100);").unwrap();
    run_migrations(&conn).unwrap();
    assert_eq!(get_schema_version(&conn), CURRENT_VERSION);
    let payload: Vec<u8> = conn
        .query_row(
            "SELECT payload FROM scheduled_sends WHERE id='job'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(payload, vec![1, 2]);
    assert!(table_exists(&conn, "mail_archives").unwrap());
    run_migrations(&conn).unwrap();

    let broken = Connection::open_in_memory().unwrap();
    broken.execute_batch("PRAGMA user_version=18; CREATE TABLE mail_archive_folders (sentinel TEXT); INSERT INTO mail_archive_folders VALUES ('preserve');").unwrap();
    assert!(run_migrations(&broken).is_err());
    assert_eq!(get_schema_version(&broken), 19);
    assert!(table_exists(&broken, "scheduled_sends").unwrap());
    assert!(!table_exists(&broken, "mail_archives").unwrap());
    let value: String = broken
        .query_row("SELECT sentinel FROM mail_archive_folders", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(value, "preserve");
}

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
    broken
        .execute_batch(
            "CREATE TABLE accounts (id TEXT, provider_display_name TEXT); PRAGMA user_version=17;",
        )
        .unwrap();
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
            "SELECT m.id, b.body_text FROM messages m
                 JOIN message_bodies b ON b.message_id = m.id
                 WHERE m.account_id='acct' AND m.remote_id='UID:42' AND m.is_deleted=0",
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
                (id, account_id, remote_id, is_deleted, created_at, updated_at)
             VALUES ('m-extra','acct','UID:42',0,3,3)",
        [],
    );
    assert!(
        dup_live.is_err(),
        "second live row must be rejected by the partial unique index"
    );

    let dup_tomb = conn.execute(
        "INSERT INTO messages
                (id, account_id, remote_id, is_deleted, created_at, updated_at)
             VALUES ('m-tomb2','acct','UID:42',1,3,3)",
        [],
    );
    assert!(
        dup_tomb.is_ok(),
        "additional tombstone must be allowed alongside one live row"
    );
    let another_local = conn.execute(
        "INSERT INTO messages
                (id, account_id, remote_id, is_deleted, created_at, updated_at)
             VALUES ('m-local-c','acct','',0,3,3)",
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
        vec![Some("#0ea5e9".to_string()), Some("#10b981".to_string())]
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

fn seed_v20_mailbox(conn: &Connection) {
    conn.execute_batch(
            r#"INSERT INTO accounts (id, email, provider, created_at, updated_at)
                   VALUES ('acct', 'me@example.com', 'imap', 1, 1);
               INSERT INTO folders (id, account_id, remote_id, name, folder_type, role)
                   VALUES ('inbox', 'acct', 'INBOX', 'Inbox', 'folder', 'inbox');
               INSERT INTO messages (id, account_id, remote_id, thread_id, subject, from_address,
                   from_name, to_list, body_text, body_html_raw, is_read, date, created_at, updated_at)
               VALUES ('m1', 'acct', '1', 't1', 'Hello', 'alice@example.com', 'Alice',
                   '[{"name":"Bob","address":"Bob@Example.com"}]', 'plain one', '<p>one</p>', 0, 100, 1, 1),
                      ('m2', 'acct', '2', NULL, 'Again', 'carol@example.com', '',
                   '[]', 'plain two', '', 1, 200, 1, 1);
               INSERT INTO message_folders (message_id, folder_id) VALUES ('m1', 'inbox'), ('m2', 'inbox');
               INSERT INTO rules (id, name, created_at, updated_at) VALUES ('r1', 'Rule', 1, 1);
               INSERT INTO labels (id, name, rule_id) VALUES ('l-live', 'Live', 'r1'),
                                                             ('l-dangling', 'Dangling', 'gone');
               INSERT INTO message_labels (message_id, label_id) VALUES ('m1', 'l-live');
               INSERT INTO pending_mail_ops (id, account_id, message_id, op_type, payload_json,
                   status, created_at, updated_at)
               VALUES ('op-live', 'acct', 'm1', 'archive', '{}', 'pending', 1, 1),
                      ('op-orphan', 'deleted-account', 'mx', 'archive', '{}', 'pending', 1, 1);"#,
        )
        .unwrap();
}

fn count(conn: &Connection, sql: &str) -> i64 {
    conn.query_row(sql, [], |row| row.get(0)).unwrap()
}

#[test]
fn v21_v22_move_bodies_denormalise_folders_and_add_foreign_keys() {
    let conn = Connection::open_in_memory().unwrap();
    run_migrations_up_to(&conn, 20).unwrap();
    assert_eq!(get_schema_version(&conn), 20);
    seed_v20_mailbox(&conn);
    conn.execute_batch("UPDATE pending_mail_ops SET rowid = 42 WHERE id = 'op-live';")
        .unwrap();

    run_migrations(&conn).unwrap();
    assert_eq!(get_schema_version(&conn), CURRENT_VERSION);

    // Bodies moved out of `messages`; the legacy columns are left empty.
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM messages WHERE body_text != '' OR body_html_raw != ''"
        ),
        0
    );
    let body: (String, String) = conn
        .query_row(
            "SELECT body_text, body_html_raw FROM message_bodies WHERE message_id = 'm1'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?)),
        )
        .unwrap();
    assert_eq!(body, ("plain one".into(), "<p>one</p>".into()));

    // message_folders denormalised and kept in sync by triggers.
    let row: (i64, i64, i64, String) = conn
            .query_row(
                "SELECT date, is_read, is_deleted, thread_key FROM message_folders WHERE message_id = 'm2'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
    assert_eq!(row, (200, 1, 0, "m2".into()));
    conn.execute_batch(
            "UPDATE messages SET is_read = 1, date = 150, thread_id = 't9', is_deleted = 1 WHERE id = 'm1';",
        )
        .unwrap();
    let row: (i64, i64, i64, String) = conn
            .query_row(
                "SELECT date, is_read, is_deleted, thread_key FROM message_folders WHERE message_id = 'm1'",
                [],
                |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?)),
            )
            .unwrap();
    assert_eq!(row, (150, 1, 1, "t9".into()));
    conn.execute_batch(
            "INSERT INTO folders (id, account_id, remote_id, name, folder_type) VALUES ('arch', 'acct', 'Archive', 'Archive', 'folder');
             INSERT INTO message_folders (message_id, folder_id) VALUES ('m2', 'arch');",
        )
        .unwrap();
    assert_eq!(
        count(
            &conn,
            "SELECT date FROM message_folders WHERE message_id = 'm2' AND folder_id = 'arch'"
        ),
        200
    );

    // Autocomplete correspondents were backfilled (normalised, named).
    let bob: (String, Option<String>, i64) = conn
        .query_row(
            "SELECT address, name, last_interaction_at FROM message_correspondents
                 WHERE account_id = 'acct' AND normalized_address = 'bob@example.com'",
            [],
            |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?)),
        )
        .unwrap();
    assert_eq!(bob, ("Bob@Example.com".into(), Some("Bob".into()), 100));
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM message_correspondents"),
        3
    );

    for index in [
        "idx_mf_folder_date",
        "idx_mf_folder_thread",
        "idx_mf_folder_unread",
        "idx_messages_tombstones",
    ] {
        assert_eq!(
            count(
                &conn,
                &format!(
                    "SELECT COUNT(*) FROM sqlite_master WHERE type = 'index' AND name = '{index}'"
                )
            ),
            1,
            "{index} must exist"
        );
    }

    // V22: orphaned op removed (rowids preserved for replay ordering),
    // account FK cascades.
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM pending_mail_ops"), 1);
    assert_eq!(
        count(
            &conn,
            "SELECT rowid FROM pending_mail_ops WHERE id = 'op-live'"
        ),
        42
    );
    assert!(conn
            .execute(
                "INSERT INTO pending_mail_ops (id, account_id, message_id, op_type, payload_json, status, created_at, updated_at)
                 VALUES ('op-bad', 'nobody', 'm1', 'archive', '{}', 'pending', 1, 1)",
                [],
            )
            .is_err());
    // labels.rule_id: dangling reference cleared, deleting the rule nulls it.
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM labels WHERE id = 'l-dangling' AND rule_id IS NULL"
        ),
        1
    );
    conn.execute("DELETE FROM rules WHERE id = 'r1'", [])
        .unwrap();
    assert_eq!(
        count(
            &conn,
            "SELECT COUNT(*) FROM labels WHERE id = 'l-live' AND rule_id IS NULL"
        ),
        1
    );
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM message_labels"), 1);
    conn.execute("DELETE FROM accounts WHERE id = 'acct'", [])
        .unwrap();
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM pending_mail_ops"), 0);
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM message_bodies"), 0);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM message_correspondents"),
        0
    );
    let fk_on: i64 = conn
        .pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .unwrap();
    assert_eq!(fk_on, 1);

    // Re-running is a no-op.
    run_migrations(&conn).unwrap();
}

#[test]
fn failed_v21_rolls_back_body_move_and_keeps_version() {
    let conn = Connection::open_in_memory().unwrap();
    run_migrations_up_to(&conn, 20).unwrap();
    seed_v20_mailbox(&conn);
    // A conflicting legacy table makes V21's setup stage fail; nothing of
    // that stage (and no later stage) may be left behind.
    conn.execute_batch(
        "CREATE TABLE message_correspondents (sentinel TEXT);
             INSERT INTO message_correspondents VALUES ('preserve');",
    )
    .unwrap();

    assert!(run_migrations(&conn).is_err());
    assert_eq!(get_schema_version(&conn), 20);
    assert!(column_exists(&conn, "messages", "body_text").unwrap());
    assert!(!table_exists(&conn, "message_bodies").unwrap());
    assert!(!column_exists(&conn, "message_folders", "date").unwrap());
    let body: String = conn
        .query_row(
            "SELECT body_text FROM messages WHERE id = 'm1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(body, "plain one");
    let sentinel: String = conn
        .query_row("SELECT sentinel FROM message_correspondents", [], |row| {
            row.get(0)
        })
        .unwrap();
    assert_eq!(sentinel, "preserve");
}

#[test]
fn failed_v22_rolls_back_rebuilds_and_restores_fk_enforcement() {
    let conn = Connection::open_in_memory().unwrap();
    run_migrations_up_to(&conn, 21).unwrap();
    seed_v20_mailbox_v21(&conn);
    conn.execute_batch("CREATE TABLE labels_v22 (sentinel TEXT);")
        .unwrap();

    assert!(run_migrations(&conn).is_err());
    assert_eq!(get_schema_version(&conn), 21);
    // pending_mail_ops rebuild (which runs first) was rolled back too.
    assert_eq!(count(&conn, "SELECT COUNT(*) FROM pending_mail_ops"), 2);
    assert_eq!(
        count(&conn, "SELECT COUNT(*) FROM labels WHERE rule_id = 'gone'"),
        1
    );
    let fk_on: i64 = conn
        .pragma_query_value(None, "foreign_keys", |row| row.get(0))
        .unwrap();
    assert_eq!(fk_on, 1);
}

fn seed_v20_mailbox_v21(conn: &Connection) {
    conn.execute_batch(
        "INSERT INTO accounts (id, email, provider, created_at, updated_at)
                 VALUES ('acct', 'me@example.com', 'imap', 1, 1);
             INSERT INTO rules (id, name, created_at, updated_at) VALUES ('r1', 'Rule', 1, 1);
             INSERT INTO labels (id, name, rule_id) VALUES ('l-dangling', 'Dangling', 'gone');
             INSERT INTO pending_mail_ops (id, account_id, message_id, op_type, payload_json,
                 status, created_at, updated_at)
             VALUES ('op-live', 'acct', 'm1', 'archive', '{}', 'pending', 1, 1),
                    ('op-orphan', 'deleted-account', 'mx', 'archive', '{}', 'pending', 1, 1);",
    )
    .unwrap();
}

#[test]
fn newer_schema_version_is_refused_without_touching_the_database() {
    let conn = Connection::open_in_memory().unwrap();
    conn.execute_batch(&format!(
        "CREATE TABLE future (x TEXT); PRAGMA user_version = {};",
        CURRENT_VERSION + 1
    ))
    .unwrap();
    let err = run_migrations(&conn).unwrap_err().to_string();
    assert!(err.contains("newer"), "unexpected error: {err}");
    assert_eq!(get_schema_version(&conn), CURRENT_VERSION + 1);
    assert!(!table_exists(&conn, "accounts").unwrap());
}

#[test]
fn pending_migrations_back_up_the_file_and_keep_the_newest_two() {
    let dir = std::env::temp_dir().join(format!("pebble-migrate-{}", pebble_core::new_id()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("pebble.db");
    {
        let conn = Connection::open(&db).unwrap();
        run_migrations_up_to(&conn, 20).unwrap();
        seed_v20_mailbox(&conn);
    }
    // Two stale backups from earlier upgrades.
    for name in ["pebble.db.v18-1.bak", "pebble.db.v19-2.bak"] {
        std::fs::write(dir.join(name), b"old").unwrap();
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    // Fresh databases (user_version 0) are never backed up.
    let fresh = dir.join("fresh.db");
    run_migrations(&Connection::open(&fresh).unwrap()).unwrap();

    let conn = Connection::open(&db).unwrap();
    run_migrations(&conn).unwrap();
    drop(conn);

    let mut backups: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".bak"))
        .collect();
    backups.sort();
    assert_eq!(backups.len(), 2, "{backups:?}");
    assert!(backups.contains(&"pebble.db.v19-2.bak".to_string()));
    let snapshot = backups
        .iter()
        .find(|n| n.starts_with("pebble.db.v20-"))
        .expect("v20 snapshot must exist");
    let restored = Connection::open(dir.join(snapshot)).unwrap();
    assert_eq!(get_schema_version(&restored), 20);
    let body: String = restored
        .query_row(
            "SELECT body_text FROM messages WHERE id = 'm1'",
            [],
            |row| row.get(0),
        )
        .unwrap();
    assert_eq!(body, "plain one");
    drop(restored);
    let _ = std::fs::remove_dir_all(&dir);
}

/// Insert `n` messages (with bodies, recipients and one or two folders each)
/// into a v20 database that already holds `seed_v20_mailbox`.
fn seed_bulk_v20(conn: &Connection, n: usize, body_bytes: usize) {
    conn.execute_batch(
        "INSERT INTO folders (id, account_id, remote_id, name, folder_type, role)
             VALUES ('bulk-arch', 'acct', 'Archive', 'Archive', 'folder', 'archive');",
    )
    .unwrap();
    let body: String = "lorem ipsum dolor sit amet ".repeat(body_bytes / 27 + 1);
    let tx = conn.unchecked_transaction().unwrap();
    {
        let mut msg = tx
            .prepare(
                "INSERT INTO messages (id, account_id, remote_id, thread_id, subject, from_address,
                     from_name, to_list, cc_list, body_text, body_html_raw, is_read, is_deleted,
                     date, created_at, updated_at)
                 VALUES (?1, 'acct', ?1, ?2, 'Bulk', ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, 1, 1)",
            )
            .unwrap();
        let mut folder = tx
            .prepare("INSERT INTO message_folders (message_id, folder_id) VALUES (?1, ?2)")
            .unwrap();
        for i in 0..n {
            let id = format!("bulk-{i}");
            let thread = (i % 7 != 0).then(|| format!("bt-{}", i / 5));
            let from = format!("sender{}@example.com", i % 97);
            let to = format!(
                r#"[{{"name":"Person {a}","address":"Person{a}@Example.com"}},{{"name":null,"address":"team@example.com"}}]"#,
                a = i % 53
            );
            let cc = if i % 3 == 0 { "not json" } else { "[]" };
            msg.execute(rusqlite::params![
                id,
                thread,
                from,
                format!("Sender {}", i % 97),
                to,
                cc,
                &body[..body_bytes.min(body.len())],
                format!("<p>{}</p>", &body[..body_bytes.min(body.len())]),
                (i % 2) as i64,
                (i % 11 == 0) as i64,
                1_000 + i as i64,
            ])
            .unwrap();
            folder.execute(rusqlite::params![id, "inbox"]).unwrap();
            if i % 4 == 0 {
                folder.execute(rusqlite::params![id, "bulk-arch"]).unwrap();
            }
        }
    }
    tx.commit().unwrap();
}

/// Everything V21 produces, in a comparable form.
fn v21_snapshot(conn: &Connection) -> Vec<String> {
    let mut out = Vec::new();
    for sql in [
        "SELECT message_id || '|' || length(body_text) || '|' || length(body_html_raw) || '|' || substr(body_text, 1, 20)
             FROM message_bodies ORDER BY message_id",
        "SELECT message_id || '|' || folder_id || '|' || date || '|' || is_read || '|' || is_deleted || '|' || thread_key
             FROM message_folders ORDER BY message_id, folder_id",
        "SELECT account_id || '|' || normalized_address || '|' || address || '|' || COALESCE(name, '<null>') || '|' || last_interaction_at
             FROM message_correspondents ORDER BY account_id, normalized_address",
        "SELECT type || '|' || name FROM sqlite_master
             WHERE name NOT LIKE 'sqlite_%' AND name != 'schema_migration_progress' ORDER BY type, name",
        "SELECT group_concat(name) FROM pragma_table_info('messages')",
    ] {
        let mut stmt = conn.prepare(sql).unwrap();
        let rows = stmt.query_map([], |row| row.get::<_, String>(0)).unwrap();
        out.push(format!("-- {sql}"));
        out.extend(rows.map(|row| row.unwrap()));
    }
    out
}

/// Every message's body must be readable at every point of an interrupted
/// V21: still in `messages`, or already in `message_bodies`.
fn assert_no_body_lost(conn: &Connection, expected: &[(String, String)]) {
    let has_column = column_exists(conn, "messages", "body_text").unwrap();
    let has_table = table_exists(conn, "message_bodies").unwrap();
    for (id, body) in expected {
        let in_messages: Option<String> = if has_column {
            conn.query_row("SELECT body_text FROM messages WHERE id = ?1", [id], |r| {
                r.get(0)
            })
            .ok()
            .filter(|b: &String| !b.is_empty())
        } else {
            None
        };
        let in_bodies: Option<String> = if has_table {
            conn.query_row(
                "SELECT body_text FROM message_bodies WHERE message_id = ?1",
                [id],
                |r| r.get(0),
            )
            .ok()
        } else {
            None
        };
        assert_eq!(
            in_bodies.or(in_messages).as_deref(),
            Some(body.as_str()),
            "body of {id} lost"
        );
    }
}

struct V21Hooks;
impl V21Hooks {
    fn set(fail: Option<(i64, u32)>, batch: Option<i64>) -> Self {
        super::v21::FAIL_AFTER_BATCHES.with(|c| c.set(fail));
        super::v21::BATCH_OVERRIDE.with(|c| c.set(batch));
        Self
    }
}
impl Drop for V21Hooks {
    fn drop(&mut self) {
        super::v21::FAIL_AFTER_BATCHES.with(|c| c.set(None));
        super::v21::BATCH_OVERRIDE.with(|c| c.set(None));
    }
}

fn bulk_v20_db(n: usize) -> (Connection, Vec<(String, String)>) {
    let conn = Connection::open_in_memory().unwrap();
    run_migrations_up_to(&conn, 20).unwrap();
    seed_v20_mailbox(&conn);
    seed_bulk_v20(&conn, n, 64);
    let mut stmt = conn.prepare("SELECT id, body_text FROM messages").unwrap();
    let bodies = stmt
        .query_map([], |r| Ok((r.get(0)?, r.get(1)?)))
        .unwrap()
        .map(|row| row.unwrap())
        .collect();
    drop(stmt);
    (conn, bodies)
}

#[test]
fn v21_resumes_after_interruption_at_any_stage_or_batch() {
    let reference = {
        let _hooks = V21Hooks::set(None, Some(4));
        let (conn, _) = bulk_v20_db(23);
        run_migrations(&conn).unwrap();
        v21_snapshot(&conn)
    };
    // Batch size must not change the outcome either.
    {
        let (conn, _) = bulk_v20_db(23);
        run_migrations(&conn).unwrap();
        assert_eq!(v21_snapshot(&conn), reference);
    }

    // Stages: 0 setup, 1 bodies, 2 folders, 3 triggers, 4 correspondents.
    for stage in 0..=4 {
        // Setup and triggers are single transactions.
        let batch_points: &[u32] = if matches!(stage, 0 | 3) {
            &[0]
        } else {
            &[0, 1, 3]
        };
        for &batches in batch_points {
            let (conn, bodies) = bulk_v20_db(23);
            {
                let _hooks = V21Hooks::set(Some((stage, batches)), Some(4));
                let err = run_migrations(&conn).unwrap_err().to_string();
                assert!(err.contains("injected failure"), "{err}");
            }
            assert_eq!(get_schema_version(&conn), 20, "stage {stage}/{batches}");
            assert_no_body_lost(&conn, &bodies);
            if stage > 0 || batches > 0 {
                assert!(super::v21::migration_in_progress(&conn).unwrap());
            }

            // A second interruption later on, then a clean finish.
            {
                let _hooks = V21Hooks::set(Some((4, 1)), Some(4));
                let _ = run_migrations(&conn);
            }
            assert_no_body_lost(&conn, &bodies);
            {
                let _hooks = V21Hooks::set(None, Some(4));
                run_migrations(&conn).unwrap();
            }
            assert_eq!(get_schema_version(&conn), CURRENT_VERSION);
            assert!(!super::v21::migration_in_progress(&conn).unwrap());
            assert_eq!(
                v21_snapshot(&conn),
                reference,
                "resume after stage {stage}, {batches} batches"
            );
        }
    }
}

#[test]
fn interrupted_v21_is_not_backed_up_again_on_resume() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pebble.db");
    {
        let conn = Connection::open(&path).unwrap();
        run_migrations_up_to(&conn, 20).unwrap();
        seed_v20_mailbox(&conn);
        seed_bulk_v20(&conn, 10, 64);
        let _hooks = V21Hooks::set(Some((2, 1)), Some(4));
        assert!(run_migrations(&conn).is_err());
    }
    let backups = || {
        std::fs::read_dir(dir.path())
            .unwrap()
            .filter(|e| {
                e.as_ref()
                    .unwrap()
                    .file_name()
                    .to_string_lossy()
                    .ends_with(".bak")
            })
            .count()
    };
    assert_eq!(backups(), 1);
    let conn = Connection::open(&path).unwrap();
    run_migrations(&conn).unwrap();
    assert_eq!(get_schema_version(&conn), CURRENT_VERSION);
    assert_eq!(
        backups(),
        1,
        "the pre-migration backup must stay the only one"
    );
    // The kept backup is the untouched v20 database.
    let backup = std::fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().path())
        .find(|p| p.to_string_lossy().ends_with(".bak"))
        .unwrap();
    let old = Connection::open(backup).unwrap();
    assert_eq!(get_schema_version(&old), 20);
    assert!(column_exists(&old, "messages", "body_text").unwrap());
}

/// Real-scale upgrade benchmark. Run with
/// `PEBBLE_MIGRATION_BENCH_MESSAGES=200000 cargo test -p pebble-store --release
///  v21_large_mailbox_benchmark -- --ignored --nocapture`.
/// Reports time, file sizes, the WAL high-water mark and peak RSS, and fails
/// if the WAL grows beyond a fraction of the database (i.e. the rewrite was
/// not batched).
#[test]
#[ignore = "benchmark: builds a large on-disk mailbox"]
fn v21_large_mailbox_benchmark() {
    use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
    use std::sync::Arc;

    let n: usize = std::env::var("PEBBLE_MIGRATION_BENCH_MESSAGES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(100_000);
    let body_bytes: usize = std::env::var("PEBBLE_MIGRATION_BENCH_BODY_BYTES")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(6_000);
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("pebble.db");
    let wal = dir.path().join("pebble.db-wal");
    {
        let conn = Connection::open(&path).unwrap();
        run_migrations_up_to(&conn, 20).unwrap();
        seed_v20_mailbox(&conn);
        seed_bulk_v20(&conn, n, body_bytes);
        conn.query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
            .unwrap();
    }
    let db_before = std::fs::metadata(&path).unwrap().len();

    fn rss_kib() -> u64 {
        std::process::Command::new("ps")
            .args(["-o", "rss=", "-p", &std::process::id().to_string()])
            .output()
            .ok()
            .and_then(|o| String::from_utf8(o.stdout).ok())
            .and_then(|s| s.trim().parse().ok())
            .unwrap_or(0)
    }
    let stop = Arc::new(AtomicBool::new(false));
    let wal_peak = Arc::new(AtomicU64::new(0));
    let stage_peaks: Arc<Vec<AtomicU64>> = Arc::new((0..8).map(|_| AtomicU64::new(0)).collect());
    let rss_peak = Arc::new(AtomicU64::new(rss_kib()));
    let rss_start = rss_peak.load(Ordering::Relaxed);
    let sampler = {
        let (stop, wal_peak, rss_peak, wal) = (
            stop.clone(),
            wal_peak.clone(),
            rss_peak.clone(),
            wal.clone(),
        );
        let stage_peaks = stage_peaks.clone();
        std::thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if let Ok(meta) = std::fs::metadata(&wal) {
                    wal_peak.fetch_max(meta.len(), Ordering::Relaxed);
                    let stage = super::v21::CURRENT_STAGE.load(Ordering::Relaxed);
                    stage_peaks[(stage + 1).clamp(0, 7) as usize]
                        .fetch_max(meta.len(), Ordering::Relaxed);
                }
                rss_peak.fetch_max(rss_kib(), Ordering::Relaxed);
                std::thread::sleep(std::time::Duration::from_millis(20));
            }
        })
    };

    let started = std::time::Instant::now();
    let conn = Connection::open(&path).unwrap();
    run_migrations(&conn).unwrap();
    let elapsed = started.elapsed();
    stop.store(true, Ordering::Relaxed);
    sampler.join().unwrap();
    assert_eq!(get_schema_version(&conn), CURRENT_VERSION);

    let db_after = std::fs::metadata(&path).unwrap().len();
    let backup_bytes: u64 = std::fs::read_dir(dir.path())
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_string_lossy().ends_with(".bak"))
        .map(|e| e.metadata().unwrap().len())
        .sum();
    let wal_peak = wal_peak.load(Ordering::Relaxed);
    let mib = |b: u64| b as f64 / 1_048_576.0;
    println!(
        "V21 benchmark: {n} messages x {body_bytes} B bodies\n  time      {:.1}s\n  db        {:.0} MiB -> {:.0} MiB\n  backup    {:.0} MiB\n  WAL peak  {:.1} MiB\n  RSS       {:.0} MiB -> peak {:.0} MiB",
        elapsed.as_secs_f64(),
        mib(db_before),
        mib(db_after),
        mib(backup_bytes),
        mib(wal_peak),
        rss_start as f64 / 1024.0,
        rss_peak.load(Ordering::Relaxed) as f64 / 1024.0,
    );
    for (index, peak) in stage_peaks.iter().enumerate() {
        println!(
            "  WAL peak in stage {:>2}: {:.1} MiB",
            index as i64 - 1,
            mib(peak.load(Ordering::Relaxed))
        );
    }
    assert!(
        wal_peak < db_before / 4,
        "WAL peaked at {:.1} MiB for a {:.0} MiB database: the rewrite is not batched",
        mib(wal_peak),
        mib(db_before)
    );
}
