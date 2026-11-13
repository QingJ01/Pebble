-- Append as a separate numbered migration after the send-controls migration.
-- No FK to accounts: deleting/rebuilding a provider cache must not delete archives.
CREATE TABLE mail_archives (
    id TEXT PRIMARY KEY,
    account_id TEXT NOT NULL,
    account_label TEXT NOT NULL,
    name TEXT NOT NULL,
    created_at INTEGER NOT NULL
);
CREATE TABLE mail_archive_folders (
    id TEXT PRIMARY KEY,
    archive_id TEXT NOT NULL REFERENCES mail_archives(id) ON DELETE CASCADE,
    name TEXT NOT NULL,
    UNIQUE(archive_id, name),
    UNIQUE(id, archive_id)
);
CREATE TABLE mail_archive_messages (
    id TEXT PRIMARY KEY,
    archive_id TEXT NOT NULL REFERENCES mail_archives(id) ON DELETE CASCADE,
    folder_id TEXT NOT NULL,
    fingerprint TEXT NOT NULL,
    raw_mime BLOB NOT NULL CHECK(length(raw_mime) <= 33554432),
    metadata TEXT NOT NULL,
    subject TEXT NOT NULL,
    sender TEXT NOT NULL,
    date INTEGER NOT NULL,
    is_read INTEGER NOT NULL DEFAULT 0,
    is_starred INTEGER NOT NULL DEFAULT 0,
    FOREIGN KEY(folder_id, archive_id) REFERENCES mail_archive_folders(id, archive_id),
    UNIQUE(archive_id, fingerprint)
);
CREATE INDEX mail_archive_messages_folder ON mail_archive_messages(folder_id, date DESC, id);
CREATE INDEX mail_archive_messages_content ON mail_archive_messages(fingerprint, archive_id);
CREATE INDEX mail_archives_account ON mail_archives(account_id);
CREATE VIRTUAL TABLE mail_archive_fts USING fts5(message_id UNINDEXED, text, tokenize='trigram');
CREATE TRIGGER mail_archive_messages_delete AFTER DELETE ON mail_archive_messages BEGIN
    DELETE FROM mail_archive_fts WHERE message_id = old.id;
END;
