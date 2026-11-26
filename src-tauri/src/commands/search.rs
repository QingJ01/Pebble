use crate::state::AppState;
use pebble_core::traits::SearchHit;
use pebble_core::{PebbleError, Result};
use pebble_store::Store;
use tauri::State;

/// Tag each hit with its owning account (the index does not store it), so
/// result lists spanning several accounts can badge rows by account.
pub(crate) fn attach_account_ids(
    store: &Store,
    mut hits: Vec<SearchHit>,
) -> Result<Vec<SearchHit>> {
    let ids: Vec<String> = hits.iter().map(|hit| hit.message_id.clone()).collect();
    let mut account_ids = store.get_message_account_ids_batch(&ids)?;
    for hit in &mut hits {
        hit.account_id = account_ids.remove(&hit.message_id);
    }
    Ok(hits)
}

#[tauri::command]
pub async fn search_messages(
    state: State<'_, AppState>,
    query: String,
    limit: Option<usize>,
) -> std::result::Result<Vec<SearchHit>, PebbleError> {
    let limit = limit.unwrap_or(50);
    let search = state.search.clone();
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let hits = search.search(&query, limit)?;
        attach_account_ids(&store, hits)
    })
    .await
    .map_err(|e| PebbleError::Internal(format!("Task join error: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::attach_account_ids;
    use pebble_core::traits::SearchHit;
    use pebble_core::{Account, Message, ProviderType};
    use pebble_store::Store;
    use serde_json::json;

    fn hit(message_id: &str) -> SearchHit {
        SearchHit {
            message_id: message_id.into(),
            score: 1.0,
            snippet: String::new(),
            subject: None,
            from_address: None,
            from_name: None,
            date: None,
            account_id: None,
        }
    }

    #[test]
    fn attach_account_ids_tags_hits_with_their_owning_account() {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_account(&Account {
                id: "acct-b".into(),
                email: "b@example.invalid".into(),
                display_name: "B".into(),
                account_label: None,
                provider_display_name: None,
                color: None,
                provider: ProviderType::Imap,
                created_at: 1,
                updated_at: 1,
            })
            .unwrap();
        let message: Message = serde_json::from_value(json!({"id":"m-1","account_id":"acct-b","remote_id":"5","message_id_header":"<m1@example.invalid>","in_reply_to":null,"references_header":null,"thread_id":null,"subject":"Test","snippet":"","from_address":"x@example.invalid","from_name":"X","to_list":[],"cc_list":[],"bcc_list":[],"body_text":"","body_html_raw":"","has_attachments":false,"is_read":false,"is_starred":false,"is_draft":false,"date":1,"remote_version":null,"is_deleted":false,"deleted_at":null,"created_at":1,"updated_at":1})).unwrap();
        store.insert_message(&message, &[]).unwrap();

        let hits = attach_account_ids(&store, vec![hit("m-1"), hit("gone")]).unwrap();

        assert_eq!(hits[0].account_id.as_deref(), Some("acct-b"));
        assert_eq!(hits[1].account_id, None);
    }
}
