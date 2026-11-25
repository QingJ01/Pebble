use async_trait::async_trait;
use pebble_core::traits::*;
use pebble_core::{Folder, FolderRole, FolderType, PebbleError, ProviderCapabilities, Result};

use crate::pop3::{Pop3Config, Pop3Provider};

pub struct Pop3MailProvider {
    inner: Pop3Provider,
    account_id: String,
}

impl Pop3MailProvider {
    pub fn new(config: Pop3Config) -> Self {
        Self {
            inner: Pop3Provider::new(config),
            account_id: String::new(),
        }
    }

    pub fn inner(&self) -> &Pop3Provider {
        &self.inner
    }

    pub fn set_account_id(&mut self, id: String) {
        self.account_id = id;
    }
}

#[async_trait]
impl MailTransport for Pop3MailProvider {
    async fn authenticate(&mut self, _credentials: &AuthCredentials) -> Result<()> {
        Pop3Provider::test_connection(&self.inner.config()).await?;
        Ok(())
    }

    async fn fetch_messages(&self, _query: &FetchQuery) -> Result<FetchResult> {
        Ok(FetchResult {
            messages: vec![],
            cursor: SyncCursor {
                value: String::new(),
            },
        })
    }

    async fn send_message(&self, _message: &OutgoingMessage) -> Result<()> {
        Err(PebbleError::Internal("Use SMTP for sending".to_string()))
    }

    async fn sync_changes(&self, _since: &SyncCursor) -> Result<ChangeSet> {
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
            has_folders: false,
            has_categories: false,
            has_push: false,
            has_threads: true,
        }
    }
}

#[async_trait]
impl FolderProvider for Pop3MailProvider {
    async fn list_folders(&self) -> Result<Vec<Folder>> {
        Ok(vec![Folder {
            id: "pop3-inbox".to_string(),
            account_id: self.account_id.clone(),
            remote_id: "INBOX".to_string(),
            name: "Inbox".to_string(),
            folder_type: FolderType::Folder,
            role: Some(FolderRole::Inbox),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 0,
        }])
    }

    async fn move_message(&self, _remote_id: &str, _to_folder_id: &str) -> Result<String> {
        Err(PebbleError::UnsupportedProvider(
            "POP3 does not support remote message moves".to_string(),
        ))
    }
}
