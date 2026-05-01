use super::network::{
    account_proxy_setting_from_parts, get_global_proxy_raw, normalize_account_proxy_setting,
    proxy_config_from_parts, resolve_effective_proxy, resolve_effective_proxy_setting,
    AccountProxyMode, AccountProxySetting,
};
use crate::account_colors::default_account_color;
use crate::state::AppState;
use pebble_core::{
    new_id, now_timestamp, Account, HttpProxyConfig, OAuthTokens, PebbleError, ProviderType,
};
use pebble_crypto::CryptoService;
use pebble_mail::gmail_sync::TokenRefresher;
use pebble_oauth::{build_http_client, OAuthConfig, OAuthError, OAuthManager, OAuthNetworkConfig};
use pebble_store::Store;
use std::sync::Arc;
use tauri::State;
use tracing::debug;

fn constant_time_eq(left: &str, right: &str) -> bool {
    if left.len() != right.len() {
        return false;
    }

    let mut diff = 0u8;
    for (a, b) in left.as_bytes().iter().zip(right.as_bytes()) {
        diff |= a ^ b;
    }
    diff == 0
}

/// Fetch the user's email and display name from the OAuth provider's userinfo endpoint.
async fn fetch_userinfo(
    provider: &str,
    access_token: &str,
    network: &OAuthNetworkConfig,
) -> Result<(String, String), PebbleError> {
    let url = match provider.to_lowercase().as_str() {
        "gmail" => "https://www.googleapis.com/oauth2/v2/userinfo",
        "outlook" => "https://graph.microsoft.com/v1.0/me",
        _ => return Err(PebbleError::UnsupportedProvider(provider.to_string())),
    };

    let client = build_http_client(network)
        .map_err(|e| PebbleError::Network(format!("Userinfo HTTP client failed: {e}")))?;
    let resp: serde_json::Value = client
        .get(url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| PebbleError::Network(format!("Userinfo request failed: {e}")))?
        .json()
        .await
        .map_err(|e| PebbleError::Network(format!("Userinfo parse failed: {e}")))?;

    let email = resp["email"]
        .as_str()
        .or_else(|| resp["mail"].as_str())
        .or_else(|| resp["userPrincipalName"].as_str())
        .unwrap_or("")
        .to_string();

    let name = resp["name"]
        .as_str()
        .or_else(|| resp["displayName"].as_str())
        .unwrap_or("")
        .to_string();

    debug!("Fetched userinfo from OAuth provider");
    Ok((email, name))
}

fn oauth_proxy_from_parts(
    proxy_host: Option<String>,
    proxy_port: Option<u16>,
) -> Result<Option<HttpProxyConfig>, PebbleError> {
    proxy_config_from_parts(proxy_host, proxy_port, "OAuth proxy")
}

/// OAuth config for Gmail (Google).
pub(crate) fn gmail_oauth_config() -> OAuthConfig {
    OAuthConfig {
        client_id: oauth_config_value(
            "GOOGLE_CLIENT_ID",
            option_env!("GOOGLE_CLIENT_ID"),
            "GOOGLE_CLIENT_ID_PLACEHOLDER",
        ),
        client_secret: oauth_config_optional_value(
            "GOOGLE_CLIENT_SECRET",
            option_env!("GOOGLE_CLIENT_SECRET"),
        ),
        auth_url: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
        token_url: "https://oauth2.googleapis.com/token".to_string(),
        scopes: vec![
            "https://mail.google.com/".to_string(),
            "https://www.googleapis.com/auth/userinfo.email".to_string(),
            "https://www.googleapis.com/auth/userinfo.profile".to_string(),
        ],
        redirect_port: 0,
    }
}

/// OAuth config for Outlook (Microsoft).
pub(crate) fn outlook_oauth_config() -> OAuthConfig {
    OAuthConfig {
        client_id: oauth_config_value(
            "MICROSOFT_CLIENT_ID",
            option_env!("MICROSOFT_CLIENT_ID"),
            "MICROSOFT_CLIENT_ID_PLACEHOLDER",
        ),
        client_secret: oauth_config_optional_value(
            "MICROSOFT_CLIENT_SECRET",
            option_env!("MICROSOFT_CLIENT_SECRET"),
        ),
        auth_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize".to_string(),
        token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token".to_string(),
        scopes: vec![
            "https://graph.microsoft.com/Mail.ReadWrite".to_string(),
            "https://graph.microsoft.com/Mail.Send".to_string(),
            "https://graph.microsoft.com/User.Read".to_string(),
            "offline_access".to_string(),
        ],
        redirect_port: 0,
    }
}

fn dotenv_lookup_from_str(contents: &str, key: &str) -> Option<String> {
    for raw_line in contents.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((name, value)) = line.split_once('=') else {
            continue;
        };
        if name.trim() != key {
            continue;
        }

        let value = value.trim();
        let unquoted = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')))
            .unwrap_or(value);
        return Some(unquoted.to_string());
    }
    None
}

fn dotenv_contents() -> Option<String> {
    let mut candidates = Vec::new();
    if let Ok(current_dir) = std::env::current_dir() {
        candidates.push(current_dir.join(".env"));
        candidates.push(current_dir.join("..").join(".env"));
    }
    candidates.push(std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".env"));
    candidates.push(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join(".env"),
    );

    candidates
        .into_iter()
        .find_map(|path| std::fs::read_to_string(path).ok())
}

fn oauth_config_value_from_sources(
    key: &str,
    env_value: Option<&str>,
    dotenv_contents: Option<&str>,
    compile_value: Option<&str>,
    placeholder: &str,
) -> String {
    env_value
        .filter(|value| !is_placeholder(value))
        .map(ToOwned::to_owned)
        .or_else(|| {
            dotenv_contents
                .and_then(|contents| dotenv_lookup_from_str(contents, key))
                .filter(|value| !is_placeholder(value))
        })
        .or_else(|| {
            compile_value
                .filter(|value| !is_placeholder(value))
                .map(ToOwned::to_owned)
        })
        .unwrap_or_else(|| placeholder.to_string())
}

fn oauth_config_value(key: &str, compile_value: Option<&str>, placeholder: &str) -> String {
    let env_value = std::env::var(key).ok();
    let dotenv = dotenv_contents();
    oauth_config_value_from_sources(
        key,
        env_value.as_deref(),
        dotenv.as_deref(),
        compile_value,
        placeholder,
    )
}

fn oauth_config_optional_value(key: &str, compile_value: Option<&str>) -> Option<String> {
    let value = oauth_config_value(key, compile_value, "");
    if is_placeholder(&value) {
        None
    } else {
        Some(value)
    }
}

fn token_exchange_error_message(provider: &str, error: &OAuthError) -> String {
    let detail = match error {
        OAuthError::TokenExchange(message) => message.as_str(),
        _ => return format!("Token exchange failed: {error}"),
    };

    if provider.eq_ignore_ascii_case("outlook")
        && detail
            .to_ascii_lowercase()
            .contains("client_secret is missing")
    {
        return "Token exchange failed: Microsoft rejected this app registration as a confidential client. Configure the Azure app registration as a public/native client with a localhost redirect URI, or set MICROSOFT_CLIENT_SECRET in .env and restart Pebble.".to_string();
    }

    if provider.eq_ignore_ascii_case("gmail")
        && detail
            .to_ascii_lowercase()
            .contains("client_secret is missing")
    {
        return "Token exchange failed: Google rejected this OAuth client because it requires a client secret. Set GOOGLE_CLIENT_SECRET in .env from the Google OAuth Desktop app credentials, then restart Pebble.".to_string();
    }

    format!("Token exchange failed: {detail}")
}

fn is_placeholder(value: &str) -> bool {
    let v = value.trim();
    v.is_empty()
        || v.eq_ignore_ascii_case("YOUR_CLIENT_ID")
        || v.eq_ignore_ascii_case("YOUR_CLIENT_SECRET")
        || v.ends_with("_PLACEHOLDER")
}

fn validate_oauth_config(config: &OAuthConfig, provider: &str) -> Result<(), PebbleError> {
    if is_placeholder(&config.client_id) {
        return Err(PebbleError::Internal(format!(
            "OAuth client_id for '{provider}' is not configured. \
             Set the appropriate environment variable before starting the OAuth flow."
        )));
    }
    if let Some(secret) = &config.client_secret {
        if is_placeholder(secret) {
            return Err(PebbleError::Internal(format!(
                "OAuth client_secret for '{provider}' is not configured. \
                 Set the appropriate environment variable before starting the OAuth flow."
            )));
        }
    }
    Ok(())
}

/// Resolve an `OAuthConfig` from a provider name, or return an error.
pub(crate) fn config_for_provider(provider: &str) -> Result<OAuthConfig, PebbleError> {
    let config = match provider.to_lowercase().as_str() {
        "gmail" => gmail_oauth_config(),
        "outlook" => outlook_oauth_config(),
        _ => {
            return Err(PebbleError::UnsupportedProvider(format!(
                "Unknown OAuth provider: {provider}"
            )))
        }
    };
    validate_oauth_config(&config, provider)?;
    Ok(config)
}

/// Resolve a `ProviderType` from a provider name.
fn provider_type(provider: &str) -> Result<ProviderType, PebbleError> {
    match provider.to_lowercase().as_str() {
        "gmail" => Ok(ProviderType::Gmail),
        "outlook" => Ok(ProviderType::Outlook),
        _ => Err(PebbleError::UnsupportedProvider(provider.to_string())),
    }
}

pub(crate) fn provider_slug(provider: &ProviderType) -> &'static str {
    match provider {
        ProviderType::Imap => "imap",
        ProviderType::Gmail => "gmail",
        ProviderType::Outlook => "outlook",
    }
}

fn ensure_oauth_account_provider(state: &AppState, account_id: &str) -> Result<(), PebbleError> {
    let account = state
        .store
        .get_account(account_id)?
        .ok_or_else(|| PebbleError::Internal(format!("Account not found: {account_id}")))?;
    if matches!(
        account.provider,
        ProviderType::Gmail | ProviderType::Outlook
    ) {
        Ok(())
    } else {
        Err(PebbleError::UnsupportedProvider(
            provider_slug(&account.provider).to_string(),
        ))
    }
}

fn persist_oauth_tokens(
    state: &AppState,
    account_id: &str,
    tokens: &OAuthTokens,
) -> Result<(), PebbleError> {
    persist_oauth_tokens_raw(&state.crypto, &state.store, account_id, tokens)
}

/// Encrypt and persist OAuth tokens without needing a full `AppState`.
/// Used inside async refresher closures where only `crypto` and `store` are
/// cloned in.
fn persist_oauth_tokens_raw(
    crypto: &CryptoService,
    store: &Store,
    account_id: &str,
    tokens: &OAuthTokens,
) -> Result<(), PebbleError> {
    let (proxy_mode, proxy) = read_stored_oauth_auth_data_raw(crypto, store, account_id)?
        .map(|stored| (stored.proxy_mode, stored.proxy))
        .unwrap_or((AccountProxyMode::Inherit, None));
    let stored =
        StoredOAuthAuthData::from_tokens_with_proxy_mode(tokens.clone(), proxy_mode, proxy);
    persist_stored_oauth_auth_data_raw(crypto, store, account_id, &stored)
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
struct StoredOAuthAuthData {
    access_token: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    refresh_token: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    expires_at: Option<i64>,
    #[serde(default)]
    scopes: Vec<String>,
    #[serde(default, skip_serializing_if = "super::network::is_inherit_proxy_mode")]
    proxy_mode: AccountProxyMode,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    proxy: Option<HttpProxyConfig>,
}

impl StoredOAuthAuthData {
    fn from_tokens(tokens: OAuthTokens, proxy: Option<HttpProxyConfig>) -> Self {
        let proxy_mode = if proxy.is_some() {
            AccountProxyMode::Custom
        } else {
            AccountProxyMode::Inherit
        };
        Self::from_tokens_with_proxy_mode(tokens, proxy_mode, proxy)
    }

    fn from_tokens_with_proxy_mode(
        tokens: OAuthTokens,
        proxy_mode: AccountProxyMode,
        proxy: Option<HttpProxyConfig>,
    ) -> Self {
        Self {
            access_token: tokens.access_token,
            refresh_token: tokens.refresh_token,
            expires_at: tokens.expires_at,
            scopes: tokens.scopes,
            proxy_mode,
            proxy,
        }
    }

    #[cfg(test)]
    fn with_tokens(mut self, tokens: OAuthTokens) -> Self {
        self.access_token = tokens.access_token;
        self.refresh_token = tokens.refresh_token;
        self.expires_at = tokens.expires_at;
        self.scopes = tokens.scopes;
        self
    }

    #[cfg(test)]
    fn with_proxy(mut self, proxy: Option<HttpProxyConfig>) -> Self {
        self.proxy_mode = if proxy.is_some() {
            AccountProxyMode::Custom
        } else {
            AccountProxyMode::Inherit
        };
        self.proxy = proxy;
        self
    }

    fn with_proxy_setting(mut self, setting: AccountProxySetting) -> Self {
        self.proxy_mode = setting.mode;
        self.proxy = setting.proxy;
        self
    }

    fn tokens(&self) -> OAuthTokens {
        OAuthTokens {
            access_token: self.access_token.clone(),
            refresh_token: self.refresh_token.clone(),
            expires_at: self.expires_at,
            scopes: self.scopes.clone(),
        }
    }
}

fn decode_stored_oauth_auth_data(bytes: &[u8]) -> Result<StoredOAuthAuthData, PebbleError> {
    serde_json::from_slice(bytes)
        .map_err(|e| PebbleError::Internal(format!("Failed to parse OAuth auth data: {e}")))
}

fn persist_stored_oauth_auth_data_raw(
    crypto: &CryptoService,
    store: &Store,
    account_id: &str,
    stored: &StoredOAuthAuthData,
) -> Result<(), PebbleError> {
    let config_bytes = serde_json::to_vec(stored)
        .map_err(|e| PebbleError::Internal(format!("Failed to serialize OAuth auth data: {e}")))?;
    let encrypted = crypto.encrypt(&config_bytes)?;
    store.set_auth_data(account_id, &encrypted)?;
    Ok(())
}

fn read_stored_oauth_auth_data_raw(
    crypto: &CryptoService,
    store: &Store,
    account_id: &str,
) -> Result<Option<StoredOAuthAuthData>, PebbleError> {
    let Some(encrypted) = store.get_auth_data(account_id)? else {
        return Ok(None);
    };
    let decrypted = crypto.decrypt(&encrypted)?;
    decode_stored_oauth_auth_data(&decrypted).map(Some)
}

fn effective_oauth_proxy(
    crypto: &CryptoService,
    store: &Store,
    stored: &StoredOAuthAuthData,
) -> Result<Option<HttpProxyConfig>, PebbleError> {
    let global_proxy = get_global_proxy_raw(crypto, store)?;
    Ok(resolve_effective_proxy_setting(
        stored.proxy_mode,
        stored.proxy.clone(),
        global_proxy,
    ))
}

/// Decoded view of an account's stored OAuth token blob.
pub(crate) struct DecodedOAuthTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
    pub proxy: Option<HttpProxyConfig>,
}

pub(crate) struct ResolvedOAuthAuth {
    pub tokens: OAuthTokens,
    pub proxy: Option<HttpProxyConfig>,
}

/// Read and decrypt an account's OAuth token blob into its components.
///
/// Replaces the hand-written decryption that used to be inlined inside each
/// provider branch of `start_sync` — keeping every OAuth-backed provider on
/// the same code path.
pub(crate) fn decode_oauth_account_tokens(
    state: &AppState,
    account_id: &str,
) -> Result<DecodedOAuthTokens, PebbleError> {
    let stored = read_stored_oauth_auth_data_raw(&state.crypto, &state.store, account_id)?
        .ok_or_else(|| PebbleError::Internal(format!("No auth data for account {account_id}")))?;
    let proxy = effective_oauth_proxy(&state.crypto, &state.store, &stored)?;
    Ok(DecodedOAuthTokens {
        access_token: stored.access_token,
        refresh_token: stored.refresh_token,
        expires_at: stored.expires_at,
        proxy,
    })
}

/// Build a [`TokenRefresher`] closure for a provider that can refresh its
/// access token via the shared `OAuthManager`.
///
/// If `refresh_token` is `None` the returned closure simply returns the last
/// known access token (used for accounts imported without a refresh token).
/// Otherwise it runs a full refresh + persist cycle on every call so the
/// encrypted auth blob stays in sync with the live token.
pub(crate) fn build_oauth_token_refresher(
    oauth_config: OAuthConfig,
    refresh_token: Option<String>,
    fallback_access_token: String,
    crypto: Arc<CryptoService>,
    store: Arc<Store>,
    account_id: String,
) -> TokenRefresher {
    match refresh_token {
        Some(initial_rt) => {
            Box::new(move || {
                let config = oauth_config.clone();
                let crypto = Arc::clone(&crypto);
                let store = Arc::clone(&store);
                let account_id = account_id.clone();
                let initial_rt = initial_rt.clone();
                Box::pin(async move {
                    // Read the latest refresh token from the encrypted store.
                    // OAuth providers (especially Microsoft) may rotate refresh tokens
                    // on each use, so the initially captured token may be stale.
                    let (rt, network) = match store.get_auth_data(&account_id)? {
                        Some(encrypted) => {
                            let decrypted = crypto.decrypt(&encrypted)?;
                            let stored = decode_stored_oauth_auth_data(&decrypted)?;
                            let effective_proxy = effective_oauth_proxy(&crypto, &store, &stored)?;
                            (
                                stored.refresh_token.clone().unwrap_or(initial_rt),
                                OAuthNetworkConfig {
                                    proxy: effective_proxy,
                                },
                            )
                        }
                        None => {
                            let effective_proxy = get_global_proxy_raw(&crypto, &store)?;
                            (
                                initial_rt,
                                OAuthNetworkConfig {
                                    proxy: effective_proxy,
                                },
                            )
                        }
                    };

                    let manager = OAuthManager::new_with_network(config, network);
                    let token_pair = manager
                        .refresh_token(&rt)
                        .await
                        .map_err(|e| PebbleError::OAuth(format!("Token refresh failed: {e}")))?;
                    let tokens = OAuthTokens {
                        access_token: token_pair.access_token.clone(),
                        refresh_token: token_pair.refresh_token.clone().or(Some(rt)),
                        expires_at: token_pair.expires_at,
                        scopes: token_pair.scopes.clone(),
                    };
                    persist_oauth_tokens_raw(&crypto, &store, &account_id, &tokens)?;
                    Ok((token_pair.access_token, token_pair.expires_at))
                })
            })
        }
        None => Box::new(move || {
            let token = fallback_access_token.clone();
            Box::pin(async move { Ok((token, None)) })
        }),
    }
}

pub(crate) async fn ensure_account_oauth_auth(
    state: &AppState,
    account_id: &str,
    provider: &str,
) -> Result<ResolvedOAuthAuth, PebbleError> {
    let stored = read_stored_oauth_auth_data_raw(&state.crypto, &state.store, account_id)?
        .ok_or_else(|| {
            PebbleError::Internal(format!("No auth data found for account {account_id}"))
        })?;
    let proxy = effective_oauth_proxy(&state.crypto, &state.store, &stored)?;
    let network = OAuthNetworkConfig {
        proxy: proxy.clone(),
    };
    let mut tokens = stored.tokens();

    let needs_refresh = tokens.refresh_token.is_some()
        && tokens
            .expires_at
            .map(|exp| exp - now_timestamp() < 300)
            .unwrap_or(false);

    if needs_refresh {
        let refresh_token = tokens.refresh_token.clone().unwrap_or_default();
        let manager = OAuthManager::new_with_network(config_for_provider(provider)?, network);
        let token_pair = manager
            .refresh_token(&refresh_token)
            .await
            .map_err(|e| PebbleError::OAuth(format!("Token refresh failed: {e}")))?;

        tokens = OAuthTokens {
            access_token: token_pair.access_token,
            refresh_token: token_pair.refresh_token.or(Some(refresh_token)),
            expires_at: token_pair.expires_at,
            scopes: token_pair.scopes,
        };
        persist_oauth_tokens(state, account_id, &tokens)?;
    }

    Ok(ResolvedOAuthAuth { tokens, proxy })
}

/// Complete the OAuth flow end-to-end.
///
/// Starts a redirect listener, waits for the browser callback, exchanges the
/// authorization code for tokens, encrypts and stores the tokens, and creates
/// the account record.
#[tauri::command]
pub async fn complete_oauth_flow(
    state: State<'_, AppState>,
    provider: String,
    email: String,
    display_name: String,
    proxy_host: Option<String>,
    proxy_port: Option<u16>,
) -> std::result::Result<Account, PebbleError> {
    let mut config = config_for_provider(&provider)?;
    let account_proxy = oauth_proxy_from_parts(proxy_host, proxy_port)?;
    let effective_proxy = resolve_effective_proxy(
        account_proxy.clone(),
        super::network::get_global_proxy_raw(&state.crypto, &state.store)?,
    );
    let network = OAuthNetworkConfig {
        proxy: effective_proxy,
    };

    // Bind the redirect listener first so the OS assigns an available port.
    // The actual port is then used in the redirect URI sent to the provider.
    let bound = pebble_oauth::redirect::bind_redirect_listener(config.redirect_port)
        .await
        .map_err(|e| PebbleError::OAuth(format!("Failed to bind redirect listener: {e}")))?;
    config.redirect_port = bound.port;

    let manager = OAuthManager::new_with_network(config, network.clone());

    // Start auth flow (generates PKCE challenge)
    let (auth_url, pkce_state) = manager
        .start_auth()
        .await
        .map_err(|e| PebbleError::OAuth(format!("Failed to start OAuth flow: {e}")))?;

    // Open the authorization URL in the system browser
    opener::open(&auth_url)
        .map_err(|e| PebbleError::OAuth(format!("Failed to open browser: {e}")))?;

    // Wait for the redirect callback with a 5-minute timeout
    let redirect = bound
        .wait()
        .await
        .map_err(|e| PebbleError::OAuth(format!("OAuth redirect failed: {e}")))?;

    if !constant_time_eq(&redirect.state, pkce_state.csrf_token.secret()) {
        return Err(PebbleError::OAuth("OAuth state mismatch".to_string()));
    }

    // Exchange code for tokens
    let token_pair = manager
        .complete_auth(&redirect.code, pkce_state)
        .await
        .map_err(|e| PebbleError::OAuth(token_exchange_error_message(&provider, &e)))?;

    // Fetch user info from Google/Microsoft to get actual email and display name
    let (real_email, real_name) = fetch_userinfo(&provider, &token_pair.access_token, &network)
        .await
        .unwrap_or_else(|_| (email.clone(), display_name.clone()));

    let final_email = if real_email.is_empty() {
        email
    } else {
        real_email
    };
    let final_name = if real_name.is_empty() {
        display_name
    } else {
        real_name
    };

    // Create the account
    let now = now_timestamp();
    let existing_accounts = state.store.list_accounts()?;
    let account_color = Some(default_account_color(&existing_accounts, &final_email));
    let account = Account {
        id: new_id(),
        email: final_email,
        display_name: final_name,
        color: account_color,
        provider: provider_type(&provider)?,
        created_at: now,
        updated_at: now,
    };

    state.store.insert_account(&account)?;

    // If any subsequent step fails, delete the account row to prevent half-creation
    if let Err(e) = (|| -> std::result::Result<(), PebbleError> {
        // Encrypt tokens and store as auth_data
        let tokens = OAuthTokens {
            access_token: token_pair.access_token,
            refresh_token: token_pair.refresh_token,
            expires_at: token_pair.expires_at,
            scopes: token_pair.scopes,
        };
        let stored = StoredOAuthAuthData::from_tokens(tokens, account_proxy);
        persist_stored_oauth_auth_data_raw(&state.crypto, &state.store, &account.id, &stored)?;

        // Store provider metadata in sync_state
        let slug = provider_slug(&account.provider).to_string();
        state.store.update_sync_state(&account.id, |s| {
            s.provider = Some(slug);
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
pub async fn get_oauth_account_proxy(
    state: State<'_, AppState>,
    account_id: String,
) -> std::result::Result<Option<HttpProxyConfig>, PebbleError> {
    Ok(get_oauth_account_proxy_setting(state, account_id)
        .await?
        .proxy)
}

#[tauri::command]
pub async fn get_oauth_account_proxy_setting(
    state: State<'_, AppState>,
    account_id: String,
) -> std::result::Result<AccountProxySetting, PebbleError> {
    ensure_oauth_account_provider(&state, &account_id)?;
    let stored = read_stored_oauth_auth_data_raw(&state.crypto, &state.store, &account_id)?
        .ok_or_else(|| {
            PebbleError::Internal(format!("No auth data found for account {account_id}"))
        })?;
    Ok(normalize_account_proxy_setting(
        stored.proxy_mode,
        stored.proxy,
    ))
}

#[tauri::command]
pub async fn update_oauth_account_proxy(
    state: State<'_, AppState>,
    account_id: String,
    proxy_host: Option<String>,
    proxy_port: Option<u16>,
) -> std::result::Result<(), PebbleError> {
    let proxy = oauth_proxy_from_parts(proxy_host, proxy_port)?;
    match proxy {
        Some(proxy) => {
            update_oauth_account_proxy_setting(
                state,
                account_id,
                AccountProxyMode::Custom,
                Some(proxy.host),
                Some(proxy.port),
            )
            .await
        }
        None => {
            update_oauth_account_proxy_setting(
                state,
                account_id,
                AccountProxyMode::Inherit,
                None,
                None,
            )
            .await
        }
    }
}

#[tauri::command]
pub async fn update_oauth_account_proxy_setting(
    state: State<'_, AppState>,
    account_id: String,
    mode: AccountProxyMode,
    proxy_host: Option<String>,
    proxy_port: Option<u16>,
) -> std::result::Result<(), PebbleError> {
    ensure_oauth_account_provider(&state, &account_id)?;
    let setting = account_proxy_setting_from_parts(mode, proxy_host, proxy_port, "OAuth proxy")?;
    let stored = read_stored_oauth_auth_data_raw(&state.crypto, &state.store, &account_id)?
        .ok_or_else(|| {
            PebbleError::Internal(format!("No auth data found for account {account_id}"))
        })?
        .with_proxy_setting(setting);
    persist_stored_oauth_auth_data_raw(&state.crypto, &state.store, &account_id, &stored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::sync::{Mutex, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    struct EnvVarGuard {
        key: &'static str,
        previous: Option<OsString>,
    }

    impl EnvVarGuard {
        fn set(key: &'static str, value: &str) -> Self {
            let previous = std::env::var_os(key);
            std::env::set_var(key, value);
            Self { key, previous }
        }
    }

    impl Drop for EnvVarGuard {
        fn drop(&mut self) {
            match &self.previous {
                Some(value) => std::env::set_var(self.key, value),
                None => std::env::remove_var(self.key),
            }
        }
    }

    #[test]
    fn dotenv_lookup_reads_unquoted_and_quoted_values() {
        let dotenv = r#"
GOOGLE_CLIENT_ID=google-client.apps.googleusercontent.com
GOOGLE_CLIENT_SECRET='google-secret'
MICROSOFT_CLIENT_ID="microsoft-client"
MICROSOFT_CLIENT_SECRET='microsoft-secret'
"#;

        assert_eq!(
            dotenv_lookup_from_str(dotenv, "GOOGLE_CLIENT_ID").as_deref(),
            Some("google-client.apps.googleusercontent.com")
        );
        assert_eq!(
            dotenv_lookup_from_str(dotenv, "GOOGLE_CLIENT_SECRET").as_deref(),
            Some("google-secret")
        );
        assert_eq!(
            dotenv_lookup_from_str(dotenv, "MICROSOFT_CLIENT_ID").as_deref(),
            Some("microsoft-client")
        );
        assert_eq!(
            dotenv_lookup_from_str(dotenv, "MICROSOFT_CLIENT_SECRET").as_deref(),
            Some("microsoft-secret")
        );
    }

    #[test]
    fn oauth_config_value_prefers_process_env_then_dotenv_then_compile_env() {
        assert_eq!(
            oauth_config_value_from_sources(
                "GOOGLE_CLIENT_ID",
                Some("from-env"),
                Some("GOOGLE_CLIENT_ID=from-dotenv"),
                Some("from-compile"),
                "placeholder",
            ),
            "from-env"
        );
        assert_eq!(
            oauth_config_value_from_sources(
                "GOOGLE_CLIENT_ID",
                None,
                Some("GOOGLE_CLIENT_ID=from-dotenv"),
                Some("from-compile"),
                "placeholder",
            ),
            "from-dotenv"
        );
        assert_eq!(
            oauth_config_value_from_sources(
                "GOOGLE_CLIENT_ID",
                None,
                None,
                Some("from-compile"),
                "placeholder",
            ),
            "from-compile"
        );
    }

    #[test]
    fn gmail_oauth_config_reads_optional_google_client_secret() {
        let _guard = env_lock().lock().expect("env lock poisoned");
        let _client_id = EnvVarGuard::set(
            "GOOGLE_CLIENT_ID",
            "google-client.apps.googleusercontent.com",
        );
        let _client_secret = EnvVarGuard::set("GOOGLE_CLIENT_SECRET", "google-secret");

        let config = gmail_oauth_config();

        assert_eq!(config.client_id, "google-client.apps.googleusercontent.com");
        assert_eq!(config.client_secret.as_deref(), Some("google-secret"));
    }

    #[test]
    fn oauth_provider_configs_wire_compile_time_client_secrets() {
        let source = include_str!("oauth.rs");
        let gmail_start = source
            .find("pub(crate) fn gmail_oauth_config")
            .expect("gmail_oauth_config should exist");
        let outlook_start = source
            .find("pub(crate) fn outlook_oauth_config")
            .expect("outlook_oauth_config should exist");
        let dotenv_start = source
            .find("fn dotenv_lookup_from_str")
            .expect("dotenv_lookup_from_str should exist");
        let gmail_section = &source[gmail_start..outlook_start];
        let outlook_section = &source[outlook_start..dotenv_start];

        assert!(gmail_section.contains(r#"option_env!("GOOGLE_CLIENT_SECRET")"#));
        assert!(outlook_section.contains(r#"option_env!("MICROSOFT_CLIENT_SECRET")"#));
    }

    #[test]
    fn stored_oauth_auth_data_decodes_legacy_token_blob_without_proxy() {
        let legacy = OAuthTokens {
            access_token: "access".to_string(),
            refresh_token: Some("refresh".to_string()),
            expires_at: Some(1234),
            scopes: vec!["scope".to_string()],
        };
        let bytes = serde_json::to_vec(&legacy).unwrap();

        let stored = decode_stored_oauth_auth_data(&bytes).unwrap();

        assert_eq!(stored.access_token, "access");
        assert_eq!(stored.refresh_token.as_deref(), Some("refresh"));
        assert_eq!(stored.expires_at, Some(1234));
        assert_eq!(stored.scopes, vec!["scope"]);
        assert_eq!(stored.proxy_mode, AccountProxyMode::Inherit);
        assert_eq!(stored.proxy, None);
    }

    #[test]
    fn stored_oauth_auth_data_replaces_tokens_and_preserves_proxy() {
        let stored = StoredOAuthAuthData {
            access_token: "old-access".to_string(),
            refresh_token: Some("old-refresh".to_string()),
            expires_at: Some(1),
            scopes: vec!["old-scope".to_string()],
            proxy_mode: AccountProxyMode::Custom,
            proxy: Some(pebble_core::HttpProxyConfig {
                host: "127.0.0.1".to_string(),
                port: 7890,
            }),
        };
        let replacement = OAuthTokens {
            access_token: "new-access".to_string(),
            refresh_token: Some("new-refresh".to_string()),
            expires_at: Some(2),
            scopes: vec!["new-scope".to_string()],
        };

        let updated = stored.with_tokens(replacement);

        assert_eq!(updated.access_token, "new-access");
        assert_eq!(updated.refresh_token.as_deref(), Some("new-refresh"));
        assert_eq!(updated.expires_at, Some(2));
        assert_eq!(updated.scopes, vec!["new-scope"]);
        assert_eq!(updated.proxy_mode, AccountProxyMode::Custom);
        assert_eq!(
            updated.proxy,
            Some(pebble_core::HttpProxyConfig {
                host: "127.0.0.1".to_string(),
                port: 7890,
            })
        );
    }

    #[test]
    fn oauth_proxy_from_parts_accepts_complete_proxy() {
        let proxy = oauth_proxy_from_parts(Some(" 127.0.0.1 ".to_string()), Some(7890))
            .unwrap()
            .unwrap();

        assert_eq!(proxy.host, "127.0.0.1");
        assert_eq!(proxy.port, 7890);
    }

    #[test]
    fn oauth_proxy_from_parts_rejects_partial_proxy() {
        let err = oauth_proxy_from_parts(Some("127.0.0.1".to_string()), None).unwrap_err();

        assert!(err.to_string().contains("proxy port"));
    }

    #[test]
    fn oauth_proxy_from_parts_rejects_invalid_proxy() {
        let err = oauth_proxy_from_parts(Some(" ".to_string()), Some(0)).unwrap_err();

        assert!(err.to_string().contains("Proxy host"));
    }

    #[test]
    fn stored_oauth_auth_data_replaces_proxy_and_preserves_tokens() {
        let stored = StoredOAuthAuthData {
            access_token: "access".to_string(),
            refresh_token: Some("refresh".to_string()),
            expires_at: Some(1234),
            scopes: vec!["scope".to_string()],
            proxy_mode: AccountProxyMode::Inherit,
            proxy: None,
        };

        let updated = stored.with_proxy(Some(pebble_core::HttpProxyConfig {
            host: "127.0.0.1".to_string(),
            port: 7890,
        }));

        assert_eq!(updated.access_token, "access");
        assert_eq!(updated.refresh_token.as_deref(), Some("refresh"));
        assert_eq!(updated.expires_at, Some(1234));
        assert_eq!(updated.scopes, vec!["scope"]);
        assert_eq!(updated.proxy_mode, AccountProxyMode::Custom);
        assert_eq!(
            updated.proxy,
            Some(pebble_core::HttpProxyConfig {
                host: "127.0.0.1".to_string(),
                port: 7890,
            })
        );
    }

    #[test]
    fn gmail_client_secret_missing_error_explains_secret_config() {
        let message = token_exchange_error_message(
            "gmail",
            &OAuthError::TokenExchange(
                "Server returned error response: invalid_request: client_secret is missing."
                    .to_string(),
            ),
        );

        assert!(message.contains("Google"));
        assert!(message.contains("GOOGLE_CLIENT_SECRET"));
        assert!(!message.contains("Token exchange failed: Token exchange failed"));
    }

    #[test]
    fn outlook_client_secret_missing_error_explains_app_registration_mode() {
        let message = token_exchange_error_message(
            "outlook",
            &OAuthError::TokenExchange(
                "Server returned error response: invalid_request: client_secret is missing."
                    .to_string(),
            ),
        );

        assert!(message.contains("confidential client"));
        assert!(message.contains("public/native client"));
        assert!(!message.contains("Token exchange failed: Token exchange failed"));
    }

    #[test]
    fn generic_token_exchange_error_is_not_double_prefixed() {
        let message = token_exchange_error_message(
            "gmail",
            &OAuthError::TokenExchange("Server returned error response: invalid_grant".to_string()),
        );

        assert_eq!(
            message,
            "Token exchange failed: Server returned error response: invalid_grant"
        );
    }
}
