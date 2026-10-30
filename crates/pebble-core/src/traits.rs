use async_trait::async_trait;

use crate::error::Result;
use crate::types::*;

pub struct FetchQuery {
    pub folder_id: String,
    pub limit: Option<u32>,
}

pub struct FetchResult {
    pub messages: Vec<Message>,
    pub cursor: SyncCursor,
}

#[derive(Debug, Clone)]
pub struct SyncCursor {
    pub value: String,
}

pub struct ChangeSet {
    pub new_messages: Vec<Message>,
    pub flag_changes: Vec<FlagChange>,
    pub moved: Vec<MoveChange>,
    pub deleted: Vec<String>,
    pub cursor: SyncCursor,
}

pub struct FlagChange {
    pub remote_id: String,
    pub is_read: Option<bool>,
    pub is_starred: Option<bool>,
}

pub struct MoveChange {
    pub remote_id: String,
    pub from_folder: String,
    pub to_folder: String,
}

pub struct AuthCredentials {
    pub provider: ProviderType,
    pub data: serde_json::Value,
}

pub struct OutgoingMessage {
    pub from: EmailAddress,
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    pub bcc: Vec<EmailAddress>,
    pub subject: String,
    pub body_text: String,
    pub body_html: Option<String>,
    pub in_reply_to: Option<String>,
    pub attachment_paths: Vec<String>,
}

pub struct StructuredQuery {
    pub text: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub subject: Option<String>,
    pub has_attachment: Option<bool>,
    pub folder_id: Option<String>,
    pub date_from: Option<i64>,
    pub date_to: Option<i64>,
}

pub enum SearchQuery {
    Structured(StructuredQuery),
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SearchHit {
    pub message_id: String,
    pub score: f32,
    pub snippet: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from_address: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub date: Option<i64>,
}

#[async_trait]
#[allow(
    clippy::double_must_use,
    reason = "async_trait adds must_use to an already must-use boxed future"
)]
pub trait MailTransport: Send + Sync {
    async fn authenticate(&mut self, credentials: &AuthCredentials) -> Result<()>;
    async fn fetch_messages(&self, query: &FetchQuery) -> Result<FetchResult>;
    async fn send_message(&self, message: &OutgoingMessage) -> Result<()>;
    async fn sync_changes(&self, since: &SyncCursor) -> Result<ChangeSet>;
    fn capabilities(&self) -> ProviderCapabilities;
}

#[async_trait]
#[allow(
    clippy::double_must_use,
    reason = "async_trait adds must_use to an already must-use boxed future"
)]
pub trait FolderProvider: Send + Sync {
    async fn list_folders(&self) -> Result<Vec<Folder>>;
    async fn move_message(&self, remote_id: &str, to_folder_id: &str) -> Result<String>;
}

#[async_trait]
#[allow(
    clippy::double_must_use,
    reason = "async_trait adds must_use to an already must-use boxed future"
)]
pub trait LabelProvider: Send + Sync {
    async fn list_labels(&self) -> Result<Vec<Folder>>;
    async fn modify_labels(&self, remote_id: &str, add: &[String], remove: &[String])
        -> Result<()>;
}

#[async_trait]
#[allow(
    clippy::double_must_use,
    reason = "async_trait adds must_use to an already must-use boxed future"
)]
pub trait SearchEngine: Send + Sync {
    async fn index_message(&self, message: &Message) -> Result<()>;
    async fn search(&self, query: &SearchQuery) -> Result<Vec<SearchHit>>;
    async fn rebuild_index(&self) -> Result<()>;
}

#[async_trait]
#[allow(
    clippy::double_must_use,
    reason = "async_trait adds must_use to an already must-use boxed future"
)]
pub trait CategoryProvider: Send + Sync {
    async fn list_categories(&self) -> Result<Vec<Category>>;
    async fn set_categories(&self, message_id: &str, categories: &[String]) -> Result<()>;
}

#[async_trait]
#[allow(
    clippy::double_must_use,
    reason = "async_trait adds must_use to an already must-use boxed future"
)]
pub trait DraftProvider: Send + Sync {
    async fn save_draft(&self, draft: &DraftMessage) -> Result<String>;
    async fn update_draft(&self, draft_id: &str, draft: &DraftMessage) -> Result<()>;
    async fn delete_draft(&self, draft_id: &str) -> Result<()>;
    async fn list_drafts(&self) -> Result<Vec<DraftMessage>>;
}

pub trait MailProvider: MailTransport + FolderProvider {
    fn as_label_provider(&self) -> Option<&dyn LabelProvider> {
        None
    }
    fn as_category_provider(&self) -> Option<&dyn CategoryProvider> {
        None
    }
    fn as_draft_provider(&self) -> Option<&dyn DraftProvider> {
        None
    }
}

// Compile-time assertion: MailProvider must be object-safe.
#[cfg(test)]
mod tests {
    use super::*;
    fn _assert_object_safe(_: &dyn MailProvider) {}
}
