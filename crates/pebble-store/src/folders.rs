use pebble_core::{Folder, FolderRole, FolderType, PebbleError, Result};
use rusqlite::{params, OptionalExtension};
use std::collections::{BTreeSet, HashSet};

use crate::{accounts::SyncState, Store};

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImapFolderSelectionChange {
    /// Messages whose folder membership changed and whose search documents
    /// therefore need to be refreshed.
    pub affected_message_ids: Vec<String>,
    /// Messages that no longer belong to any folder and were physically
    /// removed along with their attachment records.
    pub deleted_message_ids: Vec<String>,
}

fn folder_type_to_str(ft: &FolderType) -> &'static str {
    match ft {
        FolderType::Folder => "folder",
        FolderType::Label => "label",
        FolderType::Category => "category",
    }
}

fn str_to_folder_type(s: &str) -> FolderType {
    match s {
        "label" => FolderType::Label,
        "category" => FolderType::Category,
        _ => FolderType::Folder,
    }
}

fn folder_role_to_str(role: &FolderRole) -> &'static str {
    match role {
        FolderRole::Inbox => "inbox",
        FolderRole::Sent => "sent",
        FolderRole::Drafts => "drafts",
        FolderRole::Trash => "trash",
        FolderRole::Archive => "archive",
        FolderRole::Spam => "spam",
    }
}

fn str_to_folder_role(s: &str) -> Option<FolderRole> {
    match s {
        "inbox" => Some(FolderRole::Inbox),
        "sent" => Some(FolderRole::Sent),
        "drafts" => Some(FolderRole::Drafts),
        "trash" => Some(FolderRole::Trash),
        "archive" => Some(FolderRole::Archive),
        "spam" => Some(FolderRole::Spam),
        _ => None,
    }
}

impl Store {
    /// Upsert a folder. Returns the effective database id (the existing row's id
    /// when the folder already exists, or `folder.id` for a new insert).
    pub fn insert_folder(&self, folder: &Folder) -> Result<String> {
        self.with_write(|conn| {
            // Upsert: if a folder with the same (account_id, remote_id) exists,
            // update its name/role/sort_order instead of creating a duplicate.
            let existing: Option<String> = conn
                .query_row(
                    "SELECT id FROM folders WHERE account_id = ?1 AND remote_id = ?2",
                    rusqlite::params![folder.account_id, folder.remote_id],
                    |row| row.get(0),
                )
                .optional()?;

            if let Some(existing_id) = existing {
                conn.execute(
                    "UPDATE folders SET name = ?1, folder_type = ?2, role = ?3, sort_order = ?4
                     WHERE id = ?5",
                    rusqlite::params![
                        folder.name,
                        folder_type_to_str(&folder.folder_type),
                        folder.role.as_ref().map(folder_role_to_str),
                        folder.sort_order,
                        existing_id,
                    ],
                )?;
                Ok(existing_id)
            } else {
                conn.execute(
                    "INSERT INTO folders (id, account_id, remote_id, name, folder_type, role, parent_id, color, is_system, sort_order)
                     VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                    rusqlite::params![
                        folder.id,
                        folder.account_id,
                        folder.remote_id,
                        folder.name,
                        folder_type_to_str(&folder.folder_type),
                        folder.role.as_ref().map(folder_role_to_str),
                        folder.parent_id,
                        folder.color,
                        folder.is_system as i32,
                        folder.sort_order,
                    ],
                )?;
                Ok(folder.id.clone())
            }
        })
    }

    pub fn find_folder_by_role(
        &self,
        account_id: &str,
        role: FolderRole,
    ) -> Result<Option<Folder>> {
        let role_str = folder_role_to_str(&role);
        self.with_read(|conn| {
            let mut stmt = conn
                .prepare_cached(
                    "SELECT id, account_id, remote_id, name, folder_type, role, parent_id, color, is_system, sort_order
                     FROM folders WHERE account_id = ?1 AND role = ?2
                     ORDER BY CASE WHEN remote_id LIKE '\\_\\_local\\_%' ESCAPE '\\' THEN 1 ELSE 0 END, sort_order ASC
                     LIMIT 1",
                )?;
            let result = stmt
                .query_row(params![account_id, role_str], |row| {
                    let role_val: Option<String> = row.get(5)?;
                    let is_system: i32 = row.get(8)?;
                    Ok(Folder {
                        id: row.get(0)?,
                        account_id: row.get(1)?,
                        remote_id: row.get(2)?,
                        name: row.get(3)?,
                        folder_type: str_to_folder_type(&row.get::<_, String>(4)?),
                        role: role_val.and_then(|s| str_to_folder_role(&s)),
                        parent_id: row.get(6)?,
                        color: row.get(7)?,
                        is_system: is_system != 0,
                        sort_order: row.get(9)?,
                    })
                })
                .optional()?;
            Ok(result)
        })
    }

    /// Remove an empty local fallback folder (e.g. `__local_archive__`) once a
    /// server folder with the same role exists, so role lookups and the sidebar
    /// target the server folder. Fallback folders that hold messages are kept.
    pub fn remove_empty_local_fallback_folder(
        &self,
        account_id: &str,
        local_remote_id: &str,
        role: FolderRole,
    ) -> Result<bool> {
        let role_str = folder_role_to_str(&role);
        self.with_write(|conn| {
            let removed = conn.execute(
                "DELETE FROM folders
                 WHERE account_id = ?1 AND remote_id = ?2 AND role = ?3
                   AND EXISTS (
                     SELECT 1 FROM folders f2
                     WHERE f2.account_id = ?1 AND f2.role = ?3 AND f2.remote_id NOT LIKE '\\_\\_local\\_%' ESCAPE '\\'
                   )
                   AND NOT EXISTS (
                     SELECT 1 FROM message_folders mf WHERE mf.folder_id = folders.id
                   )",
                rusqlite::params![account_id, local_remote_id, role_str],
            )?;
            Ok(removed > 0)
        })
    }

    pub fn find_folder_by_name(&self, account_id: &str, name: &str) -> Result<Option<Folder>> {
        let lower = name.to_lowercase();
        let folders = self.list_folders(account_id)?;
        Ok(folders.into_iter().find(|f| f.name.to_lowercase() == lower))
    }

    pub fn delete_folder_by_remote_id(&self, account_id: &str, remote_id: &str) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "DELETE FROM folders WHERE account_id = ?1 AND remote_id = ?2",
                rusqlite::params![account_id, remote_id],
            )?;
            Ok(())
        })
    }

    pub fn list_folders(&self, account_id: &str) -> Result<Vec<Folder>> {
        self.with_read(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, remote_id, name, folder_type, role, parent_id, color, is_system, sort_order
                     FROM folders WHERE account_id = ?1 ORDER BY sort_order ASC",
                )?;
            let rows = stmt
                .query_map(rusqlite::params![account_id], |row| {
                    let role_str: Option<String> = row.get(5)?;
                    let is_system: i32 = row.get(8)?;
                    Ok(Folder {
                        id: row.get(0)?,
                        account_id: row.get(1)?,
                        remote_id: row.get(2)?,
                        name: row.get(3)?,
                        folder_type: str_to_folder_type(&row.get::<_, String>(4)?),
                        role: role_str.and_then(|s| str_to_folder_role(&s)),
                        parent_id: row.get(6)?,
                        color: row.get(7)?,
                        is_system: is_system != 0,
                        sort_order: row.get(9)?,
                    })
                })?;
            let mut folders = Vec::new();
            for row in rows {
                folders.push(row?);
            }
            Ok(folders)
        })
    }

    /// Atomically persist an explicit IMAP mailbox selection, upsert the
    /// selected remote folders, and remove local data for deselected remote
    /// folders. Local-only folders (whose remote ID starts with `__local_`) are
    /// intentionally retained.
    pub fn apply_imap_folder_selection(
        &self,
        account_id: &str,
        remote_folders: &[Folder],
        selected_remote_ids: &[String],
    ) -> Result<ImapFolderSelectionChange> {
        if remote_folders
            .iter()
            .any(|folder| folder.account_id != account_id)
        {
            return Err(PebbleError::Validation(
                "IMAP folder selection contains a folder for another account".to_string(),
            ));
        }

        let mut selected_ordered = Vec::with_capacity(selected_remote_ids.len());
        let mut seen = HashSet::with_capacity(selected_remote_ids.len());
        for remote_id in selected_remote_ids {
            if seen.insert(remote_id.clone()) {
                selected_ordered.push(remote_id.clone());
            }
        }
        let selected: HashSet<String> = selected_ordered.iter().cloned().collect();

        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()?;

            let raw_sync_state: Option<Option<String>> = tx
                .query_row(
                    "SELECT sync_state FROM accounts WHERE id = ?1",
                    params![account_id],
                    |row| row.get(0),
                )
                .optional()?;
            let Some(raw_sync_state) = raw_sync_state else {
                return Err(PebbleError::Internal(format!(
                    "Account not found: {account_id}"
                )));
            };
            let mut sync_state = SyncState::from_json_opt(raw_sync_state.as_deref())?;
            sync_state.selected_imap_folder_remote_ids = Some(selected_ordered);
            let sync_state_json = sync_state.to_json()?;

            for folder in remote_folders
                .iter()
                .filter(|folder| selected.contains(&folder.remote_id))
            {
                let existing_id: Option<String> = tx
                    .query_row(
                        "SELECT id FROM folders WHERE account_id = ?1 AND remote_id = ?2",
                        params![account_id, folder.remote_id],
                        |row| row.get(0),
                    )
                    .optional()?;

                if let Some(existing_id) = existing_id {
                    tx.execute(
                        "UPDATE folders
                         SET name = ?1, folder_type = ?2, role = ?3, parent_id = ?4,
                             color = ?5, is_system = ?6, sort_order = ?7
                         WHERE id = ?8",
                        params![
                            folder.name,
                            folder_type_to_str(&folder.folder_type),
                            folder.role.as_ref().map(folder_role_to_str),
                            folder.parent_id,
                            folder.color,
                            folder.is_system as i32,
                            folder.sort_order,
                            existing_id,
                        ],
                    )?;
                } else {
                    tx.execute(
                        "INSERT INTO folders
                         (id, account_id, remote_id, name, folder_type, role,
                          parent_id, color, is_system, sort_order)
                         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
                        params![
                            folder.id,
                            account_id,
                            folder.remote_id,
                            folder.name,
                            folder_type_to_str(&folder.folder_type),
                            folder.role.as_ref().map(folder_role_to_str),
                            folder.parent_id,
                            folder.color,
                            folder.is_system as i32,
                            folder.sort_order,
                        ],
                    )?;
                }
            }

            let stored_remote_folders = {
                let mut stmt = tx.prepare(
                    "SELECT id, remote_id FROM folders
                     WHERE account_id = ?1",
                )?;
                let rows = stmt.query_map(params![account_id], |row| {
                    Ok((row.get::<_, String>(0)?, row.get::<_, String>(1)?))
                })?;
                let mut folders = Vec::new();
                for row in rows {
                    folders.push(row?);
                }
                folders
            };

            let removed_folder_ids: Vec<String> = stored_remote_folders
                .into_iter()
                .filter(|(_, remote_id)| {
                    !remote_id.starts_with("__local_") && !selected.contains(remote_id)
                })
                .map(|(id, _)| id)
                .collect();
            let mut affected_message_ids = BTreeSet::new();

            for folder_id in &removed_folder_ids {
                let message_ids = {
                    let mut stmt =
                        tx.prepare("SELECT message_id FROM message_folders WHERE folder_id = ?1")?;
                    let rows = stmt.query_map(params![folder_id], |row| row.get::<_, String>(0))?;
                    let mut ids = Vec::new();
                    for row in rows {
                        ids.push(row?);
                    }
                    ids
                };
                affected_message_ids.extend(message_ids);

                // Delete the junction rows explicitly so cleanup remains
                // correct even on a connection where SQLite FK cascades were
                // not enabled by an older application build.
                tx.execute(
                    "DELETE FROM message_folders WHERE folder_id = ?1",
                    params![folder_id],
                )?;
                tx.execute("DELETE FROM folders WHERE id = ?1", params![folder_id])?;
            }

            let mut deleted_message_ids = Vec::new();
            for message_id in &affected_message_ids {
                let remaining_folder_count: i64 = tx.query_row(
                    "SELECT COUNT(*) FROM message_folders WHERE message_id = ?1",
                    params![message_id],
                    |row| row.get(0),
                )?;
                if remaining_folder_count == 0 {
                    tx.execute("DELETE FROM messages WHERE id = ?1", params![message_id])?;
                    deleted_message_ids.push(message_id.clone());
                }
            }

            tx.execute(
                "UPDATE accounts SET sync_state = ?1, updated_at = ?2 WHERE id = ?3",
                params![sync_state_json, pebble_core::now_timestamp(), account_id],
            )?;

            tx.commit()?;
            Ok(ImapFolderSelectionChange {
                affected_message_ids: affected_message_ids.into_iter().collect(),
                deleted_message_ids,
            })
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_core::{now_timestamp, Account, EmailAddress, Message, ProviderType};

    fn account() -> Account {
        let now = now_timestamp();
        Account {
            account_label: None,
            provider_display_name: None,
            id: "account-1".to_string(),
            email: "user@example.com".to_string(),
            display_name: "User".to_string(),
            color: None,
            provider: ProviderType::Imap,
            created_at: now,
            updated_at: now,
        }
    }

    fn folder(account_id: &str, id: &str, remote_id: &str, role: Option<FolderRole>) -> Folder {
        Folder {
            id: id.to_string(),
            account_id: account_id.to_string(),
            remote_id: remote_id.to_string(),
            name: remote_id.to_string(),
            folder_type: FolderType::Folder,
            role,
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 0,
        }
    }

    fn message(account_id: &str, id: &str, remote_id: &str) -> Message {
        let now = now_timestamp();
        Message {
            id: id.to_string(),
            account_id: account_id.to_string(),
            remote_id: remote_id.to_string(),
            message_id_header: None,
            in_reply_to: None,
            references_header: None,
            thread_id: None,
            subject: id.to_string(),
            snippet: String::new(),
            from_address: "sender@example.com".to_string(),
            from_name: "Sender".to_string(),
            to_list: vec![EmailAddress {
                name: None,
                address: "user@example.com".to_string(),
            }],
            cc_list: vec![],
            bcc_list: vec![],
            body_text: String::new(),
            body_html_raw: String::new(),
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
        }
    }

    #[test]
    fn role_lookup_prefers_server_folder_over_local_fallback() {
        let store = Store::open_in_memory().unwrap();
        let account = account();
        store.insert_account(&account).unwrap();
        store
            .insert_folder(&folder(
                &account.id,
                "local",
                "__local_archive__",
                Some(FolderRole::Archive),
            ))
            .unwrap();
        store
            .insert_folder(&folder(
                &account.id,
                "remote",
                "Archive",
                Some(FolderRole::Archive),
            ))
            .unwrap();

        let found = store
            .find_folder_by_role(&account.id, FolderRole::Archive)
            .unwrap()
            .unwrap();
        assert_eq!(found.remote_id, "Archive");
    }

    #[test]
    fn empty_local_fallback_is_removed_only_when_server_folder_exists() {
        let store = Store::open_in_memory().unwrap();
        let account = account();
        store.insert_account(&account).unwrap();
        store
            .insert_folder(&folder(
                &account.id,
                "local",
                "__local_archive__",
                Some(FolderRole::Archive),
            ))
            .unwrap();

        assert!(!store
            .remove_empty_local_fallback_folder(
                &account.id,
                "__local_archive__",
                FolderRole::Archive
            )
            .unwrap());

        store
            .insert_folder(&folder(
                &account.id,
                "remote",
                "Archive",
                Some(FolderRole::Archive),
            ))
            .unwrap();
        assert!(store
            .remove_empty_local_fallback_folder(
                &account.id,
                "__local_archive__",
                FolderRole::Archive
            )
            .unwrap());
        let folders = store.list_folders(&account.id).unwrap();
        assert!(folders.iter().all(|f| f.remote_id != "__local_archive__"));
    }

    #[test]
    fn local_fallback_holding_messages_is_kept() {
        let store = Store::open_in_memory().unwrap();
        let account = account();
        store.insert_account(&account).unwrap();
        store
            .insert_folder(&folder(
                &account.id,
                "local",
                "__local_archive__",
                Some(FolderRole::Archive),
            ))
            .unwrap();
        store
            .insert_folder(&folder(
                &account.id,
                "remote",
                "Archive",
                Some(FolderRole::Archive),
            ))
            .unwrap();
        store
            .insert_message(&message(&account.id, "m1", "1"), &["local".to_string()])
            .unwrap();

        assert!(!store
            .remove_empty_local_fallback_folder(
                &account.id,
                "__local_archive__",
                FolderRole::Archive
            )
            .unwrap());
        assert!(store
            .list_folders(&account.id)
            .unwrap()
            .iter()
            .any(|f| f.remote_id == "__local_archive__"));
    }

    #[test]
    fn applying_imap_selection_adds_and_removes_folders_and_cleans_orphans() {
        let store = Store::open_in_memory().unwrap();
        let account = account();
        store.insert_account(&account).unwrap();
        store
            .update_account_sync_state(
                &account.id,
                r#"{"provider":"imap","custom_setting":"preserved"}"#,
            )
            .unwrap();

        let inbox = folder(
            &account.id,
            "folder-inbox",
            "INBOX",
            Some(FolderRole::Inbox),
        );
        let removed = folder(&account.id, "folder-projects", "Projects", None);
        let local_archive = folder(
            &account.id,
            "folder-local-archive",
            "__local_archive__",
            Some(FolderRole::Archive),
        );
        store.insert_folder(&inbox).unwrap();
        store.insert_folder(&removed).unwrap();
        store.insert_folder(&local_archive).unwrap();

        let removed_only = message(&account.id, "message-removed", "100");
        let shared = message(&account.id, "message-shared", "101");
        store
            .insert_message(&removed_only, std::slice::from_ref(&removed.id))
            .unwrap();
        store
            .insert_message(&shared, &[removed.id.clone(), inbox.id.clone()])
            .unwrap();

        let fresh_inbox = folder(
            &account.id,
            "fresh-inbox-id",
            "INBOX",
            Some(FolderRole::Inbox),
        );
        let fresh_removed = folder(&account.id, "fresh-project-id", "Projects", None);
        let fresh_added = folder(&account.id, "fresh-team-id", "Team", None);
        let change = store
            .apply_imap_folder_selection(
                &account.id,
                &[fresh_inbox, fresh_removed, fresh_added],
                &["INBOX".to_string(), "Team".to_string()],
            )
            .unwrap();

        assert_eq!(
            change.affected_message_ids,
            vec!["message-removed".to_string(), "message-shared".to_string()]
        );
        assert_eq!(
            change.deleted_message_ids,
            vec!["message-removed".to_string()]
        );
        assert!(store.get_message("message-removed").unwrap().is_none());
        assert!(store.get_message("message-shared").unwrap().is_some());
        assert_eq!(
            store.get_message_folder_ids("message-shared").unwrap(),
            vec![inbox.id]
        );

        let stored_remote_ids: HashSet<String> = store
            .list_folders(&account.id)
            .unwrap()
            .into_iter()
            .map(|folder| folder.remote_id)
            .collect();
        assert_eq!(
            stored_remote_ids,
            HashSet::from([
                "INBOX".to_string(),
                "Team".to_string(),
                "__local_archive__".to_string(),
            ])
        );

        let state = store.get_sync_state(&account.id).unwrap().unwrap();
        assert_eq!(
            state.selected_imap_folder_remote_ids,
            Some(vec!["INBOX".to_string(), "Team".to_string()])
        );
        assert_eq!(
            state.extra.get("custom_setting"),
            Some(&serde_json::Value::String("preserved".to_string()))
        );
    }
}
