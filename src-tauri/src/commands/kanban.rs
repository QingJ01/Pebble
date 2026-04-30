use crate::state::AppState;
use pebble_core::{now_timestamp, KanbanCard, KanbanColumn, PebbleError};
use serde::{de::DeserializeOwned, Serialize};
use std::collections::HashMap;
use tauri::State;

const KANBAN_CONTEXT_NOTES_KEY: &str = "kanban_context_notes";

fn decrypt_json<T: DeserializeOwned>(
    state: &AppState,
    key: &str,
) -> Result<Option<T>, PebbleError> {
    let Some(encrypted) = state.store.get_secure_user_data(key)? else {
        return Ok(None);
    };
    let decrypted = state.crypto.decrypt(&encrypted)?;
    serde_json::from_slice(&decrypted)
        .map(Some)
        .map_err(|e| PebbleError::Internal(format!("Invalid secure user data for {key}: {e}")))
}

fn encrypt_json<T: Serialize>(state: &AppState, key: &str, value: &T) -> Result<(), PebbleError> {
    let plaintext = serde_json::to_vec(value)
        .map_err(|e| PebbleError::Internal(format!("Failed to serialize secure user data: {e}")))?;
    let encrypted = state.crypto.encrypt(&plaintext)?;
    state.store.set_secure_user_data(key, &encrypted)
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

pub(crate) fn replace_kanban_context_notes_for_state(
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
    state.store.upsert_kanban_card(&card)
}

#[tauri::command]
pub async fn list_kanban_cards(
    state: State<'_, AppState>,
    column: Option<KanbanColumn>,
) -> std::result::Result<Vec<KanbanCard>, PebbleError> {
    state.store.list_kanban_cards(column.as_ref())
}

#[tauri::command]
pub async fn remove_from_kanban(
    state: State<'_, AppState>,
    message_id: String,
) -> std::result::Result<(), PebbleError> {
    state.store.delete_kanban_card(&message_id)
}

#[tauri::command]
pub async fn list_kanban_context_notes(
    state: State<'_, AppState>,
) -> std::result::Result<HashMap<String, String>, PebbleError> {
    load_kanban_context_notes_for_state(&state)
}

#[tauri::command]
pub async fn set_kanban_context_note(
    state: State<'_, AppState>,
    message_id: String,
    note: String,
) -> std::result::Result<HashMap<String, String>, PebbleError> {
    let mut notes = load_kanban_context_notes_for_state(&state)?;
    let message_id = message_id.trim().to_string();
    if message_id.trim().is_empty() || note.is_empty() {
        notes.remove(&message_id);
    } else {
        notes.insert(message_id, note);
    }
    replace_kanban_context_notes_for_state(&state, notes)
}

#[tauri::command]
pub async fn merge_kanban_context_notes(
    state: State<'_, AppState>,
    notes: HashMap<String, String>,
) -> std::result::Result<HashMap<String, String>, PebbleError> {
    let mut current = load_kanban_context_notes_for_state(&state)?;
    for (message_id, note) in normalize_context_notes(notes) {
        current.entry(message_id).or_insert(note);
    }
    replace_kanban_context_notes_for_state(&state, current)
}
