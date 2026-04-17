use crate::state::AppState;
use pebble_core::traits::{FolderProvider, LabelProvider};
use pebble_core::{FolderRole, Message, PebbleError, ProviderType};
use tauri::State;
use tracing::{info, warn};

use super::provider_dispatch::{parse_imap_uid, ConnectedProvider};
use super::{
    connect_gmail, connect_imap, connect_outlook, find_folder_by_role, find_message_folder,
    queue_pending_remote_op, queued_remote_error, refresh_search_document,
    remote_mutation_allows_local_commit, remove_search_documents, RemoteMutationOutcome,
};
use serde_json::json;

/// Load a message and its account's provider type, surfacing a clear error
/// if either is missing. Four lifecycle commands share this preamble.
fn resolve_message_context(
    state: &AppState,
    message_id: &str,
) -> std::result::Result<(Message, ProviderType), PebbleError> {
    let msg = state
        .store
        .get_message(message_id)?
        .ok_or_else(|| PebbleError::Internal(format!("Message not found: {message_id}")))?;
    let provider_type = state
        .store
        .get_account(&msg.account_id)?
        .map(|account| account.provider)
        .ok_or_else(|| PebbleError::Internal(format!("Account not found: {}", msg.account_id)))?;
    Ok((msg, provider_type))
}

fn queue_permanent_delete_failure(
    state: &AppState,
    account_id: &str,
    message_id: &str,
    remote_id: &str,
    trash_folder_id: &str,
    trash_folder_remote_id: &str,
    error: &str,
) -> std::result::Result<(), PebbleError> {
    let payload = json!({
        "provider_account_id": account_id,
        "remote_id": remote_id,
        "op": "delete_permanent",
        "payload": {
            "source_folder_id": trash_folder_id,
            "source_folder_remote_id": trash_folder_remote_id,
            "permanent": true,
        },
    });
    let op_id = state.store.insert_pending_mail_op(
        account_id,
        message_id,
        "delete_permanent",
        &payload.to_string(),
    )?;
    state.store.mark_pending_mail_op_failed(&op_id, error)?;
    Ok(())
}

/// Returns "archived" or "unarchived" so the frontend can show the correct toast.
#[tauri::command]
pub async fn archive_message(
    state: State<'_, AppState>,
    message_id: String,
) -> std::result::Result<String, PebbleError> {
    let (msg, provider_type) = resolve_message_context(&state, &message_id)?;

    let source_folder = find_message_folder(&state, &message_id, &msg.account_id)?;
    if source_folder.role == Some(FolderRole::Archive) {
        info!(
            "Message {} already in archive, restoring to inbox",
            message_id
        );
        let inbox = find_folder_by_role(&state, &msg.account_id, FolderRole::Inbox)?;

        let local_only = source_folder.remote_id.starts_with("__local_")
            || inbox.remote_id.starts_with("__local_");
        let outcome = if local_only {
            RemoteMutationOutcome::LocalOnly
        } else {
            match provider_type {
                ProviderType::Gmail => match connect_gmail(&state, &msg.account_id).await {
                    Ok(provider) => match provider
                        .modify_labels(&msg.remote_id, &["INBOX".to_string()], &[])
                        .await
                    {
                        Ok(()) => RemoteMutationOutcome::Applied,
                        Err(e) => {
                            let error = e.to_string();
                            let outcome = queue_pending_remote_op(
                                &state,
                                &msg,
                                "unarchive",
                                json!({
                                    "source_folder_id": source_folder.id,
                                    "target_folder_id": inbox.id,
                                    "add_labels": ["INBOX"],
                                    "remove_labels": [],
                                }),
                                &error,
                            )?;
                            if !remote_mutation_allows_local_commit(outcome) {
                                return Err(queued_remote_error("unarchive", &error));
                            }
                            outcome
                        }
                    },
                    Err(e) => {
                        let error = e.to_string();
                        let outcome = queue_pending_remote_op(
                            &state,
                            &msg,
                            "unarchive",
                            json!({
                                "source_folder_id": source_folder.id,
                                "target_folder_id": inbox.id,
                                "add_labels": ["INBOX"],
                                "remove_labels": [],
                            }),
                            &error,
                        )?;
                        if !remote_mutation_allows_local_commit(outcome) {
                            return Err(queued_remote_error("unarchive", &error));
                        }
                        outcome
                    }
                },
                ProviderType::Outlook => match connect_outlook(&state, &msg.account_id).await {
                    Ok(provider) => match provider
                        .move_message(&msg.remote_id, &inbox.remote_id)
                        .await
                    {
                        Ok(()) => RemoteMutationOutcome::Applied,
                        Err(e) => {
                            let error = e.to_string();
                            let outcome = queue_pending_remote_op(
                                &state,
                                &msg,
                                "unarchive",
                                json!({
                                    "source_folder_id": source_folder.id,
                                    "source_folder_remote_id": source_folder.remote_id,
                                    "target_folder_id": inbox.id,
                                    "target_folder_remote_id": inbox.remote_id,
                                }),
                                &error,
                            )?;
                            if !remote_mutation_allows_local_commit(outcome) {
                                return Err(queued_remote_error("unarchive", &error));
                            }
                            outcome
                        }
                    },
                    Err(e) => {
                        let error = e.to_string();
                        let outcome = queue_pending_remote_op(
                            &state,
                            &msg,
                            "unarchive",
                            json!({
                                "source_folder_id": source_folder.id,
                                "source_folder_remote_id": source_folder.remote_id,
                                "target_folder_id": inbox.id,
                                "target_folder_remote_id": inbox.remote_id,
                            }),
                            &error,
                        )?;
                        if !remote_mutation_allows_local_commit(outcome) {
                            return Err(queued_remote_error("unarchive", &error));
                        }
                        outcome
                    }
                },
                ProviderType::Imap => {
                    let uid: u32 = msg.remote_id.parse().map_err(|e| {
                        PebbleError::Internal(format!("Invalid remote_id (not a UID): {e}"))
                    })?;
                    match connect_imap(&state, &msg.account_id).await {
                        Ok(imap) => {
                            let result = imap
                                .move_message(&source_folder.remote_id, uid, &inbox.remote_id)
                                .await;
                            let _ = imap.disconnect().await;
                            match result {
                                Ok(()) => RemoteMutationOutcome::Applied,
                                Err(e) => {
                                    let error = e.to_string();
                                    let outcome = queue_pending_remote_op(
                                        &state,
                                        &msg,
                                        "unarchive",
                                        json!({
                                            "source_folder_id": source_folder.id,
                                            "source_folder_remote_id": source_folder.remote_id,
                                            "target_folder_id": inbox.id,
                                            "target_folder_remote_id": inbox.remote_id,
                                        }),
                                        &error,
                                    )?;
                                    if !remote_mutation_allows_local_commit(outcome) {
                                        return Err(queued_remote_error("unarchive", &error));
                                    }
                                    outcome
                                }
                            }
                        }
                        Err(e) => {
                            let error = e.to_string();
                            let outcome = queue_pending_remote_op(
                                &state,
                                &msg,
                                "unarchive",
                                json!({
                                    "source_folder_id": source_folder.id,
                                    "source_folder_remote_id": source_folder.remote_id,
                                    "target_folder_id": inbox.id,
                                    "target_folder_remote_id": inbox.remote_id,
                                }),
                                &error,
                            )?;
                            if !remote_mutation_allows_local_commit(outcome) {
                                return Err(queued_remote_error("unarchive", &error));
                            }
                            outcome
                        }
                    }
                }
            }
        };

        if remote_mutation_allows_local_commit(outcome) {
            state.store.move_message_to_folder(&message_id, &inbox.id)?;
            refresh_search_document(&state, &message_id)?;
            return Ok("unarchived".to_string());
        }
        return Err(PebbleError::Network(
            "Remote unarchive was not applied".to_string(),
        ));
    }

    // Try to find Archive folder; if not available, just soft-delete locally
    match find_folder_by_role(&state, &msg.account_id, FolderRole::Archive) {
        Ok(archive_folder) => {
            let is_local = archive_folder.remote_id.starts_with("__local_");
            let outcome = if is_local {
                RemoteMutationOutcome::LocalOnly
            } else {
                match provider_type {
                    ProviderType::Gmail => match connect_gmail(&state, &msg.account_id).await {
                        Ok(provider) => match provider
                            .modify_labels(&msg.remote_id, &[], &["INBOX".to_string()])
                            .await
                        {
                            Ok(()) => RemoteMutationOutcome::Applied,
                            Err(e) => {
                                let error = e.to_string();
                                let outcome = queue_pending_remote_op(
                                    &state,
                                    &msg,
                                    "archive",
                                    json!({
                                        "source_folder_id": source_folder.id,
                                        "target_folder_id": archive_folder.id,
                                        "add_labels": [],
                                        "remove_labels": ["INBOX"],
                                    }),
                                    &error,
                                )?;
                                if !remote_mutation_allows_local_commit(outcome) {
                                    return Err(queued_remote_error("archive", &error));
                                }
                                outcome
                            }
                        },
                        Err(e) => {
                            let error = e.to_string();
                            let outcome = queue_pending_remote_op(
                                &state,
                                &msg,
                                "archive",
                                json!({
                                    "source_folder_id": source_folder.id,
                                    "target_folder_id": archive_folder.id,
                                    "add_labels": [],
                                    "remove_labels": ["INBOX"],
                                }),
                                &error,
                            )?;
                            if !remote_mutation_allows_local_commit(outcome) {
                                return Err(queued_remote_error("archive", &error));
                            }
                            outcome
                        }
                    },
                    ProviderType::Outlook => match connect_outlook(&state, &msg.account_id).await {
                        Ok(provider) => match provider
                            .move_message(&msg.remote_id, &archive_folder.remote_id)
                            .await
                        {
                            Ok(()) => RemoteMutationOutcome::Applied,
                            Err(e) => {
                                let error = e.to_string();
                                let outcome = queue_pending_remote_op(
                                    &state,
                                    &msg,
                                    "archive",
                                    json!({
                                        "source_folder_id": source_folder.id,
                                        "source_folder_remote_id": source_folder.remote_id,
                                        "target_folder_id": archive_folder.id,
                                        "target_folder_remote_id": archive_folder.remote_id,
                                    }),
                                    &error,
                                )?;
                                if !remote_mutation_allows_local_commit(outcome) {
                                    return Err(queued_remote_error("archive", &error));
                                }
                                outcome
                            }
                        },
                        Err(e) => {
                            let error = e.to_string();
                            let outcome = queue_pending_remote_op(
                                &state,
                                &msg,
                                "archive",
                                json!({
                                    "source_folder_id": source_folder.id,
                                    "source_folder_remote_id": source_folder.remote_id,
                                    "target_folder_id": archive_folder.id,
                                    "target_folder_remote_id": archive_folder.remote_id,
                                }),
                                &error,
                            )?;
                            if !remote_mutation_allows_local_commit(outcome) {
                                return Err(queued_remote_error("archive", &error));
                            }
                            outcome
                        }
                    },
                    ProviderType::Imap => {
                        let uid: u32 = msg.remote_id.parse().map_err(|e| {
                            PebbleError::Internal(format!("Invalid remote_id (not a UID): {e}"))
                        })?;
                        match connect_imap(&state, &msg.account_id).await {
                            Ok(imap) => {
                                let result = imap
                                    .move_message(
                                        &source_folder.remote_id,
                                        uid,
                                        &archive_folder.remote_id,
                                    )
                                    .await;
                                let _ = imap.disconnect().await;
                                match result {
                                    Ok(()) => RemoteMutationOutcome::Applied,
                                    Err(e) => {
                                        let error = e.to_string();
                                        let outcome = queue_pending_remote_op(
                                            &state,
                                            &msg,
                                            "archive",
                                            json!({
                                                "source_folder_id": source_folder.id,
                                                "source_folder_remote_id": source_folder.remote_id,
                                                "target_folder_id": archive_folder.id,
                                                "target_folder_remote_id": archive_folder.remote_id,
                                            }),
                                            &error,
                                        )?;
                                        if !remote_mutation_allows_local_commit(outcome) {
                                            return Err(queued_remote_error("archive", &error));
                                        }
                                        outcome
                                    }
                                }
                            }
                            Err(e) => {
                                let error = e.to_string();
                                let outcome = queue_pending_remote_op(
                                    &state,
                                    &msg,
                                    "archive",
                                    json!({
                                        "source_folder_id": source_folder.id,
                                        "source_folder_remote_id": source_folder.remote_id,
                                        "target_folder_id": archive_folder.id,
                                        "target_folder_remote_id": archive_folder.remote_id,
                                    }),
                                    &error,
                                )?;
                                if !remote_mutation_allows_local_commit(outcome) {
                                    return Err(queued_remote_error("archive", &error));
                                }
                                outcome
                            }
                        }
                    }
                }
            };

            if remote_mutation_allows_local_commit(outcome) {
                state
                    .store
                    .move_message_to_folder(&message_id, &archive_folder.id)?;
                refresh_search_document(&state, &message_id)?;
                Ok("archived".to_string())
            } else {
                Err(PebbleError::Network(
                    "Remote archive was not applied".to_string(),
                ))
            }
        }
        Err(_) => {
            if matches!(provider_type, ProviderType::Gmail) {
                let outcome = match connect_gmail(&state, &msg.account_id).await {
                    Ok(provider) => match provider
                        .modify_labels(&msg.remote_id, &[], &["INBOX".to_string()])
                        .await
                    {
                        Ok(()) => RemoteMutationOutcome::Applied,
                        Err(e) => {
                            let error = e.to_string();
                            let outcome = queue_pending_remote_op(
                                &state,
                                &msg,
                                "archive",
                                json!({
                                    "source_folder_id": source_folder.id,
                                    "add_labels": [],
                                    "remove_labels": ["INBOX"],
                                }),
                                &error,
                            )?;
                            if !remote_mutation_allows_local_commit(outcome) {
                                return Err(queued_remote_error("archive", &error));
                            }
                            outcome
                        }
                    },
                    Err(e) => {
                        let error = e.to_string();
                        let outcome = queue_pending_remote_op(
                            &state,
                            &msg,
                            "archive",
                            json!({
                                "source_folder_id": source_folder.id,
                                "add_labels": [],
                                "remove_labels": ["INBOX"],
                            }),
                            &error,
                        )?;
                        if !remote_mutation_allows_local_commit(outcome) {
                            return Err(queued_remote_error("archive", &error));
                        }
                        outcome
                    }
                };
                if !remote_mutation_allows_local_commit(outcome) {
                    return Err(PebbleError::Network(
                        "Remote archive was not applied".to_string(),
                    ));
                }
            }

            info!(
                "No archive folder found, soft-deleting message {} locally",
                message_id
            );
            state.store.soft_delete_message(&message_id)?;
            refresh_search_document(&state, &message_id)?;
            Ok("archived".to_string())
        }
    }
}

#[tauri::command]
pub async fn delete_message(
    state: State<'_, AppState>,
    message_id: String,
) -> std::result::Result<(), PebbleError> {
    let (msg, provider_type) = resolve_message_context(&state, &message_id)?;

    let source_folder = find_message_folder(&state, &message_id, &msg.account_id)?;
    let trash_folder = find_folder_by_role(&state, &msg.account_id, FolderRole::Trash).ok();
    let is_permanent = source_folder.role == Some(FolderRole::Trash);

    let outcome = match provider_type {
        ProviderType::Gmail => match connect_gmail(&state, &msg.account_id).await {
            Ok(provider) => {
                let result = if is_permanent {
                    provider.delete_message_permanently(&msg.remote_id).await
                } else {
                    provider.trash_message(&msg.remote_id).await
                };
                match result {
                    Ok(()) => RemoteMutationOutcome::Applied,
                    Err(e) => {
                        let error = e.to_string();
                        let outcome = queue_pending_remote_op(
                            &state,
                            &msg,
                            if is_permanent {
                                "delete_permanent"
                            } else {
                                "delete"
                            },
                            json!({
                                "source_folder_id": &source_folder.id,
                                "source_folder_remote_id": &source_folder.remote_id,
                                "trash_folder_id": trash_folder.as_ref().map(|f| f.id.as_str()),
                                "permanent": is_permanent,
                            }),
                            &error,
                        )?;
                        if !remote_mutation_allows_local_commit(outcome) {
                            return Err(queued_remote_error("delete", &error));
                        }
                        outcome
                    }
                }
            }
            Err(e) => {
                let error = e.to_string();
                let outcome = queue_pending_remote_op(
                    &state,
                    &msg,
                    if is_permanent {
                        "delete_permanent"
                    } else {
                        "delete"
                    },
                    json!({
                        "source_folder_id": &source_folder.id,
                        "source_folder_remote_id": &source_folder.remote_id,
                        "trash_folder_id": trash_folder.as_ref().map(|f| f.id.as_str()),
                        "permanent": is_permanent,
                    }),
                    &error,
                )?;
                if !remote_mutation_allows_local_commit(outcome) {
                    return Err(queued_remote_error("delete", &error));
                }
                outcome
            }
        },
        ProviderType::Outlook => match connect_outlook(&state, &msg.account_id).await {
            Ok(provider) => {
                let result = if is_permanent {
                    provider.delete_message_permanently(&msg.remote_id).await
                } else {
                    provider.trash_message(&msg.remote_id).await
                };
                match result {
                    Ok(()) => RemoteMutationOutcome::Applied,
                    Err(e) => {
                        let error = e.to_string();
                        let outcome = queue_pending_remote_op(
                            &state,
                            &msg,
                            if is_permanent {
                                "delete_permanent"
                            } else {
                                "delete"
                            },
                            json!({
                                "source_folder_id": &source_folder.id,
                                "source_folder_remote_id": &source_folder.remote_id,
                                "trash_folder_id": trash_folder.as_ref().map(|f| f.id.as_str()),
                                "permanent": is_permanent,
                            }),
                            &error,
                        )?;
                        if !remote_mutation_allows_local_commit(outcome) {
                            return Err(queued_remote_error("delete", &error));
                        }
                        outcome
                    }
                }
            }
            Err(e) => {
                let error = e.to_string();
                let outcome = queue_pending_remote_op(
                    &state,
                    &msg,
                    if is_permanent {
                        "delete_permanent"
                    } else {
                        "delete"
                    },
                    json!({
                        "source_folder_id": &source_folder.id,
                        "source_folder_remote_id": &source_folder.remote_id,
                        "trash_folder_id": trash_folder.as_ref().map(|f| f.id.as_str()),
                        "permanent": is_permanent,
                    }),
                    &error,
                )?;
                if !remote_mutation_allows_local_commit(outcome) {
                    return Err(queued_remote_error("delete", &error));
                }
                outcome
            }
        },
        ProviderType::Imap => {
            let source_is_local = source_folder.remote_id.starts_with("__local_");
            let target_trash_is_local = trash_folder
                .as_ref()
                .is_some_and(|folder| folder.remote_id.starts_with("__local_"));
            let can_move_to_remote_trash = trash_folder
                .as_ref()
                .is_some_and(|folder| folder.id != source_folder.id && !target_trash_is_local);

            if source_is_local || (!is_permanent && target_trash_is_local) {
                RemoteMutationOutcome::LocalOnly
            } else {
                let uid = parse_imap_uid(&msg.remote_id)?;
                match connect_imap(&state, &msg.account_id).await {
                    Ok(imap) => {
                        let result = if !is_permanent && can_move_to_remote_trash {
                            let trash = trash_folder.as_ref().expect("checked above");
                            imap.move_message(&source_folder.remote_id, uid, &trash.remote_id)
                                .await
                        } else {
                            imap.delete_message(&source_folder.remote_id, uid).await
                        };
                        let _ = imap.disconnect().await;
                        match result {
                            Ok(()) => RemoteMutationOutcome::Applied,
                            Err(e) => {
                                let error = e.to_string();
                                let outcome = queue_pending_remote_op(
                                    &state,
                                    &msg,
                                    if is_permanent {
                                        "delete_permanent"
                                    } else {
                                        "delete"
                                    },
                                    json!({
                                        "source_folder_id": &source_folder.id,
                                        "source_folder_remote_id": &source_folder.remote_id,
                                        "trash_folder_id": trash_folder.as_ref().map(|f| f.id.as_str()),
                                        "trash_folder_remote_id": trash_folder.as_ref().map(|f| f.remote_id.as_str()),
                                        "permanent": is_permanent,
                                    }),
                                    &error,
                                )?;
                                if !remote_mutation_allows_local_commit(outcome) {
                                    return Err(queued_remote_error("delete", &error));
                                }
                                outcome
                            }
                        }
                    }
                    Err(e) => {
                        let error = e.to_string();
                        let outcome = queue_pending_remote_op(
                            &state,
                            &msg,
                            if is_permanent {
                                "delete_permanent"
                            } else {
                                "delete"
                            },
                            json!({
                                "source_folder_id": &source_folder.id,
                                "source_folder_remote_id": &source_folder.remote_id,
                                "trash_folder_id": trash_folder.as_ref().map(|f| f.id.as_str()),
                                "trash_folder_remote_id": trash_folder.as_ref().map(|f| f.remote_id.as_str()),
                                "permanent": is_permanent,
                            }),
                            &error,
                        )?;
                        if !remote_mutation_allows_local_commit(outcome) {
                            return Err(queued_remote_error("delete", &error));
                        }
                        outcome
                    }
                }
            }
        }
    };

    if !remote_mutation_allows_local_commit(outcome) {
        return Err(PebbleError::Network(
            "Remote delete was not applied".to_string(),
        ));
    }

    if is_permanent {
        state
            .store
            .hard_delete_messages(std::slice::from_ref(&message_id))?;
        remove_search_documents(&state, std::slice::from_ref(&message_id))?;
    } else if let Some(trash_folder) = trash_folder {
        if trash_folder.id != source_folder.id {
            state
                .store
                .move_message_to_folder(&message_id, &trash_folder.id)?;
            refresh_search_document(&state, &message_id)?;
        } else {
            state.store.soft_delete_message(&message_id)?;
            refresh_search_document(&state, &message_id)?;
        }
    } else {
        state.store.soft_delete_message(&message_id)?;
        refresh_search_document(&state, &message_id)?;
    }
    Ok(())
}

#[tauri::command]
pub async fn restore_message(
    state: State<'_, AppState>,
    message_id: String,
) -> std::result::Result<(), PebbleError> {
    let (msg, provider_type) = resolve_message_context(&state, &message_id)?;

    let inbox = find_folder_by_role(&state, &msg.account_id, FolderRole::Inbox)?;

    let source_folder = find_message_folder(&state, &message_id, &msg.account_id).ok();

    let outcome = match provider_type {
        ProviderType::Gmail => match connect_gmail(&state, &msg.account_id).await {
            Ok(provider) => {
                let result = if source_folder
                    .as_ref()
                    .is_some_and(|src| src.role == Some(FolderRole::Trash))
                {
                    provider.untrash_message(&msg.remote_id).await
                } else {
                    provider
                        .modify_labels(&msg.remote_id, &["INBOX".to_string()], &[])
                        .await
                };
                match result {
                    Ok(()) => RemoteMutationOutcome::Applied,
                    Err(e) => {
                        let error = e.to_string();
                        let outcome = queue_pending_remote_op(
                            &state,
                            &msg,
                            "restore",
                            json!({
                                "source_folder_id": source_folder.as_ref().map(|f| f.id.as_str()),
                                "source_folder_remote_id": source_folder.as_ref().map(|f| f.remote_id.as_str()),
                                "target_folder_id": &inbox.id,
                                "target_folder_remote_id": &inbox.remote_id,
                            }),
                            &error,
                        )?;
                        if !remote_mutation_allows_local_commit(outcome) {
                            return Err(queued_remote_error("restore", &error));
                        }
                        outcome
                    }
                }
            }
            Err(e) => {
                let error = e.to_string();
                let outcome = queue_pending_remote_op(
                    &state,
                    &msg,
                    "restore",
                    json!({
                        "source_folder_id": source_folder.as_ref().map(|f| f.id.as_str()),
                        "source_folder_remote_id": source_folder.as_ref().map(|f| f.remote_id.as_str()),
                        "target_folder_id": &inbox.id,
                        "target_folder_remote_id": &inbox.remote_id,
                    }),
                    &error,
                )?;
                if !remote_mutation_allows_local_commit(outcome) {
                    return Err(queued_remote_error("restore", &error));
                }
                outcome
            }
        },
        ProviderType::Outlook => match connect_outlook(&state, &msg.account_id).await {
            Ok(provider) => match provider.restore_message(&msg.remote_id).await {
                Ok(()) => RemoteMutationOutcome::Applied,
                Err(e) => {
                    let error = e.to_string();
                    let outcome = queue_pending_remote_op(
                        &state,
                        &msg,
                        "restore",
                        json!({
                            "source_folder_id": source_folder.as_ref().map(|f| f.id.as_str()),
                            "source_folder_remote_id": source_folder.as_ref().map(|f| f.remote_id.as_str()),
                            "target_folder_id": &inbox.id,
                            "target_folder_remote_id": &inbox.remote_id,
                        }),
                        &error,
                    )?;
                    if !remote_mutation_allows_local_commit(outcome) {
                        return Err(queued_remote_error("restore", &error));
                    }
                    outcome
                }
            },
            Err(e) => {
                let error = e.to_string();
                let outcome = queue_pending_remote_op(
                    &state,
                    &msg,
                    "restore",
                    json!({
                        "source_folder_id": source_folder.as_ref().map(|f| f.id.as_str()),
                        "source_folder_remote_id": source_folder.as_ref().map(|f| f.remote_id.as_str()),
                        "target_folder_id": &inbox.id,
                        "target_folder_remote_id": &inbox.remote_id,
                    }),
                    &error,
                )?;
                if !remote_mutation_allows_local_commit(outcome) {
                    return Err(queued_remote_error("restore", &error));
                }
                outcome
            }
        },
        ProviderType::Imap => {
            let local_only = source_folder.as_ref().is_none_or(|src| {
                src.id == inbox.id
                    || src.remote_id.starts_with("__local_")
                    || inbox.remote_id.starts_with("__local_")
            });
            if local_only {
                RemoteMutationOutcome::LocalOnly
            } else {
                let source_folder = source_folder.as_ref().expect("checked above");
                let uid = parse_imap_uid(&msg.remote_id)?;
                match connect_imap(&state, &msg.account_id).await {
                    Ok(imap) => {
                        let result = imap
                            .move_message(&source_folder.remote_id, uid, &inbox.remote_id)
                            .await;
                        let _ = imap.disconnect().await;
                        match result {
                            Ok(()) => RemoteMutationOutcome::Applied,
                            Err(e) => {
                                let error = e.to_string();
                                let outcome = queue_pending_remote_op(
                                    &state,
                                    &msg,
                                    "restore",
                                    json!({
                                        "source_folder_id": &source_folder.id,
                                        "source_folder_remote_id": &source_folder.remote_id,
                                        "target_folder_id": &inbox.id,
                                        "target_folder_remote_id": &inbox.remote_id,
                                    }),
                                    &error,
                                )?;
                                if !remote_mutation_allows_local_commit(outcome) {
                                    return Err(queued_remote_error("restore", &error));
                                }
                                outcome
                            }
                        }
                    }
                    Err(e) => {
                        let error = e.to_string();
                        let outcome = queue_pending_remote_op(
                            &state,
                            &msg,
                            "restore",
                            json!({
                                "source_folder_id": &source_folder.id,
                                "source_folder_remote_id": &source_folder.remote_id,
                                "target_folder_id": &inbox.id,
                                "target_folder_remote_id": &inbox.remote_id,
                            }),
                            &error,
                        )?;
                        if !remote_mutation_allows_local_commit(outcome) {
                            return Err(queued_remote_error("restore", &error));
                        }
                        outcome
                    }
                }
            }
        }
    };

    if !remote_mutation_allows_local_commit(outcome) {
        return Err(PebbleError::Network(
            "Remote restore was not applied".to_string(),
        ));
    }

    state.store.move_message_to_folder(&message_id, &inbox.id)?;
    refresh_search_document(&state, &message_id)?;
    info!("Restored message {} to inbox", message_id);
    Ok(())
}

#[tauri::command]
pub async fn move_to_folder(
    state: State<'_, AppState>,
    message_id: String,
    target_folder_id: String,
) -> std::result::Result<(), PebbleError> {
    let (msg, provider_type) = resolve_message_context(&state, &message_id)?;

    let source_folder = find_message_folder(&state, &message_id, &msg.account_id)?;
    if source_folder.id == target_folder_id {
        return Ok(());
    }

    // Look up target folder to get its remote_id
    let target_folders = state.store.list_folders(&msg.account_id)?;
    let target_folder = target_folders
        .iter()
        .find(|f| f.id == target_folder_id)
        .ok_or_else(|| {
            PebbleError::Internal(format!("Target folder not found: {target_folder_id}"))
        })?;

    let is_local_move = source_folder.remote_id.starts_with("__local_")
        || target_folder.remote_id.starts_with("__local_");

    let base_payload = || {
        json!({
            "source_folder_id": source_folder.id.as_str(),
            "source_folder_remote_id": source_folder.remote_id.as_str(),
            "target_folder_id": target_folder.id.as_str(),
            "target_folder_remote_id": target_folder.remote_id.as_str(),
        })
    };

    let queue_move_failure =
        |error: &str, payload: serde_json::Value| -> std::result::Result<(), PebbleError> {
            queue_pending_remote_op(&state, &msg, "move_to_folder", payload, error)?;
            Err(queued_remote_error("move_to_folder", error))
        };

    // TODO(FC-5): add full remote-move support for Gmail label semantics.
    let outcome = match provider_type {
        ProviderType::Outlook => {
            if is_local_move {
                RemoteMutationOutcome::LocalOnly
            } else {
                match connect_outlook(&state, &msg.account_id).await {
                    Ok(provider) => match provider
                        .move_message(&msg.remote_id, &target_folder.remote_id)
                        .await
                    {
                        Ok(()) => {
                            info!(
                                "Moved Outlook message {} to folder {}",
                                message_id, target_folder.name
                            );
                            RemoteMutationOutcome::Applied
                        }
                        Err(e) => {
                            let error = e.to_string();
                            return queue_move_failure(&error, base_payload());
                        }
                    },
                    Err(e) => {
                        let error = e.to_string();
                        return queue_move_failure(&error, base_payload());
                    }
                }
            }
        }
        ProviderType::Imap => {
            if is_local_move {
                RemoteMutationOutcome::LocalOnly
            } else if let Ok(uid) = msg.remote_id.parse::<u32>() {
                match connect_imap(&state, &msg.account_id).await {
                    Ok(imap) => {
                        let result = imap
                            .move_message(&source_folder.remote_id, uid, &target_folder.remote_id)
                            .await;
                        let _ = imap.disconnect().await;
                        match result {
                            Ok(()) => {
                                info!(
                                    "Moved IMAP message {} (UID {}) to folder {}",
                                    message_id, uid, target_folder.name
                                );
                                RemoteMutationOutcome::Applied
                            }
                            Err(e) => {
                                let error = e.to_string();
                                return queue_move_failure(&error, base_payload());
                            }
                        }
                    }
                    Err(e) => {
                        let error = e.to_string();
                        return queue_move_failure(&error, base_payload());
                    }
                }
            } else {
                let error = format!("Invalid IMAP UID: {}", msg.remote_id);
                return queue_move_failure(&error, base_payload());
            }
        }
        ProviderType::Gmail => {
            if target_folder.role == Some(pebble_core::FolderRole::Spam) {
                match connect_gmail(&state, &msg.account_id).await {
                    Ok(provider) => match provider
                        .modify_labels(
                            &msg.remote_id,
                            &["SPAM".to_string()],
                            &["INBOX".to_string()],
                        )
                        .await
                    {
                        Ok(()) => RemoteMutationOutcome::Applied,
                        Err(e) => {
                            let error = e.to_string();
                            let mut payload = base_payload();
                            payload["add_labels"] = json!(["SPAM"]);
                            payload["remove_labels"] = json!(["INBOX"]);
                            return queue_move_failure(&error, payload);
                        }
                    },
                    Err(e) => {
                        let error = e.to_string();
                        let mut payload = base_payload();
                        payload["add_labels"] = json!(["SPAM"]);
                        payload["remove_labels"] = json!(["INBOX"]);
                        return queue_move_failure(&error, payload);
                    }
                }
            } else if is_local_move {
                RemoteMutationOutcome::LocalOnly
            } else {
                match connect_gmail(&state, &msg.account_id).await {
                    Ok(provider) => match provider
                        .modify_labels(
                            &msg.remote_id,
                            &[target_folder.remote_id.clone()],
                            &["INBOX".to_string()],
                        )
                        .await
                    {
                        Ok(()) => RemoteMutationOutcome::Applied,
                        Err(e) => {
                            let error = e.to_string();
                            let mut payload = base_payload();
                            payload["add_labels"] = json!([target_folder.remote_id.as_str()]);
                            payload["remove_labels"] = json!(["INBOX"]);
                            return queue_move_failure(&error, payload);
                        }
                    },
                    Err(e) => {
                        let error = e.to_string();
                        let mut payload = base_payload();
                        payload["add_labels"] = json!([target_folder.remote_id.as_str()]);
                        payload["remove_labels"] = json!(["INBOX"]);
                        return queue_move_failure(&error, payload);
                    }
                }
            }
        }
    };

    if !remote_mutation_allows_local_commit(outcome) {
        return Err(PebbleError::Network(
            "Remote move_to_folder was not applied".to_string(),
        ));
    }

    state
        .store
        .move_message_to_folder(&message_id, &target_folder_id)?;
    refresh_search_document(&state, &message_id)?;
    info!(
        "Moved message {} to folder {} ({})",
        message_id, target_folder.name, target_folder_id
    );
    Ok(())
}

#[tauri::command]
pub async fn empty_trash(
    state: State<'_, AppState>,
    account_id: String,
) -> std::result::Result<u32, PebbleError> {
    let trash = find_folder_by_role(&state, &account_id, FolderRole::Trash)?;
    let provider_type = state
        .store
        .get_account(&account_id)?
        .map(|account| account.provider)
        .ok_or_else(|| PebbleError::Internal(format!("Account not found: {account_id}")))?;

    let (conn, connect_error) =
        match ConnectedProvider::connect(&state, &account_id, &provider_type).await {
            Ok(conn) => (Some(conn), None),
            Err(e) => (None, Some(e.to_string())),
        };

    let mut total_deleted: u32 = 0;
    const PAGE_SIZE: u32 = 500;

    loop {
        let messages = state
            .store
            .list_messages_by_folder(&trash.id, PAGE_SIZE, 0)?;
        if messages.is_empty() {
            break;
        }

        let mut ids_to_delete: Vec<String> = Vec::new();

        if trash.remote_id.starts_with("__local_") {
            ids_to_delete.extend(messages.iter().map(|m| m.id.clone()));
        } else if let Some(ref conn) = conn {
            match conn {
                ConnectedProvider::Gmail(provider) => {
                    for msg in &messages {
                        match provider.delete_message_permanently(&msg.remote_id).await {
                            Ok(()) => ids_to_delete.push(msg.id.clone()),
                            Err(e) => {
                                let error = e.to_string();
                                warn!("Gmail permanent delete failed for {}: {error}", msg.id);
                                queue_permanent_delete_failure(
                                    &state,
                                    &account_id,
                                    &msg.id,
                                    &msg.remote_id,
                                    &trash.id,
                                    &trash.remote_id,
                                    &error,
                                )?;
                            }
                        }
                    }
                }
                ConnectedProvider::Outlook(provider) => {
                    for msg in &messages {
                        match provider.delete_message_permanently(&msg.remote_id).await {
                            Ok(()) => ids_to_delete.push(msg.id.clone()),
                            Err(e) => {
                                let error = e.to_string();
                                warn!("Outlook permanent delete failed for {}: {error}", msg.id);
                                queue_permanent_delete_failure(
                                    &state,
                                    &account_id,
                                    &msg.id,
                                    &msg.remote_id,
                                    &trash.id,
                                    &trash.remote_id,
                                    &error,
                                )?;
                            }
                        }
                    }
                }
                ConnectedProvider::Imap(imap) => {
                    for msg in &messages {
                        if let Ok(uid) = parse_imap_uid(&msg.remote_id) {
                            match imap.delete_message(&trash.remote_id, uid).await {
                                Ok(()) => ids_to_delete.push(msg.id.clone()),
                                Err(e) => {
                                    let error = e.to_string();
                                    warn!("IMAP permanent delete failed for {}: {error}", msg.id);
                                    queue_permanent_delete_failure(
                                        &state,
                                        &account_id,
                                        &msg.id,
                                        &msg.remote_id,
                                        &trash.id,
                                        &trash.remote_id,
                                        &error,
                                    )?;
                                }
                            }
                        } else {
                            let error = format!("Invalid IMAP UID: {}", msg.remote_id);
                            queue_permanent_delete_failure(
                                &state,
                                &account_id,
                                &msg.id,
                                &msg.remote_id,
                                &trash.id,
                                &trash.remote_id,
                                &error,
                            )?;
                        }
                    }
                }
            }
        } else {
            let error = connect_error
                .as_deref()
                .unwrap_or("Remote provider unavailable");
            for msg in &messages {
                queue_permanent_delete_failure(
                    &state,
                    &account_id,
                    &msg.id,
                    &msg.remote_id,
                    &trash.id,
                    &trash.remote_id,
                    error,
                )?;
            }
        }

        if ids_to_delete.is_empty() {
            break;
        }

        let batch_count = ids_to_delete.len() as u32;
        state.store.hard_delete_messages(&ids_to_delete)?;
        remove_search_documents(&state, &ids_to_delete)?;
        total_deleted += batch_count;

        if batch_count < PAGE_SIZE {
            break;
        }
    }

    if let Some(conn) = conn {
        conn.disconnect().await;
    }

    info!(
        "Emptied trash: {} messages permanently deleted",
        total_deleted
    );
    Ok(total_deleted)
}

#[cfg(test)]
mod remote_mutation_tests {
    use super::*;

    #[test]
    fn remote_mutation_allows_local_commit_only_after_remote_ack_or_local_only() {
        assert!(remote_mutation_allows_local_commit(
            RemoteMutationOutcome::Applied
        ));
        assert!(remote_mutation_allows_local_commit(
            RemoteMutationOutcome::LocalOnly
        ));
        assert!(!remote_mutation_allows_local_commit(
            RemoteMutationOutcome::Queued
        ));
        assert!(!remote_mutation_allows_local_commit(
            RemoteMutationOutcome::Failed
        ));
    }
}
