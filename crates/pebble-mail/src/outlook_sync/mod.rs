use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use pebble_core::traits::FolderProvider;
use pebble_core::{now_timestamp, Folder, Message, PebbleError, Result};
use pebble_store::Store;
use tokio::sync::{mpsc, watch};
use tracing::{debug, info, warn};

use crate::backoff::SyncBackoff;
use crate::gmail_sync::TokenRefresher;
use crate::provider::outlook::{
    should_hide_outlook_folder, validate_graph_continuation_url, OutlookDeltaPage, OutlookProvider,
    MAX_GRAPH_CONTINUATION_PAGES,
};
use crate::realtime_policy::{RealtimePollPolicy, RealtimeRuntimeState, SyncTrigger};
use crate::sync::{
    blocking_store, cleanup_staged_attachment_files, message_ids_with_pending_local_changes,
    recv_sync_trigger, stage_message_attachments_async,
    store_new_message_with_attachments_atomically, StoredMessage, SyncConfig, SyncError,
    SyncWorkerBase, ThreadRefresh,
};
use crate::thread::{compute_thread_id, provider_thread_id, OUTLOOK_THREAD_NAMESPACE};

mod delta;
mod persist;
mod run;
#[cfg(test)]
mod tests;

use delta::*;

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
                runtime_status_tx: None,
                progress_tx: None,
                thread_refresh_tx: None,
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

    pub fn with_token_expires_at(self, expires_at: Option<i64>) -> Self {
        *self
            .token_expires_at
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = expires_at;
        self
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
                Some(exp) => now >= exp - 60,
                None => false,
            }
        };

        if needs_refresh {
            if let Some(ref refresher) = self.token_refresher {
                match refresher().await {
                    Ok((new_token, new_expires_at)) => {
                        self.provider.set_access_token(new_token);
                        let mut expires = self
                            .token_expires_at
                            .lock()
                            .unwrap_or_else(|e| e.into_inner());
                        *expires = new_expires_at.or(Some(now + 3600));
                        info!(
                            "Outlook OAuth token refreshed for account {}",
                            self.base.account_id
                        );
                    }
                    Err(e) => {
                        warn!("Failed to refresh Outlook OAuth token: {}", e);
                        self.base.emit_error(
                            "token_refresh",
                            &format!("Outlook token refresh failed: {e}"),
                        );
                        return Err(PebbleError::Auth(format!(
                            "Outlook token refresh failed: {e}"
                        )));
                    }
                }
            }
        }
        Ok(())
    }

    async fn persist_folder_messages(
        &self,
        folder: &Folder,
        messages: Vec<Message>,
        notify_new: bool,
    ) -> u32 {
        let provider = Arc::clone(&self.provider);
        self.persist_folder_messages_with_attachment_fetch(
            folder,
            messages,
            notify_new,
            move |remote_id| {
                let provider = Arc::clone(&provider);
                async move { provider.list_message_attachments(&remote_id).await }
            },
        )
        .await
    }
}
