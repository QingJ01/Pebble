use pebble_core::{PebbleError, Result};
use rusqlite::Connection;

const CURRENT_VERSION: u32 = 6;

fn get_schema_version(conn: &Connection) -> u32 {
    conn.pragma_query_value(None, "user_version", |row| row.get(0))
        .unwrap_or(0)
}

fn set_schema_version(conn: &Connection, version: u32) -> Result<()> {
    conn.pragma_update(None, "user_version", version)
        .map_err(|e| PebbleError::Storage(format!("Failed to set schema version: {e}")))
}

pub fn run_migrations(conn: &Connection) -> Result<()> {
    conn.execute_batch("PRAGMA journal_mode=WAL;")?;
    conn.execute_batch("PRAGMA synchronous=NORMAL;")?;

    conn.execute_batch("PRAGMA foreign_keys=ON;")?;

    conn.execute_batch("PRAGMA busy_timeout=5000;")?;

    let version = get_schema_version(conn);

    // Each migration is wrapped in a transaction so that the DDL and version
    // update are atomic — a crash mid-migration won't leave an inconsistent state.

    if version < 1 {
        let tx = conn.unchecked_transaction()
            .map_err(|e| PebbleError::Storage(format!("Migration V1 begin failed: {e}")))?;
        tx.execute_batch(SCHEMA_V1)
            .map_err(|e| PebbleError::Storage(format!("Migration V1 failed: {e}")))?;
        set_schema_version(&tx, 1)?;
        tx.commit().map_err(|e| PebbleError::Storage(format!("Migration V1 commit failed: {e}")))?;
    }

    if version < 2 {
        let tx = conn.unchecked_transaction()
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
        tx.commit().map_err(|e| PebbleError::Storage(format!("Migration V2 commit failed: {e}")))?;
    }

    if version < 3 {
        let tx = conn.unchecked_transaction()
            .map_err(|e| PebbleError::Storage(format!("Migration V3 begin failed: {e}")))?;
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_messages_account_remote ON messages(account_id, remote_id);
             CREATE INDEX IF NOT EXISTS idx_snoozed_unsnoozed_at ON snoozed_messages(unsnoozed_at);
             CREATE UNIQUE INDEX IF NOT EXISTS idx_folders_account_remote ON folders(account_id, remote_id);"
        )
        .map_err(|e| PebbleError::Storage(format!("Migration V3 failed: {e}")))?;
        set_schema_version(&tx, 3)?;
        tx.commit().map_err(|e| PebbleError::Storage(format!("Migration V3 commit failed: {e}")))?;
    }

    if version < 4 {
        let tx = conn.unchecked_transaction()
            .map_err(|e| PebbleError::Storage(format!("Migration V4 begin failed: {e}")))?;
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_message_folders_folder_id ON message_folders(folder_id);
             CREATE INDEX IF NOT EXISTS idx_messages_account_starred ON messages(account_id, is_starred) WHERE is_starred = 1 AND is_deleted = 0;
             CREATE INDEX IF NOT EXISTS idx_messages_thread_date ON messages(thread_id, date) WHERE thread_id IS NOT NULL AND is_deleted = 0;"
        )
        .map_err(|e| PebbleError::Storage(format!("Migration V4 failed: {e}")))?;
        set_schema_version(&tx, 4)?;
        tx.commit().map_err(|e| PebbleError::Storage(format!("Migration V4 commit failed: {e}")))?;
    }

    if version < 5 {
        let tx = conn.unchecked_transaction()
            .map_err(|e| PebbleError::Storage(format!("Migration V5 begin failed: {e}")))?;
        tx.execute_batch(
            "CREATE INDEX IF NOT EXISTS idx_mf_folder_message ON message_folders(folder_id, message_id);",
        )
        .map_err(|e| PebbleError::Storage(format!("Migration V5 failed: {e}")))?;
        set_schema_version(&tx, 5)?;
        tx.commit().map_err(|e| PebbleError::Storage(format!("Migration V5 commit failed: {e}")))?;
    }

    // V6: search_pending table for crash-recovery of the search index
    if version < 6 {
        let tx = conn.unchecked_transaction()
            .map_err(|e| PebbleError::Storage(format!("Migration V6 begin failed: {e}")))?;
        tx.execute_batch(
            "CREATE TABLE IF NOT EXISTS search_pending (
                 message_id TEXT PRIMARY KEY,
                 operation TEXT NOT NULL CHECK(operation IN ('index', 'remove')),
                 created_at INTEGER NOT NULL
             );",
        )
        .map_err(|e| PebbleError::Storage(format!("Migration V6 failed: {e}")))?;
        set_schema_version(&tx, CURRENT_VERSION)?;
        tx.commit().map_err(|e| PebbleError::Storage(format!("Migration V6 commit failed: {e}")))?;
    }

    Ok(())
}

const SCHEMA_V1: &str = r#"
CREATE TABLE IF NOT EXISTS accounts (
    id TEXT PRIMARY KEY,
    email TEXT NOT NULL,
    display_name TEXT NOT NULL DEFAULT '',
    provider TEXT NOT NULL CHECK(provider IN ('imap', 'gmail', 'outlook')),
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
