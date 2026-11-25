use crate::state::AppState;
use pebble_core::PebbleError;
use pebble_store::labels::Label;
use pebble_store::Store;
use std::sync::Arc;
use tauri::State;

/// Run a store call on the blocking pool instead of a Tokio worker.
async fn on_store<T, F>(state: &AppState, f: F) -> std::result::Result<T, PebbleError>
where
    F: FnOnce(&Store) -> std::result::Result<T, PebbleError> + Send + 'static,
    T: Send + 'static,
{
    let store: Arc<Store> = state.store.clone();
    tokio::task::spawn_blocking(move || f(&store))
        .await
        .map_err(|e| PebbleError::Internal(format!("Task join error: {e}")))?
}

#[tauri::command]
pub async fn get_message_labels(
    state: State<'_, AppState>,
    message_id: String,
) -> std::result::Result<Vec<Label>, PebbleError> {
    on_store(&state, move |store| store.get_message_labels(&message_id)).await
}

#[tauri::command]
pub async fn get_message_labels_batch(
    state: State<'_, AppState>,
    message_ids: Vec<String>,
) -> std::result::Result<std::collections::HashMap<String, Vec<Label>>, PebbleError> {
    on_store(&state, move |store| {
        store.get_message_labels_batch(&message_ids)
    })
    .await
}

#[tauri::command]
pub async fn add_message_label(
    state: State<'_, AppState>,
    message_id: String,
    label_name: String,
) -> std::result::Result<(), PebbleError> {
    on_store(&state, move |store| {
        store.add_label(&message_id, &label_name)
    })
    .await
}

#[tauri::command]
pub async fn remove_message_label(
    state: State<'_, AppState>,
    message_id: String,
    label_name: String,
) -> std::result::Result<(), PebbleError> {
    on_store(&state, move |store| {
        store.remove_label(&message_id, &label_name)
    })
    .await
}

#[tauri::command]
pub async fn list_labels(
    state: State<'_, AppState>,
) -> std::result::Result<Vec<Label>, PebbleError> {
    on_store(&state, |store| store.list_labels()).await
}
