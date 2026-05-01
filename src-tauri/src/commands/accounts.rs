use crate::account_colors::default_account_color;
use crate::commands::network::{
    account_proxy_setting_from_parts, get_global_proxy_raw, http_proxy_from_mail_proxy,
    is_inherit_proxy_mode, mail_proxy_from_http, normalize_account_proxy_setting,
    proxy_config_from_parts, resolve_effective_proxy, resolve_mail_proxy_from_mode,
    AccountProxyMode, AccountProxySetting,
};
use crate::commands::oauth::ensure_account_oauth_auth;
use crate::state::AppState;
use pebble_core::traits::FolderProvider;
use pebble_core::{new_id, now_timestamp, Account, HttpProxyConfig, PebbleError, ProviderType};
use pebble_mail::GmailProvider;
use pebble_mail::OutlookProvider;
use pebble_mail::{ConnectionSecurity, ImapConfig, ProxyConfig, SmtpConfig};
use serde::{Deserialize, Serialize};
use tauri::State;

/// Typed view of the encrypted `auth_data` blob for an IMAP/SMTP account.
///
/// Prior code patched this blob with hand-written `serde_json::Value`
/// mutations, which silently dropped fields when serde and JSON shapes
/// drifted. Parsing into this struct makes the shape explicit and reuses
/// `ImapConfig` / `SmtpConfig`'s own legacy-aware deserializers.
#[derive(Debug, Clone)]
struct AccountCredentials {
    proxy_mode: AccountProxyMode,
    imap: ImapConfig,
    smtp: SmtpConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredAccountCredentials {
    #[serde(default, skip_serializing_if = "is_inherit_proxy_mode")]
    proxy_mode: AccountProxyMode,
    imap: StoredMailConfig,
    smtp: StoredMailConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredMailConfig {
    host: String,
    port: u16,
    username: String,
    #[serde(default)]
    password: String,
    #[serde(default)]
    security: Option<ConnectionSecurity>,
    #[serde(default)]
    use_tls: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    proxy: Option<ProxyConfig>,
}

impl StoredMailConfig {
    fn security(&self) -> ConnectionSecurity {
        self.security.clone().unwrap_or(match self.use_tls {
            Some(false) => ConnectionSecurity::Plain,
            _ => ConnectionSecurity::Tls,
        })
    }

    fn into_imap(self) -> ImapConfig {
        let security = self.security();
        ImapConfig {
            host: self.host,
            port: self.port,
            username: self.username,
            password: self.password,
            security,
            proxy: self.proxy,
        }
    }

    fn into_smtp(self) -> SmtpConfig {
        let security = self.security();
        SmtpConfig {
            host: self.host,
            port: self.port,
            username: self.username,
            password: self.password,
            security,
            proxy: self.proxy,
        }
    }
}

impl From<&ImapConfig> for StoredMailConfig {
    fn from(value: &ImapConfig) -> Self {
        Self {
            host: value.host.clone(),
            port: value.port,
            username: value.username.clone(),
            password: value.password.clone(),
            security: Some(value.security.clone()),
            use_tls: None,
            proxy: value.proxy.clone(),
        }
    }
}

impl From<&SmtpConfig> for StoredMailConfig {
    fn from(value: &SmtpConfig) -> Self {
        Self {
            host: value.host.clone(),
            port: value.port,
            username: value.username.clone(),
            password: value.password.clone(),
            security: Some(value.security.clone()),
            use_tls: None,
            proxy: value.proxy.clone(),
        }
    }
}

impl From<&AccountCredentials> for StoredAccountCredentials {
    fn from(value: &AccountCredentials) -> Self {
        Self {
            proxy_mode: value.proxy_mode,
            imap: StoredMailConfig::from(&value.imap),
            smtp: StoredMailConfig::from(&value.smtp),
        }
    }
}

impl From<StoredAccountCredentials> for AccountCredentials {
    fn from(value: StoredAccountCredentials) -> Self {
        Self {
            proxy_mode: value.proxy_mode,
            imap: value.imap.into_imap(),
            smtp: value.smtp.into_smtp(),
        }
    }
}

impl Serialize for AccountCredentials {
    fn serialize<S>(&self, serializer: S) -> std::result::Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        StoredAccountCredentials::from(self).serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for AccountCredentials {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        StoredAccountCredentials::deserialize(deserializer).map(Into::into)
    }
}

fn serialize_account_credentials(
    credentials: &AccountCredentials,
) -> std::result::Result<Vec<u8>, PebbleError> {
    serde_json::to_vec(credentials)
        .map_err(|e| PebbleError::Internal(format!("Failed to serialize config: {e}")))
}

fn deserialize_account_credentials(
    bytes: &[u8],
) -> std::result::Result<AccountCredentials, PebbleError> {
    serde_json::from_slice(bytes)
        .map_err(|e| PebbleError::Internal(format!("Failed to parse config: {e}")))
}

fn account_proxy_from_credentials(credentials: &AccountCredentials) -> Option<HttpProxyConfig> {
    credentials
        .imap
        .proxy
        .as_ref()
        .or(credentials.smtp.proxy.as_ref())
        .map(http_proxy_from_mail_proxy)
}

fn account_proxy_setting_from_credentials(credentials: &AccountCredentials) -> AccountProxySetting {
    normalize_account_proxy_setting(
        credentials.proxy_mode,
        account_proxy_from_credentials(credentials),
    )
}

fn set_account_proxy_setting_on_credentials(
    credentials: &mut AccountCredentials,
    setting: AccountProxySetting,
) {
    credentials.proxy_mode = setting.mode;
    let proxy = setting.proxy.map(mail_proxy_from_http);
    credentials.imap.proxy = proxy.clone();
    credentials.smtp.proxy = proxy;
}

#[cfg(test)]
fn set_account_proxy_on_credentials(
    credentials: &mut AccountCredentials,
    proxy: Option<HttpProxyConfig>,
) {
    let setting = AccountProxySetting {
        mode: if proxy.is_some() {
            AccountProxyMode::Custom
        } else {
            AccountProxyMode::Inherit
        },
        proxy,
    };
    set_account_proxy_setting_on_credentials(credentials, setting);
}

fn is_loopback_mail_host(host: &str) -> bool {
    matches!(
        host.trim()
            .trim_matches(&['[', ']'][..])
            .to_ascii_lowercase()
            .as_str(),
        "localhost" | "127.0.0.1" | "::1"
    )
}

fn validate_connection_security(
    label: &str,
    host: &str,
    security: &ConnectionSecurity,
) -> std::result::Result<(), PebbleError> {
    if matches!(security, ConnectionSecurity::Plain) && !is_loopback_mail_host(host) {
        return Err(PebbleError::Validation(format!(
            "{label} plaintext connections are only allowed for localhost"
        )));
    }
    Ok(())
}

fn validate_account_color(color: Option<&str>) -> std::result::Result<(), PebbleError> {
    let Some(color) = color else {
        return Ok(());
    };

    if color.len() == 7
        && color.as_bytes()[0] == b'#'
        && color.as_bytes()[1..].iter().all(|b| b.is_ascii_hexdigit())
    {
        Ok(())
    } else {
        Err(PebbleError::Validation(
            "Account color must be a hex color like #22c55e".to_string(),
        ))
    }
}

#[allow(dead_code)]
#[derive(Deserialize)]
pub struct AddAccountRequest {
    pub email: String,
    pub display_name: String,
    pub provider: String,
    pub imap_host: String,
    pub imap_port: u16,
    pub smtp_host: String,
    pub smtp_port: u16,
    pub username: String,
    pub password: String,
    pub imap_security: ConnectionSecurity,
    pub smtp_security: ConnectionSecurity,
    #[serde(default)]
    pub proxy_host: Option<String>,
    #[serde(default)]
    pub proxy_port: Option<u16>,
}

impl std::fmt::Debug for AddAccountRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AddAccountRequest")
            .field("email", &self.email)
            .field("provider", &self.provider)
            .field("password", &"[REDACTED]")
            .finish_non_exhaustive()
    }
}

#[tauri::command]
pub async fn add_account(
    state: State<'_, AppState>,
    request: AddAccountRequest,
) -> std::result::Result<Account, PebbleError> {
    let now = now_timestamp();
    let provider = match request.provider.to_lowercase().as_str() {
        "gmail" => ProviderType::Gmail,
        "outlook" => ProviderType::Outlook,
        _ => ProviderType::Imap,
    };

    validate_connection_security("IMAP", &request.imap_host, &request.imap_security)?;
    validate_connection_security("SMTP", &request.smtp_host, &request.smtp_security)?;

    let existing_accounts = state.store.list_accounts()?;
    let account = Account {
        id: new_id(),
        email: request.email.clone(),
        display_name: request.display_name.clone(),
        color: Some(default_account_color(&existing_accounts, &request.email)),
        provider: provider.clone(),
        created_at: now,
        updated_at: now,
    };

    state.store.insert_account(&account)?;

    // If any subsequent step fails, delete the account row to prevent half-creation
    if let Err(e) = (|| -> std::result::Result<(), PebbleError> {
        let proxy =
            proxy_config_from_parts(request.proxy_host, request.proxy_port, "Account proxy")?;
        let proxy_mode = if proxy.is_some() {
            AccountProxyMode::Custom
        } else {
            AccountProxyMode::Inherit
        };
        let proxy = proxy.map(mail_proxy_from_http);

        // Build typed IMAP + SMTP credentials
        let credentials = AccountCredentials {
            proxy_mode,
            imap: ImapConfig {
                host: request.imap_host,
                port: request.imap_port,
                username: request.username.clone(),
                password: request.password.clone(),
                security: request.imap_security,
                proxy: proxy.clone(),
            },
            smtp: SmtpConfig {
                host: request.smtp_host,
                port: request.smtp_port,
                username: request.username,
                password: request.password,
                security: request.smtp_security,
                proxy,
            },
        };

        // Encrypt credentials and store as auth_data
        let config_bytes = serialize_account_credentials(&credentials)?;
        let encrypted = state.crypto.encrypt(&config_bytes)?;
        state.store.set_auth_data(&account.id, &encrypted)?;

        // Store non-secret metadata in sync_state
        let provider_slug = match provider {
            ProviderType::Gmail => "gmail",
            ProviderType::Outlook => "outlook",
            ProviderType::Imap => "imap",
        };
        state.store.update_sync_state(&account.id, |s| {
            s.provider = Some(provider_slug.to_string());
        })?;
        Ok(())
    })() {
        // Rollback: remove the partially created account
        let _ = state.store.delete_account(&account.id);
        return Err(e);
    }

    Ok(account)
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
pub async fn update_account(
    state: State<'_, AppState>,
    account_id: String,
    email: String,
    display_name: String,
    password: Option<String>,
    imap_host: Option<String>,
    imap_port: Option<u16>,
    smtp_host: Option<String>,
    smtp_port: Option<u16>,
    imap_security: Option<ConnectionSecurity>,
    smtp_security: Option<ConnectionSecurity>,
    proxy_host: Option<String>,
    proxy_port: Option<u16>,
    account_color: Option<String>,
) -> std::result::Result<(), PebbleError> {
    validate_account_color(account_color.as_deref())?;

    let credentials_dirty = password.is_some()
        || imap_host.is_some()
        || smtp_host.is_some()
        || imap_port.is_some()
        || smtp_port.is_some()
        || imap_security.is_some()
        || smtp_security.is_some()
        || proxy_host.is_some()
        || proxy_port.is_some();
    if !credentials_dirty {
        state
            .store
            .update_account(&account_id, &email, &display_name, account_color.as_deref())?;
        return Ok(());
    }

    // Parse the existing encrypted blob into a typed view. If the row is
    // missing (first-time edit, or a legacy OAuth-only account moving to
    // IMAP), seed a blank template that the mutations below can fill in.
    let mut creds: AccountCredentials = match state.store.get_auth_data(&account_id)? {
        Some(encrypted) => {
            let decrypted = state.crypto.decrypt(&encrypted)?;
            deserialize_account_credentials(&decrypted)?
        }
        None => AccountCredentials {
            proxy_mode: AccountProxyMode::Inherit,
            imap: ImapConfig {
                host: String::new(),
                port: 0,
                username: String::new(),
                password: String::new(),
                security: ConnectionSecurity::default(),
                proxy: None,
            },
            smtp: SmtpConfig {
                host: String::new(),
                port: 0,
                username: String::new(),
                password: String::new(),
                security: ConnectionSecurity::default(),
                proxy: None,
            },
        },
    };

    let updated_proxy = if proxy_host.is_some() || proxy_port.is_some() {
        Some(
            proxy_config_from_parts(proxy_host.clone(), proxy_port, "Account proxy")?
                .map(mail_proxy_from_http),
        )
    } else {
        None
    };

    // IMAP side
    if let Some(h) = imap_host {
        creds.imap.host = h;
    }
    if let Some(p) = imap_port {
        creds.imap.port = p;
    }
    if let Some(ref pw) = password {
        creds.imap.password = pw.clone();
    }
    if let Some(sec) = imap_security {
        creds.imap.security = sec;
    }
    if let Some(proxy) = &updated_proxy {
        creds.proxy_mode = if proxy.is_some() {
            AccountProxyMode::Custom
        } else {
            AccountProxyMode::Inherit
        };
        creds.imap.proxy = proxy.clone();
    }
    if creds.imap.username.is_empty() {
        creds.imap.username = email.clone();
    }

    // SMTP side
    if let Some(h) = smtp_host {
        creds.smtp.host = h;
    }
    if let Some(p) = smtp_port {
        creds.smtp.port = p;
    }
    if let Some(ref pw) = password {
        creds.smtp.password = pw.clone();
    }
    if let Some(sec) = smtp_security {
        creds.smtp.security = sec;
    }
    // Mirror IMAP proxy to SMTP — both connections share the same network path.
    if let Some(proxy) = updated_proxy {
        creds.smtp.proxy = proxy;
    }
    if creds.smtp.username.is_empty() {
        creds.smtp.username = email.clone();
    }

    validate_connection_security("IMAP", &creds.imap.host, &creds.imap.security)?;
    validate_connection_security("SMTP", &creds.smtp.host, &creds.smtp.security)?;

    state
        .store
        .update_account(&account_id, &email, &display_name, account_color.as_deref())?;

    let config_bytes = serialize_account_credentials(&creds)?;
    let encrypted = state.crypto.encrypt(&config_bytes)?;
    state.store.set_auth_data(&account_id, &encrypted)?;

    Ok(())
}

#[tauri::command]
pub async fn get_account_proxy(
    state: State<'_, AppState>,
    account_id: String,
) -> std::result::Result<Option<HttpProxyConfig>, PebbleError> {
    Ok(get_account_proxy_setting(state, account_id).await?.proxy)
}

#[tauri::command]
pub async fn get_account_proxy_setting(
    state: State<'_, AppState>,
    account_id: String,
) -> std::result::Result<AccountProxySetting, PebbleError> {
    let account = state
        .store
        .get_account(&account_id)?
        .ok_or_else(|| PebbleError::Internal(format!("Account not found: {account_id}")))?;
    if account.provider != ProviderType::Imap {
        return Err(PebbleError::UnsupportedProvider(
            "Use the OAuth account proxy commands for Gmail and Outlook accounts".to_string(),
        ));
    }

    let Some(encrypted) = state.store.get_auth_data(&account_id)? else {
        return Ok(AccountProxySetting {
            mode: AccountProxyMode::Inherit,
            proxy: None,
        });
    };
    let decrypted = state.crypto.decrypt(&encrypted)?;
    let credentials = deserialize_account_credentials(&decrypted)?;
    Ok(account_proxy_setting_from_credentials(&credentials))
}

#[tauri::command]
pub async fn update_account_proxy(
    state: State<'_, AppState>,
    account_id: String,
    proxy_host: Option<String>,
    proxy_port: Option<u16>,
) -> std::result::Result<(), PebbleError> {
    let proxy = proxy_config_from_parts(proxy_host, proxy_port, "Account proxy")?;
    let mode = if proxy.is_some() {
        AccountProxyMode::Custom
    } else {
        AccountProxyMode::Inherit
    };
    match proxy {
        Some(proxy) => {
            update_account_proxy_setting(
                state,
                account_id,
                mode,
                Some(proxy.host),
                Some(proxy.port),
            )
            .await
        }
        None => update_account_proxy_setting(state, account_id, mode, None, None).await,
    }
}

#[tauri::command]
pub async fn update_account_proxy_setting(
    state: State<'_, AppState>,
    account_id: String,
    mode: AccountProxyMode,
    proxy_host: Option<String>,
    proxy_port: Option<u16>,
) -> std::result::Result<(), PebbleError> {
    let account = state
        .store
        .get_account(&account_id)?
        .ok_or_else(|| PebbleError::Internal(format!("Account not found: {account_id}")))?;
    if account.provider != ProviderType::Imap {
        return Err(PebbleError::UnsupportedProvider(
            "Use the OAuth account proxy commands for Gmail and Outlook accounts".to_string(),
        ));
    }

    let setting = account_proxy_setting_from_parts(mode, proxy_host, proxy_port, "Account proxy")?;
    let Some(encrypted) = state.store.get_auth_data(&account_id)? else {
        return Err(PebbleError::Internal(format!(
            "No auth data found for account {account_id}"
        )));
    };
    let decrypted = state.crypto.decrypt(&encrypted)?;
    let mut credentials = deserialize_account_credentials(&decrypted)?;
    set_account_proxy_setting_on_credentials(&mut credentials, setting);
    let config_bytes = serialize_account_credentials(&credentials)?;
    let encrypted = state.crypto.encrypt(&config_bytes)?;
    state.store.set_auth_data(&account_id, &encrypted)?;
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct TestConnectionRequest {
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: ConnectionSecurity,
    #[serde(default)]
    pub proxy_host: Option<String>,
    #[serde(default)]
    pub proxy_port: Option<u16>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
}

#[tauri::command]
pub async fn test_imap_connection(
    state: State<'_, AppState>,
    request: TestConnectionRequest,
) -> std::result::Result<String, PebbleError> {
    validate_connection_security("IMAP", &request.imap_host, &request.imap_security)?;

    let requested_proxy = proxy_config_from_parts(
        request.proxy_host,
        request.proxy_port,
        "Connection test proxy",
    )?;
    let proxy = resolve_effective_proxy(
        requested_proxy,
        get_global_proxy_raw(&state.crypto, &state.store)?,
    )
    .map(mail_proxy_from_http);
    let has_credentials = request.username.as_ref().is_some_and(|u| !u.is_empty())
        && request.password.as_ref().is_some_and(|p| !p.is_empty());
    let config = pebble_mail::ImapConfig {
        host: request.imap_host,
        port: request.imap_port,
        username: request.username.unwrap_or_default(),
        password: request.password.unwrap_or_default(),
        security: request.imap_security,
        proxy,
    };
    if has_credentials {
        pebble_mail::ImapProvider::test_connection_with_login(&config).await
    } else {
        pebble_mail::ImapProvider::test_connection(&config).await
    }
}

#[tauri::command]
pub async fn test_account_connection(
    state: State<'_, AppState>,
    account_id: String,
) -> std::result::Result<String, PebbleError> {
    let account = state
        .store
        .get_account(&account_id)?
        .ok_or_else(|| PebbleError::Internal(format!("Account not found: {account_id}")))?;

    if matches!(account.provider, ProviderType::Gmail) {
        let auth = ensure_account_oauth_auth(&state, &account_id, "gmail").await?;
        let provider = GmailProvider::new_with_proxy(auth.tokens.access_token, auth.proxy)?;
        let (email, _history_id) = provider.get_profile().await?;
        if email.is_empty() {
            return Ok("Gmail connection successful".to_string());
        }
        return Ok(format!("Gmail connection successful ({email})"));
    }

    if matches!(account.provider, ProviderType::Outlook) {
        let auth = ensure_account_oauth_auth(&state, &account_id, "outlook").await?;
        let provider = OutlookProvider::new_with_proxy(
            auth.tokens.access_token,
            account_id.clone(),
            auth.proxy,
        )?;
        // Graph connectivity check: list mail folders.
        let folders = provider.list_folders().await?;
        return Ok(format!(
            "Outlook connection successful ({} folders)",
            folders.len()
        ));
    }

    let existing = state
        .store
        .get_auth_data(&account_id)?
        .ok_or_else(|| PebbleError::Internal("No auth data found".into()))?;
    let decrypted = state.crypto.decrypt(&existing)?;
    let mut credentials = deserialize_account_credentials(&decrypted)?;
    credentials.imap.proxy = resolve_mail_proxy_from_mode(
        &state.crypto,
        &state.store,
        credentials.proxy_mode,
        credentials.imap.proxy.clone(),
    )?;
    pebble_mail::ImapProvider::test_connection(&credentials.imap).await
}

#[tauri::command]
pub async fn list_accounts(
    state: State<'_, AppState>,
) -> std::result::Result<Vec<Account>, PebbleError> {
    state.store.list_accounts()
}

#[tauri::command]
pub async fn delete_account(
    state: State<'_, AppState>,
    account_id: String,
) -> std::result::Result<(), PebbleError> {
    // 1. Stop sync if running
    {
        let mut handles = state.sync_handles.lock().await;
        if let Some(handle) = handles.remove(&account_id) {
            let _ = handle.stop_tx.send(true);
            handle.task.abort();
        }
    }

    // 2. Collect message IDs for attachment cleanup (before DB delete)
    let message_ids = match state.store.list_message_ids_by_account(&account_id) {
        Ok(ids) => ids,
        Err(e) => {
            tracing::warn!(
                "Failed to collect message IDs for attachment cleanup (account {account_id}): {e}"
            );
            Vec::new()
        }
    };

    // 3. Remove all documents from search index
    if let Err(e) = state.search.delete_by_account(&account_id) {
        tracing::warn!("Failed to clean search index for account {account_id}: {e}");
    }

    // 4. Delete account from DB (CASCADE handles related rows)
    state.store.delete_account(&account_id)?;

    // 5. Clean up attachment files on disk
    let attachments_dir = state.attachments_dir.clone();
    let account_id_for_log = account_id.clone();
    if let Err(e) = tokio::task::spawn_blocking(move || {
        for msg_id in &message_ids {
            let msg_dir = attachments_dir.join(msg_id);
            if msg_dir.exists() {
                if let Err(e) = std::fs::remove_dir_all(&msg_dir) {
                    tracing::warn!("Failed to remove attachments for message {msg_id}: {e}");
                }
            }
        }
    })
    .await
    {
        tracing::warn!("Attachment cleanup task failed for account {account_id_for_log}: {e}");
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_plaintext_security_for_remote_hosts() {
        let err =
            validate_connection_security("IMAP", "mail.example.com", &ConnectionSecurity::Plain)
                .expect_err("remote plaintext mail connections must be rejected");

        assert!(matches!(err, PebbleError::Validation(_)));
    }

    #[test]
    fn allows_plaintext_security_for_localhost() {
        validate_connection_security("IMAP", "localhost", &ConnectionSecurity::Plain)
            .expect("localhost plaintext is useful for local test servers");
        validate_connection_security("SMTP", "127.0.0.1", &ConnectionSecurity::Plain)
            .expect("loopback plaintext is useful for local test servers");
    }

    #[test]
    fn account_credentials_storage_round_trip_preserves_passwords() {
        let credentials = AccountCredentials {
            proxy_mode: AccountProxyMode::Inherit,
            imap: ImapConfig {
                host: "imap.example.com".to_string(),
                port: 993,
                username: "user@example.com".to_string(),
                password: "imap-secret".to_string(),
                security: ConnectionSecurity::Tls,
                proxy: None,
            },
            smtp: SmtpConfig {
                host: "smtp.example.com".to_string(),
                port: 465,
                username: "user@example.com".to_string(),
                password: "smtp-secret".to_string(),
                security: ConnectionSecurity::Tls,
                proxy: None,
            },
        };

        let bytes = serialize_account_credentials(&credentials).unwrap();
        let decoded: AccountCredentials = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(decoded.imap.password, "imap-secret");
        assert_eq!(decoded.smtp.password, "smtp-secret");
    }

    #[test]
    fn account_proxy_from_credentials_reads_imap_proxy() {
        let credentials = AccountCredentials {
            proxy_mode: AccountProxyMode::Inherit,
            imap: ImapConfig {
                host: "imap.example.com".to_string(),
                port: 993,
                username: "user@example.com".to_string(),
                password: "imap-secret".to_string(),
                security: ConnectionSecurity::Tls,
                proxy: Some(ProxyConfig {
                    host: "127.0.0.1".to_string(),
                    port: 7890,
                }),
            },
            smtp: SmtpConfig {
                host: "smtp.example.com".to_string(),
                port: 465,
                username: "user@example.com".to_string(),
                password: "smtp-secret".to_string(),
                security: ConnectionSecurity::Tls,
                proxy: None,
            },
        };

        let proxy = account_proxy_from_credentials(&credentials).unwrap();

        assert_eq!(proxy.host, "127.0.0.1");
        assert_eq!(proxy.port, 7890);
    }

    #[test]
    fn set_account_proxy_on_credentials_mirrors_imap_and_smtp() {
        let mut credentials = AccountCredentials {
            proxy_mode: AccountProxyMode::Inherit,
            imap: ImapConfig {
                host: "imap.example.com".to_string(),
                port: 993,
                username: "user@example.com".to_string(),
                password: "imap-secret".to_string(),
                security: ConnectionSecurity::Tls,
                proxy: None,
            },
            smtp: SmtpConfig {
                host: "smtp.example.com".to_string(),
                port: 465,
                username: "user@example.com".to_string(),
                password: "smtp-secret".to_string(),
                security: ConnectionSecurity::Tls,
                proxy: None,
            },
        };

        set_account_proxy_on_credentials(
            &mut credentials,
            Some(pebble_core::HttpProxyConfig {
                host: "10.0.0.2".to_string(),
                port: 1080,
            }),
        );

        assert_eq!(credentials.imap.proxy.as_ref().unwrap().host, "10.0.0.2");
        assert_eq!(credentials.smtp.proxy.as_ref().unwrap().port, 1080);
        assert_eq!(credentials.proxy_mode, AccountProxyMode::Custom);

        set_account_proxy_on_credentials(&mut credentials, None);

        assert!(credentials.imap.proxy.is_none());
        assert!(credentials.smtp.proxy.is_none());
        assert_eq!(credentials.proxy_mode, AccountProxyMode::Inherit);
    }
}
