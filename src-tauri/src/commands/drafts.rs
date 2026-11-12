use crate::state::AppState;
use pebble_core::{
    traits::DraftProvider, DraftMessage, EmailAddress, FolderRole, PebbleError, ProviderType,
};
use tauri::State;
use tracing::warn;

use super::attachments::{cleanup_staged_attachment_records, stage_local_attachment_records};
use super::compose::validate_attachment_paths;
use super::messages::provider_dispatch::ConnectedProvider;

#[derive(Debug, Clone, PartialEq, Eq)]
struct DraftProvenance {
    local_id: Option<String>,
    remote_id: Option<String>,
}

const INLINE_DRAFT_UNSUPPORTED: &str = "draft_inline_attachments_unsupported";

fn ensure_draft_editable(store: &pebble_store::Store, draft_id: &str) -> Result<(), PebbleError> {
    let has_cid_body = store
        .get_message(draft_id)?
        .is_some_and(|message| message.body_html_raw.to_ascii_lowercase().contains("cid:"));
    if has_cid_body
        || store
            .list_attachments_by_message(draft_id)?
            .iter()
            .any(|attachment| attachment.is_inline || attachment.content_id.is_some())
    {
        return Err(PebbleError::Validation(INLINE_DRAFT_UNSUPPORTED.into()));
    }
    Ok(())
}

trait RemoteDraftOperations {
    async fn create_draft(&self, draft: &DraftMessage) -> Result<RemoteDraftIdentity, PebbleError>;
    async fn update_draft(
        &self,
        draft_id: &str,
        draft: &DraftMessage,
    ) -> Result<RemoteDraftIdentity, PebbleError>;
    async fn delete_draft(&self, draft_id: &str) -> Result<(), PebbleError>;
}

impl RemoteDraftOperations for ConnectedProvider {
    async fn create_draft(&self, draft: &DraftMessage) -> Result<RemoteDraftIdentity, PebbleError> {
        match self {
            Self::Gmail(provider) => {
                provider
                    .save_draft_with_message_id(draft)
                    .await
                    .map(|(draft_id, message_id)| RemoteDraftIdentity {
                        draft_id,
                        message_id,
                    })
            }
            Self::Outlook(provider) => {
                provider
                    .save_draft(draft)
                    .await
                    .map(|draft_id| RemoteDraftIdentity {
                        message_id: Some(draft_id.clone()),
                        draft_id,
                    })
            }
            Self::Imap(_) => Err(PebbleError::UnsupportedProvider(
                "IMAP remote drafts are not supported".to_string(),
            )),
        }
    }

    async fn update_draft(
        &self,
        draft_id: &str,
        draft: &DraftMessage,
    ) -> Result<RemoteDraftIdentity, PebbleError> {
        match self {
            Self::Gmail(provider) => provider
                .update_draft_with_message_id(draft_id, draft)
                .await
                .map(|message_id| RemoteDraftIdentity {
                    draft_id: draft_id.into(),
                    message_id,
                }),
            Self::Outlook(provider) => {
                provider
                    .update_draft(draft_id, draft)
                    .await
                    .map(|()| RemoteDraftIdentity {
                        draft_id: draft_id.into(),
                        message_id: Some(draft_id.into()),
                    })
            }
            Self::Imap(_) => Err(PebbleError::UnsupportedProvider(
                "IMAP remote drafts are not supported".to_string(),
            )),
        }
    }

    async fn delete_draft(&self, draft_id: &str) -> Result<(), PebbleError> {
        match self {
            Self::Gmail(provider) => provider.delete_draft(draft_id).await,
            Self::Outlook(provider) => provider.delete_draft(draft_id).await,
            Self::Imap(_) => Ok(()),
        }
    }
}

struct RemoteDraftIdentity {
    draft_id: String,
    message_id: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize)]
struct DraftEditorMetadata {
    editor_mode: Option<String>,
    remote_draft_id: Option<String>,
}

fn draft_editor_key(account_id: &str, identity_type: &str, id: &str) -> String {
    format!(
        "draft-editor:{}",
        serde_json::json!([account_id, identity_type, id])
    )
}

fn store_draft_editor_metadata(
    state: &AppState,
    key: &str,
    mode: Option<&str>,
    remote_draft_id: Option<&str>,
) -> Result<(), PebbleError> {
    let editor_mode = mode
        .filter(|mode| matches!(*mode, "rich" | "markdown" | "html"))
        .map(str::to_owned);
    if editor_mode.is_none() && remote_draft_id.is_none() {
        return Ok(());
    }
    let metadata = DraftEditorMetadata {
        editor_mode,
        remote_draft_id: remote_draft_id.map(str::to_owned),
    };
    let value =
        serde_json::to_vec(&metadata).map_err(|error| PebbleError::Internal(error.to_string()))?;
    super::encrypted_store::store_secure_user_data(&state.crypto, &state.store, key, &value)
}

fn load_draft_editor_metadata(
    state: &AppState,
    message: &pebble_core::Message,
) -> Result<Option<DraftEditorMetadata>, PebbleError> {
    let mut keys = vec![draft_editor_key(&message.account_id, "local", &message.id)];
    if !message.remote_id.is_empty() {
        keys.push(draft_editor_key(
            &message.account_id,
            "remote",
            &message.remote_id,
        ));
    }
    // Only local UUIDs can safely read legacy unscoped metadata.
    keys.push(format!("draft-editor:{}", message.id));
    for key in keys {
        let Some(bytes) =
            super::encrypted_store::load_secure_user_data(&state.crypto, &state.store, &key)?
        else {
            continue;
        };
        if let Ok(metadata) = serde_json::from_slice::<DraftEditorMetadata>(&bytes) {
            return Ok(Some(metadata));
        }
        if let Ok(mode) = String::from_utf8(bytes) {
            if matches!(mode.as_str(), "rich" | "markdown" | "html") {
                return Ok(Some(DraftEditorMetadata {
                    editor_mode: Some(mode),
                    remote_draft_id: None,
                }));
            }
        }
    }
    Ok(None)
}

fn resolve_editable_draft_provenance(
    state: &AppState,
    account_id: &str,
    draft_id: Option<&str>,
) -> Result<DraftProvenance, PebbleError> {
    let mut provenance = resolve_draft_provenance(&state.store, account_id, draft_id)?;
    if let Some(id) = provenance.local_id.as_deref() {
        if let Some(message) = state.store.get_message(id)? {
            if let Some(remote_id) = load_draft_editor_metadata(state, &message)?
                .and_then(|metadata| metadata.remote_draft_id)
            {
                provenance.remote_id = Some(remote_id);
            }
        }
    }
    Ok(provenance)
}

fn requires_remote_draft_delete(provider_type: Option<ProviderType>) -> bool {
    matches!(
        provider_type,
        Some(ProviderType::Gmail | ProviderType::Outlook)
    )
}

fn resolve_draft_provenance(
    store: &pebble_store::Store,
    account_id: &str,
    existing_draft_id: Option<&str>,
) -> std::result::Result<DraftProvenance, PebbleError> {
    let Some(draft_id) = existing_draft_id else {
        return Ok(DraftProvenance {
            local_id: None,
            remote_id: None,
        });
    };

    let Some(existing) = store.get_message(draft_id)? else {
        return Ok(DraftProvenance {
            local_id: None,
            remote_id: Some(draft_id.to_string()),
        });
    };

    if existing.account_id != account_id || !existing.is_draft {
        return Err(PebbleError::Validation(
            "Existing draft does not belong to the selected account".to_string(),
        ));
    }

    Ok(DraftProvenance {
        local_id: Some(existing.id),
        remote_id: (!existing.remote_id.is_empty()).then_some(existing.remote_id),
    })
}

fn cleanup_unreferenced_local_attachment_paths(state: &AppState, local_paths: &[String]) {
    for path in local_paths {
        match state.store.is_attachment_local_path_referenced(path) {
            Ok(true) => continue,
            Ok(false) => {}
            Err(error) => {
                warn!("Failed to check local attachment reference {path}: {error}");
                continue;
            }
        }
        if let Err(error) = std::fs::remove_file(path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                warn!("Failed to delete local draft attachment {path}: {error}");
            }
        }
        if let Some(parent) = std::path::Path::new(path).parent() {
            let _ = std::fs::remove_dir(parent);
        }
    }
}

fn delete_local_draft(state: &AppState, draft_id: &str) -> Result<(), PebbleError> {
    let message = state.store.get_message(draft_id)?;
    let local_paths: Vec<String> = state
        .store
        .list_attachments_by_message(draft_id)?
        .into_iter()
        .filter_map(|attachment| attachment.local_path)
        .collect();
    state.store.hard_delete_messages(&[draft_id.to_string()])?;
    if let Some(message) = message {
        state.store.delete_secure_user_data(&draft_editor_key(
            &message.account_id,
            "local",
            draft_id,
        ))?;
    }
    state
        .store
        .delete_secure_user_data(&format!("draft-editor:{draft_id}"))?;
    cleanup_unreferenced_local_attachment_paths(state, &local_paths);
    Ok(())
}

#[tauri::command]
#[allow(clippy::too_many_arguments)]
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
    editor_mode: Option<String>,
) -> std::result::Result<String, PebbleError> {
    let raw_attachment_paths = attachment_paths.unwrap_or_default();
    let attachment_paths = if raw_attachment_paths.is_empty() {
        raw_attachment_paths
    } else {
        validate_attachment_paths(&raw_attachment_paths, &state.attachments_dir)?
    };
    let provenance =
        resolve_editable_draft_provenance(&state, &account_id, existing_draft_id.as_deref())?;
    let account = state
        .store
        .get_account(&account_id)?
        .ok_or_else(|| PebbleError::Validation("Account not found".into()))?;
    if let Some(id) = provenance.local_id.as_deref() {
        ensure_draft_editable(&state.store, id)?;
    }
    let sender = account.sender_identity();
    pebble_mail::sender::sender_mailbox(&sender)?;
    let draft = DraftMessage {
        from: Some(sender),
        id: provenance.remote_id.clone(),
        to: to
            .into_iter()
            .map(|a| EmailAddress {
                name: None,
                address: a,
            })
            .collect(),
        cc: cc
            .into_iter()
            .map(|a| EmailAddress {
                name: None,
                address: a,
            })
            .collect(),
        bcc: bcc
            .into_iter()
            .map(|a| EmailAddress {
                name: None,
                address: a,
            })
            .collect(),
        subject,
        body_text,
        body_html,
        in_reply_to,
        attachment_paths,
    };

    let saved_id = if matches!(
        account.provider,
        ProviderType::Gmail | ProviderType::Outlook
    ) {
        // Reuse the verified connection itself; a second connect could read different credentials.
        let prepared = super::compose::prepare_send_transport(&state, &account)
            .await
            .and_then(|(transport, sender)| {
                use super::compose::PreparedSendTransport;
                let remote = match transport {
                    PreparedSendTransport::Gmail(provider) => ConnectedProvider::Gmail(provider),
                    PreparedSendTransport::Outlook(provider) => {
                        ConnectedProvider::Outlook(provider)
                    }
                    PreparedSendTransport::Smtp(_) => {
                        return Err(PebbleError::UnsupportedProvider(
                            "Remote drafts require OAuth".into(),
                        ))
                    }
                };
                Ok((remote, sender))
            });
        save_prepared_oauth_draft(
            &state,
            &account_id,
            &draft,
            &provenance,
            prepared,
            editor_mode.as_deref(),
        )
        .await
    } else {
        save_local_draft_with_metadata(
            &state,
            &account_id,
            &draft,
            &provenance,
            editor_mode.as_deref(),
        )
    }?;
    Ok(saved_id)
}

#[derive(serde::Serialize)]
pub struct DraftContent {
    message: pebble_core::Message,
    attachments: Vec<pebble_core::Attachment>,
    editor_mode: String,
}

#[tauri::command]
pub async fn get_draft(
    state: State<'_, AppState>,
    draft_id: String,
) -> Result<DraftContent, PebbleError> {
    load_draft_content(&state, &draft_id)
}

fn load_draft_content(state: &AppState, draft_id: &str) -> Result<DraftContent, PebbleError> {
    let message = state
        .store
        .get_message(draft_id)?
        .filter(|message| message.is_draft)
        .ok_or_else(|| PebbleError::Validation("Draft not found".into()))?;
    ensure_draft_editable(&state.store, draft_id)?;
    let attachments = state.store.list_attachments_by_message(draft_id)?;
    if attachments.iter().any(|attachment| {
        !attachment.is_inline
            && attachment
                .local_path
                .as_deref()
                .is_none_or(|path| !std::path::Path::new(path).is_file())
    }) {
        return Err(PebbleError::Validation(
            "Draft attachments are not available locally; sync the account first".into(),
        ));
    }
    let editor_mode = load_draft_editor_metadata(state, &message)?
        .and_then(|metadata| metadata.editor_mode)
        .filter(|mode| matches!(mode.as_str(), "rich" | "markdown" | "html"))
        .unwrap_or_else(|| {
            if message.body_html_raw.is_empty() {
                "markdown"
            } else {
                "rich"
            }
            .into()
        });
    let source_paths: Vec<String> = attachments
        .iter()
        .filter(|attachment| !attachment.is_inline)
        .filter_map(|attachment| attachment.local_path.clone())
        .collect();
    let source_paths = validate_attachment_paths(&source_paths, &state.attachments_dir)?;
    let attachments =
        stage_local_attachment_records(&state.attachments_dir, "compose_staging", &source_paths)?;
    Ok(DraftContent {
        message,
        attachments,
        editor_mode,
    })
}

pub(crate) fn delete_local_only_draft(
    state: &AppState,
    message: &pebble_core::Message,
) -> Result<bool, PebbleError> {
    if !message.is_draft || !message.remote_id.is_empty() {
        return Ok(false);
    }
    delete_local_draft(state, &message.id)?;
    super::messages::remove_search_documents(state, std::slice::from_ref(&message.id))?;
    Ok(true)
}

async fn save_prepared_oauth_draft<R: RemoteDraftOperations>(
    state: &AppState,
    account_id: &str,
    draft: &DraftMessage,
    provenance: &DraftProvenance,
    prepared: Result<(R, EmailAddress), PebbleError>,
    editor_mode: Option<&str>,
) -> Result<String, PebbleError> {
    match prepared {
        Ok((remote, sender)) => {
            let mut verified_draft = draft.clone();
            verified_draft.from = Some(sender);
            save_oauth_draft_with_fallback(
                state,
                account_id,
                &verified_draft,
                provenance,
                &remote,
                editor_mode,
            )
            .await
        }
        Err(error) => {
            warn!("Draft identity verification failed; preserving local draft: {error}");
            save_local_draft_with_metadata(state, account_id, draft, provenance, editor_mode)
        }
    }
}

async fn save_oauth_draft_with_fallback<R: RemoteDraftOperations>(
    state: &AppState,
    account_id: &str,
    draft: &DraftMessage,
    provenance: &DraftProvenance,
    remote: &R,
    editor_mode: Option<&str>,
) -> Result<String, PebbleError> {
    if let Some(id) = provenance.local_id.as_deref() {
        ensure_draft_editable(&state.store, id)?;
    }
    let remote_result = if let Some(remote_id) = provenance.remote_id.as_deref() {
        remote.update_draft(remote_id, draft).await
    } else {
        remote.create_draft(draft).await
    };

    match remote_result {
        Ok(identity) => {
            let remote_id = identity.draft_id;
            let message_id = identity.message_id.as_deref().unwrap_or(&remote_id);
            store_draft_editor_metadata(
                state,
                &draft_editor_key(account_id, "remote", message_id),
                editor_mode,
                Some(&remote_id),
            )?;
            if let Some(local_id) = provenance.local_id.as_deref() {
                if let Err(error) = delete_local_draft(state, local_id) {
                    warn!(
                        "Remote draft {remote_id} was saved, but local fallback {local_id} could not be deleted: {error}"
                    );
                }
            }
            Ok(remote_id)
        }
        Err(error) => {
            warn!("Remote draft save failed; preserving encrypted local fallback: {error}");
            save_local_draft_with_metadata(state, account_id, draft, provenance, editor_mode)
        }
    }
}

fn save_local_draft_with_metadata(
    state: &AppState,
    account_id: &str,
    draft: &DraftMessage,
    provenance: &DraftProvenance,
    editor_mode: Option<&str>,
) -> Result<String, PebbleError> {
    let id = save_draft_locally(
        state,
        account_id,
        draft,
        provenance.local_id.as_deref(),
        provenance.remote_id.as_deref(),
    )?;
    store_draft_editor_metadata(
        state,
        &draft_editor_key(account_id, "local", &id),
        editor_mode,
        provenance.remote_id.as_deref(),
    )?;
    Ok(id)
}

fn save_draft_locally(
    state: &AppState,
    account_id: &str,
    draft: &DraftMessage,
    existing_local_id: Option<&str>,
    remote_draft_id: Option<&str>,
) -> std::result::Result<String, PebbleError> {
    if let Some(id) = existing_local_id {
        ensure_draft_editable(&state.store, id)?;
    }
    let id = existing_local_id
        .map(ToOwned::to_owned)
        .unwrap_or_else(pebble_core::new_id);
    let previous_local_paths: Vec<String> = state
        .store
        .list_attachments_by_message(&id)?
        .into_iter()
        .filter_map(|attachment| attachment.local_path)
        .collect();
    let attachment_records =
        stage_local_attachment_records(&state.attachments_dir, &id, &draft.attachment_paths)?;

    let msg = pebble_core::Message {
        id: id.clone(),
        account_id: account_id.to_string(),
        remote_id: remote_draft_id.unwrap_or_default().to_string(),
        message_id_header: None,
        in_reply_to: draft.in_reply_to.clone(),
        references_header: None,
        thread_id: None,
        subject: draft.subject.clone(),
        snippet: draft.body_text.chars().take(200).collect(),
        from_address: draft
            .from
            .as_ref()
            .map(|from| from.address.clone())
            .unwrap_or_default(),
        from_name: draft
            .from
            .as_ref()
            .and_then(|from| from.name.clone())
            .unwrap_or_default(),
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
    let folder_ids: Vec<String> = match state
        .store
        .find_folder_by_role(account_id, FolderRole::Drafts)
    {
        Ok(Some(f)) => vec![f.id],
        _ => Vec::new(),
    };
    if let Err(error) =
        state
            .store
            .replace_message_with_attachments(&msg, &folder_ids, &attachment_records)
    {
        cleanup_staged_attachment_records(&attachment_records);
        return Err(error);
    }
    cleanup_unreferenced_local_attachment_paths(state, &previous_local_paths);
    Ok(id)
}

#[tauri::command]
pub async fn delete_draft(
    state: State<'_, AppState>,
    account_id: String,
    draft_id: String,
) -> std::result::Result<(), PebbleError> {
    let provenance = resolve_editable_draft_provenance(&state, &account_id, Some(&draft_id))?;
    let provider_type = state.store.get_account(&account_id)?.map(|a| a.provider);

    if requires_remote_draft_delete(provider_type.clone()) {
        if let Some(remote_id) = provenance.remote_id.as_deref() {
            let provider = provider_type.as_ref().ok_or_else(|| {
                PebbleError::Internal(
                    "OAuth draft deletion requires a persisted account provider".to_string(),
                )
            })?;
            let conn = ConnectedProvider::connect(&state, &account_id, provider)
                .await
                .map_err(|error| {
                    PebbleError::Network(format!(
                        "Could not connect to delete remote draft; local fallback was retained: {error}"
                    ))
                })?;
            let delete_result = conn.delete_draft(remote_id).await;
            conn.disconnect().await;
            delete_result?;
        }
    }

    if let Some(local_id) = provenance.local_id.as_deref() {
        delete_local_draft(&state, local_id)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_core::{new_id, now_timestamp, Account, Message, ProviderType};
    use pebble_crypto::CryptoService;
    use pebble_search::TantivySearch;
    use pebble_store::Store;
    use std::sync::Mutex;

    #[test]
    fn local_only_draft_deletion_needs_no_imap_uid_or_connection() {
        let state = test_state();
        let message = make_local_draft("account-a", "local-draft", "");
        state.store.insert_message(&message, &[]).unwrap();
        assert!(delete_local_only_draft(&state, &message).unwrap());
        assert!(state.store.get_message(&message.id).unwrap().is_none());
        let remote = make_local_draft("account-a", "remote-draft", "123");
        state.store.insert_message(&remote, &[]).unwrap();
        assert!(!delete_local_only_draft(&state, &remote).unwrap());
        assert!(state.store.get_message(&remote.id).unwrap().is_some());
    }

    #[test]
    fn resumed_draft_keeps_metadata_and_stable_attachment_sources_across_saves() {
        let state = test_state();
        std::fs::create_dir_all(&state.attachments_dir).unwrap();
        let source = state.attachments_dir.join("original.txt");
        std::fs::write(&source, b"payload").unwrap();
        let mut draft = draft_message("正文");
        draft.bcc.push(EmailAddress {
            name: None,
            address: "hidden@example.com".into(),
        });
        draft.in_reply_to = Some("<parent@example.com>".into());
        draft.body_text = "**正文**".into();
        draft.attachment_paths = vec![source.to_string_lossy().into_owned()];
        let id = save_draft_locally(&state, "account-a", &draft, None, None).unwrap();
        super::super::encrypted_store::store_secure_user_data(
            &state.crypto,
            &state.store,
            &format!("draft-editor:{id}"),
            b"markdown",
        )
        .unwrap();
        let loaded = load_draft_content(&state, &id).unwrap();
        assert_eq!(loaded.editor_mode, "markdown");
        assert_eq!(loaded.message.bcc_list[0].address, "hidden@example.com");
        assert_eq!(loaded.message.in_reply_to, draft.in_reply_to);
        let staged = loaded.attachments[0].local_path.clone().unwrap();
        assert!(std::path::Path::new(&staged)
            .starts_with(state.attachments_dir.join("compose_staging")));
        draft.attachment_paths = vec![staged.clone()];
        for subject in ["First edit", "Second edit"] {
            draft.subject = subject.into();
            assert_eq!(
                save_draft_locally(&state, "account-a", &draft, Some(&id), None).unwrap(),
                id
            );
            assert_eq!(std::fs::read(&staged).unwrap(), b"payload");
        }
        delete_local_draft(&state, &id).unwrap();
        assert!(state.store.get_message(&id).unwrap().is_none());
        assert!(state
            .store
            .get_secure_user_data(&format!("draft-editor:{id}"))
            .unwrap()
            .is_none());
        let _ = std::fs::remove_dir_all(&state.attachments_dir);
    }

    #[test]
    fn resume_rejects_missing_attachments_and_non_drafts() {
        let state = test_state();
        let mut message = make_local_draft("account-a", "draft", "");
        message.is_draft = false;
        state.store.insert_message(&message, &[]).unwrap();
        assert!(load_draft_content(&state, &message.id).is_err());
        message.id = "draft-with-missing-attachment".into();
        message.is_draft = true;
        state.store.insert_message(&message, &[]).unwrap();
        state
            .store
            .insert_attachment(&pebble_core::Attachment {
                id: new_id(),
                message_id: message.id.clone(),
                filename: "lost.txt".into(),
                mime_type: "text/plain".into(),
                size: 2,
                local_path: None,
                content_id: None,
                is_inline: false,
            })
            .unwrap();
        assert!(load_draft_content(&state, &message.id).is_err());
    }

    #[test]
    fn inline_drafts_cannot_be_resumed_or_overwritten_and_keep_original_payload() {
        for inline_flag in [true, false] {
            let state = test_state();
            std::fs::create_dir_all(&state.attachments_dir).unwrap();
            let path = state.attachments_dir.join("inline.png");
            std::fs::write(&path, b"original inline image").unwrap();
            let mut message = make_local_draft("account-a", "inline-draft", "");
            message.body_html_raw = "<img src=\"cid:original\">".into();
            state.store.insert_message(&message, &[]).unwrap();
            let attachment = pebble_core::Attachment {
                id: new_id(),
                message_id: message.id.clone(),
                filename: "inline.png".into(),
                mime_type: "image/png".into(),
                size: 21,
                local_path: Some(path.to_string_lossy().into_owned()),
                content_id: Some("original".into()),
                is_inline: inline_flag,
            };
            state.store.insert_attachment(&attachment).unwrap();
            assert!(
                matches!(load_draft_content(&state, &message.id), Err(PebbleError::Validation(error)) if error == INLINE_DRAFT_UNSUPPORTED)
            );
            assert!(save_draft_locally(
                &state,
                "account-a",
                &draft_message("replacement"),
                Some(&message.id),
                None
            )
            .is_err());
            let original = state.store.get_message(&message.id).unwrap().unwrap();
            assert_eq!(original.body_html_raw, message.body_html_raw);
            assert_eq!(
                state
                    .store
                    .list_attachments_by_message(&message.id)
                    .unwrap()[0]
                    .id,
                attachment.id
            );
            assert_eq!(std::fs::read(&path).unwrap(), b"original inline image");
            let _ = std::fs::remove_dir_all(&state.attachments_dir);
        }
    }

    #[test]
    fn cid_body_is_protected_even_when_attachment_records_are_missing() {
        let state = test_state();
        let mut message = make_local_draft("account-a", "cid-draft", "");
        message.body_html_raw = "<img src=\"CID:not-downloaded\">".into();
        state.store.insert_message(&message, &[]).unwrap();
        assert!(
            matches!(load_draft_content(&state, &message.id), Err(PebbleError::Validation(error)) if error == INLINE_DRAFT_UNSUPPORTED)
        );
        assert!(save_draft_locally(
            &state,
            "account-a",
            &draft_message("replacement"),
            Some(&message.id),
            None
        )
        .is_err());
        assert_eq!(
            state
                .store
                .get_message(&message.id)
                .unwrap()
                .unwrap()
                .body_html_raw,
            message.body_html_raw
        );
    }

    fn make_account(id: &str, email: &str, provider: ProviderType) -> Account {
        let now = now_timestamp();
        Account {
            account_label: None,
            provider_display_name: None,
            id: id.to_string(),
            email: email.to_string(),
            display_name: email.to_string(),
            color: None,
            provider,
            created_at: now,
            updated_at: now,
        }
    }

    fn make_local_draft(account_id: &str, id: &str, remote_id: &str) -> Message {
        let now = now_timestamp();
        Message {
            id: id.to_string(),
            account_id: account_id.to_string(),
            remote_id: remote_id.to_string(),
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

    fn draft_message(subject: &str) -> DraftMessage {
        DraftMessage {
            from: None,
            id: None,
            to: Vec::new(),
            cc: Vec::new(),
            bcc: Vec::new(),
            subject: subject.to_string(),
            body_text: format!("{subject} body"),
            body_html: None,
            in_reply_to: None,
            attachment_paths: Vec::new(),
        }
    }

    fn test_state() -> AppState {
        let store = Store::open_in_memory().unwrap();
        store
            .insert_account(&make_account(
                "account-a",
                "a@example.com",
                ProviderType::Gmail,
            ))
            .unwrap();
        let (snooze_stop_tx, _snooze_stop_rx) = std::sync::mpsc::channel();
        AppState::new(
            store,
            TantivySearch::open_in_memory().unwrap(),
            CryptoService::from_key([7; 32]),
            snooze_stop_tx,
            std::env::temp_dir().join(format!("pebble-draft-test-{}", new_id())),
        )
    }

    struct SuccessfulRemote {
        created_id: String,
        message_id: Option<String>,
        updated_ids: Mutex<Vec<String>>,
        created_from: std::sync::Arc<Mutex<Vec<Option<EmailAddress>>>>,
    }

    impl SuccessfulRemote {
        fn new(created_id: &str) -> Self {
            Self {
                created_id: created_id.to_string(),
                message_id: None,
                updated_ids: Mutex::new(Vec::new()),
                created_from: Default::default(),
            }
        }
    }

    impl RemoteDraftOperations for SuccessfulRemote {
        async fn create_draft(
            &self,
            draft: &DraftMessage,
        ) -> Result<RemoteDraftIdentity, PebbleError> {
            self.created_from.lock().unwrap().push(draft.from.clone());
            Ok(RemoteDraftIdentity {
                draft_id: self.created_id.clone(),
                message_id: Some(
                    self.message_id
                        .clone()
                        .unwrap_or_else(|| self.created_id.clone()),
                ),
            })
        }

        async fn update_draft(
            &self,
            draft_id: &str,
            _draft: &DraftMessage,
        ) -> Result<RemoteDraftIdentity, PebbleError> {
            self.updated_ids.lock().unwrap().push(draft_id.to_string());
            Ok(RemoteDraftIdentity {
                draft_id: draft_id.into(),
                message_id: Some(self.message_id.clone().unwrap_or_else(|| draft_id.into())),
            })
        }

        async fn delete_draft(&self, _draft_id: &str) -> Result<(), PebbleError> {
            Ok(())
        }
    }

    struct FailingRemote;

    #[tokio::test]
    async fn inline_oauth_drafts_are_rejected_before_any_remote_write_or_local_fallback() {
        let state = test_state();
        let mut message = make_local_draft("account-a", "inline-oauth", "gmail-message");
        message.body_html_raw = "<img src=\"cid:original\">".into();
        state.store.insert_message(&message, &[]).unwrap();
        let provenance =
            resolve_editable_draft_provenance(&state, "account-a", Some(&message.id)).unwrap();
        let remote = SuccessfulRemote::new("must-not-create");
        let error = save_oauth_draft_with_fallback(
            &state,
            "account-a",
            &draft_message("replacement"),
            &provenance,
            &remote,
            Some("html"),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(error, PebbleError::Validation(error) if error == INLINE_DRAFT_UNSUPPORTED)
        );
        assert!(remote.created_from.lock().unwrap().is_empty());
        assert!(remote.updated_ids.lock().unwrap().is_empty());
        assert_eq!(
            state
                .store
                .get_message(&message.id)
                .unwrap()
                .unwrap()
                .body_html_raw,
            message.body_html_raw
        );
        assert!(!state.attachments_dir.exists());
    }

    #[tokio::test]
    async fn oauth_modes_follow_synced_message_identity_on_create_and_update() {
        for provider in [ProviderType::Gmail, ProviderType::Outlook] {
            for mode in ["html", "markdown", "rich"] {
                let state = test_state();
                let account_id = if provider == ProviderType::Gmail {
                    "account-a"
                } else {
                    "account-outlook"
                };
                if provider == ProviderType::Outlook {
                    state
                        .store
                        .insert_account(&make_account(
                            account_id,
                            "outlook@example.com",
                            provider.clone(),
                        ))
                        .unwrap();
                }
                let draft_id = "remote-draft";
                let message_id = if provider == ProviderType::Gmail {
                    "gmail-message"
                } else {
                    draft_id
                };
                let mut remote = SuccessfulRemote::new(draft_id);
                remote.message_id = Some(message_id.into());
                let mut original = draft_message("original source");
                original.body_html =
                    Some("<table style=\"width:73%\"><tr><td>HTML source</td></tr></table>".into());
                original.body_text = "# Original Markdown source".into();
                let provenance = DraftProvenance {
                    local_id: None,
                    remote_id: None,
                };
                let saved = save_oauth_draft_with_fallback(
                    &state,
                    account_id,
                    &original,
                    &provenance,
                    &remote,
                    Some(mode),
                )
                .await
                .unwrap();
                assert_eq!(saved, draft_id);
                let mut synced = make_local_draft(account_id, &new_id(), message_id);
                synced.body_html_raw = original.body_html.clone().unwrap();
                synced.body_text = original.body_text.clone();
                state.store.insert_message(&synced, &[]).unwrap();
                assert_ne!(synced.id, saved);
                let loaded = load_draft_content(&state, &synced.id).unwrap();
                assert_eq!(loaded.editor_mode, mode);
                assert_eq!(
                    loaded.message.body_html_raw,
                    original.body_html.clone().unwrap()
                );
                assert_eq!(loaded.message.body_text, original.body_text);
                let provenance =
                    resolve_editable_draft_provenance(&state, account_id, Some(&synced.id))
                        .unwrap();
                assert_eq!(provenance.remote_id.as_deref(), Some(draft_id));

                let updated_message_id = if provider == ProviderType::Gmail {
                    "gmail-replaced-message"
                } else {
                    draft_id
                };
                remote.message_id = Some(updated_message_id.into());
                let updated_mode = match mode {
                    "html" => "markdown",
                    "markdown" => "rich",
                    _ => "html",
                };
                save_oauth_draft_with_fallback(
                    &state,
                    account_id,
                    &draft_message("updated"),
                    &provenance,
                    &remote,
                    Some(updated_mode),
                )
                .await
                .unwrap();
                assert_eq!(remote.updated_ids.lock().unwrap().as_slice(), &[draft_id]);
                let resynced = make_local_draft(account_id, &new_id(), updated_message_id);
                state.store.insert_message(&resynced, &[]).unwrap();
                assert_eq!(
                    load_draft_content(&state, &resynced.id)
                        .unwrap()
                        .editor_mode,
                    updated_mode
                );
                assert_eq!(
                    resolve_editable_draft_provenance(&state, account_id, Some(&resynced.id))
                        .unwrap()
                        .remote_id
                        .as_deref(),
                    Some(draft_id)
                );
            }
        }
    }

    #[tokio::test]
    async fn oauth_local_fallback_keeps_modes_through_promotion_and_offline_updates() {
        for mode in ["html", "markdown", "rich"] {
            let state = test_state();
            let provenance = DraftProvenance {
                local_id: None,
                remote_id: None,
            };
            let draft = draft_message("offline");
            let local_id = save_oauth_draft_with_fallback(
                &state,
                "account-a",
                &draft,
                &provenance,
                &FailingRemote,
                Some(mode),
            )
            .await
            .unwrap();
            assert_eq!(
                load_draft_content(&state, &local_id).unwrap().editor_mode,
                mode
            );
            let provenance =
                resolve_editable_draft_provenance(&state, "account-a", Some(&local_id)).unwrap();
            let mut remote = SuccessfulRemote::new("gmail-draft");
            remote.message_id = Some("gmail-message".into());
            save_oauth_draft_with_fallback(
                &state,
                "account-a",
                &draft,
                &provenance,
                &remote,
                Some(mode),
            )
            .await
            .unwrap();
            assert!(state.store.get_message(&local_id).unwrap().is_none());
            let synced = make_local_draft("account-a", &new_id(), "gmail-message");
            state.store.insert_message(&synced, &[]).unwrap();
            assert_eq!(
                load_draft_content(&state, &synced.id).unwrap().editor_mode,
                mode
            );
            let provenance =
                resolve_editable_draft_provenance(&state, "account-a", Some(&synced.id)).unwrap();
            let fallback = save_oauth_draft_with_fallback(
                &state,
                "account-a",
                &draft,
                &provenance,
                &FailingRemote,
                Some(mode),
            )
            .await
            .unwrap();
            let provenance =
                resolve_editable_draft_provenance(&state, "account-a", Some(&fallback)).unwrap();
            assert_eq!(provenance.remote_id.as_deref(), Some("gmail-draft"));
            assert_eq!(
                load_draft_content(&state, &fallback).unwrap().editor_mode,
                mode
            );
            remote.message_id = Some("gmail-new-message".into());
            save_oauth_draft_with_fallback(
                &state,
                "account-a",
                &draft,
                &provenance,
                &remote,
                Some(mode),
            )
            .await
            .unwrap();
            assert_eq!(
                remote.updated_ids.lock().unwrap().as_slice(),
                &["gmail-draft"]
            );
            let resynced = make_local_draft("account-a", &new_id(), "gmail-new-message");
            state.store.insert_message(&resynced, &[]).unwrap();
            assert_eq!(
                load_draft_content(&state, &resynced.id)
                    .unwrap()
                    .editor_mode,
                mode
            );
        }
    }

    #[test]
    fn editor_metadata_is_account_scoped_and_does_not_alias_local_and_remote_ids() {
        let state = test_state();
        state
            .store
            .insert_account(&make_account(
                "account-b",
                "b@example.com",
                ProviderType::Outlook,
            ))
            .unwrap();
        for (account, mode) in [("account-a", "html"), ("account-b", "markdown")] {
            let key = draft_editor_key(account, "remote", "same-remote-id");
            store_draft_editor_metadata(&state, &key, Some(mode), Some("same-draft-id")).unwrap();
            let message = make_local_draft(account, &new_id(), "same-remote-id");
            state.store.insert_message(&message, &[]).unwrap();
            assert_eq!(
                load_draft_content(&state, &message.id).unwrap().editor_mode,
                mode
            );
        }
        let first_account_again = make_local_draft("account-a", &new_id(), "same-remote-id");
        assert_eq!(
            load_draft_editor_metadata(&state, &first_account_again)
                .unwrap()
                .unwrap()
                .editor_mode
                .as_deref(),
            Some("html")
        );
        assert_ne!(
            draft_editor_key("account-a", "local", "same-remote-id"),
            draft_editor_key("account-a", "remote", "same-remote-id")
        );
        assert_ne!(
            draft_editor_key("a:b", "remote", "c"),
            draft_editor_key("a", "remote", "b:c")
        );
        let unrelated = make_local_draft("account-b", &new_id(), "other-id");
        state.store.insert_message(&unrelated, &[]).unwrap();
        assert!(load_draft_editor_metadata(&state, &unrelated)
            .unwrap()
            .is_none());
    }

    #[tokio::test]
    async fn prepared_draft_uses_verified_sender_from_the_same_connection() {
        let state = test_state();
        let draft = draft_message("Legacy draft");
        let remote = SuccessfulRemote::new("verified-remote");
        let observed = remote.created_from.clone();
        let sender = EmailAddress {
            name: Some("Verified sender".into()),
            address: "a@example.com".into(),
        };
        let provenance = DraftProvenance {
            local_id: None,
            remote_id: None,
        };
        save_prepared_oauth_draft(
            &state,
            "account-a",
            &draft,
            &provenance,
            Ok((remote, sender.clone())),
            None,
        )
        .await
        .unwrap();
        let observed = observed.lock().unwrap();
        assert_eq!(observed.len(), 1);
        let from = observed[0]
            .as_ref()
            .expect("Verified sender must reach the remote draft");
        assert_eq!(from.address, sender.address);
        assert_eq!(from.name, sender.name);
    }

    #[tokio::test]
    async fn prepared_draft_identity_mismatch_stays_local_without_remote_calls() {
        let state = test_state();
        let draft = draft_message("Keep private draft");
        let remote = SuccessfulRemote::new("must-not-upload");
        let observed = remote.created_from.clone();
        let identity = pebble_core::OAuthMailboxIdentity {
            subject: "gmail:other".into(),
            email: "other@example.com".into(),
            display_name: None,
        };
        let prepared = state
            .store
            .apply_verified_oauth_identity("account-a", "a@example.com", &identity, false)
            .map(|()| {
                (
                    remote,
                    EmailAddress {
                        name: identity.display_name.clone(),
                        address: identity.email.clone(),
                    },
                )
            });
        assert!(prepared.is_err());
        let provenance = DraftProvenance {
            local_id: None,
            remote_id: None,
        };
        let id =
            save_prepared_oauth_draft(&state, "account-a", &draft, &provenance, prepared, None)
                .await
                .unwrap();
        assert!(observed.lock().unwrap().is_empty());
        let stored = state.store.get_message(&id).unwrap().unwrap();
        assert!(stored.is_draft);
        assert_eq!(stored.subject, "Keep private draft");
        assert_eq!(stored.body_text, draft.body_text);
    }

    impl RemoteDraftOperations for FailingRemote {
        async fn create_draft(
            &self,
            _draft: &DraftMessage,
        ) -> Result<RemoteDraftIdentity, PebbleError> {
            Err(PebbleError::Network("provider unavailable".to_string()))
        }

        async fn update_draft(
            &self,
            _draft_id: &str,
            _draft: &DraftMessage,
        ) -> Result<RemoteDraftIdentity, PebbleError> {
            Err(PebbleError::Network("provider unavailable".to_string()))
        }

        async fn delete_draft(&self, _draft_id: &str) -> Result<(), PebbleError> {
            Err(PebbleError::Network("provider unavailable".to_string()))
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
    fn draft_provenance_rejects_cross_account_local_id() {
        let store = Store::open_in_memory().unwrap();
        let account_a = make_account("account-a", "a@example.com", ProviderType::Gmail);
        let account_b = make_account("account-b", "b@example.com", ProviderType::Gmail);
        store.insert_account(&account_a).unwrap();
        store.insert_account(&account_b).unwrap();
        let draft_id = new_id();
        let draft = make_local_draft(&account_a.id, &draft_id, "");
        store.insert_message(&draft, &[]).unwrap();

        let err = resolve_draft_provenance(&store, &account_b.id, Some(&draft_id)).unwrap_err();

        assert!(matches!(err, PebbleError::Validation(_)));
    }

    #[test]
    fn provenance_distinguishes_local_fallback_from_remote_id() {
        let store = Store::open_in_memory().unwrap();
        let account = make_account("account-a", "a@example.com", ProviderType::Gmail);
        store.insert_account(&account).unwrap();
        let draft_id = new_id();
        let draft = make_local_draft(&account.id, &draft_id, "remote-draft-1");
        store.insert_message(&draft, &[]).unwrap();

        assert_eq!(
            resolve_draft_provenance(&store, &account.id, Some(&draft_id)).unwrap(),
            DraftProvenance {
                local_id: Some(draft_id),
                remote_id: Some("remote-draft-1".to_string()),
            }
        );
        assert_eq!(
            resolve_draft_provenance(&store, &account.id, Some("remote-draft-2")).unwrap(),
            DraftProvenance {
                local_id: None,
                remote_id: Some("remote-draft-2".to_string()),
            }
        );
    }

    #[tokio::test]
    async fn local_only_fallback_is_promoted_to_remote_without_reusing_local_id() {
        let state = test_state();
        let local_id = new_id();
        state
            .store
            .insert_message(&make_local_draft("account-a", &local_id, ""), &[])
            .unwrap();
        let provenance =
            resolve_draft_provenance(&state.store, "account-a", Some(&local_id)).unwrap();
        let remote = SuccessfulRemote::new("remote-created");

        let saved_id = save_oauth_draft_with_fallback(
            &state,
            "account-a",
            &draft_message("online"),
            &provenance,
            &remote,
            None,
        )
        .await
        .unwrap();

        assert_eq!(saved_id, "remote-created");
        assert!(state.store.get_message(&local_id).unwrap().is_none());
    }

    #[tokio::test]
    async fn failed_remote_update_persists_origin_and_recovers_online() {
        let state = test_state();
        let remote_provenance = DraftProvenance {
            local_id: None,
            remote_id: Some("remote-original".to_string()),
        };
        let local_id = save_oauth_draft_with_fallback(
            &state,
            "account-a",
            &draft_message("offline edit"),
            &remote_provenance,
            &FailingRemote,
            None,
        )
        .await
        .unwrap();
        let local = state.store.get_message(&local_id).unwrap().unwrap();
        assert_eq!(local.remote_id, "remote-original");
        assert_eq!(local.subject, "offline edit");

        let recovered =
            resolve_draft_provenance(&state.store, "account-a", Some(&local_id)).unwrap();
        let remote = SuccessfulRemote::new("must-not-create");
        let saved_id = save_oauth_draft_with_fallback(
            &state,
            "account-a",
            &draft_message("online edit"),
            &recovered,
            &remote,
            None,
        )
        .await
        .unwrap();

        assert_eq!(saved_id, "remote-original");
        assert_eq!(
            remote.updated_ids.lock().unwrap().as_slice(),
            &["remote-original".to_string()]
        );
        assert!(state.store.get_message(&local_id).unwrap().is_none());
    }

    #[tokio::test]
    async fn failed_new_remote_draft_is_saved_as_local_only() {
        let state = test_state();
        let local_id = save_oauth_draft_with_fallback(
            &state,
            "account-a",
            &draft_message("new offline"),
            &DraftProvenance {
                local_id: None,
                remote_id: None,
            },
            &FailingRemote,
            None,
        )
        .await
        .unwrap();
        let local = state.store.get_message(&local_id).unwrap().unwrap();
        assert!(local.remote_id.is_empty());
        assert_eq!(local.subject, "new offline");
    }

    #[test]
    fn failed_local_store_removes_newly_staged_attachment_files() {
        fn count_files(path: &std::path::Path) -> usize {
            let Ok(entries) = std::fs::read_dir(path) else {
                return 0;
            };
            entries
                .filter_map(|entry| entry.ok())
                .map(|entry| {
                    if entry.file_type().is_ok_and(|file_type| file_type.is_dir()) {
                        count_files(&entry.path())
                    } else {
                        usize::from(entry.file_type().is_ok_and(|file_type| file_type.is_file()))
                    }
                })
                .sum()
        }

        let state = test_state();
        let source_dir = std::env::temp_dir().join(format!("pebble-draft-source-{}", new_id()));
        std::fs::create_dir_all(&source_dir).unwrap();
        let source = source_dir.join("report.txt");
        std::fs::write(&source, b"draft attachment").unwrap();
        let mut draft = draft_message("local failure");
        draft.attachment_paths = vec![source.to_string_lossy().to_string()];

        let result = save_draft_locally(&state, "missing-account", &draft, None, None);

        assert!(result.is_err());
        let stored_files = count_files(&state.attachments_dir);
        assert_eq!(stored_files, 0);

        let _ = std::fs::remove_dir_all(source_dir);
        let _ = std::fs::remove_dir_all(&state.attachments_dir);
    }
}
