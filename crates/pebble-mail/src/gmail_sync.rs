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
    recv_sync_trigger, remote_message_requires_fetch,
    store_new_message_with_attachments_atomically, StoredMessage, SyncConfig, SyncError,
    SyncWorkerBase,
};
use crate::thread::compute_thread_id;

fn collect_ref_ids_from_messages(messages: &[GmailFetchedMessage]) -> Vec<String> {
    let mut refs = std::collections::HashSet::new();
    for fetched in messages {
        if let Some(irt) = &fetched.message.in_reply_to {
            for id in irt.split_whitespace() {
                refs.insert(id.trim().to_string());
            }
        }
        if let Some(r) = &fetched.message.references_header {
            for id in r.split_whitespace() {
                refs.insert(id.trim().to_string());
            }
        }
    }
    refs.into_iter().collect()
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
    for change in gmail_history_flag_changes(entries) {
        if let Some(local_id) = store.find_message_id_by_remote(account_id, &change.remote_id)? {
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

async fn collect_paginated_gmail_refs<F, Fut>(
    label_id: &str,
    limit: u32,
    mut fetch_page: F,
) -> Result<Vec<GmailMessageRef>>
where
    F: FnMut(String, u32, Option<String>) -> Fut,
    Fut: Future<Output = Result<(Vec<GmailMessageRef>, Option<String>)>>,
{
    let mut all_refs = Vec::new();
    let mut page_token = None;

    loop {
        let (mut refs, next_page) =
            fetch_page(label_id.to_string(), limit, page_token.take()).await?;
        all_refs.append(&mut refs);

        match next_page {
            Some(token) if !token.is_empty() => page_token = Some(token),
            _ => break,
        }
    }

    Ok(all_refs)
}

#[derive(Debug, serde::Deserialize)]
struct GmailHistoryPage {
    history: Option<Vec<GmailHistoryEntry>>,
    #[serde(rename = "historyId")]
    history_id: Option<String>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
struct GmailHistoryEntry {
    #[serde(rename = "messagesAdded")]
    messages_added: Option<Vec<GmailHistoryMessage>>,
    #[serde(rename = "messagesDeleted")]
    messages_deleted: Option<Vec<GmailHistoryMessage>>,
    #[serde(rename = "labelsAdded")]
    labels_added: Option<Vec<GmailHistoryLabelChange>>,
    #[serde(rename = "labelsRemoved")]
    labels_removed: Option<Vec<GmailHistoryLabelChange>>,
}

#[derive(Debug, serde::Deserialize)]
struct GmailHistoryMessage {
    message: GmailHistoryMessageRef,
}

#[derive(Debug, serde::Deserialize)]
struct GmailHistoryLabelChange {
    message: GmailHistoryMessageRef,
    #[serde(rename = "labelIds")]
    label_ids: Vec<String>,
}

#[derive(Debug, serde::Deserialize)]
struct GmailHistoryMessageRef {
    id: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GmailHistoryLabelAction {
    Add,
    Remove,
}

#[derive(Debug, PartialEq, Eq)]
struct GmailHistoryLabelMutation {
    remote_id: String,
    label_id: String,
    action: GmailHistoryLabelAction,
}

fn gmail_history_label_mutations(entries: &[GmailHistoryEntry]) -> Vec<GmailHistoryLabelMutation> {
    let mut sequence = 0usize;
    let mut last_events: HashMap<(String, String), (usize, GmailHistoryLabelAction)> =
        HashMap::new();

    for entry in entries {
        for (changes, action) in [
            (&entry.labels_added, GmailHistoryLabelAction::Add),
            (&entry.labels_removed, GmailHistoryLabelAction::Remove),
        ] {
            for change in changes.iter().flatten() {
                for label_id in &change.label_ids {
                    last_events.insert(
                        (change.message.id.clone(), label_id.clone()),
                        (sequence, action),
                    );
                    sequence += 1;
                }
            }
        }
    }

    let mut final_events = last_events
        .into_iter()
        .map(|((remote_id, label_id), (sequence, action))| {
            (
                sequence,
                GmailHistoryLabelMutation {
                    remote_id,
                    label_id,
                    action,
                },
            )
        })
        .collect::<Vec<_>>();
    final_events.sort_by_key(|(sequence, _)| *sequence);
    final_events
        .into_iter()
        .map(|(_, mutation)| mutation)
        .collect()
}

#[derive(Debug, PartialEq, Eq)]
struct GmailHistoryFlagChange {
    remote_id: String,
    is_read: Option<bool>,
    is_starred: Option<bool>,
}

fn gmail_history_flag_changes(entries: &[GmailHistoryEntry]) -> Vec<GmailHistoryFlagChange> {
    let mut by_message: HashMap<String, (usize, Option<bool>, Option<bool>)> = HashMap::new();
    for (sequence, mutation) in gmail_history_label_mutations(entries)
        .into_iter()
        .enumerate()
    {
        let entry = by_message
            .entry(mutation.remote_id)
            .or_insert((sequence, None, None));
        entry.0 = sequence;
        match mutation.label_id.as_str() {
            "UNREAD" => {
                entry.1 = Some(matches!(mutation.action, GmailHistoryLabelAction::Remove));
            }
            "STARRED" => {
                entry.2 = Some(matches!(mutation.action, GmailHistoryLabelAction::Add));
            }
            _ => {}
        }
    }

    let mut changes = by_message
        .into_iter()
        .filter_map(|(remote_id, (sequence, is_read, is_starred))| {
            (is_read.is_some() || is_starred.is_some()).then_some((
                sequence,
                GmailHistoryFlagChange {
                    remote_id,
                    is_read,
                    is_starred,
                },
            ))
        })
        .collect::<Vec<_>>();
    changes.sort_by_key(|(sequence, _)| *sequence);
    changes.into_iter().map(|(_, change)| change).collect()
}

enum GmailHistoryPageFetch {
    Page(GmailHistoryPage),
    CursorExpired,
}

enum GmailHistoryReplayOutcome {
    Applied(Option<String>),
    CursorExpired,
}

async fn replay_gmail_history_and_checkpoint<F, Fut>(
    replay: F,
    checkpoint: impl FnOnce(&str) -> Result<()>,
) -> Result<GmailHistoryReplayOutcome>
where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<GmailHistoryReplayOutcome>>,
{
    let outcome = replay().await?;
    if let GmailHistoryReplayOutcome::Applied(Some(history_id)) = &outcome {
        checkpoint(history_id)?;
    }
    Ok(outcome)
}

async fn fetch_cached_gmail_metadata<F, Fut>(
    remote_ids: Vec<String>,
    fetch: F,
) -> Result<Vec<(String, Option<GmailMessageMetadata>)>>
where
    F: Fn(String) -> Fut + Clone + Send + 'static,
    Fut: Future<Output = Result<Option<GmailMessageMetadata>>> + Send,
{
    use futures::stream::{self, StreamExt};
    let fetched = stream::iter(remote_ids.into_iter().map(|remote_id| {
        let fetch = fetch.clone();
        async move {
            let metadata = fetch(remote_id.clone()).await?;
            Ok((remote_id, metadata))
        }
    }))
    .buffer_unordered(10)
    .collect::<Vec<Result<_>>>()
    .await;
    fetched.into_iter().collect()
}

struct GmailHistoryBatch {
    entries: Vec<GmailHistoryEntry>,
    history_id: Option<String>,
    cursor_expired: bool,
}

fn gmail_history_cursor_expired(status: reqwest::StatusCode) -> bool {
    status == reqwest::StatusCode::NOT_FOUND
}

async fn collect_paginated_gmail_history<F, Fut>(
    _history_id: &str,
    mut fetch_page: F,
) -> Result<GmailHistoryBatch>
where
    F: FnMut(Option<String>) -> Fut,
    Fut: Future<Output = Result<GmailHistoryPageFetch>>,
{
    let mut entries = Vec::new();
    let mut history_id = None;
    let mut page_token = None;

    loop {
        let page = match fetch_page(page_token.take()).await? {
            GmailHistoryPageFetch::Page(page) => page,
            GmailHistoryPageFetch::CursorExpired => {
                return Ok(GmailHistoryBatch {
                    entries: Vec::new(),
                    history_id: None,
                    cursor_expired: true,
                });
            }
        };
        entries.extend(page.history.unwrap_or_default());
        if page.history_id.is_some() {
            history_id = page.history_id;
        }

        match page.next_page_token {
            Some(token) if !token.is_empty() => page_token = Some(token),
            _ => break,
        }
    }

    Ok(GmailHistoryBatch {
        entries,
        history_id,
        cursor_expired: false,
    })
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
        let _ = tx.send(StoredMessage {
            message,
            folder_ids,
            notify: false,
            reconciliation: false,
        });
    }

    async fn store_fetched_message(
        &self,
        fetched: GmailFetchedMessage,
        fallback_folder_id: &str,
        thread_mappings: &mut HashMap<String, String>,
        folders_by_remote: &HashMap<String, String>,
        notify: bool,
    ) -> Result<bool> {
        let GmailFetchedMessage {
            mut message,
            visible_label_ids,
            attachments,
        } = fetched;

        let remote_ids = vec![message.remote_id.clone()];
        let incomplete = self
            .base
            .store
            .get_incomplete_attachment_message_map_by_remote_ids(
                &self.base.account_id,
                None,
                &remote_ids,
            )?;
        let repairing_incomplete = if let Some(local_id) = incomplete.get(&message.remote_id) {
            message.id = local_id.clone();
            true
        } else {
            false
        };

        let thread_id = compute_thread_id(&message, thread_mappings);
        message.thread_id = Some(thread_id);

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

        if let (Some(mid), Some(tid)) = (&message.message_id_header, &message.thread_id) {
            thread_mappings.insert(mid.clone(), tid.clone());
        }
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

    /// Perform initial sync: list folders, fetch messages for each label.
    async fn initial_sync(&self) -> Result<()> {
        info!(
            "Starting Gmail initial sync for account {}",
            self.base.account_id
        );

        self.refresh_folders().await?;

        // Get the stored sync cursor (historyId) if any
        let sync_state = self
            .base
            .store
            .get_sync_state(&self.base.account_id)?
            .unwrap_or_default();
        let sync_since = sync_state.sync_since;
        let stored_cursor = saved_gmail_history_cursor(&sync_state);
        let notify_new = should_notify_gmail_startup_fetch(stored_cursor.as_deref());

        // Capture the history checkpoint before replay or bootstrap so changes
        // arriving during either operation are replayed on the next poll.
        let (_email, profile_history_id) = self.provider.get_profile().await?;
        if let Some(cursor) = stored_cursor.as_deref() {
            match self.replay_gmail_history(cursor).await? {
                GmailHistoryReplayOutcome::Applied(_) => {}
                GmailHistoryReplayOutcome::CursorExpired => {
                    self.reconcile_cached_gmail_metadata().await?;
                }
            }
        }

        // Sync every visible remote label, prioritizing system folders first.
        let all_folders = self.base.store.list_folders(&self.base.account_id)?;
        let labels_to_sync = build_sync_label_ids(&all_folders);
        let folders_by_remote: HashMap<String, String> = all_folders
            .into_iter()
            .map(|f| (f.remote_id, f.id))
            .collect();
        let limit = if stored_cursor.is_some() { 50 } else { 200 };
        let mut thread_mappings = self.base.store.get_thread_mappings(&self.base.account_id)?;
        let mut failure_count = 0usize;

        for label_id in &labels_to_sync {
            match self
                .sync_label(
                    label_id,
                    limit,
                    &folders_by_remote,
                    &mut thread_mappings,
                    notify_new,
                    sync_since,
                )
                .await
            {
                Ok(outcome) => {
                    failure_count += outcome.failure_count;
                }
                Err(e) => {
                    failure_count += 1;
                    warn!("Gmail sync label {} failed: {}", label_id, e);
                }
            }
        }

        // Store the historyId as sync cursor for future delta syncs
        if !profile_history_id.is_empty() && can_advance_gmail_cursor(failure_count) {
            self.base
                .store
                .update_sync_state(&self.base.account_id, |state| {
                    state.last_sync_cursor = Some(profile_history_id.clone());
                    state.gmail_sync_since = sync_since;
                    state.has_gmail_sync_scope = true;
                })?;
        } else if failure_count > 0 {
            warn!(
                "Gmail initial sync had {} failures; keeping previous history cursor",
                failure_count
            );
            return Err(PebbleError::Network(format!(
                "Gmail initial sync had {failure_count} failed label syncs"
            )));
        }

        info!(
            "Gmail initial sync completed for account {}",
            self.base.account_id
        );
        Ok(())
    }

    async fn reconcile_cached_gmail_metadata(&self) -> Result<()> {
        let folders = self.base.store.list_folders(&self.base.account_id)?;
        let visible_folders = folders
            .iter()
            .filter(|folder| {
                !folder.remote_id.starts_with("__local_")
                    && !visible_label_ids(std::slice::from_ref(&folder.remote_id)).is_empty()
            })
            .map(|folder| (folder.remote_id.clone(), folder.id.clone()))
            .collect::<HashMap<_, _>>();
        let folders_by_id = folders
            .iter()
            .map(|folder| (folder.id.clone(), folder.clone()))
            .collect::<HashMap<_, _>>();

        let remote_ids = self
            .base
            .store
            .list_cached_remote_ids_by_account(&self.base.account_id)?;
        let provider = Arc::clone(&self.provider);
        let fetched = fetch_cached_gmail_metadata(remote_ids, move |remote_id| {
            let provider = Arc::clone(&provider);
            async move { provider.fetch_message_metadata(&remote_id).await }
        })
        .await?;

        for (remote_id, metadata) in fetched {
            let Some(local_id) = self
                .base
                .store
                .find_message_id_by_remote(&self.base.account_id, &remote_id)?
            else {
                continue;
            };
            let Some(metadata) = metadata else {
                self.base.store.soft_delete_message(&local_id)?;
                self.emit_message_refresh(&local_id);
                continue;
            };
            if metadata.id != remote_id {
                return Err(PebbleError::Network(format!(
                    "Mismatched Gmail metadata for {remote_id}"
                )));
            }

            self.base.store.update_message_flags(
                &local_id,
                Some(!metadata.label_ids.iter().any(|label| label == "UNREAD")),
                Some(metadata.label_ids.iter().any(|label| label == "STARRED")),
            )?;
            let desired_labels: HashSet<_> =
                visible_label_ids(&metadata.label_ids).into_iter().collect();
            let existing_folder_ids = self.base.store.get_message_folder_ids(&local_id)?;
            for folder_id in existing_folder_ids {
                if folders_by_id.get(&folder_id).is_some_and(|folder| {
                    !folder.remote_id.starts_with("__local_")
                        && !visible_label_ids(std::slice::from_ref(&folder.remote_id)).is_empty()
                        && !desired_labels.contains(&folder.remote_id)
                }) {
                    self.base
                        .store
                        .remove_message_from_folder(&local_id, &folder_id)?;
                }
            }
            for label_id in &desired_labels {
                if let Some(folder_id) = visible_folders.get(label_id) {
                    self.base
                        .store
                        .add_message_to_folder(&local_id, folder_id)?;
                }
            }
            self.emit_message_refresh(&local_id);
        }
        Ok(())
    }

    /// Sync messages for a specific Gmail label.
    async fn sync_label(
        &self,
        label_id: &str,
        limit: u32,
        folders_by_remote: &HashMap<String, String>,
        thread_mappings: &mut HashMap<String, String>,
        notify_new: bool,
        sync_since: Option<i64>,
    ) -> Result<GmailLabelSyncOutcome> {
        let folder_id = match folders_by_remote.get(label_id) {
            Some(id) => id.clone(),
            None => {
                debug!("No local folder found for label {}, skipping", label_id);
                return Ok(GmailLabelSyncOutcome::default());
            }
        };

        // List all message IDs from Gmail. The list endpoint is paginated and
        // silently truncates the initial sync if the next page token is ignored.
        let msg_refs =
            collect_paginated_gmail_refs(label_id, limit, |label_id, limit, page_token| {
                let provider = Arc::clone(&self.provider);
                async move {
                    provider
                        .list_message_ids(&label_id, limit, page_token.as_deref(), sync_since)
                        .await
                }
            })
            .await?;
        if msg_refs.is_empty() {
            return Ok(GmailLabelSyncOutcome::default());
        }

        let remote_ids: Vec<String> = msg_refs.iter().map(|r| r.id.clone()).collect();
        let existing = self
            .base
            .store
            .get_existing_message_map_by_remote_ids(&self.base.account_id, &remote_ids)?;
        let incomplete = self
            .base
            .store
            .get_incomplete_attachment_message_map_by_remote_ids(
                &self.base.account_id,
                None,
                &remote_ids,
            )?;

        let mut outcome = GmailLabelSyncOutcome::default();

        // Separate already-existing messages (just add folder) from new ones to fetch
        let mut to_fetch = Vec::new();
        for msg_ref in msg_refs {
            if remote_message_requires_fetch(
                existing.contains_key(&msg_ref.id),
                incomplete.contains_key(&msg_ref.id),
            ) {
                to_fetch.push(msg_ref.id.clone());
            } else if let Some(local_id) = existing.get(&msg_ref.id) {
                if let Err(e) = self.base.store.add_message_to_folder(local_id, &folder_id) {
                    outcome.failure_count += 1;
                    warn!(
                        "Failed to add Gmail label {} to existing message {}: {}",
                        label_id, msg_ref.id, e
                    );
                } else {
                    self.emit_message_refresh(local_id);
                }
            } else {
                to_fetch.push(msg_ref.id.clone());
            }
        }

        // Fetch new messages concurrently (up to 10 in flight)
        use futures::stream::{self, StreamExt};
        let fetched_results: Vec<_> = stream::iter(to_fetch.into_iter().map(|gmail_id| {
            let provider = Arc::clone(&self.provider);
            let account_id = self.base.account_id.clone();
            async move {
                let result = provider.fetch_sync_message(&gmail_id, &account_id).await;
                (gmail_id, result)
            }
        }))
        .buffer_unordered(10)
        .collect()
        .await;

        for (gmail_id, result) in fetched_results {
            match result {
                Ok(fetched) => match self
                    .store_fetched_message(
                        fetched,
                        &folder_id,
                        thread_mappings,
                        folders_by_remote,
                        notify_new,
                    )
                    .await
                {
                    Ok(true) => outcome.stored_count += 1,
                    Ok(false) => {}
                    Err(e) => {
                        outcome.failure_count += 1;
                        error!("Failed to store Gmail message {}: {}", gmail_id, e);
                    }
                },
                Err(e) => {
                    outcome.failure_count += 1;
                    warn!("Failed to fetch Gmail message {}: {}", gmail_id, e);
                }
            }
        }

        if outcome.stored_count > 0 {
            info!(
                "Stored {} messages for label {}",
                outcome.stored_count, label_id
            );
        }
        Ok(outcome)
    }

    async fn replay_gmail_history(&self, history_id: &str) -> Result<GmailHistoryReplayOutcome> {
        let history_cursor = history_id.to_string();
        let provider = Arc::clone(&self.provider);
        let history = collect_paginated_gmail_history(history_id, move |page_token| {
            let provider = Arc::clone(&provider);
            let history_cursor = history_cursor.clone();
            async move {
                let mut url =
                    reqwest::Url::parse("https://www.googleapis.com/gmail/v1/users/me/history")
                        .map_err(|e| {
                            PebbleError::Network(format!("Build Gmail history URL: {e}"))
                        })?;
                {
                    let mut query = url.query_pairs_mut();
                    query.append_pair("startHistoryId", &history_cursor);
                    if let Some(page_token) = page_token {
                        query.append_pair("pageToken", &page_token);
                    }
                }

                let resp = provider.get(url.as_str()).await?;
                let status = resp.status();
                if gmail_history_cursor_expired(status) {
                    return Ok(GmailHistoryPageFetch::CursorExpired);
                }
                if !status.is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    return Err(PebbleError::Network(format!(
                        "Gmail history request failed with status {status}: {body}"
                    )));
                }

                let page = resp.json::<GmailHistoryPage>().await.map_err(|e| {
                    PebbleError::Network(format!("Parse Gmail history response: {e}"))
                })?;
                Ok(GmailHistoryPageFetch::Page(page))
            }
        })
        .await?;

        if history.cursor_expired {
            return Ok(GmailHistoryReplayOutcome::CursorExpired);
        }

        let mut new_ids = Vec::new();
        let mut deleted_ids = Vec::new();
        let mut failure_count = 0usize;

        for entry in &history.entries {
            if let Some(ref added) = entry.messages_added {
                for m in added {
                    new_ids.push(m.message.id.clone());
                }
            }
            if let Some(ref deleted) = entry.messages_deleted {
                for m in deleted {
                    deleted_ids.push(m.message.id.clone());
                }
            }
        }

        let folders_by_remote: HashMap<String, String> = self
            .base
            .store
            .list_folders(&self.base.account_id)?
            .into_iter()
            .map(|folder| (folder.remote_id, folder.id))
            .collect();
        for local_id in apply_gmail_history_mutations(
            &self.base.store,
            &self.base.account_id,
            &folders_by_remote,
            &history.entries,
        )? {
            self.emit_message_refresh(&local_id);
        }
        if !deleted_ids.is_empty() {
            info!("Applied {} Gmail history deletions", deleted_ids.len());
        }

        let new_ids = filter_deleted_history_additions(new_ids, &deleted_ids);

        // Fetch new messages concurrently, then collect refs and store.
        if !new_ids.is_empty() {
            let existing = match self
                .base
                .store
                .get_existing_message_map_by_remote_ids(&self.base.account_id, &new_ids)
            {
                Ok(existing) => existing,
                Err(e) => {
                    failure_count += 1;
                    warn!("Failed to look up existing Gmail history messages: {}", e);
                    HashMap::new()
                }
            };
            let incomplete = match self
                .base
                .store
                .get_incomplete_attachment_message_map_by_remote_ids(
                    &self.base.account_id,
                    None,
                    &new_ids,
                ) {
                Ok(incomplete) => incomplete,
                Err(e) => {
                    failure_count += 1;
                    warn!("Failed to inspect incomplete Gmail history messages: {e}");
                    HashMap::new()
                }
            };

            // Emit refresh for already-known messages; collect truly new IDs.
            let to_fetch: Vec<String> = new_ids
                .into_iter()
                .filter(|gid| {
                    if remote_message_requires_fetch(
                        existing.contains_key(gid),
                        incomplete.contains_key(gid),
                    ) {
                        true
                    } else if let Some(local_id) = existing.get(gid) {
                        self.emit_message_refresh(local_id);
                        false
                    } else {
                        true
                    }
                })
                .collect();

            if !to_fetch.is_empty() {
                let inbox_folder_id = folders_by_remote.get("INBOX").cloned().unwrap_or_default();

                // Phase 1: fetch all messages concurrently.
                use futures::stream::{self, StreamExt};
                let fetched_results: Vec<_> = stream::iter(to_fetch.into_iter().map(|gmail_id| {
                    let provider = Arc::clone(&self.provider);
                    let account_id = self.base.account_id.clone();
                    async move {
                        let result = provider.fetch_sync_message(&gmail_id, &account_id).await;
                        (gmail_id, result)
                    }
                }))
                .buffer_unordered(10)
                .collect()
                .await;

                let mut fetched_messages = Vec::new();
                for (gmail_id, result) in fetched_results {
                    match result {
                        Ok(fetched) => fetched_messages.push(fetched),
                        Err(e) => {
                            if should_count_gmail_history_fetch_failure(&e) {
                                failure_count += 1;
                                warn!("Failed to fetch history message {}: {}", gmail_id, e);
                            } else {
                                debug!(
                                    "Skipping stale Gmail history message {} after fetch returned not found",
                                    gmail_id
                                );
                            }
                        }
                    }
                }

                // Phase 2: collect refs from fetched messages for targeted thread lookup.
                let ref_ids = collect_ref_ids_from_messages(&fetched_messages);
                let mut thread_mappings = self
                    .base
                    .store
                    .get_thread_mappings_for_refs(&self.base.account_id, &ref_ids)?;

                // Phase 3: store all fetched messages.
                for fetched in fetched_messages {
                    let gmail_id = fetched.message.remote_id.clone();
                    if let Err(e) = self
                        .store_fetched_message(
                            fetched,
                            &inbox_folder_id,
                            &mut thread_mappings,
                            &folders_by_remote,
                            true,
                        )
                        .await
                    {
                        failure_count += 1;
                        warn!("Failed to store history message {}: {}", gmail_id, e);
                    }
                }
            }
        }

        if failure_count > 0 {
            return Err(PebbleError::Network(format!(
                "Gmail history replay had {failure_count} failures"
            )));
        }

        Ok(GmailHistoryReplayOutcome::Applied(history.history_id))
    }

    /// Poll for new messages using the Gmail History API (delta sync).
    async fn poll_changes(&self) -> Result<()> {
        let sync_state = self
            .base
            .store
            .get_sync_state(&self.base.account_id)?
            .unwrap_or_default();
        let history_id = match gmail_sync_dispatch(&sync_state) {
            GmailSyncDispatch::History(id) => id,
            GmailSyncDispatch::Bootstrap => {
                debug!("No history cursor or the scope changed; performing bootstrap sync");
                return self.initial_sync().await;
            }
        };

        match replay_gmail_history_and_checkpoint(
            || self.replay_gmail_history(&history_id),
            |cursor| {
                self.base
                    .store
                    .set_sync_cursor(&self.base.account_id, cursor)
            },
        )
        .await?
        {
            GmailHistoryReplayOutcome::CursorExpired => {
                warn!(
                    "Gmail history cursor expired for account {}; running a full resync",
                    self.base.account_id
                );
                self.initial_sync().await
            }
            GmailHistoryReplayOutcome::Applied(Some(_)) => Ok(()),
            GmailHistoryReplayOutcome::Applied(None) => Ok(()),
        }
    }

    async fn run_poll_cycle(&self, backoff: &mut SyncBackoff, allow_circuit_attempt: bool) {
        if backoff.is_circuit_open() {
            warn!(
                "Circuit open for Gmail account {} ({} failures), current delay {:?}",
                self.base.account_id,
                backoff.failure_count(),
                backoff.current_delay()
            );
            if !allow_circuit_attempt {
                return;
            }
        }

        self.base.emit_sync_started("poll");
        if let Err(e) = self.ensure_valid_token().await {
            warn!("Token refresh failed: {}", e);
            self.base.emit_sync_error("poll", &e.to_string());
            let _ = backoff.record_failure();
            return;
        }
        match self.poll_changes().await {
            Ok(()) => {
                self.base.emit_sync_completed("poll");
                backoff.record_success()
            }
            Err(e) => {
                warn!(
                    "Gmail poll failed for account {}: {}",
                    self.base.account_id, e
                );
                self.base.emit_error("sync", &format!("Poll failed: {e}"));
                self.base.emit_sync_error("poll", &e.to_string());
                let _ = backoff.record_failure();
            }
        }
    }

    /// Main sync loop.
    pub async fn run(
        &self,
        config: SyncConfig,
        trigger_rx: Option<mpsc::UnboundedReceiver<SyncTrigger>>,
    ) {
        // Ensure token is valid before starting
        if let Err(e) = self.ensure_valid_token().await {
            error!(
                "Token validation failed for account {}: {}",
                self.base.account_id, e
            );
            self.base
                .emit_error("auth", &format!("Token validation failed: {e}"));
            self.base.emit_sync_error("auth", &e.to_string());
            return;
        }

        let startup_dispatch = self
            .base
            .store
            .get_sync_state(&self.base.account_id)
            .map(|state| gmail_sync_dispatch(&state.unwrap_or_default()));

        // Initial startup pass. Existing accounts must use Gmail History delta;
        // otherwise startup can skip older changes outside the latest-label fetch
        // window and then incorrectly advance the cursor.
        self.base.emit_sync_started("initial");
        let startup_result = match startup_dispatch {
            Ok(GmailSyncDispatch::History(_)) => match self.refresh_folders().await {
                Ok(()) => self.poll_changes().await,
                Err(e) => Err(e),
            },
            Ok(GmailSyncDispatch::Bootstrap) => self.initial_sync().await,
            Err(error) => Err(error),
        };

        if let Err(e) = startup_result {
            error!(
                "Gmail startup sync failed for account {}: {}",
                self.base.account_id, e
            );
            self.base
                .emit_error("sync", &format!("Startup sync failed: {e}"));
            self.base.emit_sync_error("initial", &e.to_string());
            // Don't return — still enter poll loop so we can retry
        } else {
            self.base.emit_sync_completed("initial");
        }

        if config.manual_only() {
            info!(
                "Gmail manual sync completed for account {}",
                self.base.account_id
            );
            return;
        }

        let policy = RealtimePollPolicy::from_foreground_interval_secs(config.poll_interval_secs);
        let mut stop_rx = self.stop_rx.clone();
        let mut backoff = SyncBackoff::new();
        let mut trigger_rx = trigger_rx;
        let mut runtime = RealtimeRuntimeState::new(Duration::from_secs(60), Instant::now());

        loop {
            let next_delay =
                policy.next_delay(runtime.context(backoff.failure_count(), Instant::now()));

            tokio::select! {
                _ = stop_rx.changed() => {
                    if *stop_rx.borrow() {
                        info!("Gmail sync stopped for account {}", self.base.account_id);
                        break;
                    }
                }
                _ = tokio::time::sleep(next_delay) => {
                    self.run_poll_cycle(&mut backoff, true).await;
                }
                trigger = recv_sync_trigger(&mut trigger_rx) => {
                    match trigger {
                        Some(trigger) => {
                            runtime.record_trigger(trigger, Instant::now());
                            if trigger.should_sync_now() {
                                self.run_poll_cycle(
                                    &mut backoff,
                                    trigger.bypasses_circuit_backoff(),
                                ).await;
                            }
                        }
                        None => {
                            trigger_rx = None;
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_core::{Account, Folder, FolderRole, FolderType, Message, ProviderType};
    use std::collections::VecDeque;

    fn gmail_test_message(id: &str, remote_id: &str) -> Message {
        let now = now_timestamp();
        Message {
            id: id.to_string(),
            account_id: "gmail-account".to_string(),
            remote_id: remote_id.to_string(),
            message_id_header: None,
            in_reply_to: None,
            references_header: None,
            thread_id: None,
            subject: id.to_string(),
            snippet: String::new(),
            from_address: String::new(),
            from_name: String::new(),
            to_list: Vec::new(),
            cc_list: Vec::new(),
            bcc_list: Vec::new(),
            body_text: String::new(),
            body_html_raw: String::new(),
            has_attachments: false,
            is_read: true,
            is_starred: true,
            is_draft: false,
            date: now,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn gmail_test_folder(id: &str, remote_id: &str) -> Folder {
        Folder {
            id: id.to_string(),
            account_id: "gmail-account".to_string(),
            remote_id: remote_id.to_string(),
            name: remote_id.to_string(),
            folder_type: FolderType::Label,
            role: if remote_id == "INBOX" {
                Some(FolderRole::Inbox)
            } else {
                None
            },
            parent_id: None,
            color: None,
            is_system: remote_id == "INBOX",
            sort_order: 0,
        }
    }

    fn gmail_test_account() -> Account {
        let now = now_timestamp();
        Account {
            account_label: None,
            provider_display_name: None,
            id: "gmail-account".to_string(),
            email: "gmail@example.com".to_string(),
            display_name: "Gmail".to_string(),
            color: None,
            provider: ProviderType::Gmail,
            created_at: now,
            updated_at: now,
        }
    }

    struct GmailTestServer {
        endpoint: String,
        requests: mpsc::UnboundedReceiver<String>,
        responses: mpsc::UnboundedSender<(u16, String)>,
        task: tokio::task::JoinHandle<()>,
    }

    impl GmailTestServer {
        async fn next_request(&mut self) -> String {
            tokio::time::timeout(std::time::Duration::from_secs(10), self.requests.recv())
                .await
                .expect("timed out waiting for Gmail request")
                .expect("Gmail test server stopped")
        }

        fn respond(&self, status: u16, body: &str) {
            self.responses
                .send((status, body.to_string()))
                .expect("Gmail test server stopped");
        }

        fn stop(self) {
            self.task.abort();
        }
    }

    async fn start_gmail_test_server() -> GmailTestServer {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use tokio::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!("http://{}", listener.local_addr().unwrap());
        let (request_tx, requests) = mpsc::unbounded_channel();
        let (responses, mut response_rx) = mpsc::unbounded_channel::<(u16, String)>();
        let task = tokio::spawn(async move {
            loop {
                let Ok((mut stream, _)) = listener.accept().await else {
                    break;
                };
                let mut request = Vec::new();
                let mut buffer = [0; 1024];
                loop {
                    let Ok(count) = stream.read(&mut buffer).await else {
                        return;
                    };
                    if count == 0 {
                        return;
                    }
                    request.extend_from_slice(&buffer[..count]);
                    if request.windows(4).any(|window| window == b"\r\n\r\n") {
                        break;
                    }
                }
                let request_line = String::from_utf8_lossy(&request)
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .to_string();
                let request_path = request_line
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_string();
                if request_tx.send(request_path).is_err() {
                    break;
                }
                let Some((status, body)) = response_rx.recv().await else {
                    break;
                };
                let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                if stream.write_all(response.as_bytes()).await.is_err() {
                    break;
                }
            }
        });
        GmailTestServer {
            endpoint,
            requests,
            responses,
            task,
        }
    }

    fn prepare_gmail_sync_store(saved_since: i64, current_since: i64) -> Arc<Store> {
        let store = Arc::new(Store::open_in_memory().unwrap());
        store.insert_account(&gmail_test_account()).unwrap();
        for folder in [
            gmail_test_folder("gmail-inbox", "INBOX"),
            gmail_test_folder("gmail-projects", "Label_Projects"),
            gmail_test_folder("gmail-keep", "Label_Keep"),
        ] {
            store.insert_folder(&folder).unwrap();
        }
        store
            .update_sync_state("gmail-account", |state| {
                state.last_sync_cursor = Some("history-old".to_string());
                state.sync_since = Some(current_since);
                state.gmail_sync_since = Some(saved_since);
                state.has_gmail_sync_scope = true;
            })
            .unwrap();
        store
    }

    fn gmail_test_worker(store: Arc<Store>, endpoint: String) -> GmailSyncWorker {
        let provider = Arc::new(GmailProvider::new("test-token".to_string()));
        provider.set_test_api_endpoint(endpoint);
        let (_stop_tx, stop_rx) = watch::channel(false);
        GmailSyncWorker::new(
            "gmail-account",
            provider,
            store,
            stop_rx,
            std::env::temp_dir().join(new_id()),
        )
    }

    fn gmail_labels_response() -> &'static str {
        r#"{"labels":[{"id":"INBOX","name":"Inbox","type":"system"},{"id":"Label_Projects","name":"Projects","type":"user"},{"id":"Label_Keep","name":"Keep","type":"user"}]}"#
    }

    async fn respond_to_request(
        server: &mut GmailTestServer,
        observed: &mut Vec<String>,
        expected: &str,
        status: u16,
        body: &str,
    ) -> String {
        let request = server.next_request().await;
        assert!(
            request.contains(expected),
            "expected {expected:?} in request {request:?}"
        );
        observed.push(request.clone());
        server.respond(status, body);
        request
    }

    async fn respond_to_empty_bootstrap_labels(
        server: &mut GmailTestServer,
        observed: &mut Vec<String>,
        count: usize,
    ) {
        for _ in 0..count {
            respond_to_request(
                server,
                observed,
                "/messages?labelIds=",
                200,
                r#"{"messages":[]}"#,
            )
            .await;
        }
    }

    #[tokio::test]
    async fn scope_mismatch_poll_replays_history_before_bootstrap_checkpoint() {
        let store = prepare_gmail_sync_store(100, 200);
        let inbox_id = "gmail-inbox".to_string();
        let projects_id = "gmail-projects".to_string();
        let keep_id = "gmail-keep".to_string();
        let flag_message = gmail_test_message("flag-local", "flag-message");
        let deleted_message = gmail_test_message("deleted-local", "deleted-message");
        let removed_label_message = gmail_test_message("removed-local", "removed-label");
        let added_label_message = gmail_test_message("added-local", "added-label");
        for message in [
            &flag_message,
            &deleted_message,
            &removed_label_message,
            &added_label_message,
        ] {
            store
                .insert_message(message, std::slice::from_ref(&inbox_id))
                .unwrap();
        }
        store
            .add_message_to_folder(&removed_label_message.id, &projects_id)
            .unwrap();

        let mut server = start_gmail_test_server().await;
        let worker = gmail_test_worker(Arc::clone(&store), server.endpoint.clone());
        let worker_task = tokio::spawn(async move { worker.poll_changes().await });
        let mut observed = Vec::new();

        respond_to_request(
            &mut server,
            &mut observed,
            "/labels",
            200,
            gmail_labels_response(),
        )
        .await;
        respond_to_request(
            &mut server,
            &mut observed,
            "/profile",
            200,
            r#"{"emailAddress":"gmail@example.com","historyId":"profile-checkpoint"}"#,
        )
        .await;
        respond_to_request(
            &mut server,
            &mut observed,
            "/history?startHistoryId=history-old",
            200,
            r#"{"history":[{"labelsAdded":[{"message":{"id":"flag-message"},"labelIds":["UNREAD"]},{"message":{"id":"added-label"},"labelIds":["Label_Keep"]}]},{"labelsRemoved":[{"message":{"id":"flag-message"},"labelIds":["STARRED"]},{"message":{"id":"removed-label"},"labelIds":["Label_Projects"]}]},{"messagesDeleted":[{"message":{"id":"deleted-message"}}]}],"historyId":"history-replayed"}"#,
        )
        .await;

        let bootstrap_request = server.next_request().await;
        assert!(bootstrap_request.contains("/messages?labelIds="));
        observed.push(bootstrap_request.clone());
        let state = store.get_sync_state("gmail-account").unwrap().unwrap();
        assert_eq!(state.last_sync_cursor.as_deref(), Some("history-old"));
        assert_eq!(state.gmail_sync_since, Some(100));
        assert_eq!(state.sync_since, Some(200));

        let flag = store.get_message(&flag_message.id).unwrap().unwrap();
        assert!(!flag.is_read);
        assert!(!flag.is_starred);
        assert!(
            store
                .get_message(&deleted_message.id)
                .unwrap()
                .unwrap()
                .is_deleted
        );
        let removed_folders = store
            .get_message_folder_ids(&removed_label_message.id)
            .unwrap();
        assert!(!removed_folders.contains(&projects_id));
        let added_folders = store
            .get_message_folder_ids(&added_label_message.id)
            .unwrap();
        assert!(added_folders.contains(&keep_id));

        server.respond(200, r#"{"messages":[]}"#);
        respond_to_empty_bootstrap_labels(&mut server, &mut observed, 2).await;
        worker_task.await.unwrap().unwrap();

        assert!(observed[1].contains("/profile"));
        assert!(observed[2].contains("/history?startHistoryId=history-old"));
        let state = store.get_sync_state("gmail-account").unwrap().unwrap();
        assert_eq!(
            state.last_sync_cursor.as_deref(),
            Some("profile-checkpoint")
        );
        assert_eq!(state.gmail_sync_since, Some(200));
        assert!(state.has_gmail_sync_scope);
        server.stop();
    }

    #[tokio::test]
    async fn expired_history_initial_sync_refreshes_folderless_metadata_without_bodies() {
        let store = prepare_gmail_sync_store(100, 200);
        let folderless = gmail_test_message("folderless-local", "folderless-cached");
        let deleted = gmail_test_message("deleted-local", "deleted-cached");
        store.insert_message(&folderless, &[]).unwrap();
        store
            .insert_message(&deleted, &["gmail-inbox".to_string()])
            .unwrap();

        let mut server = start_gmail_test_server().await;
        let worker = gmail_test_worker(Arc::clone(&store), server.endpoint.clone());
        let worker_task = tokio::spawn(async move { worker.poll_changes().await });
        let mut observed = Vec::new();

        respond_to_request(
            &mut server,
            &mut observed,
            "/labels",
            200,
            gmail_labels_response(),
        )
        .await;
        respond_to_request(
            &mut server,
            &mut observed,
            "/profile",
            200,
            r#"{"emailAddress":"gmail@example.com","historyId":"profile-checkpoint"}"#,
        )
        .await;
        respond_to_request(
            &mut server,
            &mut observed,
            "/history?startHistoryId=history-old",
            404,
            r#"{"error":"history expired"}"#,
        )
        .await;

        for _ in 0..2 {
            let request = server.next_request().await;
            assert!(request.contains("?format=minimal"), "{request}");
            observed.push(request.clone());
            if request.contains("/messages/folderless-cached?") {
                server.respond(
                    200,
                    r#"{"id":"folderless-cached","labelIds":["UNREAD","STARRED","Label_Keep"]}"#,
                );
            } else if request.contains("/messages/deleted-cached?") {
                server.respond(404, r#"{"error":"not found"}"#);
            } else {
                panic!("unexpected metadata request: {request}");
            }
        }

        let bootstrap_request = server.next_request().await;
        assert!(bootstrap_request.contains("/messages?labelIds="));
        observed.push(bootstrap_request.clone());
        let state = store.get_sync_state("gmail-account").unwrap().unwrap();
        assert_eq!(state.last_sync_cursor.as_deref(), Some("history-old"));
        assert_eq!(state.gmail_sync_since, Some(100));

        server.respond(200, r#"{"messages":[]}"#);
        respond_to_empty_bootstrap_labels(&mut server, &mut observed, 2).await;
        worker_task.await.unwrap().unwrap();

        let folderless = store.get_message(&folderless.id).unwrap().unwrap();
        assert!(!folderless.is_read);
        assert!(folderless.is_starred);
        assert!(store
            .get_message_folder_ids(&folderless.id)
            .unwrap()
            .contains(&"gmail-keep".to_string()));
        assert!(store.get_message(&deleted.id).unwrap().unwrap().is_deleted);
        assert!(observed
            .iter()
            .all(|request| !request.contains("format=full") && !request.contains("/attachments/")));
        assert_eq!(
            store
                .get_sync_state("gmail-account")
                .unwrap()
                .unwrap()
                .last_sync_cursor
                .as_deref(),
            Some("profile-checkpoint")
        );
        server.stop();
    }

    #[tokio::test]
    async fn expired_history_metadata_failure_keeps_old_cursor_and_scope() {
        let store = prepare_gmail_sync_store(100, 200);
        let folderless = gmail_test_message("folderless-local", "folderless-failure");
        store.insert_message(&folderless, &[]).unwrap();

        let mut server = start_gmail_test_server().await;
        let worker = gmail_test_worker(Arc::clone(&store), server.endpoint.clone());
        let worker_task = tokio::spawn(async move { worker.poll_changes().await });
        let mut observed = Vec::new();

        respond_to_request(
            &mut server,
            &mut observed,
            "/labels",
            200,
            gmail_labels_response(),
        )
        .await;
        respond_to_request(
            &mut server,
            &mut observed,
            "/profile",
            200,
            r#"{"emailAddress":"gmail@example.com","historyId":"profile-checkpoint"}"#,
        )
        .await;
        respond_to_request(
            &mut server,
            &mut observed,
            "/history?startHistoryId=history-old",
            404,
            r#"{"error":"history expired"}"#,
        )
        .await;
        respond_to_request(
            &mut server,
            &mut observed,
            "/messages/folderless-failure?format=minimal",
            500,
            r#"{"error":"temporarily unavailable"}"#,
        )
        .await;

        assert!(worker_task.await.unwrap().is_err());
        let state = store.get_sync_state("gmail-account").unwrap().unwrap();
        assert_eq!(state.last_sync_cursor.as_deref(), Some("history-old"));
        assert_eq!(state.gmail_sync_since, Some(100));
        assert_eq!(state.sync_since, Some(200));
        assert!(!observed
            .iter()
            .any(|request| request.contains("/messages?labelIds=")));
        server.stop();
    }

    #[tokio::test]
    async fn expired_history_bootstrap_failure_keeps_old_cursor_and_scope() {
        let store = prepare_gmail_sync_store(100, 200);
        let folderless = gmail_test_message("folderless-local", "folderless-bootstrap");
        store.insert_message(&folderless, &[]).unwrap();

        let mut server = start_gmail_test_server().await;
        let worker = gmail_test_worker(Arc::clone(&store), server.endpoint.clone());
        let worker_task = tokio::spawn(async move { worker.initial_sync().await });
        let mut observed = Vec::new();

        respond_to_request(
            &mut server,
            &mut observed,
            "/labels",
            200,
            gmail_labels_response(),
        )
        .await;
        respond_to_request(
            &mut server,
            &mut observed,
            "/profile",
            200,
            r#"{"emailAddress":"gmail@example.com","historyId":"profile-checkpoint"}"#,
        )
        .await;
        respond_to_request(
            &mut server,
            &mut observed,
            "/history?startHistoryId=history-old",
            404,
            r#"{"error":"history expired"}"#,
        )
        .await;
        respond_to_request(
            &mut server,
            &mut observed,
            "/messages/folderless-bootstrap?format=minimal",
            200,
            r#"{"id":"folderless-bootstrap","labelIds":["INBOX"]}"#,
        )
        .await;
        respond_to_request(
            &mut server,
            &mut observed,
            "/messages?labelIds=",
            500,
            r#"{"error":"bootstrap failed"}"#,
        )
        .await;
        respond_to_empty_bootstrap_labels(&mut server, &mut observed, 2).await;

        assert!(worker_task.await.unwrap().is_err());
        let state = store.get_sync_state("gmail-account").unwrap().unwrap();
        assert_eq!(state.last_sync_cursor.as_deref(), Some("history-old"));
        assert_eq!(state.gmail_sync_since, Some(100));
        assert_eq!(state.sync_since, Some(200));
        server.stop();
    }

    #[test]
    fn test_build_sync_label_ids_includes_visible_custom_labels() {
        let folders = vec![
            Folder {
                id: "inbox".to_string(),
                account_id: "acct".to_string(),
                remote_id: "INBOX".to_string(),
                name: "Inbox".to_string(),
                folder_type: FolderType::Label,
                role: Some(FolderRole::Inbox),
                parent_id: None,
                color: None,
                is_system: true,
                sort_order: 0,
            },
            Folder {
                id: "starred".to_string(),
                account_id: "acct".to_string(),
                remote_id: "STARRED".to_string(),
                name: "Starred".to_string(),
                folder_type: FolderType::Label,
                role: None,
                parent_id: None,
                color: None,
                is_system: true,
                sort_order: 1,
            },
            Folder {
                id: "custom".to_string(),
                account_id: "acct".to_string(),
                remote_id: "Label_Projects".to_string(),
                name: "Projects".to_string(),
                folder_type: FolderType::Label,
                role: None,
                parent_id: None,
                color: None,
                is_system: false,
                sort_order: 5,
            },
            Folder {
                id: "trash".to_string(),
                account_id: "acct".to_string(),
                remote_id: "TRASH".to_string(),
                name: "Trash".to_string(),
                folder_type: FolderType::Label,
                role: Some(FolderRole::Trash),
                parent_id: None,
                color: None,
                is_system: true,
                sort_order: 6,
            },
            Folder {
                id: "local-archive".to_string(),
                account_id: "acct".to_string(),
                remote_id: "__local_archive__".to_string(),
                name: "Archive".to_string(),
                folder_type: FolderType::Folder,
                role: Some(FolderRole::Archive),
                parent_id: None,
                color: None,
                is_system: true,
                sort_order: 7,
            },
        ];

        let label_ids = build_sync_label_ids(&folders);
        assert_eq!(
            label_ids,
            vec![
                "INBOX".to_string(),
                "TRASH".to_string(),
                "Label_Projects".to_string(),
            ]
        );
    }

    #[test]
    fn gmail_cursor_does_not_advance_with_failures() {
        assert!(!can_advance_gmail_cursor(1));
    }

    #[test]
    fn gmail_cursor_advances_without_failures() {
        assert!(can_advance_gmail_cursor(0));
    }

    #[test]
    fn gmail_history_additions_exclude_messages_deleted_in_same_batch() {
        let added = vec![
            "deleted-message".to_string(),
            "surviving-message".to_string(),
        ];
        let deleted = vec!["deleted-message".to_string()];

        assert_eq!(
            filter_deleted_history_additions(added, &deleted),
            vec!["surviving-message".to_string()]
        );
    }

    #[test]
    fn gmail_history_fetch_404_does_not_count_as_cursor_blocking_failure() {
        let error = PebbleError::Network(
            "Failed to fetch message gone-message (status 404 Not Found): {}".to_string(),
        );

        assert!(!should_count_gmail_history_fetch_failure(&error));
    }

    #[test]
    fn gmail_history_fetch_non_404_counts_as_cursor_blocking_failure() {
        let error = PebbleError::Network(
            "Failed to fetch message rate-limited-message (status 429 Too Many Requests): {}"
                .to_string(),
        );

        assert!(should_count_gmail_history_fetch_failure(&error));
    }

    #[test]
    fn gmail_startup_fetch_notifies_only_when_history_cursor_exists() {
        assert!(!should_notify_gmail_startup_fetch(None));
        assert!(!should_notify_gmail_startup_fetch(Some("")));
        assert!(should_notify_gmail_startup_fetch(Some("history-1")));
    }

    #[test]
    fn scope_mismatch_bootstrap_replays_pending_flags_deletions_and_labels() {
        let sync_state = pebble_store::accounts::SyncState {
            last_sync_cursor: Some("history-old".to_string()),
            sync_since: Some(86_400),
            gmail_sync_since: Some(0),
            has_gmail_sync_scope: true,
            ..Default::default()
        };
        assert_eq!(
            gmail_sync_dispatch(&sync_state),
            GmailSyncDispatch::Bootstrap
        );
        let history_cursor = saved_gmail_history_cursor(&sync_state);
        assert_eq!(history_cursor.as_deref(), Some("history-old"));

        let store = Store::open_in_memory().unwrap();
        store.insert_account(&gmail_test_account()).unwrap();
        let inbox = gmail_test_folder("gmail-inbox", "INBOX");
        let projects = gmail_test_folder("gmail-projects", "Label_Projects");
        let keep = gmail_test_folder("gmail-keep", "Label_Keep");
        for folder in [&inbox, &projects, &keep] {
            store.insert_folder(folder).unwrap();
        }
        let flag_message = gmail_test_message("local-flag", "flag-message");
        let deleted_message = gmail_test_message("local-deleted", "deleted-message");
        let removed_label_message = gmail_test_message("local-removed-label", "removed-label");
        let added_label_message = gmail_test_message("local-added-label", "added-label");
        for message in [
            &flag_message,
            &deleted_message,
            &removed_label_message,
            &added_label_message,
        ] {
            store
                .insert_message(message, std::slice::from_ref(&inbox.id))
                .unwrap();
        }
        store
            .add_message_to_folder(&removed_label_message.id, &projects.id)
            .unwrap();

        let page = serde_json::from_value::<GmailHistoryPage>(serde_json::json!({
            "history": [
                {"labelsAdded": [
                    {"message": {"id": "flag-message"}, "labelIds": ["UNREAD"]},
                    {"message": {"id": "added-label"}, "labelIds": ["Label_Keep"]}
                ]},
                {"labelsRemoved": [
                    {"message": {"id": "flag-message"}, "labelIds": ["STARRED"]},
                    {"message": {"id": "removed-label"}, "labelIds": ["Label_Projects"]}
                ]},
                {"messagesDeleted": [{"message": {"id": "deleted-message"}}]}
            ]
        }))
        .unwrap();
        let folders_by_remote = HashMap::from([
            ("INBOX".to_string(), inbox.id.clone()),
            ("Label_Projects".to_string(), projects.id.clone()),
            ("Label_Keep".to_string(), keep.id.clone()),
        ]);

        apply_gmail_history_mutations(
            &store,
            "gmail-account",
            &folders_by_remote,
            &page.history.unwrap(),
        )
        .unwrap();

        let updated_flag = store.get_message(&flag_message.id).unwrap().unwrap();
        assert!(!updated_flag.is_read);
        assert!(!updated_flag.is_starred);
        assert!(
            store
                .get_message(&deleted_message.id)
                .unwrap()
                .unwrap()
                .is_deleted
        );
        assert!(!store
            .get_message_folder_ids(&removed_label_message.id)
            .unwrap()
            .contains(&projects.id));
        assert!(store
            .get_message_folder_ids(&added_label_message.id)
            .unwrap()
            .contains(&keep.id));
    }

    #[tokio::test]
    async fn failed_gmail_history_replay_does_not_checkpoint_cursor() {
        let mut checkpoint_count = 0;
        let result = replay_gmail_history_and_checkpoint(
            || async {
                Err(PebbleError::Network(
                    "history event application failed".to_string(),
                ))
            },
            |_| {
                checkpoint_count += 1;
                Ok(())
            },
        )
        .await;

        assert!(result.is_err());
        assert_eq!(checkpoint_count, 0);
    }

    #[tokio::test]
    async fn expired_gmail_history_replay_does_not_checkpoint_cursor() {
        let mut checkpoint_count = 0;
        let outcome = replay_gmail_history_and_checkpoint(
            || async { Ok(GmailHistoryReplayOutcome::CursorExpired) },
            |_| {
                checkpoint_count += 1;
                Ok(())
            },
        )
        .await
        .unwrap();

        assert!(matches!(outcome, GmailHistoryReplayOutcome::CursorExpired));
        assert_eq!(checkpoint_count, 0);
    }

    #[tokio::test]
    async fn expired_gmail_metadata_failure_is_retryable_without_checkpoint() {
        let store = Store::open_in_memory().unwrap();
        store.insert_account(&gmail_test_account()).unwrap();
        store
            .set_sync_cursor("gmail-account", "history-old")
            .unwrap();
        store
            .update_sync_state("gmail-account", |state| {
                state.gmail_sync_since = Some(86_400);
                state.has_gmail_sync_scope = true;
            })
            .unwrap();

        let error = fetch_cached_gmail_metadata(vec!["cached-message".to_string()], |_| async {
            Err(PebbleError::Network(
                "metadata service unavailable".to_string(),
            ))
        })
        .await
        .unwrap_err();

        assert!(error.to_string().contains("metadata service unavailable"));
        let state = store.get_sync_state("gmail-account").unwrap().unwrap();
        assert_eq!(state.last_sync_cursor.as_deref(), Some("history-old"));
        assert_eq!(state.gmail_sync_since, Some(86_400));
    }

    #[test]
    fn gmail_scope_dispatch_retries_partial_backfill_for_expansion_and_all_mail() {
        let mut state = pebble_store::accounts::SyncState {
            last_sync_cursor: Some("history-old".to_string()),
            sync_since: Some(0),
            gmail_sync_since: Some(86_400),
            has_gmail_sync_scope: true,
            ..Default::default()
        };

        assert_eq!(gmail_sync_dispatch(&state), GmailSyncDispatch::Bootstrap);
        assert!(!can_advance_gmail_cursor(1));
        assert_eq!(gmail_sync_dispatch(&state), GmailSyncDispatch::Bootstrap);

        state.sync_since = None;
        assert_eq!(gmail_sync_dispatch(&state), GmailSyncDispatch::Bootstrap);

        state.gmail_sync_since = None;
        state.last_sync_cursor = Some("history-new".to_string());
        assert_eq!(
            gmail_sync_dispatch(&state),
            GmailSyncDispatch::History("history-new".to_string())
        );
    }

    #[tokio::test]
    async fn collect_paginated_gmail_refs_fetches_until_next_page_is_empty() {
        let mut pages = VecDeque::from([
            (
                vec![crate::provider::gmail::GmailMessageRef {
                    id: "gmail-1".to_string(),
                    thread_id: None,
                }],
                Some("page-2".to_string()),
            ),
            (
                vec![crate::provider::gmail::GmailMessageRef {
                    id: "gmail-2".to_string(),
                    thread_id: None,
                }],
                None,
            ),
        ]);
        let mut requested_tokens = Vec::new();

        let refs = collect_paginated_gmail_refs("INBOX", 2, |label_id, limit, page_token| {
            requested_tokens.push((label_id, limit, page_token));
            let page = pages.pop_front().expect("expected a page request");
            async move { Ok(page) }
        })
        .await
        .unwrap();

        let ids: Vec<_> = refs.into_iter().map(|r| r.id).collect();
        assert_eq!(ids, vec!["gmail-1".to_string(), "gmail-2".to_string()]);
        assert_eq!(
            requested_tokens,
            vec![
                ("INBOX".to_string(), 2, None),
                ("INBOX".to_string(), 2, Some("page-2".to_string())),
            ]
        );
    }

    #[tokio::test]
    async fn collect_paginated_gmail_history_fetches_every_page() {
        let mut pages = VecDeque::from([
            serde_json::from_value::<GmailHistoryPage>(serde_json::json!({
                "history": [{"messagesAdded": [{"message": {"id": "gmail-1"}}]}],
                "historyId": "history-2",
                "nextPageToken": "page-2"
            }))
            .unwrap(),
            serde_json::from_value::<GmailHistoryPage>(serde_json::json!({
                "history": [{"messagesAdded": [{"message": {"id": "gmail-2"}}]}],
                "historyId": "history-2"
            }))
            .unwrap(),
        ]);
        let mut requested_tokens = Vec::new();

        let batch = collect_paginated_gmail_history("history-1", |page_token| {
            requested_tokens.push(page_token);
            let page = pages.pop_front().expect("expected a history page request");
            async move { Ok(GmailHistoryPageFetch::Page(page)) }
        })
        .await
        .unwrap();

        assert_eq!(
            batch
                .entries
                .iter()
                .flat_map(|entry| entry.messages_added.iter().flatten())
                .map(|message| message.message.id.as_str())
                .collect::<Vec<_>>(),
            vec!["gmail-1", "gmail-2"]
        );
        assert_eq!(batch.history_id.as_deref(), Some("history-2"));
        assert_eq!(requested_tokens, vec![None, Some("page-2".to_string())]);
    }

    #[tokio::test]
    async fn collect_paginated_gmail_history_reports_expired_cursor() {
        let batch = collect_paginated_gmail_history("expired-history", |_| async {
            Ok(GmailHistoryPageFetch::CursorExpired)
        })
        .await
        .unwrap();

        assert!(batch.cursor_expired);
        assert!(batch.entries.is_empty());
        assert!(batch.history_id.is_none());
    }

    #[test]
    fn gmail_history_404_means_cursor_expired() {
        assert!(gmail_history_cursor_expired(reqwest::StatusCode::NOT_FOUND));
        assert!(!gmail_history_cursor_expired(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR
        ));
    }

    #[test]
    fn gmail_history_labels_update_read_and_starred_flags() {
        let page = serde_json::from_value::<GmailHistoryPage>(serde_json::json!({
            "history": [
                {"labelsAdded": [{
                    "message": {"id": "gmail-1"},
                    "labelIds": ["UNREAD", "STARRED"]
                }]},
                {"labelsRemoved": [{
                    "message": {"id": "gmail-2"},
                    "labelIds": ["UNREAD", "STARRED"]
                }]}
            ],
            "historyId": "history-2"
        }))
        .unwrap();

        let changes = gmail_history_flag_changes(&page.history.unwrap());

        assert_eq!(changes.len(), 2);
        assert_eq!(changes[0].remote_id, "gmail-1");
        assert_eq!(changes[0].is_read, Some(false));
        assert_eq!(changes[0].is_starred, Some(true));
        assert_eq!(changes[1].remote_id, "gmail-2");
        assert_eq!(changes[1].is_read, Some(true));
        assert_eq!(changes[1].is_starred, Some(false));
    }

    #[test]
    fn gmail_history_label_add_then_remove_finishes_without_label() {
        let page = serde_json::from_value::<GmailHistoryPage>(serde_json::json!({
            "history": [
                {"labelsAdded": [{
                    "message": {"id": "gmail-1"},
                    "labelIds": ["Label_Projects"]
                }]},
                {"labelsRemoved": [{
                    "message": {"id": "gmail-1"},
                    "labelIds": ["Label_Projects"]
                }]}
            ]
        }))
        .unwrap();

        assert_eq!(
            gmail_history_label_mutations(&page.history.unwrap()),
            vec![GmailHistoryLabelMutation {
                remote_id: "gmail-1".to_string(),
                label_id: "Label_Projects".to_string(),
                action: GmailHistoryLabelAction::Remove,
            }]
        );
    }

    #[test]
    fn gmail_history_label_remove_then_add_finishes_with_label() {
        let page = serde_json::from_value::<GmailHistoryPage>(serde_json::json!({
            "history": [
                {"labelsRemoved": [{
                    "message": {"id": "gmail-1"},
                    "labelIds": ["Label_Projects"]
                }]},
                {"labelsAdded": [{
                    "message": {"id": "gmail-1"},
                    "labelIds": ["Label_Projects"]
                }]}
            ]
        }))
        .unwrap();

        assert_eq!(
            gmail_history_label_mutations(&page.history.unwrap()),
            vec![GmailHistoryLabelMutation {
                remote_id: "gmail-1".to_string(),
                label_id: "Label_Projects".to_string(),
                action: GmailHistoryLabelAction::Add,
            }]
        );
    }

    #[test]
    fn gmail_history_label_failures_keep_cursor_for_retry() {
        assert!(!can_advance_gmail_cursor(1));
        assert!(can_advance_gmail_cursor(0));
    }
}
