use pebble_core::{Message, PebbleError, ThreadSummary};
use tauri::State;

use crate::state::AppState;

#[tauri::command]
pub async fn list_thread_messages(
    state: State<'_, AppState>,
    account_id: Option<String>,
    thread_id: String,
) -> std::result::Result<Vec<Message>, PebbleError> {
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        store.list_messages_by_thread(account_id.as_deref(), &thread_id)
    })
    .await
    .map_err(|e| PebbleError::Internal(format!("Task join error: {e}")))?
}

#[tauri::command]
pub async fn list_threads(
    state: State<'_, AppState>,
    folder_id: String,
    folder_ids: Option<Vec<String>>,
    limit: u32,
    offset: u32,
    unread_only: Option<bool>,
) -> std::result::Result<Vec<ThreadSummary>, PebbleError> {
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let folders = match folder_ids {
            Some(ids) if !ids.is_empty() => ids,
            _ => vec![folder_id],
        };
        store.list_threads_by_folders_filtered(
            &folders,
            limit,
            offset,
            unread_only.unwrap_or(false),
        )
    })
    .await
    .map_err(|e| PebbleError::Internal(format!("Task join error: {e}")))?
}
