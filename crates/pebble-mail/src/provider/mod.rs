pub mod gmail;
pub mod imap_provider;
pub mod mutations;
pub mod outlook;
pub mod pop3_provider;

use std::sync::Arc;

use pebble_core::traits::MailProvider;
use pebble_core::{HttpProxyConfig, PebbleError, ProviderType, Result};

pub(crate) fn http_client_with_proxy(proxy: Option<&HttpProxyConfig>) -> Result<reqwest::Client> {
    let mut builder = reqwest::ClientBuilder::new().no_proxy();
    if let Some(proxy) = proxy {
        let uri = proxy.socks5h_uri().map_err(PebbleError::Network)?;
        let reqwest_proxy = reqwest::Proxy::all(&uri)
            .map_err(|e| PebbleError::Network(format!("Invalid proxy: {e}")))?;
        builder = builder.proxy(reqwest_proxy);
    }
    builder
        .build()
        .map_err(|e| PebbleError::Network(format!("Failed to build HTTP client: {e}")))
}

fn proxy_from_credentials(credentials: &serde_json::Value) -> Result<Option<HttpProxyConfig>> {
    credentials
        .get("proxy")
        .cloned()
        .map(serde_json::from_value)
        .transpose()
        .map_err(|e| PebbleError::Auth(format!("Invalid OAuth proxy config: {e}")))
}

/// Create a trait-based mail provider from the given provider type and credentials.
pub async fn create_provider(
    provider_type: &ProviderType,
    credentials: &serde_json::Value,
    account_id: &str,
) -> Result<Arc<dyn MailProvider>> {
    match provider_type {
        ProviderType::Imap => {
            let imap_config: crate::imap::ImapConfig = serde_json::from_value(credentials.clone())
                .map_err(|e| PebbleError::Auth(format!("Invalid IMAP config: {e}")))?;
            let provider = imap_provider::ImapMailProvider::new(imap_config);
            Ok(Arc::new(provider))
        }
        ProviderType::Pop3 => {
            let pop3_config: crate::pop3::Pop3Config = serde_json::from_value(credentials.clone())
                .map_err(|e| PebbleError::Auth(format!("Invalid POP3 config: {e}")))?;
            let provider = pop3_provider::Pop3MailProvider::new(pop3_config);
            Ok(Arc::new(provider))
        }
        ProviderType::Gmail => {
            let token = credentials
                .get("access_token")
                .and_then(|v| v.as_str())
                .ok_or_else(|| PebbleError::Auth("Missing access_token for Gmail".to_string()))?
                .to_string();
            let provider =
                gmail::GmailProvider::new_with_proxy(token, proxy_from_credentials(credentials)?)?;
            Ok(Arc::new(provider))
        }
        ProviderType::Outlook => {
            let token = credentials
                .get("access_token")
                .and_then(|v| v.as_str())
                .ok_or_else(|| PebbleError::Auth("Missing access_token for Outlook".to_string()))?
                .to_string();
            let provider = outlook::OutlookProvider::new_with_proxy(
                token,
                account_id.to_string(),
                proxy_from_credentials(credentials)?,
            )?;
            Ok(Arc::new(provider))
        }
    }
}

#[cfg(test)]
pub(crate) mod proxy_test_support {
    use std::ffi::OsString;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::Mutex;

    static PROXY_ENV_LOCK: Mutex<()> = Mutex::new(());
    const PROXY_ENV_KEYS: [&str; 8] = [
        "HTTP_PROXY",
        "http_proxy",
        "HTTPS_PROXY",
        "https_proxy",
        "ALL_PROXY",
        "all_proxy",
        "NO_PROXY",
        "no_proxy",
    ];

    struct ProxyEnvironmentGuard(Vec<(&'static str, Option<OsString>)>);

    impl ProxyEnvironmentGuard {
        fn install(proxy_url: &str) -> Self {
            let previous = PROXY_ENV_KEYS
                .into_iter()
                .map(|key| (key, std::env::var_os(key)))
                .collect();
            for key in PROXY_ENV_KEYS {
                if key.eq_ignore_ascii_case("ALL_PROXY") {
                    std::env::set_var(key, proxy_url);
                } else {
                    std::env::remove_var(key);
                }
            }
            Self(previous)
        }
    }

    impl Drop for ProxyEnvironmentGuard {
        fn drop(&mut self) {
            for (key, value) in self.0.drain(..) {
                match value {
                    Some(value) => std::env::set_var(key, value),
                    None => std::env::remove_var(key),
                }
            }
        }
    }

    fn start_http_origin() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 1024];
            let _ = stream.read(&mut request);
            stream
                .write_all(
                    b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
        });
        format!("http://{address}/health")
    }

    fn unused_http_proxy_url() -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        drop(listener);
        format!("http://{address}")
    }

    pub(crate) async fn assert_client_builder_ignores_all_proxy(
        build_client: impl FnOnce() -> reqwest::Client,
    ) {
        let origin_url = start_http_origin();
        let client = {
            let _lock = PROXY_ENV_LOCK.lock().unwrap();
            let _proxy_environment = ProxyEnvironmentGuard::install(&unused_http_proxy_url());
            build_client()
        };

        let response = client
            .get(origin_url)
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn http_client_without_explicit_proxy_ignores_all_proxy() {
        proxy_test_support::assert_client_builder_ignores_all_proxy(|| {
            http_client_with_proxy(None).unwrap()
        })
        .await;
    }

    #[test]
    fn http_client_accepts_explicit_socks_proxy() {
        let proxy = HttpProxyConfig {
            host: "127.0.0.1".to_string(),
            port: 7890,
        };

        assert!(http_client_with_proxy(Some(&proxy)).is_ok());
    }
}
