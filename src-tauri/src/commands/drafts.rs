use crate::state::AppState;
use pebble_core::{
    traits::DraftProvider, DraftMessage, EmailAddress, FolderRole, PebbleError, ProviderType,
};
use tauri::State;
use tracing::warn;

use super::messages::provider_dispatch::ConnectedProvider;

fn requires_remote_draft_delete(provider_type: Option<ProviderType>) -> bool {
    matches!(
        provider_type,
        Some(ProviderType::Gmail | ProviderType::Outlook)
    )
}

fn should_delete_local_draft(
    provider_type: Option<ProviderType>,
    remote_delete_confirmed: bool,
) -> bool {
    !requires_remote_draft_delete(provider_type) || remote_delete_confirmed
}

fn hard_delete_local_draft(state: &AppState, draft_id: &str) {
    if let Err(e) = state.store.hard_delete_messages(&[draft_id.to_string()]) {
        warn!("Failed to delete local draft {draft_id}: {e}");
    }
}

#[tauri::command]
pub async fn save_draft(
    state: State<'_, AppState>,
    account_id: String,
    to: Vec<String>,
    cc: Vec<String>,
    bcc: Vec<String>,
    subject: String,
    body_text: String,
    body_html: Option<String>,
    in_reply_to: Option<String>,
    existing_draft_id: Option<String>,
) -> std::result::Result<String, PebbleError> {
    let draft = DraftMessage {
        id: existing_draft_id.clone(),
        to: to.into_iter().map(|a| EmailAddress { name: None, address: a }).collect(),
        cc: cc.into_iter().map(|a| EmailAddress { name: None, address: a }).collect(),
        bcc: bcc.into_iter().map(|a| EmailAddress { name: None, address: a }).collect(),
        subject,
        body_text,
        body_html,
        in_reply_to,
    };

    let provider_type = state.store.get_account(&account_id)?
        .map(|a| a.provider);

    match provider_type {
        Some(pt) => {
            if let Ok(conn) = ConnectedProvider::connect(&state, &account_id, &pt).await {
                let result = match (&conn, &existing_draft_id) {
                    (ConnectedProvider::Gmail(p), Some(did)) => {
                        p.update_draft(did, &draft).await.map(|_| did.clone())
                    }
                    (ConnectedProvider::Gmail(p), None) => p.save_draft(&draft).await,
                    (ConnectedProvider::Outlook(p), Some(did)) => {
                        p.update_draft(did, &draft).await.map(|_| did.clone())
                    }
                    (ConnectedProvider::Outlook(p), None) => p.save_draft(&draft).await,
                    _ => {
                        // IMAP — fall back to local-only
                        save_draft_locally(&state, &account_id, &draft)
                    }
                };
                conn.disconnect().await;
                result
            } else {
                save_draft_locally(&state, &account_id, &draft)
            }
        }
        None => save_draft_locally(&state, &account_id, &draft),
    }
}

fn save_draft_locally(
    state: &AppState,
    account_id: &str,
    draft: &DraftMessage,
) -> std::result::Result<String, PebbleError> {
    let id = draft.id.clone().unwrap_or_else(pebble_core::new_id);

    // Delete any existing draft with this ID to implement upsert semantics
    if draft.id.is_some() {
        let _ = state.store.hard_delete_messages(&[id.clone()]);
    }

    let msg = pebble_core::Message {
        id: id.clone(),
        account_id: account_id.to_string(),
        remote_id: String::new(),
        message_id_header: None,
        in_reply_to: draft.in_reply_to.clone(),
        references_header: None,
        thread_id: None,
        subject: draft.subject.clone(),
        snippet: draft.body_text.chars().take(200).collect(),
        from_address: String::new(),
        from_name: String::new(),
        to_list: draft.to.clone(),
        cc_list: draft.cc.clone(),
        bcc_list: draft.bcc.clone(),
        body_text: draft.body_text.clone(),
        body_html_raw: draft.body_html.clone().unwrap_or_default(),
        has_attachments: false,
        is_read: true,
        is_starred: false,
        is_draft: true,
        date: pebble_core::now_timestamp(),
        remote_version: None,
        is_deleted: false,
        deleted_at: None,
        created_at: pebble_core::now_timestamp(),
        updated_at: pebble_core::now_timestamp(),
    };
    // Attach the draft to the account's Drafts folder if one exists, so it
    // shows up in the Drafts view. Falls back to no-folder for accounts
    // without a Drafts folder (e.g. brand-new IMAP account that hasn't yet
    // synced folder structure).
    let folder_ids: Vec<String> = match state.store.find_folder_by_role(account_id, FolderRole::Drafts) {
        Ok(Some(f)) => vec![f.id],
        _ => Vec::new(),
    };
    state.store.insert_message(&msg, &folder_ids)?;
    Ok(id)
}

#[tauri::command]
pub async fn delete_draft(
    state: State<'_, AppState>,
    account_id: String,
    draft_id: String,
) -> std::result::Result<(), PebbleError> {
    let provider_type = state.store.get_account(&account_id)?.map(|a| a.provider);

    if should_delete_local_draft(provider_type.clone(), false) {
        hard_delete_local_draft(&state, &draft_id);
        return Ok(());
    }

    let mut remote_ok = true;
    if let Some(pt) = provider_type.clone() {
        if let Ok(conn) = ConnectedProvider::connect(&state, &account_id, &pt).await {
            let result = match &conn {
                ConnectedProvider::Gmail(p) => p.delete_draft(&draft_id).await,
                ConnectedProvider::Outlook(p) => p.delete_draft(&draft_id).await,
                _ => Ok(()),
            };
            conn.disconnect().await;
            if let Err(e) = result {
                warn!("Failed to delete remote draft: {e}");
                remote_ok = false;
            }
        } else {
            // Gmail and Outlook drafts have provider-side draft records. Keep
            // the local draft when the remote delete cannot be confirmed.
            remote_ok = false;
        }
    }
    if should_delete_local_draft(provider_type, remote_ok) {
        hard_delete_local_draft(&state, &draft_id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{requires_remote_draft_delete, should_delete_local_draft};
    use pebble_core::ProviderType;

    #[test]
    fn draft_delete_does_not_require_remote_delete_for_local_or_imap() {
        assert!(!requires_remote_draft_delete(None));
        assert!(!requires_remote_draft_delete(Some(ProviderType::Imap)));
    }

    #[test]
    fn draft_delete_requires_remote_delete_for_oauth_providers() {
        assert!(requires_remote_draft_delete(Some(ProviderType::Gmail)));
        assert!(requires_remote_draft_delete(Some(ProviderType::Outlook)));
    }

    #[test]
    fn draft_delete_local_decision_skips_remote_requirement_for_local_and_imap() {
        assert!(should_delete_local_draft(None, false));
        assert!(should_delete_local_draft(Some(ProviderType::Imap), false));
    }

    #[test]
    fn draft_delete_local_decision_requires_remote_confirmation_for_oauth_providers() {
        assert!(!should_delete_local_draft(Some(ProviderType::Gmail), false));
        assert!(!should_delete_local_draft(Some(ProviderType::Outlook), false));
        assert!(should_delete_local_draft(Some(ProviderType::Gmail), true));
        assert!(should_delete_local_draft(Some(ProviderType::Outlook), true));
    }
}
