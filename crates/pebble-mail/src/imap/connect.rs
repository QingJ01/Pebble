use super::*;

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
    pub(super) fn needs_id_command(&self) -> bool {
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
    pub(super) async fn send_id_before_login<S: AsyncRead + AsyncWrite + Unpin>(
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
    pub(super) async fn negotiate_starttls(mut tcp: TcpStream) -> Result<TcpStream> {
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
    pub(super) async fn connect_starttls(
        &self,
        tcp: TcpStream,
        needs_id: bool,
        use_native_tls: bool,
    ) -> Result<ImapSession> {
        let mut tcp = tcp;

        // Read and discard the plaintext greeting. Anything it advertises
        // (including CAPABILITY) was sent before TLS and could have been
        // injected by an attacker, so it must never be replayed into the
        // TLS session (RFC 3501 section 6.2.1, RFC 2595 section 3.1).
        let mut greeting = vec![0u8; 8192];
        let _ = with_imap_timeout(
            "Read greeting",
            IMAP_COMMAND_TIMEOUT_SECS,
            tcp.read(&mut greeting),
        )
        .await?;
        drop(greeting);

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

        // The server sends no new greeting after STARTTLS; start the client
        // on the bare TLS stream so no pre-TLS bytes reach the session.
        let stream = PrefixedStream::with_prefix(starttls_session_prefix(), inner);
        let client = Client::new(stream);
        let mut session = tokio::time::timeout(
            Duration::from_secs(IMAP_COMMAND_TIMEOUT_SECS),
            client.login(&self.config.username, &self.config.password),
        )
        .await
        .map_err(|_| imap_timeout_error("IMAP login", IMAP_COMMAND_TIMEOUT_SECS))?
        .map_err(|(e, _)| PebbleError::Auth(format!("IMAP login failed: {e}")))?;

        // Re-learn capabilities over the protected channel; pre-TLS
        // capabilities are discarded above.
        match with_imap_timeout(
            "CAPABILITY after STARTTLS",
            IMAP_COMMAND_TIMEOUT_SECS,
            session.capabilities(),
        )
        .await
        {
            Ok(caps) => debug!(
                "IMAP capabilities after STARTTLS: CONDSTORE={} IDLE={}",
                caps.has_str("CONDSTORE"),
                caps.has_str("IDLE")
            ),
            Err(error) => debug!("CAPABILITY after STARTTLS failed: {error}"),
        }
        Ok(session)
    }

    /// Establish a TCP connection, optionally through a SOCKS5 proxy.
    pub(super) async fn tcp_connect(&self) -> Result<TcpStream> {
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
}
