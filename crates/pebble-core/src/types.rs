use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Account {
    /// A local label, never used in outbound mail headers.
    #[serde(default)]
    pub account_label: Option<String>,
    /// Last verified provider-managed name (Outlook).
    #[serde(default)]
    pub provider_display_name: Option<String>,
    pub id: String,
    pub email: String,
    pub display_name: String,
    pub color: Option<String>,
    pub provider: ProviderType,
    pub created_at: i64,
    pub updated_at: i64,
}

impl Account {
    /// Local account labels must never participate in the outgoing identity.
    pub fn sender_identity(&self) -> EmailAddress {
        let name = if self.provider == ProviderType::Outlook {
            self.provider_display_name.as_deref()
        } else {
            Some(self.display_name.as_str())
        };
        EmailAddress {
            name: name
                .map(str::trim)
                .filter(|name| !name.is_empty())
                .map(str::to_owned),
            address: self.email.trim().to_owned(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct OAuthMailboxIdentity {
    pub subject: String,
    pub email: String,
    pub display_name: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ProviderType {
    Imap,
    Pop3,
    Gmail,
    Outlook,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HttpProxyConfig {
    pub host: String,
    pub port: u16,
}

impl HttpProxyConfig {
    pub fn validate(&self) -> Result<(), String> {
        if self.host.trim().is_empty() {
            return Err("Proxy host is required".to_string());
        }
        if self.port == 0 {
            return Err("Proxy port must be between 1 and 65535".to_string());
        }
        Ok(())
    }

    pub fn socks5h_uri(&self) -> Result<String, String> {
        self.validate()?;
        Ok(format!("socks5h://{}:{}", self.host.trim(), self.port))
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Folder {
    pub id: String,
    pub account_id: String,
    pub remote_id: String,
    pub name: String,
    pub folder_type: FolderType,
    pub role: Option<FolderRole>,
    pub parent_id: Option<String>,
    pub color: Option<String>,
    pub is_system: bool,
    pub sort_order: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FolderType {
    Folder,
    Label,
    Category,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum FolderRole {
    Inbox,
    Sent,
    Drafts,
    Trash,
    Archive,
    Spam,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub account_id: String,
    pub remote_id: String,
    pub message_id_header: Option<String>,
    pub in_reply_to: Option<String>,
    pub references_header: Option<String>,
    pub thread_id: Option<String>,
    pub subject: String,
    pub snippet: String,
    pub from_address: String,
    pub from_name: String,
    pub to_list: Vec<EmailAddress>,
    pub cc_list: Vec<EmailAddress>,
    pub bcc_list: Vec<EmailAddress>,
    pub body_text: String,
    pub body_html_raw: String,
    pub has_attachments: bool,
    pub is_read: bool,
    pub is_starred: bool,
    pub is_draft: bool,
    pub date: i64,
    pub remote_version: Option<String>,
    pub is_deleted: bool,
    pub deleted_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

/// Lightweight message data for list views (excludes body_text and body_html_raw).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MessageSummary {
    pub id: String,
    pub account_id: String,
    pub remote_id: String,
    pub message_id_header: Option<String>,
    pub in_reply_to: Option<String>,
    pub references_header: Option<String>,
    pub thread_id: Option<String>,
    pub subject: String,
    pub snippet: String,
    pub from_address: String,
    pub from_name: String,
    pub to_list: Vec<EmailAddress>,
    pub cc_list: Vec<EmailAddress>,
    pub bcc_list: Vec<EmailAddress>,
    pub has_attachments: bool,
    pub is_read: bool,
    pub is_starred: bool,
    pub is_draft: bool,
    pub date: i64,
    pub remote_version: Option<String>,
    pub is_deleted: bool,
    pub deleted_at: Option<i64>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmailAddress {
    pub name: Option<String>,
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Attachment {
    pub id: String,
    pub message_id: String,
    pub filename: String,
    pub mime_type: String,
    pub size: i64,
    pub local_path: Option<String>,
    pub content_id: Option<String>,
    pub is_inline: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserLabel {
    pub id: String,
    pub name: String,
    pub color: String,
    pub is_system: bool,
    pub rule_id: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum KanbanColumn {
    Todo,
    Waiting,
    Done,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KanbanCard {
    pub message_id: String,
    pub column: KanbanColumn,
    pub position: i32,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SnoozedMessage {
    pub message_id: String,
    pub snoozed_at: i64,
    pub unsnoozed_at: i64,
    pub return_to: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrustedSender {
    pub account_id: String,
    pub email: String,
    pub trust_type: TrustType,
    pub created_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TrustType {
    Images,
    All,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rule {
    pub id: String,
    pub name: String,
    pub priority: i32,
    pub conditions: String,
    pub actions: String,
    pub is_enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PrivacyMode {
    Strict,
    TrustSender(String),
    LoadOnce,
    Off,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RenderedHtml {
    pub html: String,
    pub trackers_blocked: Vec<TrackerInfo>,
    pub images_blocked: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrackerInfo {
    pub domain: String,
    pub tracker_type: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProviderCapabilities {
    pub has_labels: bool,
    pub has_folders: bool,
    pub has_categories: bool,
    pub has_push: bool,
    pub has_threads: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TranslateConfig {
    pub id: String,
    pub provider_type: String,
    pub config: String,
    pub is_enabled: bool,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ThreadSummary {
    pub thread_id: String,
    pub subject: String,
    pub snippet: String,
    pub last_date: i64,
    pub message_count: u32,
    pub unread_count: u32,
    pub is_starred: bool,
    pub participants: Vec<String>,
    pub has_attachments: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KnownContact {
    pub name: Option<String>,
    pub address: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ContactEmailLabel {
    Work,
    Personal,
    Other,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContactEmail {
    pub id: String,
    pub address: String,
    pub label: ContactEmailLabel,
    pub is_primary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Contact {
    pub id: String,
    pub display_name: String,
    pub notes: String,
    pub is_favorite: bool,
    pub emails: Vec<ContactEmail>,
    pub created_at: i64,
    pub updated_at: i64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContactEmailInput {
    pub id: Option<String>,
    pub address: String,
    pub label: ContactEmailLabel,
    pub is_primary: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContactInput {
    pub id: Option<String>,
    pub display_name: String,
    pub notes: String,
    pub is_favorite: bool,
    pub emails: Vec<ContactEmailInput>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum ContactSuggestionSource {
    Saved,
    Recent,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct ContactSuggestion {
    pub contact_id: Option<String>,
    pub name: Option<String>,
    pub address: String,
    pub source: ContactSuggestionSource,
    pub is_favorite: bool,
    pub last_interaction_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct VcardImportResult {
    pub created: usize,
    pub merged: usize,
    pub skipped: usize,
    pub invalid: usize,
    pub errors: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Category {
    pub id: String,
    pub name: String,
    pub color: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DraftMessage {
    #[serde(default)]
    pub from: Option<EmailAddress>,
    pub id: Option<String>,
    pub to: Vec<EmailAddress>,
    pub cc: Vec<EmailAddress>,
    pub bcc: Vec<EmailAddress>,
    pub subject: String,
    pub body_text: String,
    pub body_html: Option<String>,
    pub in_reply_to: Option<String>,
    #[serde(default)]
    pub attachment_paths: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthTokens {
    pub access_token: String,
    pub refresh_token: Option<String>,
    pub expires_at: Option<i64>,
    pub scopes: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OAuthConfig {
    pub client_id: String,
    pub auth_url: String,
    pub token_url: String,
    pub scopes: Vec<String>,
    pub redirect_port: u16,
}

pub fn new_id() -> String {
    Uuid::new_v4().to_string()
}

pub fn now_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

pub fn utc_date_from_unix_seconds(timestamp: i64) -> (i64, u32, u32) {
    let days = timestamp.div_euclid(86_400) + 719_468;
    let era = if days >= 0 { days } else { days - 146_096 }.div_euclid(146_097);
    let day_of_era = days - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    (year, month as u32, day as u32)
}
