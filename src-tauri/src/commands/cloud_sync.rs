use crate::commands::kanban::{
    load_kanban_context_notes_for_state, replace_kanban_context_notes_for_state,
};
use crate::state::AppState;
use pebble_core::PebbleError;
use pebble_store::cloud_sync::{preview_backup, BackupPreview, SettingsBackup, WebDavClient};
use tauri::State;

const BACKUP_FILENAME: &str = "pebble-settings-backup.json";

#[tauri::command]
pub async fn test_webdav_connection(
    url: String,
    username: String,
    password: String,
) -> std::result::Result<String, PebbleError> {
    let client = WebDavClient::new(url, username, password)?;
    client.test_connection().await?;
    Ok("Connection successful".to_string())
}

#[tauri::command]
pub async fn backup_to_webdav(
    state: State<'_, AppState>,
    url: String,
    username: String,
    password: String,
) -> std::result::Result<String, PebbleError> {
    let exported = state.store.export_settings()?;
    let mut backup: SettingsBackup = serde_json::from_slice(&exported)
        .map_err(|e| PebbleError::Internal(format!("Failed to build backup payload: {e}")))?;
    backup.kanban_context_notes = load_kanban_context_notes_for_state(&state)?;
    let data = serde_json::to_vec_pretty(&backup)
        .map_err(|e| PebbleError::Internal(format!("Failed to serialize backup payload: {e}")))?;
    let client = WebDavClient::new(url, username, password)?;
    client.upload(BACKUP_FILENAME, &data).await?;
    Ok("Settings backup completed successfully".to_string())
}

/// Download the backup and return a summary so the user can review the
/// contents before committing to a restore. Enforces size limits and schema
/// version validation in `download` and `preview_backup`.
#[tauri::command]
pub async fn preview_webdav_backup(
    url: String,
    username: String,
    password: String,
) -> std::result::Result<BackupPreview, PebbleError> {
    let client = WebDavClient::new(url, username, password)?;
    let data = client.download(BACKUP_FILENAME).await?;
    preview_backup(&data)
}

#[tauri::command]
pub async fn restore_from_webdav(
    state: State<'_, AppState>,
    url: String,
    username: String,
    password: String,
) -> std::result::Result<String, PebbleError> {
    let client = WebDavClient::new(url, username, password)?;
    let data = client.download(BACKUP_FILENAME).await?;
    // Re-validate before import; `import_settings` enforces size + version too.
    let _ = preview_backup(&data)?;
    let backup_value: serde_json::Value = serde_json::from_slice(&data)
        .map_err(|e| PebbleError::Validation(format!("Failed to parse backup: {e}")))?;
    let has_kanban_context_notes = backup_value.get("kanban_context_notes").is_some();
    let backup: SettingsBackup = serde_json::from_value(backup_value)
        .map_err(|e| PebbleError::Validation(format!("Failed to parse backup: {e}")))?;
    state.store.import_settings(&data)?;
    if has_kanban_context_notes {
        replace_kanban_context_notes_for_state(&state, backup.kanban_context_notes)?;
    }
    Ok("Settings backup restored. Reconnect accounts to continue syncing.".to_string())
}
