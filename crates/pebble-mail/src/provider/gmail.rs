use std::sync::RwLock;

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::debug;

use crate::parser::{AttachmentData, AttachmentMeta};
use pebble_core::traits::*;
use pebble_core::{
    new_id, now_timestamp, DraftMessage, EmailAddress, Folder, FolderRole, FolderType, Message,
    PebbleError, ProviderCapabilities, Result,
};

const GMAIL_API_BASE: &str = "https://www.googleapis.com/gmail/v1/users/me";

// ---------------------------------------------------------------------------
// Gmail API response types (internal)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
#[derive(Deserialize)]
struct GmailMessageList {
    messages: Option<Vec<GmailMessageRef>>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
pub struct GmailMessageRef {
    pub id: String,
    #[serde(rename = "threadId")]
    pub thread_id: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct GmailMessage {
    id: String,
    #[serde(rename = "threadId")]
    thread_id: Option<String>,
    #[serde(rename = "labelIds")]
    label_ids: Option<Vec<String>>,
    snippet: Option<String>,
    payload: Option<GmailPayload>,
    #[serde(rename = "internalDate")]
    internal_date: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct GmailPayload {
    headers: Option<Vec<GmailHeader>>,
    #[serde(rename = "mimeType")]
    mime_type: Option<String>,
    body: Option<GmailBody>,
    parts: Option<Vec<GmailPayload>>,
    filename: Option<String>,
}

#[derive(Deserialize)]
struct GmailHeader {
    name: String,
    value: String,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct GmailBody {
    size: Option<u64>,
    data: Option<String>,
    #[serde(rename = "attachmentId")]
    attachment_id: Option<String>,
}

#[derive(Deserialize)]
struct GmailLabel {
    id: String,
    name: String,
    #[serde(rename = "type")]
    label_type: Option<String>,
}

#[derive(Deserialize)]
struct GmailLabelList {
    labels: Option<Vec<GmailLabel>>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct GmailHistoryList {
    history: Option<Vec<GmailHistoryEntry>>,
    #[serde(rename = "nextPageToken")]
    next_page_token: Option<String>,
    #[serde(rename = "historyId")]
    history_id: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct GmailHistoryEntry {
    #[serde(rename = "messagesAdded")]
    messages_added: Option<Vec<GmailHistoryMessage>>,
    #[serde(rename = "messagesDeleted")]
    messages_deleted: Option<Vec<GmailHistoryMessage>>,
    #[serde(rename = "labelsAdded")]
    labels_added: Option<Vec<GmailHistoryLabelChange>>,
    #[serde(rename = "labelsRemoved")]
    labels_removed: Option<Vec<GmailHistoryLabelChange>>,
}

#[derive(Deserialize)]
struct GmailHistoryMessage {
    message: GmailMessageRef,
}

#[derive(Deserialize)]
struct GmailHistoryLabelChange {
    message: GmailMessageRef,
    #[serde(rename = "labelIds")]
    label_ids: Vec<String>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct GmailDraft {
    id: String,
    message: Option<GmailMessageRef>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct GmailDraftList {
    drafts: Option<Vec<GmailDraft>>,
}

#[derive(Debug, Clone)]
pub struct GmailFetchedMessage {
    pub message: Message,
    pub visible_label_ids: Vec<String>,
    pub attachments: Vec<AttachmentData>,
}

#[derive(Debug, Clone)]
struct GmailAttachmentDescriptor {
    filename: String,
    mime_type: String,
    size: usize,
    content_id: Option<String>,
    is_inline: bool,
    data: Option<Vec<u8>>,
    attachment_id: Option<String>,
}

// ---------------------------------------------------------------------------
// GmailProvider
// ---------------------------------------------------------------------------

pub struct GmailProvider {
    client: Client,
    access_token: RwLock<String>,
}

impl GmailProvider {
    pub fn new(access_token: String) -> Self {
        Self {
            client: Client::new(),
            access_token: RwLock::new(access_token),
        }
    }

    pub fn set_access_token(&self, token: String) {
        *self.access_token.write().unwrap_or_else(|e| e.into_inner()) = token;
    }

    pub fn token(&self) -> String {
        self.access_token.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub(crate) async fn get(&self, url: &str) -> Result<reqwest::Response> {
        self.client
            .get(url)
            .bearer_auth(self.token())
            .send()
            .await
            .map_err(|e| PebbleError::Network(format!("Gmail API request failed: {e}")))
    }

    async fn post_json<T: Serialize + Send + Sync>(
        &self,
        url: &str,
        body: &T,
    ) -> Result<reqwest::Response> {
        self.client
            .post(url)
            .bearer_auth(self.token())
            .json(body)
            .send()
            .await
            .map_err(|e| PebbleError::Network(format!("Gmail API POST failed: {e}")))
    }

    async fn delete(&self, url: &str) -> Result<reqwest::Response> {
        self.client
            .delete(url)
            .bearer_auth(self.token())
            .send()
            .await
            .map_err(|e| PebbleError::Network(format!("Gmail API DELETE failed: {e}")))
    }

    fn get_header<'a>(headers: &'a [GmailHeader], name: &str) -> Option<&'a str> {
        headers
            .iter()
            .find(|h| h.name.eq_ignore_ascii_case(name))
            .map(|h| h.value.as_str())
    }

    async fn fetch_full_gmail_message(&self, gmail_id: &str) -> Result<GmailMessage> {
        let url = format!("{GMAIL_API_BASE}/messages/{gmail_id}?format=full");
        let resp = self.get(&url).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to fetch message {gmail_id} (status {status}): {text}"
            )));
        }
        resp.json()
            .await
            .map_err(|e| PebbleError::Network(format!("Failed to parse message {gmail_id}: {e}")))
    }

    async fn fetch_attachment_bytes(&self, gmail_id: &str, attachment_id: &str) -> Result<Vec<u8>> {
        let url = format!("{GMAIL_API_BASE}/messages/{gmail_id}/attachments/{attachment_id}");
        let resp = self.get(&url).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to fetch attachment {attachment_id} for message {gmail_id} (status {status}): {text}"
            )));
        }

        #[derive(Deserialize)]
        struct GmailAttachmentResponse {
            data: Option<String>,
        }

        let attachment: GmailAttachmentResponse = resp
            .json()
            .await
            .map_err(|e| PebbleError::Network(format!("Failed to parse attachment body: {e}")))?;

        Ok(attachment
            .data
            .as_deref()
            .map(base64url_decode)
            .unwrap_or_default())
    }

    async fn fetch_attachment_parts(
        &self,
        gmail_id: &str,
        payload: &GmailPayload,
    ) -> Result<Vec<AttachmentData>> {
        let descriptors = collect_attachment_descriptors(payload);
        let mut attachments = Vec::with_capacity(descriptors.len());

        for descriptor in descriptors {
            let data = match (descriptor.data, descriptor.attachment_id.as_deref()) {
                (Some(data), _) => data,
                (None, Some(attachment_id)) => {
                    self.fetch_attachment_bytes(gmail_id, attachment_id).await?
                }
                (None, None) => Vec::new(),
            };

            attachments.push(AttachmentData {
                meta: AttachmentMeta {
                    filename: descriptor.filename,
                    mime_type: descriptor.mime_type,
                    size: descriptor.size.max(data.len()),
                    content_id: descriptor.content_id,
                    is_inline: descriptor.is_inline,
                },
                data,
            });
        }

        Ok(attachments)
    }

    /// Fetch a single full message by its Gmail ID.
    pub async fn fetch_full_message(&self, gmail_id: &str, account_id: &str) -> Result<Message> {
        let fetched = self.fetch_sync_message(gmail_id, account_id).await?;
        Ok(fetched.message)
    }

    pub async fn fetch_sync_message(
        &self,
        gmail_id: &str,
        account_id: &str,
    ) -> Result<GmailFetchedMessage> {
        let gm = self.fetch_full_gmail_message(gmail_id).await?;
        let visible_label_ids = visible_label_ids(gm.label_ids.as_deref().unwrap_or(&[]));
        let attachments = if let Some(payload) = gm.payload.as_ref() {
            self.fetch_attachment_parts(gmail_id, payload).await?
        } else {
            Vec::new()
        };

        Ok(GmailFetchedMessage {
            message: Self::gmail_message_to_message(&gm, account_id),
            visible_label_ids,
            attachments,
        })
    }

    /// List message IDs (and thread IDs) for a given label, with pagination.
    pub async fn list_message_ids(
        &self,
        label_id: &str,
        max_results: u32,
        page_token: Option<&str>,
    ) -> Result<(Vec<GmailMessageRef>, Option<String>)> {
        let mut url = format!(
            "{GMAIL_API_BASE}/messages?labelIds={label_id}&maxResults={max_results}"
        );
        if let Some(token) = page_token {
            url.push_str(&format!("&pageToken={token}"));
        }
        let resp = self.get(&url).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to list messages for label {label_id} (status {status}): {text}"
            )));
        }
        let list: GmailMessageList = resp
            .json()
            .await
            .map_err(|e| PebbleError::Network(format!("Failed to parse message list: {e}")))?;
        let refs = list.messages.unwrap_or_default();
        Ok((refs, list.next_page_token))
    }

    /// Get the user's Gmail profile (contains historyId for sync).
    pub async fn get_profile(&self) -> Result<(String, String)> {
        let resp = self.get(&format!("{GMAIL_API_BASE}/profile")).await?;
        let profile: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| PebbleError::Network(format!("Failed to parse profile: {e}")))?;
        let email = profile["emailAddress"].as_str().unwrap_or("").to_string();
        // historyId is a number in the API response, not a string
        let history_id = match &profile["historyId"] {
            serde_json::Value::String(s) => s.clone(),
            serde_json::Value::Number(n) => n.to_string(),
            _ => String::new(),
        };
        debug!(email = %email, history_id = %history_id, "Gmail profile");
        Ok((email, history_id))
    }

    pub async fn trash_message(&self, remote_id: &str) -> Result<()> {
        let url = format!("{GMAIL_API_BASE}/messages/{remote_id}/trash");
        let resp = self.post_json(&url, &serde_json::json!({})).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to move message to trash (status {status}): {text}"
            )));
        }
        Ok(())
    }

    pub async fn untrash_message(&self, remote_id: &str) -> Result<()> {
        let url = format!("{GMAIL_API_BASE}/messages/{remote_id}/untrash");
        let resp = self.post_json(&url, &serde_json::json!({})).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to restore message from trash (status {status}): {text}"
            )));
        }
        Ok(())
    }

    pub async fn delete_message_permanently(&self, remote_id: &str) -> Result<()> {
        let url = format!("{GMAIL_API_BASE}/messages/{remote_id}");
        let resp = self.delete(&url).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to permanently delete message (status {status}): {text}"
            )));
        }
        Ok(())
    }

    fn gmail_message_to_message(gm: &GmailMessage, account_id: &str) -> Message {
        let now = now_timestamp();
        let payload = gm.payload.as_ref();
        let headers = payload.and_then(|p| p.headers.as_ref());
        let empty_headers: Vec<GmailHeader> = vec![];
        let hdrs = headers.unwrap_or(&empty_headers);

        debug!(
            gmail_id = %gm.id,
            header_count = hdrs.len(),
            headers = ?hdrs.iter().map(|h| format!("{}={}", h.name, &h.value[..h.value.len().min(60)])).collect::<Vec<_>>(),
            "Parsing Gmail message headers"
        );

        let subject = Self::get_header(hdrs, "Subject")
            .unwrap_or("")
            .to_string();
        let from_raw = Self::get_header(hdrs, "From").unwrap_or("");
        let (from_name, from_address) = parse_email_header(from_raw);
        let to_raw = Self::get_header(hdrs, "To").unwrap_or("");
        let to_list = parse_address_list(to_raw);
        let cc_raw = Self::get_header(hdrs, "Cc").unwrap_or("");
        let cc_list = parse_address_list(cc_raw);
        let message_id_header = Self::get_header(hdrs, "Message-ID").map(|s| s.to_string());
        let in_reply_to = Self::get_header(hdrs, "In-Reply-To").map(|s| s.to_string());
        let references = Self::get_header(hdrs, "References").map(|s| s.to_string());

        let date = gm
            .internal_date
            .as_ref()
            .and_then(|d| d.parse::<i64>().ok())
            .map(|ms| ms / 1000)
            .unwrap_or(now);

        let label_ids = gm.label_ids.as_deref().unwrap_or(&[]);
        let is_read = !label_ids.iter().any(|l| l == "UNREAD");
        let is_starred = label_ids.iter().any(|l| l == "STARRED");
        let is_draft = label_ids.iter().any(|l| l == "DRAFT");

        // Extract body content from payload
        let (body_text, body_html_raw) = payload
            .map(extract_body_parts)
            .unwrap_or_default();
        let has_attachments = gm.payload.as_ref().map(has_attachment_parts).unwrap_or(false);

        debug!(
            gmail_id = %gm.id,
            subject = %subject,
            from_name = %from_name,
            from_address = %from_address,
            body_text_len = body_text.len(),
            body_html_len = body_html_raw.len(),
            snippet_len = gm.snippet.as_ref().map(|s| s.len()).unwrap_or(0),
            "Parsed Gmail message"
        );

        Message {
            id: new_id(),
            account_id: account_id.to_string(),
            remote_id: gm.id.clone(),
            message_id_header,
            in_reply_to,
            references_header: references,
            thread_id: gm.thread_id.clone(),
            subject,
            snippet: gm.snippet.clone().unwrap_or_default(),
            from_address,
            from_name,
            to_list,
            cc_list,
            bcc_list: vec![],
            body_text,
            body_html_raw,
            has_attachments,
            is_read,
            is_starred,
            is_draft,
            date,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        }
    }
}

// ---------------------------------------------------------------------------
// Trait implementations
// ---------------------------------------------------------------------------

#[async_trait]
impl MailTransport for GmailProvider {
    async fn authenticate(&mut self, credentials: &AuthCredentials) -> Result<()> {
        if let Some(token) = credentials.data.get("access_token").and_then(|v| v.as_str()) {
            self.set_access_token(token.to_string());
        }
        // Verify by making a profile request
        let resp = self.get(&format!("{GMAIL_API_BASE}/profile")).await?;
        if !resp.status().is_success() {
            return Err(PebbleError::Auth("Gmail authentication failed".to_string()));
        }
        debug!("Gmail authentication successful");
        Ok(())
    }

    async fn fetch_messages(&self, query: &FetchQuery) -> Result<FetchResult> {
        let limit = query.limit.unwrap_or(50);
        let url = format!(
            "{GMAIL_API_BASE}/messages?labelIds={}&maxResults={limit}",
            query.folder_id
        );
        let resp = self.get(&url).await?;
        let list: GmailMessageList = resp
            .json()
            .await
            .map_err(|e| PebbleError::Network(format!("Failed to parse message list: {e}")))?;

        debug!(
            count = list.messages.as_ref().map(|m| m.len()).unwrap_or(0),
            "Fetched Gmail message IDs"
        );

        // Gmail list endpoint only returns IDs; full message fetch would require
        // individual GET requests for each message. Return the cursor for pagination.
        let cursor_value = list.next_page_token.unwrap_or_default();
        Ok(FetchResult {
            messages: vec![],
            cursor: SyncCursor {
                value: cursor_value,
            },
        })
    }

    async fn send_message(&self, message: &OutgoingMessage) -> Result<()> {
        let raw = build_raw_message(message);
        let encoded = base64url_encode(&raw);
        let body = serde_json::json!({ "raw": encoded });
        let resp = self
            .post_json(&format!("{GMAIL_API_BASE}/messages/send"), &body)
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to send message via Gmail (status {status}): {text}"
            )));
        }
        debug!("Message sent via Gmail API");
        Ok(())
    }

    async fn sync_changes(&self, since: &SyncCursor) -> Result<ChangeSet> {
        let url = format!(
            "{GMAIL_API_BASE}/history?startHistoryId={}",
            since.value
        );
        let resp = self.get(&url).await?;
        let history: GmailHistoryList = resp
            .json()
            .await
            .map_err(|e| PebbleError::Network(format!("Failed to parse history: {e}")))?;

        let mut deleted = Vec::new();
        let mut flag_changes = Vec::new();

        if let Some(entries) = &history.history {
            for entry in entries {
                // Collect deleted message IDs
                if let Some(ref removed) = entry.messages_deleted {
                    for m in removed {
                        deleted.push(m.message.id.clone());
                    }
                }
                // Collect label additions as flag changes (read/starred)
                if let Some(ref added) = entry.labels_added {
                    for change in added {
                        let is_read = if change.label_ids.iter().any(|l| l == "UNREAD") {
                            Some(false)
                        } else {
                            None
                        };
                        let is_starred = if change.label_ids.iter().any(|l| l == "STARRED") {
                            Some(true)
                        } else {
                            None
                        };
                        if is_read.is_some() || is_starred.is_some() {
                            flag_changes.push(FlagChange {
                                remote_id: change.message.id.clone(),
                                is_read,
                                is_starred,
                            });
                        }
                    }
                }
                // Collect label removals as flag changes
                if let Some(ref removed) = entry.labels_removed {
                    for change in removed {
                        let is_read = if change.label_ids.iter().any(|l| l == "UNREAD") {
                            Some(true)
                        } else {
                            None
                        };
                        let is_starred = if change.label_ids.iter().any(|l| l == "STARRED") {
                            Some(false)
                        } else {
                            None
                        };
                        if is_read.is_some() || is_starred.is_some() {
                            flag_changes.push(FlagChange {
                                remote_id: change.message.id.clone(),
                                is_read,
                                is_starred,
                            });
                        }
                    }
                }
            }
        }

        Ok(ChangeSet {
            new_messages: vec![],
            flag_changes,
            moved: vec![],
            deleted,
            cursor: SyncCursor {
                value: history.history_id.unwrap_or_default(),
            },
        })
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            has_labels: true,
            has_folders: false,
            has_categories: false,
            has_push: false,
            has_threads: true,
        }
    }
}

#[async_trait]
impl FolderProvider for GmailProvider {
    async fn list_folders(&self) -> Result<Vec<Folder>> {
        let resp = self.get(&format!("{GMAIL_API_BASE}/labels")).await?;
        let label_list: GmailLabelList = resp
            .json()
            .await
            .map_err(|e| PebbleError::Network(format!("Failed to parse labels: {e}")))?;
        let labels = label_list.labels.unwrap_or_default();
        Ok(labels
            .iter()
            .filter(|l| !is_hidden_gmail_label(&l.id))
            .map(gmail_label_to_folder)
            .collect())
    }

    async fn move_message(&self, remote_id: &str, to_folder_id: &str) -> Result<()> {
        // Gmail "move" is implemented as label modification
        let body = serde_json::json!({ "addLabelIds": [to_folder_id] });
        let url = format!("{GMAIL_API_BASE}/messages/{remote_id}/modify");
        let resp = self.post_json(&url, &body).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            return Err(PebbleError::Network(format!(
                "Failed to move message (status {status})"
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl LabelProvider for GmailProvider {
    async fn list_labels(&self) -> Result<Vec<Folder>> {
        self.list_folders().await
    }

    async fn modify_labels(
        &self,
        remote_id: &str,
        add: &[String],
        remove: &[String],
    ) -> Result<()> {
        let body = serde_json::json!({
            "addLabelIds": add,
            "removeLabelIds": remove,
        });
        let url = format!("{GMAIL_API_BASE}/messages/{remote_id}/modify");
        let resp = self.post_json(&url, &body).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            return Err(PebbleError::Network(format!(
                "Failed to modify labels (status {status})"
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl DraftProvider for GmailProvider {
    async fn save_draft(&self, draft: &DraftMessage) -> Result<String> {
        let raw = build_draft_raw(draft);
        let encoded = base64url_encode(&raw);
        let body = serde_json::json!({ "message": { "raw": encoded } });
        let resp = self
            .post_json(&format!("{GMAIL_API_BASE}/drafts"), &body)
            .await?;
        let gmail_draft: GmailDraft = resp
            .json()
            .await
            .map_err(|e| PebbleError::Network(format!("Failed to parse draft response: {e}")))?;
        Ok(gmail_draft.id)
    }

    async fn update_draft(&self, draft_id: &str, draft: &DraftMessage) -> Result<()> {
        let raw = build_draft_raw(draft);
        let encoded = base64url_encode(&raw);
        let body = serde_json::json!({ "message": { "raw": encoded } });
        let url = format!("{GMAIL_API_BASE}/drafts/{draft_id}");
        let resp = self
            .client
            .put(&url)
            .bearer_auth(self.token())
            .json(&body)
            .send()
            .await
            .map_err(|e| PebbleError::Network(format!("Failed to update draft: {e}")))?;
        if !resp.status().is_success() {
            let status = resp.status();
            return Err(PebbleError::Network(format!(
                "Failed to update draft (status {status})"
            )));
        }
        Ok(())
    }

    async fn delete_draft(&self, draft_id: &str) -> Result<()> {
        let resp = self
            .delete(&format!("{GMAIL_API_BASE}/drafts/{draft_id}"))
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            return Err(PebbleError::Network(format!(
                "Failed to delete draft (status {status})"
            )));
        }
        Ok(())
    }

    async fn list_drafts(&self) -> Result<Vec<DraftMessage>> {
        let resp = self.get(&format!("{GMAIL_API_BASE}/drafts")).await?;
        let _draft_list: GmailDraftList = resp
            .json()
            .await
            .map_err(|e| PebbleError::Network(format!("Failed to list drafts: {e}")))?;
        // Each draft requires an individual fetch for full content
        Ok(vec![])
    }
}

impl MailProvider for GmailProvider {
    fn as_label_provider(&self) -> Option<&dyn LabelProvider> {
        Some(self)
    }

    fn as_draft_provider(&self) -> Option<&dyn DraftProvider> {
        Some(self)
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Gmail system labels that should not appear as sidebar folders.
fn is_hidden_gmail_label(id: &str) -> bool {
    matches!(
        id,
        "CHAT"
            | "IMPORTANT"
            | "STARRED"
            | "UNREAD"
            | "CATEGORY_FORUMS"
            | "CATEGORY_UPDATES"
            | "CATEGORY_PERSONAL"
            | "CATEGORY_PROMOTIONS"
            | "CATEGORY_SOCIAL"
    )
}

pub(crate) fn visible_label_ids(label_ids: &[String]) -> Vec<String> {
    label_ids
        .iter()
        .filter(|label_id| !is_hidden_gmail_label(label_id))
        .cloned()
        .collect()
}

fn gmail_label_to_folder(label: &GmailLabel) -> Folder {
    let role = match label.id.as_str() {
        "INBOX" => Some(FolderRole::Inbox),
        "SENT" => Some(FolderRole::Sent),
        "DRAFT" => Some(FolderRole::Drafts),
        "TRASH" => Some(FolderRole::Trash),
        "SPAM" => Some(FolderRole::Spam),
        _ => None,
    };
    let sort_order = crate::imap::folder_sort_order(&role);
    Folder {
        id: new_id(),
        account_id: String::new(),
        remote_id: label.id.clone(),
        name: label.name.clone(),
        folder_type: FolderType::Label,
        role,
        parent_id: None,
        color: None,
        is_system: label.label_type.as_deref() == Some("system"),
        sort_order,
    }
}

fn parse_email_header(raw: &str) -> (String, String) {
    // Parse "Display Name <email@example.com>" or just "email@example.com"
    if let Some(start) = raw.rfind('<') {
        if let Some(end) = raw.rfind('>') {
            let name = raw[..start].trim().trim_matches('"').to_string();
            let addr = raw[start + 1..end].trim().to_string();
            return (name, addr);
        }
    }
    (String::new(), raw.trim().to_string())
}

fn parse_address_list(raw: &str) -> Vec<EmailAddress> {
    if raw.is_empty() {
        return vec![];
    }
    raw.split(',')
        .map(|s| {
            let (name, address) = parse_email_header(s.trim());
            EmailAddress {
                name: if name.is_empty() { None } else { Some(name) },
                address,
            }
        })
        .collect()
}

fn format_address(addr: &EmailAddress) -> String {
    match &addr.name {
        Some(name) => format!("{name} <{}>", addr.address),
        None => addr.address.clone(),
    }
}

fn write_common_headers(
    raw: &mut String,
    to: &[EmailAddress],
    cc: &[EmailAddress],
    bcc: &[EmailAddress],
    subject: &str,
    in_reply_to: Option<&str>,
) {
    let to = to.iter().map(format_address).collect::<Vec<_>>().join(", ");
    let cc = cc.iter().map(format_address).collect::<Vec<_>>().join(", ");
    let bcc = bcc.iter().map(format_address).collect::<Vec<_>>().join(", ");

    raw.push_str(&format!("To: {to}\r\n"));
    raw.push_str(&format!("Subject: {subject}\r\n"));
    if !cc.is_empty() {
        raw.push_str(&format!("Cc: {cc}\r\n"));
    }
    if !bcc.is_empty() {
        raw.push_str(&format!("Bcc: {bcc}\r\n"));
    }
    if let Some(irt) = in_reply_to {
        raw.push_str(&format!("In-Reply-To: {irt}\r\n"));
    }
    raw.push_str("MIME-Version: 1.0\r\n");
}

fn append_body(raw: &mut String, body_text: &str, body_html: Option<&str>) {
    if let Some(body_html) = body_html {
        const BOUNDARY: &str = "pebble-gmail-boundary";
        raw.push_str(&format!(
            "Content-Type: multipart/alternative; boundary=\"{BOUNDARY}\"\r\n\r\n"
        ));
        raw.push_str(&format!(
            "--{BOUNDARY}\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n{body_text}\r\n"
        ));
        raw.push_str(&format!(
            "--{BOUNDARY}\r\nContent-Type: text/html; charset=utf-8\r\n\r\n{body_html}\r\n"
        ));
        raw.push_str(&format!("--{BOUNDARY}--\r\n"));
    } else {
        raw.push_str("Content-Type: text/plain; charset=utf-8\r\n\r\n");
        raw.push_str(body_text);
    }
}

fn build_raw_message(msg: &OutgoingMessage) -> Vec<u8> {
    let mut raw = String::new();
    write_common_headers(
        &mut raw,
        &msg.to,
        &msg.cc,
        &msg.bcc,
        &msg.subject,
        msg.in_reply_to.as_deref(),
    );
    append_body(&mut raw, &msg.body_text, msg.body_html.as_deref());
    raw.into_bytes()
}

fn build_draft_raw(draft: &DraftMessage) -> Vec<u8> {
    let mut raw = String::new();
    write_common_headers(
        &mut raw,
        &draft.to,
        &draft.cc,
        &draft.bcc,
        &draft.subject,
        draft.in_reply_to.as_deref(),
    );
    append_body(&mut raw, &draft.body_text, draft.body_html.as_deref());
    raw.into_bytes()
}

/// Base64url decoding without padding (RFC 4648 section 5).
fn base64url_decode(input: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(input.len() * 3 / 4);
    let bytes: Vec<u8> = input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace())
        .map(|b| match b {
            b'A'..=b'Z' => b - b'A',
            b'a'..=b'z' => b - b'a' + 26,
            b'0'..=b'9' => b - b'0' + 52,
            b'-' => 62,
            b'_' => 63,
            _ => 0,
        })
        .collect();

    let chunks = bytes.chunks(4);
    for chunk in chunks {
        if chunk.len() >= 2 {
            let b0 = (chunk[0] as u32) << 18
                | (chunk[1] as u32) << 12
                | chunk.get(2).copied().unwrap_or(0) as u32 * 64
                | chunk.get(3).copied().unwrap_or(0) as u32;
            buf.push((b0 >> 16) as u8);
            if chunk.len() >= 3 {
                buf.push((b0 >> 8) as u8);
            }
            if chunk.len() >= 4 {
                buf.push(b0 as u8);
            }
        }
    }
    buf
}

/// Extract text/plain and text/html body parts from a Gmail payload, recursively.
fn extract_body_parts(payload: &GmailPayload) -> (String, String) {
    let mut text = String::new();
    let mut html = String::new();
    extract_body_recursive(payload, &mut text, &mut html);
    (text, html)
}

fn extract_body_recursive(payload: &GmailPayload, text: &mut String, html: &mut String) {
    let mime = payload.mime_type.as_deref().unwrap_or("");

    // If this part has direct body data, decode it
    if let Some(ref body) = payload.body {
        if let Some(ref data) = body.data {
            if !data.is_empty() {
                let decoded = base64url_decode(data);
                if let Ok(s) = String::from_utf8(decoded) {
                    if mime == "text/plain" && text.is_empty() {
                        *text = s;
                    } else if mime == "text/html" && html.is_empty() {
                        *html = s;
                    }
                }
            }
        }
    }

    // Recurse into sub-parts
    if let Some(ref parts) = payload.parts {
        for part in parts {
            extract_body_recursive(part, text, html);
        }
    }
}

/// Check if a payload has attachment parts.
fn has_attachment_parts(payload: &GmailPayload) -> bool {
    !collect_attachment_descriptors(payload).is_empty()
}

fn is_attachment_part(payload: &GmailPayload) -> bool {
    let filename = payload.filename.as_deref().unwrap_or("").trim();
    if !filename.is_empty() {
        return true;
    }

    payload
        .body
        .as_ref()
        .and_then(|body| body.attachment_id.as_ref())
        .is_some()
}

fn payload_content_id(payload: &GmailPayload) -> Option<String> {
    payload
        .headers
        .as_ref()
        .and_then(|headers| GmailProvider::get_header(headers, "Content-ID"))
        .map(|value| value.trim_matches(|ch| ch == '<' || ch == '>').to_string())
}

fn payload_is_inline(payload: &GmailPayload) -> bool {
    payload
        .headers
        .as_ref()
        .and_then(|headers| GmailProvider::get_header(headers, "Content-Disposition"))
        .map(|value| value.to_ascii_lowercase().contains("inline"))
        .unwrap_or(false)
}

fn collect_attachment_descriptors(payload: &GmailPayload) -> Vec<GmailAttachmentDescriptor> {
    let mut attachments = Vec::new();
    collect_attachment_descriptors_recursive(payload, &mut attachments);
    attachments
}

fn collect_attachment_descriptors_recursive(
    payload: &GmailPayload,
    attachments: &mut Vec<GmailAttachmentDescriptor>,
) {
    if is_attachment_part(payload) {
        let inline_data = payload
            .body
            .as_ref()
            .and_then(|body| body.data.as_deref())
            .map(base64url_decode);
        let size = payload
            .body
            .as_ref()
            .and_then(|body| body.size)
            .map(|size| size as usize)
            .or_else(|| inline_data.as_ref().map(Vec::len))
            .unwrap_or_default();

        attachments.push(GmailAttachmentDescriptor {
            filename: payload
                .filename
                .clone()
                .filter(|filename| !filename.trim().is_empty())
                .unwrap_or_else(|| "unnamed_attachment".to_string()),
            mime_type: payload
                .mime_type
                .clone()
                .unwrap_or_else(|| "application/octet-stream".to_string()),
            size,
            content_id: payload_content_id(payload),
            is_inline: payload_is_inline(payload),
            data: inline_data,
            attachment_id: payload
                .body
                .as_ref()
                .and_then(|body| body.attachment_id.clone()),
        });
    }

    if let Some(parts) = payload.parts.as_ref() {
        for part in parts {
            collect_attachment_descriptors_recursive(part, attachments);
        }
    }
}

#[cfg(test)]
fn collect_attachment_parts(payload: &GmailPayload) -> Result<Vec<AttachmentData>> {
    let mut attachments = Vec::new();
    for descriptor in collect_attachment_descriptors(payload) {
        if let Some(data) = descriptor.data {
            attachments.push(AttachmentData {
                meta: AttachmentMeta {
                    filename: descriptor.filename,
                    mime_type: descriptor.mime_type,
                    size: descriptor.size.max(data.len()),
                    content_id: descriptor.content_id,
                    is_inline: descriptor.is_inline,
                },
                data,
            });
        }
    }
    Ok(attachments)
}

/// Base64url encoding without padding (RFC 4648 section 5).
/// Implemented inline to avoid adding a `base64` crate dependency.
fn base64url_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_";

    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    let chunks = data.chunks(3);
    for chunk in chunks {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let triple = (b0 << 16) | (b1 << 8) | b2;

        result.push(ALPHABET[((triple >> 18) & 0x3F) as usize] as char);
        result.push(ALPHABET[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(ALPHABET[((triple >> 6) & 0x3F) as usize] as char);
        }
        if chunk.len() > 2 {
            result.push(ALPHABET[(triple & 0x3F) as usize] as char);
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_email_header_with_name() {
        let (name, addr) = parse_email_header("John Doe <john@example.com>");
        assert_eq!(name, "John Doe");
        assert_eq!(addr, "john@example.com");
    }

    #[test]
    fn test_parse_email_header_no_name() {
        let (name, addr) = parse_email_header("john@example.com");
        assert_eq!(name, "");
        assert_eq!(addr, "john@example.com");
    }

    #[test]
    fn test_parse_email_header_quoted_name() {
        let (name, addr) = parse_email_header("\"Jane Doe\" <jane@example.com>");
        assert_eq!(name, "Jane Doe");
        assert_eq!(addr, "jane@example.com");
    }

    #[test]
    fn test_parse_address_list() {
        let addrs = parse_address_list("Alice <a@b.com>, bob@c.com");
        assert_eq!(addrs.len(), 2);
        assert_eq!(addrs[0].name, Some("Alice".to_string()));
        assert_eq!(addrs[0].address, "a@b.com");
        assert_eq!(addrs[1].name, None);
        assert_eq!(addrs[1].address, "bob@c.com");
    }

    #[test]
    fn test_parse_address_list_empty() {
        let addrs = parse_address_list("");
        assert!(addrs.is_empty());
    }

    #[test]
    fn test_gmail_label_to_folder_inbox() {
        let label = GmailLabel {
            id: "INBOX".to_string(),
            name: "Inbox".to_string(),
            label_type: Some("system".to_string()),
        };
        let folder = gmail_label_to_folder(&label);
        assert_eq!(folder.role, Some(FolderRole::Inbox));
        assert_eq!(folder.folder_type, FolderType::Label);
        assert!(folder.is_system);
        assert_eq!(folder.remote_id, "INBOX");
    }

    #[test]
    fn test_gmail_label_to_folder_custom() {
        let label = GmailLabel {
            id: "Label_123".to_string(),
            name: "My Label".to_string(),
            label_type: Some("user".to_string()),
        };
        let folder = gmail_label_to_folder(&label);
        assert_eq!(folder.role, None);
        assert!(!folder.is_system);
        assert_eq!(folder.name, "My Label");
    }

    #[test]
    fn test_gmail_label_to_folder_sent() {
        let label = GmailLabel {
            id: "SENT".to_string(),
            name: "Sent".to_string(),
            label_type: Some("system".to_string()),
        };
        let folder = gmail_label_to_folder(&label);
        assert_eq!(folder.role, Some(FolderRole::Sent));
    }

    #[test]
    fn test_capabilities() {
        let provider = GmailProvider::new("token".to_string());
        let caps = provider.capabilities();
        assert!(caps.has_labels);
        assert!(!caps.has_folders);
        assert!(!caps.has_categories);
        assert!(!caps.has_push);
        assert!(caps.has_threads);
    }

    #[test]
    fn test_base64url_encode_basic() {
        let encoded = base64url_encode(b"Hello, World!");
        // Verify no standard base64 chars that differ in URL-safe variant
        assert!(!encoded.contains('+'));
        assert!(!encoded.contains('/'));
        assert!(!encoded.contains('='));
        assert_eq!(encoded, "SGVsbG8sIFdvcmxkIQ");
    }

    #[test]
    fn test_base64url_encode_empty() {
        let encoded = base64url_encode(b"");
        assert_eq!(encoded, "");
    }

    #[test]
    fn test_base64url_encode_padding_cases() {
        // 1 byte -> 2 base64 chars (no padding)
        assert_eq!(base64url_encode(b"a"), "YQ");
        // 2 bytes -> 3 base64 chars (no padding)
        assert_eq!(base64url_encode(b"ab"), "YWI");
        // 3 bytes -> 4 base64 chars (exact)
        assert_eq!(base64url_encode(b"abc"), "YWJj");
    }

    #[test]
    fn test_format_address_with_name() {
        let addr = EmailAddress {
            name: Some("Alice".to_string()),
            address: "alice@example.com".to_string(),
        };
        assert_eq!(format_address(&addr), "Alice <alice@example.com>");
    }

    #[test]
    fn test_format_address_no_name() {
        let addr = EmailAddress {
            name: None,
            address: "bob@example.com".to_string(),
        };
        assert_eq!(format_address(&addr), "bob@example.com");
    }

    #[test]
    fn test_build_raw_message() {
        let msg = OutgoingMessage {
            to: vec![EmailAddress {
                name: None,
                address: "test@example.com".to_string(),
            }],
            cc: vec![],
            bcc: vec![],
            subject: "Test Subject".to_string(),
            body_text: "Hello".to_string(),
            body_html: None,
            in_reply_to: None,
        };
        let raw = String::from_utf8(build_raw_message(&msg)).unwrap();
        assert!(raw.contains("To: test@example.com"));
        assert!(raw.contains("Subject: Test Subject"));
        assert!(raw.contains("Hello"));
        // Should not contain Cc header when cc is empty
        assert!(!raw.contains("Cc:"));
    }

    #[test]
    fn test_build_raw_message_with_cc_and_reply() {
        let msg = OutgoingMessage {
            to: vec![EmailAddress {
                name: Some("Alice".to_string()),
                address: "alice@example.com".to_string(),
            }],
            cc: vec![EmailAddress {
                name: None,
                address: "bob@example.com".to_string(),
            }],
            bcc: vec![],
            subject: "Re: Hello".to_string(),
            body_text: "Reply body".to_string(),
            body_html: None,
            in_reply_to: Some("<msg123@example.com>".to_string()),
        };
        let raw = String::from_utf8(build_raw_message(&msg)).unwrap();
        assert!(raw.contains("Cc: bob@example.com"));
        assert!(raw.contains("In-Reply-To: <msg123@example.com>"));
    }

    #[test]
    fn test_build_raw_message_with_bcc_and_html_body() {
        let msg = OutgoingMessage {
            to: vec![EmailAddress {
                name: None,
                address: "alice@example.com".to_string(),
            }],
            cc: vec![],
            bcc: vec![EmailAddress {
                name: Some("Hidden".to_string()),
                address: "hidden@example.com".to_string(),
            }],
            subject: "HTML update".to_string(),
            body_text: "Plain text body".to_string(),
            body_html: Some("<p><strong>HTML</strong> body</p>".to_string()),
            in_reply_to: None,
        };

        let raw = String::from_utf8(build_raw_message(&msg)).unwrap();
        assert!(raw.contains("Bcc: Hidden <hidden@example.com>"));
        assert!(raw.contains("multipart/alternative"));
        assert!(raw.contains("Content-Type: text/html; charset=utf-8"));
        assert!(raw.contains("<p><strong>HTML</strong> body</p>"));
    }

    #[test]
    fn test_visible_label_ids_exclude_hidden_labels() {
        let visible = visible_label_ids(&[
            "INBOX".to_string(),
            "STARRED".to_string(),
            "Label_123".to_string(),
            "UNREAD".to_string(),
            "TRASH".to_string(),
        ]);

        assert_eq!(
            visible,
            vec![
                "INBOX".to_string(),
                "Label_123".to_string(),
                "TRASH".to_string(),
            ]
        );
    }

    #[test]
    fn test_collect_attachment_parts_decodes_inline_attachment_data() {
        let payload = GmailPayload {
            headers: None,
            mime_type: Some("multipart/mixed".to_string()),
            body: None,
            parts: Some(vec![GmailPayload {
                headers: Some(vec![GmailHeader {
                    name: "Content-Disposition".to_string(),
                    value: "attachment".to_string(),
                }]),
                mime_type: Some("application/pdf".to_string()),
                body: Some(GmailBody {
                    size: Some(3),
                    data: Some(base64url_encode(b"pdf")),
                    attachment_id: None,
                }),
                parts: None,
                filename: Some("report.pdf".to_string()),
            }]),
            filename: None,
        };

        let attachments = collect_attachment_parts(&payload).unwrap();
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].meta.filename, "report.pdf");
        assert_eq!(attachments[0].meta.mime_type, "application/pdf");
        assert_eq!(attachments[0].meta.size, 3);
        assert_eq!(attachments[0].data, b"pdf");
    }

    #[test]
    fn test_set_access_token() {
        let provider = GmailProvider::new("initial".to_string());
        assert_eq!(provider.token(), "initial");
        provider.set_access_token("updated".to_string());
        assert_eq!(provider.token(), "updated");
    }
}
