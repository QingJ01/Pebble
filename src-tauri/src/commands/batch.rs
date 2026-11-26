use super::messages::provider_dispatch::{
    archives_without_folder, delete_op, folder_change, remote_ids_are_folder_scoped,
};
use super::messages::{
    find_folder_by_role, find_message_folder, refresh_search_documents,
    remote_mutation_allows_local_commit, RemoteMutationOutcome, RemoteSession,
};
use crate::state::AppState;
use pebble_core::{FolderRole, Message, PebbleError, ProviderType};
use pebble_store::pending_ops::{FlagsOp, MailOp};
use pebble_store::Store;
use std::collections::{HashMap, HashSet};
use tauri::State;
use tracing::{info, warn};

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
#[cfg_attr(test, derive(ts_rs::TS))]
#[serde(rename_all = "camelCase")]
pub struct BatchMarkReadResult {
    /// Messages whose local read state was updated immediately.
    pub updated: u32,
    /// Messages with a pending remote retry. This may overlap `updated` when
    /// the desired state was committed locally while offline.
    pub queued: u32,
    /// Requested messages that were neither updated nor queued.
    pub failed: u32,
}

/// Group messages by account_id and resolve their provider type.
/// Uses a batch query to avoid N+1 individual lookups.
fn group_by_account(
    store: &Store,
    message_ids: &[String],
) -> std::result::Result<HashMap<String, (ProviderType, Vec<Message>)>, PebbleError> {
    let messages = store.get_messages_batch(message_ids)?;
    let mut groups: HashMap<String, (ProviderType, Vec<Message>)> = HashMap::new();
    for msg in messages {
        let provider = store
            .get_account(&msg.account_id)?
            .map(|a| a.provider)
            .unwrap_or(ProviderType::Imap);
        groups
            .entry(msg.account_id.clone())
            .or_insert_with(|| (provider, Vec::new()))
            .1
            .push(msg);
    }
    Ok(groups)
}

fn batch_mark_read_result(
    requested_ids: &[String],
    updated_ids: &[String],
    queued_ids: &[String],
) -> BatchMarkReadResult {
    let requested: HashSet<&str> = requested_ids.iter().map(String::as_str).collect();
    let updated: HashSet<&str> = updated_ids
        .iter()
        .map(String::as_str)
        .filter(|id| requested.contains(id))
        .collect();
    let queued: HashSet<&str> = queued_ids
        .iter()
        .map(String::as_str)
        .filter(|id| requested.contains(id))
        .collect();
    let resolved: HashSet<&str> = updated.union(&queued).copied().collect();
    BatchMarkReadResult {
        updated: updated.len() as u32,
        queued: queued.len() as u32,
        failed: requested.difference(&resolved).count() as u32,
    }
}

fn find_remote_message_folder(
    state: &AppState,
    message_id: &str,
    account_id: &str,
) -> std::result::Result<Option<pebble_core::Folder>, PebbleError> {
    let folder_ids = state.store.get_message_folder_ids(message_id)?;
    let folders = state.store.list_folders(account_id)?;
    Ok(folder_ids.iter().find_map(|fid| {
        folders
            .iter()
            .find(|folder| &folder.id == fid && !folder.remote_id.starts_with("__local_"))
            .cloned()
    }))
}

fn batch_local_commit_ids(
    requested_ids: &[String],
    remote_succeeded_ids: &[String],
    queued_for_local_commit_ids: &[String],
) -> Vec<String> {
    let mut commit_ids: HashSet<&str> = remote_succeeded_ids.iter().map(String::as_str).collect();
    commit_ids.extend(queued_for_local_commit_ids.iter().map(String::as_str));
    requested_ids
        .iter()
        .filter(|id| commit_ids.contains(id.as_str()))
        .cloned()
        .collect()
}

/// Apply `op` to one message of a batch. Returns whether the local change
/// may be committed; failures are logged and leave the message untouched.
async fn apply_batch_op(
    state: &AppState,
    session: &RemoteSession,
    message: &Message,
    op: MailOp,
    action: &str,
) -> bool {
    match session.apply(state, message, op).await {
        Ok(applied) => {
            if let Some(error) = applied.error.as_deref() {
                warn!("Batch {action} for {} was queued: {error}", message.id);
            }
            remote_mutation_allows_local_commit(applied.outcome)
        }
        Err(e) => {
            warn!("Batch {action} failed for {}: {e}", message.id);
            false
        }
    }
}

/// Shared preamble for every batch_* command: enforce the 1000-id cap, then
/// resolve the provider groupings off the Tokio runtime. Returns `Ok(None)`
/// for an empty input so the caller can short-circuit with a zero count.
async fn prepare_batch(
    store: std::sync::Arc<Store>,
    message_ids: &[String],
) -> std::result::Result<Option<HashMap<String, (ProviderType, Vec<Message>)>>, PebbleError> {
    if message_ids.is_empty() {
        return Ok(None);
    }
    if message_ids.len() > 1000 {
        return Err(PebbleError::Internal(
            "Batch size exceeds limit of 1000".into(),
        ));
    }
    let ids = message_ids.to_vec();
    let groups = tokio::task::spawn_blocking(move || group_by_account(&store, &ids))
        .await
        .map_err(|e| PebbleError::Internal(format!("Task join error: {e}")))??;
    Ok(Some(groups))
}

#[tauri::command]
pub async fn batch_archive(
    state: State<'_, AppState>,
    message_ids: Vec<String>,
) -> std::result::Result<u32, PebbleError> {
    let Some(groups) = prepare_batch(state.store.clone(), &message_ids).await? else {
        return Ok(0);
    };
    let mut success_count: u32 = 0;
    let mut archived_ids = Vec::new();

    for (account_id, (provider_type, messages)) in &groups {
        let archive_folder = find_folder_by_role(&state, account_id, FolderRole::Archive).ok();

        // Remote sync: connect once per account, operate, disconnect.
        // Mailbox providers need a usable (non-local) archive folder on the
        // server; skip the connection entirely when there isn't one.
        let has_remote_archive = archive_folder
            .as_ref()
            .is_some_and(|af| !af.remote_id.starts_with("__local_"));
        let needs_connection = archives_without_folder(provider_type) || has_remote_archive;

        let mut local_commit_ids: HashSet<&str> = HashSet::new();
        if needs_connection {
            let session = RemoteSession::open(&state, account_id, provider_type).await;
            for msg in messages {
                let source = find_message_folder(&state, &msg.id, account_id).ok();
                let op = MailOp::Archive(folder_change(source.as_ref(), archive_folder.as_ref()));
                if apply_batch_op(&state, &session, msg, op, "archive").await {
                    local_commit_ids.insert(&msg.id);
                }
            }
            session.close().await;
        } else {
            // No remote target needed; all messages succeed locally
            local_commit_ids.extend(messages.iter().map(|msg| msg.id.as_str()));
        }

        // Local store update: only for messages the remote accepted (or
        // that were queued while offline).
        for msg in messages
            .iter()
            .filter(|msg| local_commit_ids.contains(msg.id.as_str()))
        {
            let result = match &archive_folder {
                Some(af) => state.store.move_message_to_folder(&msg.id, &af.id),
                None => state.store.soft_delete_message(&msg.id),
            };
            match result {
                Ok(()) => {
                    success_count += 1;
                    archived_ids.push(msg.id.clone());
                }
                Err(e) => warn!("Failed to archive message {}: {e}", msg.id),
            }
        }
    }

    // Update search index for archived messages in a single commit
    if let Err(e) = refresh_search_documents(&state, &archived_ids) {
        warn!("Failed to refresh search documents after batch archive: {e}");
    }

    info!(
        "Batch archive: {}/{} messages archived",
        success_count,
        message_ids.len()
    );
    Ok(success_count)
}

#[tauri::command]
pub async fn batch_delete(
    state: State<'_, AppState>,
    message_ids: Vec<String>,
) -> std::result::Result<u32, PebbleError> {
    let Some(mut groups) = prepare_batch(state.store.clone(), &message_ids).await? else {
        return Ok(0);
    };
    let mut local_draft_count = 0;
    for (_, messages) in groups.values_mut() {
        let mut local_drafts = std::collections::HashSet::new();
        for message in messages.iter() {
            if super::drafts::delete_local_only_draft(&state, message)? {
                local_drafts.insert(message.id.clone());
                local_draft_count += 1;
            }
        }
        messages.retain(|message| !local_drafts.contains(&message.id));
    }
    groups.retain(|_, (_, messages)| !messages.is_empty());

    let mut deleted_ids: Vec<String> = Vec::new();
    // Remote sync: connect once per account, operate, disconnect.
    for (account_id, (provider_type, messages)) in &groups {
        let session = RemoteSession::open(&state, account_id, provider_type).await;
        let trash_folder = find_folder_by_role(&state, account_id, FolderRole::Trash).ok();
        let folder_scoped = remote_ids_are_folder_scoped(provider_type);
        // A connected mailbox provider without a trash folder has nowhere to
        // move messages; those messages are left untouched.
        if session.is_connected() && folder_scoped && trash_folder.is_none() {
            session.close().await;
            continue;
        }
        for msg in messages {
            let source = find_message_folder(&state, &msg.id, account_id).ok();
            // In a mailbox provider's trash, delete means expunge.
            let permanent = folder_scoped
                && source
                    .as_ref()
                    .zip(trash_folder.as_ref())
                    .is_some_and(|(src, trash)| src.id == trash.id);
            let op = delete_op(source.as_ref(), trash_folder.as_ref(), permanent);
            if apply_batch_op(&state, &session, msg, op, "delete").await {
                deleted_ids.push(msg.id.clone());
            }
        }
        session.close().await;
    }

    // Apply locally after remote success or after a provider connection failure
    // that was queued for retry. Per-message remote failures after a successful
    // connection remain remote-only failures.
    let ids_to_delete = batch_local_commit_ids(&message_ids, &deleted_ids, &[]);

    // Local bulk soft-delete
    state.store.bulk_soft_delete(&ids_to_delete)?;
    let success_count = ids_to_delete.len() as u32 + local_draft_count;

    // Update search index: remove deleted messages. The local delete has
    // already committed, so index problems are logged rather than failing
    // the command; the durable `search_pending` markers are cleared only for
    // removals that were applied AND committed, so crash/startup recovery
    // retries anything else.
    {
        let store = state.store.clone();
        let search = state.search.clone();
        let ids = ids_to_delete.clone();
        if let Err(e) =
            tokio::task::spawn_blocking(move || remove_deleted_from_search(&store, &search, &ids))
                .await
        {
            warn!("Search removal task failed after batch delete: {e}");
        }
    }

    info!(
        "Batch delete: {}/{} messages deleted",
        success_count,
        message_ids.len()
    );
    Ok(success_count)
}

/// Shared body of `batch_mark_read` and `batch_star`. Returns the ids that
/// were committed locally and the ids that got a queued remote retry.
async fn batch_update_flags(
    state: &AppState,
    message_ids: &[String],
    is_read: Option<bool>,
    is_starred: Option<bool>,
) -> std::result::Result<(Vec<String>, Vec<String>), PebbleError> {
    let Some(groups) = prepare_batch(state.store.clone(), message_ids).await? else {
        return Ok((Vec::new(), Vec::new()));
    };

    let mut synced_ids: Vec<String> = Vec::new();
    let mut queued_ids: Vec<String> = Vec::new();
    // Remote sync: connect once per account, operate, disconnect.
    for (account_id, (provider_type, messages)) in &groups {
        let session = RemoteSession::open(state, account_id, provider_type).await;
        let folder_scoped = remote_ids_are_folder_scoped(provider_type);
        for msg in messages {
            let folder_remote_id = if folder_scoped {
                match find_remote_message_folder(state, &msg.id, account_id) {
                    // Only in local folders (e.g. the local outbox): nothing
                    // exists on the server to flag.
                    Ok(None) => {
                        synced_ids.push(msg.id.clone());
                        continue;
                    }
                    Ok(Some(folder)) => Some(folder.remote_id),
                    Err(e) => {
                        warn!(
                            "Batch flag update: folder lookup failed for {}: {e}",
                            msg.id
                        );
                        None
                    }
                }
            } else {
                None
            };
            let op = MailOp::UpdateFlags(FlagsOp {
                is_read,
                is_starred,
                folder_remote_id,
                ..FlagsOp::default()
            });
            match session.apply(state, msg, op).await {
                Ok(applied) => {
                    if let Some(error) = applied.error.as_deref() {
                        warn!("Batch flag update for {} was queued: {error}", msg.id);
                    }
                    if matches!(
                        applied.outcome,
                        RemoteMutationOutcome::Queued | RemoteMutationOutcome::QueuedLocalCommit
                    ) {
                        queued_ids.push(msg.id.clone());
                    }
                    if remote_mutation_allows_local_commit(applied.outcome) {
                        synced_ids.push(msg.id.clone());
                    }
                }
                Err(e) => warn!("Batch flag update failed for {}: {e}", msg.id),
            }
        }
        session.close().await;
    }

    let ids_to_update = batch_local_commit_ids(message_ids, &synced_ids, &[]);

    // Local bulk flag update: only for messages that succeeded remotely, are
    // local-only, or were queued while offline.
    let changes: Vec<(String, Option<bool>, Option<bool>)> = ids_to_update
        .iter()
        .map(|id| (id.clone(), is_read, is_starred))
        .collect();
    state.store.bulk_update_flags(&changes)?;
    Ok((ids_to_update, queued_ids))
}

pub(crate) async fn batch_mark_read_detailed(
    state: State<'_, AppState>,
    message_ids: Vec<String>,
    is_read: bool,
) -> std::result::Result<BatchMarkReadResult, PebbleError> {
    let (updated_ids, queued_ids) =
        batch_update_flags(&state, &message_ids, Some(is_read), None).await?;
    let result = batch_mark_read_result(&message_ids, &updated_ids, &queued_ids);

    info!(
        "Batch mark_read({}): {}/{} messages updated, {} queued, {} failed",
        is_read,
        result.updated,
        message_ids.len(),
        result.queued,
        result.failed,
    );
    Ok(result)
}

#[tauri::command]
pub async fn batch_mark_read(
    state: State<'_, AppState>,
    message_ids: Vec<String>,
    is_read: bool,
) -> std::result::Result<u32, PebbleError> {
    Ok(batch_mark_read_detailed(state, message_ids, is_read)
        .await?
        .updated)
}

#[tauri::command]
pub async fn batch_star(
    state: State<'_, AppState>,
    message_ids: Vec<String>,
    starred: bool,
) -> std::result::Result<u32, PebbleError> {
    let success_count = batch_update_flags(&state, &message_ids, None, Some(starred))
        .await?
        .0
        .len() as u32;
    info!(
        "Batch star({}): {}/{} messages updated",
        starred,
        success_count,
        message_ids.len()
    );
    Ok(success_count)
}

/// Remove soft-deleted messages from the search index with crash-safe
/// `search_pending` bookkeeping (see `batch_delete`).
fn remove_deleted_from_search(
    store: &Store,
    search: &pebble_search::TantivySearch,
    ids: &[String],
) {
    if ids.is_empty() {
        return;
    }
    if let Err(e) = store.add_search_pending(ids, "remove") {
        // Without a durable marker a crash before commit would leave stale
        // documents; still attempt the removal below.
        warn!(
            "Failed to record {} pending search removals: {e}",
            ids.len()
        );
    }
    let _guard = crate::commands::indexing::search_write_guard();
    let mut removed = Vec::with_capacity(ids.len());
    for id in ids {
        match search.remove_message(id) {
            Ok(()) => removed.push(id.clone()),
            Err(e) => warn!("Failed to remove deleted message {id} from search index: {e}"),
        }
    }
    if let Err(e) = search.commit() {
        warn!("Failed to commit search index after batch delete; removals stay queued for recovery: {e}");
        return;
    }
    if let Err(e) = store.clear_search_pending(&removed) {
        warn!("Search removals committed but recovery markers could not be cleared: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::{batch_local_commit_ids, batch_mark_read_result};

    #[test]
    fn batch_local_commit_ids_include_remote_successes_and_queued_connection_failures_in_request_order(
    ) {
        let requested = vec![
            "message-1".to_string(),
            "message-2".to_string(),
            "message-3".to_string(),
        ];
        let remote_succeeded = vec!["message-3".to_string()];
        let queued_for_local_commit = vec!["message-1".to_string()];

        assert_eq!(
            batch_local_commit_ids(&requested, &remote_succeeded, &queued_for_local_commit),
            vec!["message-1".to_string(), "message-3".to_string()],
        );
    }

    #[test]
    fn batch_local_commit_ids_are_empty_when_all_remote_operations_failed_after_connecting() {
        let requested = vec!["message-1".to_string(), "message-2".to_string()];

        assert!(batch_local_commit_ids(&requested, &[], &[]).is_empty());
    }

    #[test]
    fn batch_mark_read_result_reports_updated_queued_and_failed_without_double_counting() {
        let requested = vec![
            "updated".to_string(),
            "queued".to_string(),
            "failed".to_string(),
            "updated".to_string(),
        ];
        let updated = vec!["updated".to_string(), "queued".to_string()];
        let queued = vec!["queued".to_string()];

        let result = batch_mark_read_result(&requested, &updated, &queued);

        assert_eq!(result.updated, 2);
        assert_eq!(result.queued, 1);
        assert_eq!(result.failed, 1);
    }
}
