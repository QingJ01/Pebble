use crate::commands::encrypted_store::{
    encrypt_secure_user_data, load_secure_user_data, lock_secure_user_data_key,
    store_secure_user_data,
};
use crate::state::AppState;
use pebble_core::{now_timestamp, KanbanCard, KanbanColumn, PebbleError};
use serde::{de::DeserializeOwned, Serialize};
use std::collections::HashMap;
use tauri::State;

pub(crate) const KANBAN_CONTEXT_NOTES_KEY: &str = "kanban_context_notes";

fn decrypt_json<T: DeserializeOwned>(
    state: &AppState,
    key: &str,
) -> Result<Option<T>, PebbleError> {
    let Some(decrypted) = load_secure_user_data(&state.crypto, &state.store, key)? else {
        return Ok(None);
    };
    serde_json::from_slice(&decrypted)
        .map(Some)
        .map_err(|e| PebbleError::Internal(format!("Invalid secure user data for {key}: {e}")))
}

fn encrypt_json_bytes<T: Serialize>(
    state: &AppState,
    key: &str,
    value: &T,
) -> Result<Vec<u8>, PebbleError> {
    let plaintext = serde_json::to_vec(value)
        .map_err(|e| PebbleError::Internal(format!("Failed to serialize secure user data: {e}")))?;
    encrypt_secure_user_data(&state.crypto, key, &plaintext)
}

fn encrypt_json<T: Serialize>(state: &AppState, key: &str, value: &T) -> Result<(), PebbleError> {
    let plaintext = serde_json::to_vec(value)
        .map_err(|e| PebbleError::Internal(format!("Failed to serialize secure user data: {e}")))?;
    store_secure_user_data(&state.crypto, &state.store, key, &plaintext)
}

fn normalize_context_notes(notes: HashMap<String, String>) -> HashMap<String, String> {
    notes
        .into_iter()
        .filter_map(|(message_id, note)| {
            let message_id = message_id.trim().to_string();
            if message_id.is_empty() || note.is_empty() {
                None
            } else {
                Some((message_id, note))
            }
        })
        .collect()
}

pub(crate) fn load_kanban_context_notes_for_state(
    state: &AppState,
) -> Result<HashMap<String, String>, PebbleError> {
    Ok(decrypt_json(state, KANBAN_CONTEXT_NOTES_KEY)?.unwrap_or_default())
}

pub(crate) fn encrypt_kanban_context_notes_for_state(
    state: &AppState,
    notes: HashMap<String, String>,
) -> Result<Option<Vec<u8>>, PebbleError> {
    let notes = normalize_context_notes(notes);
    if notes.is_empty() {
        Ok(None)
    } else {
        encrypt_json_bytes(state, KANBAN_CONTEXT_NOTES_KEY, &notes).map(Some)
    }
}

fn replace_kanban_context_notes_unlocked(
    state: &AppState,
    notes: HashMap<String, String>,
) -> Result<HashMap<String, String>, PebbleError> {
    let notes = normalize_context_notes(notes);
    if notes.is_empty() {
        state
            .store
            .delete_secure_user_data(KANBAN_CONTEXT_NOTES_KEY)?;
    } else {
        encrypt_json(state, KANBAN_CONTEXT_NOTES_KEY, &notes)?;
    }
    Ok(notes)
}

#[allow(dead_code)] // Full-snapshot restore callers must take the same keyed lock as set/merge.
pub(crate) async fn replace_kanban_context_notes_for_state(
    state: &AppState,
    notes: HashMap<String, String>,
) -> Result<HashMap<String, String>, PebbleError> {
    let _guard = lock_secure_user_data_key(state, KANBAN_CONTEXT_NOTES_KEY).await;
    crate::commands::messages::query::run_blocking(|| {
        replace_kanban_context_notes_unlocked(state, notes)
    })
}

#[tauri::command]
pub async fn move_to_kanban(
    state: State<'_, AppState>,
    message_id: String,
    column: KanbanColumn,
    position: Option<i32>,
) -> std::result::Result<(), PebbleError> {
    let now = now_timestamp();
    let card = KanbanCard {
        message_id,
        column,
        position: position.unwrap_or(0),
        created_at: now,
        updated_at: now,
    };
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || store.upsert_kanban_card(&card))
        .await
        .map_err(|e| PebbleError::Internal(format!("Task join error: {e}")))?
}

#[tauri::command]
pub async fn list_kanban_cards(
    state: State<'_, AppState>,
    column: Option<KanbanColumn>,
) -> std::result::Result<Vec<KanbanCard>, PebbleError> {
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || store.list_visible_kanban_cards(column.as_ref()))
        .await
        .map_err(|e| PebbleError::Internal(format!("Task join error: {e}")))?
}

#[tauri::command]
pub async fn remove_from_kanban(
    state: State<'_, AppState>,
    message_id: String,
) -> std::result::Result<(), PebbleError> {
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || store.delete_kanban_card(&message_id))
        .await
        .map_err(|e| PebbleError::Internal(format!("Task join error: {e}")))?
}

#[tauri::command]
pub async fn list_kanban_context_notes(
    state: State<'_, AppState>,
) -> std::result::Result<HashMap<String, String>, PebbleError> {
    crate::commands::messages::query::run_blocking(|| load_kanban_context_notes_for_state(&state))
}

#[tauri::command]
pub async fn set_kanban_context_note(
    state: State<'_, AppState>,
    message_id: String,
    note: String,
) -> std::result::Result<HashMap<String, String>, PebbleError> {
    set_kanban_context_note_for_state(&state, message_id, note).await
}

async fn set_kanban_context_note_for_state(
    state: &AppState,
    message_id: String,
    note: String,
) -> Result<HashMap<String, String>, PebbleError> {
    let _guard = lock_secure_user_data_key(state, KANBAN_CONTEXT_NOTES_KEY).await;
    let mut notes = crate::commands::messages::query::run_blocking(|| {
        load_kanban_context_notes_for_state(state)
    })?;
    let message_id = message_id.trim().to_string();
    if message_id.trim().is_empty() || note.is_empty() {
        notes.remove(&message_id);
    } else {
        notes.insert(message_id, note);
    }
    crate::commands::messages::query::run_blocking(|| {
        replace_kanban_context_notes_unlocked(state, notes)
    })
}

#[tauri::command]
pub async fn merge_kanban_context_notes(
    state: State<'_, AppState>,
    notes: HashMap<String, String>,
) -> std::result::Result<HashMap<String, String>, PebbleError> {
    merge_kanban_context_notes_for_state(&state, notes).await
}

async fn merge_kanban_context_notes_for_state(
    state: &AppState,
    notes: HashMap<String, String>,
) -> Result<HashMap<String, String>, PebbleError> {
    let _guard = lock_secure_user_data_key(state, KANBAN_CONTEXT_NOTES_KEY).await;
    let mut current = load_kanban_context_notes_for_state(state)?;
    for (message_id, note) in normalize_context_notes(notes) {
        current.entry(message_id).or_insert(note);
    }
    replace_kanban_context_notes_unlocked(state, current)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_crypto::CryptoService;
    use pebble_search::TantivySearch;
    use pebble_store::Store;
    use std::sync::Arc;

    fn test_state() -> AppState {
        let (snooze_stop_tx, _snooze_stop_rx) = std::sync::mpsc::channel();
        AppState::new(
            Store::open_in_memory().unwrap(),
            TantivySearch::open_in_memory().unwrap(),
            CryptoService::from_key([43; 32]),
            snooze_stop_tx,
            std::env::temp_dir().join(format!("pebble-kanban-test-{}", now_timestamp())),
        )
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_context_note_updates_do_not_lose_either_message() {
        let state = Arc::new(test_state());
        let barrier = Arc::new(tokio::sync::Barrier::new(2));
        let update = |state: Arc<AppState>,
                      barrier: Arc<tokio::sync::Barrier>,
                      message_id: &'static str,
                      note: &'static str| {
            tokio::spawn(async move {
                barrier.wait().await;
                set_kanban_context_note_for_state(&state, message_id.to_string(), note.to_string())
                    .await
            })
        };
        let first = update(Arc::clone(&state), Arc::clone(&barrier), "m1", "one");
        let second = update(Arc::clone(&state), Arc::clone(&barrier), "m2", "two");

        first.await.unwrap().unwrap();
        second.await.unwrap().unwrap();

        assert_eq!(
            load_kanban_context_notes_for_state(&state).unwrap(),
            HashMap::from([
                ("m1".to_string(), "one".to_string()),
                ("m2".to_string(), "two".to_string()),
            ])
        );
    }
}
