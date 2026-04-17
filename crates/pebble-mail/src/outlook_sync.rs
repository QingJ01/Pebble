use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use pebble_core::traits::{FetchQuery, FolderProvider, MailTransport};
use pebble_core::{now_timestamp, PebbleError, Result};
use pebble_store::Store;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};

use crate::backoff::SyncBackoff;
use crate::provider::outlook::OutlookProvider;
use crate::gmail_sync::TokenRefresher;
use crate::sync::{StoredMessage, SyncConfig, SyncError, SyncWorkerBase, persist_message_attachments_async};
use crate::thread::compute_thread_id;

/// A sync worker for Outlook accounts using the Microsoft Graph API.
pub struct OutlookSyncWorker {
    pub(crate) base: SyncWorkerBase,
    provider: Arc<OutlookProvider>,
    token_refresher: Option<Arc<TokenRefresher>>,
    /// Last known token expiry (unix timestamp).
    token_expires_at: StdMutex<Option<i64>>,
}

impl OutlookSyncWorker {
    pub fn new(
        account_id: impl Into<String>,
        provider: Arc<OutlookProvider>,
        store: Arc<Store>,
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

    pub fn with_token_expires_at(self, expires_at: Option<i64>) -> Self {
        *self.token_expires_at.lock().unwrap_or_else(|e| e.into_inner()) = expires_at;
        self
    }

    /// Ensure the access token is still valid; refresh if needed.
    async fn ensure_valid_token(&self) -> Result<()> {
        let now = now_timestamp();
        let needs_refresh = {
            let expires = self.token_expires_at.lock().unwrap_or_else(|e| e.into_inner());
            match *expires {
                Some(exp) => now >= exp - 60,
                None => false,
            }
        };

        if needs_refresh {
            if let Some(ref refresher) = self.token_refresher {
                match refresher().await {
                    Ok((new_token, new_expires_at)) => {
                        self.provider.set_access_token(new_token);
                        let mut expires = self.token_expires_at.lock().unwrap_or_else(|e| e.into_inner());
                        *expires = new_expires_at.or(Some(now + 3600));
                        info!("Outlook OAuth token refreshed for account {}", self.base.account_id);
                    }
                    Err(e) => {
                        warn!("Failed to refresh Outlook OAuth token: {}", e);
                        self.base.emit_error("token_refresh", &format!("Outlook token refresh failed: {e}"));
                        return Err(PebbleError::Auth(format!("Outlook token refresh failed: {e}")));
                    }
                }
            }
        }
        Ok(())
    }

    /// Main sync loop.
    pub async fn run(&self, config: SyncConfig, mut stop_rx: watch::Receiver<bool>) {
        let poll_interval = tokio::time::Duration::from_secs(config.poll_interval_secs);
        let mut backoff = SyncBackoff::new();

        loop {
            if *stop_rx.borrow() {
                break;
            }

            // Check circuit breaker at start of each iteration
            if backoff.is_circuit_open() {
                let delay = backoff.current_delay();
                warn!(
                    "Circuit open for Outlook account {} ({} failures), waiting {:?}",
                    self.base.account_id, backoff.failure_count(), delay
                );
                match tokio::time::timeout(delay, stop_rx.changed()).await {
                    Ok(Ok(())) if *stop_rx.borrow() => break,
                    _ => {}
                }
                continue;
            }

            // Refresh token if needed
            if let Err(e) = self.ensure_valid_token().await {
                warn!("Outlook token validation failed: {}", e);
                self.base.emit_error("auth", &format!("Token validation failed: {e}"));
                let _ = backoff.record_failure();
                if backoff.is_circuit_open() {
                    let delay = backoff.current_delay();
                    let _ = tokio::time::timeout(delay, stop_rx.changed()).await;
                }
                continue;
            }

            // List folders and fetch messages per folder
            let folders = match self.provider.list_folders().await {
                Ok(f) => {
                    backoff.record_success();
                    f
                }
                Err(e) => {
                    warn!("Outlook folder list failed: {e}");
                    self.base.emit_error("sync", &format!("Outlook folder list failed: {e}"));
                    let delay = backoff.record_failure();
                    if backoff.is_circuit_open() {
                        warn!(
                            "Circuit open for Outlook account {} ({} failures), waiting {:?}",
                            self.base.account_id, backoff.failure_count(), delay
                        );
                    }
                    let wait = if backoff.is_circuit_open() { delay } else { poll_interval };
                    let _ = tokio::time::timeout(wait, stop_rx.changed()).await;
                    continue;
                }
            };

            for folder in &folders {
                // Persist folder
                let _ = self.base.store.insert_folder(folder);
            }

            // Re-read folders from DB so we use persisted IDs (upsert may keep old IDs)
            let db_folders = self.base.store.list_folders(&self.base.account_id).unwrap_or_default();

            for folder in &db_folders {
                let query = FetchQuery {
                    folder_id: folder.remote_id.clone(),
                    limit: Some(50),
                };
                match self.provider.fetch_messages(&query).await {
                    Ok(result) => {
                        let remote_ids: Vec<String> =
                            result.messages.iter().map(|m| m.remote_id.clone()).collect();
                        let existing = self
                            .base.store
                            .get_existing_remote_ids(&self.base.account_id, &remote_ids)
                            .unwrap_or_default();

                        // Collect all referenced message-ID headers from this batch so we can
                        // load thread mappings in a single query.
                        let ref_ids: Vec<String> = {
                            let mut refs = std::collections::HashSet::new();
                            for msg in &result.messages {
                                if let Some(irt) = &msg.in_reply_to {
                                    for id in irt.split_whitespace() {
                                        refs.insert(id.trim().to_string());
                                    }
                                }
                                if let Some(r) = &msg.references_header {
                                    for id in r.split_whitespace() {
                                        refs.insert(id.trim().to_string());
                                    }
                                }
                            }
                            refs.into_iter().collect()
                        };

                        // Load thread mappings for this batch. Kept mutable so intra-batch
                        // replies can find their parent within the same fetch.
                        let mut thread_mappings = self
                            .base.store
                            .get_thread_mappings_for_refs(&self.base.account_id, &ref_ids)
                            .unwrap_or_default();

                        for msg in &result.messages {
                            if existing.contains(&msg.remote_id) {
                                continue;
                            }

                            // Compute thread_id before inserting.
                            let mut msg = msg.clone();
                            let thread_id = compute_thread_id(&msg, &thread_mappings);
                            msg.thread_id = Some(thread_id);

                            let folder_ids = vec![folder.id.clone()];
                            if let Err(e) = self.base.store.insert_message(&msg, &folder_ids) {
                                warn!("Failed to store Outlook message: {e}");
                                continue;
                            }

                            // Update in-memory thread mappings so later messages in this batch
                            // can find this message as a thread parent.
                            if let (Some(mid), Some(tid)) = (&msg.message_id_header, &msg.thread_id) {
                                thread_mappings.insert(mid.clone(), tid.clone());
                            }

                            // Fetch + persist attachments for messages that advertise them.
                            if msg.has_attachments {
                                match self.provider.list_message_attachments(&msg.remote_id).await {
                                    Ok(attachments) if !attachments.is_empty() => {
                                        persist_message_attachments_async(
                                            Arc::clone(&self.base.store),
                                            self.base.attachments_dir.clone(),
                                            msg.id.clone(),
                                            attachments,
                                        )
                                        .await;
                                    }
                                    Ok(_) => {}
                                    Err(e) => {
                                        warn!(
                                            "Failed to fetch Outlook attachments for {}: {e}",
                                            msg.remote_id
                                        );
                                    }
                                }
                            }

                            self.base.emit_message(StoredMessage {
                                message: msg.clone(),
                                folder_ids,
                            });
                        }
                    }
                    Err(e) => {
                        warn!("Outlook sync fetch failed for folder {}: {e}", folder.name);
                    }
                }

                if *stop_rx.borrow() {
                    break;
                }
            }

            // Wait for next poll or stop signal
            let _ = tokio::time::timeout(poll_interval, stop_rx.changed()).await;
        }

        info!("Outlook sync task completed for account {}", self.base.account_id);
    }
}
