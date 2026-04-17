use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use async_imap::Client;
use futures::TryStreamExt;
use pebble_core::{Folder, FolderRole, FolderType, PebbleError, Result, new_id};
use serde::de::Deserializer;
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_rustls::client::TlsStream;
use tracing::debug;

/// Connection security mode for mail protocols.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionSecurity {
    /// Implicit TLS — connect over TLS immediately (IMAP 993, SMTP 465).
    Tls,
    /// STARTTLS — connect plain then upgrade to TLS (IMAP 143, SMTP 587).
    #[serde(rename = "starttls")]
    StartTls,
    /// No encryption (not recommended).
    Plain,
}

impl Default for ConnectionSecurity {
    fn default() -> Self {
        Self::Tls
    }
}

/// Optional SOCKS5 proxy configuration.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ProxyConfig {
    pub host: String,
    pub port: u16,
}

/// Configuration for an IMAP connection.
#[derive(Clone, serde::Serialize)]
pub struct ImapConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(skip_serializing)]
    pub password: String,
    pub security: ConnectionSecurity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyConfig>,
}

impl std::fmt::Debug for ImapConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ImapConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .field("security", &self.security)
            .field("proxy", &self.proxy)
            .finish()
    }
}

// Custom Deserialize to handle legacy `use_tls: bool` configs.
impl<'de> serde::Deserialize<'de> for ImapConfig {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Raw {
            host: String,
            port: u16,
            username: String,
            password: String,
            #[serde(default)]
            security: Option<ConnectionSecurity>,
            #[serde(default)]
            use_tls: Option<bool>,
            #[serde(default)]
            proxy: Option<ProxyConfig>,
        }

        let raw = Raw::deserialize(deserializer)?;
        let security = raw.security.unwrap_or_else(|| match raw.use_tls {
            Some(false) => ConnectionSecurity::Plain,
            _ => ConnectionSecurity::Tls,
        });

        Ok(ImapConfig {
            host: raw.host,
            port: raw.port,
            username: raw.username,
            password: raw.password,
            security,
            proxy: raw.proxy,
        })
    }
}

/// Configuration for an SMTP connection.
#[derive(Clone, serde::Serialize)]
pub struct SmtpConfig {
    pub host: String,
    pub port: u16,
    pub username: String,
    #[serde(skip_serializing)]
    pub password: String,
    pub security: ConnectionSecurity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<ProxyConfig>,
}

impl std::fmt::Debug for SmtpConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SmtpConfig")
            .field("host", &self.host)
            .field("port", &self.port)
            .field("username", &self.username)
            .field("password", &"[REDACTED]")
            .field("security", &self.security)
            .field("proxy", &self.proxy)
            .finish()
    }
}

// Custom Deserialize to handle legacy `use_tls: bool` configs.
impl<'de> serde::Deserialize<'de> for SmtpConfig {
    fn deserialize<D>(deserializer: D) -> std::result::Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        #[derive(serde::Deserialize)]
        struct Raw {
            host: String,
            port: u16,
            username: String,
            password: String,
            #[serde(default)]
            security: Option<ConnectionSecurity>,
            #[serde(default)]
            use_tls: Option<bool>,
            #[serde(default)]
            proxy: Option<ProxyConfig>,
        }

        let raw = Raw::deserialize(deserializer)?;
        let security = raw.security.unwrap_or_else(|| match raw.use_tls {
            Some(false) => ConnectionSecurity::Plain,
            _ => ConnectionSecurity::Tls,
        });

        Ok(SmtpConfig {
            host: raw.host,
            port: raw.port,
            username: raw.username,
            password: raw.password,
            security,
            proxy: raw.proxy,
        })
    }
}

/// Stream wrapper that replays buffered prefix bytes, then delegates to inner.
/// Used to replay the IMAP greeting after manually sending an ID command.
#[derive(Debug)]
struct PrefixedStream<T> {
    prefix: Vec<u8>,
    pos: usize,
    inner: T,
}

impl<T> PrefixedStream<T> {
    fn with_prefix(prefix: Vec<u8>, inner: T) -> Self {
        Self { prefix, pos: 0, inner }
    }
}

impl<T: AsyncRead + Unpin> AsyncRead for PrefixedStream<T> {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        let this = self.get_mut();
        if this.pos < this.prefix.len() {
            let remaining = &this.prefix[this.pos..];
            let n = remaining.len().min(buf.remaining());
            buf.put_slice(&remaining[..n]);
            this.pos += n;
            Poll::Ready(Ok(()))
        } else {
            Pin::new(&mut this.inner).poll_read(cx, buf)
        }
    }
}

impl<T: AsyncWrite + Unpin> AsyncWrite for PrefixedStream<T> {
    fn poll_write(self: Pin<&mut Self>, cx: &mut Context<'_>, buf: &[u8]) -> Poll<io::Result<usize>> {
        Pin::new(&mut self.get_mut().inner).poll_write(cx, buf)
    }
    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_flush(cx)
    }
    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        Pin::new(&mut self.get_mut().inner).poll_shutdown(cx)
    }
}

/// A stream that is either wrapped in TLS or raw TCP. Implementing the
/// tokio async I/O traits on this enum means `async_imap::Session` is
/// generic over a single type, so every operation site can drop the old
/// `match self.session { Tls(_) => ..., Plain(_) => ... }` duplication.
enum InnerStream {
    Tls(TlsStream<TcpStream>),
    Plain(TcpStream),
}

impl std::fmt::Debug for InnerStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InnerStream::Tls(_) => f.debug_struct("InnerStream::Tls").finish(),
            InnerStream::Plain(_) => f.debug_struct("InnerStream::Plain").finish(),
        }
    }
}

impl AsyncRead for InnerStream {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<io::Result<()>> {
        match self.get_mut() {
            InnerStream::Tls(s) => Pin::new(s).poll_read(cx, buf),
            InnerStream::Plain(s) => Pin::new(s).poll_read(cx, buf),
        }
    }
}

impl AsyncWrite for InnerStream {
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        match self.get_mut() {
            InnerStream::Tls(s) => Pin::new(s).poll_write(cx, buf),
            InnerStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            InnerStream::Tls(s) => Pin::new(s).poll_flush(cx),
            InnerStream::Plain(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            InnerStream::Tls(s) => Pin::new(s).poll_shutdown(cx),
            InnerStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

/// Unified IMAP session type — a single `async_imap::Session` regardless
/// of whether the underlying transport is TLS or plain TCP.
type ImapSession = async_imap::Session<PrefixedStream<InnerStream>>;

/// An IMAP provider that manages a connection and session.
pub struct ImapProvider {
    config: ImapConfig,
    session: Arc<Mutex<Option<ImapSession>>>,
}

/// Build a rustls TLS connector with bundled root certificates.
fn build_tls_connector() -> tokio_rustls::TlsConnector {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .expect("TLS protocol versions")
        .with_root_certificates(root_store)
        .with_no_client_auth();
    tokio_rustls::TlsConnector::from(Arc::new(config))
}

/// Perform a TLS handshake using rustls on the given TCP stream.
async fn tls_connect(host: &str, tcp: TcpStream) -> Result<TlsStream<TcpStream>> {
    let connector = build_tls_connector();
    let server_name = rustls::pki_types::ServerName::try_from(host)
        .map_err(|e| PebbleError::Network(format!("Invalid server name '{}': {}", host, e)))?
        .to_owned();
    connector.connect(server_name, tcp).await
        .map_err(|e| PebbleError::Network(format!("TLS handshake with {}: {}", host, e)))
}

impl ImapProvider {
    /// Create a new provider with the given configuration.
    pub fn new(config: ImapConfig) -> Self {
        Self {
            config,
            session: Arc::new(Mutex::new(None)),
        }
    }

    /// Return a clone of the connection configuration.
    pub fn config(&self) -> ImapConfig {
        self.config.clone()
    }

    /// Whether this host requires an RFC 2971 ID command before LOGIN
    /// (Netease/163/126/188 servers reject as "Unsafe Login" without it).
    fn needs_id_command(&self) -> bool {
        let h = self.config.host.to_lowercase();
        h.contains("163.com") || h.contains("126.com") || h.contains("188.com")
            || h.contains("yeah.net") || h.contains("netease.com")
            || h.contains("sina.com") || h.contains("sina.cn")
    }

    /// Send IMAP ID command on a raw stream, returning the greeting bytes
    /// so they can be replayed for `Client::new()`.
    async fn send_id_before_login<S: AsyncRead + AsyncWrite + Unpin>(
        stream: &mut S,
    ) -> Result<Vec<u8>> {
        // Read server greeting (e.g. "* OK Coremail ...")
        let mut greeting = vec![0u8; 8192];
        let n = stream
            .read(&mut greeting)
            .await
            .map_err(|e| PebbleError::Network(format!("Read greeting: {e}")))?;
        greeting.truncate(n);

        // Send ID command
        stream
            .write_all(b"A000 ID (\"name\" \"Pebble\" \"version\" \"1.0\" \"vendor\" \"Pebble\")\r\n")
            .await
            .map_err(|e| PebbleError::Network(format!("Send ID: {e}")))?;
        stream
            .flush()
            .await
            .map_err(|e| PebbleError::Network(format!("Flush ID: {e}")))?;

        // Read ID response until we see the tagged response (A000 OK/NO/BAD)
        let mut resp = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = stream
                .read(&mut buf)
                .await
                .map_err(|e| PebbleError::Network(format!("Read ID response: {e}")))?;
            if n == 0 {
                return Err(PebbleError::Network("Connection closed during ID".into()));
            }
            resp.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&resp);
            if text.contains("A000 OK") || text.contains("A000 NO") || text.contains("A000 BAD") {
                break;
            }
        }
        debug!("IMAP ID command accepted");
        Ok(greeting)
    }

    /// Send STARTTLS command on a raw TCP stream and upgrade to TLS.
    /// Returns the original greeting bytes (for replay) and the TLS stream.
    async fn starttls_upgrade(
        host: &str,
        tcp: TcpStream,
        greeting: Vec<u8>,
    ) -> Result<(Vec<u8>, TlsStream<TcpStream>)> {
        let mut tcp = tcp;

        // Send STARTTLS command
        tcp.write_all(b"A001 STARTTLS\r\n")
            .await
            .map_err(|e| PebbleError::Network(format!("Send STARTTLS: {e}")))?;
        tcp.flush()
            .await
            .map_err(|e| PebbleError::Network(format!("Flush STARTTLS: {e}")))?;

        // Read STARTTLS response
        let mut resp = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = tcp
                .read(&mut buf)
                .await
                .map_err(|e| PebbleError::Network(format!("Read STARTTLS response: {e}")))?;
            if n == 0 {
                return Err(PebbleError::Network(
                    "Connection closed during STARTTLS".into(),
                ));
            }
            resp.extend_from_slice(&buf[..n]);
            let text = String::from_utf8_lossy(&resp);
            if text.contains("A001 OK") {
                break;
            }
            if text.contains("A001 NO") || text.contains("A001 BAD") {
                return Err(PebbleError::Network(format!(
                    "Server rejected STARTTLS: {}",
                    text.trim()
                )));
            }
        }
        debug!("STARTTLS accepted, upgrading connection");

        // Upgrade to TLS
        let tls_stream = tls_connect(host, tcp).await?;

        Ok((greeting, tls_stream))
    }

    /// Establish a TCP connection, optionally through a SOCKS5 proxy.
    async fn tcp_connect(&self) -> Result<TcpStream> {
        let addr = format!("{}:{}", self.config.host, self.config.port);

        let tcp = if let Some(ref proxy) = self.config.proxy {
            let proxy_addr = format!("{}:{}", proxy.host, proxy.port);
            debug!("Connecting to {} via SOCKS5 proxy {} (security={:?})...", addr, proxy_addr, self.config.security);
            let stream = tokio_socks::tcp::Socks5Stream::connect(
                proxy_addr.as_str(),
                addr.as_str(),
            )
            .await
            .map_err(|e| PebbleError::Network(format!("SOCKS5 proxy connect to {addr} via {proxy_addr}: {e}")))?;
            let tcp = stream.into_inner();
            if let Ok(peer) = tcp.peer_addr() {
                debug!("SOCKS5 connected to {} (proxy peer: {})", addr, peer);
            }
            tcp
        } else {
            debug!("Resolving and connecting to {} (security={:?})...", addr, self.config.security);
            let tcp = TcpStream::connect(&addr)
                .await
                .map_err(|e| PebbleError::Network(format!("TCP connect to {addr}: {e}")))?;
            if let Ok(peer) = tcp.peer_addr() {
                debug!("TCP connected to {} (resolved IP: {})", addr, peer);
            }
            tcp
        };

        Ok(tcp)
    }

    /// Connect to the IMAP server and log in.
    pub async fn connect(&self) -> Result<()> {
        let tcp = self.tcp_connect().await?;

        let needs_id = self.needs_id_command();

        let session: ImapSession = match self.config.security {
            ConnectionSecurity::Tls => {
                // Implicit TLS — wrap immediately
                debug!("Starting TLS handshake (rustls) with SNI={}", self.config.host);
                let mut tls_stream = tls_connect(&self.config.host, tcp).await?;

                let prefix = if needs_id {
                    Self::send_id_before_login(&mut tls_stream).await?
                } else {
                    Vec::new()
                };
                let stream = PrefixedStream::with_prefix(prefix, InnerStream::Tls(tls_stream));

                let client = Client::new(stream);
                client
                    .login(&self.config.username, &self.config.password)
                    .await
                    .map_err(|(e, _)| PebbleError::Auth(format!("IMAP login failed: {e}")))?
            }
            ConnectionSecurity::StartTls => {
                // Connect plain, read greeting, optionally send ID, then STARTTLS upgrade
                let mut tcp = tcp;

                // Read greeting
                let mut greeting = vec![0u8; 8192];
                let n = tcp
                    .read(&mut greeting)
                    .await
                    .map_err(|e| PebbleError::Network(format!("Read greeting: {e}")))?;
                greeting.truncate(n);

                // Send ID command before STARTTLS if needed (on plain connection)
                if needs_id {
                    tcp.write_all(b"A000 ID (\"name\" \"Pebble\" \"version\" \"1.0\" \"vendor\" \"Pebble\")\r\n")
                        .await
                        .map_err(|e| PebbleError::Network(format!("Send ID: {e}")))?;
                    tcp.flush()
                        .await
                        .map_err(|e| PebbleError::Network(format!("Flush ID: {e}")))?;

                    let mut resp = Vec::new();
                    let mut buf = [0u8; 4096];
                    loop {
                        let n = tcp
                            .read(&mut buf)
                            .await
                            .map_err(|e| PebbleError::Network(format!("Read ID response: {e}")))?;
                        if n == 0 {
                            return Err(PebbleError::Network("Connection closed during ID".into()));
                        }
                        resp.extend_from_slice(&buf[..n]);
                        let text = String::from_utf8_lossy(&resp);
                        if text.contains("A000 OK") || text.contains("A000 NO") || text.contains("A000 BAD") {
                            break;
                        }
                    }
                    debug!("IMAP ID command accepted (pre-STARTTLS)");
                }

                // STARTTLS upgrade
                let (greeting, tls_stream) =
                    Self::starttls_upgrade(&self.config.host, tcp, greeting).await?;

                // Replay the original greeting so Client::new() is happy
                let stream = PrefixedStream::with_prefix(greeting, InnerStream::Tls(tls_stream));
                let client = Client::new(stream);
                client
                    .login(&self.config.username, &self.config.password)
                    .await
                    .map_err(|(e, _)| PebbleError::Auth(format!("IMAP login failed: {e}")))?
            }
            ConnectionSecurity::Plain => {
                // Plain TCP — no encryption
                let mut tcp = tcp;
                let prefix = if needs_id {
                    Self::send_id_before_login(&mut tcp).await?
                } else {
                    Vec::new()
                };
                let stream = PrefixedStream::with_prefix(prefix, InnerStream::Plain(tcp));

                let client = Client::new(stream);
                client
                    .login(&self.config.username, &self.config.password)
                    .await
                    .map_err(|(e, _)| PebbleError::Auth(format!("IMAP login failed: {e}")))?
            }
        };

        let mut guard = self.session.lock().await;
        *guard = Some(session);
        debug!("IMAP connected to {} ({:?})", self.config.host, self.config.security);
        Ok(())
    }

    /// Test connectivity without logging in. Returns a diagnostic summary.
    /// Tries TCP connect → TLS handshake → read IMAP greeting.
    pub async fn test_connection(config: &ImapConfig) -> Result<String> {
        use std::time::Instant;
        let addr = format!("{}:{}", config.host, config.port);
        let mut report = String::new();

        // Step 1: TCP connect (optionally via proxy)
        let t0 = Instant::now();
        let tcp = if let Some(ref proxy) = config.proxy {
            let proxy_addr = format!("{}:{}", proxy.host, proxy.port);
            let stream = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                tokio_socks::tcp::Socks5Stream::connect(proxy_addr.as_str(), addr.as_str()),
            )
            .await
            .map_err(|_| PebbleError::Network(format!("SOCKS5 connect to {proxy_addr} timed out (10s)")))?
            .map_err(|e| PebbleError::Network(format!("SOCKS5 proxy: {e}")))?;
            let tcp = stream.into_inner();
            report.push_str(&format!("TCP via SOCKS5 {proxy_addr}: OK ({:.0}ms)\n", t0.elapsed().as_millis()));
            tcp
        } else {
            let tcp = tokio::time::timeout(
                std::time::Duration::from_secs(10),
                TcpStream::connect(&addr),
            )
            .await
            .map_err(|_| PebbleError::Network(format!("TCP connect to {addr} timed out (10s)")))?
            .map_err(|e| PebbleError::Network(format!("TCP connect: {e}")))?;
            if let Ok(peer) = tcp.peer_addr() {
                report.push_str(&format!("TCP direct to {addr} (IP: {peer}): OK ({:.0}ms)\n", t0.elapsed().as_millis()));
            } else {
                report.push_str(&format!("TCP direct to {addr}: OK ({:.0}ms)\n", t0.elapsed().as_millis()));
            }
            tcp
        };

        // Step 2: TLS handshake (if applicable)
        match config.security {
            ConnectionSecurity::Tls => {
                let t1 = Instant::now();
                let mut tls = tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    tls_connect(&config.host, tcp),
                )
                .await
                .map_err(|_| PebbleError::Network("TLS handshake timed out (10s)".into()))?
                ?;
                report.push_str(&format!("TLS handshake (implicit): OK ({:.0}ms)\n", t1.elapsed().as_millis()));

                // Step 3: Read IMAP greeting
                let t2 = Instant::now();
                let mut buf = vec![0u8; 4096];
                let n = tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    tls.read(&mut buf),
                )
                .await
                .map_err(|_| PebbleError::Network("Read IMAP greeting timed out (10s)".into()))?
                .map_err(|e| PebbleError::Network(format!("Read greeting: {e}")))?;
                let greeting = String::from_utf8_lossy(&buf[..n]);
                report.push_str(&format!("IMAP greeting ({:.0}ms): {}\n", t2.elapsed().as_millis(), greeting.trim()));
            }
            ConnectionSecurity::StartTls => {
                // Read plain greeting first
                let mut tcp = tcp;
                let t1 = Instant::now();
                let mut buf = vec![0u8; 4096];
                let n = tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    tcp.read(&mut buf),
                )
                .await
                .map_err(|_| PebbleError::Network("Read plain greeting timed out (10s)".into()))?
                .map_err(|e| PebbleError::Network(format!("Read greeting: {e}")))?;
                let greeting = String::from_utf8_lossy(&buf[..n]);
                report.push_str(&format!("Plain greeting ({:.0}ms): {}\n", t1.elapsed().as_millis(), greeting.trim()));

                // Send STARTTLS
                let t2 = Instant::now();
                tcp.write_all(b"A001 STARTTLS\r\n").await
                    .map_err(|e| PebbleError::Network(format!("Send STARTTLS: {e}")))?;
                tcp.flush().await
                    .map_err(|e| PebbleError::Network(format!("Flush: {e}")))?;
                let mut resp = vec![0u8; 4096];
                let n = tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    tcp.read(&mut resp),
                )
                .await
                .map_err(|_| PebbleError::Network("STARTTLS response timed out (10s)".into()))?
                .map_err(|e| PebbleError::Network(format!("Read STARTTLS response: {e}")))?;
                let resp_str = String::from_utf8_lossy(&resp[..n]);
                report.push_str(&format!("STARTTLS response ({:.0}ms): {}\n", t2.elapsed().as_millis(), resp_str.trim()));

                // TLS upgrade
                let t3 = Instant::now();
                tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    tls_connect(&config.host, tcp),
                )
                .await
                .map_err(|_| PebbleError::Network("TLS upgrade timed out (10s)".into()))?
                ?;
                report.push_str(&format!("TLS upgrade (STARTTLS): OK ({:.0}ms)\n", t3.elapsed().as_millis()));
            }
            ConnectionSecurity::Plain => {
                // Read plain greeting
                let mut tcp = tcp;
                let t1 = Instant::now();
                let mut buf = vec![0u8; 4096];
                let n = tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    tcp.read(&mut buf),
                )
                .await
                .map_err(|_| PebbleError::Network("Read plain greeting timed out (10s)".into()))?
                .map_err(|e| PebbleError::Network(format!("Read greeting: {e}")))?;
                let greeting = String::from_utf8_lossy(&buf[..n]);
                report.push_str(&format!("Plain greeting ({:.0}ms): {}\n", t1.elapsed().as_millis(), greeting.trim()));
            }
        }

        report.push_str("Connection test: PASSED");
        Ok(report)
    }

    /// Test connection with login. Extends `test_connection` by also attempting LOGIN.
    pub async fn test_connection_with_login(config: &ImapConfig) -> Result<String> {
        // First do the basic connectivity test
        let mut report = Self::test_connection(config).await?;

        // Now try an actual IMAP login
        report.push_str("\n--- Login test ---\n");
        let provider = ImapProvider::new(config.clone());
        match provider.connect().await {
            Ok(()) => {
                report.push_str("LOGIN: OK\n");
                report.push_str("Authentication test: PASSED");
            }
            Err(e) => {
                report.push_str(&format!("LOGIN: FAILED — {e}\n"));
                return Err(PebbleError::Auth(format!("Authentication failed: {e}")));
            }
        }
        Ok(report)
    }

    /// List folders for the given account, returning `Folder` structs.
    pub async fn list_folders(&self, account_id: &str) -> Result<Vec<Folder>> {
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;

        let names: Vec<String> = {
            let stream = sess
                .list(None, Some("*"))
                .await
                .map_err(|e| PebbleError::Network(format!("LIST failed: {e}")))?;
            stream
                .map_ok(|n| n.name().to_string())
                .try_collect()
                .await
                .map_err(|e| PebbleError::Network(format!("LIST collect: {e}")))?
        };

        let mut folders: Vec<Folder> = names
            .into_iter()
            .map(|raw_name| {
                // Decode IMAP Modified UTF-7 folder name to UTF-8
                let display_name = utf7_imap::decode_utf7_imap(raw_name.clone());
                let role = detect_folder_role(&raw_name)
                    .or_else(|| detect_folder_role(&display_name));
                let sort_order = folder_sort_order(&role);
                Folder {
                    id: new_id(),
                    account_id: account_id.to_string(),
                    remote_id: raw_name,
                    name: display_name,
                    folder_type: FolderType::Folder,
                    role,
                    parent_id: None,
                    color: None,
                    is_system: true,
                    sort_order,
                }
            })
            .collect();

        folders.sort_by_key(|f| f.sort_order);
        Ok(folders)
    }

    /// Fetch raw message bytes from a mailbox.
    /// Returns a list of `(uid, raw_bytes)` pairs.
    pub async fn fetch_messages_raw(
        &self,
        mailbox: &str,
        since_uid: Option<u32>,
        limit: u32,
    ) -> Result<Vec<(u32, Vec<u8>)>> {
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;

        macro_rules! do_fetch {
            ($s:expr) => {{
                let mailbox_info = $s
                    .select(mailbox)
                    .await
                    .map_err(|e| PebbleError::Network(format!("SELECT failed: {e}")))?;

                let exists = mailbox_info.exists;
                if exists == 0 {
                    return Ok(Vec::new());
                }

                let mut results = Vec::new();

                if let Some(uid) = since_uid {
                    let next_uid = match uid.checked_add(1) {
                        Some(n) => n,
                        None => return Ok(Vec::new()),
                    };
                    let uid_set = format!("{next_uid}:*");
                    let fetches: Vec<async_imap::types::Fetch> = $s
                        .uid_fetch(&uid_set, "(UID BODY.PEEK[])")
                        .await
                        .map_err(|e| PebbleError::Network(format!("UID FETCH failed: {e}")))?
                        .try_collect()
                        .await
                        .map_err(|e| PebbleError::Network(format!("UID FETCH collect: {e}")))?;
                    for fetch in fetches {
                        if let Some(uid) = fetch.uid {
                            if let Some(body) = fetch.body() {
                                results.push((uid, body.to_vec()));
                            }
                        } else {
                            tracing::warn!("Skipping message without UID (seq={})", fetch.message);
                        }
                    }
                } else {
                    let start = if exists > limit { exists - limit + 1 } else { 1 };
                    let seq_set = format!("{start}:{exists}");
                    let fetches: Vec<async_imap::types::Fetch> = $s
                        .fetch(&seq_set, "(UID BODY.PEEK[])")
                        .await
                        .map_err(|e| PebbleError::Network(format!("FETCH failed: {e}")))?
                        .try_collect()
                        .await
                        .map_err(|e| PebbleError::Network(format!("FETCH collect: {e}")))?;
                    for fetch in fetches {
                        if let Some(uid) = fetch.uid {
                            if let Some(body) = fetch.body() {
                                results.push((uid, body.to_vec()));
                            }
                        } else {
                            tracing::warn!("Skipping message without UID (seq={})", fetch.message);
                        }
                    }
                }

                results
            }};
        }

        let results = do_fetch!(sess);

        Ok(results)
    }

    /// Fetch flags for a set of UIDs. Returns `(uid, is_read, is_starred)`.
    pub async fn fetch_flags(
        &self,
        mailbox: &str,
        uids: &[u32],
    ) -> Result<Vec<(u32, bool, bool)>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }

        let uid_set: String = uids
            .iter()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(",");

        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;

        macro_rules! do_flags {
            ($s:expr) => {{
                $s.select(mailbox)
                    .await
                    .map_err(|e| PebbleError::Network(format!("SELECT failed: {e}")))?;

                let fetches: Vec<async_imap::types::Fetch> = $s
                    .uid_fetch(&uid_set, "FLAGS")
                    .await
                    .map_err(|e| PebbleError::Network(format!("UID FETCH FLAGS failed: {e}")))?
                    .try_collect()
                    .await
                    .map_err(|e| PebbleError::Network(format!("FLAGS collect: {e}")))?;

                fetches
                    .into_iter()
                    .filter_map(|fetch| {
                        let uid = fetch.uid.or_else(|| {
                            tracing::warn!("Skipping flags for message without UID (seq={})", fetch.message);
                            None
                        })?;
                        let (is_read, is_starred) = parse_flags(fetch.flags());
                        Some((uid, is_read, is_starred))
                    })
                    .collect::<Vec<_>>()
            }};
        }

        let results = do_flags!(sess);
        Ok(results)
    }

    /// Set flags on a message identified by UID.
    pub async fn set_flags(
        &self,
        mailbox: &str,
        uid: u32,
        is_read: Option<bool>,
        is_starred: Option<bool>,
    ) -> Result<()> {
        let uid_str = uid.to_string();

        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;

        macro_rules! do_store {
            ($s:expr) => {{
                $s.select(mailbox)
                    .await
                    .map_err(|e| PebbleError::Network(format!("SELECT failed: {e}")))?;

                if let Some(read) = is_read {
                    let flag_cmd = if read {
                        format!("+FLAGS (\\Seen)")
                    } else {
                        format!("-FLAGS (\\Seen)")
                    };
                    let _: Vec<async_imap::types::Fetch> = $s
                        .uid_store(&uid_str, &flag_cmd)
                        .await
                        .map_err(|e| {
                            PebbleError::Network(format!("STORE \\Seen failed: {e}"))
                        })?
                        .try_collect()
                        .await
                        .map_err(|e| {
                            PebbleError::Network(format!("STORE \\Seen collect: {e}"))
                        })?;
                }

                if let Some(starred) = is_starred {
                    let flag_cmd = if starred {
                        format!("+FLAGS (\\Flagged)")
                    } else {
                        format!("-FLAGS (\\Flagged)")
                    };
                    let _: Vec<async_imap::types::Fetch> = $s
                        .uid_store(&uid_str, &flag_cmd)
                        .await
                        .map_err(|e| {
                            PebbleError::Network(format!("STORE \\Flagged failed: {e}"))
                        })?
                        .try_collect()
                        .await
                        .map_err(|e| {
                            PebbleError::Network(format!("STORE \\Flagged collect: {e}"))
                        })?;
                }
            }};
        }

        do_store!(sess);

        Ok(())
    }

    /// Move a message by UID from one mailbox to another.
    ///
    /// Tries IMAP MOVE (uid_mv) first, falls back to UID COPY + UID STORE \Deleted + EXPUNGE.
    pub async fn move_message(
        &self,
        source_mailbox: &str,
        uid: u32,
        dest_mailbox: &str,
    ) -> Result<()> {
        let uid_str = uid.to_string();

        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;

        macro_rules! do_move {
            ($s:expr) => {{
                $s.select(source_mailbox)
                    .await
                    .map_err(|e| PebbleError::Network(format!("SELECT failed: {e}")))?;

                // Try MOVE extension first
                match $s.uid_mv(&uid_str, dest_mailbox).await {
                    Ok(_) => {
                        debug!("MOVE UID {} from {} to {} succeeded", uid, source_mailbox, dest_mailbox);
                    }
                    Err(_move_err) => {
                        // Fallback: COPY + flag Deleted + EXPUNGE
                        debug!("MOVE not supported, falling back to COPY+DELETE for UID {}", uid);

                        // Re-select in case MOVE attempt changed state
                        $s.select(source_mailbox)
                            .await
                            .map_err(|e| PebbleError::Network(format!("SELECT failed: {e}")))?;

                        $s.uid_copy(&uid_str, dest_mailbox)
                            .await
                            .map_err(|e| PebbleError::Network(format!("UID COPY failed: {e}")))?;

                        let _: Vec<async_imap::types::Fetch> = $s
                            .uid_store(&uid_str, "+FLAGS (\\Deleted)")
                            .await
                            .map_err(|e| PebbleError::Network(format!("STORE \\Deleted failed: {e}")))?
                            .try_collect()
                            .await
                            .map_err(|e| PebbleError::Network(format!("STORE \\Deleted collect: {e}")))?;

                        let _: Vec<u32> = $s
                            .expunge()
                            .await
                            .map_err(|e| PebbleError::Network(format!("EXPUNGE failed: {e}")))?
                            .try_collect()
                            .await
                            .map_err(|e| PebbleError::Network(format!("EXPUNGE collect: {e}")))?;

                        debug!("COPY+DELETE UID {} from {} to {} succeeded", uid, source_mailbox, dest_mailbox);
                    }
                }
            }};
        }

        do_move!(sess);

        Ok(())
    }

    /// Delete a message by UID: flag as \Deleted and EXPUNGE.
    pub async fn delete_message(&self, mailbox: &str, uid: u32) -> Result<()> {
        let uid_str = uid.to_string();

        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;

        macro_rules! do_delete {
            ($s:expr) => {{
                $s.select(mailbox)
                    .await
                    .map_err(|e| PebbleError::Network(format!("SELECT failed: {e}")))?;

                let _: Vec<async_imap::types::Fetch> = $s
                    .uid_store(&uid_str, "+FLAGS (\\Deleted)")
                    .await
                    .map_err(|e| PebbleError::Network(format!("STORE \\Deleted failed: {e}")))?
                    .try_collect()
                    .await
                    .map_err(|e| PebbleError::Network(format!("STORE \\Deleted collect: {e}")))?;

                let _: Vec<u32> = $s
                    .expunge()
                    .await
                    .map_err(|e| PebbleError::Network(format!("EXPUNGE failed: {e}")))?
                    .try_collect()
                    .await
                    .map_err(|e| PebbleError::Network(format!("EXPUNGE collect: {e}")))?;

                debug!("Deleted UID {} from {}", uid, mailbox);
            }};
        }

        do_delete!(sess);

        Ok(())
    }

    /// Fetch all UIDs in a mailbox via UID SEARCH ALL.
    pub async fn fetch_all_uids(&self, mailbox: &str) -> Result<Vec<u32>> {
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;

        macro_rules! do_search {
            ($s:expr) => {{
                $s.select(mailbox)
                    .await
                    .map_err(|e| PebbleError::Network(format!("SELECT failed: {e}")))?;

                let uids: Vec<u32> = $s
                    .uid_search("ALL")
                    .await
                    .map_err(|e| PebbleError::Network(format!("UID SEARCH ALL failed: {e}")))?
                    .into_iter()
                    .collect();
                uids
            }};
        }

        let results = do_search!(sess);

        Ok(results)
    }

    /// SELECT a mailbox and return the EXISTS count without fetching UIDs.
    pub async fn select_exists(&self, mailbox: &str) -> Result<u32> {
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;

        let mbox = sess
            .select(mailbox)
            .await
            .map_err(|e| PebbleError::Network(format!("SELECT failed: {e}")))?;

        Ok(mbox.exists)
    }

    /// Check if the server advertises the CONDSTORE capability (RFC 4551).
    pub async fn supports_condstore(&self) -> bool {
        let mut guard = self.session.lock().await;
        let sess = match guard.as_mut() {
            Some(s) => s,
            None => return false,
        };

        match sess.capabilities().await {
            Ok(caps) => caps.has_str("CONDSTORE"),
            Err(_) => false,
        }
    }

    /// SELECT a mailbox and return the HIGHESTMODSEQ value if the server supports CONDSTORE.
    /// Returns `Ok(Some(modseq))` if available, `Ok(None)` otherwise.
    pub async fn get_highest_modseq(&self, mailbox: &str) -> Result<Option<u64>> {
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;

        macro_rules! do_select {
            ($s:expr) => {{
                let mailbox_info = $s
                    .select(mailbox)
                    .await
                    .map_err(|e| PebbleError::Network(format!("SELECT failed: {e}")))?;
                mailbox_info.highest_modseq
            }};
        }

        let result = do_select!(sess);

        Ok(result)
    }

    /// Fetch flags for a set of UIDs along with per-message MODSEQ values.
    /// Returns `(flags_vec, highest_modseq)` where highest_modseq is the maximum
    /// MODSEQ seen across all fetched messages (or 0 if the server did not return any).
    pub async fn fetch_flags_with_modseq(
        &self,
        mailbox: &str,
        uids: &[u32],
    ) -> Result<(Vec<(u32, bool, bool)>, u64)> {
        if uids.is_empty() {
            return Ok((Vec::new(), 0));
        }

        let uid_set: String = uids
            .iter()
            .map(|u| u.to_string())
            .collect::<Vec<_>>()
            .join(",");

        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;

        macro_rules! do_flags_modseq {
            ($s:expr) => {{
                $s.select(mailbox)
                    .await
                    .map_err(|e| PebbleError::Network(format!("SELECT failed: {e}")))?;

                let fetches: Vec<async_imap::types::Fetch> = $s
                    .uid_fetch(&uid_set, "(FLAGS MODSEQ)")
                    .await
                    .map_err(|e| {
                        PebbleError::Network(format!("UID FETCH FLAGS MODSEQ failed: {e}"))
                    })?
                    .try_collect()
                    .await
                    .map_err(|e| {
                        PebbleError::Network(format!("FLAGS MODSEQ collect: {e}"))
                    })?;

                let mut highest = 0u64;
                let results: Vec<(u32, bool, bool)> = fetches
                    .into_iter()
                    .filter_map(|fetch| {
                        let uid = fetch.uid.or_else(|| {
                            tracing::warn!("Skipping modseq flags for message without UID (seq={})", fetch.message);
                            None
                        })?;
                        if let Some(ms) = fetch.modseq {
                            if ms > highest {
                                highest = ms;
                            }
                        }
                        let (is_read, is_starred) = parse_flags(fetch.flags());
                        Some((uid, is_read, is_starred))
                    })
                    .collect();

                (results, highest)
            }};
        }

        let results = do_flags_modseq!(sess);

        Ok(results)
    }

    /// Check if the server advertises the IDLE capability (RFC 2177).
    pub async fn supports_idle(&self) -> bool {
        let mut guard = self.session.lock().await;
        let sess = match guard.as_mut() {
            Some(s) => s,
            None => return false,
        };

        match sess.capabilities().await {
            Ok(caps) => caps.has_str("IDLE"),
            Err(_) => false,
        }
    }

    /// Enter IMAP IDLE mode and wait for server notifications or timeout.
    ///
    /// The session is temporarily taken out of `self.session` while IDLE is
    /// active and restored when the command completes (or on error).
    /// Timeout should be <= 29 minutes per RFC 2177 recommendation.
    pub async fn idle_wait(
        &self,
        mailbox: &str,
        timeout_dur: std::time::Duration,
    ) -> Result<super::idle::IdleEvent> {
        // Take the session out so we can pass ownership to the idle handle.
        let sess = {
            let mut guard = self.session.lock().await;
            guard
                .take()
                .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?
        };

        // Select the mailbox first.
        let mut session = sess;
        if let Err(e) = session.select(mailbox).await {
            // Restore session before returning error.
            let mut guard = self.session.lock().await;
            *guard = Some(session);
            return Err(PebbleError::Network(format!("SELECT failed: {e}")));
        }

        let mut idle_handle = session.idle();
        if let Err(e) = idle_handle.init().await {
            // init() failed; the handle still owns the session.
            // Call done() to recover the session.
            match idle_handle.done().await {
                Ok(recovered) => {
                    let mut guard = self.session.lock().await;
                    *guard = Some(recovered);
                }
                Err(_) => {
                    // Session is lost; caller will need to reconnect.
                }
            }
            return Err(PebbleError::Network(format!("IDLE init failed: {e}")));
        }

        let (wait_fut, _stop_source) = idle_handle.wait_with_timeout(timeout_dur);
        let idle_result = wait_fut.await;

        // Recover the session by sending DONE.
        let event = match idle_result {
            Ok(resp) => {
                use async_imap::extensions::idle::IdleResponse;
                match resp {
                    IdleResponse::NewData(_) => super::idle::IdleEvent::NewMail,
                    IdleResponse::Timeout => super::idle::IdleEvent::Timeout,
                    IdleResponse::ManualInterrupt => super::idle::IdleEvent::Timeout,
                }
            }
            Err(e) => super::idle::IdleEvent::Error(format!("IDLE wait error: {e}")),
        };

        match idle_handle.done().await {
            Ok(recovered) => {
                let mut guard = self.session.lock().await;
                *guard = Some(recovered);
            }
            Err(_) => {
                // Session is lost; caller will need to reconnect.
                tracing::warn!("Failed to recover session after IDLE DONE");
            }
        }

        Ok(event)
    }

    /// Disconnect from the IMAP server.
    pub async fn disconnect(&self) -> Result<()> {
        let mut guard = self.session.lock().await;
        if let Some(sess) = guard.as_mut() {
            let _ = sess.logout().await;
            *guard = None;
        }
        Ok(())
    }
}

/// Parse flags from an iterator of `Flag` values.
fn parse_flags<'a>(flags: impl Iterator<Item = async_imap::types::Flag<'a>>) -> (bool, bool) {
    let mut is_read = false;
    let mut is_starred = false;
    for flag in flags {
        match flag {
            async_imap::types::Flag::Seen => is_read = true,
            async_imap::types::Flag::Flagged => is_starred = true,
            _ => {}
        }
    }
    (is_read, is_starred)
}

/// Detect a folder role based on its name.
pub fn detect_folder_role(name: &str) -> Option<FolderRole> {
    let lower = name.to_lowercase();
    // Check last component after hierarchy separator
    let leaf = lower.rsplit('/').next().unwrap_or(&lower);
    let leaf = leaf.rsplit('.').next().unwrap_or(leaf);

    if leaf == "inbox" || leaf == "收件箱" {
        Some(FolderRole::Inbox)
    } else if leaf.contains("sent") || leaf.contains("已发送") || leaf.contains("已发件") {
        Some(FolderRole::Sent)
    } else if leaf.contains("draft") || leaf.contains("草稿") {
        Some(FolderRole::Drafts)
    } else if leaf.contains("trash") || leaf.contains("deleted") || leaf.contains("已删除") || leaf.contains("废纸篓") {
        Some(FolderRole::Trash)
    } else if leaf.contains("archive") || leaf.contains("归档") || leaf.contains("存档") {
        Some(FolderRole::Archive)
    } else if leaf.contains("spam") || leaf.contains("junk") || leaf.contains("垃圾") || leaf.contains("病毒") || leaf.contains("广告") {
        Some(FolderRole::Spam)
    } else {
        None
    }
}

/// Sort order for folder roles.
pub fn folder_sort_order(role: &Option<FolderRole>) -> i32 {
    match role {
        Some(FolderRole::Inbox) => 0,
        Some(FolderRole::Drafts) => 1,
        Some(FolderRole::Sent) => 2,
        Some(FolderRole::Archive) => 3,
        Some(FolderRole::Spam) => 4,
        Some(FolderRole::Trash) => 5,
        None => 100,
    }
}
