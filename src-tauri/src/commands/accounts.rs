use crate::account_colors::default_account_color;
use crate::commands::encrypted_store::{load_account_auth_data, store_account_auth_data};
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
use pebble_mail::{
    ConnectionSecurity, ImapConfig, Pop3Config, Pop3Provider, ProxyConfig, SmtpConfig,
};
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
    /// Per-account opt-in to permit unencrypted (plaintext) connections to a
    /// non-localhost server (issue #70). Off by default; enabling it lets the
    /// user connect to legacy servers that only offer plaintext, accepting that
    /// credentials are sent in cleartext.
    allow_plaintext: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct StoredAccountCredentials {
    #[serde(default, skip_serializing_if = "is_inherit_proxy_mode")]
    proxy_mode: AccountProxyMode,
    imap: StoredMailConfig,
    smtp: StoredMailConfig,
    #[serde(default, skip_serializing_if = "is_false")]
    allow_plaintext: bool,
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
    #[serde(default, skip_serializing_if = "is_false")]
    accept_invalid_certs: bool,
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
            accept_invalid_certs: self.accept_invalid_certs,
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
            accept_invalid_certs: self.accept_invalid_certs,
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
            accept_invalid_certs: value.accept_invalid_certs,
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
            accept_invalid_certs: value.accept_invalid_certs,
            proxy: value.proxy.clone(),
        }
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

impl From<&AccountCredentials> for StoredAccountCredentials {
    fn from(value: &AccountCredentials) -> Self {
        Self {
            proxy_mode: value.proxy_mode,
            imap: StoredMailConfig::from(&value.imap),
            smtp: StoredMailConfig::from(&value.smtp),
            allow_plaintext: value.allow_plaintext,
        }
    }
}

impl From<StoredAccountCredentials> for AccountCredentials {
    fn from(value: StoredAccountCredentials) -> Self {
        Self {
            proxy_mode: value.proxy_mode,
            imap: value.imap.into_imap(),
            smtp: value.smtp.into_smtp(),
            allow_plaintext: value.allow_plaintext,
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
    allow_plaintext: bool,
) -> std::result::Result<(), PebbleError> {
    if matches!(security, ConnectionSecurity::Plain)
        && !is_loopback_mail_host(host)
        && !allow_plaintext
    {
        return Err(PebbleError::Validation(format!(
            "{label} plaintext connections are disabled by default. Enable \"Allow \
             unencrypted connection\" for this account to connect to a server that only \
             supports plaintext (your password will be sent unencrypted)."
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
    #[serde(default)]
    pub account_label: Option<String>,
    #[serde(default)]
    pub sync_range_days: Option<u32>,
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
    pub accept_invalid_certs: bool,
    #[serde(default)]
    pub allow_plaintext: bool,
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

/// Resolve the login username for IMAP/SMTP, defaulting to the account email
/// when left blank (the common case where the mail login equals the address).
/// Applied on both account creation and update so the two paths stay consistent.
fn resolve_username(username: &str, email: &str) -> String {
    if username.is_empty() {
        email.to_string()
    } else {
        username.to_string()
    }
}

#[tauri::command]
pub async fn add_account(
    state: State<'_, AppState>,
    request: AddAccountRequest,
) -> std::result::Result<Account, PebbleError> {
    pebble_store::accounts::validate_sync_range_days(request.sync_range_days)?;
    let now = now_timestamp();
    pebble_mail::sender::sender_mailbox(&pebble_core::EmailAddress {
        name: Some(request.display_name.clone()),
        address: request.email.clone(),
    })?;
    let provider = match request.provider.to_lowercase().as_str() {
        "gmail" => ProviderType::Gmail,
        "outlook" => ProviderType::Outlook,
        "pop3" => ProviderType::Pop3,
        _ => ProviderType::Imap,
    };
    if provider == ProviderType::Pop3 && request.sync_range_days.is_some() {
        return Err(PebbleError::Validation(
            "POP3 does not support a bounded sync range".into(),
        ));
    }

    let incoming_label = if provider == ProviderType::Pop3 {
        "POP3"
    } else {
        "IMAP"
    };
    validate_connection_security(
        incoming_label,
        &request.imap_host,
        &request.imap_security,
        request.allow_plaintext,
    )?;
    validate_connection_security(
        "SMTP",
        &request.smtp_host,
        &request.smtp_security,
        request.allow_plaintext,
    )?;

    let existing_accounts = state.store.list_accounts()?;
    let account = Account {
        account_label: pebble_store::accounts::normalize_account_label(
            request.account_label.as_deref(),
        )?,
        provider_display_name: None,
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

        // Login username defaults to the email address when left blank.
        let username = resolve_username(&request.username, &request.email);

        // Build typed IMAP + SMTP credentials
        let credentials = AccountCredentials {
            proxy_mode,
            imap: ImapConfig {
                host: request.imap_host,
                port: request.imap_port,
                username: username.clone(),
                password: request.password.clone(),
                security: request.imap_security,
                accept_invalid_certs: request.accept_invalid_certs,
                proxy: proxy.clone(),
            },
            smtp: SmtpConfig {
                host: request.smtp_host,
                port: request.smtp_port,
                username,
                password: request.password,
                security: request.smtp_security,
                accept_invalid_certs: request.accept_invalid_certs,
                proxy,
            },
            allow_plaintext: request.allow_plaintext,
        };

        // Encrypt credentials and store as auth_data
        let config_bytes = serialize_account_credentials(&credentials)?;
        store_account_auth_data(&state.crypto, &state.store, &account.id, &config_bytes)?;

        // Store non-secret metadata in sync_state
        let provider_slug = match &provider {
            ProviderType::Gmail => "gmail",
            ProviderType::Outlook => "outlook",
            ProviderType::Pop3 => "pop3",
            ProviderType::Imap => "imap",
        };
        state.store.update_sync_state(&account.id, |s| {
            s.provider = Some(provider_slug.to_string());
            s.sync_range_days = request.sync_range_days;
            s.sync_since = pebble_store::accounts::sync_range_cutoff(request.sync_range_days, now);
            // New IMAP accounts start with Inbox only. Users can opt into
            // additional mailboxes from the account editor after the account
            // exists and its saved credentials can be used for IMAP LIST.
            if provider == ProviderType::Imap {
                s.selected_imap_folder_remote_ids = Some(Vec::new());
            }
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
    accept_invalid_certs: Option<bool>,
    proxy_host: Option<String>,
    proxy_port: Option<u16>,
    account_color: Option<String>,
    account_label: Option<String>,
) -> std::result::Result<(), PebbleError> {
    validate_account_color(account_color.as_deref())?;
    pebble_mail::sender::sender_mailbox(&pebble_core::EmailAddress {
        name: Some(display_name.clone()),
        address: email.clone(),
    })?;
    pebble_store::accounts::normalize_account_label(account_label.as_deref())?;

    let credentials_dirty = password.is_some()
        || imap_host.is_some()
        || smtp_host.is_some()
        || imap_port.is_some()
        || smtp_port.is_some()
        || imap_security.is_some()
        || smtp_security.is_some()
        || accept_invalid_certs.is_some()
        || proxy_host.is_some()
        || proxy_port.is_some();
    if !credentials_dirty {
        state.store.update_account_details(
            &account_id,
            &email,
            &display_name,
            account_color.as_deref(),
            account_label.as_deref(),
        )?;
        return Ok(());
    }

    let provider = state
        .store
        .get_account(&account_id)?
        .map(|account| account.provider)
        .ok_or_else(|| PebbleError::Internal(format!("Account not found: {account_id}")))?;

    // Parse the existing encrypted blob into a typed view. If the row is
    // missing (first-time edit, or a legacy OAuth-only account moving to
    // IMAP), seed a blank template that the mutations below can fill in.
    let mut creds: AccountCredentials =
        match load_account_auth_data(&state.crypto, &state.store, &account_id)? {
            Some(decrypted) => deserialize_account_credentials(&decrypted)?,
            None => AccountCredentials {
                proxy_mode: AccountProxyMode::Inherit,
                imap: ImapConfig {
                    host: String::new(),
                    port: 0,
                    username: String::new(),
                    password: String::new(),
                    security: ConnectionSecurity::default(),
                    accept_invalid_certs: false,
                    proxy: None,
                },
                smtp: SmtpConfig {
                    host: String::new(),
                    port: 0,
                    username: String::new(),
                    password: String::new(),
                    security: ConnectionSecurity::default(),
                    accept_invalid_certs: false,
                    proxy: None,
                },
                allow_plaintext: false,
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

    // Incoming side. POP3 accounts reuse the stored IMAP-shaped credential
    // object for compatibility with existing encrypted account data.
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
    if let Some(accept_invalid_certs) = accept_invalid_certs {
        creds.imap.accept_invalid_certs = accept_invalid_certs;
    }
    if let Some(proxy) = &updated_proxy {
        creds.proxy_mode = if proxy.is_some() {
            AccountProxyMode::Custom
        } else {
            AccountProxyMode::Inherit
        };
        creds.imap.proxy = proxy.clone();
    }
    creds.imap.username = resolve_username(&creds.imap.username, &email);

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
    if let Some(accept_invalid_certs) = accept_invalid_certs {
        creds.smtp.accept_invalid_certs = accept_invalid_certs;
    }
    // Mirror incoming proxy to SMTP; both connections share the same network path.
    if let Some(proxy) = updated_proxy {
        creds.smtp.proxy = proxy;
    }
    creds.smtp.username = resolve_username(&creds.smtp.username, &email);

    // `allow_plaintext` is set only at account creation; it is preserved here by
    // the deserialize→serialize round-trip of the stored credentials, so an
    // existing plaintext account still validates on edit.

    let incoming_label = if provider == ProviderType::Pop3 {
        "POP3"
    } else {
        "IMAP"
    };
    validate_connection_security(
        incoming_label,
        &creds.imap.host,
        &creds.imap.security,
        creds.allow_plaintext,
    )?;
    validate_connection_security(
        "SMTP",
        &creds.smtp.host,
        &creds.smtp.security,
        creds.allow_plaintext,
    )?;

    state.store.update_account_details(
        &account_id,
        &email,
        &display_name,
        account_color.as_deref(),
        account_label.as_deref(),
    )?;

    let config_bytes = serialize_account_credentials(&creds)?;
    store_account_auth_data(&state.crypto, &state.store, &account_id, &config_bytes)?;

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
    if !matches!(account.provider, ProviderType::Imap | ProviderType::Pop3) {
        return Err(PebbleError::UnsupportedProvider(
            "Use the OAuth account proxy commands for Gmail and Outlook accounts".to_string(),
        ));
    }

    let Some(decrypted) = load_account_auth_data(&state.crypto, &state.store, &account_id)? else {
        return Ok(AccountProxySetting {
            mode: AccountProxyMode::Inherit,
            proxy: None,
        });
    };
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
    if !matches!(account.provider, ProviderType::Imap | ProviderType::Pop3) {
        return Err(PebbleError::UnsupportedProvider(
            "Use the OAuth account proxy commands for Gmail and Outlook accounts".to_string(),
        ));
    }

    let setting = account_proxy_setting_from_parts(mode, proxy_host, proxy_port, "Account proxy")?;
    let Some(decrypted) = load_account_auth_data(&state.crypto, &state.store, &account_id)? else {
        return Err(PebbleError::Internal(format!(
            "No auth data found for account {account_id}"
        )));
    };
    let mut credentials = deserialize_account_credentials(&decrypted)?;
    set_account_proxy_setting_on_credentials(&mut credentials, setting);
    let config_bytes = serialize_account_credentials(&credentials)?;
    store_account_auth_data(&state.crypto, &state.store, &account_id, &config_bytes)?;
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct TestConnectionRequest {
    pub imap_host: String,
    pub imap_port: u16,
    pub imap_security: ConnectionSecurity,
    #[serde(default)]
    pub accept_invalid_certs: bool,
    #[serde(default)]
    pub proxy_host: Option<String>,
    #[serde(default)]
    pub proxy_port: Option<u16>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub allow_plaintext: bool,
}

/// Resolve the effective IMAP test login name and whether the connection test
/// should attempt an authenticated LOGIN.
///
/// The login name falls back to the account email when the username field is
/// blank — the same rule `add_account` applies when persisting credentials. A
/// blank username therefore still produces a real LOGIN test (issue #60),
/// instead of silently downgrading to a connectivity-only check and reporting a
/// misleading "passed" while the actual credentials are never verified.
fn resolve_imap_test_login(username: &str, email: &str, password: &str) -> (String, bool) {
    let resolved = resolve_username(username, email);
    let should_login = !resolved.is_empty() && !password.is_empty();
    (resolved, should_login)
}

#[derive(Debug, Deserialize)]
pub struct TestPop3ConnectionRequest {
    pub pop3_host: String,
    pub pop3_port: u16,
    pub pop3_security: ConnectionSecurity,
    #[serde(default)]
    pub accept_invalid_certs: bool,
    #[serde(default)]
    pub proxy_host: Option<String>,
    #[serde(default)]
    pub proxy_port: Option<u16>,
    #[serde(default)]
    pub username: Option<String>,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub allow_plaintext: bool,
}

#[tauri::command]
pub async fn test_imap_connection(
    state: State<'_, AppState>,
    request: TestConnectionRequest,
) -> std::result::Result<String, PebbleError> {
    validate_connection_security(
        "IMAP",
        &request.imap_host,
        &request.imap_security,
        request.allow_plaintext,
    )?;

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
    let password = request.password.unwrap_or_default();
    let (username, has_credentials) = resolve_imap_test_login(
        request.username.as_deref().unwrap_or_default(),
        request.email.as_deref().unwrap_or_default(),
        &password,
    );
    let config = pebble_mail::ImapConfig {
        host: request.imap_host,
        port: request.imap_port,
        username,
        password,
        security: request.imap_security,
        accept_invalid_certs: request.accept_invalid_certs,
        proxy,
    };
    if has_credentials {
        pebble_mail::ImapProvider::test_connection_with_login(&config).await
    } else {
        pebble_mail::ImapProvider::test_connection(&config).await
    }
}

#[tauri::command]
pub async fn test_pop3_connection(
    state: State<'_, AppState>,
    request: TestPop3ConnectionRequest,
) -> std::result::Result<String, PebbleError> {
    validate_connection_security(
        "POP3",
        &request.pop3_host,
        &request.pop3_security,
        request.allow_plaintext,
    )?;

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
    let username = request.username.unwrap_or_default();
    let password = request.password.unwrap_or_default();
    if username.is_empty() || password.is_empty() {
        return Err(PebbleError::Auth(
            "POP3 username and password are required for connection test".to_string(),
        ));
    }
    let config = Pop3Config {
        host: request.pop3_host,
        port: request.pop3_port,
        username,
        password,
        security: request.pop3_security,
        accept_invalid_certs: request.accept_invalid_certs,
        proxy,
    };
    Pop3Provider::test_connection(&config).await
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

    if matches!(account.provider, ProviderType::Pop3) {
        let decrypted = load_account_auth_data(&state.crypto, &state.store, &account_id)?
            .ok_or_else(|| PebbleError::Internal("No auth data found".into()))?;
        let credentials = deserialize_account_credentials(&decrypted)?;
        let proxy = resolve_mail_proxy_from_mode(
            &state.crypto,
            &state.store,
            credentials.proxy_mode,
            credentials.imap.proxy.clone(),
        )?;
        let config = Pop3Config {
            host: credentials.imap.host,
            port: credentials.imap.port,
            username: credentials.imap.username,
            password: credentials.imap.password,
            security: credentials.imap.security,
            accept_invalid_certs: credentials.imap.accept_invalid_certs,
            proxy,
        };
        return Pop3Provider::test_connection(&config).await;
    }

    let decrypted = load_account_auth_data(&state.crypto, &state.store, &account_id)?
        .ok_or_else(|| PebbleError::Internal("No auth data found".into()))?;
    let mut credentials = deserialize_account_credentials(&decrypted)?;
    credentials.imap.proxy = resolve_mail_proxy_from_mode(
        &state.crypto,
        &state.store,
        credentials.proxy_mode,
        credentials.imap.proxy.clone(),
    )?;
    pebble_mail::ImapProvider::test_connection_with_login(&credentials.imap).await
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
    fn resolve_username_defaults_empty_to_email() {
        assert_eq!(resolve_username("", "user@example.com"), "user@example.com");
    }

    #[test]
    fn resolve_username_keeps_non_empty() {
        assert_eq!(
            resolve_username("legacy-login", "user@example.com"),
            "legacy-login"
        );
    }

    #[test]
    fn imap_test_login_uses_email_when_username_blank() {
        // Issue #60: Outlook/hotmail users leave the username blank; the test
        // must still authenticate using the email as the login name rather than
        // downgrade to a connectivity-only check.
        let (username, should_login) =
            resolve_imap_test_login("", "user@hotmail.com", "app-password");
        assert_eq!(username, "user@hotmail.com");
        assert!(should_login);
    }

    #[test]
    fn imap_test_login_keeps_explicit_username() {
        let (username, should_login) =
            resolve_imap_test_login("custom-login", "user@example.com", "pw");
        assert_eq!(username, "custom-login");
        assert!(should_login);
    }

    #[test]
    fn imap_test_login_skips_login_without_password() {
        let (username, should_login) = resolve_imap_test_login("", "user@example.com", "");
        assert_eq!(username, "user@example.com");
        assert!(!should_login);
    }

    #[test]
    fn imap_test_login_skips_login_without_username_or_email() {
        // No email yet (e.g. connectivity probe before filling the form): there
        // is no login name to try, so fall back to a connectivity-only check.
        let (username, should_login) = resolve_imap_test_login("", "", "app-password");
        assert!(username.is_empty());
        assert!(!should_login);
    }

    #[test]
    fn rejects_plaintext_security_for_remote_hosts() {
        let err = validate_connection_security(
            "IMAP",
            "mail.example.com",
            &ConnectionSecurity::Plain,
            false,
        )
        .expect_err("remote plaintext mail connections must be rejected");

        assert!(matches!(err, PebbleError::Validation(_)));
    }

    #[test]
    fn allows_plaintext_security_for_localhost() {
        validate_connection_security("IMAP", "localhost", &ConnectionSecurity::Plain, false)
            .expect("localhost plaintext is useful for local test servers");
        validate_connection_security("SMTP", "127.0.0.1", &ConnectionSecurity::Plain, false)
            .expect("loopback plaintext is useful for local test servers");
    }

    #[test]
    fn allows_plaintext_for_remote_host_when_opted_in() {
        // Issue #70: some providers (e.g. Perfect World mail) only offer
        // unencrypted connections. With the per-account opt-in enabled, a remote
        // plaintext connection must be permitted instead of hard-rejected.
        validate_connection_security("IMAP", "mail.88.com", &ConnectionSecurity::Plain, true)
            .expect("remote plaintext must be allowed when the account opts in");
        validate_connection_security("SMTP", "mail.88.com", &ConnectionSecurity::Plain, true)
            .expect("remote plaintext must be allowed when the account opts in");
    }

    #[test]
    fn opt_in_does_not_affect_encrypted_security() {
        // The opt-in only relaxes plaintext; TLS/STARTTLS are always fine and
        // are unaffected by the flag either way.
        validate_connection_security("IMAP", "mail.88.com", &ConnectionSecurity::Tls, false)
            .expect("TLS is always allowed");
        validate_connection_security("IMAP", "mail.88.com", &ConnectionSecurity::StartTls, false)
            .expect("STARTTLS is always allowed");
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
                accept_invalid_certs: true,
                proxy: None,
            },
            smtp: SmtpConfig {
                host: "smtp.example.com".to_string(),
                port: 465,
                username: "user@example.com".to_string(),
                password: "smtp-secret".to_string(),
                security: ConnectionSecurity::Tls,
                accept_invalid_certs: true,
                proxy: None,
            },
            allow_plaintext: true,
        };

        let bytes = serialize_account_credentials(&credentials).unwrap();
        let decoded: AccountCredentials = serde_json::from_slice(&bytes).unwrap();

        assert_eq!(decoded.imap.password, "imap-secret");
        assert_eq!(decoded.smtp.password, "smtp-secret");
        assert!(decoded.imap.accept_invalid_certs);
        assert!(decoded.smtp.accept_invalid_certs);
        assert!(decoded.allow_plaintext);
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
                accept_invalid_certs: false,
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
                accept_invalid_certs: false,
                proxy: None,
            },
            allow_plaintext: false,
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
                accept_invalid_certs: false,
                proxy: None,
            },
            smtp: SmtpConfig {
                host: "smtp.example.com".to_string(),
                port: 465,
                username: "user@example.com".to_string(),
                password: "smtp-secret".to_string(),
                security: ConnectionSecurity::Tls,
                accept_invalid_certs: false,
                proxy: None,
            },
            allow_plaintext: false,
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
