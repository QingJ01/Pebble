use std::time::Duration;

use crate::OAuthError;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpListener;
use url::Url;

const SUCCESS_HTML: &str = r#"<!DOCTYPE html>
<html><head><title>Pebble</title></head>
<body style="font-family:sans-serif;text-align:center;padding:3rem">
<h2>Authentication successful</h2>
<p>You can close this tab and return to Pebble.</p>
</body></html>"#;

const REDIRECT_TIMEOUT: Duration = Duration::from_secs(300); // 5 minutes

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OAuthRedirect {
    pub code: String,
    pub state: String,
}

/// A bound TCP listener ready to accept the OAuth redirect.
/// Created by [`bind_redirect_listener`], consumed by [`BoundRedirectListener::wait`].
pub struct BoundRedirectListener {
    listener: TcpListener,
    /// The actual port the listener is bound to.
    pub port: u16,
}

/// Bind a TCP listener on `127.0.0.1:{port}`.
/// If port is 0, the OS assigns an available port.
pub async fn bind_redirect_listener(port: u16) -> Result<BoundRedirectListener, OAuthError> {
    let listener = TcpListener::bind(format!("127.0.0.1:{}", port)).await?;
    let port = listener
        .local_addr()
        .map_err(|e| OAuthError::Redirect(format!("Failed to get local addr: {e}")))?
        .port();
    tracing::debug!("OAuth redirect listener bound on port {}", port);
    Ok(BoundRedirectListener { listener, port })
}

const CALLBACK_PATH: &str = "/callback";
/// Per-connection budget for reading the request head, so a stalled client
/// cannot block the listener.
const REQUEST_READ_TIMEOUT: Duration = Duration::from_secs(10);
const MAX_REQUEST_HEAD_BYTES: usize = 8 * 1024;

const ERROR_HTML: &str = r#"<!DOCTYPE html>
<html><head><title>Pebble</title></head>
<body style="font-family:sans-serif;text-align:center;padding:3rem">
<h2>Authentication was not completed</h2>
<p>Return to Pebble and try again.</p>
</body></html>"#;

/// Outcome of a single connection to the loopback listener.
enum ConnectionOutcome {
    /// A valid callback with the expected state.
    Done(Result<OAuthRedirect, OAuthError>),
    /// Unrelated or forged request; keep listening.
    Ignored,
}

impl BoundRedirectListener {
    /// Wait for the OAuth redirect callback with a 5-minute overall timeout.
    ///
    /// Connections that are not a `GET /callback` request carrying the
    /// expected `state` are answered with 400/404 and ignored, so a local
    /// process or web page probing the port cannot end the flow or receive
    /// the success page.
    pub async fn wait(self, expected_state: &str) -> Result<OAuthRedirect, OAuthError> {
        self.wait_with_timeout(expected_state, REDIRECT_TIMEOUT)
            .await
    }

    async fn wait_with_timeout(
        self,
        expected_state: &str,
        overall: Duration,
    ) -> Result<OAuthRedirect, OAuthError> {
        let deadline = tokio::time::Instant::now() + overall;
        loop {
            let accepted = tokio::time::timeout_at(deadline, self.listener.accept())
                .await
                .map_err(|_| {
                    OAuthError::Redirect(
                        "OAuth redirect timed out after 5 minutes. Please try again.".into(),
                    )
                })?;
            let (mut stream, _addr) = match accepted {
                Ok(connection) => connection,
                Err(error) => {
                    tracing::debug!("OAuth redirect accept failed: {error}");
                    continue;
                }
            };

            let outcome = tokio::time::timeout_at(
                deadline.min(tokio::time::Instant::now() + REQUEST_READ_TIMEOUT),
                handle_connection(&mut stream, self.port, expected_state),
            )
            .await;
            match outcome {
                Ok(Ok(ConnectionOutcome::Done(result))) => return result,
                Ok(Ok(ConnectionOutcome::Ignored)) => {}
                Ok(Err(error)) => tracing::debug!("OAuth redirect connection error: {error}"),
                Err(_) => tracing::debug!("OAuth redirect connection timed out"),
            }
        }
    }
}

async fn handle_connection(
    stream: &mut tokio::net::TcpStream,
    port: u16,
    expected_state: &str,
) -> Result<ConnectionOutcome, OAuthError> {
    let mut buf = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    while !buf.windows(4).any(|window| window == b"\r\n\r\n") {
        let n = stream.read(&mut chunk).await?;
        if n == 0 {
            break;
        }
        buf.extend_from_slice(&chunk[..n]);
        if buf.len() > MAX_REQUEST_HEAD_BYTES {
            write_response(stream, "431 Request Header Fields Too Large", "").await?;
            return Ok(ConnectionOutcome::Ignored);
        }
    }
    let request = String::from_utf8_lossy(&buf);
    let mut parts = request.lines().next().unwrap_or("").split_whitespace();
    let method = parts.next().unwrap_or("");
    let target = parts.next().unwrap_or("");

    if method != "GET" || !target.starts_with('/') {
        write_response(stream, "400 Bad Request", "").await?;
        return Ok(ConnectionOutcome::Ignored);
    }
    let Ok(url) = Url::parse(&format!("http://127.0.0.1:{port}{target}")) else {
        write_response(stream, "400 Bad Request", "").await?;
        return Ok(ConnectionOutcome::Ignored);
    };
    if url.path() != CALLBACK_PATH {
        write_response(stream, "404 Not Found", "").await?;
        return Ok(ConnectionOutcome::Ignored);
    }

    let param = |name: &str| {
        url.query_pairs()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.into_owned())
    };
    let state_matches = param("state")
        .map(|state| constant_time_eq(state.as_bytes(), expected_state.as_bytes()))
        .unwrap_or(false);
    if !state_matches {
        tracing::warn!("OAuth redirect ignored a callback with a missing or mismatched state");
        write_response(stream, "400 Bad Request", ERROR_HTML).await?;
        return Ok(ConnectionOutcome::Ignored);
    }

    let Some(code) = param("code") else {
        let error = param("error").unwrap_or_else(|| "unknown".into());
        write_response(stream, "400 Bad Request", ERROR_HTML).await?;
        return Ok(ConnectionOutcome::Done(Err(OAuthError::Redirect(format!(
            "Authorization denied or missing code: {error}"
        )))));
    };

    write_response(stream, "200 OK", SUCCESS_HTML).await?;
    tracing::debug!("OAuth redirect received authorization code");
    Ok(ConnectionOutcome::Done(Ok(OAuthRedirect {
        code,
        state: expected_state.to_string(),
    })))
}

async fn write_response(
    stream: &mut tokio::net::TcpStream,
    status: &str,
    body: &str,
) -> Result<(), OAuthError> {
    let response = format!(
        "HTTP/1.1 {status}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n\r\n{body}",
        body.len()
    );
    stream.write_all(response.as_bytes()).await?;
    stream.flush().await?;
    Ok(())
}

/// Compare two byte strings without short-circuiting on the first mismatch.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Convenience wrapper: bind + wait in one call.
pub async fn wait_for_redirect(
    port: u16,
    expected_state: &str,
) -> Result<OAuthRedirect, OAuthError> {
    let bound = bind_redirect_listener(port).await?;
    bound.wait(expected_state).await
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn send(port: u16, request: &str) -> String {
        let mut stream = tokio::net::TcpStream::connect(format!("127.0.0.1:{port}"))
            .await
            .unwrap();
        stream.write_all(request.as_bytes()).await.unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).await.unwrap();
        response
    }

    #[tokio::test]
    async fn redirect_extracts_code_and_state() {
        let bound = bind_redirect_listener(0).await.unwrap();
        let port = bound.port;
        let handle = tokio::spawn(async move { bound.wait("xyz").await });

        let response = send(
            port,
            "GET /callback?code=test_code_123&state=xyz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
        )
        .await;

        assert!(response.starts_with("HTTP/1.1 200 OK"));
        let redirect = handle.await.unwrap().unwrap();
        assert_eq!(redirect.code, "test_code_123");
        assert_eq!(redirect.state, "xyz");
    }

    #[tokio::test]
    async fn redirect_returns_error_when_provider_denies_access() {
        let bound = bind_redirect_listener(0).await.unwrap();
        let port = bound.port;
        let handle = tokio::spawn(async move { bound.wait("xyz").await });

        let response = send(
            port,
            "GET /callback?error=access_denied&state=xyz HTTP/1.1\r\nHost: 127.0.0.1\r\n\r\n",
        )
        .await;

        assert!(response.starts_with("HTTP/1.1 400"));
        assert!(handle.await.unwrap().is_err());
    }

    #[tokio::test]
    async fn redirect_ignores_unrelated_and_forged_requests_until_valid_callback() {
        let bound = bind_redirect_listener(0).await.unwrap();
        let port = bound.port;
        let handle = tokio::spawn(async move { bound.wait("expected-state").await });

        let favicon = send(port, "GET /favicon.ico HTTP/1.1\r\n\r\n").await;
        assert!(favicon.starts_with("HTTP/1.1 404"));

        let missing_state = send(port, "GET /callback?code=evil HTTP/1.1\r\n\r\n").await;
        assert!(missing_state.starts_with("HTTP/1.1 400"));
        assert!(!missing_state.contains("Authentication successful"));

        let forged = send(
            port,
            "GET /callback?code=evil&state=wrong-state HTTP/1.1\r\n\r\n",
        )
        .await;
        assert!(forged.starts_with("HTTP/1.1 400"));

        let post = send(
            port,
            "POST /callback?code=evil&state=expected-state HTTP/1.1\r\n\r\n",
        )
        .await;
        assert!(post.starts_with("HTTP/1.1 400"));

        assert!(!handle.is_finished());

        let ok = send(
            port,
            "GET /callback?code=real&state=expected-state HTTP/1.1\r\n\r\n",
        )
        .await;
        assert!(ok.starts_with("HTTP/1.1 200 OK"));
        assert_eq!(handle.await.unwrap().unwrap().code, "real");
    }

    #[tokio::test]
    async fn redirect_times_out_overall() {
        let bound = bind_redirect_listener(0).await.unwrap();
        let result = bound
            .wait_with_timeout("xyz", Duration::from_millis(50))
            .await;
        assert!(result.is_err());
    }

    #[test]
    fn constant_time_eq_compares_content_and_length() {
        assert!(constant_time_eq(b"abc", b"abc"));
        assert!(!constant_time_eq(b"abc", b"abd"));
        assert!(!constant_time_eq(b"abc", b"abcd"));
    }
}
