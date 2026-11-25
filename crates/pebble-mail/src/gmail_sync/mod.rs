use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pebble_core::traits::FolderProvider;
use pebble_core::{new_id, now_timestamp, Folder, FolderRole, PebbleError, Result};
use pebble_store::Store;
use std::sync::Mutex as StdMutex;
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, warn};

use crate::backoff::SyncBackoff;
use crate::provider::gmail::{
    visible_label_ids, GmailFetchedMessage, GmailMessageMetadata, GmailMessageRef, GmailProvider,
};
use crate::realtime_policy::{RealtimePollPolicy, RealtimeRuntimeState, SyncTrigger};
use crate::sync::{
    blocking_store, message_ids_with_pending_local_changes, recv_sync_trigger,
    remote_message_requires_fetch, store_new_message_with_attachments_atomically, StoredMessage,
    SyncConfig, SyncError, SyncWorkerBase, ThreadRefresh,
};
use crate::thread::{
    assign_thread_ids, compute_thread_id, provider_thread_id, GMAIL_THREAD_NAMESPACE,
};

/// Page size used when listing a label during the first bootstrap.
mod bootstrap;
mod history;
mod poll;
#[cfg(test)]
mod tests;

use history::*;

const GMAIL_BOOTSTRAP_PAGE_SIZE: u32 = 200;

/// Per-label bootstrap progress, stored in `folder_sync_state` for the
/// label's folder while a bootstrap is unfinished. It lets an interrupted
/// first sync resume from the last fully stored page instead of listing
/// and downloading the whole label again.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct GmailLabelProgress {
    /// History checkpoint captured when the interrupted bootstrap began.
    /// Resumed runs keep using it so changes made since are replayed.
    history_id: String,
    sync_since: Option<i64>,
    /// `pageToken` of the first page not yet fully stored.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    page_token: Option<String>,
    /// Every page of the label was stored without failures.
    #[serde(default)]
    complete: bool,
}

const GMAIL_LABEL_PROGRESS_KEY: &str = "gmail_bootstrap";

fn parse_gmail_label_progress(
    state: Option<&str>,
    sync_since: Option<i64>,
) -> Option<GmailLabelProgress> {
    let value: serde_json::Value = serde_json::from_str(state?).ok()?;
    let progress: GmailLabelProgress =
        serde_json::from_value(value.get(GMAIL_LABEL_PROGRESS_KEY)?.clone()).ok()?;
    (progress.sync_since == sync_since && !progress.history_id.is_empty()).then_some(progress)
}

fn serialize_gmail_label_progress(progress: Option<&GmailLabelProgress>) -> String {
    match progress {
        Some(progress) => serde_json::json!({ GMAIL_LABEL_PROGRESS_KEY: progress }).to_string(),
        None => "{}".to_string(),
    }
}

/// History checkpoint for this bootstrap: an unfinished earlier bootstrap's
/// checkpoint wins over the current profile historyId, because labels it
/// already finished are skipped and must have their later changes replayed.
fn gmail_bootstrap_checkpoint<'a>(
    progress: impl IntoIterator<Item = &'a GmailLabelProgress>,
    profile_history_id: &str,
) -> String {
    progress
        .into_iter()
        .map(|progress| progress.history_id.clone())
        .next()
        .unwrap_or_else(|| profile_history_id.to_string())
}

fn folder_sync_priority(folder: &Folder) -> i32 {
    match folder.role {
        Some(FolderRole::Inbox) => 0,
        Some(FolderRole::Sent) => 1,
        Some(FolderRole::Drafts) => 2,
        Some(FolderRole::Trash) => 3,
        Some(FolderRole::Spam) => 4,
        Some(FolderRole::Archive) => 5,
        None => 10,
    }
}

fn build_sync_label_ids(folders: &[Folder]) -> Vec<String> {
    let mut visible: Vec<&Folder> = folders
        .iter()
        .filter(|folder| {
            !folder.remote_id.starts_with("__local_")
                && !visible_label_ids(std::slice::from_ref(&folder.remote_id)).is_empty()
        })
        .collect();

    visible.sort_by(|left, right| {
        folder_sync_priority(left)
            .cmp(&folder_sync_priority(right))
            .then(left.sort_order.cmp(&right.sort_order))
            .then(left.name.to_lowercase().cmp(&right.name.to_lowercase()))
    });

    visible
        .into_iter()
        .map(|folder| folder.remote_id.clone())
        .collect()
}

fn can_advance_gmail_cursor(failure_count: usize) -> bool {
    failure_count == 0
}

fn filter_deleted_history_additions(added_ids: Vec<String>, deleted_ids: &[String]) -> Vec<String> {
    if deleted_ids.is_empty() {
        return added_ids;
    }

    let deleted: HashSet<&str> = deleted_ids.iter().map(String::as_str).collect();
    added_ids
        .into_iter()
        .filter(|id| !deleted.contains(id.as_str()))
        .collect()
}

fn should_count_gmail_history_fetch_failure(error: &PebbleError) -> bool {
    !matches!(
        error,
        PebbleError::Network(message) if message.contains("status 404")
    )
}

fn should_notify_gmail_startup_fetch(stored_cursor: Option<&str>) -> bool {
    stored_cursor.is_some_and(|cursor| !cursor.trim().is_empty())
}

fn gmail_sync_scope_matches(
    has_saved_scope: bool,
    saved_sync_since: Option<i64>,
    sync_since: Option<i64>,
) -> bool {
    if has_saved_scope {
        saved_sync_since == sync_since
    } else {
        sync_since.is_none()
    }
}

#[derive(Debug, PartialEq, Eq)]
enum GmailSyncDispatch {
    Bootstrap,
    History(String),
}

fn gmail_sync_dispatch(sync_state: &pebble_store::accounts::SyncState) -> GmailSyncDispatch {
    let scope_matches = gmail_sync_scope_matches(
        sync_state.has_gmail_sync_scope,
        sync_state.gmail_sync_since,
        sync_state.sync_since,
    );
    match (scope_matches, sync_state.last_sync_cursor.as_deref()) {
        (true, Some(cursor)) if !cursor.trim().is_empty() => {
            GmailSyncDispatch::History(cursor.to_string())
        }
        _ => GmailSyncDispatch::Bootstrap,
    }
}

fn saved_gmail_history_cursor(sync_state: &pebble_store::accounts::SyncState) -> Option<String> {
    sync_state
        .last_sync_cursor
        .clone()
        .filter(|cursor| !cursor.trim().is_empty())
}

fn apply_gmail_history_mutations(
    store: &Store,
    account_id: &str,
    folders_by_remote: &HashMap<String, String>,
    entries: &[GmailHistoryEntry],
) -> Result<Vec<String>> {
    let mut refreshed = HashSet::new();
    // Messages with queued local operations keep their optimistic local
    // flags and labels until the operation reaches Gmail.
    let mut referenced = HashSet::new();
    for entry in entries {
        for changes in [&entry.labels_added, &entry.labels_removed] {
            for change in changes.iter().flatten() {
                referenced.insert(change.message.id.clone());
            }
        }
    }
    let referenced: Vec<String> = referenced.into_iter().collect();
    let local_ids: Vec<String> = store
        .get_existing_message_map_by_remote_ids(account_id, &referenced)?
        .into_values()
        .collect();
    let pending = store.message_ids_with_active_pending_ops(account_id, &local_ids)?;

    for change in gmail_history_flag_changes(entries) {
        if let Some(local_id) = store.find_message_id_by_remote(account_id, &change.remote_id)? {
            if pending.contains(&local_id) {
                continue;
            }
            store.update_message_flags(&local_id, change.is_read, change.is_starred)?;
            refreshed.insert(local_id);
        }
    }

    for entry in entries {
        for deleted in entry.messages_deleted.iter().flatten() {
            if let Some(local_id) =
                store.find_message_id_by_remote(account_id, &deleted.message.id)?
            {
                store.soft_delete_message(&local_id)?;
                refreshed.insert(local_id);
            }
        }
    }

    for mutation in gmail_history_label_mutations(entries) {
        if visible_label_ids(std::slice::from_ref(&mutation.label_id)).is_empty() {
            continue;
        }
        let Some(local_id) = store.find_message_id_by_remote(account_id, &mutation.remote_id)?
        else {
            continue;
        };
        if pending.contains(&local_id) {
            continue;
        }
        if let Some(folder_id) = folders_by_remote.get(&mutation.label_id) {
            match mutation.action {
                GmailHistoryLabelAction::Add => {
                    store.add_message_to_folder(&local_id, folder_id)?;
                }
                GmailHistoryLabelAction::Remove => {
                    store.remove_message_from_folder(&local_id, folder_id)?;
                }
            }
        }
        refreshed.insert(local_id);
    }
    Ok(refreshed.into_iter().collect())
}

#[derive(Debug, Clone, Copy, Default)]
struct GmailLabelSyncOutcome {
    stored_count: u32,
    failure_count: usize,
}

fn resolve_folder_ids(
    folders_by_remote: &HashMap<String, String>,
    label_ids: &[String],
    fallback_folder_id: &str,
) -> Vec<String> {
    let mut folder_ids = Vec::new();
    for label_id in label_ids {
        if let Some(folder_id) = folders_by_remote.get(label_id) {
            if !folder_ids.contains(folder_id) {
                folder_ids.push(folder_id.clone());
            }
        }
    }

    if folder_ids.is_empty() {
        folder_ids.push(fallback_folder_id.to_string());
    }

    folder_ids
}

/// Callback that refreshes the OAuth token and returns (new_access_token, expires_at).
pub type TokenRefresher = Box<
    dyn Fn() -> Pin<Box<dyn Future<Output = Result<(String, Option<i64>)>> + Send>> + Send + Sync,
>;

/// A sync worker for Gmail accounts using the REST API (HTTPS on port 443).
pub struct GmailSyncWorker {
    pub(crate) base: SyncWorkerBase,
    provider: Arc<GmailProvider>,
    stop_rx: watch::Receiver<bool>,
    token_refresher: Option<Arc<TokenRefresher>>,
    /// Last known token expiry (unix timestamp).
    token_expires_at: StdMutex<Option<i64>>,
}

impl GmailSyncWorker {
    pub fn new(
        account_id: impl Into<String>,
        provider: Arc<GmailProvider>,
        store: Arc<Store>,
        stop_rx: watch::Receiver<bool>,
        attachments_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            base: SyncWorkerBase {
                account_id: account_id.into(),
                store,
                attachments_dir: attachments_dir.into(),
                error_tx: None,
                message_tx: None,
                runtime_status_tx: None,
                progress_tx: None,
                thread_refresh_tx: None,
            },
            provider,
            stop_rx,
            token_refresher: None,
            token_expires_at: StdMutex::new(None),
        }
    }

    pub fn with_error_tx(mut self, tx: mpsc::UnboundedSender<SyncError>) -> Self {
        self.base.error_tx = Some(tx);
        self
    }

    pub fn with_message_tx(mut self, tx: mpsc::UnboundedSender<StoredMessage>) -> Self {
        self.base.message_tx = Some(tx);
        self
    }

    pub fn with_thread_refresh_tx(mut self, tx: mpsc::UnboundedSender<ThreadRefresh>) -> Self {
        self.base.thread_refresh_tx = Some(tx);
        self
    }

    pub fn with_progress_tx(
        mut self,
        tx: mpsc::UnboundedSender<crate::sync::SyncProgress>,
    ) -> Self {
        self.base.progress_tx = Some(tx);
        self
    }

    pub fn with_token_refresher(
        mut self,
        refresher: TokenRefresher,
        expires_at: Option<i64>,
    ) -> Self {
        self.token_refresher = Some(Arc::new(refresher));
        *self
            .token_expires_at
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = expires_at;
        self
    }

    fn emit_message_refresh(&self, message_id: &str) {
        let Some(tx) = &self.base.message_tx else {
            return;
        };

        let Ok(Some(message)) = self.base.store.get_message(message_id) else {
            return;
        };
        let folder_ids = self
            .base
            .store
            .get_message_folder_ids(message_id)
            .unwrap_or_default();
        // A refresh of an already-stored message (flags/labels changed on the
        // server): re-index it, but never surface it as new mail or run user
        // rules again — that would fight the user's own moves.
        let _ = tx.send(StoredMessage {
            message,
            folder_ids: folder_ids.clone(),
            notify: false,
            reconciliation: true,
        });
        self.base
            .emit_list_refresh(folder_ids, vec![message_id.to_string()]);
    }

    async fn store_fetched_message(
        &self,
        fetched: GmailFetchedMessage,
        fallback_folder_id: &str,
        folders_by_remote: &HashMap<String, String>,
        notify: bool,
    ) -> Result<bool> {
        let GmailFetchedMessage {
            mut message,
            visible_label_ids,
            attachments,
        } = fetched;

        let account_id = self.base.account_id.clone();
        let remote_ids = vec![message.remote_id.clone()];
        let incomplete = blocking_store(&self.base.store, move |store| {
            store.get_incomplete_attachment_message_map_by_remote_ids(
                &account_id,
                None,
                &remote_ids,
            )
        })
        .await?;
        let repairing_incomplete = if let Some(local_id) = incomplete.get(&message.remote_id) {
            message.id = local_id.clone();
            true
        } else {
            false
        };

        // Gmail already groups messages into threads; use its threadId so
        // arrival order (e.g. INBOX before SENT) cannot split a conversation.
        message.thread_id =
            match provider_thread_id(GMAIL_THREAD_NAMESPACE, message.thread_id.as_deref()) {
                Some(thread_id) => Some(thread_id),
                None => {
                    let refs: Vec<String> = [&message.in_reply_to, &message.references_header]
                        .into_iter()
                        .flatten()
                        .flat_map(|header| header.split_whitespace())
                        .map(str::to_string)
                        .collect();
                    let account_id = self.base.account_id.clone();
                    let mappings = blocking_store(&self.base.store, move |store| {
                        store.get_thread_mappings_for_refs(&account_id, &refs)
                    })
                    .await
                    .unwrap_or_default();
                    Some(compute_thread_id(&message, &mappings))
                }
            };

        let folder_ids =
            resolve_folder_ids(folders_by_remote, &visible_label_ids, fallback_folder_id);

        store_new_message_with_attachments_atomically(
            Arc::clone(&self.base.store),
            self.base.attachments_dir.clone(),
            message.clone(),
            folder_ids.clone(),
            attachments,
        )
        .await?;

        self.base.emit_message(StoredMessage {
            message,
            folder_ids,
            notify: notify && !repairing_incomplete,
            reconciliation: repairing_incomplete,
        });

        Ok(true)
    }

    /// Ensure the access token is still valid; refresh if needed.
    async fn ensure_valid_token(&self) -> Result<()> {
        let now = now_timestamp();
        let needs_refresh = {
            let expires = self
                .token_expires_at
                .lock()
                .unwrap_or_else(|e| e.into_inner());
            match *expires {
                Some(exp) => now >= exp - 300, // 5 minute buffer
                None => false,                 // No expiry info — assume valid
            }
        };

        if needs_refresh {
            if let Some(ref refresher) = self.token_refresher {
                debug!(
                    "Refreshing Gmail OAuth token for account {}",
                    self.base.account_id
                );
                match refresher().await {
                    Ok((new_token, new_expires_at)) => {
                        self.provider.set_access_token(new_token);
                        let mut expires = self
                            .token_expires_at
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        *expires = new_expires_at.or(Some(now + 3600));
                        info!(
                            "Gmail OAuth token refreshed for account {}",
                            self.base.account_id
                        );
                    }
                    Err(e) => {
                        warn!("Failed to refresh OAuth token: {}", e);
                        self.base
                            .emit_error("auth", &format!("Token refresh failed: {e}"));
                        return Err(PebbleError::Auth(format!("Token refresh failed: {e}")));
                    }
                }
            }
        }
        Ok(())
    }

    async fn refresh_folders(&self) -> Result<()> {
        let folders = self.provider.list_folders().await?;
        for mut folder in folders {
            folder.account_id = self.base.account_id.clone();
            self.base.store.insert_folder(&folder)?;
        }

        // Clean up any previously-synced hidden Gmail labels from the local store
        let hidden = [
            "CHAT",
            "IMPORTANT",
            "STARRED",
            "UNREAD",
            "CATEGORY_FORUMS",
            "CATEGORY_UPDATES",
            "CATEGORY_PERSONAL",
            "CATEGORY_PROMOTIONS",
            "CATEGORY_SOCIAL",
        ];
        for label_id in &hidden {
            self.base
                .store
                .delete_folder_by_remote_id(&self.base.account_id, label_id)?;
        }

        // Ensure an Archive folder exists locally
        let local_folders = self.base.store.list_folders(&self.base.account_id)?;
        let has_archive = local_folders
            .iter()
            .any(|f| f.role == Some(pebble_core::FolderRole::Archive));
        if !has_archive {
            let archive = pebble_core::Folder {
                id: new_id(),
                account_id: self.base.account_id.clone(),
                remote_id: "__local_archive__".to_string(),
                name: "Archive".to_string(),
                folder_type: pebble_core::FolderType::Folder,
                role: Some(pebble_core::FolderRole::Archive),
                parent_id: None,
                color: None,
                is_system: true,
                sort_order: 3,
            };
            self.base.store.insert_folder(&archive)?;
        }

        Ok(())
    }
}
