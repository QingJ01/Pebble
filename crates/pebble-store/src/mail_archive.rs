use crate::Store;
use pebble_core::{EmailAddress, PebbleError, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

/// Owned, local-only archive storage. None of these identities are provider IDs.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MailArchive {
    pub id: String,
    pub account_id: String,
    pub account_label: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveFolder {
    pub id: String,
    pub archive_id: String,
    pub name: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveAttachment {
    pub filename: String,
    pub mime_type: String,
    pub size: usize,
    pub content_id: Option<String>,
    pub is_inline: bool,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveMetadata {
    pub message_id_header: Option<String>,
    pub subject: String,
    pub from_address: String,
    pub from_name: String,
    pub to_list: Vec<EmailAddress>,
    pub cc_list: Vec<EmailAddress>,
    pub bcc_list: Vec<EmailAddress>,
    pub date: i64,
    pub body_text: String,
    pub has_html: bool,
    pub attachments: Vec<ArchiveAttachment>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveMessageSummary {
    pub id: String,
    pub folder_id: String,
    pub subject: String,
    pub sender: String,
    pub date: i64,
    pub is_read: bool,
    pub is_starred: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ArchiveMessageDetail {
    pub id: String,
    pub folder_id: String,
    pub fingerprint: String,
    pub is_read: bool,
    pub is_starred: bool,
    pub metadata: ArchiveMetadata,
}

fn validate_name(name: &str) -> Result<&str> {
    let name = name.trim();
    if name.is_empty() || name.len() > 200 || name.chars().any(char::is_control) {
        return Err(PebbleError::Validation(
            "Invalid archive/folder name".into(),
        ));
    }
    Ok(name)
}

fn missing() -> PebbleError {
    PebbleError::Validation("Archive message or destination not found".into())
}

impl Store {
    pub fn create_mail_archive(&self, account_id: &str, name: &str) -> Result<MailArchive> {
        let name = validate_name(name)?;
        let account = self.get_account(account_id)?.ok_or_else(missing)?;
        let archive = MailArchive {
            id: pebble_core::new_id(),
            account_id: account_id.into(),
            account_label: account.email,
            name: name.into(),
        };
        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()?;
            tx.execute("INSERT INTO mail_archives (id,account_id,account_label,name,created_at) VALUES (?1,?2,?3,?4,?5)",
                params![archive.id,archive.account_id,archive.account_label,archive.name,pebble_core::now_timestamp()])?;
            tx.execute("INSERT INTO mail_archive_folders (id,archive_id,name) VALUES (?1,?2,'Imported')", params![pebble_core::new_id(),archive.id])?;
            tx.commit()?;
            Ok(archive)
        })
    }

    pub fn list_mail_archives(&self) -> Result<Vec<MailArchive>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT id,account_id,account_label,name FROM mail_archives ORDER BY created_at,id",
            )?;
            let rows = stmt.query_map([], |r| {
                Ok(MailArchive {
                    id: r.get(0)?,
                    account_id: r.get(1)?,
                    account_label: r.get(2)?,
                    name: r.get(3)?,
                })
            })?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    pub fn create_archive_folder(&self, archive_id: &str, name: &str) -> Result<ArchiveFolder> {
        let folder = ArchiveFolder {
            id: pebble_core::new_id(),
            archive_id: archive_id.into(),
            name: validate_name(name)?.into(),
        };
        self.with_write(|conn| {
            conn.execute(
                "INSERT INTO mail_archive_folders (id,archive_id,name) VALUES (?1,?2,?3)",
                params![folder.id, folder.archive_id, folder.name],
            )?;
            Ok(folder)
        })
    }

    pub fn list_archive_folders(&self, archive_id: &str) -> Result<Vec<ArchiveFolder>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare("SELECT id,archive_id,name FROM mail_archive_folders WHERE archive_id=?1 ORDER BY name,id")?;
            let rows = stmt.query_map([archive_id], |r| Ok(ArchiveFolder { id:r.get(0)?,archive_id:r.get(1)?,name:r.get(2)? }))?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    pub fn archive_has_content(&self, archive_id: &str, fingerprint: &str) -> Result<bool> {
        self.with_read(|conn| Ok(conn.query_row("SELECT EXISTS(SELECT 1 FROM mail_archive_messages m JOIN mail_archives a ON a.id=m.archive_id WHERE a.account_id=(SELECT account_id FROM mail_archives WHERE id=?1) AND m.fingerprint=?2)", params![archive_id,fingerprint], |r| r.get(0))?))
    }

    /// MIME, metadata and index either all commit, or none do. Duplicate scope is
    /// the account's local archives, not Message-ID or folder.
    pub fn insert_archive_message(
        &self,
        archive_id: &str,
        folder_id: &str,
        fingerprint: &str,
        raw: &[u8],
        metadata: &ArchiveMetadata,
    ) -> Result<bool> {
        if raw.is_empty()
            || raw.len() > 32 * 1024 * 1024
            || fingerprint.len() != 64
            || !fingerprint.bytes().all(|c| c.is_ascii_hexdigit())
        {
            return Err(PebbleError::Validation(
                "Invalid archive MIME or fingerprint".into(),
            ));
        }
        let metadata_json =
            serde_json::to_string(metadata).map_err(|e| PebbleError::Storage(e.to_string()))?;
        let searchable = format!(
            "{} {} {} {} {} {} {}",
            metadata.subject,
            metadata.from_name,
            metadata.from_address,
            serde_json::to_string(&metadata.to_list).unwrap_or_default(),
            serde_json::to_string(&metadata.cc_list).unwrap_or_default(),
            serde_json::to_string(&metadata.bcc_list).unwrap_or_default(),
            metadata.body_text
        );
        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()?;
            let destination:bool = tx.query_row("SELECT EXISTS(SELECT 1 FROM mail_archive_folders WHERE id=?1 AND archive_id=?2)",params![folder_id,archive_id],|r|r.get(0))?;
            if !destination { return Err(missing()); }
            let existing:Option<Vec<u8>> = tx.query_row("SELECT m.raw_mime FROM mail_archive_messages m JOIN mail_archives a ON a.id=m.archive_id WHERE a.account_id=(SELECT account_id FROM mail_archives WHERE id=?1) AND m.fingerprint=?2 LIMIT 1",params![archive_id,fingerprint],|r|r.get(0)).optional()?;
            if let Some(existing) = existing {
                if existing != raw { return Err(PebbleError::Validation("Archive content fingerprint collision".into())); }
                return Ok(false);
            }
            let id = pebble_core::new_id();
            tx.execute("INSERT INTO mail_archive_messages (id,archive_id,folder_id,fingerprint,raw_mime,metadata,subject,sender,date) VALUES (?1,?2,?3,?4,?5,?6,?7,?8,?9)",
                params![id,archive_id,folder_id,fingerprint,raw,metadata_json,metadata.subject,metadata.from_address,metadata.date])?;
            tx.execute("INSERT INTO mail_archive_fts (message_id,text) VALUES (?1,?2)",params![id,searchable])?;
            tx.commit()?;
            Ok(true)
        })
    }

    pub fn list_archive_messages(
        &self,
        archive_id: &str,
        folder_id: Option<&str>,
        query: &str,
        offset: u32,
    ) -> Result<Vec<ArchiveMessageSummary>> {
        if query.len() > 500 {
            return Err(PebbleError::Validation("Archive search too long".into()));
        }
        self.with_read(|conn| {
            // Literal phrases cannot inject FTS operators. Trigrams index Unicode
            // substrings; 1–2 character searches use a bounded-page LIKE fallback.
            let long_query = query.chars().count() >= 3;
            let phrase = format!("\"{}\"",query.replace('"',"\"\""));
            let literal = format!("%{}%", query.replace('\\',"\\\\").replace('%',"\\%").replace('_',"\\_"));
            let filter = if query.is_empty() { "" } else if long_query {
                " AND m.id IN (SELECT message_id FROM mail_archive_fts WHERE text MATCH ?3)"
            } else { " AND m.id IN (SELECT message_id FROM mail_archive_fts WHERE text LIKE ?3 ESCAPE '\\')" };
            let sql = format!("SELECT m.id,m.folder_id,m.subject,m.sender,m.date,m.is_read,m.is_starred FROM mail_archive_messages m WHERE m.archive_id=?1 AND (?2 IS NULL OR m.folder_id=?2){filter} AND ?3 IS NOT NULL ORDER BY m.date DESC,m.id LIMIT 100 OFFSET ?4");
            let mut stmt = conn.prepare(&sql)?;
            let search = if long_query { &phrase } else { &literal };
            let rows = stmt.query_map(params![archive_id,folder_id,search,offset], |r| Ok(ArchiveMessageSummary { id:r.get(0)?,folder_id:r.get(1)?,subject:r.get(2)?,sender:r.get(3)?,date:r.get(4)?,is_read:r.get(5)?,is_starred:r.get(6)? }))?;
            Ok(rows.collect::<rusqlite::Result<_>>()?)
        })
    }

    pub fn get_archive_message(&self, archive_id: &str, id: &str) -> Result<ArchiveMessageDetail> {
        self.with_read(|conn| {
            let result = conn.query_row("SELECT id,folder_id,fingerprint,is_read,is_starred,metadata FROM mail_archive_messages WHERE id=?1 AND archive_id=?2",params![id,archive_id],|r| Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,String>(2)?,r.get::<_,bool>(3)?,r.get::<_,bool>(4)?,r.get::<_,String>(5)?))).optional()?.ok_or_else(missing)?;
            Ok(ArchiveMessageDetail {id:result.0,folder_id:result.1,fingerprint:result.2,is_read:result.3,is_starred:result.4,metadata:serde_json::from_str(&result.5).map_err(|e|PebbleError::Storage(e.to_string()))?})
        })
    }

    pub fn get_archive_mime(&self, archive_id: &str, id: &str) -> Result<Vec<u8>> {
        self.with_read(|conn| {
            conn.query_row(
                "SELECT raw_mime FROM mail_archive_messages WHERE id=?1 AND archive_id=?2",
                params![id, archive_id],
                |r| r.get(0),
            )
            .optional()?
            .ok_or_else(missing)
        })
    }

    pub fn update_archive_message(
        &self,
        archive_id: &str,
        id: &str,
        folder_id: &str,
        is_read: bool,
        is_starred: bool,
    ) -> Result<()> {
        self.with_write(|conn| {
            let changed = conn.execute("UPDATE mail_archive_messages SET folder_id=?3,is_read=?4,is_starred=?5 WHERE id=?1 AND archive_id=?2 AND EXISTS(SELECT 1 FROM mail_archive_folders WHERE id=?3 AND archive_id=?2)",params![id,archive_id,folder_id,is_read,is_starred])?;
            if changed != 1 { return Err(missing()); }
            Ok(())
        })
    }

    pub fn delete_archive_message(&self, archive_id: &str, id: &str) -> Result<()> {
        self.with_write(|conn| {
            if conn.execute(
                "DELETE FROM mail_archive_messages WHERE id=?1 AND archive_id=?2",
                params![id, archive_id],
            )? != 1
            {
                return Err(missing());
            }
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    pub fn setup() -> (Store, MailArchive, ArchiveFolder) {
        let store = Store::open_in_memory().unwrap();
        store
            .with_write(|conn| {
                let exists: bool = conn.query_row(
                    "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='mail_archives')",
                    [],
                    |r| r.get(0),
                )?;
                if !exists {
                    conn.execute_batch(include_str!("mail_archive_schema.sql"))?;
                }
                Ok(())
            })
            .unwrap();
        let account = pebble_core::Account {
            id: "account".into(),
            email: "test@example.com".into(),
            display_name: "Test".into(),
            account_label: None,
            provider_display_name: None,
            color: None,
            provider: pebble_core::ProviderType::Imap,
            created_at: 0,
            updated_at: 0,
        };
        store.insert_account(&account).unwrap();
        let archive = store.create_mail_archive("account", "Archive").unwrap();
        let folder = store.list_archive_folders(&archive.id).unwrap().remove(0);
        (store, archive, folder)
    }

    fn meta() -> ArchiveMetadata {
        ArchiveMetadata {
            message_id_header: Some("<same>".into()),
            subject: "Unicode 迁移归档".into(),
            from_name: "Alice".into(),
            from_address: "sender@example.com".into(),
            to_list: vec![],
            cc_list: vec![],
            bcc_list: vec![],
            date: 10,
            body_text: "private searchable content 你好 literal % _".into(),
            has_html: false,
            attachments: vec![],
        }
    }

    #[test]
    fn import_index_flags_move_delete_are_local_and_survive_account_cleanup() {
        let (store, archive, folder) = setup();
        let hash = "a".repeat(64);
        assert!(store
            .insert_archive_message(
                &archive.id,
                &folder.id,
                &hash,
                b"Subject: test\n\nbody",
                &meta()
            )
            .unwrap());
        let rows = store
            .list_archive_messages(&archive.id, None, "searchable", 0)
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(
            store
                .list_archive_messages(&archive.id, None, "迁移归", 0)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store
                .list_archive_messages(&archive.id, None, "你好", 0)
                .unwrap()
                .len(),
            1
        );
        assert_eq!(
            store
                .list_archive_messages(&archive.id, None, "%", 0)
                .unwrap()
                .len(),
            1
        );
        assert!(store
            .list_archive_messages(&archive.id, None, "\" OR *", 0)
            .unwrap()
            .is_empty());
        let id = &rows[0].id;
        assert!(store.get_message(id).unwrap().is_none());
        let other = store.create_archive_folder(&archive.id, "Other").unwrap();
        store
            .update_archive_message(&archive.id, id, &other.id, true, true)
            .unwrap();
        assert!(store
            .update_archive_message(&archive.id, id, "INBOX", false, false)
            .is_err());
        assert!(
            store
                .get_archive_message(&archive.id, id)
                .unwrap()
                .is_starred
        );
        store.delete_account("account").unwrap();
        assert!(store.get_archive_message(&archive.id, id).is_ok());
        assert_eq!(store.list_mail_archives().unwrap().len(), 1);
        store.delete_archive_message(&archive.id, id).unwrap();
        assert!(store
            .list_archive_messages(&archive.id, None, "", 0)
            .unwrap()
            .is_empty());
        store
            .with_read(|conn| {
                let count: i64 =
                    conn.query_row("SELECT count(*) FROM mail_archive_fts", [], |r| r.get(0))?;
                assert_eq!(count, 0);
                Ok(())
            })
            .unwrap();
    }

    #[test]
    fn duplicates_use_full_content_per_account_and_cross_archive_moves_fail() {
        let (store, archive, folder) = setup();
        let raw = b"Subject: test\n\nbody";
        let hash = "a".repeat(64);
        assert!(store
            .insert_archive_message(&archive.id, &folder.id, &hash, raw, &meta())
            .unwrap());
        assert!(!store
            .insert_archive_message(&archive.id, &folder.id, &hash, raw, &meta())
            .unwrap());
        assert!(store
            .insert_archive_message(
                &archive.id,
                &folder.id,
                &"b".repeat(64),
                b"different content same Message-ID",
                &meta()
            )
            .unwrap());
        assert!(store
            .insert_archive_message(&archive.id, &folder.id, &hash, b"collision", &meta())
            .is_err());
        let other = store.create_mail_archive("account", "Other").unwrap();
        let other_folder = store.list_archive_folders(&other.id).unwrap().remove(0);
        assert!(!store
            .insert_archive_message(&other.id, &other_folder.id, &hash, raw, &meta())
            .unwrap());
        assert!(store.archive_has_content(&other.id, &hash).unwrap());
        let mut other_account = store.get_account("account").unwrap().unwrap();
        other_account.id = "other-account".into();
        other_account.email = "other@example.com".into();
        store.insert_account(&other_account).unwrap();
        let independent = store
            .create_mail_archive(&other_account.id, "Independent")
            .unwrap();
        let independent_folder = store
            .list_archive_folders(&independent.id)
            .unwrap()
            .remove(0);
        assert!(!store.archive_has_content(&independent.id, &hash).unwrap());
        assert!(store
            .insert_archive_message(&independent.id, &independent_folder.id, &hash, raw, &meta())
            .unwrap());
        let row = store
            .list_archive_messages(&archive.id, None, "", 0)
            .unwrap()
            .remove(0);
        assert!(store
            .update_archive_message(&archive.id, &row.id, &other_folder.id, false, false)
            .is_err());
        assert!(store.get_archive_mime(&other.id, &row.id).is_err());
    }

    #[test]
    fn failed_index_write_rolls_back_mime_and_schema_upgrade_is_transactional() {
        let (store, archive, folder) = setup();
        store
            .with_write(|conn| {
                conn.execute_batch("DROP TABLE mail_archive_fts")?;
                Ok(())
            })
            .unwrap();
        assert!(store
            .insert_archive_message(&archive.id, &folder.id, &"c".repeat(64), b"raw", &meta())
            .is_err());
        assert!(store
            .list_archive_messages(&archive.id, None, "", 0)
            .unwrap()
            .is_empty());
        let conn = rusqlite::Connection::open_in_memory().unwrap();
        (|| -> Result<()> {
            {
                let tx = conn.unchecked_transaction()?;
                tx.execute_batch(include_str!("mail_archive_schema.sql"))?;
            }
            let exists: bool = conn.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='mail_archives')",
                [],
                |r| r.get(0),
            )?;
            assert!(!exists);
            let tx = conn.unchecked_transaction()?;
            tx.execute_batch(include_str!("mail_archive_schema.sql"))?;
            tx.commit()?;
            Ok(())
        })()
        .unwrap();
        let exists: bool = conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='mail_archives')",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert!(exists);
    }
}
