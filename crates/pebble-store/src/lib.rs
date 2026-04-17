pub mod accounts;
pub use accounts::SyncState;
pub mod attachments;
pub mod auth_data;
pub mod cloud_sync;
pub mod contacts;
pub mod folders;
pub mod kanban;
pub mod labels;
pub mod messages;
pub mod migrations;
pub mod pending_ops;
pub mod rules;
pub mod search_pending;
pub mod snooze;
pub mod sync_failures;
pub mod translate_config;
pub mod trusted_senders;

use pebble_core::{PebbleError, Result};
use r2d2::Pool;
use r2d2_sqlite::SqliteConnectionManager;
use rusqlite::{Connection, OpenFlags};
use std::path::Path;
use std::sync::Arc;

pub struct Store {
    read_pool: Pool<SqliteConnectionManager>,
    write_pool: Pool<SqliteConnectionManager>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let write_manager = SqliteConnectionManager::file(path).with_init(|conn| {
            conn.execute_batch(
                "PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000; PRAGMA synchronous=NORMAL;",
            )?;
            Ok(())
        });
        let write_pool = Pool::builder()
            .max_size(1)
            .build(write_manager)
            .map_err(|e| PebbleError::Storage(format!("Failed to create write pool: {e}")))?;

        let read_manager = SqliteConnectionManager::file(path).with_init(|conn| {
            conn.execute_batch(
                "PRAGMA foreign_keys=ON; PRAGMA busy_timeout=5000; PRAGMA synchronous=NORMAL;",
            )?;
            Ok(())
        });
        let read_pool = Pool::builder()
            .max_size(4)
            .build(read_manager)
            .map_err(|e| PebbleError::Storage(format!("Failed to create read pool: {e}")))?;

        let store = Self {
            read_pool,
            write_pool,
        };
        store.initialize()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self> {
        // Use a uniquely-named shared-cache in-memory database so all
        // connections in both pools see the same data.
        let db_name = format!(
            "file:pebble_{}?mode=memory&cache=shared",
            pebble_core::new_id()
        );
        let flags = OpenFlags::SQLITE_OPEN_READ_WRITE
            | OpenFlags::SQLITE_OPEN_CREATE
            | OpenFlags::SQLITE_OPEN_URI
            | OpenFlags::SQLITE_OPEN_NO_MUTEX;

        let write_manager = SqliteConnectionManager::file(&db_name)
            .with_flags(flags)
            .with_init(|conn| {
                conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=NORMAL;")?;
                Ok(())
            });
        let write_pool = Pool::builder()
            .max_size(1)
            .build(write_manager)
            .map_err(|e| PebbleError::Storage(format!("Failed to create write pool: {e}")))?;

        let read_manager = SqliteConnectionManager::file(&db_name)
            .with_flags(flags)
            .with_init(|conn| {
                conn.execute_batch("PRAGMA foreign_keys=ON; PRAGMA synchronous=NORMAL;")?;
                Ok(())
            });
        let read_pool = Pool::builder()
            .max_size(2)
            .build(read_manager)
            .map_err(|e| PebbleError::Storage(format!("Failed to create read pool: {e}")))?;

        let store = Self {
            read_pool,
            write_pool,
        };
        store.initialize()?;
        Ok(store)
    }

    fn initialize(&self) -> Result<()> {
        let conn = self
            .write_pool
            .get()
            .map_err(|e| PebbleError::Internal(format!("Pool error: {e}")))?;
        migrations::run_migrations(&conn)
    }

    /// Obtain a read-only connection from the pool.
    pub(crate) fn with_read<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T>,
    {
        let conn = self
            .read_pool
            .get()
            .map_err(|e| PebbleError::Internal(format!("Pool error: {e}")))?;
        f(&conn)
    }

    /// Obtain the write connection from the pool.
    pub(crate) fn with_write<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T>,
    {
        let conn = self
            .write_pool
            .get()
            .map_err(|e| PebbleError::Internal(format!("Pool error: {e}")))?;
        f(&conn)
    }

    /// Run a closure against a read connection on a blocking thread.
    /// Use from async sync workers to avoid blocking the Tokio runtime.
    pub async fn with_read_async<F, T>(self: &Arc<Self>, f: F) -> Result<T>
    where
        F: FnOnce(&rusqlite::Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let store = Arc::clone(self);
        tokio::task::spawn_blocking(move || store.with_read(f))
            .await
            .map_err(|e| PebbleError::Internal(format!("spawn_blocking: {e}")))?
    }

    /// Run a closure against the write connection on a blocking thread.
    /// Use from async sync workers to avoid blocking the Tokio runtime.
    pub async fn with_write_async<F, T>(self: &Arc<Self>, f: F) -> Result<T>
    where
        F: FnOnce(&rusqlite::Connection) -> Result<T> + Send + 'static,
        T: Send + 'static,
    {
        let store = Arc::clone(self);
        tokio::task::spawn_blocking(move || store.with_write(f))
            .await
            .map_err(|e| PebbleError::Internal(format!("spawn_blocking: {e}")))?
    }

    /// Run VACUUM to reclaim space from soft-deleted rows.
    pub fn vacuum(&self) -> Result<()> {
        self.with_write(|conn| {
            conn.execute_batch("VACUUM")?;
            Ok(())
        })
    }

    /// Run a quick integrity check on the database.
    /// Returns the check result string (normally "ok").
    pub fn quick_check(&self) -> Result<String> {
        self.with_read(|conn| {
            conn.query_row("PRAGMA quick_check", [], |row| row.get::<_, String>(0))
                .map_err(|e| PebbleError::Storage(e.to_string()))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_core::ProviderType;

    #[test]
    fn test_open_in_memory() {
        let store = Store::open_in_memory();
        assert!(store.is_ok());
    }

    #[test]
    fn test_account_crud() {
        let store = Store::open_in_memory().unwrap();
        let account = pebble_core::Account {
            id: pebble_core::new_id(),
            email: "test@example.com".to_string(),
            display_name: "Test User".to_string(),
            provider: ProviderType::Imap,
            created_at: pebble_core::now_timestamp(),
            updated_at: pebble_core::now_timestamp(),
        };
        store.insert_account(&account).unwrap();
        let fetched = store.get_account(&account.id).unwrap();
        assert!(fetched.is_some());
        let fetched = fetched.unwrap();
        assert_eq!(fetched.email, "test@example.com");
        assert_eq!(fetched.provider, ProviderType::Imap);
        let accounts = store.list_accounts().unwrap();
        assert_eq!(accounts.len(), 1);
        store.delete_account(&account.id).unwrap();
        let accounts = store.list_accounts().unwrap();
        assert_eq!(accounts.len(), 0);
    }

    #[test]
    fn test_folder_crud() {
        let store = Store::open_in_memory().unwrap();
        let account = pebble_core::Account {
            id: pebble_core::new_id(),
            email: "test@example.com".to_string(),
            display_name: "Test".to_string(),
            provider: ProviderType::Imap,
            created_at: pebble_core::now_timestamp(),
            updated_at: pebble_core::now_timestamp(),
        };
        store.insert_account(&account).unwrap();
        let folder = pebble_core::Folder {
            id: pebble_core::new_id(),
            account_id: account.id.clone(),
            remote_id: "INBOX".to_string(),
            name: "Inbox".to_string(),
            folder_type: pebble_core::FolderType::Folder,
            role: Some(pebble_core::FolderRole::Inbox),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 0,
        };
        store.insert_folder(&folder).unwrap();
        let folders = store.list_folders(&account.id).unwrap();
        assert_eq!(folders.len(), 1);
        assert_eq!(folders[0].name, "Inbox");
        assert_eq!(folders[0].role, Some(pebble_core::FolderRole::Inbox));
    }

    #[test]
    fn test_message_insert_and_query() {
        let store = Store::open_in_memory().unwrap();
        let now = pebble_core::now_timestamp();
        let account = pebble_core::Account {
            id: pebble_core::new_id(),
            email: "test@example.com".to_string(),
            display_name: "Test".to_string(),
            provider: ProviderType::Imap,
            created_at: now,
            updated_at: now,
        };
        store.insert_account(&account).unwrap();
        let folder = pebble_core::Folder {
            id: pebble_core::new_id(),
            account_id: account.id.clone(),
            remote_id: "INBOX".to_string(),
            name: "Inbox".to_string(),
            folder_type: pebble_core::FolderType::Folder,
            role: Some(pebble_core::FolderRole::Inbox),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 0,
        };
        store.insert_folder(&folder).unwrap();
        let msg = pebble_core::Message {
            id: pebble_core::new_id(),
            account_id: account.id.clone(),
            remote_id: "12345".to_string(),
            message_id_header: Some("<abc@example.com>".to_string()),
            in_reply_to: None,
            references_header: None,
            thread_id: None,
            subject: "Hello World".to_string(),
            snippet: "This is a test...".to_string(),
            from_address: "sender@example.com".to_string(),
            from_name: "Sender".to_string(),
            to_list: vec![pebble_core::EmailAddress {
                name: Some("Test".to_string()),
                address: "test@example.com".to_string(),
            }],
            cc_list: vec![],
            bcc_list: vec![],
            body_text: "This is a test email.".to_string(),
            body_html_raw: "<p>This is a test email.</p>".to_string(),
            has_attachments: false,
            is_read: false,
            is_starred: false,
            is_draft: false,
            date: now,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        };
        store.insert_message(&msg, &[folder.id.clone()]).unwrap();
        let messages = store.list_messages_by_folder(&folder.id, 50, 0).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].subject, "Hello World");
        assert_eq!(messages[0].from_address, "sender@example.com");
    }
}
