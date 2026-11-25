use super::*;

/// Bytes replayed into the IMAP client after a STARTTLS upgrade: none. The
/// plaintext greeting must not be trusted once TLS is negotiated.
pub(super) fn starttls_session_prefix() -> Vec<u8> {
    Vec::new()
}

/// Stream wrapper that replays buffered prefix bytes, then delegates to inner.
/// Used to replay the IMAP greeting after manually sending an ID command.
#[derive(Debug)]
pub(super) struct PrefixedStream<T> {
    pub(super) prefix: Vec<u8>,
    pub(super) pos: usize,
    pub(super) inner: T,
}

impl<T> PrefixedStream<T> {
    pub(super) fn with_prefix(prefix: Vec<u8>, inner: T) -> Self {
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
pub(super) enum InnerStream {
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
pub(super) type ImapSession = async_imap::Session<PrefixedStream<InnerStream>>;

pub(super) const IMAP_CONNECT_TIMEOUT_SECS: u64 = 15;
pub(super) const IMAP_COMMAND_TIMEOUT_SECS: u64 = 45;

pub(super) fn imap_timeout_error(operation: &str, seconds: u64) -> PebbleError {
    PebbleError::Network(format!("{operation} timed out after {seconds}s"))
}

pub(super) async fn with_imap_timeout<T, E, Fut>(
    operation: &str,
    seconds: u64,
    future: Fut,
) -> Result<T>
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

/// Build a rustls TLS connector with bundled root certificates.
pub(super) fn build_tls_connector(
    accept_invalid_certs: bool,
) -> Result<tokio_rustls::TlsConnector> {
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
pub(super) async fn tls_connect(
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
pub(super) fn build_native_tls_connector(
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
pub(super) async fn native_tls_connect(
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
    pub(super) roots: rustls::RootCertStore,
    pub(super) provider: Arc<CryptoProvider>,
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
