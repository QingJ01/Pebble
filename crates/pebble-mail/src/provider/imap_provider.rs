use async_trait::async_trait;
use pebble_core::{Folder, PebbleError, ProviderCapabilities, Result};
use pebble_core::traits::*;

use crate::imap::ImapProvider;

/// Wraps ImapProvider to implement the MailProvider trait hierarchy.
pub struct ImapMailProvider {
    inner: ImapProvider,
    account_id: String,
}

impl ImapMailProvider {
    pub fn new(config: crate::imap::ImapConfig) -> Self {
        Self {
            inner: ImapProvider::new(config),
            account_id: String::new(),
        }
    }

    /// Access the underlying ImapProvider for IMAP-specific operations
    /// (e.g., fetch_messages_raw, fetch_flags, fetch_all_uids used by sync).
    pub fn inner(&self) -> &ImapProvider {
        &self.inner
    }

    /// Create a second provider with the same config, for use as a dedicated IDLE connection.
    pub fn clone_for_idle(&self) -> Self {
        Self {
            inner: ImapProvider::new(self.inner.config()),
            account_id: self.account_id.clone(),
        }
    }

    /// Set the account ID used by FolderProvider::list_folders.
    pub fn set_account_id(&mut self, id: String) {
        self.account_id = id;
    }

    /// Connect to the IMAP server.
    pub async fn connect(&self) -> Result<()> {
        self.inner.connect().await
    }

    /// Disconnect from the IMAP server.
    pub async fn disconnect(&self) -> Result<()> {
        self.inner.disconnect().await
    }
}

#[async_trait]
impl MailTransport for ImapMailProvider {
    async fn authenticate(&mut self, _credentials: &AuthCredentials) -> Result<()> {
        // For IMAP, authentication happens during connect()
        self.inner.connect().await
    }

    async fn fetch_messages(&self, query: &FetchQuery) -> Result<FetchResult> {
        // Delegate to ImapProvider's fetch_messages_raw and parse.
        // This is a simplified version - the actual sync uses sync_folder directly.
        let _limit = query.limit.unwrap_or(50);
        let _raw = self
            .inner
            .fetch_messages_raw(&query.folder_id, None, _limit)
            .await?;
        // For now return empty result - actual fetching happens through sync_folder
        Ok(FetchResult {
            messages: vec![],
            cursor: SyncCursor {
                value: String::new(),
            },
        })
    }

    async fn send_message(&self, _message: &OutgoingMessage) -> Result<()> {
        // SMTP sending is handled by the smtp module, not IMAP
        Err(PebbleError::Internal("Use SMTP for sending".to_string()))
    }

    async fn sync_changes(&self, _since: &SyncCursor) -> Result<ChangeSet> {
        // IMAP sync uses the poll/reconcile pattern, not ChangeSet
        Ok(ChangeSet {
            new_messages: vec![],
            flag_changes: vec![],
            moved: vec![],
            deleted: vec![],
            cursor: SyncCursor {
                value: String::new(),
            },
        })
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            has_labels: false,
            has_folders: true,
            has_categories: false,
            has_push: false,
            has_threads: true,
        }
    }
}

#[async_trait]
impl FolderProvider for ImapMailProvider {
    async fn list_folders(&self) -> Result<Vec<Folder>> {
        self.inner.list_folders(&self.account_id).await
    }

    async fn move_message(&self, _remote_id: &str, _to_folder_id: &str) -> Result<()> {
        // Note: actual IMAP move is done via ImapProvider::move_message directly,
        // because FolderProvider::move_message uses remote IDs, not UIDs.
        // The Tauri command layer handles UID lookup and calls inner() directly.
        Err(PebbleError::Internal(
            "Use ImapProvider::move_message with UIDs instead".to_string(),
        ))
    }
}

impl pebble_core::traits::MailProvider for ImapMailProvider {}
