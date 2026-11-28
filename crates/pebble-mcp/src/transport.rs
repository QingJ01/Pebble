//! Reaching the Pebble app and remembering the token.

use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};

use crate::protocol::{BridgeRequest, BridgeResponse};

pub type ConnectFuture<'a> =
    Pin<Box<dyn Future<Output = Result<BridgeResponse, String>> + Send + 'a>>;

/// Sends one request to Pebble and returns its answer. `Err` means Pebble
/// couldn't be reached (not running, or MCP turned off).
pub trait Connector: Send + Sync {
    fn send<'a>(&'a self, request: &'a BridgeRequest, wait: Duration) -> ConnectFuture<'a>;
}

/// Where the token for a client is kept.
pub trait TokenStore: Send + Sync {
    fn get(&self, client: &str) -> Option<String>;
    fn set(&self, client: &str, token: &str);
    fn delete(&self, client: &str);
}

/// One request per connection over a Unix socket or Windows named pipe.
pub struct SocketConnector {
    endpoint: String,
}

impl SocketConnector {
    pub fn new(endpoint: String) -> Self {
        Self { endpoint }
    }
}

/// Write the request line, read the answer line.
pub async fn exchange<S: AsyncRead + AsyncWrite + Unpin>(
    stream: S,
    request: &BridgeRequest,
    wait: Duration,
) -> Result<BridgeResponse, String> {
    let (read, mut write) = tokio::io::split(stream);
    let mut line = serde_json::to_string(request).map_err(|e| e.to_string())?;
    line.push('\n');
    write
        .write_all(line.as_bytes())
        .await
        .map_err(|e| e.to_string())?;
    write.flush().await.map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(read);
    let mut answer = String::new();
    match tokio::time::timeout(wait, reader.read_line(&mut answer)).await {
        Err(_) => Err("Pebble didn't answer in time".into()),
        Ok(Err(e)) => Err(e.to_string()),
        Ok(Ok(0)) => Err("Pebble closed the connection".into()),
        Ok(Ok(_)) => serde_json::from_str(answer.trim()).map_err(|e| e.to_string()),
    }
}

impl Connector for SocketConnector {
    fn send<'a>(&'a self, request: &'a BridgeRequest, wait: Duration) -> ConnectFuture<'a> {
        Box::pin(async move {
            #[cfg(unix)]
            {
                let stream = tokio::net::UnixStream::connect(&self.endpoint)
                    .await
                    .map_err(|e| format!("can't reach Pebble ({e})"))?;
                exchange(stream, request, wait).await
            }
            #[cfg(windows)]
            {
                let stream = tokio::net::windows::named_pipe::ClientOptions::new()
                    .open(&self.endpoint)
                    .map_err(|e| format!("can't reach Pebble ({e})"))?;
                exchange(stream, request, wait).await
            }
        })
    }
}

/// Tokens in the system keychain, one per client and Pebble profile.
pub struct KeyringTokens {
    endpoint: String,
}

const KEYRING_SERVICE: &str = "com.qingj01.pebble.mcp";

impl KeyringTokens {
    pub fn new(endpoint: String) -> Self {
        Self { endpoint }
    }

    fn entry(&self, client: &str) -> Option<keyring::Entry> {
        keyring::Entry::new(KEYRING_SERVICE, &format!("{}|{client}", self.endpoint)).ok()
    }
}

impl TokenStore for KeyringTokens {
    fn get(&self, client: &str) -> Option<String> {
        self.entry(client)?.get_password().ok()
    }

    fn set(&self, client: &str, token: &str) {
        if let Some(entry) = self.entry(client) {
            if let Err(e) = entry.set_password(token) {
                eprintln!("pebble-mcp: couldn't save the token in the keychain ({e}); you'll be asked to approve again next time");
            }
        }
    }

    fn delete(&self, client: &str) {
        if let Some(entry) = self.entry(client) {
            let _ = entry.delete_credential();
        }
    }
}
