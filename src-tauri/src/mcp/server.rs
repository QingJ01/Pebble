//! The local listener: pairing, token checks, rate limits, audit.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use pebble_core::{new_id, now_timestamp};
use pebble_mcp::protocol::{BridgeRequest, BridgeResponse, ErrorCode};
use pebble_search::TantivySearch;
use pebble_store::Store;
use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, BufReader};
use tokio::sync::oneshot;
use tracing::{info, warn};

use super::{token_hash, tools, CHANGED_EVENT, PAIR_EVENT};
use crate::state::AppState;

/// Calls per client per minute.
const RATE_LIMIT: usize = 60;
/// How long a pairing request waits for the user.
const PAIR_TIMEOUT: Duration = Duration::from_secs(170);
/// Longest request line accepted.
const MAX_LINE: usize = 256 * 1024;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PendingPair {
    pub id: String,
    pub client: String,
    pub created_at: i64,
}

struct Pending {
    info: PendingPair,
    reply: oneshot::Sender<BridgeResponse>,
}

#[derive(Default)]
struct Shared {
    pending: HashMap<String, Pending>,
    calls: HashMap<String, VecDeque<Instant>>,
}

fn shared() -> &'static Mutex<Shared> {
    static SHARED: OnceLock<Mutex<Shared>> = OnceLock::new();
    SHARED.get_or_init(|| Mutex::new(Shared::default()))
}

struct Running {
    endpoint: String,
    stop: oneshot::Sender<()>,
}

fn running() -> &'static Mutex<Option<Running>> {
    static RUNNING: OnceLock<Mutex<Option<Running>>> = OnceLock::new();
    RUNNING.get_or_init(|| Mutex::new(None))
}

pub fn is_running() -> bool {
    running().lock().map(|r| r.is_some()).unwrap_or(false)
}

pub fn pending_pairs() -> Vec<PendingPair> {
    shared()
        .lock()
        .map(|s| s.pending.values().map(|p| p.info.clone()).collect())
        .unwrap_or_default()
}

/// The user's answer to a pairing request. Approving issues a token with
/// the given grants (already sanitised by the caller).
pub fn respond_to_pairing(
    store: &Store,
    id: &str,
    approve: bool,
    grants: &BTreeMap<String, String>,
) -> Result<(), String> {
    let pending = shared()
        .lock()
        .map_err(|_| "busy".to_string())?
        .pending
        .remove(id)
        .ok_or("This request is no longer waiting")?;
    let response = if approve {
        let token = super::new_token();
        store
            .insert_mcp_client(
                &new_id(),
                &pending.info.client,
                &token_hash(&token),
                grants,
                now_timestamp(),
            )
            .map_err(|e| e.to_string())?;
        BridgeResponse::token(token)
    } else {
        BridgeResponse::error(ErrorCode::Denied, "The user declined the connection")
    };
    let _ = pending.reply.send(response);
    Ok(())
}

fn rate_limited(client_id: &str) -> bool {
    let Ok(mut shared) = shared().lock() else {
        return false;
    };
    let now = Instant::now();
    let calls = shared.calls.entry(client_id.to_string()).or_default();
    while calls
        .front()
        .is_some_and(|t| now.duration_since(*t) > Duration::from_secs(60))
    {
        calls.pop_front();
    }
    if calls.len() >= RATE_LIMIT {
        return true;
    }
    calls.push_back(now);
    false
}

/// Check the token and run the tool. Audit rows never hold arguments or
/// results, only the tool, the accounts, a count and the outcome.
pub fn handle_call(
    store: &Store,
    search: &TantivySearch,
    token: &str,
    tool: &str,
    args: serde_json::Value,
) -> BridgeResponse {
    let client = match store.mcp_client_by_token_hash(&token_hash(token)) {
        Ok(Some(client)) => client,
        Ok(None) => {
            return BridgeResponse::error(ErrorCode::Unauthorized, "Unknown or revoked connection")
        }
        Err(e) => return BridgeResponse::error(ErrorCode::Internal, e.to_string()),
    };
    let now = now_timestamp();
    let audit = |accounts: &[String], results: u32, outcome: &str| {
        if let Err(e) = store.insert_mcp_audit(&client.id, tool, accounts, results, outcome, now) {
            warn!("Failed to record MCP call: {e}");
        }
    };
    if rate_limited(&client.id) {
        audit(&[], 0, ErrorCode::RateLimited.as_str());
        return BridgeResponse::error(
            ErrorCode::RateLimited,
            "Too many calls; Pebble allows 60 a minute per client. Wait a moment.",
        );
    }
    let _ = store.touch_mcp_client(&client.id, now);
    match tools::run(store, search, &client, tool, args) {
        Ok(result) => {
            audit(&result.accounts, result.results, "ok");
            BridgeResponse::ok(result.value)
        }
        Err((code, message)) => {
            audit(&[], 0, code.as_str());
            BridgeResponse::error(code, message)
        }
    }
}

async fn handle_pair(app: &AppHandle, client: String) -> BridgeResponse {
    let client: String = client.trim().chars().take(80).collect();
    let client = if client.is_empty() {
        "MCP client".to_string()
    } else {
        client
    };
    let (tx, rx) = oneshot::channel();
    let info = PendingPair {
        id: new_id(),
        client,
        created_at: now_timestamp(),
    };
    {
        let Ok(mut shared) = shared().lock() else {
            return BridgeResponse::error(ErrorCode::Internal, "busy");
        };
        // One open request per client name.
        if shared
            .pending
            .values()
            .any(|p| p.info.client == info.client)
        {
            return BridgeResponse::error(
                ErrorCode::Pending,
                "A request from this client is already waiting for the user in Pebble.",
            );
        }
        shared.pending.insert(
            info.id.clone(),
            Pending {
                info: info.clone(),
                reply: tx,
            },
        );
    }
    let _ = app.emit(PAIR_EVENT, &info);
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    match tokio::time::timeout(PAIR_TIMEOUT, rx).await {
        Ok(Ok(response)) => {
            let _ = app.emit(CHANGED_EVENT, ());
            response
        }
        _ => {
            if let Ok(mut shared) = shared().lock() {
                shared.pending.remove(&info.id);
            }
            let _ = app.emit(CHANGED_EVENT, ());
            BridgeResponse::error(
                ErrorCode::Pending,
                "Nobody answered the request in Pebble. Try again and approve it there.",
            )
        }
    }
}

async fn serve_connection<S: AsyncRead + AsyncWrite + Unpin>(app: AppHandle, stream: S) {
    let (read, mut write) = tokio::io::split(stream);
    let mut reader = BufReader::new(read).take(MAX_LINE as u64);
    let mut line = String::new();
    let response = match reader.read_line(&mut line).await {
        Ok(0) | Err(_) => return,
        Ok(_) => match serde_json::from_str::<BridgeRequest>(line.trim()) {
            Err(_) => BridgeResponse::error(ErrorCode::Invalid, "Unreadable request"),
            Ok(BridgeRequest::Pair { client }) => handle_pair(&app, client).await,
            Ok(BridgeRequest::Call { token, tool, args }) => {
                let Some(state) = app.try_state::<AppState>() else {
                    return;
                };
                let store = Arc::clone(&state.store);
                let search = Arc::clone(&state.search);
                let changed_app = app.clone();
                let response = tokio::task::spawn_blocking(move || {
                    handle_call(&store, &search, &token, &tool, args)
                })
                .await
                .unwrap_or_else(|_| BridgeResponse::error(ErrorCode::Internal, "Pebble failed"));
                let _ = changed_app.emit(CHANGED_EVENT, ());
                response
            }
        },
    };
    let mut text = serde_json::to_string(&response).unwrap_or_else(|_| "{}".into());
    text.push('\n');
    let _ = write.write_all(text.as_bytes()).await;
    let _ = write.flush().await;
}

/// Start listening (no-op when already running).
pub async fn start(app: AppHandle, endpoint: String) -> Result<(), String> {
    if is_running() {
        return Ok(());
    }
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();

    #[cfg(unix)]
    let listener = {
        use std::os::unix::fs::PermissionsExt;
        let path = std::path::Path::new(&endpoint);
        if path.exists() {
            // A socket left by a previous run.
            let _ = std::fs::remove_file(path);
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let listener = tokio::net::UnixListener::bind(path).map_err(|e| e.to_string())?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        listener
    };

    if let Ok(mut slot) = running().lock() {
        *slot = Some(Running {
            endpoint: endpoint.clone(),
            stop: stop_tx,
        });
    }
    info!("MCP: listening on {endpoint}");

    #[cfg(unix)]
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut stop_rx => break,
                accepted = listener.accept() => match accepted {
                    Ok((stream, _)) => {
                        tauri::async_runtime::spawn(serve_connection(app.clone(), stream));
                    }
                    Err(e) => warn!("MCP: accept failed: {e}"),
                },
            }
        }
        let _ = std::fs::remove_file(&endpoint);
    });

    #[cfg(windows)]
    tauri::async_runtime::spawn(async move {
        use tokio::net::windows::named_pipe::ServerOptions;
        // Remote clients are rejected; the default security only lets the
        // owner (and administrators) open the pipe for writing.
        let mut first = true;
        loop {
            let server = match ServerOptions::new()
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .create(&endpoint)
            {
                Ok(server) => server,
                Err(e) => {
                    warn!("MCP: can't create pipe: {e}");
                    break;
                }
            };
            first = false;
            tokio::select! {
                _ = &mut stop_rx => break,
                connected = server.connect() => match connected {
                    Ok(()) => {
                        tauri::async_runtime::spawn(serve_connection(app.clone(), server));
                    }
                    Err(e) => warn!("MCP: connect failed: {e}"),
                },
            }
        }
    });

    Ok(())
}

/// Stop listening and turn away pairing requests still open.
pub fn stop() {
    if let Ok(mut slot) = running().lock() {
        if let Some(running) = slot.take() {
            let _ = running.stop.send(());
            info!("MCP: stopped listening on {}", running.endpoint);
        }
    }
    if let Ok(mut shared) = shared().lock() {
        for (_, pending) in shared.pending.drain() {
            let _ = pending.reply.send(BridgeResponse::error(
                ErrorCode::Disabled,
                "MCP was turned off in Pebble",
            ));
        }
    }
}
