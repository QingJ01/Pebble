use crate::state::AppState;
use pebble_core::{
    traits::DraftProvider, Attachment, DraftMessage, EmailAddress, FolderRole, PebbleError,
    ProviderType,
};
use std::path::Path;
use tauri::State;
use tracing::warn;

use super::compose::validate_attachment_paths;
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

fn validate_existing_local_draft_account(
    store: &pebble_store::Store,
    account_id: &str,
    existing_draft_id: Option<&str>,
) -> std::result::Result<(), PebbleError> {
    let Some(draft_id) = existing_draft_id else {
        return Ok(());
    };

    let Some(existing) = store.get_message(draft_id)? else {
        // Remote provider draft IDs may not have a local mirror yet.
        return Ok(());
    };

    if existing.account_id != account_id || !existing.is_draft {
        return Err(PebbleError::Validation(
            "Existing draft does not belong to the selected account".to_string(),
        ));
    }

    Ok(())
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
    attachment_paths: Option<Vec<String>>,
    existing_draft_id: Option<String>,
) -> std::result::Result<String, PebbleError> {
    let raw_attachment_paths = attachment_paths.unwrap_or_default();
    let attachment_paths = if raw_attachment_paths.is_empty() {
        raw_attachment_paths
    } else {
        validate_attachment_paths(&raw_attachment_paths, &state.attachments_dir)?
    };
    validate_existing_local_draft_account(
        state.store.as_ref(),
        &account_id,
        existing_draft_id.as_deref(),
    )?;
    let draft = DraftMessage {
        id: existing_draft_id.clone(),
        to: to.into_iter().map(|a| EmailAddress { name: None, address: a }).collect(),
        cc: cc.into_iter().map(|a| EmailAddress { name: None, address: a }).collect(),
        bcc: bcc.into_iter().map(|a| EmailAddress { name: None, address: a }).collect(),
        subject,
        body_text,
        body_html,
        in_reply_to,
        attachment_paths,
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
    let attachment_records = draft_attachment_records(&id, &draft.attachment_paths);

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
        has_attachments: !attachment_records.is_empty(),
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
    state.store.replace_message_with_attachments(&msg, &folder_ids, &attachment_records)?;
    Ok(id)
}

fn draft_attachment_records(draft_id: &str, attachment_paths: &[String]) -> Vec<Attachment> {
    attachment_paths
        .iter()
        .map(|path| {
            let filename = Path::new(path)
                .file_name()
                .and_then(|name| name.to_str())
                .unwrap_or("attachment")
                .to_string();
            let size = std::fs::metadata(path).map(|metadata| metadata.len() as i64).unwrap_or(0);
            Attachment {
                id: pebble_core::new_id(),
                message_id: draft_id.to_string(),
                filename,
                mime_type: "application/octet-stream".to_string(),
                size,
                local_path: Some(path.clone()),
                content_id: None,
                is_inline: false,
            }
        })
        .collect()
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
    use super::{
        requires_remote_draft_delete, should_delete_local_draft,
        validate_existing_local_draft_account,
    };
    use pebble_core::{
        new_id, now_timestamp, Account, Message, PebbleError, ProviderType,
    };
    use pebble_store::Store;

    fn make_account(id: &str, email: &str) -> Account {
        let now = now_timestamp();
        Account {
            id: id.to_string(),
            email: email.to_string(),
            display_name: email.to_string(),
            provider: ProviderType::Imap,
            created_at: now,
            updated_at: now,
        }
    }

    fn make_draft(account_id: &str, id: &str) -> Message {
        let now = now_timestamp();
        Message {
            id: id.to_string(),
            account_id: account_id.to_string(),
            remote_id: String::new(),
            message_id_header: None,
            in_reply_to: None,
            references_header: None,
            thread_id: None,
            subject: "Draft".to_string(),
            snippet: "Draft body".to_string(),
            from_address: String::new(),
            from_name: String::new(),
            to_list: Vec::new(),
            cc_list: Vec::new(),
            bcc_list: Vec::new(),
            body_text: "Draft body".to_string(),
            body_html_raw: String::new(),
            has_attachments: false,
            is_read: true,
            is_starred: false,
            is_draft: true,
            date: now,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        }
    }

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

    #[test]
    fn existing_local_draft_validation_rejects_cross_account_draft_id() {
        let store = Store::open_in_memory().unwrap();
        let account_a = make_account("account-a", "a@example.com");
        let account_b = make_account("account-b", "b@example.com");
        store.insert_account(&account_a).unwrap();
        store.insert_account(&account_b).unwrap();
        let draft_id = new_id();
        let draft = make_draft(&account_a.id, &draft_id);
        store.insert_message(&draft, &[]).unwrap();

        let err =
            validate_existing_local_draft_account(&store, &account_b.id, Some(&draft_id))
                .unwrap_err();

        assert!(matches!(err, PebbleError::Validation(_)));
    }

    #[test]
    fn existing_local_draft_validation_allows_same_account_draft_id() {
        let store = Store::open_in_memory().unwrap();
        let account = make_account("account-a", "a@example.com");
        store.insert_account(&account).unwrap();
        let draft_id = new_id();
        let draft = make_draft(&account.id, &draft_id);
        store.insert_message(&draft, &[]).unwrap();

        validate_existing_local_draft_account(&store, &account.id, Some(&draft_id)).unwrap();
    }

    #[test]
    fn existing_local_draft_validation_allows_missing_local_record() {
        let store = Store::open_in_memory().unwrap();

        validate_existing_local_draft_account(&store, "account-a", Some("remote-draft-1"))
            .unwrap();
    }
}
