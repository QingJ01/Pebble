use pebble_core::Result;
use rusqlite::{params, OptionalExtension};

use crate::Store;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncFailure {
    pub account_id: String,
    pub folder_id: String,
    pub remote_id: String,
    pub provider: String,
    pub reason: String,
    pub attempts: i64,
    pub updated_at: i64,
}

impl Store {
    pub fn upsert_sync_failure(
        &self,
        account_id: &str,
        folder_id: &str,
        remote_id: &str,
        provider: &str,
        reason: &str,
    ) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "INSERT INTO sync_failures
                    (account_id, folder_id, remote_id, provider, reason, attempts, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, 1, ?6)
                 ON CONFLICT(account_id, folder_id, remote_id) DO UPDATE SET
                    provider = excluded.provider,
                    reason = excluded.reason,
                    attempts = sync_failures.attempts + 1,
                    updated_at = excluded.updated_at",
                params![
                    account_id,
                    folder_id,
                    remote_id,
                    provider,
                    reason,
                    pebble_core::now_timestamp(),
                ],
            )?;
            Ok(())
        })
    }

    pub fn get_sync_failure(
        &self,
        account_id: &str,
        folder_id: &str,
        remote_id: &str,
    ) -> Result<Option<SyncFailure>> {
        self.with_read(|conn| {
            let failure = conn
                .query_row(
                    "SELECT account_id, folder_id, remote_id, provider, reason, attempts, updated_at
                     FROM sync_failures
                     WHERE account_id = ?1 AND folder_id = ?2 AND remote_id = ?3",
                    params![account_id, folder_id, remote_id],
                    |row| {
                        Ok(SyncFailure {
                            account_id: row.get(0)?,
                            folder_id: row.get(1)?,
                            remote_id: row.get(2)?,
                            provider: row.get(3)?,
                            reason: row.get(4)?,
                            attempts: row.get(5)?,
                            updated_at: row.get(6)?,
                        })
                    },
                )
                .optional()?;
            Ok(failure)
        })
    }

    pub fn has_sync_failures_for_folder(&self, account_id: &str, folder_id: &str) -> Result<bool> {
        self.with_read(|conn| {
            let count: i64 = conn.query_row(
                "SELECT COUNT(*)
                 FROM sync_failures
                 WHERE account_id = ?1 AND folder_id = ?2",
                params![account_id, folder_id],
                |row| row.get(0),
            )?;
            Ok(count > 0)
        })
    }

    pub fn clear_sync_failure(
        &self,
        account_id: &str,
        folder_id: &str,
        remote_id: &str,
    ) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "DELETE FROM sync_failures
                 WHERE account_id = ?1 AND folder_id = ?2 AND remote_id = ?3",
                params![account_id, folder_id, remote_id],
            )?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_core::*;

    fn test_account() -> Account {
        Account {
            id: new_id(),
            email: "test@example.com".to_string(),
            display_name: "Test".to_string(),
            provider: ProviderType::Imap,
            created_at: now_timestamp(),
            updated_at: now_timestamp(),
        }
    }

    fn test_folder(account_id: &str) -> Folder {
        Folder {
            id: new_id(),
            account_id: account_id.to_string(),
            remote_id: "INBOX".to_string(),
            name: "Inbox".to_string(),
            folder_type: FolderType::Folder,
            role: Some(FolderRole::Inbox),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 0,
        }
    }

    #[test]
    fn sync_failure_upsert_increments_attempts() {
        let store = Store::open_in_memory().unwrap();
        let account = test_account();
        store.insert_account(&account).unwrap();
        let folder = test_folder(&account.id);
        store.insert_folder(&folder).unwrap();

        store
            .upsert_sync_failure(&account.id, &folder.id, "123", "imap", "parse failed")
            .unwrap();
        store
            .upsert_sync_failure(&account.id, &folder.id, "123", "imap", "parse failed again")
            .unwrap();

        let failure = store
            .get_sync_failure(&account.id, &folder.id, "123")
            .unwrap()
            .unwrap();

        assert_eq!(failure.account_id, account.id);
        assert_eq!(failure.folder_id, folder.id);
        assert_eq!(failure.remote_id, "123");
        assert_eq!(failure.provider, "imap");
        assert_eq!(failure.reason, "parse failed again");
        assert_eq!(failure.attempts, 2);
        assert!(store
            .has_sync_failures_for_folder(&account.id, &folder.id)
            .unwrap());
    }
}
