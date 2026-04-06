use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;

use pebble_core::traits::{FetchQuery, FolderProvider, MailTransport};
use pebble_core::{now_timestamp, Result};
use pebble_store::Store;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};

use crate::provider::outlook::OutlookProvider;
use crate::gmail_sync::TokenRefresher;
use crate::sync::{StoredMessage, SyncConfig, SyncError};

/// A sync worker for Outlook accounts using the Microsoft Graph API.
pub struct OutlookSyncWorker {
    account_id: String,
    provider: Arc<OutlookProvider>,
    store: Arc<Store>,
    #[allow(dead_code)]
    attachments_dir: PathBuf,
    error_tx: Option<mpsc::UnboundedSender<SyncError>>,
    message_tx: Option<mpsc::UnboundedSender<StoredMessage>>,
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
            account_id: account_id.into(),
            provider,
            store,
            attachments_dir: attachments_dir.into(),
            error_tx: None,
            message_tx: None,
            token_refresher: None,
            token_expires_at: StdMutex::new(None),
        }
    }

    pub fn with_error_tx(mut self, tx: mpsc::UnboundedSender<SyncError>) -> Self {
        self.error_tx = Some(tx);
        self
    }

    pub fn with_message_tx(mut self, tx: mpsc::UnboundedSender<StoredMessage>) -> Self {
        self.message_tx = Some(tx);
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

    fn emit_error(&self, error_type: &str, message: &str) {
        if let Some(tx) = &self.error_tx {
            let _ = tx.send(SyncError {
                error_type: error_type.to_string(),
                message: message.to_string(),
                timestamp: now_timestamp() as u64,
            });
        }
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
                    Ok(new_token) => {
                        self.provider.set_access_token(new_token);
                        let mut expires = self.token_expires_at.lock().unwrap_or_else(|e| e.into_inner());
                        *expires = Some(now + 3600);
                        info!("Outlook OAuth token refreshed for account {}", self.account_id);
                    }
                    Err(e) => {
                        warn!("Failed to refresh Outlook OAuth token: {}", e);
                        self.emit_error("token_refresh", &format!("Outlook token refresh failed: {e}"));
                    }
                }
            }
        }
        Ok(())
    }

    /// Main sync loop.
    pub async fn run(&self, config: SyncConfig, mut stop_rx: watch::Receiver<bool>) {
        let poll_interval = tokio::time::Duration::from_secs(config.poll_interval_secs);

        loop {
            if *stop_rx.borrow() {
                break;
            }

            // Refresh token if needed
            if let Err(e) = self.ensure_valid_token().await {
                warn!("Outlook token validation failed: {}", e);
                self.emit_error("auth", &format!("Token validation failed: {e}"));
            }

            // List folders and fetch messages per folder
            let folders = match self.provider.list_folders().await {
                Ok(f) => f,
                Err(e) => {
                    warn!("Outlook folder list failed: {e}");
                    self.emit_error("sync", &format!("Outlook folder list failed: {e}"));
                    vec![]
                }
            };

            for folder in &folders {
                // Persist folder
                let _ = self.store.insert_folder(folder);

                let query = FetchQuery {
                    folder_id: folder.remote_id.clone(),
                    limit: Some(50),
                };
                match self.provider.fetch_messages(&query).await {
                    Ok(result) => {
                        let remote_ids: Vec<String> =
                            result.messages.iter().map(|m| m.remote_id.clone()).collect();
                        let existing = self
                            .store
                            .get_existing_remote_ids(&self.account_id, &remote_ids)
                            .unwrap_or_default();
                        for msg in &result.messages {
                            if existing.contains(&msg.remote_id) {
                                continue;
                            }
                            let folder_ids = vec![folder.id.clone()];
                            if let Err(e) = self.store.insert_message(msg, &folder_ids) {
                                warn!("Failed to store Outlook message: {e}");
                            } else if let Some(tx) = &self.message_tx {
                                let _ = tx.send(StoredMessage {
                                    message: msg.clone(),
                                    folder_ids,
                                });
                            }
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

        info!("Outlook sync task completed for account {}", self.account_id);
    }
}
