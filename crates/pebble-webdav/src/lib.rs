//! HTTP/WebDAV transport for settings backups.
//!
//! Kept out of `pebble-store` so the storage layer has no network
//! dependencies; the store only (de)serialises backup payloads.

use pebble_core::{PebbleError, Result};

/// Default cap for downloaded files (matches the store's backup size limit).
pub const DEFAULT_MAX_DOWNLOAD_BYTES: usize = 16 * 1024 * 1024;

/// Minimal WebDAV client used for settings backups.
///
/// Security properties: only `https://` endpoints are accepted (credentials
/// travel with every request), redirects are never followed, system/env
/// proxies are ignored, and downloads are capped at `max_download_bytes`
/// without buffering an oversized body.
pub struct WebDavClient {
    url: String,
    username: String,
    password: String,
    client: reqwest::Client,
    max_download_bytes: usize,
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
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|e| PebbleError::Internal(format!("Failed to create HTTP client: {e}")))?;
        Ok(Self {
            url: trimmed,
            username,
            password,
            client,
            max_download_bytes: DEFAULT_MAX_DOWNLOAD_BYTES,
        })
    }

    /// Override the download size cap (defaults to [`DEFAULT_MAX_DOWNLOAD_BYTES`]).
    pub fn with_max_download_bytes(mut self, max_download_bytes: usize) -> Self {
        self.max_download_bytes = max_download_bytes;
        self
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
    /// Rejects responses larger than the download cap without buffering
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
            if len as usize > self.max_download_bytes {
                return Err(PebbleError::Validation(format!(
                    "Backup file is too large ({} bytes, max {})",
                    len, self.max_download_bytes
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
            if buf.len() + chunk.len() > self.max_download_bytes {
                return Err(PebbleError::Validation(format!(
                    "Backup file exceeds maximum size ({} bytes)",
                    self.max_download_bytes
                )));
            }
            buf.extend_from_slice(&chunk);
        }
        Ok(buf)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
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

    #[tokio::test]
    async fn webdav_client_ignores_all_proxy() {
        let origin_url = start_http_origin();
        let webdav = {
            let _lock = PROXY_ENV_LOCK.lock().unwrap();
            let _proxy_environment = ProxyEnvironmentGuard::install(&unused_http_proxy_url());
            WebDavClient::new(
                "https://example.com/webdav".to_string(),
                "user".to_string(),
                "password".to_string(),
            )
            .unwrap()
        };

        let response = webdav
            .client
            .get(origin_url)
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
            .unwrap();

        assert_eq!(response.status(), reqwest::StatusCode::NO_CONTENT);
    }

    #[test]
    fn rejects_non_https_urls() {
        for url in ["http://example.com/dav", "ftp://example.com", "example.com"] {
            let err = WebDavClient::new(url.to_string(), "u".into(), "p".into())
                .err()
                .expect("non-https URL must be rejected");
            assert!(matches!(err, PebbleError::Validation(_)), "{url}: {err:?}");
        }
        assert!(
            WebDavClient::new("https://example.com/dav/".into(), "u".into(), "p".into()).is_ok()
        );
    }

    fn serve_once(response: Vec<u8>) -> String {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 2048];
            let _ = stream.read(&mut request);
            let _ = stream.write_all(&response);
        });
        format!("http://{address}/")
    }

    #[tokio::test]
    async fn client_does_not_follow_redirects() {
        let target = serve_once(
            b"HTTP/1.1 302 Found\r\nLocation: https://example.com/elsewhere\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec(),
        );
        let webdav =
            WebDavClient::new("https://example.com/dav".into(), "u".into(), "p".into()).unwrap();
        let response = webdav
            .client
            .get(target)
            .timeout(std::time::Duration::from_secs(2))
            .send()
            .await
            .unwrap();
        assert_eq!(response.status(), reqwest::StatusCode::FOUND);
    }

    #[test]
    fn download_cap_defaults_and_can_be_overridden() {
        let webdav =
            WebDavClient::new("https://example.com/dav".into(), "u".into(), "p".into()).unwrap();
        assert_eq!(webdav.max_download_bytes, DEFAULT_MAX_DOWNLOAD_BYTES);
        assert_eq!(webdav.with_max_download_bytes(10).max_download_bytes, 10);
    }
}
