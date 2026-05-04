use pebble_core::{PebbleError, Result};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::Store;

/// Maximum accepted size for a settings backup download.
/// Settings backups (accounts metadata, rules, kanban cards, translate config)
/// fit comfortably under this limit; anything larger is almost certainly
/// corrupted, malicious, or not a settings backup at all.
pub const MAX_BACKUP_SIZE_BYTES: usize = 16 * 1024 * 1024;

/// Highest backup schema version this build understands.
pub const BACKUP_SCHEMA_VERSION: u32 = 1;
pub const SETTINGS_BACKUP_FILENAME: &str = "pebble-settings-backup.json";

fn validate_backup_payload_shape(data: &[u8]) -> Result<()> {
    let data = data.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(data);
    let Some(first) = data.iter().copied().find(|b| !b.is_ascii_whitespace()) else {
        return Err(PebbleError::Validation(
            "Remote WebDAV backup file is empty. Check that the WebDAV URL points to the same directory used for backup and run a settings backup first.".to_string(),
        ));
    };

    if first == b'<' {
        return Err(PebbleError::Validation(format!(
            "WebDAV returned a page or directory listing instead of a Pebble settings backup. Check that {SETTINGS_BACKUP_FILENAME} exists and that the WebDAV URL points to the backup directory, not a browser share page."
        )));
    }

    if first != b'{' {
        return Err(PebbleError::Validation(format!(
            "Remote WebDAV file does not look like a Pebble settings backup. Check that {SETTINGS_BACKUP_FILENAME} was created by Pebble and that the WebDAV URL points to the backup directory."
        )));
    }

    Ok(())
}

fn provider_slug(provider: &pebble_core::ProviderType) -> &'static str {
    match provider {
        pebble_core::ProviderType::Imap => "imap",
        pebble_core::ProviderType::Gmail => "gmail",
        pebble_core::ProviderType::Outlook => "outlook",
    }
}

/// Lightweight summary of a backup for user confirmation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupPreview {
    pub version: u32,
    pub exported_at: i64,
    pub account_count: usize,
    pub rule_count: usize,
    pub kanban_card_count: usize,
    pub kanban_note_count: usize,
    pub has_translate_config: bool,
    pub size_bytes: usize,
}

/// Validate a downloaded backup payload and return a preview summary.
/// Enforces size limit, JSON validity, and schema version compatibility.
pub fn preview_backup(data: &[u8]) -> Result<BackupPreview> {
    if data.len() > MAX_BACKUP_SIZE_BYTES {
        return Err(PebbleError::Validation(format!(
            "Backup file is too large ({} bytes, max {})",
            data.len(),
            MAX_BACKUP_SIZE_BYTES
        )));
    }
    validate_backup_payload_shape(data)?;
    let backup: SettingsBackup = serde_json::from_slice(data).map_err(|e| {
        PebbleError::Validation(format!("Backup file is not a valid settings backup: {e}"))
    })?;
    if backup.version == 0 || backup.version > BACKUP_SCHEMA_VERSION {
        return Err(PebbleError::Validation(format!(
            "Unsupported backup version {} (this build supports up to {})",
            backup.version, BACKUP_SCHEMA_VERSION
        )));
    }
    Ok(BackupPreview {
        version: backup.version,
        exported_at: backup.exported_at,
        account_count: backup.accounts.len(),
        rule_count: backup.rules.len(),
        kanban_card_count: backup.kanban_cards.len(),
        kanban_note_count: backup.kanban_context_notes.len(),
        has_translate_config: backup
            .translate_config
            .as_ref()
            .map(|tc| !tc.config.is_empty())
            .unwrap_or(false),
        size_bytes: data.len(),
    })
}

/// Portable settings backup payload.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SettingsBackup {
    pub version: u32,
    pub exported_at: i64,
    pub accounts: Vec<AccountBackup>,
    pub rules: Vec<pebble_core::Rule>,
    pub kanban_cards: Vec<pebble_core::KanbanCard>,
    #[serde(default)]
    pub kanban_context_notes: HashMap<String, String>,
    pub translate_config: Option<pebble_core::TranslateConfig>,
}

/// Account data without passwords or auth secrets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountBackup {
    pub id: String,
    pub email: String,
    pub display_name: String,
    #[serde(default)]
    pub color: Option<String>,
    pub provider: pebble_core::ProviderType,
}

pub struct WebDavClient {
    url: String,
    username: String,
    password: String,
    client: reqwest::Client,
}

impl WebDavClient {
    pub fn new(url: String, username: String, password: String) -> Result<Self> {
        let trimmed = url.trim_end_matches('/').to_string();
        if !trimmed.starts_with("https://") {
            return Err(PebbleError::Validation(
                "WebDAV URL must use HTTPS to protect credentials".to_string(),
            ));
        }
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| PebbleError::Internal(format!("Failed to create HTTP client: {e}")))?;
        Ok(Self {
            url: trimmed,
            username,
            password,
            client,
        })
    }

    /// Validate credentials with a PROPFIND request to the WebDAV root.
    pub async fn test_connection(&self) -> Result<()> {
        let resp = self
            .client
            .request(reqwest::Method::from_bytes(b"PROPFIND").unwrap(), &self.url)
            .basic_auth(&self.username, Some(&self.password))
            .header("Depth", "0")
            .header("Content-Type", "application/xml")
            .send()
            .await
            .map_err(|e| PebbleError::Network(format!("WebDAV PROPFIND failed: {e}")))?;

        let status = resp.status().as_u16();
        if status == 207 || status == 200 {
            Ok(())
        } else if status == 401 || status == 403 {
            Err(PebbleError::Auth(format!(
                "WebDAV authentication failed (HTTP {status})"
            )))
        } else {
            Err(PebbleError::Network(format!(
                "WebDAV returned unexpected status {status}"
            )))
        }
    }

    /// Upload data to a path relative to the WebDAV root.
    pub async fn upload(&self, path: &str, data: &[u8]) -> Result<()> {
        let url = format!("{}/{}", self.url, path.trim_start_matches('/'));
        let resp = self
            .client
            .put(&url)
            .basic_auth(&self.username, Some(&self.password))
            .header("Content-Type", "application/json")
            .body(data.to_vec())
            .send()
            .await
            .map_err(|e| PebbleError::Network(format!("WebDAV PUT failed: {e}")))?;

        let status = resp.status().as_u16();
        if (200..300).contains(&status) {
            Ok(())
        } else {
            let body = resp.text().await.unwrap_or_default();
            Err(PebbleError::Network(format!(
                "WebDAV PUT returned {status}: {body}"
            )))
        }
    }

    /// Download data from a path relative to the WebDAV root.
    /// Rejects responses larger than `MAX_BACKUP_SIZE_BYTES` without buffering
    /// the full body into memory — important for defending against malicious
    /// or corrupt files on the remote server.
    pub async fn download(&self, path: &str) -> Result<Vec<u8>> {
        let url = format!("{}/{}", self.url, path.trim_start_matches('/'));
        let mut resp = self
            .client
            .get(&url)
            .basic_auth(&self.username, Some(&self.password))
            .send()
            .await
            .map_err(|e| PebbleError::Network(format!("WebDAV GET failed: {e}")))?;

        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(PebbleError::Network(format!(
                "WebDAV GET returned {status}"
            )));
        }

        // Reject immediately if server advertises a size over the limit.
        if let Some(len) = resp.content_length() {
            if len as usize > MAX_BACKUP_SIZE_BYTES {
                return Err(PebbleError::Validation(format!(
                    "Backup file is too large ({} bytes, max {})",
                    len, MAX_BACKUP_SIZE_BYTES
                )));
            }
        }

        // Stream the body chunk-by-chunk with a hard cap so a lying or missing
        // Content-Length cannot blow up memory.
        let mut buf: Vec<u8> = Vec::with_capacity(8 * 1024);
        while let Some(chunk) = resp
            .chunk()
            .await
            .map_err(|e| PebbleError::Network(format!("Failed to read response body: {e}")))?
        {
            if buf.len() + chunk.len() > MAX_BACKUP_SIZE_BYTES {
                return Err(PebbleError::Validation(format!(
                    "Backup file exceeds maximum size ({} bytes)",
                    MAX_BACKUP_SIZE_BYTES
                )));
            }
            buf.extend_from_slice(&chunk);
        }
        Ok(buf)
    }
}

impl Store {
    /// Export settings (accounts without passwords, rules, kanban cards, translate config) as JSON bytes.
    pub fn export_settings(&self) -> Result<Vec<u8>> {
        let accounts = self.list_accounts()?;
        let account_backups: Vec<AccountBackup> = accounts
            .into_iter()
            .map(|a| AccountBackup {
                id: a.id,
                email: a.email,
                display_name: a.display_name,
                color: a.color,
                provider: a.provider,
            })
            .collect();

        let rules = self.list_rules()?;
        let kanban_cards = self.list_kanban_cards(None)?;
        // Redact translate config — never export API keys or encrypted secrets
        let translate_config = self.get_translate_config()?.map(|mut tc| {
            tc.config = String::new();
            tc
        });

        let backup = SettingsBackup {
            version: 1,
            exported_at: pebble_core::now_timestamp(),
            accounts: account_backups,
            rules,
            kanban_cards,
            kanban_context_notes: HashMap::new(),
            translate_config,
        };

        let json = serde_json::to_vec_pretty(&backup)
            .map_err(|e| PebbleError::Internal(format!("Failed to serialize settings: {e}")))?;
        Ok(json)
    }

    /// Import settings from JSON bytes, upserting into the store.
    ///
    /// The entire import runs inside a single transaction so that a crash
    /// mid-import cannot leave the store in a partially-deleted state.
    /// Validates size and schema version before touching the database.
    pub fn import_settings(&self, data: &[u8]) -> Result<()> {
        if data.len() > MAX_BACKUP_SIZE_BYTES {
            return Err(PebbleError::Validation(format!(
                "Backup file is too large ({} bytes, max {})",
                data.len(),
                MAX_BACKUP_SIZE_BYTES
            )));
        }
        validate_backup_payload_shape(data)?;
        let backup: SettingsBackup = serde_json::from_slice(data)
            .map_err(|e| PebbleError::Validation(format!("Failed to deserialize settings: {e}")))?;
        if backup.version == 0 || backup.version > BACKUP_SCHEMA_VERSION {
            return Err(PebbleError::Validation(format!(
                "Unsupported backup version {} (this build supports up to {})",
                backup.version, BACKUP_SCHEMA_VERSION
            )));
        }

        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()
                .map_err(|e| PebbleError::Storage(format!("Failed to begin transaction: {e}")))?;

            // Merge account metadata: insert restored accounts, update existing
            // account display fields without touching auth_data.
            for ab in &backup.accounts {
                let exists: bool = tx
                    .query_row(
                        "SELECT EXISTS(SELECT 1 FROM accounts WHERE id = ?1)",
                        rusqlite::params![&ab.id],
                        |row| row.get(0),
                    )
                    .map_err(|e| PebbleError::Storage(e.to_string()))?;
                let existing_color: Option<String> = if exists {
                    tx.query_row(
                        "SELECT color FROM accounts WHERE id = ?1",
                        rusqlite::params![&ab.id],
                        |row| row.get(0),
                    )
                    .map_err(|e| PebbleError::Storage(e.to_string()))?
                } else {
                    None
                };
                let restored_color = ab.color.as_deref().or(existing_color.as_deref());
                if !exists {
                    let now = pebble_core::now_timestamp();
                    let mut sync_state = crate::accounts::SyncState {
                        provider: Some(provider_slug(&ab.provider).to_string()),
                        ..Default::default()
                    };
                    sync_state
                        .extra
                        .insert("needs_reauth".into(), serde_json::Value::Bool(true));
                    sync_state.extra.insert(
                        "restore_is_partial".into(),
                        serde_json::Value::Bool(true),
                    );
                    sync_state.extra.insert(
                        "restored_from_backup_at".into(),
                        serde_json::Value::Number(now.into()),
                    );
                    let sync_state_json = sync_state.to_json()?;
                    tx.execute(
                        "INSERT INTO accounts (id, email, display_name, color, provider, sync_state, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                        rusqlite::params![&ab.id, &ab.email, &ab.display_name, restored_color, provider_slug(&ab.provider), sync_state_json, now, now],
                    ).map_err(|e| PebbleError::Storage(e.to_string()))?;
                } else {
                    tx.execute(
                        "UPDATE accounts SET email = ?1, display_name = ?2, color = ?3, updated_at = ?4 WHERE id = ?5",
                        rusqlite::params![&ab.email, &ab.display_name, restored_color, pebble_core::now_timestamp(), &ab.id],
                    )
                    .map_err(|e| PebbleError::Storage(e.to_string()))?;
                }
            }

            // Replace rules atomically — delete existing, then insert from backup
            tx.execute("DELETE FROM rules", [])
                .map_err(|e| PebbleError::Storage(e.to_string()))?;
            for rule in &backup.rules {
                tx.execute(
                    "INSERT INTO rules (id, name, priority, conditions, actions, is_enabled, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![&rule.id, &rule.name, rule.priority, &rule.conditions, &rule.actions, rule.is_enabled, rule.created_at, rule.updated_at],
                ).map_err(|e| PebbleError::Storage(e.to_string()))?;
            }

            // Replace kanban cards atomically so restored boards match the backup.
            tx.execute("DELETE FROM kanban_cards", [])
                .map_err(|e| PebbleError::Storage(e.to_string()))?;
            for card in &backup.kanban_cards {
                Self::upsert_kanban_card_with_conn(&tx, card)?;
            }

            // Upsert translate config — skip if config field is empty (redacted export)
            if let Some(tc) = &backup.translate_config {
                if !tc.config.is_empty() {
                    Self::save_translate_config_with_conn(&tx, tc)?;
                }
            }

            tx.commit()
                .map_err(|e| PebbleError::Storage(format!("Failed to commit: {e}")))?;
            Ok(())
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_core::*;

    #[test]
    fn test_export_import_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let now = now_timestamp();

        // Create test account
        let account = Account {
            id: new_id(),
            email: "test@example.com".to_string(),
            display_name: "Test User".to_string(),
            color: Some("#22c55e".to_string()),
            provider: ProviderType::Imap,
            created_at: now,
            updated_at: now,
        };
        store.insert_account(&account).unwrap();

        // Create test rule
        let rule = Rule {
            id: new_id(),
            name: "Auto-archive".to_string(),
            priority: 10,
            conditions: r#"{"from":"noreply@example.com"}"#.to_string(),
            actions: r#"["archive"]"#.to_string(),
            is_enabled: true,
            created_at: now,
            updated_at: now,
        };
        store.insert_rule(&rule).unwrap();

        // Create translate config
        let tc = TranslateConfig {
            id: "active".to_string(),
            provider_type: "deeplx".to_string(),
            config: r#"{"endpoint":"http://localhost:1188/translate"}"#.to_string(),
            is_enabled: true,
            created_at: now,
            updated_at: now,
        };
        store.save_translate_config(&tc).unwrap();

        // Export
        let data = store.export_settings().unwrap();
        let backup: SettingsBackup = serde_json::from_slice(&data).unwrap();
        assert_eq!(backup.version, 1);
        assert_eq!(backup.accounts.len(), 1);
        assert_eq!(backup.accounts[0].email, "test@example.com");
        assert_eq!(backup.accounts[0].color.as_deref(), Some("#22c55e"));
        assert_eq!(backup.rules.len(), 1);
        assert_eq!(backup.rules[0].name, "Auto-archive");
        assert!(backup.translate_config.is_some());
        // Config field should be redacted (empty) in export
        assert_eq!(backup.translate_config.as_ref().unwrap().config, "");

        // Import into a fresh store
        let store2 = Store::open_in_memory().unwrap();
        store2.import_settings(&data).unwrap();

        // Verify imported data
        let accounts = store2.list_accounts().unwrap();
        assert_eq!(accounts.len(), 1);
        assert_eq!(accounts[0].email, "test@example.com");
        assert_eq!(accounts[0].color.as_deref(), Some("#22c55e"));

        let rules = store2.list_rules().unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "Auto-archive");

        // Translate config should NOT be imported when config is redacted
        let tc_loaded = store2.get_translate_config().unwrap();
        assert!(tc_loaded.is_none());
    }

    #[test]
    fn preview_backup_rejects_empty_webdav_response_with_actionable_message() {
        let err = preview_backup(b"").unwrap_err().to_string();

        assert!(err.contains("Remote WebDAV backup file is empty"));
        assert!(err.contains("run a settings backup first"));
    }

    #[test]
    fn preview_backup_rejects_webdav_page_response_with_actionable_message() {
        let err = preview_backup(
            br#"<?xml version="1.0"?><d:multistatus xmlns:d="DAV:"></d:multistatus>"#,
        )
        .unwrap_err()
        .to_string();

        assert!(err.contains("WebDAV returned a page or directory listing"));
        assert!(err.contains("pebble-settings-backup.json"));
    }

    #[test]
    fn test_import_does_not_duplicate_existing_accounts() {
        let store = Store::open_in_memory().unwrap();
        let now = now_timestamp();

        let account = Account {
            id: "fixed-id".to_string(),
            email: "test@example.com".to_string(),
            display_name: "Test".to_string(),
            color: None,
            provider: ProviderType::Imap,
            created_at: now,
            updated_at: now,
        };
        store.insert_account(&account).unwrap();

        // Export, then import into the same store
        let data = store.export_settings().unwrap();
        store.import_settings(&data).unwrap();

        let accounts = store.list_accounts().unwrap();
        assert_eq!(accounts.len(), 1);
    }

    #[test]
    fn test_import_legacy_backup_preserves_existing_account_color() {
        let store = Store::open_in_memory().unwrap();
        let now = now_timestamp();

        let account = Account {
            id: "fixed-id".to_string(),
            email: "test@example.com".to_string(),
            display_name: "Test".to_string(),
            color: Some("#22c55e".to_string()),
            provider: ProviderType::Imap,
            created_at: now,
            updated_at: now,
        };
        store.insert_account(&account).unwrap();

        let legacy_backup = serde_json::json!({
            "version": 1,
            "exported_at": now,
            "accounts": [{
                "id": "fixed-id",
                "email": "renamed@example.com",
                "display_name": "Renamed",
                "provider": "imap"
            }],
            "rules": [],
            "kanban_cards": [],
            "kanban_context_notes": {},
            "translate_config": null
        });

        store
            .import_settings(&serde_json::to_vec(&legacy_backup).unwrap())
            .unwrap();

        let updated = store.get_account("fixed-id").unwrap().unwrap();
        assert_eq!(updated.email, "renamed@example.com");
        assert_eq!(updated.color.as_deref(), Some("#22c55e"));
    }

    #[test]
    fn test_import_replaces_rules() {
        let store = Store::open_in_memory().unwrap();
        let now = now_timestamp();

        let rule1 = Rule {
            id: new_id(),
            name: "Old Rule".to_string(),
            priority: 1,
            conditions: "{}".to_string(),
            actions: "[]".to_string(),
            is_enabled: true,
            created_at: now,
            updated_at: now,
        };
        store.insert_rule(&rule1).unwrap();

        // Build a backup with a different rule
        let backup = SettingsBackup {
            version: 1,
            exported_at: now,
            accounts: vec![],
            rules: vec![Rule {
                id: new_id(),
                name: "New Rule".to_string(),
                priority: 5,
                conditions: "{}".to_string(),
                actions: "[]".to_string(),
                is_enabled: false,
                created_at: now,
                updated_at: now,
            }],
            kanban_cards: vec![],
            kanban_context_notes: HashMap::new(),
            translate_config: None,
        };
        let data = serde_json::to_vec(&backup).unwrap();
        store.import_settings(&data).unwrap();

        let rules = store.list_rules().unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "New Rule");
    }

    #[test]
    fn test_import_replaces_kanban_cards() {
        let store = Store::open_in_memory().unwrap();
        let now = now_timestamp();

        let account = Account {
            id: new_id(),
            email: "test@example.com".to_string(),
            display_name: "Test User".to_string(),
            color: None,
            provider: ProviderType::Imap,
            created_at: now,
            updated_at: now,
        };
        store.insert_account(&account).unwrap();
        let folder = Folder {
            id: new_id(),
            account_id: account.id.clone(),
            remote_id: "INBOX".to_string(),
            name: "Inbox".to_string(),
            folder_type: FolderType::Folder,
            role: Some(FolderRole::Inbox),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 0,
        };
        store.insert_folder(&folder).unwrap();
        let make_msg = |subject: &str| Message {
            id: new_id(),
            account_id: account.id.clone(),
            remote_id: new_id(),
            message_id_header: None,
            in_reply_to: None,
            references_header: None,
            thread_id: None,
            subject: subject.to_string(),
            snippet: String::new(),
            from_address: "sender@example.com".to_string(),
            from_name: String::new(),
            to_list: vec![],
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
        };
        let old_msg = make_msg("old");
        let new_msg = make_msg("new");
        store
            .insert_message(&old_msg, std::slice::from_ref(&folder.id))
            .unwrap();
        store.insert_message(&new_msg, &[folder.id]).unwrap();
        store
            .upsert_kanban_card(&KanbanCard {
                message_id: old_msg.id,
                column: KanbanColumn::Todo,
                position: 0,
                created_at: now,
                updated_at: now,
            })
            .unwrap();

        let backup = SettingsBackup {
            version: 1,
            exported_at: now,
            accounts: vec![],
            rules: vec![],
            kanban_cards: vec![KanbanCard {
                message_id: new_msg.id.clone(),
                column: KanbanColumn::Done,
                position: 0,
                created_at: now,
                updated_at: now,
            }],
            kanban_context_notes: HashMap::new(),
            translate_config: None,
        };

        store
            .import_settings(&serde_json::to_vec(&backup).unwrap())
            .unwrap();

        let cards = store.list_kanban_cards(None).unwrap();
        assert_eq!(cards.len(), 1);
        assert_eq!(cards[0].message_id, new_msg.id);
        assert_eq!(cards[0].column, KanbanColumn::Done);
    }

    #[test]
    fn test_import_marks_restored_accounts_as_needing_reauth() {
        let store = Store::open_in_memory().unwrap();
        let now = now_timestamp();

        let backup = SettingsBackup {
            version: 1,
            exported_at: now,
            accounts: vec![AccountBackup {
                id: "gmail-account".to_string(),
                email: "gmail@example.com".to_string(),
                display_name: "Gmail User".to_string(),
                color: Some("#3b82f6".to_string()),
                provider: ProviderType::Gmail,
            }],
            rules: vec![],
            kanban_cards: vec![],
            kanban_context_notes: HashMap::new(),
            translate_config: None,
        };

        let data = serde_json::to_vec(&backup).unwrap();
        store.import_settings(&data).unwrap();

        let sync_state = store
            .get_sync_state("gmail-account")
            .unwrap()
            .expect("expected sync_state metadata");
        assert_eq!(sync_state.provider.as_deref(), Some("gmail"));
        let accounts = store.list_accounts().unwrap();
        assert_eq!(accounts[0].color.as_deref(), Some("#3b82f6"));
        assert_eq!(
            sync_state.extra.get("needs_reauth"),
            Some(&serde_json::Value::Bool(true))
        );
        assert_eq!(
            sync_state.extra.get("restore_is_partial"),
            Some(&serde_json::Value::Bool(true))
        );
    }
}
