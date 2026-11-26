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

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, ts_rs::TS)]
pub struct SearchHit {
    pub message_id: String,
    pub score: f32,
    pub snippet: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub subject: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub from_address: Option<String>,
    /// Sender display name, when the message has one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub from_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub date: Option<i64>,
    /// Owning account, filled in from the store after the index search.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    #[ts(optional)]
    pub account_id: Option<String>,
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

/// Gmail-style label edit. Providers that model folders as real mailboxes
/// ignore it; Gmail uses it verbatim when a caller (or a queued op) supplies it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct LabelDelta {
    pub add_labels: Vec<String>,
    pub remove_labels: Vec<String>,
}

/// The remote identity of a cached message.
#[derive(Debug, Clone, Copy)]
pub struct RemoteMessageRef<'a> {
    /// Provider message id: Gmail/Graph id, or the IMAP UID in `folder`.
    pub remote_id: &'a str,
    /// RFC 5322 Message-ID, used to re-find a message whose id changed.
    pub message_id_header: Option<&'a str>,
}

/// A folder as the provider knows it.
#[derive(Debug, Clone, Copy)]
pub struct RemoteFolderRef<'a> {
    pub remote_id: &'a str,
    pub role: Option<&'a FolderRole>,
}

impl<'a> RemoteFolderRef<'a> {
    pub fn of(folder: &'a Folder) -> Self {
        Self {
            remote_id: &folder.remote_id,
            role: folder.role.as_ref(),
        }
    }

    pub fn has_role(&self, role: FolderRole) -> bool {
        self.role == Some(&role)
    }
}

/// How a successful remote mutation changed the message's remote identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RemoteIdUpdate {
    /// The remote id is unchanged (Gmail, IMAP STORE, ...).
    Unchanged,
    /// A new account-global id (Graph assigns a new id on every move).
    Global(String),
    /// A new id that is only meaningful inside the destination folder (IMAP UID).
    InFolder {
        folder_remote_id: String,
        remote_id: String,
    },
}

#[derive(Debug, Clone, Copy, Default)]
pub struct FlagUpdate<'a> {
    pub is_read: Option<bool>,
    pub is_starred: Option<bool>,
    /// Mailbox the message lives in, for folder-scoped ids (IMAP).
    pub folder_remote_id: Option<&'a str>,
    /// Exact label edit recorded by an earlier attempt (Gmail).
    pub labels: Option<&'a LabelDelta>,
}

#[derive(Debug, Clone, Copy)]
pub struct MoveRequest<'a> {
    pub source: Option<RemoteFolderRef<'a>>,
    pub target: RemoteFolderRef<'a>,
    /// Exact label edit recorded by an earlier attempt (Gmail).
    pub labels: Option<&'a LabelDelta>,
    /// Check whether an earlier, unacknowledged attempt already moved the
    /// message before moving it again (idempotent Move To replay).
    pub recover_previous_attempt: bool,
}

#[derive(Debug, Clone, Copy)]
pub struct CreateFolderRequest<'a> {
    pub account_id: &'a str,
    pub name: &'a str,
    /// Local parent folder, if any.
    pub parent: Option<&'a Folder>,
    /// Remote-only parent namespace (IMAP nonselectable containers).
    pub parent_remote_id: Option<&'a str>,
}

#[derive(Debug, Clone, Copy)]
pub struct RenameFolderRequest<'a> {
    pub account_id: &'a str,
    /// The account's cached folders, used to rewrite renamed hierarchies.
    pub local_folders: &'a [Folder],
    pub folder: &'a Folder,
    pub name: &'a str,
}

/// Server-side mutations of an already-connected mail account.
///
/// Each method expresses one user-level intent; provider quirks (Gmail
/// labels, Graph id changes, IMAP folder-scoped UIDs and MOVE tracking) live
/// in the implementations. Methods a provider cannot express return
/// [`crate::PebbleError::UnsupportedProvider`].
#[async_trait]
#[allow(
    clippy::double_must_use,
    reason = "async_trait adds must_use to an already must-use boxed future"
)]
pub trait RemoteMailOps: Send + Sync {
    async fn apply_flags(
        &self,
        message: RemoteMessageRef<'_>,
        update: FlagUpdate<'_>,
    ) -> Result<()>;

    /// Move between two folders (or swap labels on Gmail).
    async fn move_to_folder(
        &self,
        message: RemoteMessageRef<'_>,
        request: MoveRequest<'_>,
    ) -> Result<RemoteIdUpdate>;

    /// Remove from the inbox. `archive` is the account's archive folder, if
    /// it has one; mailbox-based providers require it.
    async fn archive(
        &self,
        message: RemoteMessageRef<'_>,
        source: Option<RemoteFolderRef<'_>>,
        archive: Option<RemoteFolderRef<'_>>,
    ) -> Result<RemoteIdUpdate>;

    /// Bring a message back to the inbox (from Archive, Trash, ...).
    async fn restore_to_inbox(
        &self,
        message: RemoteMessageRef<'_>,
        source: Option<RemoteFolderRef<'_>>,
        inbox: RemoteFolderRef<'_>,
    ) -> Result<RemoteIdUpdate>;

    /// Move to the provider's trash. Without a distinct trash folder,
    /// mailbox-based providers delete the message instead.
    async fn trash(
        &self,
        message: RemoteMessageRef<'_>,
        source: Option<RemoteFolderRef<'_>>,
        trash: Option<RemoteFolderRef<'_>>,
    ) -> Result<RemoteIdUpdate>;

    async fn delete_permanently(
        &self,
        message: RemoteMessageRef<'_>,
        source: Option<RemoteFolderRef<'_>>,
    ) -> Result<()>;

    /// Create a folder (or label) on the server and return it as listed there.
    async fn create_remote_folder(&self, request: CreateFolderRequest<'_>) -> Result<Folder>;

    /// Rename on the server; returns every cached folder whose name or
    /// remote id changed (a renamed IMAP hierarchy includes its children).
    async fn rename_remote_folder(&self, request: RenameFolderRequest<'_>) -> Result<Vec<Folder>>;

    /// Release the connection (IMAP LOGOUT); a no-op for HTTP providers.
    async fn close(&self) {}
}

// Compile-time assertion: the provider traits must be object-safe.
#[cfg(test)]
mod tests {
    use super::*;
    fn _assert_remote_ops_object_safe(_: &dyn RemoteMailOps) {}
}
