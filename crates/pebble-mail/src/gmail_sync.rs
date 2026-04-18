use std::collections::HashMap;
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
use crate::provider::gmail::{visible_label_ids, GmailFetchedMessage, GmailMessageRef, GmailProvider};
use crate::realtime_policy::{RealtimePollPolicy, RealtimeRuntimeState, SyncTrigger};
use crate::sync::{
    persist_message_attachments_async, recv_sync_trigger, StoredMessage, SyncConfig, SyncError, SyncWorkerBase,
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
        });
    }

    async fn store_fetched_message(
        &self,
        fetched: GmailFetchedMessage,
        fallback_folder_id: &str,
        thread_mappings: &mut HashMap<String, String>,
        folders_by_remote: &HashMap<String, String>,
    ) -> Result<bool> {
        let GmailFetchedMessage {
            mut message,
            visible_label_ids,
            attachments,
        } = fetched;

        let thread_id = compute_thread_id(&message, thread_mappings);
        message.thread_id = Some(thread_id);

        let folder_ids =
            resolve_folder_ids(folders_by_remote, &visible_label_ids, fallback_folder_id);

        self.base.store.insert_message(&message, &folder_ids)?;
        persist_message_attachments_async(
            Arc::clone(&self.base.store),
            self.base.attachments_dir.clone(),
            message.id.clone(),
            attachments,
        )
        .await;

        if let (Some(mid), Some(tid)) = (&message.message_id_header, &message.thread_id) {
            thread_mappings.insert(mid.clone(), tid.clone());
        }
        self.base.emit_message(StoredMessage {
            message,
            folder_ids,
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

    /// Perform initial sync: list folders, fetch messages for each label.
    async fn initial_sync(&self) -> Result<()> {
        info!(
            "Starting Gmail initial sync for account {}",
            self.base.account_id
        );

        // Sync folders (labels) — hidden labels are already filtered by the provider
        let folders = self.provider.list_folders().await?;
        for mut folder in folders {
            folder.account_id = self.base.account_id.clone();
            let _ = self.base.store.insert_folder(&folder);
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
            let _ = self
                .base
                .store
                .delete_folder_by_remote_id(&self.base.account_id, label_id);
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
            let _ = self.base.store.insert_folder(&archive);
        }

        // Get the stored sync cursor (historyId) if any
        let stored_cursor = self
            .base
            .store
            .get_sync_cursor(&self.base.account_id)
            .ok()
            .flatten();

        // Get the user profile for the latest historyId
        let (_email, profile_history_id) = self.provider.get_profile().await?;

        // Sync every visible remote label, prioritizing system folders first.
        let all_folders = self.base.store.list_folders(&self.base.account_id)?;
        let labels_to_sync = build_sync_label_ids(&all_folders);
        let folders_by_remote: HashMap<String, String> = all_folders
            .into_iter()
            .map(|f| (f.remote_id, f.id))
            .collect();
        let limit = if stored_cursor.is_some() { 50 } else { 200 };
        let mut thread_mappings = self
            .base
            .store
            .get_thread_mappings(&self.base.account_id)
            .unwrap_or_default();
        let mut failure_count = 0usize;

        for label_id in &labels_to_sync {
            match self
                .sync_label(label_id, limit, &folders_by_remote, &mut thread_mappings)
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
            let _ = self
                .base
                .store
                .set_sync_cursor(&self.base.account_id, &profile_history_id);
        } else if failure_count > 0 {
            warn!(
                "Gmail initial sync had {} failures; keeping previous history cursor",
                failure_count
            );
        }

        info!(
            "Gmail initial sync completed for account {}",
            self.base.account_id
        );
        Ok(())
    }

    /// Sync messages for a specific Gmail label.
    async fn sync_label(
        &self,
        label_id: &str,
        limit: u32,
        folders_by_remote: &HashMap<String, String>,
        thread_mappings: &mut HashMap<String, String>,
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
        let msg_refs = collect_paginated_gmail_refs(label_id, limit, |label_id, limit, page_token| {
            let provider = Arc::clone(&self.provider);
            async move {
                provider
                    .list_message_ids(&label_id, limit, page_token.as_deref())
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
            .get_existing_message_map_by_remote_ids(&self.base.account_id, &remote_ids)
            .unwrap_or_default();

        let mut outcome = GmailLabelSyncOutcome::default();

        // Separate already-existing messages (just add folder) from new ones to fetch
        let mut to_fetch = Vec::new();
        for msg_ref in msg_refs {
            if let Some(local_id) = existing.get(&msg_ref.id) {
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
                    .store_fetched_message(fetched, &folder_id, thread_mappings, folders_by_remote)
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

    /// Poll for new messages using the Gmail History API (delta sync).
    async fn poll_changes(&self) -> Result<()> {
        let cursor = self
            .base
            .store
            .get_sync_cursor(&self.base.account_id)
            .ok()
            .flatten();
        let history_id = match cursor {
            Some(id) if !id.is_empty() => id,
            _ => {
                debug!("No history cursor, doing full re-sync");
                return self.initial_sync().await;
            }
        };

        let url = format!(
            "https://www.googleapis.com/gmail/v1/users/me/history?startHistoryId={history_id}"
        );
        let resp = self.provider.get(&url).await?;

        #[derive(serde::Deserialize)]
        struct HistoryList {
            history: Option<Vec<HistoryEntry>>,
            #[serde(rename = "historyId")]
            history_id: Option<String>,
        }
        #[derive(serde::Deserialize)]
        struct HistoryEntry {
            #[serde(rename = "messagesAdded")]
            messages_added: Option<Vec<HistoryMsg>>,
            #[serde(rename = "messagesDeleted")]
            messages_deleted: Option<Vec<HistoryMsg>>,
            #[serde(rename = "labelsAdded")]
            labels_added: Option<Vec<HistoryLabelChange>>,
            #[serde(rename = "labelsRemoved")]
            labels_removed: Option<Vec<HistoryLabelChange>>,
        }
        #[derive(serde::Deserialize)]
        struct HistoryMsg {
            message: MsgRef,
        }
        #[derive(serde::Deserialize)]
        struct HistoryLabelChange {
            message: MsgRef,
            #[serde(rename = "labelIds")]
            label_ids: Vec<String>,
        }
        #[derive(serde::Deserialize)]
        struct MsgRef {
            id: String,
        }

        let history: HistoryList = resp
            .json()
            .await
            .map_err(|e| pebble_core::PebbleError::Network(format!("Parse history: {e}")))?;

        let mut new_ids = Vec::new();
        let mut deleted_ids = Vec::new();
        let mut labels_added = Vec::new();
        let mut labels_removed = Vec::new();
        let mut failure_count = 0usize;

        if let Some(entries) = &history.history {
            for entry in entries {
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
                if let Some(ref added) = entry.labels_added {
                    for change in added {
                        labels_added.push((change.message.id.clone(), change.label_ids.clone()));
                    }
                }
                if let Some(ref removed) = entry.labels_removed {
                    for change in removed {
                        labels_removed.push((change.message.id.clone(), change.label_ids.clone()));
                    }
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

        // Handle deletions
        if !deleted_ids.is_empty() {
            for remote_id in &deleted_ids {
                match self
                    .base
                    .store
                    .find_message_id_by_remote(&self.base.account_id, remote_id)
                {
                    Ok(Some(local_id)) => match self.base.store.soft_delete_message(&local_id) {
                        Ok(()) => self.emit_message_refresh(&local_id),
                        Err(e) => {
                            failure_count += 1;
                            warn!(
                                "Failed to soft-delete Gmail history message {}: {}",
                                remote_id, e
                            );
                        }
                    },
                    Ok(None) => {}
                    Err(e) => {
                        failure_count += 1;
                        warn!(
                            "Failed to look up deleted Gmail message {}: {}",
                            remote_id, e
                        );
                    }
                }
            }
            info!("Deleted {} messages via history", deleted_ids.len());
        }

        for (remote_id, label_ids) in labels_removed {
            match self
                .base
                .store
                .find_message_id_by_remote(&self.base.account_id, &remote_id)
            {
                Ok(Some(local_id)) => {
                    for label_id in visible_label_ids(&label_ids) {
                        if let Some(folder_id) = folders_by_remote.get(&label_id) {
                            if let Err(e) = self
                                .base
                                .store
                                .remove_message_from_folder(&local_id, folder_id)
                            {
                                failure_count += 1;
                                warn!(
                                    "Failed to remove Gmail label {} from message {}: {}",
                                    label_id, remote_id, e
                                );
                            }
                        }
                    }
                    self.emit_message_refresh(&local_id);
                }
                Ok(None) => {}
                Err(e) => {
                    failure_count += 1;
                    warn!(
                        "Failed to look up Gmail label removal message {}: {}",
                        remote_id, e
                    );
                }
            }
        }

        for (remote_id, label_ids) in labels_added {
            match self
                .base
                .store
                .find_message_id_by_remote(&self.base.account_id, &remote_id)
            {
                Ok(Some(local_id)) => {
                    for label_id in visible_label_ids(&label_ids) {
                        if let Some(folder_id) = folders_by_remote.get(&label_id) {
                            if let Err(e) =
                                self.base.store.add_message_to_folder(&local_id, folder_id)
                            {
                                failure_count += 1;
                                warn!(
                                    "Failed to add Gmail label {} to message {}: {}",
                                    label_id, remote_id, e
                                );
                            }
                        }
                    }
                    self.emit_message_refresh(&local_id);
                }
                Ok(None) => {}
                Err(e) => {
                    failure_count += 1;
                    warn!(
                        "Failed to look up Gmail label addition message {}: {}",
                        remote_id, e
                    );
                }
            }
        }

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

            // Emit refresh for already-known messages; collect truly new IDs.
            let to_fetch: Vec<String> = new_ids
                .into_iter()
                .filter(|gid| {
                    if let Some(local_id) = existing.get(gid) {
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
                            failure_count += 1;
                            warn!("Failed to fetch history message {}: {}", gmail_id, e);
                        }
                    }
                }

                // Phase 2: collect refs from fetched messages for targeted thread lookup.
                let ref_ids = collect_ref_ids_from_messages(&fetched_messages);
                let mut thread_mappings = self
                    .base
                    .store
                    .get_thread_mappings_for_refs(&self.base.account_id, &ref_ids)
                    .unwrap_or_default();

                // Phase 3: store all fetched messages.
                for fetched in fetched_messages {
                    let gmail_id = fetched.message.remote_id.clone();
                    if let Err(e) = self
                        .store_fetched_message(
                            fetched,
                            &inbox_folder_id,
                            &mut thread_mappings,
                            &folders_by_remote,
                        )
                        .await
                    {
                        failure_count += 1;
                        warn!("Failed to store history message {}: {}", gmail_id, e);
                    }
                }
            }
        }

        // Update cursor
        if let Some(new_hid) = history.history_id {
            if can_advance_gmail_cursor(failure_count) {
                let _ = self
                    .base
                    .store
                    .set_sync_cursor(&self.base.account_id, &new_hid);
            } else {
                warn!(
                    "Gmail history sync had {} failures; keeping previous history cursor",
                    failure_count
                );
            }
        }

        Ok(())
    }

    async fn run_poll_cycle(&self, backoff: &mut SyncBackoff) {
        if backoff.is_circuit_open() {
            warn!(
                "Circuit open for Gmail account {} ({} failures), waiting {:?}",
                self.base.account_id,
                backoff.failure_count(),
                backoff.current_delay()
            );
            return;
        }

        if let Err(e) = self.ensure_valid_token().await {
            warn!("Token refresh failed: {}", e);
            let _ = backoff.record_failure();
            return;
        }
        match self.poll_changes().await {
            Ok(()) => backoff.record_success(),
            Err(e) => {
                warn!("Gmail poll failed for account {}: {}", self.base.account_id, e);
                self.base.emit_error("sync", &format!("Poll failed: {e}"));
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
            return;
        }

        // Initial sync
        if let Err(e) = self.initial_sync().await {
            error!(
                "Gmail initial sync failed for account {}: {}",
                self.base.account_id, e
            );
            self.base
                .emit_error("sync", &format!("Initial sync failed: {e}"));
            // Don't return — still enter poll loop so we can retry
        }

        if config.manual_only() {
            info!("Gmail manual sync completed for account {}", self.base.account_id);
            return;
        }

        let policy = RealtimePollPolicy::from_foreground_interval_secs(config.poll_interval_secs);
        let mut stop_rx = self.stop_rx.clone();
        let mut backoff = SyncBackoff::new();
        let mut trigger_rx = trigger_rx;
        let mut runtime = RealtimeRuntimeState::new(Duration::from_secs(60), Instant::now());

        loop {
            let next_delay = policy.next_delay(runtime.context(backoff.failure_count(), Instant::now()));

            tokio::select! {
                _ = stop_rx.changed() => {
                    if *stop_rx.borrow() {
                        info!("Gmail sync stopped for account {}", self.base.account_id);
                        break;
                    }
                }
                _ = tokio::time::sleep(next_delay) => {
                    self.run_poll_cycle(&mut backoff).await;
                }
                trigger = recv_sync_trigger(&mut trigger_rx) => {
                    match trigger {
                        Some(trigger) => {
                            runtime.record_trigger(trigger, Instant::now());
                            if trigger.should_sync_now() {
                                self.run_poll_cycle(&mut backoff).await;
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
    use pebble_core::{Folder, FolderRole, FolderType};
    use std::collections::VecDeque;

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
}
