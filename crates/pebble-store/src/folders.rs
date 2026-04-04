use pebble_core::{Folder, FolderRole, FolderType, PebbleError, Result};

use crate::Store;

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
    pub fn insert_folder(&self, folder: &Folder) -> Result<()> {
        self.with_conn(|conn| {
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
            )
            .map_err(|e| PebbleError::Storage(e.to_string()))?;
            Ok(())
        })
    }

    pub fn list_folders(&self, account_id: &str) -> Result<Vec<Folder>> {
        self.with_conn(|conn| {
            let mut stmt = conn
                .prepare(
                    "SELECT id, account_id, remote_id, name, folder_type, role, parent_id, color, is_system, sort_order
                     FROM folders WHERE account_id = ?1 ORDER BY sort_order ASC",
                )
                .map_err(|e| PebbleError::Storage(e.to_string()))?;
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
                })
                .map_err(|e| PebbleError::Storage(e.to_string()))?;
            let mut folders = Vec::new();
            for row in rows {
                folders.push(row.map_err(|e| PebbleError::Storage(e.to_string()))?);
            }
            Ok(folders)
        })
    }
}
