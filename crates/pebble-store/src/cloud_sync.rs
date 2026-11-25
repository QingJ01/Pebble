use pebble_core::{PebbleError, Result};
use rusqlite::OptionalExtension;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::{contacts::list_all_contacts_for_backup_with_conn, Store};

/// Maximum accepted size for a settings backup download.
/// Settings backups (accounts metadata, rules, kanban cards, translate config)
/// fit comfortably under this limit; anything larger is almost certainly
/// corrupted, malicious, or not a settings backup at all.
pub const MAX_BACKUP_SIZE_BYTES: usize = 16 * 1024 * 1024;

/// Highest backup schema version this build understands.
pub const BACKUP_SCHEMA_VERSION: u32 = 3;
pub const SETTINGS_BACKUP_FILENAME: &str = "pebble-settings-backup.json";

fn validate_backup_schema(backup: &SettingsBackup) -> Result<()> {
    if backup.version == 0 || backup.version > BACKUP_SCHEMA_VERSION {
        return Err(PebbleError::Validation(format!(
            "Unsupported backup version {} (this build supports up to {})",
            backup.version, BACKUP_SCHEMA_VERSION
        )));
    }
    if backup.version >= 2 && backup.contacts.is_none() {
        return Err(PebbleError::Validation(
            "Backup version 2 is missing the required contacts field".to_string(),
        ));
    }
    Ok(())
}

pub fn serialize_backup(backup: &SettingsBackup) -> Result<Vec<u8>> {
    let json = serde_json::to_vec_pretty(backup)
        .map_err(|e| PebbleError::Internal(format!("Failed to serialize settings: {e}")))?;
    if json.len() > MAX_BACKUP_SIZE_BYTES {
        return Err(PebbleError::Validation(format!(
            "Backup file is too large ({} bytes, max {})",
            json.len(),
            MAX_BACKUP_SIZE_BYTES
        )));
    }
    Ok(json)
}

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
        pebble_core::ProviderType::Pop3 => "pop3",
        pebble_core::ProviderType::Gmail => "gmail",
        pebble_core::ProviderType::Outlook => "outlook",
    }
}

/// Lightweight summary of a backup for user confirmation.
#[derive(Debug, Clone, Serialize, Deserialize, ts_rs::TS)]
pub struct BackupPreview {
    pub version: u32,
    pub exported_at: i64,
    pub account_count: usize,
    pub rule_count: usize,
    pub kanban_card_count: usize,
    pub kanban_note_count: usize,
    pub contact_count: usize,
    pub has_translate_config: bool,
    pub has_encrypted_secrets: bool,
    pub secret_account_count: usize,
    pub has_translate_secret: bool,
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
    validate_backup_schema(&backup)?;
    Ok(BackupPreview {
        version: backup.version,
        exported_at: backup.exported_at,
        account_count: backup.accounts.len(),
        rule_count: backup.rules.len(),
        kanban_card_count: backup.kanban_cards.len(),
        kanban_note_count: backup.kanban_context_notes.len(),
        contact_count: backup.contacts.as_ref().map(Vec::len).unwrap_or(0),
        has_translate_config: backup
            .translate_config
            .as_ref()
            .map(|tc| !tc.config.is_empty())
            .unwrap_or(false),
        has_encrypted_secrets: backup.encrypted_secrets.is_some(),
        secret_account_count: backup
            .secret_summary
            .as_ref()
            .map(|summary| summary.account_auth_count)
            .unwrap_or(0),
        has_translate_secret: backup
            .secret_summary
            .as_ref()
            .map(|summary| summary.has_translate_config)
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
    #[serde(default)]
    pub contacts: Option<Vec<pebble_core::Contact>>,
    pub translate_config: Option<pebble_core::TranslateConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secret_summary: Option<BackupSecretSummary>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub encrypted_secrets: Option<serde_json::Value>,
}

/// Account data without passwords or auth secrets.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountBackup {
    #[serde(default)]
    pub account_label: Option<String>,
    pub id: String,
    pub email: String,
    pub display_name: String,
    #[serde(default)]
    pub color: Option<String>,
    pub provider: pebble_core::ProviderType,
}

#[derive(Debug, Clone)]
pub struct RestoredAuthData {
    pub account_id: String,
    pub provider: String,
    pub encrypted: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct RestoredSecureUserData {
    pub key: String,
    pub encrypted: Option<Vec<u8>>,
}

#[derive(Debug, Clone, Default)]
pub struct RestoredPrivateData {
    pub auth_data: Vec<RestoredAuthData>,
    pub secure_user_data: Vec<RestoredSecureUserData>,
    pub translate_config: Option<pebble_core::TranslateConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupSecretSummary {
    pub account_auth_count: usize,
    pub has_translate_config: bool,
}

impl Store {
    /// Export settings (accounts without passwords, rules, kanban cards, contacts, translate config) as JSON bytes.
    pub fn export_settings(&self) -> Result<Vec<u8>> {
        let accounts = self.list_accounts()?;
        let account_backups: Vec<AccountBackup> = accounts
            .into_iter()
            .map(|a| AccountBackup {
                account_label: a.account_label,
                id: a.id,
                email: a.email,
                display_name: a.display_name,
                color: a.color,
                provider: a.provider,
            })
            .collect();

        let rules = self.list_rules()?;
        let kanban_cards = self.list_kanban_cards(None)?;
        let contacts = self.with_read(|conn| {
            let tx = conn.unchecked_transaction()?;
            let contacts = list_all_contacts_for_backup_with_conn(&tx)?;
            tx.commit()?;
            Ok(contacts)
        })?;
        // Redact translate config — never export API keys or encrypted secrets
        let translate_config = self.get_translate_config()?.map(|mut tc| {
            tc.config = String::new();
            tc
        });

        let backup = SettingsBackup {
            version: BACKUP_SCHEMA_VERSION,
            exported_at: pebble_core::now_timestamp(),
            accounts: account_backups,
            rules,
            kanban_cards,
            kanban_context_notes: HashMap::new(),
            contacts: Some(contacts),
            translate_config,
            secret_summary: None,
            encrypted_secrets: None,
        };

        serialize_backup(&backup)
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
        validate_backup_schema(&backup)?;

        self.with_write(|conn| {
            let tx = conn.unchecked_transaction()
                .map_err(|e| PebbleError::Storage(format!("Failed to begin transaction: {e}")))?;

            // Merge account metadata: insert restored accounts, update existing
            // account display fields without touching auth_data.
            for ab in &backup.accounts {
                let restored_label = crate::accounts::normalize_account_label(ab.account_label.as_deref())?;
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
                        "INSERT INTO accounts (id, email, display_name, color, provider, sync_state, created_at, updated_at, account_label) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                        rusqlite::params![&ab.id, &ab.email, &ab.display_name, restored_color, provider_slug(&ab.provider), sync_state_json, now, now, &restored_label],
                    ).map_err(|e| PebbleError::Storage(e.to_string()))?;
                } else {
                    tx.execute(
                        "UPDATE accounts SET email = CASE WHEN provider IN ('gmail', 'outlook') THEN email ELSE ?1 END, display_name = ?2, color = ?3, updated_at = ?4, account_label = ?6 WHERE id = ?5",
                        rusqlite::params![&ab.email, &ab.display_name, restored_color, pebble_core::now_timestamp(), &ab.id, &restored_label],
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

            // Contacts were added in schema v2. A v1 restore must preserve
            // local contacts because the older file could not contain them.
            if backup.version >= 2 {
                if let Some(contacts) = &backup.contacts {
                    crate::contacts::replace_contacts_with_conn(&tx, contacts)?;
                }
            }

            // The translate config in the plaintext section of a backup is
            // unauthenticated, so it is never restored from there. Only
            // passphrase-encrypted secrets can restore it (see
            // `import_settings_with_private_data`).

            tx.commit()
                .map_err(|e| PebbleError::Storage(format!("Failed to commit: {e}")))?;
            Ok(())
        })
    }

    /// Import settings and already-encrypted private data in one transaction.
    pub fn import_settings_with_private_data(
        &self,
        data: &[u8],
        private_data: RestoredPrivateData,
    ) -> Result<()> {
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
        validate_backup_schema(&backup)?;

        self.with_write(|conn| {
            let tx = conn
                .unchecked_transaction()
                .map_err(|e| PebbleError::Storage(format!("Failed to begin transaction: {e}")))?;

            for ab in &backup.accounts {
                let restored_label = crate::accounts::normalize_account_label(ab.account_label.as_deref())?;
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
                        "INSERT INTO accounts (id, email, display_name, color, provider, sync_state, created_at, updated_at, account_label) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
                        rusqlite::params![&ab.id, &ab.email, &ab.display_name, restored_color, provider_slug(&ab.provider), sync_state_json, now, now, &restored_label],
                    ).map_err(|e| PebbleError::Storage(e.to_string()))?;
                } else {
                    tx.execute(
                        "UPDATE accounts SET email = CASE WHEN provider IN ('gmail', 'outlook') THEN email ELSE ?1 END, display_name = ?2, color = ?3, updated_at = ?4, account_label = ?6 WHERE id = ?5",
                        rusqlite::params![&ab.email, &ab.display_name, restored_color, pebble_core::now_timestamp(), &ab.id, &restored_label],
                    )
                    .map_err(|e| PebbleError::Storage(e.to_string()))?;
                }
            }

            tx.execute("DELETE FROM rules", [])
                .map_err(|e| PebbleError::Storage(e.to_string()))?;
            for rule in &backup.rules {
                tx.execute(
                    "INSERT INTO rules (id, name, priority, conditions, actions, is_enabled, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
                    rusqlite::params![&rule.id, &rule.name, rule.priority, &rule.conditions, &rule.actions, rule.is_enabled, rule.created_at, rule.updated_at],
                ).map_err(|e| PebbleError::Storage(e.to_string()))?;
            }

            tx.execute("DELETE FROM kanban_cards", [])
                .map_err(|e| PebbleError::Storage(e.to_string()))?;
            for card in &backup.kanban_cards {
                Self::upsert_kanban_card_with_conn(&tx, card)?;
            }

            if backup.version >= 2 {
                if let Some(contacts) = &backup.contacts {
                    crate::contacts::replace_contacts_with_conn(&tx, contacts)?;
                }
            }

            // Never restore the translate config from the unauthenticated
            // plaintext section; only `private_data.translate_config`, which the
            // caller builds from passphrase-encrypted secrets, is applied below.

            for item in &private_data.secure_user_data {
                if item.key.trim().is_empty() {
                    return Err(PebbleError::Validation(
                        "Restored secure user data key cannot be empty".to_string(),
                    ));
                }
                if let Some(encrypted) = &item.encrypted {
                    tx.execute(
                        "INSERT INTO secure_user_data (key, value, updated_at)
                         VALUES (?1, ?2, ?3)
                         ON CONFLICT(key) DO UPDATE SET
                             value = excluded.value,
                             updated_at = excluded.updated_at",
                        rusqlite::params![&item.key, encrypted, pebble_core::now_timestamp()],
                    )
                    .map_err(|e| PebbleError::Storage(e.to_string()))?;
                } else {
                    tx.execute(
                        "DELETE FROM secure_user_data WHERE key = ?1",
                        rusqlite::params![&item.key],
                    )
                    .map_err(|e| PebbleError::Storage(e.to_string()))?;
                }
            }

            for auth in &private_data.auth_data {
                let (provider, has_auth, bound_subject, has_mailbox_data): (String, bool, Option<String>, bool) = tx.query_row(
                    "SELECT provider, auth_data IS NOT NULL, oauth_subject,
                        EXISTS(SELECT 1 FROM messages WHERE account_id = accounts.id)
                        OR EXISTS(SELECT 1 FROM folders WHERE account_id = accounts.id)
                        OR COALESCE(json_extract(sync_state, '$.last_sync_cursor'), '') != ''
                     FROM accounts WHERE id = ?1",
                    [&auth.account_id], |row| Ok((row.get(0)?, row.get(1)?, row.get(2)?, row.get(3)?))).optional()?.ok_or_else(|| PebbleError::Validation(format!("Backup secret references missing account: {}", auth.account_id)))?;
                if provider != auth.provider {
                    return Err(PebbleError::Validation("Backup authorization does not match the account provider".into()));
                }
                // Never replace a connected/bound OAuth mailbox with credentials from a backup.
                // New, unbound accounts can still restore their authorization normally.
                if matches!(provider.as_str(), "gmail" | "outlook") && (has_auth || bound_subject.is_some() || has_mailbox_data) {
                    continue;
                }
                let now = pebble_core::now_timestamp();
                let rows_affected = tx
                    .execute(
                        "UPDATE accounts SET auth_data = ?1, updated_at = ?2 WHERE id = ?3",
                        rusqlite::params![&auth.encrypted, now, &auth.account_id],
                    )
                    .map_err(|e| PebbleError::Storage(e.to_string()))?;
                if rows_affected == 0 {
                    return Err(PebbleError::Validation(format!(
                        "Backup secret references missing account: {}",
                        auth.account_id
                    )));
                }

                let current: Option<String> = tx
                    .query_row(
                        "SELECT sync_state FROM accounts WHERE id = ?1",
                        rusqlite::params![&auth.account_id],
                        |row| row.get::<_, Option<String>>(0),
                    )
                    .optional()?
                    .flatten();
                let mut sync_state =
                    crate::accounts::SyncState::from_json_opt(current.as_deref())?;
                sync_state.provider = Some(auth.provider.clone());
                sync_state.extra.remove("needs_reauth");
                sync_state.extra.remove("restore_is_partial");
                let sync_state_json = sync_state.to_json()?;
                tx.execute(
                    "UPDATE accounts SET sync_state = ?1, updated_at = ?2 WHERE id = ?3",
                    rusqlite::params![
                        sync_state_json,
                        pebble_core::now_timestamp(),
                        &auth.account_id
                    ],
                )
                .map_err(|e| PebbleError::Storage(e.to_string()))?;
            }

            if let Some(config) = &private_data.translate_config {
                Self::save_translate_config_with_conn(&tx, config)?;
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
            account_label: None,
            provider_display_name: None,
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

        store
            .save_contact(&ContactInput {
                id: None,
                display_name: "Alice Example".to_string(),
                notes: "Met at RustConf".to_string(),
                is_favorite: true,
                emails: vec![
                    ContactEmailInput {
                        id: None,
                        address: "alice@example.com".to_string(),
                        label: ContactEmailLabel::Work,
                        is_primary: true,
                    },
                    ContactEmailInput {
                        id: None,
                        address: "alice@home.example".to_string(),
                        label: ContactEmailLabel::Personal,
                        is_primary: false,
                    },
                ],
            })
            .unwrap();

        // Export
        let data = store.export_settings().unwrap();
        let backup: SettingsBackup = serde_json::from_slice(&data).unwrap();
        assert_eq!(backup.version, BACKUP_SCHEMA_VERSION);
        assert_eq!(backup.accounts.len(), 1);
        assert_eq!(backup.accounts[0].email, "test@example.com");
        assert_eq!(backup.accounts[0].color.as_deref(), Some("#22c55e"));
        assert_eq!(backup.rules.len(), 1);
        assert_eq!(backup.rules[0].name, "Auto-archive");
        let backed_up_contacts = backup.contacts.as_ref().unwrap();
        assert_eq!(backed_up_contacts.len(), 1);
        assert_eq!(backed_up_contacts[0].emails.len(), 2);
        assert!(backed_up_contacts[0].is_favorite);
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

        let contacts = store2.list_contacts(None, false, 200, 0).unwrap();
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].display_name, "Alice Example");
        assert_eq!(contacts[0].notes, "Met at RustConf");
        assert_eq!(contacts[0].emails.len(), 2);
        assert!(contacts[0].is_favorite);
        assert_eq!(
            contacts[0]
                .emails
                .iter()
                .find(|email| email.is_primary)
                .map(|email| email.address.as_str()),
            Some("alice@example.com")
        );

        // Translate config should NOT be imported when config is redacted
        let tc_loaded = store2.get_translate_config().unwrap();
        assert!(tc_loaded.is_none());
    }

    #[test]
    fn tampered_plaintext_translate_config_is_never_restored() {
        let backup = serde_json::json!({
            "version": 1,
            "exported_at": 1,
            "accounts": [],
            "rules": [],
            "kanban_cards": [],
            "translate_config": {
                "id": "active",
                "provider_type": "generic_api",
                "config": "{\"endpoint\":\"https://attacker.example/collect\"}",
                "is_enabled": true,
                "created_at": 1,
                "updated_at": 1
            }
        })
        .to_string();

        let store = Store::open_in_memory().unwrap();
        store.import_settings(backup.as_bytes()).unwrap();
        assert!(store.get_translate_config().unwrap().is_none());

        store
            .import_settings_with_private_data(backup.as_bytes(), RestoredPrivateData::default())
            .unwrap();
        assert!(store.get_translate_config().unwrap().is_none());
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
    fn preview_backup_reports_encrypted_secret_summary() {
        let backup = serde_json::json!({
            "version": 1,
            "exported_at": now_timestamp(),
            "accounts": [],
            "rules": [],
            "kanban_cards": [],
            "kanban_context_notes": {},
            "translate_config": null,
            "secret_summary": {
                "account_auth_count": 2,
                "has_translate_config": true
            },
            "encrypted_secrets": {
                "algorithm": "aes-256-gcm",
                "kdf": "pbkdf2-hmac-sha256",
                "iterations": 210000,
                "salt_hex": "00112233445566778899aabbccddeeff",
                "ciphertext_hex": "00"
            }
        });

        let preview = preview_backup(&serde_json::to_vec(&backup).unwrap()).unwrap();

        assert!(preview.has_encrypted_secrets);
        assert_eq!(preview.secret_account_count, 2);
        assert!(preview.has_translate_secret);
    }

    #[test]
    fn version_one_backup_has_no_contacts_and_preserves_local_contacts() {
        let store = Store::open_in_memory().unwrap();
        store
            .save_contact(&ContactInput {
                id: None,
                display_name: "Local contact".to_string(),
                notes: String::new(),
                is_favorite: false,
                emails: vec![ContactEmailInput {
                    id: None,
                    address: "local@example.com".to_string(),
                    label: ContactEmailLabel::Other,
                    is_primary: true,
                }],
            })
            .unwrap();
        let backup = serde_json::json!({
            "version": 1,
            "exported_at": now_timestamp(),
            "accounts": [],
            "rules": [],
            "kanban_cards": [],
            "kanban_context_notes": {},
            "translate_config": null
        });
        let data = serde_json::to_vec(&backup).unwrap();

        let preview = preview_backup(&data).unwrap();
        assert_eq!(preview.contact_count, 0);
        store.import_settings(&data).unwrap();

        let contacts = store.list_contacts(None, false, 200, 0).unwrap();
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].display_name, "Local contact");
    }

    #[test]
    fn version_two_backup_requires_contacts_field() {
        let backup = serde_json::json!({
            "version": 2,
            "exported_at": now_timestamp(),
            "accounts": [],
            "rules": [],
            "kanban_cards": [],
            "kanban_context_notes": {},
            "translate_config": null
        });
        let data = serde_json::to_vec(&backup).unwrap();

        let error = preview_backup(&data).unwrap_err().to_string();

        assert!(error.contains("contacts"));
        assert!(error.contains("version 2"));
    }

    #[test]
    fn export_rejects_backup_that_cannot_be_restored_due_to_size() {
        let store = Store::open_in_memory().unwrap();
        let notes = "x".repeat(2000);
        store
            .with_write(|conn| {
                let tx = conn.unchecked_transaction()?;
                for index in 0..8_500 {
                    let contact_id = format!("contact-{index}");
                    let email_id = format!("email-{index}");
                    let address = format!("user{index}@example.com");
                    tx.execute(
                        "INSERT INTO contacts
                            (id, display_name, notes, is_favorite, created_at, updated_at)
                         VALUES (?1, ?2, ?3, 0, 1, 1)",
                        rusqlite::params![contact_id, address, notes],
                    )?;
                    tx.execute(
                        "INSERT INTO contact_emails
                            (id, contact_id, address, normalized_address, label, is_primary, created_at)
                         VALUES (?1, ?2, ?3, ?3, 'other', 1, 1)",
                        rusqlite::params![email_id, contact_id, address],
                    )?;
                }
                tx.commit()?;
                Ok(())
            })
            .unwrap();

        let error = store.export_settings().unwrap_err().to_string();

        assert!(error.contains("too large"));
        assert!(error.contains(&MAX_BACKUP_SIZE_BYTES.to_string()));
    }

    #[test]
    fn version_two_contact_restore_rolls_back_on_duplicate_email() {
        let store = Store::open_in_memory().unwrap();
        store
            .save_contact(&ContactInput {
                id: None,
                display_name: "Keep me".to_string(),
                notes: String::new(),
                is_favorite: false,
                emails: vec![ContactEmailInput {
                    id: None,
                    address: "keep@example.com".to_string(),
                    label: ContactEmailLabel::Other,
                    is_primary: true,
                }],
            })
            .unwrap();
        let contact = |id: &str, email_id: &str| {
            serde_json::json!({
                "id": id,
                "display_name": id,
                "notes": "",
                "is_favorite": false,
                "emails": [{
                    "id": email_id,
                    "address": "duplicate@example.com",
                    "label": "other",
                    "is_primary": true
                }],
                "created_at": 1,
                "updated_at": 1
            })
        };
        let backup = serde_json::json!({
            "version": 2,
            "exported_at": now_timestamp(),
            "accounts": [],
            "rules": [],
            "kanban_cards": [],
            "kanban_context_notes": {},
            "contacts": [contact("first", "first-email"), contact("second", "second-email")],
            "translate_config": null
        });

        assert!(store
            .import_settings(&serde_json::to_vec(&backup).unwrap())
            .is_err());
        let contacts = store.list_contacts(None, false, 200, 0).unwrap();
        assert_eq!(contacts.len(), 1);
        assert_eq!(contacts[0].display_name, "Keep me");
    }

    #[test]
    fn test_import_does_not_duplicate_existing_accounts() {
        let store = Store::open_in_memory().unwrap();
        let now = now_timestamp();

        let account = Account {
            account_label: None,
            provider_display_name: None,
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
            account_label: None,
            provider_display_name: None,
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
            contacts: None,
            translate_config: None,
            secret_summary: None,
            encrypted_secrets: None,
        };
        let data = serde_json::to_vec(&backup).unwrap();
        store.import_settings(&data).unwrap();

        let rules = store.list_rules().unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "New Rule");
    }

    #[test]
    fn import_settings_with_private_data_rolls_back_when_private_data_is_invalid() {
        let store = Store::open_in_memory().unwrap();
        let now = now_timestamp();

        let old_rule = Rule {
            id: new_id(),
            name: "Old Rule".to_string(),
            priority: 1,
            conditions: "{}".to_string(),
            actions: "[]".to_string(),
            is_enabled: true,
            created_at: now,
            updated_at: now,
        };
        store.insert_rule(&old_rule).unwrap();

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
            contacts: None,
            translate_config: None,
            secret_summary: None,
            encrypted_secrets: None,
        };
        let data = serde_json::to_vec(&backup).unwrap();

        let err = store
            .import_settings_with_private_data(
                &data,
                RestoredPrivateData {
                    auth_data: vec![RestoredAuthData {
                        account_id: "missing-account".to_string(),
                        provider: "imap".to_string(),
                        encrypted: b"encrypted-auth".to_vec(),
                    }],
                    ..RestoredPrivateData::default()
                },
            )
            .unwrap_err()
            .to_string();

        assert!(err.contains("missing-account"));
        let rules = store.list_rules().unwrap();
        assert_eq!(rules.len(), 1);
        assert_eq!(rules[0].name, "Old Rule");
    }

    #[test]
    fn test_import_replaces_kanban_cards() {
        let store = Store::open_in_memory().unwrap();
        let now = now_timestamp();

        let account = Account {
            account_label: None,
            provider_display_name: None,
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
            contacts: None,
            translate_config: None,
            secret_summary: None,
            encrypted_secrets: None,
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
                account_label: None,
                id: "gmail-account".to_string(),
                email: "gmail@example.com".to_string(),
                display_name: "Gmail User".to_string(),
                color: Some("#3b82f6".to_string()),
                provider: ProviderType::Gmail,
            }],
            rules: vec![],
            kanban_cards: vec![],
            kanban_context_notes: HashMap::new(),
            contacts: None,
            translate_config: None,
            secret_summary: None,
            encrypted_secrets: None,
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
