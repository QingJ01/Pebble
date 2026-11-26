use crate::state::AppState;
use pebble_core::traits::SearchHit;
use pebble_core::PebbleError;
use pebble_search::AdvancedSearchParams;
use serde::Deserialize;
use tauri::State;

#[derive(Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
#[cfg_attr(test, ts(optional_fields = nullable))]
pub struct AdvancedSearchQuery {
    pub text: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub subject: Option<String>,
    pub date_from: Option<i64>,
    pub date_to: Option<i64>,
    pub has_attachment: Option<bool>,
    pub folder_id: Option<String>,
}

#[tauri::command]
pub async fn advanced_search(
    state: State<'_, AppState>,
    query: AdvancedSearchQuery,
    limit: Option<usize>,
) -> std::result::Result<Vec<SearchHit>, PebbleError> {
    let search = state.search.clone();
    let store = state.store.clone();
    let limit = limit.unwrap_or(50);
    tokio::task::spawn_blocking(move || {
        let hits = search.advanced_search(AdvancedSearchParams {
            text: query.text.as_deref(),
            from: query.from.as_deref(),
            to: query.to.as_deref(),
            subject: query.subject.as_deref(),
            date_from: query.date_from,
            date_to: query.date_to,
            has_attachment: query.has_attachment,
            folder_id: query.folder_id.as_deref(),
            limit,
        })?;
        super::search::attach_account_ids(&store, hits)
    })
    .await
    .map_err(|e| PebbleError::Internal(format!("Task join error: {e}")))?
}
