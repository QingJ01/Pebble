pub mod accounts;
pub mod folders;
pub mod messages;
pub mod migrations;

use pebble_core::{PebbleError, Result};
use rusqlite::Connection;
use std::path::Path;
use std::sync::Mutex;

pub struct Store {
    conn: Mutex<Connection>,
}

impl Store {
    pub fn open(path: &Path) -> Result<Self> {
        let conn =
            Connection::open(path).map_err(|e| PebbleError::Storage(e.to_string()))?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.initialize()?;
        Ok(store)
    }

    pub fn open_in_memory() -> Result<Self> {
        let conn =
            Connection::open_in_memory().map_err(|e| PebbleError::Storage(e.to_string()))?;
        let store = Self {
            conn: Mutex::new(conn),
        };
        store.initialize()?;
        Ok(store)
    }

    fn initialize(&self) -> Result<()> {
        let conn = self.conn.lock().unwrap();
        migrations::run_migrations(&conn)
    }

    pub(crate) fn with_conn<F, T>(&self, f: F) -> Result<T>
    where
        F: FnOnce(&Connection) -> Result<T>,
    {
        let conn = self.conn.lock().unwrap();
        f(&conn)
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
