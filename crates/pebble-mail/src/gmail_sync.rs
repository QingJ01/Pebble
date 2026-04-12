use std::future::Future;
use std::collections::HashMap;
use std::path::PathBuf;
use std::pin::Pin;
use std::sync::Arc;

use pebble_core::traits::FolderProvider;
use pebble_core::{new_id, now_timestamp, Folder, FolderRole, Result};
use pebble_store::Store;
use std::sync::Mutex as StdMutex;
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, warn};

use crate::backoff::SyncBackoff;
use crate::provider::gmail::{GmailFetchedMessage, GmailProvider, visible_label_ids};
use crate::sync::{StoredMessage, SyncConfig, SyncError, SyncWorkerBase, persist_message_attachments};
use crate::thread::compute_thread_id;

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

/// Callback that refreshes the OAuth token and returns the new access token.
pub type TokenRefresher =
    Box<dyn Fn() -> Pin<Box<dyn Future<Output = Result<String>> + Send>> + Send + Sync>;

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

    pub fn with_token_refresher(mut self, refresher: TokenRefresher, expires_at: Option<i64>) -> Self {
        self.token_refresher = Some(Arc::new(refresher));
        *self.token_expires_at.lock().unwrap_or_else(|e| e.into_inner()) = expires_at;
        self
    }

    fn emit_message_refresh(&self, message_id: &str) {
        let Some(tx) = &self.base.message_tx else {
            return;
        };

        let Ok(Some(message)) = self.base.store.get_message(message_id) else {
            return;
        };
        let folder_ids = self.base.store.get_message_folder_ids(message_id).unwrap_or_default();
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

        let folder_ids = resolve_folder_ids(folders_by_remote, &visible_label_ids, fallback_folder_id);

        self.base.store.insert_message(&message, &folder_ids)?;
        persist_message_attachments(
            &self.base.store,
            &self.base.attachments_dir,
            &message.id,
            attachments,
        );

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
            let expires = self.token_expires_at.lock().unwrap_or_else(|e| e.into_inner());
            match *expires {
                Some(exp) => now >= exp - 300, // 5 minute buffer
                None => false,                 // No expiry info — assume valid
            }
        };

        if needs_refresh {
            if let Some(ref refresher) = self.token_refresher {
                debug!("Refreshing Gmail OAuth token for account {}", self.base.account_id);
                match refresher().await {
                    Ok(new_token) => {
                        self.provider.set_access_token(new_token);
                        // Assume the new token is valid for 1 hour
                        let mut expires = self.token_expires_at.lock().unwrap_or_else(|e| e.into_inner());
                        *expires = Some(now + 3600);
                        info!("Gmail OAuth token refreshed for account {}", self.base.account_id);
                    }
                    Err(e) => {
                        warn!("Failed to refresh OAuth token: {}", e);
                        self.base.emit_error("auth", &format!("Token refresh failed: {e}"));
                    }
                }
            }
        }
        Ok(())
    }

    /// Perform initial sync: list folders, fetch messages for each label.
    async fn initial_sync(&self) -> Result<()> {
        info!("Starting Gmail initial sync for account {}", self.base.account_id);

        // Sync folders (labels) — hidden labels are already filtered by the provider
        let folders = self.provider.list_folders().await?;
        for mut folder in folders {
            folder.account_id = self.base.account_id.clone();
            let _ = self.base.store.insert_folder(&folder);
        }

        // Clean up any previously-synced hidden Gmail labels from the local store
        let hidden = [
            "CHAT", "IMPORTANT", "STARRED", "UNREAD",
            "CATEGORY_FORUMS", "CATEGORY_UPDATES", "CATEGORY_PERSONAL",
            "CATEGORY_PROMOTIONS", "CATEGORY_SOCIAL",
        ];
        for label_id in &hidden {
            let _ = self.base.store.delete_folder_by_remote_id(&self.base.account_id, label_id);
        }

        // Ensure an Archive folder exists locally
        let local_folders = self.base.store.list_folders(&self.base.account_id)?;
        let has_archive = local_folders.iter().any(|f| f.role == Some(pebble_core::FolderRole::Archive));
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
            .base.store
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

        for label_id in &labels_to_sync {
            if let Err(e) = self.sync_label(label_id, limit, &folders_by_remote).await {
                warn!("Gmail sync label {} failed: {}", label_id, e);
            }
        }

        // Store the historyId as sync cursor for future delta syncs
        if !profile_history_id.is_empty() {
            let _ = self.base.store.set_sync_cursor(&self.base.account_id, &profile_history_id);
        }

        info!("Gmail initial sync completed for account {}", self.base.account_id);
        Ok(())
    }

    /// Sync messages for a specific Gmail label.
    async fn sync_label(&self, label_id: &str, limit: u32, folders_by_remote: &HashMap<String, String>) -> Result<u32> {
        let folder_id = match folders_by_remote.get(label_id) {
            Some(id) => id.clone(),
            None => {
                debug!("No local folder found for label {}, skipping", label_id);
                return Ok(0);
            }
        };

        // List message IDs from Gmail
        let (msg_refs, _next_page) = self.provider.list_message_ids(label_id, limit, None).await?;
        if msg_refs.is_empty() {
            return Ok(0);
        }

        let remote_ids: Vec<String> = msg_refs.iter().map(|r| r.id.clone()).collect();
        let existing = self
            .base.store
            .get_existing_message_map_by_remote_ids(&self.base.account_id, &remote_ids)
            .unwrap_or_default();

        // Load thread mappings for thread ID computation
        let mut thread_mappings = self
            .base.store
            .get_thread_mappings(&self.base.account_id)
            .unwrap_or_default();

        let mut stored_count = 0u32;

        for msg_ref in msg_refs {
            if let Some(local_id) = existing.get(&msg_ref.id) {
                if let Err(e) = self.base.store.add_message_to_folder(local_id, &folder_id) {
                    warn!(
                        "Failed to add Gmail label {} to existing message {}: {}",
                        label_id, msg_ref.id, e
                    );
                } else {
                    self.emit_message_refresh(local_id);
                }
                continue;
            }

            match self
                .provider
                .fetch_sync_message(&msg_ref.id, &self.base.account_id)
                .await
            {
                Ok(fetched) => match self
                    .store_fetched_message(fetched, &folder_id, &mut thread_mappings, folders_by_remote)
                    .await
                {
                    Ok(true) => stored_count += 1,
                    Ok(false) => {}
                    Err(e) => {
                        error!("Failed to store Gmail message {}: {}", msg_ref.id, e);
                    }
                },
                Err(e) => {
                    warn!("Failed to fetch Gmail message {}: {}", msg_ref.id, e);
                }
            }
        }

        if stored_count > 0 {
            info!("Stored {} messages for label {}", stored_count, label_id);
        }
        Ok(stored_count)
    }

    /// Poll for new messages using the Gmail History API (delta sync).
    async fn poll_changes(&self) -> Result<()> {
        let cursor = self
            .base.store
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
            .base.store
            .list_folders(&self.base.account_id)?
            .into_iter()
            .map(|folder| (folder.remote_id, folder.id))
            .collect();

        // Handle deletions
        if !deleted_ids.is_empty() {
            for remote_id in &deleted_ids {
                if let Ok(Some(local_id)) =
                    self.base.store.find_message_id_by_remote(&self.base.account_id, remote_id)
                {
                    let _ = self.base.store.soft_delete_message(&local_id);
                    self.emit_message_refresh(&local_id);
                }
            }
            info!("Deleted {} messages via history", deleted_ids.len());
        }

        for (remote_id, label_ids) in labels_removed {
            if let Ok(Some(local_id)) = self.base.store.find_message_id_by_remote(&self.base.account_id, &remote_id) {
                for label_id in visible_label_ids(&label_ids) {
                    if let Some(folder_id) = folders_by_remote.get(&label_id) {
                        let _ = self.base.store.remove_message_from_folder(&local_id, folder_id);
                    }
                }
                self.emit_message_refresh(&local_id);
            }
        }

        for (remote_id, label_ids) in labels_added {
            if let Ok(Some(local_id)) = self.base.store.find_message_id_by_remote(&self.base.account_id, &remote_id) {
                for label_id in visible_label_ids(&label_ids) {
                    if let Some(folder_id) = folders_by_remote.get(&label_id) {
                        let _ = self.base.store.add_message_to_folder(&local_id, folder_id);
                    }
                }
                self.emit_message_refresh(&local_id);
            }
        }

        // Fetch new messages
        if !new_ids.is_empty() {
            let existing = self
                .base.store
                .get_existing_message_map_by_remote_ids(&self.base.account_id, &new_ids)
                .unwrap_or_default();

            let inbox_folder_id = folders_by_remote
                .get("INBOX")
                .cloned()
                .unwrap_or_default();
            let mut thread_mappings = self
                .base.store
                .get_thread_mappings(&self.base.account_id)
                .unwrap_or_default();

            for gmail_id in new_ids {
                if let Some(local_id) = existing.get(&gmail_id) {
                    self.emit_message_refresh(local_id);
                    continue;
                }

                match self
                    .provider
                    .fetch_sync_message(&gmail_id, &self.base.account_id)
                    .await
                {
                    Ok(fetched) => {
                        if let Err(e) = self
                            .store_fetched_message(fetched, &inbox_folder_id, &mut thread_mappings, &folders_by_remote)
                            .await
                        {
                            warn!("Failed to store history message {}: {}", gmail_id, e);
                        }
                    }
                    Err(e) => warn!("Failed to fetch history message {}: {}", gmail_id, e),
                }
            }
        }

        // Update cursor
        if let Some(new_hid) = history.history_id {
            let _ = self.base.store.set_sync_cursor(&self.base.account_id, &new_hid);
        }

        Ok(())
    }

    /// Main sync loop.
    pub async fn run(&self, config: SyncConfig) {
        let poll_interval = tokio::time::Duration::from_secs(config.poll_interval_secs);

        // Ensure token is valid before starting
        if let Err(e) = self.ensure_valid_token().await {
            error!("Token validation failed for account {}: {}", self.base.account_id, e);
            self.base.emit_error("auth", &format!("Token validation failed: {e}"));
            return;
        }

        // Initial sync
        if let Err(e) = self.initial_sync().await {
            error!("Gmail initial sync failed for account {}: {}", self.base.account_id, e);
            self.base.emit_error("sync", &format!("Initial sync failed: {e}"));
            // Don't return — still enter poll loop so we can retry
        }

        let mut poll_ticker = tokio::time::interval(poll_interval);
        let mut stop_rx = self.stop_rx.clone();
        let mut backoff = SyncBackoff::new();

        loop {
            tokio::select! {
                _ = stop_rx.changed() => {
                    if *stop_rx.borrow() {
                        info!("Gmail sync stopped for account {}", self.base.account_id);
                        break;
                    }
                }
                _ = poll_ticker.tick() => {
                    if backoff.is_circuit_open() {
                        let delay = backoff.current_delay();
                        warn!(
                            "Circuit open for Gmail account {} ({} failures), waiting {:?}",
                            self.base.account_id, backoff.failure_count(), delay
                        );
                        match tokio::time::timeout(delay, stop_rx.changed()).await {
                            Ok(Ok(())) if *stop_rx.borrow() => break,
                            _ => {}
                        }
                    }

                    if let Err(e) = self.ensure_valid_token().await {
                        warn!("Token refresh failed: {}", e);
                        let _ = backoff.record_failure();
                        continue;
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
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_core::{Folder, FolderRole, FolderType};

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
}
