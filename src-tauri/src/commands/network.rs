use crate::commands::encrypted_store::{load_secure_user_data, store_secure_user_data};
use crate::state::AppState;
use pebble_core::{HttpProxyConfig, PebbleError};
use pebble_crypto::CryptoService;
use pebble_mail::ProxyConfig;
use pebble_store::Store;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use tauri::State;

const GLOBAL_PROXY_KEY: &str = "global_network_proxy";

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "lowercase")]
pub enum AccountProxyMode {
    #[default]
    Inherit,
    Disabled,
    Custom,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct AccountProxySetting {
    pub mode: AccountProxyMode,
    pub proxy: Option<HttpProxyConfig>,
}

pub(crate) fn is_inherit_proxy_mode(mode: &AccountProxyMode) -> bool {
    matches!(mode, AccountProxyMode::Inherit)
}

fn decrypt_json<T: DeserializeOwned>(
    crypto: &CryptoService,
    store: &Store,
    key: &str,
) -> Result<Option<T>, PebbleError> {
    let Some(decrypted) = load_secure_user_data(crypto, store, key)? else {
        return Ok(None);
    };
    serde_json::from_slice(&decrypted)
        .map(Some)
        .map_err(|e| PebbleError::Internal(format!("Invalid secure user data for {key}: {e}")))
}

fn encrypt_json<T: Serialize>(
    crypto: &CryptoService,
    store: &Store,
    key: &str,
    value: &T,
) -> Result<(), PebbleError> {
    let plaintext = serde_json::to_vec(value)
        .map_err(|e| PebbleError::Internal(format!("Failed to serialize secure user data: {e}")))?;
    store_secure_user_data(crypto, store, key, &plaintext)
}

pub(crate) fn proxy_config_from_parts(
    proxy_host: Option<String>,
    proxy_port: Option<u16>,
    label: &str,
) -> Result<Option<HttpProxyConfig>, PebbleError> {
    match (proxy_host, proxy_port) {
        (None, None) => Ok(None),
        (Some(host), None) if host.trim().is_empty() => Ok(None),
        (Some(_), None) => Err(PebbleError::Network(format!(
            "{label} port is required when proxy host is set"
        ))),
        (None, Some(_)) => Err(PebbleError::Network(format!(
            "{label} host is required when proxy port is set"
        ))),
        (Some(host), Some(port)) => {
            let proxy = HttpProxyConfig {
                host: host.trim().to_string(),
                port,
            };
            proxy.validate().map_err(PebbleError::Network)?;
            Ok(Some(proxy))
        }
    }
}

pub(crate) fn resolve_effective_proxy(
    account_proxy: Option<HttpProxyConfig>,
    global_proxy: Option<HttpProxyConfig>,
) -> Option<HttpProxyConfig> {
    account_proxy.or(global_proxy)
}

pub(crate) fn resolve_effective_proxy_setting(
    mode: AccountProxyMode,
    account_proxy: Option<HttpProxyConfig>,
    global_proxy: Option<HttpProxyConfig>,
) -> Option<HttpProxyConfig> {
    match mode {
        AccountProxyMode::Inherit => resolve_effective_proxy(account_proxy, global_proxy),
        AccountProxyMode::Disabled => None,
        AccountProxyMode::Custom => account_proxy,
    }
}

pub(crate) fn normalize_account_proxy_setting(
    mode: AccountProxyMode,
    proxy: Option<HttpProxyConfig>,
) -> AccountProxySetting {
    let mode = if matches!(mode, AccountProxyMode::Inherit) && proxy.is_some() {
        AccountProxyMode::Custom
    } else {
        mode
    };
    let proxy = if matches!(mode, AccountProxyMode::Custom) {
        proxy
    } else {
        None
    };

    AccountProxySetting { mode, proxy }
}

pub(crate) fn account_proxy_setting_from_parts(
    mode: AccountProxyMode,
    proxy_host: Option<String>,
    proxy_port: Option<u16>,
    label: &str,
) -> Result<AccountProxySetting, PebbleError> {
    let proxy = proxy_config_from_parts(proxy_host, proxy_port, label)?;
    match mode {
        AccountProxyMode::Custom => {
            let proxy = proxy.ok_or_else(|| {
                PebbleError::Network(format!(
                    "{label} host and port are required when custom proxy is selected"
                ))
            })?;
            Ok(AccountProxySetting {
                mode,
                proxy: Some(proxy),
            })
        }
        AccountProxyMode::Inherit | AccountProxyMode::Disabled => {
            Ok(AccountProxySetting { mode, proxy: None })
        }
    }
}

pub(crate) fn mail_proxy_from_http(proxy: HttpProxyConfig) -> ProxyConfig {
    ProxyConfig {
        host: proxy.host,
        port: proxy.port,
    }
}

pub(crate) fn http_proxy_from_mail_proxy(proxy: &ProxyConfig) -> HttpProxyConfig {
    HttpProxyConfig {
        host: proxy.host.clone(),
        port: proxy.port,
    }
}

pub(crate) fn get_global_proxy_raw(
    crypto: &CryptoService,
    store: &Store,
) -> Result<Option<HttpProxyConfig>, PebbleError> {
    decrypt_json(crypto, store, GLOBAL_PROXY_KEY)
}

pub(crate) fn set_global_proxy_raw(
    crypto: &CryptoService,
    store: &Store,
    proxy: Option<HttpProxyConfig>,
) -> Result<(), PebbleError> {
    match proxy {
        Some(proxy) => encrypt_json(crypto, store, GLOBAL_PROXY_KEY, &proxy),
        None => store.delete_secure_user_data(GLOBAL_PROXY_KEY),
    }
}

pub(crate) fn resolve_mail_proxy_from_mode(
    crypto: &CryptoService,
    store: &Store,
    mode: AccountProxyMode,
    account_proxy: Option<ProxyConfig>,
) -> Result<Option<ProxyConfig>, PebbleError> {
    let account_proxy = account_proxy.as_ref().map(http_proxy_from_mail_proxy);
    let global_proxy = get_global_proxy_raw(crypto, store)?;
    Ok(
        resolve_effective_proxy_setting(mode, account_proxy, global_proxy)
            .map(mail_proxy_from_http),
    )
}

pub(crate) fn account_proxy_mode_from_auth_value(value: &serde_json::Value) -> AccountProxyMode {
    value
        .get("proxy_mode")
        .and_then(|mode| serde_json::from_value(mode.clone()).ok())
        .unwrap_or_default()
}

#[tauri::command]
pub async fn get_global_proxy(
    state: State<'_, AppState>,
) -> Result<Option<HttpProxyConfig>, PebbleError> {
    get_global_proxy_raw(&state.crypto, &state.store)
}

#[tauri::command]
pub async fn update_global_proxy(
    state: State<'_, AppState>,
    proxy_host: Option<String>,
    proxy_port: Option<u16>,
) -> Result<(), PebbleError> {
    let proxy = proxy_config_from_parts(proxy_host, proxy_port, "Global proxy")?;
    set_global_proxy_raw(&state.crypto, &state.store, proxy)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_core::HttpProxyConfig;

    #[test]
    fn global_proxy_from_parts_rejects_partial_config() {
        let err = proxy_config_from_parts(Some("127.0.0.1".to_string()), None, "Global proxy")
            .unwrap_err();

        assert!(err.to_string().contains("port"));
    }

    #[test]
    fn global_proxy_from_parts_trims_complete_config() {
        let proxy =
            proxy_config_from_parts(Some(" 127.0.0.1 ".to_string()), Some(7890), "Global proxy")
                .unwrap()
                .unwrap();

        assert_eq!(proxy.host, "127.0.0.1");
        assert_eq!(proxy.port, 7890);
    }

    #[test]
    fn account_proxy_overrides_global_proxy() {
        let account = Some(HttpProxyConfig {
            host: "10.0.0.2".to_string(),
            port: 1080,
        });
        let global = Some(HttpProxyConfig {
            host: "127.0.0.1".to_string(),
            port: 7890,
        });

        assert_eq!(resolve_effective_proxy(account.clone(), global), account);
    }

    #[test]
    fn empty_account_proxy_inherits_global_proxy() {
        let global = Some(HttpProxyConfig {
            host: "127.0.0.1".to_string(),
            port: 7890,
        });

        assert_eq!(resolve_effective_proxy(None, global.clone()), global);
    }

    #[test]
    fn disabled_account_proxy_mode_ignores_global_proxy() {
        let global = Some(HttpProxyConfig {
            host: "127.0.0.1".to_string(),
            port: 7890,
        });

        assert_eq!(
            resolve_effective_proxy_setting(AccountProxyMode::Disabled, None, global),
            None
        );
    }
}
