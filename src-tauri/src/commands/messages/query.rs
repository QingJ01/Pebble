use crate::state::AppState;
use pebble_core::{Message, MessageSummary, PebbleError};
use pebble_store::messages::MessageCursor;
use tauri::State;

/// Run synchronous store work that borrows command state (and so cannot be
/// moved into `spawn_blocking`) without stalling a Tokio worker: on the
/// multi-threaded runtime the worker hands its other tasks off first via
/// `block_in_place`; elsewhere (unit tests on a current-thread runtime) it
/// simply runs inline.
pub(crate) fn run_blocking<T>(f: impl FnOnce() -> T) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(handle) if handle.runtime_flavor() == tokio::runtime::RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(f)
        }
        _ => f(),
    }
}

/// List a folder page newest-first. Pass `before` (the `(date, id)` of the
/// last message already shown) for keyset pagination; `offset` is kept for
/// older callers and applies after the cursor.
#[tauri::command]
pub async fn list_messages(
    state: State<'_, AppState>,
    folder_id: String,
    folder_ids: Option<Vec<String>>,
    limit: u32,
    offset: u32,
    before: Option<MessageCursor>,
    unread_only: Option<bool>,
) -> std::result::Result<Vec<MessageSummary>, PebbleError> {
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let folders = match folder_ids {
            Some(ids) if !ids.is_empty() => ids,
            _ => vec![folder_id],
        };
        store.list_messages_page_filtered(
            &folders,
            before.as_ref(),
            limit,
            offset,
            unread_only.unwrap_or(false),
        )
    })
    .await
    .map_err(|e| PebbleError::Internal(format!("Task join error: {e}")))?
}

#[tauri::command]
pub async fn list_starred_messages(
    state: State<'_, AppState>,
    account_id: String,
    limit: u32,
    offset: u32,
) -> std::result::Result<Vec<MessageSummary>, PebbleError> {
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || store.list_starred_messages(&account_id, limit, offset))
        .await
        .map_err(|e| PebbleError::Internal(format!("Task join error: {e}")))?
}

#[tauri::command]
pub async fn get_message(
    state: State<'_, AppState>,
    message_id: String,
) -> std::result::Result<Option<Message>, PebbleError> {
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || store.get_message(&message_id))
        .await
        .map_err(|e| PebbleError::Internal(format!("Task join error: {e}")))?
}

#[tauri::command]
pub async fn get_messages_batch(
    state: State<'_, AppState>,
    message_ids: Vec<String>,
) -> std::result::Result<Vec<Message>, PebbleError> {
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || store.get_messages_batch(&message_ids))
        .await
        .map_err(|e| PebbleError::Internal(format!("Task join error: {e}")))?
}
