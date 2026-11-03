use std::collections::HashSet;
use std::fmt::Display;
use std::future::Future;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use async_imap::{types::NameAttribute, Client};
use futures::TryStreamExt;
use pebble_core::{new_id, Folder, FolderRole, FolderType, PebbleError, Result};
use serde::de::Deserializer;
use tokio::io::{self, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tokio_native_tls as async_native_tls;
use tokio_rustls::client::TlsStream;
use tracing::debug;

use rustls::{
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    crypto::{verify_tls12_signature, verify_tls13_signature, CryptoProvider},
    pki_types::{CertificateDer, ServerName, UnixTime},
    server::ParsedCertificate,
    DigitallySignedStruct, Error as TlsError, SignatureScheme,
};

/// Connection security mode for mail protocols.
#[derive(Debug, Clone, Default, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConnectionSecurity {
    /// Implicit TLS — connect over TLS immediately (IMAP 993, SMTP 465).
    #[default]
    Tls,
    /// STARTTLS — connect plain then upgrade to TLS (IMAP 143, SMTP 587).
    #[serde(rename = "starttls")]
    StartTls,
    /// No encryption (not recommended).
    Plain,
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
    #[serde(default, skip_serializing_if = "is_false")]
    pub accept_invalid_certs: bool,
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
            .field("accept_invalid_certs", &self.accept_invalid_certs)
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
            accept_invalid_certs: bool,
            #[serde(default)]
            proxy: Option<ProxyConfig>,
        }

        let raw = Raw::deserialize(deserializer)?;
        let security = raw.security.unwrap_or(match raw.use_tls {
            Some(false) => ConnectionSecurity::Plain,
            _ => ConnectionSecurity::Tls,
        });

        Ok(ImapConfig {
            host: raw.host,
            port: raw.port,
            username: raw.username,
            password: raw.password,
            security,
            accept_invalid_certs: raw.accept_invalid_certs,
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
    #[serde(default, skip_serializing_if = "is_false")]
    pub accept_invalid_certs: bool,
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
            .field("accept_invalid_certs", &self.accept_invalid_certs)
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
            accept_invalid_certs: bool,
            #[serde(default)]
            proxy: Option<ProxyConfig>,
        }

        let raw = Raw::deserialize(deserializer)?;
        let security = raw.security.unwrap_or(match raw.use_tls {
            Some(false) => ConnectionSecurity::Plain,
            _ => ConnectionSecurity::Tls,
        });

        Ok(SmtpConfig {
            host: raw.host,
            port: raw.port,
            username: raw.username,
            password: raw.password,
            security,
            accept_invalid_certs: raw.accept_invalid_certs,
            proxy: raw.proxy,
        })
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}

fn incremental_fetch_uids(since_uid: Option<u32>, searched_uids: &[u32]) -> Vec<u32> {
    let Some(since_uid) = since_uid else {
        return searched_uids.to_vec();
    };
    let mut uids = searched_uids
        .iter()
        .copied()
        .filter(|uid| *uid > since_uid)
        .collect::<Vec<_>>();
    uids.sort_unstable();
    uids.dedup();
    uids
}

fn should_search_incremental_uids(next_uid: u32, mailbox_uid_next: Option<u32>) -> bool {
    mailbox_uid_next.is_none_or(|uid_next| next_uid < uid_next)
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
        Self {
            prefix,
            pos: 0,
            inner,
        }
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
    fn poll_write(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
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
    Tls(Box<TlsStream<TcpStream>>),
    NativeTls(Box<async_native_tls::TlsStream<TcpStream>>),
    Plain(TcpStream),
}

impl std::fmt::Debug for InnerStream {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            InnerStream::Tls(_) => f.debug_struct("InnerStream::Tls").finish(),
            InnerStream::NativeTls(_) => f.debug_struct("InnerStream::NativeTls").finish(),
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
            InnerStream::Tls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
            InnerStream::NativeTls(s) => Pin::new(s.as_mut()).poll_read(cx, buf),
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
            InnerStream::Tls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
            InnerStream::NativeTls(s) => Pin::new(s.as_mut()).poll_write(cx, buf),
            InnerStream::Plain(s) => Pin::new(s).poll_write(cx, buf),
        }
    }

    fn poll_flush(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            InnerStream::Tls(s) => Pin::new(s.as_mut()).poll_flush(cx),
            InnerStream::NativeTls(s) => Pin::new(s.as_mut()).poll_flush(cx),
            InnerStream::Plain(s) => Pin::new(s).poll_flush(cx),
        }
    }

    fn poll_shutdown(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        match self.get_mut() {
            InnerStream::Tls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
            InnerStream::NativeTls(s) => Pin::new(s.as_mut()).poll_shutdown(cx),
            InnerStream::Plain(s) => Pin::new(s).poll_shutdown(cx),
        }
    }
}

/// Unified IMAP session type — a single `async_imap::Session` regardless
/// of whether the underlying transport is TLS or plain TCP.
type ImapSession = async_imap::Session<PrefixedStream<InnerStream>>;

const IMAP_CONNECT_TIMEOUT_SECS: u64 = 15;
const IMAP_COMMAND_TIMEOUT_SECS: u64 = 45;

fn imap_timeout_error(operation: &str, seconds: u64) -> PebbleError {
    PebbleError::Network(format!("{operation} timed out after {seconds}s"))
}

async fn with_imap_timeout<T, E, Fut>(operation: &str, seconds: u64, future: Fut) -> Result<T>
where
    E: Display,
    Fut: Future<Output = std::result::Result<T, E>>,
{
    match tokio::time::timeout(Duration::from_secs(seconds), future).await {
        Ok(Ok(value)) => Ok(value),
        Ok(Err(error)) => Err(PebbleError::Network(format!("{operation} failed: {error}"))),
        Err(_) => Err(imap_timeout_error(operation, seconds)),
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ImapMailboxStatus {
    pub uid_validity: Option<u32>,
    pub highest_modseq: Option<u64>,
    pub uid_next: Option<u32>,
    /// Number of messages in the mailbox as reported by SELECT.
    pub exists: u32,
}

fn imap_since_date(timestamp: i64) -> String {
    let (year, month, day) = pebble_core::utc_date_from_unix_seconds(timestamp);
    let month = match month {
        1 => "Jan",
        2 => "Feb",
        3 => "Mar",
        4 => "Apr",
        5 => "May",
        6 => "Jun",
        7 => "Jul",
        8 => "Aug",
        9 => "Sep",
        10 => "Oct",
        11 => "Nov",
        _ => "Dec",
    };
    format!("{day:02}-{month}-{year:04}")
}

fn imap_since_search_criteria(cutoff: i64, since_uid: Option<u32>) -> Option<String> {
    let mut criteria = format!("SINCE {}", imap_since_date(cutoff));
    if let Some(uid) = since_uid {
        let next_uid = uid.checked_add(1)?;
        criteria.push_str(&format!(" UID {next_uid}:*"));
    }
    Some(criteria)
}

/// An IMAP provider that manages a connection and session.
pub struct ImapProvider {
    config: ImapConfig,
    session: Arc<Mutex<Option<ImapSession>>>,
}

/// Build a rustls TLS connector with bundled root certificates.
fn build_tls_connector(accept_invalid_certs: bool) -> Result<tokio_rustls::TlsConnector> {
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());
    let builder = rustls::ClientConfig::builder_with_provider(provider.clone())
        .with_safe_default_protocol_versions()
        .map_err(|e| PebbleError::Network(format!("TLS protocol versions: {e}")))?;
    let config = if accept_invalid_certs {
        builder
            .dangerous()
            .with_custom_certificate_verifier(Arc::new(AcceptInvalidCertsVerifier {
                roots: root_store,
                provider,
            }))
            .with_no_client_auth()
    } else {
        builder
            .with_root_certificates(root_store)
            .with_no_client_auth()
    };
    Ok(tokio_rustls::TlsConnector::from(Arc::new(config)))
}

/// Perform a TLS handshake using rustls on the given TCP stream.
async fn tls_connect(
    host: &str,
    tcp: TcpStream,
    accept_invalid_certs: bool,
) -> Result<TlsStream<TcpStream>> {
    let connector = build_tls_connector(accept_invalid_certs)?;
    let server_name = rustls::pki_types::ServerName::try_from(host)
        .map_err(|e| PebbleError::Network(format!("Invalid server name '{}': {}", host, e)))?
        .to_owned();
    with_imap_timeout(
        &format!("TLS handshake with {host}"),
        IMAP_CONNECT_TIMEOUT_SECS,
        connector.connect(server_name, tcp),
    )
    .await
}

/// Build a native-tls connector (delegates to OS TLS: SChannel/SecureTransport/OpenSSL).
fn build_native_tls_connector(
    accept_invalid_certs: bool,
) -> Result<async_native_tls::TlsConnector> {
    let mut builder = native_tls::TlsConnector::builder();
    if accept_invalid_certs {
        builder.danger_accept_invalid_certs(true);
        builder.danger_accept_invalid_hostnames(true);
        builder.min_protocol_version(Some(native_tls::Protocol::Tlsv10));
    }
    let connector = builder
        .build()
        .map_err(|e| PebbleError::Network(format!("native-tls init: {e}")))?;
    Ok(async_native_tls::TlsConnector::from(connector))
}

/// Perform a TLS handshake using native-tls (OS TLS backend) on the given TCP stream.
/// Used as fallback when rustls fails (e.g. servers that only offer DHE cipher suites).
async fn native_tls_connect(
    host: &str,
    tcp: TcpStream,
    accept_invalid_certs: bool,
) -> Result<async_native_tls::TlsStream<TcpStream>> {
    let connector = build_native_tls_connector(accept_invalid_certs)?;
    with_imap_timeout(
        &format!("TLS handshake (native-tls) with {host}"),
        IMAP_CONNECT_TIMEOUT_SECS,
        connector.connect(host, tcp),
    )
    .await
}

#[derive(Debug)]
pub(crate) struct AcceptInvalidCertsVerifier {
    roots: rustls::RootCertStore,
    provider: Arc<CryptoProvider>,
}

impl AcceptInvalidCertsVerifier {
    pub(crate) fn new(roots: rustls::RootCertStore) -> Self {
        Self {
            roots,
            provider: Arc::new(rustls::crypto::aws_lc_rs::default_provider()),
        }
    }
}

impl ServerCertVerifier for AcceptInvalidCertsVerifier {
    fn verify_server_cert(
        &self,
        end_entity: &CertificateDer<'_>,
        intermediates: &[CertificateDer<'_>],
        server_name: &ServerName<'_>,
        _ocsp_response: &[u8],
        now: UnixTime,
    ) -> std::result::Result<ServerCertVerified, TlsError> {
        if !self.roots.is_empty() {
            if let Ok(cert) = ParsedCertificate::try_from(end_entity) {
                let _ = rustls::client::verify_server_cert_signed_by_trust_anchor(
                    &cert,
                    &self.roots,
                    intermediates,
                    now,
                    self.provider.signature_verification_algorithms.all,
                );
                let _ = rustls::client::verify_server_name(&cert, server_name);
            }
        }
        Ok(ServerCertVerified::assertion())
    }

    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        verify_tls12_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> std::result::Result<HandshakeSignatureValid, TlsError> {
        verify_tls13_signature(
            message,
            cert,
            dss,
            &self.provider.signature_verification_algorithms,
        )
    }

    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.provider
            .signature_verification_algorithms
            .supported_schemes()
    }
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
    /// (Coremail-based servers reject as "Unsafe Login" without it).
    fn needs_id_command(&self) -> bool {
        let h = self.config.host.to_lowercase();
        h.contains("163.com")
            || h.contains("126.com")
            || h.contains("188.com")
            || h.contains("yeah.net")
            || h.contains("netease.com")
            || h.contains("sina.com")
            || h.contains("sina.cn")
            || h.contains("qq.com")
            || h.contains("exmail.qq.com")
            || h.contains("tencent.com")
    }

    /// Send IMAP ID command on a raw stream, returning the greeting bytes
    /// so they can be replayed for `Client::new()`.
    async fn send_id_before_login<S: AsyncRead + AsyncWrite + Unpin>(
        stream: &mut S,
    ) -> Result<Vec<u8>> {
        // Read server greeting (e.g. "* OK Coremail ...")
        let mut greeting = vec![0u8; 8192];
        let n = with_imap_timeout(
            "Read greeting",
            IMAP_COMMAND_TIMEOUT_SECS,
            stream.read(&mut greeting),
        )
        .await?;
        greeting.truncate(n);

        // Send ID command
        with_imap_timeout(
            "Send ID",
            IMAP_COMMAND_TIMEOUT_SECS,
            stream.write_all(
                b"A000 ID (\"name\" \"Pebble\" \"version\" \"1.0\" \"vendor\" \"Pebble\")\r\n",
            ),
        )
        .await?;
        with_imap_timeout("Flush ID", IMAP_COMMAND_TIMEOUT_SECS, stream.flush()).await?;

        // Read ID response until we see the tagged response (A000 OK/NO/BAD)
        let mut resp = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = with_imap_timeout(
                "Read ID response",
                IMAP_COMMAND_TIMEOUT_SECS,
                stream.read(&mut buf),
            )
            .await?;
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

    /// Send STARTTLS command and read the server response.
    /// Returns the TCP stream ready for TLS upgrade.
    async fn negotiate_starttls(mut tcp: TcpStream) -> Result<TcpStream> {
        with_imap_timeout(
            "Send STARTTLS",
            IMAP_COMMAND_TIMEOUT_SECS,
            tcp.write_all(b"A001 STARTTLS\r\n"),
        )
        .await?;
        with_imap_timeout("Flush STARTTLS", IMAP_COMMAND_TIMEOUT_SECS, tcp.flush()).await?;

        let mut resp = Vec::new();
        let mut buf = [0u8; 4096];
        loop {
            let n = with_imap_timeout(
                "Read STARTTLS response",
                IMAP_COMMAND_TIMEOUT_SECS,
                tcp.read(&mut buf),
            )
            .await?;
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
        Ok(tcp)
    }

    /// Connect via STARTTLS: read greeting, optionally send ID, negotiate
    /// STARTTLS, upgrade to TLS (rustls or native-tls), then login.
    async fn connect_starttls(
        &self,
        tcp: TcpStream,
        needs_id: bool,
        use_native_tls: bool,
    ) -> Result<ImapSession> {
        let mut tcp = tcp;

        // Read greeting
        let mut greeting = vec![0u8; 8192];
        let n = with_imap_timeout(
            "Read greeting",
            IMAP_COMMAND_TIMEOUT_SECS,
            tcp.read(&mut greeting),
        )
        .await?;
        greeting.truncate(n);

        // Send ID command before STARTTLS if needed (on plain connection)
        if needs_id {
            with_imap_timeout(
                "Send ID",
                IMAP_COMMAND_TIMEOUT_SECS,
                tcp.write_all(
                    b"A000 ID (\"name\" \"Pebble\" \"version\" \"1.0\" \"vendor\" \"Pebble\")\r\n",
                ),
            )
            .await?;
            with_imap_timeout("Flush ID", IMAP_COMMAND_TIMEOUT_SECS, tcp.flush()).await?;

            let mut resp = Vec::new();
            let mut buf = [0u8; 4096];
            loop {
                let n = with_imap_timeout(
                    "Read ID response",
                    IMAP_COMMAND_TIMEOUT_SECS,
                    tcp.read(&mut buf),
                )
                .await?;
                if n == 0 {
                    return Err(PebbleError::Network("Connection closed during ID".into()));
                }
                resp.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&resp);
                if text.contains("A000 OK") || text.contains("A000 NO") || text.contains("A000 BAD")
                {
                    break;
                }
            }
            debug!("IMAP ID command accepted (pre-STARTTLS)");
        }

        // STARTTLS negotiation
        let tcp = Self::negotiate_starttls(tcp).await?;

        // TLS upgrade
        let inner = if use_native_tls {
            let tls = native_tls_connect(&self.config.host, tcp, self.config.accept_invalid_certs)
                .await?;
            InnerStream::NativeTls(Box::new(tls))
        } else {
            let tls = tls_connect(&self.config.host, tcp, self.config.accept_invalid_certs).await?;
            InnerStream::Tls(Box::new(tls))
        };

        // Replay the original greeting so Client::new() is happy
        let stream = PrefixedStream::with_prefix(greeting, inner);
        let client = Client::new(stream);
        tokio::time::timeout(
            Duration::from_secs(IMAP_COMMAND_TIMEOUT_SECS),
            client.login(&self.config.username, &self.config.password),
        )
        .await
        .map_err(|_| imap_timeout_error("IMAP login", IMAP_COMMAND_TIMEOUT_SECS))?
        .map_err(|(e, _)| PebbleError::Auth(format!("IMAP login failed: {e}")))
    }

    /// Establish a TCP connection, optionally through a SOCKS5 proxy.
    async fn tcp_connect(&self) -> Result<TcpStream> {
        let addr = format!("{}:{}", self.config.host, self.config.port);

        let tcp = if let Some(ref proxy) = self.config.proxy {
            let proxy_addr = format!("{}:{}", proxy.host, proxy.port);
            debug!(
                "Connecting to {} via SOCKS5 proxy {} (security={:?})...",
                addr, proxy_addr, self.config.security
            );
            let stream = with_imap_timeout(
                &format!("SOCKS5 proxy connect to {addr} via {proxy_addr}"),
                IMAP_CONNECT_TIMEOUT_SECS,
                tokio_socks::tcp::Socks5Stream::connect(proxy_addr.as_str(), addr.as_str()),
            )
            .await?;
            let tcp = stream.into_inner();
            if let Ok(peer) = tcp.peer_addr() {
                debug!("SOCKS5 connected to {} (proxy peer: {})", addr, peer);
            }
            tcp
        } else {
            debug!(
                "Resolving and connecting to {} (security={:?})...",
                addr, self.config.security
            );
            let tcp = with_imap_timeout(
                &format!("TCP connect to {addr}"),
                IMAP_CONNECT_TIMEOUT_SECS,
                TcpStream::connect(&addr),
            )
            .await?;
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
                // Implicit TLS — wrap immediately; try rustls, fall back to native-tls
                debug!("Starting TLS handshake with SNI={}", self.config.host);
                let inner = match tls_connect(
                    &self.config.host,
                    tcp,
                    self.config.accept_invalid_certs,
                )
                .await
                {
                    Ok(tls) => InnerStream::Tls(Box::new(tls)),
                    Err(rustls_err) => {
                        debug!(
                            "rustls handshake failed ({}), retrying with native-tls",
                            rustls_err
                        );
                        let tcp = self.tcp_connect().await?;
                        let tls = native_tls_connect(
                            &self.config.host,
                            tcp,
                            self.config.accept_invalid_certs,
                        )
                            .await
                            .map_err(|e| {
                                PebbleError::Network(format!(
                                    "TLS failed with both backends — rustls: {rustls_err}, native-tls: {e}"
                                ))
                            })?;
                        InnerStream::NativeTls(Box::new(tls))
                    }
                };

                let mut inner = inner;
                let prefix = if needs_id {
                    Self::send_id_before_login(&mut inner).await?
                } else {
                    Vec::new()
                };
                let stream = PrefixedStream::with_prefix(prefix, inner);

                let client = Client::new(stream);
                tokio::time::timeout(
                    Duration::from_secs(IMAP_COMMAND_TIMEOUT_SECS),
                    client.login(&self.config.username, &self.config.password),
                )
                .await
                .map_err(|_| imap_timeout_error("IMAP login", IMAP_COMMAND_TIMEOUT_SECS))?
                .map_err(|(e, _)| PebbleError::Auth(format!("IMAP login failed: {e}")))?
            }
            ConnectionSecurity::StartTls => {
                // Connect plain, read greeting, optionally send ID, STARTTLS, upgrade TLS.
                // Try rustls first, fall back to native-tls on handshake failure.
                match self.connect_starttls(tcp, needs_id, false).await {
                    Ok(session) => session,
                    Err(rustls_err) => {
                        debug!(
                            "STARTTLS with rustls failed ({}), retrying with native-tls",
                            rustls_err
                        );
                        let tcp = self.tcp_connect().await?;
                        self.connect_starttls(tcp, needs_id, true)
                            .await
                            .map_err(|native_err| {
                                PebbleError::Network(format!(
                                    "STARTTLS failed with both TLS backends — rustls: {rustls_err}, native-tls: {native_err}"
                                ))
                            })?
                    }
                }
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
                tokio::time::timeout(
                    Duration::from_secs(IMAP_COMMAND_TIMEOUT_SECS),
                    client.login(&self.config.username, &self.config.password),
                )
                .await
                .map_err(|_| imap_timeout_error("IMAP login", IMAP_COMMAND_TIMEOUT_SECS))?
                .map_err(|(e, _)| PebbleError::Auth(format!("IMAP login failed: {e}")))?
            }
        };

        let mut guard = self.session.lock().await;
        *guard = Some(session);
        debug!(
            "IMAP connected to {} ({:?})",
            self.config.host, self.config.security
        );
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
            .map_err(|_| {
                PebbleError::Network(format!("SOCKS5 connect to {proxy_addr} timed out (10s)"))
            })?
            .map_err(|e| PebbleError::Network(format!("SOCKS5 proxy: {e}")))?;
            let tcp = stream.into_inner();
            report.push_str(&format!(
                "TCP via SOCKS5 {proxy_addr}: OK ({:.0}ms)\n",
                t0.elapsed().as_millis()
            ));
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
                report.push_str(&format!(
                    "TCP direct to {addr} (IP: {peer}): OK ({:.0}ms)\n",
                    t0.elapsed().as_millis()
                ));
            } else {
                report.push_str(&format!(
                    "TCP direct to {addr}: OK ({:.0}ms)\n",
                    t0.elapsed().as_millis()
                ));
            }
            tcp
        };

        // Helper: reconnect TCP for TLS fallback (no report, just the socket)
        let reconnect_tcp = |config: &ImapConfig| {
            let addr_clone = addr.clone();
            let proxy = config.proxy.clone();
            async move {
                if let Some(ref proxy) = proxy {
                    let proxy_addr = format!("{}:{}", proxy.host, proxy.port);
                    let stream = tokio::time::timeout(
                        std::time::Duration::from_secs(10),
                        tokio_socks::tcp::Socks5Stream::connect(
                            proxy_addr.as_str(),
                            addr_clone.as_str(),
                        ),
                    )
                    .await
                    .map_err(|_| {
                        PebbleError::Network(format!(
                            "SOCKS5 reconnect to {proxy_addr} timed out (10s)"
                        ))
                    })?
                    .map_err(|e| PebbleError::Network(format!("SOCKS5 proxy: {e}")))?;
                    Ok::<TcpStream, PebbleError>(stream.into_inner())
                } else {
                    let tcp = tokio::time::timeout(
                        std::time::Duration::from_secs(10),
                        TcpStream::connect(&addr_clone),
                    )
                    .await
                    .map_err(|_| {
                        PebbleError::Network(format!(
                            "TCP reconnect to {addr_clone} timed out (10s)"
                        ))
                    })?
                    .map_err(|e| PebbleError::Network(format!("TCP reconnect: {e}")))?;
                    Ok::<TcpStream, PebbleError>(tcp)
                }
            }
        };

        // Step 2: TLS handshake (if applicable)
        match config.security {
            ConnectionSecurity::Tls => {
                let t1 = Instant::now();
                match tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    tls_connect(&config.host, tcp, config.accept_invalid_certs),
                )
                .await
                {
                    Ok(Ok(mut tls)) => {
                        report.push_str(&format!(
                            "TLS handshake (rustls): OK ({:.0}ms)\n",
                            t1.elapsed().as_millis()
                        ));
                        let t2 = Instant::now();
                        let mut buf = vec![0u8; 4096];
                        let n = tokio::time::timeout(
                            std::time::Duration::from_secs(10),
                            tls.read(&mut buf),
                        )
                        .await
                        .map_err(|_| {
                            PebbleError::Network("Read IMAP greeting timed out (10s)".into())
                        })?
                        .map_err(|e| PebbleError::Network(format!("Read greeting: {e}")))?;
                        let greeting = String::from_utf8_lossy(&buf[..n]);
                        report.push_str(&format!(
                            "IMAP greeting ({:.0}ms): {}\n",
                            t2.elapsed().as_millis(),
                            greeting.trim()
                        ));
                    }
                    Ok(Err(rustls_err)) => {
                        report
                            .push_str(&format!("TLS handshake (rustls): FAILED — {rustls_err}\n"));
                        let tcp = reconnect_tcp(config).await?;
                        let t_ntls = Instant::now();
                        let mut tls = tokio::time::timeout(
                            std::time::Duration::from_secs(10),
                            native_tls_connect(&config.host, tcp, config.accept_invalid_certs),
                        )
                        .await
                        .map_err(|_| {
                            PebbleError::Network("native-tls handshake timed out (10s)".into())
                        })??;
                        report.push_str(&format!(
                            "TLS handshake (native-tls fallback): OK ({:.0}ms)\n",
                            t_ntls.elapsed().as_millis()
                        ));
                        let t2 = Instant::now();
                        let mut buf = vec![0u8; 4096];
                        let n = tokio::time::timeout(
                            std::time::Duration::from_secs(10),
                            tls.read(&mut buf),
                        )
                        .await
                        .map_err(|_| {
                            PebbleError::Network("Read IMAP greeting timed out (10s)".into())
                        })?
                        .map_err(|e| PebbleError::Network(format!("Read greeting: {e}")))?;
                        let greeting = String::from_utf8_lossy(&buf[..n]);
                        report.push_str(&format!(
                            "IMAP greeting ({:.0}ms): {}\n",
                            t2.elapsed().as_millis(),
                            greeting.trim()
                        ));
                    }
                    Err(_) => {
                        return Err(PebbleError::Network("TLS handshake timed out (10s)".into()));
                    }
                }
            }
            ConnectionSecurity::StartTls => {
                // Read plain greeting first
                let mut tcp = tcp;
                let t1 = Instant::now();
                let mut buf = vec![0u8; 4096];
                let n =
                    tokio::time::timeout(std::time::Duration::from_secs(10), tcp.read(&mut buf))
                        .await
                        .map_err(|_| {
                            PebbleError::Network("Read plain greeting timed out (10s)".into())
                        })?
                        .map_err(|e| PebbleError::Network(format!("Read greeting: {e}")))?;
                let greeting = String::from_utf8_lossy(&buf[..n]);
                report.push_str(&format!(
                    "Plain greeting ({:.0}ms): {}\n",
                    t1.elapsed().as_millis(),
                    greeting.trim()
                ));

                // Send STARTTLS
                let t2 = Instant::now();
                tcp.write_all(b"A001 STARTTLS\r\n")
                    .await
                    .map_err(|e| PebbleError::Network(format!("Send STARTTLS: {e}")))?;
                tcp.flush()
                    .await
                    .map_err(|e| PebbleError::Network(format!("Flush: {e}")))?;
                let mut resp = vec![0u8; 4096];
                let n =
                    tokio::time::timeout(std::time::Duration::from_secs(10), tcp.read(&mut resp))
                        .await
                        .map_err(|_| {
                            PebbleError::Network("STARTTLS response timed out (10s)".into())
                        })?
                        .map_err(|e| {
                            PebbleError::Network(format!("Read STARTTLS response: {e}"))
                        })?;
                let resp_str = String::from_utf8_lossy(&resp[..n]);
                report.push_str(&format!(
                    "STARTTLS response ({:.0}ms): {}\n",
                    t2.elapsed().as_millis(),
                    resp_str.trim()
                ));

                // TLS upgrade — try rustls, fall back to native-tls
                let t3 = Instant::now();
                match tokio::time::timeout(
                    std::time::Duration::from_secs(10),
                    tls_connect(&config.host, tcp, config.accept_invalid_certs),
                )
                .await
                {
                    Ok(Ok(_)) => {
                        report.push_str(&format!(
                            "TLS upgrade (STARTTLS, rustls): OK ({:.0}ms)\n",
                            t3.elapsed().as_millis()
                        ));
                    }
                    Ok(Err(rustls_err)) => {
                        report.push_str(&format!(
                            "TLS upgrade (STARTTLS, rustls): FAILED — {rustls_err}\n"
                        ));
                        // Reconnect and redo STARTTLS with native-tls
                        let tcp = reconnect_tcp(config).await?;
                        let mut tcp = tcp;
                        // Re-read greeting (discard)
                        let mut discard = vec![0u8; 4096];
                        let _ = tokio::time::timeout(
                            std::time::Duration::from_secs(10),
                            tcp.read(&mut discard),
                        )
                        .await
                        .map_err(|_| PebbleError::Network("Read greeting timed out (10s)".into()))?
                        .map_err(|e| PebbleError::Network(format!("Read greeting: {e}")))?;
                        // Re-send STARTTLS
                        tcp.write_all(b"A001 STARTTLS\r\n")
                            .await
                            .map_err(|e| PebbleError::Network(format!("Send STARTTLS: {e}")))?;
                        tcp.flush()
                            .await
                            .map_err(|e| PebbleError::Network(format!("Flush: {e}")))?;
                        let mut resp2 = vec![0u8; 4096];
                        let _ = tokio::time::timeout(
                            std::time::Duration::from_secs(10),
                            tcp.read(&mut resp2),
                        )
                        .await
                        .map_err(|_| {
                            PebbleError::Network("STARTTLS response timed out (10s)".into())
                        })?
                        .map_err(|e| {
                            PebbleError::Network(format!("Read STARTTLS response: {e}"))
                        })?;
                        let t4 = Instant::now();
                        tokio::time::timeout(
                            std::time::Duration::from_secs(10),
                            native_tls_connect(&config.host, tcp, config.accept_invalid_certs),
                        )
                        .await
                        .map_err(|_| {
                            PebbleError::Network("native-tls upgrade timed out (10s)".into())
                        })??;
                        report.push_str(&format!(
                            "TLS upgrade (STARTTLS, native-tls fallback): OK ({:.0}ms)\n",
                            t4.elapsed().as_millis()
                        ));
                    }
                    Err(_) => {
                        return Err(PebbleError::Network("TLS upgrade timed out (10s)".into()));
                    }
                }
            }
            ConnectionSecurity::Plain => {
                // Read plain greeting
                let mut tcp = tcp;
                let t1 = Instant::now();
                let mut buf = vec![0u8; 4096];
                let n =
                    tokio::time::timeout(std::time::Duration::from_secs(10), tcp.read(&mut buf))
                        .await
                        .map_err(|_| {
                            PebbleError::Network("Read plain greeting timed out (10s)".into())
                        })?
                        .map_err(|e| PebbleError::Network(format!("Read greeting: {e}")))?;
                let greeting = String::from_utf8_lossy(&buf[..n]);
                report.push_str(&format!(
                    "Plain greeting ({:.0}ms): {}\n",
                    t1.elapsed().as_millis(),
                    greeting.trim()
                ));
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

        let mailboxes: Vec<(String, Vec<NameAttribute<'static>>)> = {
            let stream = sess
                .list(None, Some("*"))
                .await
                .map_err(|e| PebbleError::Network(format!("LIST failed: {e}")))?;
            stream
                .map_ok(|n| {
                    let attributes = n
                        .attributes()
                        .iter()
                        .cloned()
                        .map(NameAttribute::into_owned)
                        .collect();
                    (n.name().to_string(), attributes)
                })
                .try_collect()
                .await
                .map_err(|e| PebbleError::Network(format!("LIST collect: {e}")))?
        };

        let mut folders: Vec<Folder> = mailboxes
            .into_iter()
            .filter(|(_, attributes)| should_sync_listed_mailbox(attributes))
            .map(|(raw_name, attributes)| {
                // Decode IMAP Modified UTF-7 folder name to UTF-8
                let display_name = utf7_imap::decode_utf7_imap(raw_name.clone());
                let role = detect_folder_role_from_attributes(&attributes)
                    .or_else(|| detect_folder_role(&raw_name))
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
                let mailbox_info =
                    with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, $s.select(mailbox))
                        .await?;

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
                    if !should_search_incremental_uids(next_uid, mailbox_info.uid_next) {
                        return Ok(Vec::new());
                    }
                    let search_query = format!("UID {next_uid}:*");
                    let searched_uids: Vec<u32> = with_imap_timeout(
                        "UID SEARCH",
                        IMAP_COMMAND_TIMEOUT_SECS,
                        $s.uid_search(&search_query),
                    )
                    .await?
                    .into_iter()
                    .collect();
                    let fetch_uids = incremental_fetch_uids(Some(uid), &searched_uids);
                    if fetch_uids.is_empty() {
                        return Ok(Vec::new());
                    }
                    let uid_set = fetch_uids
                        .iter()
                        .map(u32::to_string)
                        .collect::<Vec<_>>()
                        .join(",");
                    let fetches = with_imap_timeout(
                        "UID FETCH",
                        IMAP_COMMAND_TIMEOUT_SECS,
                        $s.uid_fetch(&uid_set, "(UID BODY.PEEK[])"),
                    )
                    .await?;
                    let fetches: Vec<async_imap::types::Fetch> = with_imap_timeout(
                        "UID FETCH collect",
                        IMAP_COMMAND_TIMEOUT_SECS,
                        fetches.try_collect(),
                    )
                    .await?;
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
                    let start = if exists > limit {
                        exists - limit + 1
                    } else {
                        1
                    };
                    return Self::fetch_messages_sequence_range($s, start, exists).await;
                }

                results
            }};
        }

        let results = do_fetch!(sess);

        Ok(results)
    }

    /// Fetch the raw bytes of the messages whose sequence numbers fall within
    /// `[start_seq, end_seq]` (inclusive), after selecting `mailbox`.
    ///
    /// Sequence numbers are stable for the lifetime of a session; callers that
    /// page backwards through a mailbox must issue these fetches on the same
    /// connection. Returns `(uid, raw_bytes)` pairs ordered by the server
    /// response, which is ascending sequence number.
    pub async fn fetch_messages_page(
        &self,
        mailbox: &str,
        start_seq: u32,
        end_seq: u32,
    ) -> Result<Vec<(u32, Vec<u8>)>> {
        if start_seq == 0 || end_seq < start_seq {
            return Ok(Vec::new());
        }
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;

        macro_rules! do_fetch {
            ($s:expr) => {{
                let mailbox_info =
                    with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, $s.select(mailbox))
                        .await?;
                if mailbox_info.exists == 0 || start_seq > mailbox_info.exists {
                    return Ok(Vec::new());
                }
                let end_seq = end_seq.min(mailbox_info.exists);
                Self::fetch_messages_sequence_range($s, start_seq, end_seq).await?
            }};
        }

        let results = do_fetch!(sess);

        Ok(results)
    }

    pub async fn search_uids_since(
        &self,
        mailbox: &str,
        cutoff: i64,
        since_uid: Option<u32>,
    ) -> Result<(Vec<u32>, Option<u32>)> {
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;
        let mailbox_info =
            with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, sess.select(mailbox)).await?;
        let Some(criteria) = imap_since_search_criteria(cutoff, since_uid) else {
            return Ok((Vec::new(), mailbox_info.uid_next));
        };
        let mut uids = with_imap_timeout(
            "UID SEARCH",
            IMAP_COMMAND_TIMEOUT_SECS,
            sess.uid_search(&criteria),
        )
        .await?
        .into_iter()
        .collect::<Vec<_>>();
        uids.sort_unstable();
        uids.dedup();
        Ok((uids, mailbox_info.uid_next))
    }

    pub async fn fetch_messages_by_uids(
        &self,
        mailbox: &str,
        uids: &[u32],
    ) -> Result<Vec<(u32, Vec<u8>)>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let uid_set = uids
            .iter()
            .map(u32::to_string)
            .collect::<Vec<_>>()
            .join(",");
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;
        let _ =
            with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, sess.select(mailbox)).await?;
        let fetches = with_imap_timeout(
            "UID FETCH",
            IMAP_COMMAND_TIMEOUT_SECS,
            sess.uid_fetch(&uid_set, "(UID BODY.PEEK[])"),
        )
        .await?;
        let fetches: Vec<async_imap::types::Fetch> = with_imap_timeout(
            "UID FETCH collect",
            IMAP_COMMAND_TIMEOUT_SECS,
            fetches.try_collect(),
        )
        .await?;
        let requested: HashSet<_> = uids.iter().copied().collect();
        let mut fetched = Vec::with_capacity(requested.len());
        let mut fetched_uids = HashSet::with_capacity(requested.len());
        for fetch in fetches {
            let Some(uid) = fetch.uid else {
                continue;
            };
            let Some(body) = fetch.body() else {
                return Err(PebbleError::Network(format!(
                    "IMAP UID FETCH returned no body for UID {uid}"
                )));
            };
            fetched_uids.insert(uid);
            fetched.push((uid, body.to_vec()));
        }
        if fetched_uids != requested {
            return Err(PebbleError::Network(
                "IMAP UID FETCH did not return every requested UID".to_string(),
            ));
        }
        Ok(fetched)
    }

    /// Fetch the raw bytes of the messages whose sequence numbers fall within
    /// `[start_seq, end_seq]` (inclusive). The caller must have already
    /// selected the mailbox on the same session.
    async fn fetch_messages_sequence_range(
        sess: &mut ImapSession,
        start_seq: u32,
        end_seq: u32,
    ) -> Result<Vec<(u32, Vec<u8>)>> {
        let seq_set = if start_seq == end_seq {
            start_seq.to_string()
        } else {
            format!("{start_seq}:{end_seq}")
        };
        let fetches = with_imap_timeout(
            "FETCH",
            IMAP_COMMAND_TIMEOUT_SECS,
            sess.fetch(&seq_set, "(UID BODY.PEEK[])"),
        )
        .await?;
        let fetches: Vec<async_imap::types::Fetch> = with_imap_timeout(
            "FETCH collect",
            IMAP_COMMAND_TIMEOUT_SECS,
            fetches.try_collect(),
        )
        .await?;
        let mut results = Vec::with_capacity(fetches.len());
        for fetch in fetches {
            if let Some(uid) = fetch.uid {
                if let Some(body) = fetch.body() {
                    results.push((uid, body.to_vec()));
                }
            } else {
                tracing::warn!("Skipping message without UID (seq={})", fetch.message);
            }
        }
        Ok(results)
    }

    /// Fetch flags for a set of UIDs. Returns `(uid, is_read, is_starred)`.
    pub async fn fetch_flags(&self, mailbox: &str, uids: &[u32]) -> Result<Vec<(u32, bool, bool)>> {
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
                with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, $s.select(mailbox)).await?;

                let fetches = with_imap_timeout(
                    "UID FETCH FLAGS",
                    IMAP_COMMAND_TIMEOUT_SECS,
                    $s.uid_fetch(&uid_set, "FLAGS"),
                )
                .await?;
                let fetches: Vec<async_imap::types::Fetch> = with_imap_timeout(
                    "FLAGS collect",
                    IMAP_COMMAND_TIMEOUT_SECS,
                    fetches.try_collect(),
                )
                .await?;

                fetches
                    .into_iter()
                    .filter_map(|fetch| {
                        let uid = fetch.uid.or_else(|| {
                            tracing::warn!(
                                "Skipping flags for message without UID (seq={})",
                                fetch.message
                            );
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
                with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, $s.select(mailbox)).await?;

                if let Some(read) = is_read {
                    let flag_cmd = if read {
                        format!("+FLAGS (\\Seen)")
                    } else {
                        format!("-FLAGS (\\Seen)")
                    };
                    let store_result = with_imap_timeout(
                        "STORE \\Seen",
                        IMAP_COMMAND_TIMEOUT_SECS,
                        $s.uid_store(&uid_str, &flag_cmd),
                    )
                    .await?;
                    let _: Vec<async_imap::types::Fetch> = with_imap_timeout(
                        "STORE \\Seen collect",
                        IMAP_COMMAND_TIMEOUT_SECS,
                        store_result.try_collect(),
                    )
                    .await?;
                }

                if let Some(starred) = is_starred {
                    let flag_cmd = if starred {
                        format!("+FLAGS (\\Flagged)")
                    } else {
                        format!("-FLAGS (\\Flagged)")
                    };
                    let store_result = with_imap_timeout(
                        "STORE \\Flagged",
                        IMAP_COMMAND_TIMEOUT_SECS,
                        $s.uid_store(&uid_str, &flag_cmd),
                    )
                    .await?;
                    let _: Vec<async_imap::types::Fetch> = with_imap_timeout(
                        "STORE \\Flagged collect",
                        IMAP_COMMAND_TIMEOUT_SECS,
                        store_result.try_collect(),
                    )
                    .await?;
                }
            }};
        }

        do_store!(sess);

        Ok(())
    }

    /// Move a message by UID from one mailbox to another.
    ///
    /// Tries IMAP MOVE (uid_mv) first, falls back to UID COPY + UID STORE \Deleted + EXPUNGE.
    /// One-shot wrapper around [`move_message_with_dedup`] with retry-dedup disabled.
    pub async fn move_message(
        &self,
        source_mailbox: &str,
        uid: u32,
        dest_mailbox: &str,
    ) -> Result<()> {
        self.move_message_with_dedup(source_mailbox, uid, dest_mailbox, None)
            .await
    }

    /// Move a message by UID with an idempotent COPY fallback.
    ///
    /// Pass the message's `Message-ID` header so that, on retry after a partial
    /// COPY/EXPUNGE failure, the destination is searched for a matching copy
    /// before COPY runs — preventing duplicate messages. `None` or an empty
    /// header disables the check (one-shot move). `STORE \Deleted` + `EXPUNGE`
    /// are always run and are themselves idempotent.
    pub async fn move_message_with_dedup(
        &self,
        source_mailbox: &str,
        uid: u32,
        dest_mailbox: &str,
        message_id_header: Option<&str>,
    ) -> Result<()> {
        let uid_str = uid.to_string();

        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;

        macro_rules! do_move {
            ($s:expr) => {{
                with_imap_timeout(
                    "SELECT",
                    IMAP_COMMAND_TIMEOUT_SECS,
                    $s.select(source_mailbox),
                )
                .await?;

                // Try MOVE extension first
                match with_imap_timeout(
                    "UID MOVE",
                    IMAP_COMMAND_TIMEOUT_SECS,
                    $s.uid_mv(&uid_str, dest_mailbox),
                )
                .await
                {
                    Ok(_) => {
                        debug!(
                            "MOVE UID {} from {} to {} succeeded",
                            uid, source_mailbox, dest_mailbox
                        );
                    }
                    Err(_move_err) => {
                        // Fallback: COPY + flag Deleted + EXPUNGE
                        debug!(
                            "MOVE not supported, falling back to COPY+DELETE for UID {}",
                            uid
                        );

                        // Idempotency: on retry after a partial COPY/EXPUNGE
                        // failure the destination may already hold a copy (COPY
                        // preserves the Message-ID header). Search the dest and
                        // skip COPY if a match is found, so a retried move does
                        // not create duplicates. Any error here is treated as
                        // "no match" (fall through to COPY) — never block the move.
                        let mut already_copied = false;
                        if let Some(h) = message_id_header {
                            if !h.is_empty() {
                                let criteria = format!(
                                    "HEADER \"Message-ID\" \"{}\"",
                                    h.replace('\\', "\\\\").replace('"', "\\\"")
                                );
                                let dest_selected = with_imap_timeout(
                                    "SELECT",
                                    IMAP_COMMAND_TIMEOUT_SECS,
                                    $s.select(dest_mailbox),
                                )
                                .await
                                .is_ok();
                                if dest_selected {
                                    already_copied = match with_imap_timeout(
                                        "UID SEARCH",
                                        IMAP_COMMAND_TIMEOUT_SECS,
                                        $s.uid_search(criteria.as_str()),
                                    )
                                    .await
                                    {
                                        Ok(hits) => hits.into_iter().next().is_some(),
                                        Err(_) => false,
                                    };
                                }
                            }
                        }

                        // Re-select source (restores selection after the dest
                        // search above and in case the MOVE attempt changed state).
                        with_imap_timeout(
                            "SELECT",
                            IMAP_COMMAND_TIMEOUT_SECS,
                            $s.select(source_mailbox),
                        )
                        .await?;

                        if !already_copied {
                            with_imap_timeout(
                                "UID COPY",
                                IMAP_COMMAND_TIMEOUT_SECS,
                                $s.uid_copy(&uid_str, dest_mailbox),
                            )
                            .await?;
                        }

                        let store_result = with_imap_timeout(
                            "STORE \\Deleted",
                            IMAP_COMMAND_TIMEOUT_SECS,
                            $s.uid_store(&uid_str, "+FLAGS (\\Deleted)"),
                        )
                        .await?;
                        let _: Vec<async_imap::types::Fetch> = with_imap_timeout(
                            "STORE \\Deleted collect",
                            IMAP_COMMAND_TIMEOUT_SECS,
                            store_result.try_collect(),
                        )
                        .await?;

                        let expunge_result =
                            with_imap_timeout("EXPUNGE", IMAP_COMMAND_TIMEOUT_SECS, $s.expunge())
                                .await?;
                        let _: Vec<u32> = with_imap_timeout(
                            "EXPUNGE collect",
                            IMAP_COMMAND_TIMEOUT_SECS,
                            expunge_result.try_collect(),
                        )
                        .await?;

                        debug!(
                            "COPY+DELETE UID {} from {} to {} succeeded",
                            uid, source_mailbox, dest_mailbox
                        );
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
                with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, $s.select(mailbox)).await?;

                let store_result = with_imap_timeout(
                    "STORE \\Deleted",
                    IMAP_COMMAND_TIMEOUT_SECS,
                    $s.uid_store(&uid_str, "+FLAGS (\\Deleted)"),
                )
                .await?;
                let _: Vec<async_imap::types::Fetch> = with_imap_timeout(
                    "STORE \\Deleted collect",
                    IMAP_COMMAND_TIMEOUT_SECS,
                    store_result.try_collect(),
                )
                .await?;

                let expunge_result =
                    with_imap_timeout("EXPUNGE", IMAP_COMMAND_TIMEOUT_SECS, $s.expunge()).await?;
                let _: Vec<u32> = with_imap_timeout(
                    "EXPUNGE collect",
                    IMAP_COMMAND_TIMEOUT_SECS,
                    expunge_result.try_collect(),
                )
                .await?;

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
                with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, $s.select(mailbox)).await?;

                let uids: Vec<u32> = with_imap_timeout(
                    "UID SEARCH ALL",
                    IMAP_COMMAND_TIMEOUT_SECS,
                    $s.uid_search("ALL"),
                )
                .await?
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

        let mbox =
            with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, sess.select(mailbox)).await?;

        Ok(mbox.exists)
    }

    /// Check if the server advertises the CONDSTORE capability (RFC 4551).
    pub async fn supports_condstore(&self) -> bool {
        let mut guard = self.session.lock().await;
        let sess = match guard.as_mut() {
            Some(s) => s,
            None => return false,
        };

        match with_imap_timeout("CAPABILITY", IMAP_COMMAND_TIMEOUT_SECS, sess.capabilities()).await
        {
            Ok(caps) => caps.has_str("CONDSTORE"),
            Err(_) => false,
        }
    }

    /// SELECT a mailbox and return the HIGHESTMODSEQ value if the server supports CONDSTORE.
    /// Returns `Ok(Some(modseq))` if available, `Ok(None)` otherwise.
    pub async fn get_highest_modseq(&self, mailbox: &str) -> Result<Option<u64>> {
        Ok(self.get_mailbox_status(mailbox).await?.highest_modseq)
    }

    /// SELECT a mailbox and return the UIDVALIDITY/HIGHESTMODSEQ values advertised by the server.
    pub async fn get_mailbox_status(&self, mailbox: &str) -> Result<ImapMailboxStatus> {
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;

        macro_rules! do_select {
            ($s:expr) => {{
                let mailbox_info =
                    with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, $s.select(mailbox))
                        .await?;
                ImapMailboxStatus {
                    uid_validity: mailbox_info.uid_validity,
                    highest_modseq: mailbox_info.highest_modseq,
                    uid_next: mailbox_info.uid_next,
                    exists: mailbox_info.exists,
                }
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
                with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, $s.select(mailbox)).await?;

                let fetches = with_imap_timeout(
                    "UID FETCH FLAGS MODSEQ",
                    IMAP_COMMAND_TIMEOUT_SECS,
                    $s.uid_fetch(&uid_set, "(FLAGS MODSEQ)"),
                )
                .await?;
                let fetches: Vec<async_imap::types::Fetch> = with_imap_timeout(
                    "FLAGS MODSEQ collect",
                    IMAP_COMMAND_TIMEOUT_SECS,
                    fetches.try_collect(),
                )
                .await?;

                let mut highest = 0u64;
                let results: Vec<(u32, bool, bool)> = fetches
                    .into_iter()
                    .filter_map(|fetch| {
                        let uid = fetch.uid.or_else(|| {
                            tracing::warn!(
                                "Skipping modseq flags for message without UID (seq={})",
                                fetch.message
                            );
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

        match with_imap_timeout("CAPABILITY", IMAP_COMMAND_TIMEOUT_SECS, sess.capabilities()).await
        {
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
        if let Err(e) =
            with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, session.select(mailbox)).await
        {
            // Restore session before returning error.
            let mut guard = self.session.lock().await;
            *guard = Some(session);
            return Err(e);
        }

        let mut idle_handle = session.idle();
        if let Err(e) =
            with_imap_timeout("IDLE init", IMAP_COMMAND_TIMEOUT_SECS, idle_handle.init()).await
        {
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
            return Err(e);
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

fn should_sync_listed_mailbox(attributes: &[NameAttribute<'_>]) -> bool {
    !attributes
        .iter()
        .any(|attribute| matches!(attribute, NameAttribute::NoSelect))
}

fn detect_folder_role_from_attributes(attributes: &[NameAttribute<'_>]) -> Option<FolderRole> {
    attributes.iter().find_map(|attribute| match attribute {
        NameAttribute::Archive => Some(FolderRole::Archive),
        NameAttribute::Drafts => Some(FolderRole::Drafts),
        NameAttribute::Junk => Some(FolderRole::Spam),
        NameAttribute::Sent => Some(FolderRole::Sent),
        NameAttribute::Trash => Some(FolderRole::Trash),
        NameAttribute::Extension(value) if value.eq_ignore_ascii_case("\\Inbox") => {
            Some(FolderRole::Inbox)
        }
        _ => None,
    })
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
    } else if leaf.contains("trash")
        || leaf.contains("deleted")
        || leaf.contains("已删除")
        || leaf.contains("废纸篓")
    {
        Some(FolderRole::Trash)
    } else if leaf.contains("archive") || leaf.contains("归档") || leaf.contains("存档") {
        Some(FolderRole::Archive)
    } else if leaf.contains("spam")
        || leaf.contains("junk")
        || leaf.contains("垃圾")
        || leaf.contains("病毒")
        || leaf.contains("广告")
    {
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

#[cfg(test)]
mod tls_config_tests {
    use super::{
        build_native_tls_connector, build_tls_connector, imap_timeout_error, ImapConfig, SmtpConfig,
    };

    #[test]
    fn build_tls_connector_returns_result() {
        assert!(build_tls_connector(false).is_ok());
        assert!(build_tls_connector(true).is_ok());
    }

    #[test]
    fn build_native_tls_connector_returns_result() {
        assert!(build_native_tls_connector(false).is_ok());
        assert!(build_native_tls_connector(true).is_ok());
    }

    #[test]
    fn imap_config_defaults_to_certificate_verification() {
        let config: ImapConfig = serde_json::from_value(serde_json::json!({
            "host": "mail.example.com",
            "port": 993,
            "username": "user",
            "password": "secret",
            "security": "tls"
        }))
        .unwrap();

        assert!(!config.accept_invalid_certs);
    }

    #[test]
    fn smtp_config_preserves_invalid_certificate_override() {
        let config: SmtpConfig = serde_json::from_value(serde_json::json!({
            "host": "mail.example.com",
            "port": 465,
            "username": "user",
            "password": "secret",
            "security": "tls",
            "accept_invalid_certs": true
        }))
        .unwrap();

        assert!(config.accept_invalid_certs);
    }

    #[test]
    fn imap_timeout_error_names_operation_and_seconds() {
        let error = imap_timeout_error("UID FETCH", 30);

        assert_eq!(
            error.to_string(),
            "Network error: UID FETCH timed out after 30s"
        );
    }
}

#[cfg(test)]
mod folder_list_tests {
    use async_imap::types::NameAttribute;
    use pebble_core::FolderRole;

    use super::{detect_folder_role_from_attributes, should_sync_listed_mailbox};

    #[test]
    fn special_use_attributes_detect_system_folder_roles() {
        assert_eq!(
            detect_folder_role_from_attributes(&[NameAttribute::Sent]),
            Some(FolderRole::Sent)
        );
        assert_eq!(
            detect_folder_role_from_attributes(&[NameAttribute::Trash]),
            Some(FolderRole::Trash)
        );
        assert_eq!(
            detect_folder_role_from_attributes(&[NameAttribute::Drafts]),
            Some(FolderRole::Drafts)
        );
        assert_eq!(
            detect_folder_role_from_attributes(&[NameAttribute::Archive]),
            Some(FolderRole::Archive)
        );
        assert_eq!(
            detect_folder_role_from_attributes(&[NameAttribute::Junk]),
            Some(FolderRole::Spam)
        );
    }

    #[test]
    fn extension_inbox_attribute_detects_inbox_role() {
        assert_eq!(
            detect_folder_role_from_attributes(&[NameAttribute::Extension("\\Inbox".into())]),
            Some(FolderRole::Inbox)
        );
    }

    #[test]
    fn noselect_mailboxes_are_not_synced_as_real_folders() {
        assert!(!should_sync_listed_mailbox(&[NameAttribute::NoSelect]));
        assert!(!should_sync_listed_mailbox(&[
            NameAttribute::NoSelect,
            NameAttribute::Extension("\\HasChildren".into()),
        ]));
        assert!(should_sync_listed_mailbox(&[NameAttribute::Sent]));
    }
}

#[cfg(test)]
mod incremental_uid_tests {
    use super::{
        imap_since_search_criteria, incremental_fetch_uids, should_search_incremental_uids,
    };

    #[test]
    fn sync_range_search_uses_inclusive_utc_date_and_uid_cursor() {
        assert_eq!(
            imap_since_search_criteria(1_704_067_200, Some(42)),
            Some("SINCE 01-Jan-2024 UID 43:*".to_string())
        );
        assert_eq!(
            imap_since_search_criteria(1_704_067_200, Some(u32::MAX)),
            None
        );
    }

    #[test]
    fn incremental_fetch_skips_when_no_uid_is_newer_than_cursor() {
        let uids = incremental_fetch_uids(Some(3291), &[1, 42, 3291]);

        assert!(uids.is_empty());
    }

    #[test]
    fn incremental_fetch_returns_only_newer_uids_sorted_and_deduped() {
        let uids = incremental_fetch_uids(Some(42), &[45, 41, 43, 45, 42]);

        assert_eq!(uids, vec![43, 45]);
    }

    #[test]
    fn incremental_fetch_skips_search_when_uidnext_shows_no_new_mail() {
        assert!(!should_search_incremental_uids(3292, Some(3292)));
        assert!(!should_search_incremental_uids(3293, Some(3292)));
    }

    #[test]
    fn incremental_fetch_searches_when_uidnext_is_missing_or_newer() {
        assert!(should_search_incremental_uids(3292, None));
        assert!(should_search_incremental_uids(3292, Some(3293)));
    }
}
