use crate::state::AppState;
use pebble_core::{new_id, Folder, FolderRole, FolderType, PebbleError, ProviderType};
use pebble_mail::should_hide_outlook_folder;
use serde::Serialize;
use std::collections::HashSet;
use tauri::State;

#[derive(Debug, Clone, Serialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
pub struct ImapSyncFolderSettings {
    pub folders: Vec<Folder>,
    pub selected_remote_ids: Vec<String>,
}

fn provider_folders_have_arrived(folders: &[Folder]) -> bool {
    folders
        .iter()
        .any(|folder| !folder.remote_id.starts_with("__local_"))
}

fn should_seed_local_archive(folders: &[Folder]) -> bool {
    let has_archive = folders
        .iter()
        .any(|folder| folder.role == Some(FolderRole::Archive));

    provider_folders_have_arrived(folders) && !has_archive
}

fn should_hide_stored_outlook_folder(folder: &Folder) -> bool {
    folder.role.is_none()
        && !folder.remote_id.starts_with("__local_")
        && should_hide_outlook_folder(Some(&folder.name), None)
}

fn filter_display_folders(provider: Option<&ProviderType>, folders: Vec<Folder>) -> Vec<Folder> {
    if !matches!(provider, Some(ProviderType::Outlook)) {
        return folders;
    }

    folders
        .into_iter()
        .filter(|folder| !should_hide_stored_outlook_folder(folder))
        .collect()
}

#[tauri::command]
pub async fn list_folders(
    state: State<'_, AppState>,
    account_id: String,
) -> std::result::Result<Vec<Folder>, PebbleError> {
    let store = state.store.clone();
    tokio::task::spawn_blocking(move || {
        let provider = store
            .get_account(&account_id)?
            .map(|account| account.provider);
        let folders = store.list_folders(&account_id)?;

        if !provider_folders_have_arrived(&folders) {
            return Ok(Vec::new());
        }

        // Ensure a local archive folder exists after provider folders have arrived.
        // During first OAuth sign-in, folders may still be syncing; returning an
        // empty list lets the sidebar keep its placeholder folders instead of
        // caching a misleading "Archive only" account.
        if should_seed_local_archive(&folders) {
            let archive = Folder {
                id: new_id(),
                account_id: account_id.clone(),
                remote_id: "__local_archive__".to_string(),
                name: "Archive".to_string(),
                folder_type: FolderType::Folder,
                role: Some(FolderRole::Archive),
                parent_id: None,
                color: None,
                is_system: true,
                sort_order: 3,
            };
            if let Err(error) = store.insert_folder(&archive) {
                // A concurrent caller may have seeded it first; re-listing below
                // shows whichever row won. Anything else is still surfaced in logs.
                tracing::warn!(
                    "Failed to seed local archive folder for account {account_id}: {error}"
                );
            }
            let folders = store.list_folders(&account_id)?;
            return Ok(filter_display_folders(provider.as_ref(), folders));
        }

        Ok(filter_display_folders(provider.as_ref(), folders))
    })
    .await
    .map_err(|e| PebbleError::Internal(format!("Task join error: {e}")))?
}

async fn discover_imap_folders(
    state: &AppState,
    account_id: &str,
) -> std::result::Result<Vec<Folder>, PebbleError> {
    let account = state
        .store
        .get_account(account_id)?
        .ok_or_else(|| PebbleError::Internal(format!("Account not found: {account_id}")))?;
    if account.provider != ProviderType::Imap {
        return Err(PebbleError::UnsupportedProvider(
            "Folder selection is currently available for IMAP accounts".to_string(),
        ));
    }

    let provider = crate::commands::messages::connect_imap(state, account_id).await?;
    let result = provider.list_folders(account_id).await;
    if let Err(error) = provider.disconnect().await {
        tracing::debug!(
            "Failed to disconnect IMAP folder discovery session for account {account_id}: {error}"
        );
    }
    result
}

fn selected_remote_ids_for_settings(
    folders: &[Folder],
    configured: Option<Vec<String>>,
) -> Vec<String> {
    let configured = configured.map(|ids| ids.into_iter().collect::<HashSet<_>>());
    folders
        .iter()
        .filter(|folder| {
            folder.role == Some(FolderRole::Inbox)
                || folder.remote_id.eq_ignore_ascii_case("INBOX")
                || configured
                    .as_ref()
                    .is_none_or(|selected| selected.contains(&folder.remote_id))
        })
        .map(|folder| folder.remote_id.clone())
        .collect()
}

#[tauri::command]
pub async fn get_imap_sync_folders(
    state: State<'_, AppState>,
    account_id: String,
) -> std::result::Result<ImapSyncFolderSettings, PebbleError> {
    let folders = discover_imap_folders(&state, &account_id).await?;
    let configured = state
        .store
        .get_sync_state(&account_id)?
        .and_then(|sync_state| sync_state.selected_imap_folder_remote_ids);
    let selected_remote_ids = selected_remote_ids_for_settings(&folders, configured);

    Ok(ImapSyncFolderSettings {
        folders,
        selected_remote_ids,
    })
}

#[tauri::command]
pub async fn update_imap_sync_folders(
    state: State<'_, AppState>,
    account_id: String,
    selected_remote_ids: Vec<String>,
) -> std::result::Result<ImapSyncFolderSettings, PebbleError> {
    // Refresh LIST when saving as well as when opening the picker. This avoids
    // persisting stale or fabricated remote IDs when the server changed while
    // the account dialog was open.
    let folders = discover_imap_folders(&state, &account_id).await?;
    let available_remote_ids: HashSet<&str> = folders
        .iter()
        .map(|folder| folder.remote_id.as_str())
        .collect();
    let unknown_remote_ids: Vec<&str> = selected_remote_ids
        .iter()
        .map(String::as_str)
        .filter(|remote_id| !available_remote_ids.contains(remote_id))
        .collect();
    if !unknown_remote_ids.is_empty() {
        return Err(PebbleError::Validation(format!(
            "Unknown IMAP folders: {}",
            unknown_remote_ids.join(", ")
        )));
    }

    let requested: HashSet<&str> = selected_remote_ids.iter().map(String::as_str).collect();
    let effective_selected_remote_ids: Vec<String> = folders
        .iter()
        .filter(|folder| {
            folder.role == Some(FolderRole::Inbox)
                || folder.remote_id.eq_ignore_ascii_case("INBOX")
                || requested.contains(folder.remote_id.as_str())
        })
        .map(|folder| folder.remote_id.clone())
        .collect();
    if !folders.iter().any(|folder| {
        folder.role == Some(FolderRole::Inbox) || folder.remote_id.eq_ignore_ascii_case("INBOX")
    }) {
        return Err(PebbleError::Validation(
            "The IMAP server did not return an Inbox folder".to_string(),
        ));
    }

    let store = state.store.clone();
    let account_id_for_store = account_id.clone();
    let folders_for_store = folders.clone();
    let selected_for_store = effective_selected_remote_ids.clone();
    let change = tokio::task::spawn_blocking(move || {
        store.apply_imap_folder_selection(
            &account_id_for_store,
            &folders_for_store,
            &selected_for_store,
        )
    })
    .await
    .map_err(|error| PebbleError::Internal(format!("Task join error: {error}")))??;

    if let Err(error) =
        crate::commands::messages::refresh_search_documents(&state, &change.affected_message_ids)
    {
        tracing::warn!(
            "Failed to refresh search documents after changing IMAP folders for account {account_id}: {error}"
        );
    }

    let attachments_dir = state.attachments_dir.clone();
    let deleted_message_ids = change.deleted_message_ids;
    if let Err(error) = tokio::task::spawn_blocking(move || {
        for message_id in deleted_message_ids {
            let message_dir = attachments_dir.join(&message_id);
            if message_dir.exists() {
                if let Err(error) = std::fs::remove_dir_all(&message_dir) {
                    tracing::warn!(
                        "Failed to remove attachments for deselected-folder message {message_id}: {error}"
                    );
                }
            }
        }
    })
    .await
    {
        tracing::warn!(
            "Attachment cleanup task failed after changing IMAP folders for account {account_id}: {error}"
        );
    }

    Ok(ImapSyncFolderSettings {
        folders,
        selected_remote_ids: effective_selected_remote_ids,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(role: FolderRole, remote_id: &str) -> Folder {
        Folder {
            id: new_id(),
            account_id: "account-1".to_string(),
            remote_id: remote_id.to_string(),
            name: remote_id.to_string(),
            folder_type: FolderType::Folder,
            role: Some(role),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 0,
        }
    }

    #[test]
    fn archive_seed_waits_until_provider_folders_exist() {
        assert!(!provider_folders_have_arrived(&[]));
        assert!(!provider_folders_have_arrived(&[folder(
            FolderRole::Archive,
            "__local_archive__"
        )]));
        assert!(provider_folders_have_arrived(&[folder(
            FolderRole::Inbox,
            "INBOX"
        )]));

        assert!(!should_seed_local_archive(&[]));
        assert!(!should_seed_local_archive(&[folder(
            FolderRole::Sent,
            "__local_outbox__"
        )]));
        assert!(should_seed_local_archive(&[folder(
            FolderRole::Inbox,
            "INBOX"
        )]));
        assert!(!should_seed_local_archive(&[folder(
            FolderRole::Archive,
            "__local_archive__"
        )]));
    }

    #[test]
    fn display_filter_hides_outlook_service_folders_but_keeps_local_outbox() {
        let mut conversation_history = folder(FolderRole::Inbox, "conversation-history-id");
        conversation_history.role = None;
        conversation_history.name = "对话历史记录".to_string();

        let mut remote_outbox = folder(FolderRole::Inbox, "remote-outbox-id");
        remote_outbox.role = None;
        remote_outbox.name = "发件箱".to_string();

        let mut local_outbox = folder(FolderRole::Inbox, "__local_outbox__");
        local_outbox.role = None;
        local_outbox.name = "Outbox".to_string();

        let inbox = folder(FolderRole::Inbox, "inbox-id");
        let filtered = filter_display_folders(
            Some(&pebble_core::ProviderType::Outlook),
            vec![
                conversation_history,
                remote_outbox,
                local_outbox.clone(),
                inbox.clone(),
            ],
        );

        assert_eq!(
            filtered
                .iter()
                .map(|folder| folder.remote_id.as_str())
                .collect::<Vec<_>>(),
            vec!["__local_outbox__", "inbox-id"]
        );
    }

    #[test]
    fn explicit_imap_folder_settings_keep_inbox_selected() {
        let inbox = folder(FolderRole::Inbox, "INBOX");
        let sent = folder(FolderRole::Sent, "Sent");
        let mut custom = folder(FolderRole::Archive, "Newsletters");
        custom.role = None;

        let selected = selected_remote_ids_for_settings(
            &[inbox, sent, custom],
            Some(vec!["Newsletters".to_string()]),
        );

        assert_eq!(
            selected,
            vec!["INBOX".to_string(), "Newsletters".to_string()]
        );
    }

    #[test]
    fn legacy_imap_folder_settings_select_every_discovered_folder() {
        let inbox = folder(FolderRole::Inbox, "INBOX");
        let sent = folder(FolderRole::Sent, "Sent");

        assert_eq!(
            selected_remote_ids_for_settings(&[inbox, sent], None),
            vec!["INBOX".to_string(), "Sent".to_string()]
        );
    }
}
