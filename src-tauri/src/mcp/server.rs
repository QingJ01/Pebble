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
/// A request must arrive this quickly after connecting.
const READ_TIMEOUT: Duration = Duration::from_secs(5);
/// Connections served at once; more are closed.
const MAX_CONNECTIONS: usize = 32;
/// Pairing requests waiting for the user at once.
const MAX_PENDING: usize = 3;

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
    /// Which start this is; an old listener only cleans up after itself.
    generation: u64,
}

fn connections() -> &'static Arc<tokio::sync::Semaphore> {
    static CONNECTIONS: OnceLock<Arc<tokio::sync::Semaphore>> = OnceLock::new();
    CONNECTIONS.get_or_init(|| Arc::new(tokio::sync::Semaphore::new(MAX_CONNECTIONS)))
}

/// A client name safe to show: no control, bidi or zero-width characters,
/// whitespace collapsed, at most 60 characters.
pub fn clean_client_name(name: &str) -> String {
    let invisible = |c: char| {
        c.is_control()
            || matches!(c,
                '\u{200B}'..='\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2060}'..='\u{2069}' | '\u{FEFF}')
    };
    let cleaned: String = name
        .chars()
        .filter(|c| !invisible(*c))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let cleaned: String = cleaned.chars().take(60).collect();
    if cleaned.is_empty() {
        "MCP client".to_string()
    } else {
        cleaned
    }
}

fn running() -> &'static Mutex<Option<Running>> {
    static RUNNING: OnceLock<Mutex<Option<Running>>> = OnceLock::new();
    RUNNING.get_or_init(|| Mutex::new(None))
}

pub fn is_running() -> bool {
    running().lock().map(|r| r.is_some()).unwrap_or(false)
}

/// Open pairing requests, oldest first (the dialog answers them in order).
pub fn pending_pairs() -> Vec<PendingPair> {
    let mut list: Vec<PendingPair> = shared()
        .lock()
        .map(|s| s.pending.values().map(|p| p.info.clone()).collect())
        .unwrap_or_default();
    list.sort_by(|a, b| a.created_at.cmp(&b.created_at).then(a.id.cmp(&b.id)));
    list
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
    if !approve {
        let _ = pending.reply.send(BridgeResponse::error(
            ErrorCode::Denied,
            "The user declined the connection",
        ));
        return Ok(());
    }
    // The client gave up (it quit, or the request timed out): don't keep a
    // connection nobody can use.
    if pending.reply.is_closed() {
        return Err("The app stopped waiting. Connect it again and approve the new request".into());
    }
    let token = super::new_token();
    let client_id = new_id();
    store
        .insert_mcp_client(
            &client_id,
            &pending.info.client,
            &token_hash(&token),
            grants,
            now_timestamp(),
        )
        .map_err(|e| e.to_string())?;
    if pending.reply.send(BridgeResponse::token(token)).is_err() {
        let _ = store.revoke_mcp_client(&client_id, now_timestamp());
        return Err("The app stopped waiting. Connect it again and approve the new request".into());
    }
    Ok(())
}

/// `Some(first)` when over the limit (`first` for the first refusal in the
/// window, which is the only one audited).
fn rate_limited(client_id: &str) -> Option<bool> {
    let Ok(mut shared) = shared().lock() else {
        return None;
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
        let first = calls.len() == RATE_LIMIT;
        if first {
            // Mark the window as reported (an extra entry; it expires too).
            calls.push_back(now);
        }
        return Some(first);
    }
    calls.push_back(now);
    None
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
    // Unknown tool names are client input: don't store them as given.
    let tool_name = if pebble_mcp::protocol::Level::required_for(tool).is_some() {
        tool
    } else {
        "unknown"
    };
    let audit = |accounts: &[String], results: u32, outcome: &str| {
        if let Err(e) =
            store.insert_mcp_audit(&client.id, tool_name, accounts, results, outcome, now)
        {
            warn!("Failed to record MCP call: {e}");
        }
    };
    if let Some(first) = rate_limited(&client.id) {
        if first {
            audit(&[], 0, ErrorCode::RateLimited.as_str());
        }
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
    let client = clean_client_name(&client);
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
        // One open request per client name, and only a few in all.
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
        if shared.pending.len() >= MAX_PENDING {
            return BridgeResponse::error(
                ErrorCode::Pending,
                "Pebble has other connection requests waiting. Try again later.",
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
    // Pebble may be hidden in the tray: bring the window back, and say so
    // in a desktop notification too.
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
    let labels = crate::commands::notifications::notification_labels();
    let body = crate::commands::notifications::fill(&labels.mcp_pair, &[("client", &info.client)]);
    if let Err(e) =
        crate::commands::notifications::show_desktop_notification(app, &labels.agent_title, &body)
    {
        warn!("MCP: couldn't show the pairing notification: {e}");
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
    let read = tokio::time::timeout(READ_TIMEOUT, reader.read_line(&mut line)).await;
    let response = match read {
        Err(_) | Ok(Ok(0)) | Ok(Err(_)) => return,
        Ok(Ok(_)) => match serde_json::from_str::<BridgeRequest>(line.trim()) {
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

/// Serve one accepted connection if there is room; otherwise drop it.
fn accept<S>(app: &AppHandle, stream: S)
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    match Arc::clone(connections()).try_acquire_owned() {
        Ok(permit) => {
            let app = app.clone();
            tauri::async_runtime::spawn(async move {
                serve_connection(app, stream).await;
                drop(permit);
            });
        }
        Err(_) => warn!("MCP: too many connections; one was closed"),
    }
}

/// Start listening (no-op when already running). Holds the state lock
/// from the check to the bind, so two starts can't race.
pub async fn start(app: AppHandle, endpoint: String) -> Result<(), String> {
    use std::sync::atomic::{AtomicU64, Ordering};
    static GENERATION: AtomicU64 = AtomicU64::new(1);
    let mut slot = running().lock().map_err(|_| "busy".to_string())?;
    if slot.is_some() {
        return Ok(());
    }
    let generation = GENERATION.fetch_add(1, Ordering::SeqCst);
    let (stop_tx, mut stop_rx) = oneshot::channel::<()>();

    #[cfg(unix)]
    let listener = {
        use std::os::unix::fs::PermissionsExt;
        let path = std::path::Path::new(&endpoint);
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if path.exists() {
            // A socket left by a previous run.
            let _ = std::fs::remove_file(path);
        }
        let listener = tokio::net::UnixListener::bind(path).map_err(|e| e.to_string())?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))
            .map_err(|e| e.to_string())?;
        listener
    };

    // On Windows the first pipe instance is created here, so a pipe already
    // taken by another process fails the start instead of failing later.
    #[cfg(windows)]
    let first_server = tokio::net::windows::named_pipe::ServerOptions::new()
        .first_pipe_instance(true)
        .reject_remote_clients(true)
        .create(&endpoint)
        .map_err(|e| format!("can't create the pipe ({e})"))?;

    *slot = Some(Running {
        endpoint: endpoint.clone(),
        stop: stop_tx,
        generation,
    });
    drop(slot);
    info!("MCP: listening on {endpoint}");

    #[cfg(unix)]
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::select! {
                _ = &mut stop_rx => break,
                accepted = listener.accept() => match accepted {
                    Ok((stream, _)) => accept(&app, stream),
                    Err(e) => warn!("MCP: accept failed: {e}"),
                },
            }
        }
        // Remove the socket unless a newer listener owns the path now.
        let newer = running()
            .lock()
            .map(|r| r.as_ref().is_some_and(|r| r.generation != generation))
            .unwrap_or(true);
        if !newer {
            let _ = std::fs::remove_file(&endpoint);
        }
    });

    #[cfg(windows)]
    tauri::async_runtime::spawn(async move {
        use tokio::net::windows::named_pipe::ServerOptions;
        // Remote clients are rejected; the default security only lets the
        // owner (and administrators) open the pipe for writing.
        let mut server = first_server;
        loop {
            tokio::select! {
                _ = &mut stop_rx => break,
                connected = server.connect() => {
                    if let Err(e) = connected {
                        warn!("MCP: connect failed: {e}");
                    }
                }
            }
            let next = match ServerOptions::new()
                .reject_remote_clients(true)
                .create(&endpoint)
            {
                Ok(next) => next,
                Err(e) => {
                    warn!("MCP: can't create pipe: {e}");
                    if let Ok(mut slot) = running().lock() {
                        if slot.as_ref().is_some_and(|r| r.generation == generation) {
                            *slot = None;
                        }
                    }
                    break;
                }
            };
            let connected = std::mem::replace(&mut server, next);
            accept(&app, connected);
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
