//! Settings → Connections: turn local MCP on or off, answer pairing
//! requests, manage paired clients (#107, M4).

use std::collections::BTreeMap;

use crate::mcp::{self, server, tools};
use crate::state::AppState;
use pebble_core::{now_timestamp, PebbleError};
use pebble_store::mcp::{McpAuditRow, McpClient};
use serde::Serialize;
use tauri::{AppHandle, Emitter, State};

type CmdResult<T> = std::result::Result<T, PebbleError>;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct McpStatus {
    pub enabled: bool,
    pub running: bool,
    pub endpoint: String,
    pub command: String,
    pub args: Vec<String>,
    /// Ready to paste into an MCP client's configuration.
    pub config_json: String,
}

fn endpoint(app: &AppHandle) -> CmdResult<String> {
    let dir = crate::profile::managed_app_data_dir(app)
        .ok_or_else(|| PebbleError::Internal("No data directory".into()))?;
    Ok(mcp::endpoint_for(&dir))
}

#[tauri::command]
pub async fn mcp_get_status(state: State<'_, AppState>, app: AppHandle) -> CmdResult<McpStatus> {
    let endpoint = endpoint(&app)?;
    let (command, args) = mcp::client_command(&endpoint);
    let config_json = serde_json::to_string_pretty(&serde_json::json!({
        "mcpServers": { "pebble": { "command": command, "args": args } }
    }))
    .unwrap_or_default();
    Ok(McpStatus {
        enabled: state.store.mcp_enabled()?,
        running: server::is_running(),
        endpoint,
        command,
        args,
        config_json,
    })
}

#[tauri::command]
pub async fn mcp_set_enabled(
    state: State<'_, AppState>,
    app: AppHandle,
    enabled: bool,
) -> CmdResult<()> {
    state.store.set_mcp_enabled(enabled)?;
    if enabled {
        server::start(app.clone(), endpoint(&app)?)
            .await
            .map_err(PebbleError::Internal)?;
    } else {
        server::stop();
    }
    let _ = app.emit(mcp::CHANGED_EVENT, ());
    Ok(())
}

/// Start listening at launch when the user turned MCP on.
pub async fn start_if_enabled(app: AppHandle, enabled: bool) {
    if !enabled {
        return;
    }
    match endpoint(&app) {
        Ok(endpoint) => {
            if let Err(e) = server::start(app, endpoint).await {
                tracing::warn!("MCP: couldn't start: {e}");
            }
        }
        Err(e) => tracing::warn!("MCP: {e}"),
    }
}

#[tauri::command]
pub async fn mcp_pending_pairs() -> CmdResult<Vec<server::PendingPair>> {
    Ok(server::pending_pairs())
}

/// Answer a pairing request. `grants` maps account id → level; Gmail and
/// unknown accounts are dropped.
#[tauri::command]
pub async fn mcp_respond_pair(
    state: State<'_, AppState>,
    app: AppHandle,
    id: String,
    approve: bool,
    grants: BTreeMap<String, String>,
) -> CmdResult<()> {
    let grants = tools::sanitize_grants(&state.store, &grants);
    if approve && grants.is_empty() {
        return Err(PebbleError::Validation("Allow at least one account".into()));
    }
    server::respond_to_pairing(&state.store, &id, approve, &grants)
        .map_err(PebbleError::Validation)?;
    let _ = app.emit(mcp::CHANGED_EVENT, ());
    Ok(())
}

#[tauri::command]
pub async fn mcp_list_clients(state: State<'_, AppState>) -> CmdResult<Vec<McpClient>> {
    state.store.list_mcp_clients()
}

#[tauri::command]
pub async fn mcp_set_grants(
    state: State<'_, AppState>,
    app: AppHandle,
    client_id: String,
    grants: BTreeMap<String, String>,
) -> CmdResult<()> {
    let grants = tools::sanitize_grants(&state.store, &grants);
    state.store.set_mcp_grants(&client_id, &grants)?;
    let _ = app.emit(mcp::CHANGED_EVENT, ());
    Ok(())
}

#[tauri::command]
pub async fn mcp_revoke(
    state: State<'_, AppState>,
    app: AppHandle,
    client_id: String,
) -> CmdResult<()> {
    state.store.revoke_mcp_client(&client_id, now_timestamp())?;
    let _ = app.emit(mcp::CHANGED_EVENT, ());
    Ok(())
}

#[tauri::command]
pub async fn mcp_list_audit(
    state: State<'_, AppState>,
    client_id: Option<String>,
    limit: Option<u32>,
) -> CmdResult<Vec<McpAuditRow>> {
    state
        .store
        .list_mcp_audit(client_id.as_deref(), limit.unwrap_or(50).min(500))
}
