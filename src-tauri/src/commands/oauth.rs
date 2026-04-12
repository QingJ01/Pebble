use crate::state::AppState;
use pebble_core::{Account, OAuthTokens, PebbleError, ProviderType, new_id, now_timestamp};
use pebble_crypto::CryptoService;
use pebble_mail::gmail_sync::TokenRefresher;
use pebble_oauth::{OAuthConfig, OAuthManager};
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
async fn fetch_userinfo(provider: &str, access_token: &str) -> Result<(String, String), PebbleError> {
    let url = match provider.to_lowercase().as_str() {
        "gmail" => "https://www.googleapis.com/oauth2/v2/userinfo",
        "outlook" => "https://graph.microsoft.com/v1.0/me",
        _ => return Err(PebbleError::UnsupportedProvider(provider.to_string())),
    };

    let client = reqwest::Client::new();
    let resp: serde_json::Value = client
        .get(url)
        .bearer_auth(access_token)
        .send()
        .await
        .map_err(|e| PebbleError::Network(format!("Userinfo request failed: {e}")))?
        .json()
        .await
        .map_err(|e| PebbleError::Network(format!("Userinfo parse failed: {e}")))?;

    let email = resp["email"].as_str()
        .or_else(|| resp["mail"].as_str())
        .or_else(|| resp["userPrincipalName"].as_str())
        .unwrap_or("")
        .to_string();

    let name = resp["name"].as_str()
        .or_else(|| resp["displayName"].as_str())
        .unwrap_or("")
        .to_string();

    debug!("Fetched userinfo from OAuth provider");
    Ok((email, name))
}

/// OAuth config for Gmail (Google).
pub(crate) fn gmail_oauth_config() -> OAuthConfig {
    OAuthConfig {
        client_id: option_env!("GOOGLE_CLIENT_ID")
            .unwrap_or("GOOGLE_CLIENT_ID_PLACEHOLDER")
            .to_string(),
        client_secret: None,
        auth_url: "https://accounts.google.com/o/oauth2/v2/auth".to_string(),
        token_url: "https://oauth2.googleapis.com/token".to_string(),
        scopes: vec![
            "https://mail.google.com/".to_string(),
            "https://www.googleapis.com/auth/userinfo.email".to_string(),
            "https://www.googleapis.com/auth/userinfo.profile".to_string(),
        ],
        redirect_port: 8756,
    }
}

/// OAuth config for Outlook (Microsoft).
pub(crate) fn outlook_oauth_config() -> OAuthConfig {
    OAuthConfig {
        client_id: std::env::var("MICROSOFT_CLIENT_ID")
            .unwrap_or_else(|_| "MICROSOFT_CLIENT_ID_PLACEHOLDER".to_string()),
        client_secret: std::env::var("MICROSOFT_CLIENT_SECRET").ok(),
        auth_url: "https://login.microsoftonline.com/common/oauth2/v2.0/authorize".to_string(),
        token_url: "https://login.microsoftonline.com/common/oauth2/v2.0/token".to_string(),
        scopes: vec![
            "https://graph.microsoft.com/Mail.ReadWrite".to_string(),
            "https://graph.microsoft.com/Mail.Send".to_string(),
            "https://graph.microsoft.com/User.Read".to_string(),
            "offline_access".to_string(),
        ],
        redirect_port: 8757,
    }
}

/// Resolve an `OAuthConfig` from a provider name, or return an error.
pub(crate) fn config_for_provider(provider: &str) -> Result<OAuthConfig, PebbleError> {
    match provider.to_lowercase().as_str() {
        "gmail" => Ok(gmail_oauth_config()),
        "outlook" => Ok(outlook_oauth_config()),
        _ => Err(PebbleError::UnsupportedProvider(format!(
            "Unknown OAuth provider: {provider}"
        ))),
    }
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
    let config_bytes = serde_json::to_vec(tokens)
        .map_err(|e| PebbleError::Internal(format!("Failed to serialize tokens: {e}")))?;
    let encrypted = crypto.encrypt(&config_bytes)?;
    store.set_auth_data(account_id, &encrypted)?;
    Ok(())
}

/// Decoded view of an account's stored OAuth token blob.
pub(crate) struct DecodedOAuthTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
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
    let encrypted = state
        .store
        .get_auth_data(account_id)?
        .ok_or_else(|| PebbleError::Internal(format!("No auth data for account {account_id}")))?;
    let decrypted = state.crypto.decrypt(&encrypted)?;
    let token_data: serde_json::Value = serde_json::from_slice(&decrypted)
        .map_err(|e| PebbleError::Internal(format!("Failed to parse token data: {e}")))?;
    Ok(DecodedOAuthTokens {
        access_token: token_data["access_token"]
            .as_str()
            .unwrap_or("")
            .to_string(),
        refresh_token: token_data["refresh_token"].as_str().map(|s| s.to_string()),
        expires_at: token_data["expires_at"].as_i64(),
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
                    let rt = match store.get_auth_data(&account_id)? {
                        Some(encrypted) => {
                            let decrypted = crypto.decrypt(&encrypted)?;
                            let token_data: serde_json::Value = serde_json::from_slice(&decrypted)
                                .map_err(|e| PebbleError::Internal(
                                    format!("Failed to parse token data: {e}")
                                ))?;
                            token_data["refresh_token"]
                                .as_str()
                                .map(|s| s.to_string())
                                .unwrap_or(initial_rt)
                        }
                        None => initial_rt,
                    };

                    let manager = OAuthManager::new(config);
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
                    Ok(token_pair.access_token)
                })
            })
        },
        None => Box::new(move || {
            let token = fallback_access_token.clone();
            Box::pin(async move { Ok(token) })
        }),
    }
}

pub(crate) async fn ensure_account_oauth_tokens(
    state: &AppState,
    account_id: &str,
    provider: &str,
) -> Result<OAuthTokens, PebbleError> {
    let encrypted = state
        .store
        .get_auth_data(account_id)?
        .ok_or_else(|| PebbleError::Internal(format!("No auth data found for account {account_id}")))?;
    let decrypted = state.crypto.decrypt(&encrypted)?;
    let mut tokens: OAuthTokens = serde_json::from_slice(&decrypted)
        .map_err(|e| PebbleError::Internal(format!("Failed to parse OAuth tokens: {e}")))?;

    let needs_refresh = tokens.refresh_token.is_some()
        && tokens
            .expires_at
            .map(|exp| exp - now_timestamp() < 300)
            .unwrap_or(false);

    if needs_refresh {
        let refresh_token = tokens.refresh_token.clone().unwrap_or_default();
        let manager = OAuthManager::new(config_for_provider(provider)?);
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

    Ok(tokens)
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
) -> std::result::Result<Account, PebbleError> {
    let config = config_for_provider(&provider)?;
    let manager = OAuthManager::new(config);

    // Start auth flow (generates PKCE challenge)
    let (auth_url, pkce_state) = manager
        .start_auth()
        .await
        .map_err(|e| PebbleError::OAuth(format!("Failed to start OAuth flow: {e}")))?;

    // Open the authorization URL in the system browser
    opener::open(&auth_url)
        .map_err(|e| PebbleError::OAuth(format!("Failed to open browser: {e}")))?;

    // Wait for the redirect callback with the authorization code
    let redirect = manager
        .wait_for_redirect()
        .await
        .map_err(|e| PebbleError::OAuth(format!("OAuth redirect failed: {e}")))?;

    if !constant_time_eq(&redirect.state, pkce_state.csrf_token.secret()) {
        return Err(PebbleError::OAuth("OAuth state mismatch".to_string()));
    }

    // Exchange code for tokens
    let token_pair = manager
        .complete_auth(&redirect.code, pkce_state)
        .await
        .map_err(|e| PebbleError::OAuth(format!("Token exchange failed: {e}")))?;

    // Fetch user info from Google/Microsoft to get actual email and display name
    let (real_email, real_name) = fetch_userinfo(&provider, &token_pair.access_token).await
        .unwrap_or_else(|_| (email.clone(), display_name.clone()));

    let final_email = if real_email.is_empty() { email } else { real_email };
    let final_name = if real_name.is_empty() { display_name } else { real_name };

    // Create the account
    let now = now_timestamp();
    let account = Account {
        id: new_id(),
        email: final_email,
        display_name: final_name,
        provider: provider_type(&provider)?,
        created_at: now,
        updated_at: now,
    };

    state.store.insert_account(&account)?;

    // Encrypt tokens and store as auth_data
    let tokens = OAuthTokens {
        access_token: token_pair.access_token,
        refresh_token: token_pair.refresh_token,
        expires_at: token_pair.expires_at,
        scopes: token_pair.scopes,
    };
    persist_oauth_tokens(&state, &account.id, &tokens)?;

    // Store provider metadata in sync_state
    let slug = provider_slug(&account.provider).to_string();
    state.store.update_sync_state(&account.id, |s| {
        s.provider = Some(slug);
    })?;

    Ok(account)
}
