use crate::commands::encrypted_store::ACTIVE_TRANSLATE_CONFIG_ID;
use crate::commands::encrypted_store::{
    encrypt_account_auth_data, load_account_auth_data, load_secure_user_data,
    store_secure_user_data,
};
use crate::commands::kanban::{
    encrypt_kanban_context_notes_for_state, load_kanban_context_notes_for_state,
    KANBAN_CONTEXT_NOTES_KEY,
};
use crate::commands::translate::{
    decrypt_config as decrypt_translate_config, encrypt_config, validate_restored_translate_config,
};
use crate::state::AppState;
use pebble_core::PebbleError;
use pebble_crypto::passphrase::{
    decrypt_with_passphrase, encrypt_with_passphrase, encrypt_with_stored_passphrase,
    validate_new_passphrase, PassphraseEncryptedBlob, MIN_PASSPHRASE_CHARS,
};
use pebble_store::cloud_sync::{
    preview_backup, serialize_backup, BackupPreview, BackupSecretSummary, RestoredAuthData,
    RestoredPrivateData, RestoredSecureUserData, SettingsBackup, MAX_BACKUP_SIZE_BYTES,
    SETTINGS_BACKUP_FILENAME,
};
use pebble_webdav::WebDavClient;
use serde::{Deserialize, Serialize};
use tauri::{Emitter, Manager, State};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct AccountAuthBackup {
    account_id: String,
    provider: String,
    auth_data: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
struct BackupSecrets {
    #[serde(default)]
    account_auth: Vec<AccountAuthBackup>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    translate_config: Option<String>,
}

impl BackupSecrets {
    fn is_empty(&self) -> bool {
        self.account_auth.is_empty() && self.translate_config.is_none()
    }
}

fn secret_summary(secrets: &BackupSecrets) -> BackupSecretSummary {
    BackupSecretSummary {
        account_auth_count: secrets.account_auth.len(),
        has_translate_config: secrets.translate_config.is_some(),
    }
}

/// A passphrase used to create a backup, and where it came from.
#[derive(Debug, Clone, PartialEq)]
struct BackupPassphrase {
    value: String,
    /// Already saved in the auto-backup config. Such passphrases may predate
    /// the minimum-length policy and must keep working; the policy is only
    /// enforced when the user sets or changes a passphrase.
    stored: bool,
}

impl BackupPassphrase {
    fn entered(value: String) -> Self {
        Self {
            value,
            stored: false,
        }
    }

    fn stored(value: String) -> Self {
        Self {
            value,
            stored: true,
        }
    }
}

fn encrypt_backup_secrets(
    secrets: &BackupSecrets,
    passphrase: &BackupPassphrase,
) -> std::result::Result<PassphraseEncryptedBlob, PebbleError> {
    let plaintext = serde_json::to_vec(secrets)
        .map_err(|e| PebbleError::Internal(format!("Failed to serialize backup secrets: {e}")))?;
    if passphrase.stored {
        encrypt_with_stored_passphrase(&plaintext, &passphrase.value)
    } else {
        encrypt_with_passphrase(&plaintext, &passphrase.value)
    }
}

fn decrypt_backup_secrets(
    encrypted: &PassphraseEncryptedBlob,
    passphrase: &str,
) -> std::result::Result<BackupSecrets, PebbleError> {
    let plaintext = decrypt_with_passphrase(encrypted, passphrase)?;
    serde_json::from_slice(&plaintext)
        .map_err(|e| PebbleError::Validation(format!("Failed to parse backup secrets: {e}")))
}

fn collect_backup_secrets(state: &AppState) -> std::result::Result<BackupSecrets, PebbleError> {
    let mut account_auth = Vec::new();
    for account in state.store.list_accounts()? {
        let Some(decrypted) = load_account_auth_data(&state.crypto, &state.store, &account.id)?
        else {
            continue;
        };
        let auth_data = serde_json::from_slice(&decrypted).map_err(|e| {
            PebbleError::Internal(format!(
                "Failed to parse decrypted auth data for {}: {e}",
                account.email
            ))
        })?;
        account_auth.push(AccountAuthBackup {
            account_id: account.id,
            provider: provider_slug(&account.provider).to_string(),
            auth_data,
        });
    }

    let translate_config = state
        .store
        .get_translate_config()?
        .map(|tc| decrypt_translate_config(state, &tc.config))
        .transpose()?;

    Ok(BackupSecrets {
        account_auth,
        translate_config,
    })
}

fn attach_encrypted_secrets(
    state: &AppState,
    backup: &mut SettingsBackup,
    secret_passphrase: Option<BackupPassphrase>,
) -> std::result::Result<(), PebbleError> {
    let Some(passphrase) = secret_passphrase else {
        return Ok(());
    };
    let secrets = collect_backup_secrets(state)?;
    if secrets.is_empty() {
        return Ok(());
    }
    let encrypted = encrypt_backup_secrets(&secrets, &passphrase)?;
    backup.secret_summary = Some(secret_summary(&secrets));
    backup.encrypted_secrets = Some(serde_json::to_value(encrypted).map_err(|e| {
        PebbleError::Internal(format!("Failed to serialize encrypted backup secrets: {e}"))
    })?);
    Ok(())
}

fn decrypt_secrets_from_backup(
    backup: &SettingsBackup,
    secret_passphrase: Option<String>,
) -> std::result::Result<Option<BackupSecrets>, PebbleError> {
    let Some(value) = &backup.encrypted_secrets else {
        return Ok(None);
    };
    let Some(passphrase) = secret_passphrase else {
        return Err(PebbleError::Validation(
            "This backup contains encrypted account passwords, OAuth tokens, or API keys. Enter the backup encryption password to restore them.".to_string(),
        ));
    };
    let encrypted: PassphraseEncryptedBlob =
        serde_json::from_value(value.clone()).map_err(|e| {
            PebbleError::Validation(format!("Invalid encrypted backup secret payload: {e}"))
        })?;
    decrypt_backup_secrets(&encrypted, &passphrase).map(Some)
}

fn prepare_restored_private_data(
    state: &AppState,
    backup: &SettingsBackup,
    has_kanban_context_notes: bool,
    secrets: Option<BackupSecrets>,
) -> std::result::Result<RestoredPrivateData, PebbleError> {
    let mut private_data = RestoredPrivateData::default();

    if has_kanban_context_notes {
        private_data.secure_user_data.push(RestoredSecureUserData {
            key: KANBAN_CONTEXT_NOTES_KEY.to_string(),
            encrypted: encrypt_kanban_context_notes_for_state(
                state,
                backup.kanban_context_notes.clone(),
            )?,
        });
    }

    let Some(secrets) = secrets else {
        return Ok(private_data);
    };

    for account in secrets.account_auth {
        let auth_bytes = serde_json::to_vec(&account.auth_data).map_err(|e| {
            PebbleError::Internal(format!(
                "Failed to serialize restored auth data for account {}: {e}",
                account.account_id
            ))
        })?;
        let encrypted = encrypt_account_auth_data(&state.crypto, &account.account_id, &auth_bytes)?;
        private_data.auth_data.push(RestoredAuthData {
            account_id: account.account_id,
            provider: account.provider,
            encrypted,
        });
    }

    // The translate config is restored only from the passphrase-encrypted
    // secrets. The plaintext section of the backup is unauthenticated, so it
    // may only contribute the harmless `is_enabled` flag; the provider type is
    // derived from the authenticated config itself.
    if let Some(secret_config) = secrets.translate_config {
        let (provider_type, normalized) = validate_restored_translate_config(&secret_config)?;
        let now = pebble_core::now_timestamp();
        private_data.translate_config = Some(pebble_core::TranslateConfig {
            id: ACTIVE_TRANSLATE_CONFIG_ID.to_string(),
            provider_type,
            config: encrypt_config(state, &normalized)?,
            is_enabled: backup
                .translate_config
                .as_ref()
                .map(|config| config.is_enabled)
                .unwrap_or(true),
            created_at: now,
            updated_at: now,
        });
    }

    Ok(private_data)
}

fn provider_slug(provider: &pebble_core::ProviderType) -> &'static str {
    match provider {
        pebble_core::ProviderType::Imap => "imap",
        pebble_core::ProviderType::Pop3 => "pop3",
        pebble_core::ProviderType::Gmail => "gmail",
        pebble_core::ProviderType::Outlook => "outlook",
    }
}

fn build_backup_data(
    state: &AppState,
    secret_passphrase: Option<BackupPassphrase>,
) -> std::result::Result<Vec<u8>, PebbleError> {
    let exported = state.store.export_settings()?;
    let mut backup: SettingsBackup = serde_json::from_slice(&exported)
        .map_err(|e| PebbleError::Internal(format!("Failed to build backup payload: {e}")))?;
    backup.kanban_context_notes = load_kanban_context_notes_for_state(state)?;
    attach_encrypted_secrets(state, &mut backup, secret_passphrase)?;
    serialize_backup(&backup)
}

fn restore_backup_data(
    state: &AppState,
    data: &[u8],
    secret_passphrase: Option<String>,
) -> std::result::Result<String, PebbleError> {
    // Re-validate before import; `import_settings` enforces size + version too.
    let _ = preview_backup(data)?;
    let backup_value: serde_json::Value = serde_json::from_slice(data)
        .map_err(|e| PebbleError::Validation(format!("Failed to parse backup: {e}")))?;
    let has_kanban_context_notes = backup_value.get("kanban_context_notes").is_some();
    let backup: SettingsBackup = serde_json::from_value(backup_value)
        .map_err(|e| PebbleError::Validation(format!("Failed to parse backup: {e}")))?;
    let backup_secrets = decrypt_secrets_from_backup(&backup, secret_passphrase)?;
    let restored_secrets = backup_secrets.is_some();
    let private_data =
        prepare_restored_private_data(state, &backup, has_kanban_context_notes, backup_secrets)?;
    state
        .store
        .import_settings_with_private_data(data, private_data)?;
    if restored_secrets {
        return Ok(
            "Settings backup restored with saved credentials. Existing OAuth connections and mailboxes with local data were preserved."
                .to_string(),
        );
    }
    Ok("Settings backup restored. Reconnect accounts to continue syncing.".to_string())
}

/// A blank WebDAV password from the UI means "use the saved one". The saved
/// password is only reused for the exact URL and username it was saved for,
/// so it can never be sent to a different server.
/// Backup WebDAV client: https-only, no redirects/proxies, downloads capped at
/// the store's maximum backup size.
fn webdav_client(
    url: String,
    username: String,
    password: String,
) -> Result<WebDavClient, PebbleError> {
    Ok(WebDavClient::new(url, username, password)?.with_max_download_bytes(MAX_BACKUP_SIZE_BYTES))
}

fn resolve_webdav_password(
    state: &AppState,
    url: &str,
    username: &str,
    password: String,
) -> std::result::Result<String, PebbleError> {
    if !password.is_empty() {
        return Ok(password);
    }
    let stored = load_auto_backup_config_inner(&state.store, &state.crypto)?;
    Ok(stored
        .filter(|config| config.url == url && config.username == username)
        .map(|config| config.password)
        .unwrap_or_default())
}

/// A blank backup passphrase from the UI means "use the saved one".
fn resolve_secret_passphrase(
    state: &AppState,
    passphrase: Option<String>,
) -> std::result::Result<Option<BackupPassphrase>, PebbleError> {
    match passphrase {
        Some(value) if value.trim().is_empty() => {
            let stored = load_auto_backup_config_inner(&state.store, &state.crypto)?
                .and_then(|config| config.secret_passphrase)
                .filter(|stored| !stored.trim().is_empty());
            Ok(Some(match stored {
                Some(stored) => BackupPassphrase::stored(stored),
                None => BackupPassphrase::entered(value),
            }))
        }
        other => Ok(other.map(BackupPassphrase::entered)),
    }
}

#[tauri::command]
pub async fn test_webdav_connection(
    state: State<'_, AppState>,
    url: String,
    username: String,
    password: String,
) -> std::result::Result<String, PebbleError> {
    let password = resolve_webdav_password(&state, &url, &username, password)?;
    let client = webdav_client(url, username, password)?;
    client.test_connection().await?;
    Ok("Connection successful".to_string())
}

#[tauri::command]
pub async fn backup_to_webdav(
    state: State<'_, AppState>,
    url: String,
    username: String,
    password: String,
    secret_passphrase: Option<String>,
) -> std::result::Result<String, PebbleError> {
    let password = resolve_webdav_password(&state, &url, &username, password)?;
    let secret_passphrase = resolve_secret_passphrase(&state, secret_passphrase)?;
    let data = build_backup_data(&state, secret_passphrase)?;
    let client = webdav_client(url, username, password)?;
    client.upload(SETTINGS_BACKUP_FILENAME, &data).await?;
    Ok("Settings backup completed successfully".to_string())
}

#[tauri::command]
pub fn export_backup_file(
    state: State<'_, AppState>,
    secret_passphrase: Option<String>,
) -> std::result::Result<String, PebbleError> {
    let secret_passphrase = resolve_secret_passphrase(&state, secret_passphrase)?;
    let data = build_backup_data(&state, secret_passphrase)?;
    String::from_utf8(data)
        .map_err(|e| PebbleError::Internal(format!("Backup JSON was not valid UTF-8: {e}")))
}

#[tauri::command]
pub fn preview_backup_file(data: String) -> std::result::Result<BackupPreview, PebbleError> {
    preview_backup(data.as_bytes())
}

#[tauri::command]
pub fn import_backup_file(
    state: State<'_, AppState>,
    data: String,
    secret_passphrase: Option<String>,
) -> std::result::Result<String, PebbleError> {
    let secret_passphrase =
        resolve_secret_passphrase(&state, secret_passphrase)?.map(|passphrase| passphrase.value);
    restore_backup_data(&state, data.as_bytes(), secret_passphrase)
}

/// Download the backup and return a summary so the user can review the
/// contents before committing to a restore. Enforces size limits and schema
/// version validation in `download` and `preview_backup`.
#[tauri::command]
pub async fn preview_webdav_backup(
    state: State<'_, AppState>,
    url: String,
    username: String,
    password: String,
) -> std::result::Result<BackupPreview, PebbleError> {
    let password = resolve_webdav_password(&state, &url, &username, password)?;
    let client = webdav_client(url, username, password)?;
    let data = client.download(SETTINGS_BACKUP_FILENAME).await?;
    preview_backup(&data)
}

#[tauri::command]
pub async fn restore_from_webdav(
    state: State<'_, AppState>,
    url: String,
    username: String,
    password: String,
    secret_passphrase: Option<String>,
) -> std::result::Result<String, PebbleError> {
    let password = resolve_webdav_password(&state, &url, &username, password)?;
    let secret_passphrase =
        resolve_secret_passphrase(&state, secret_passphrase)?.map(|passphrase| passphrase.value);
    let client = webdav_client(url, username, password)?;
    let data = client.download(SETTINGS_BACKUP_FILENAME).await?;
    restore_backup_data(&state, &data, secret_passphrase)
}

const AUTO_BACKUP_CONFIG_KEY: &str = "auto-backup-config";
const AUTO_BACKUP_CHECK_INTERVAL_SECS: u64 = 60;
const SUPPORTED_AUTO_BACKUP_INTERVAL_MINUTES: &[u64] = &[30, 60, 180, 360, 720, 1440];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AutoBackupConfig {
    pub url: String,
    pub username: String,
    pub password: String,
    pub secret_passphrase: Option<String>,
    pub interval_minutes: u64,
    pub enabled: bool,
}

fn auto_backup_interval_duration(
    interval_minutes: u64,
) -> std::result::Result<std::time::Duration, PebbleError> {
    if !SUPPORTED_AUTO_BACKUP_INTERVAL_MINUTES.contains(&interval_minutes) {
        return Err(PebbleError::Internal(format!(
            "Unsupported auto-backup interval: {interval_minutes} minutes"
        )));
    }
    let seconds = interval_minutes
        .checked_mul(60)
        .ok_or_else(|| PebbleError::Internal("Auto-backup interval is too large".to_string()))?;
    Ok(std::time::Duration::from_secs(seconds))
}

/// What the webview sees of the stored auto-backup configuration: secrets are
/// never returned, only whether they are set.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AutoBackupConfigView {
    pub url: String,
    pub username: String,
    pub has_password: bool,
    pub has_secret_passphrase: bool,
    /// The stored passphrase predates the minimum-length policy. Backups keep
    /// working; the UI only suggests updating it.
    pub secret_passphrase_is_weak: bool,
    pub interval_minutes: u64,
    pub enabled: bool,
}

impl From<&AutoBackupConfig> for AutoBackupConfigView {
    fn from(config: &AutoBackupConfig) -> Self {
        Self {
            url: config.url.clone(),
            username: config.username.clone(),
            has_password: !config.password.is_empty(),
            has_secret_passphrase: config
                .secret_passphrase
                .as_deref()
                .is_some_and(|passphrase| !passphrase.trim().is_empty()),
            secret_passphrase_is_weak: config.secret_passphrase.as_deref().is_some_and(
                |passphrase| {
                    !passphrase.trim().is_empty()
                        && passphrase.trim().chars().count() < MIN_PASSPHRASE_CHARS
                },
            ),
            interval_minutes: config.interval_minutes,
            enabled: config.enabled,
        }
    }
}

/// Merge an auto-backup config submitted by the webview with the stored one.
///
/// * A blank password keeps the stored password, but only when the URL and
///   username are unchanged (the secret must never follow a new server).
/// * `secret_passphrase: None` disables secret backup; `Some("")` keeps the
///   stored passphrase; any other value must satisfy the passphrase policy.
fn merge_auto_backup_config(
    mut submitted: AutoBackupConfig,
    stored: Option<AutoBackupConfig>,
) -> std::result::Result<AutoBackupConfig, PebbleError> {
    let stored = stored.as_ref();
    if submitted.password.is_empty() {
        if let Some(stored) = stored
            .filter(|stored| stored.url == submitted.url && stored.username == submitted.username)
        {
            submitted.password = stored.password.clone();
        }
    }
    submitted.secret_passphrase = match submitted.secret_passphrase.take() {
        None => None,
        Some(passphrase) if passphrase.trim().is_empty() => {
            let stored_passphrase = stored
                .and_then(|stored| stored.secret_passphrase.clone())
                .filter(|stored| !stored.trim().is_empty());
            Some(stored_passphrase.ok_or_else(|| {
                PebbleError::Validation(
                    "Enter a backup encryption password before including secrets".to_string(),
                )
            })?)
        }
        Some(passphrase) => {
            validate_new_passphrase(&passphrase)?;
            Some(passphrase)
        }
    };
    Ok(submitted)
}

#[tauri::command]
pub fn save_auto_backup_config(
    state: State<'_, AppState>,
    config: AutoBackupConfig,
) -> std::result::Result<(), PebbleError> {
    auto_backup_interval_duration(config.interval_minutes)?;
    // An unreadable stored config must not block saving a fresh one.
    let stored = load_auto_backup_config_inner(&state.store, &state.crypto)
        .ok()
        .flatten();
    let config = merge_auto_backup_config(config, stored)?;
    let json = serde_json::to_vec(&config)
        .map_err(|e| PebbleError::Internal(format!("Failed to serialize config: {e}")))?;
    store_secure_user_data(&state.crypto, &state.store, AUTO_BACKUP_CONFIG_KEY, &json)
}

#[tauri::command]
pub fn load_auto_backup_config(
    state: State<'_, AppState>,
) -> std::result::Result<Option<AutoBackupConfigView>, PebbleError> {
    Ok(load_auto_backup_config_inner(&state.store, &state.crypto)?
        .as_ref()
        .map(AutoBackupConfigView::from))
}

fn load_auto_backup_config_inner(
    store: &pebble_store::Store,
    crypto: &pebble_crypto::CryptoService,
) -> std::result::Result<Option<AutoBackupConfig>, PebbleError> {
    let Some(json) = load_secure_user_data(crypto, store, AUTO_BACKUP_CONFIG_KEY)? else {
        return Ok(None);
    };
    let config: AutoBackupConfig = serde_json::from_slice(&json)
        .map_err(|e| PebbleError::Internal(format!("Failed to deserialize config: {e}")))?;
    auto_backup_interval_duration(config.interval_minutes)?;
    Ok(Some(config))
}

#[tauri::command]
pub fn delete_auto_backup_config(
    state: State<'_, AppState>,
) -> std::result::Result<(), PebbleError> {
    state
        .store
        .delete_secure_user_data(AUTO_BACKUP_CONFIG_KEY)?;
    Ok(())
}

pub async fn run_auto_backup_worker(app: tauri::AppHandle) {
    let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(
        AUTO_BACKUP_CHECK_INTERVAL_SECS,
    ));
    let mut last_backup_at: Option<std::time::Instant> = None;

    loop {
        interval.tick().await;

        let state = app.state::<AppState>();
        let config = match load_auto_backup_config_inner(&state.store, &state.crypto) {
            Ok(Some(c)) if c.enabled => c,
            Ok(_) => continue,
            Err(error) => {
                tracing::warn!("[auto-backup] invalid stored config; backup disabled: {error}");
                continue;
            }
        };

        let backup_interval = match auto_backup_interval_duration(config.interval_minutes) {
            Ok(interval) => interval,
            Err(error) => {
                tracing::warn!("[auto-backup] invalid interval; backup disabled: {error}");
                continue;
            }
        };
        if let Some(last) = last_backup_at {
            if last.elapsed() < backup_interval {
                continue;
            }
        }

        tracing::info!("[auto-backup] starting scheduled WebDAV backup");
        let backup_result: std::result::Result<(), PebbleError> = async {
            // The stored passphrase may predate the 12-character minimum.
            let passphrase = config
                .secret_passphrase
                .clone()
                .map(BackupPassphrase::stored);
            let data = build_backup_data(&state, passphrase)?;
            let client = webdav_client(config.url, config.username, config.password)?;
            client.upload(SETTINGS_BACKUP_FILENAME, &data).await?;
            Ok(())
        }
        .await;

        match backup_result {
            Ok(()) => {
                last_backup_at = Some(std::time::Instant::now());
                tracing::info!("[auto-backup] backup completed successfully");
                let _ = app.emit("cloud-sync:auto-backup-complete", ());
            }
            Err(e) => {
                tracing::warn!("[auto-backup] backup failed: {e}");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_backup_interval_accepts_only_supported_values() {
        for minutes in [30, 60, 180, 360, 720, 1440] {
            assert_eq!(
                auto_backup_interval_duration(minutes).unwrap(),
                std::time::Duration::from_secs(minutes * 60),
            );
        }
        for minutes in [0, 15, 1441, u64::MAX] {
            assert!(auto_backup_interval_duration(minutes).is_err());
        }
    }

    #[test]
    fn encrypted_backup_secrets_round_trip_without_plaintext_leaks() {
        let secrets = BackupSecrets {
            account_auth: vec![AccountAuthBackup {
                account_id: "imap-account".to_string(),
                provider: "imap".to_string(),
                auth_data: serde_json::json!({
                    "imap": { "password": "imap-secret" },
                    "smtp": { "password": "smtp-secret" }
                }),
            }],
            translate_config: Some(r#"{"type":"deepl","api_key":"deepl-secret"}"#.to_string()),
        };

        let passphrase = BackupPassphrase::entered("backup passphrase long".to_string());
        let encrypted = encrypt_backup_secrets(&secrets, &passphrase).unwrap();
        let serialized = serde_json::to_string(&encrypted).unwrap();

        assert!(!serialized.contains("imap-secret"));
        assert!(!serialized.contains("smtp-secret"));
        assert!(!serialized.contains("deepl-secret"));

        let decrypted = decrypt_backup_secrets(&encrypted, "backup passphrase long").unwrap();
        assert_eq!(decrypted, secrets);
    }

    #[test]
    fn backup_secret_summary_counts_accounts_and_translate_config() {
        let secrets = BackupSecrets {
            account_auth: vec![
                AccountAuthBackup {
                    account_id: "a1".to_string(),
                    provider: "imap".to_string(),
                    auth_data: serde_json::json!({"password":"one"}),
                },
                AccountAuthBackup {
                    account_id: "a2".to_string(),
                    provider: "gmail".to_string(),
                    auth_data: serde_json::json!({"access_token":"token"}),
                },
            ],
            translate_config: Some(r#"{"api_key":"key"}"#.to_string()),
        };

        let summary = secret_summary(&secrets);

        assert_eq!(summary.account_auth_count, 2);
        assert!(summary.has_translate_config);
    }

    fn auto_config(password: &str, passphrase: Option<&str>) -> AutoBackupConfig {
        AutoBackupConfig {
            url: "https://dav.example.com/backup".to_string(),
            username: "user".to_string(),
            password: password.to_string(),
            secret_passphrase: passphrase.map(str::to_string),
            interval_minutes: 60,
            enabled: true,
        }
    }

    #[test]
    fn stored_short_passphrase_keeps_working_but_new_short_one_is_rejected() {
        let secrets = BackupSecrets {
            account_auth: vec![],
            translate_config: Some(r#"{"type":"deeplx","endpoint":"http://localhost:1"}"#.into()),
        };
        let stored =
            encrypt_backup_secrets(&secrets, &BackupPassphrase::stored("short".into())).unwrap();
        assert_eq!(decrypt_backup_secrets(&stored, "short").unwrap(), secrets);
        assert!(
            encrypt_backup_secrets(&secrets, &BackupPassphrase::entered("short".into())).is_err()
        );

        // Keeping a stored short passphrase on save is allowed and flagged as weak.
        let merged = merge_auto_backup_config(
            auto_config("", Some("")),
            Some(auto_config("pw", Some("short"))),
        )
        .unwrap();
        assert_eq!(merged.secret_passphrase.as_deref(), Some("short"));
        assert!(AutoBackupConfigView::from(&merged).secret_passphrase_is_weak);
        assert!(
            !AutoBackupConfigView::from(&auto_config("pw", Some("long enough passphrase")))
                .secret_passphrase_is_weak
        );
    }

    #[test]
    fn auto_backup_view_never_exposes_secrets() {
        let config = auto_config("webdav-secret", Some("backup passphrase"));
        let view = AutoBackupConfigView::from(&config);
        let json = serde_json::to_string(&view).unwrap();

        assert!(view.has_password);
        assert!(view.has_secret_passphrase);
        assert!(!json.contains("webdav-secret"));
        assert!(!json.contains("backup passphrase"));
    }

    #[test]
    fn blank_auto_backup_secrets_keep_stored_values() {
        let stored = auto_config("webdav-secret", Some("stored passphrase"));
        let merged = merge_auto_backup_config(auto_config("", Some("")), Some(stored)).unwrap();

        assert_eq!(merged.password, "webdav-secret");
        assert_eq!(
            merged.secret_passphrase.as_deref(),
            Some("stored passphrase")
        );
    }

    #[test]
    fn stored_webdav_password_does_not_follow_a_new_server() {
        let stored = auto_config("webdav-secret", None);
        let mut submitted = auto_config("", None);
        submitted.url = "https://attacker.example/dav".to_string();

        let merged = merge_auto_backup_config(submitted, Some(stored)).unwrap();

        assert_eq!(merged.password, "");
    }

    #[test]
    fn new_auto_backup_passphrase_must_meet_policy_and_none_clears_it() {
        let stored = auto_config("pw", Some("stored passphrase"));
        assert!(
            merge_auto_backup_config(auto_config("", Some("short")), Some(stored.clone())).is_err()
        );
        let cleared = merge_auto_backup_config(auto_config("", None), Some(stored)).unwrap();
        assert_eq!(cleared.secret_passphrase, None);
        assert!(merge_auto_backup_config(auto_config("", Some(" ")), None).is_err());
    }

    #[test]
    fn preview_backup_file_accepts_backup_json_text() {
        let backup = serde_json::json!({
            "version": 1,
            "exported_at": 1,
            "accounts": [],
            "rules": [],
            "kanban_cards": [],
            "kanban_context_notes": {},
            "translate_config": null
        });

        let preview = preview_backup_file(backup.to_string()).unwrap();

        assert_eq!(preview.version, 1);
        assert_eq!(preview.account_count, 0);
    }
}
