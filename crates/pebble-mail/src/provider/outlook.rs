use std::sync::RwLock;

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::debug;

use pebble_core::traits::*;
use pebble_core::{
    new_id, now_timestamp, Category, DraftMessage, EmailAddress, Folder, FolderRole, FolderType,
    Message, PebbleError, ProviderCapabilities, Result,
};

const GRAPH_API_BASE: &str = "https://graph.microsoft.com/v1.0/me";

// ---------------------------------------------------------------------------
// Microsoft Graph API response types (internal)
// ---------------------------------------------------------------------------

#[allow(dead_code)]
#[derive(Deserialize)]
struct GraphMessageList {
    value: Vec<GraphMessage>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
    #[serde(rename = "@odata.deltaLink")]
    delta_link: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct GraphMessage {
    id: String,
    subject: Option<String>,
    #[serde(rename = "bodyPreview")]
    body_preview: Option<String>,
    body: Option<GraphBody>,
    from: Option<GraphRecipient>,
    #[serde(rename = "toRecipients")]
    to_recipients: Option<Vec<GraphRecipient>>,
    #[serde(rename = "ccRecipients")]
    cc_recipients: Option<Vec<GraphRecipient>>,
    #[serde(rename = "isRead")]
    is_read: Option<bool>,
    flag: Option<GraphFlag>,
    #[serde(rename = "isDraft")]
    is_draft: Option<bool>,
    #[serde(rename = "receivedDateTime")]
    received_date_time: Option<String>,
    #[serde(rename = "internetMessageId")]
    internet_message_id: Option<String>,
    #[serde(rename = "conversationId")]
    conversation_id: Option<String>,
    #[serde(rename = "hasAttachments")]
    has_attachments: Option<bool>,
    categories: Option<Vec<String>>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct GraphBody {
    #[serde(rename = "contentType")]
    content_type: Option<String>,
    content: Option<String>,
}

#[derive(Deserialize)]
struct GraphRecipient {
    #[serde(rename = "emailAddress")]
    email_address: GraphEmailAddress,
}

#[derive(Deserialize)]
struct GraphEmailAddress {
    name: Option<String>,
    address: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct GraphFlag {
    #[serde(rename = "flagStatus")]
    flag_status: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct GraphFolder {
    id: String,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    #[serde(rename = "totalItemCount")]
    total_item_count: Option<i64>,
    #[serde(rename = "childFolderCount")]
    child_folder_count: Option<i64>,
    #[serde(rename = "wellKnownName")]
    well_known_name: Option<String>,
}

#[derive(Deserialize)]
struct GraphFolderList {
    value: Vec<GraphFolder>,
}

#[derive(Deserialize)]
struct GraphCategory {
    id: Option<String>,
    #[serde(rename = "displayName")]
    display_name: Option<String>,
    color: Option<String>,
}

#[derive(Deserialize)]
struct GraphCategoryList {
    value: Vec<GraphCategory>,
}

#[derive(Serialize)]
struct GraphSendMail {
    message: GraphOutgoingMessage,
}

#[derive(Serialize)]
struct GraphOutgoingMessage {
    subject: String,
    body: GraphOutgoingBody,
    #[serde(rename = "toRecipients")]
    to_recipients: Vec<GraphOutgoingRecipient>,
    #[serde(rename = "ccRecipients")]
    cc_recipients: Vec<GraphOutgoingRecipient>,
    #[serde(rename = "bccRecipients")]
    bcc_recipients: Vec<GraphOutgoingRecipient>,
    #[serde(rename = "replyTo", skip_serializing_if = "Option::is_none")]
    reply_to: Option<Vec<GraphOutgoingRecipient>>,
}

#[derive(Serialize)]
struct GraphOutgoingBody {
    #[serde(rename = "contentType")]
    content_type: String,
    content: String,
}

#[derive(Serialize)]
struct GraphOutgoingRecipient {
    #[serde(rename = "emailAddress")]
    email_address: GraphOutgoingEmailAddress,
}

#[derive(Serialize)]
struct GraphOutgoingEmailAddress {
    name: Option<String>,
    address: String,
}

#[derive(Serialize)]
struct GraphMoveRequest {
    #[serde(rename = "destinationId")]
    destination_id: String,
}

#[derive(Serialize)]
struct GraphCategoryPatch {
    categories: Vec<String>,
}

#[derive(Serialize)]
struct GraphDraftMessage {
    subject: String,
    body: GraphOutgoingBody,
    #[serde(rename = "toRecipients")]
    to_recipients: Vec<GraphOutgoingRecipient>,
    #[serde(rename = "ccRecipients")]
    cc_recipients: Vec<GraphOutgoingRecipient>,
    #[serde(rename = "bccRecipients")]
    bcc_recipients: Vec<GraphOutgoingRecipient>,
    #[serde(rename = "isDraft")]
    is_draft: bool,
}

#[derive(Deserialize)]
struct GraphDraftResponse {
    id: String,
}

// ---------------------------------------------------------------------------
// OutlookProvider
// ---------------------------------------------------------------------------

pub struct OutlookProvider {
    client: Client,
    access_token: RwLock<String>,
    account_id: String,
}

impl OutlookProvider {
    pub fn new(access_token: String, account_id: String) -> Self {
        Self {
            client: Client::new(),
            access_token: RwLock::new(access_token),
            account_id,
        }
    }

    pub fn set_access_token(&self, token: String) {
        *self.access_token.write().unwrap_or_else(|e| e.into_inner()) = token;
    }

    pub fn token(&self) -> String {
        self.access_token.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    async fn get(&self, url: &str) -> Result<reqwest::Response> {
        self.client
            .get(url)
            .bearer_auth(self.token())
            .send()
            .await
            .map_err(|e| PebbleError::Network(format!("Graph API GET failed: {e}")))
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
            .map_err(|e| PebbleError::Network(format!("Graph API POST failed: {e}")))
    }

    async fn patch_json<T: Serialize + Send + Sync>(
        &self,
        url: &str,
        body: &T,
    ) -> Result<reqwest::Response> {
        self.client
            .patch(url)
            .bearer_auth(self.token())
            .json(body)
            .send()
            .await
            .map_err(|e| PebbleError::Network(format!("Graph API PATCH failed: {e}")))
    }

    async fn delete(&self, url: &str) -> Result<reqwest::Response> {
        self.client
            .delete(url)
            .bearer_auth(self.token())
            .send()
            .await
            .map_err(|e| PebbleError::Network(format!("Graph API DELETE failed: {e}")))
    }

    fn graph_message_to_message(gm: &GraphMessage, account_id: &str) -> Message {
        let now = now_timestamp();

        let subject = gm.subject.clone().unwrap_or_default();
        let snippet = gm.body_preview.clone().unwrap_or_default();

        let (from_name, from_address) = gm
            .from
            .as_ref()
            .map(graph_recipient_to_parts)
            .unwrap_or_default();

        let to_list = gm
            .to_recipients
            .as_ref()
            .map(|rs| rs.iter().map(graph_recipient_to_email_address).collect())
            .unwrap_or_default();

        let cc_list = gm
            .cc_recipients
            .as_ref()
            .map(|rs| rs.iter().map(graph_recipient_to_email_address).collect())
            .unwrap_or_default();

        let is_read = gm.is_read.unwrap_or(false);
        let is_starred = gm
            .flag
            .as_ref()
            .and_then(|f| f.flag_status.as_deref())
            .map(|s| s == "flagged")
            .unwrap_or(false);
        let is_draft = gm.is_draft.unwrap_or(false);
        let has_attachments = gm.has_attachments.unwrap_or(false);

        let date = gm
            .received_date_time
            .as_ref()
            .and_then(|d| parse_graph_datetime(d))
            .unwrap_or(now);

        let (body_text, body_html_raw) = gm
            .body
            .as_ref()
            .map(|b| {
                let content = b.content.clone().unwrap_or_default();
                let ct = b.content_type.as_deref().unwrap_or("text");
                if ct.eq_ignore_ascii_case("html") {
                    (String::new(), content)
                } else {
                    (content, String::new())
                }
            })
            .unwrap_or_default();

        Message {
            id: new_id(),
            account_id: account_id.to_string(),
            remote_id: gm.id.clone(),
            message_id_header: gm.internet_message_id.clone(),
            in_reply_to: None,
            references_header: None,
            thread_id: gm.conversation_id.clone(),
            subject,
            snippet,
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
impl MailTransport for OutlookProvider {
    async fn authenticate(&mut self, credentials: &AuthCredentials) -> Result<()> {
        if let Some(token) = credentials.data.get("access_token").and_then(|v| v.as_str()) {
            self.set_access_token(token.to_string());
        }
        // Verify by making a profile request
        let resp = self.get(GRAPH_API_BASE).await?;
        if !resp.status().is_success() {
            return Err(PebbleError::Auth(
                "Outlook authentication failed".to_string(),
            ));
        }
        debug!("Outlook authentication successful");
        Ok(())
    }

    async fn fetch_messages(&self, query: &FetchQuery) -> Result<FetchResult> {
        let limit = query.limit.unwrap_or(50);
        let select = "id,subject,bodyPreview,body,from,toRecipients,ccRecipients,isRead,flag,isDraft,receivedDateTime,internetMessageId,conversationId,hasAttachments,categories";
        let url = format!(
            "{GRAPH_API_BASE}/mailFolders/{}/messages?$top={limit}&$select={select}",
            query.folder_id
        );
        let resp = self.get(&url).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to fetch messages (status {status}): {text}"
            )));
        }
        let list: GraphMessageList = resp
            .json()
            .await
            .map_err(|e| PebbleError::Network(format!("Failed to parse message list: {e}")))?;

        debug!(count = list.value.len(), "Fetched Outlook messages");

        let messages: Vec<Message> = list
            .value
            .iter()
            .map(|gm| Self::graph_message_to_message(gm, &self.account_id))
            .collect();

        let cursor_value = list.next_link.unwrap_or_default();
        Ok(FetchResult {
            messages,
            cursor: SyncCursor {
                value: cursor_value,
            },
        })
    }

    async fn send_message(&self, message: &OutgoingMessage) -> Result<()> {
        let (content_type, content) = if let Some(ref html) = message.body_html {
            ("HTML".to_string(), html.clone())
        } else {
            ("Text".to_string(), message.body_text.clone())
        };

        let body = GraphSendMail {
            message: GraphOutgoingMessage {
                subject: message.subject.clone(),
                body: GraphOutgoingBody {
                    content_type,
                    content,
                },
                to_recipients: message.to.iter().map(email_to_graph_recipient).collect(),
                cc_recipients: message.cc.iter().map(email_to_graph_recipient).collect(),
                bcc_recipients: message.bcc.iter().map(email_to_graph_recipient).collect(),
                reply_to: None,
            },
        };

        let resp = self
            .post_json(&format!("{GRAPH_API_BASE}/sendMail"), &body)
            .await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to send message via Outlook (status {status}): {text}"
            )));
        }
        debug!("Message sent via Graph API");
        Ok(())
    }

    async fn sync_changes(&self, since: &SyncCursor) -> Result<ChangeSet> {
        // Use delta link from previous sync, or start a new delta query
        let url = if since.value.starts_with("https://") {
            since.value.clone()
        } else {
            format!(
                "{GRAPH_API_BASE}/mailFolders/{}/messages/delta",
                since.value
            )
        };

        let resp = self.get(&url).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to sync changes (status {status}): {text}"
            )));
        }

        let list: GraphMessageList = resp
            .json()
            .await
            .map_err(|e| PebbleError::Network(format!("Failed to parse delta response: {e}")))?;

        let new_messages: Vec<Message> = list
            .value
            .iter()
            .map(|gm| Self::graph_message_to_message(gm, &self.account_id))
            .collect();

        let cursor = list
            .delta_link
            .or(list.next_link)
            .unwrap_or_default();

        Ok(ChangeSet {
            new_messages,
            flag_changes: vec![],
            moved: vec![],
            deleted: vec![],
            cursor: SyncCursor { value: cursor },
        })
    }

    fn capabilities(&self) -> ProviderCapabilities {
        ProviderCapabilities {
            has_labels: false,
            has_folders: true,
            has_categories: true,
            has_push: false,
            has_threads: true,
        }
    }
}

#[async_trait]
impl FolderProvider for OutlookProvider {
    async fn list_folders(&self) -> Result<Vec<Folder>> {
        let url = format!("{GRAPH_API_BASE}/mailFolders?$top=100");
        let resp = self.get(&url).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to list folders (status {status}): {text}"
            )));
        }
        let folder_list: GraphFolderList = resp
            .json()
            .await
            .map_err(|e| PebbleError::Network(format!("Failed to parse folder list: {e}")))?;

        Ok(folder_list
            .value
            .iter()
            .map(graph_folder_to_folder)
            .collect())
    }

    async fn move_message(&self, remote_id: &str, to_folder_id: &str) -> Result<()> {
        let body = GraphMoveRequest {
            destination_id: to_folder_id.to_string(),
        };
        let url = format!("{GRAPH_API_BASE}/messages/{remote_id}/move");
        let resp = self.post_json(&url, &body).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to move message (status {status}): {text}"
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl CategoryProvider for OutlookProvider {
    async fn list_categories(&self) -> Result<Vec<Category>> {
        let url = format!("{GRAPH_API_BASE}/outlook/masterCategories");
        let resp = self.get(&url).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to list categories (status {status}): {text}"
            )));
        }
        let cat_list: GraphCategoryList = resp
            .json()
            .await
            .map_err(|e| PebbleError::Network(format!("Failed to parse categories: {e}")))?;

        Ok(cat_list
            .value
            .iter()
            .map(graph_category_to_category)
            .collect())
    }

    async fn set_categories(&self, message_id: &str, categories: &[String]) -> Result<()> {
        let body = GraphCategoryPatch {
            categories: categories.to_vec(),
        };
        let url = format!("{GRAPH_API_BASE}/messages/{message_id}");
        let resp = self.patch_json(&url, &body).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to set categories (status {status}): {text}"
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl DraftProvider for OutlookProvider {
    async fn save_draft(&self, draft: &DraftMessage) -> Result<String> {
        let (content_type, content) = if let Some(ref html) = draft.body_html {
            ("HTML".to_string(), html.clone())
        } else {
            ("Text".to_string(), draft.body_text.clone())
        };

        let body = GraphDraftMessage {
            subject: draft.subject.clone(),
            body: GraphOutgoingBody {
                content_type,
                content,
            },
            to_recipients: draft.to.iter().map(email_to_graph_recipient).collect(),
            cc_recipients: draft.cc.iter().map(email_to_graph_recipient).collect(),
            bcc_recipients: draft.bcc.iter().map(email_to_graph_recipient).collect(),
            is_draft: true,
        };

        let url = format!("{GRAPH_API_BASE}/messages");
        let resp = self.post_json(&url, &body).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to save draft (status {status}): {text}"
            )));
        }
        let draft_resp: GraphDraftResponse = resp
            .json()
            .await
            .map_err(|e| PebbleError::Network(format!("Failed to parse draft response: {e}")))?;
        Ok(draft_resp.id)
    }

    async fn update_draft(&self, draft_id: &str, draft: &DraftMessage) -> Result<()> {
        let (content_type, content) = if let Some(ref html) = draft.body_html {
            ("HTML".to_string(), html.clone())
        } else {
            ("Text".to_string(), draft.body_text.clone())
        };

        let body = GraphDraftMessage {
            subject: draft.subject.clone(),
            body: GraphOutgoingBody {
                content_type,
                content,
            },
            to_recipients: draft.to.iter().map(email_to_graph_recipient).collect(),
            cc_recipients: draft.cc.iter().map(email_to_graph_recipient).collect(),
            bcc_recipients: draft.bcc.iter().map(email_to_graph_recipient).collect(),
            is_draft: true,
        };

        let url = format!("{GRAPH_API_BASE}/messages/{draft_id}");
        let resp = self.patch_json(&url, &body).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to update draft (status {status}): {text}"
            )));
        }
        Ok(())
    }

    async fn delete_draft(&self, draft_id: &str) -> Result<()> {
        let url = format!("{GRAPH_API_BASE}/messages/{draft_id}");
        let resp = self.delete(&url).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to delete draft (status {status}): {text}"
            )));
        }
        Ok(())
    }

    async fn list_drafts(&self) -> Result<Vec<DraftMessage>> {
        let select = "id,subject,body,toRecipients,ccRecipients,isDraft";
        let url = format!(
            "{GRAPH_API_BASE}/mailFolders/Drafts/messages?$select={select}"
        );
        let resp = self.get(&url).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to list drafts (status {status}): {text}"
            )));
        }
        let list: GraphMessageList = resp
            .json()
            .await
            .map_err(|e| PebbleError::Network(format!("Failed to parse drafts list: {e}")))?;

        Ok(list
            .value
            .iter()
            .map(graph_message_to_draft)
            .collect())
    }
}

impl MailProvider for OutlookProvider {
    fn as_category_provider(&self) -> Option<&dyn CategoryProvider> {
        Some(self)
    }

    fn as_draft_provider(&self) -> Option<&dyn DraftProvider> {
        Some(self)
    }
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

fn graph_recipient_to_parts(r: &GraphRecipient) -> (String, String) {
    let name = r.email_address.name.clone().unwrap_or_default();
    let address = r.email_address.address.clone().unwrap_or_default();
    (name, address)
}

fn graph_recipient_to_email_address(r: &GraphRecipient) -> EmailAddress {
    let (name, address) = graph_recipient_to_parts(r);
    EmailAddress {
        name: if name.is_empty() { None } else { Some(name) },
        address,
    }
}

fn email_to_graph_recipient(addr: &EmailAddress) -> GraphOutgoingRecipient {
    GraphOutgoingRecipient {
        email_address: GraphOutgoingEmailAddress {
            name: addr.name.clone(),
            address: addr.address.clone(),
        },
    }
}

/// Map Graph API well-known folder names to FolderRole.
fn well_known_name_to_role(name: &str) -> Option<FolderRole> {
    match name.to_lowercase().as_str() {
        "inbox" => Some(FolderRole::Inbox),
        "sentitems" => Some(FolderRole::Sent),
        "drafts" => Some(FolderRole::Drafts),
        "deleteditems" => Some(FolderRole::Trash),
        "archive" => Some(FolderRole::Archive),
        "junkemail" => Some(FolderRole::Spam),
        _ => None,
    }
}

fn graph_folder_to_folder(gf: &GraphFolder) -> Folder {
    let role = gf
        .well_known_name
        .as_deref()
        .and_then(well_known_name_to_role);
    let is_system = role.is_some();
    let sort_order = crate::imap::folder_sort_order(&role);
    Folder {
        id: new_id(),
        account_id: String::new(),
        remote_id: gf.id.clone(),
        name: gf.display_name.clone().unwrap_or_default(),
        folder_type: FolderType::Folder,
        role,
        parent_id: None,
        color: None,
        is_system,
        sort_order,
    }
}

fn graph_category_to_category(gc: &GraphCategory) -> Category {
    Category {
        id: gc.id.clone().unwrap_or_default(),
        name: gc.display_name.clone().unwrap_or_default(),
        color: gc.color.clone(),
    }
}

fn graph_message_to_draft(gm: &GraphMessage) -> DraftMessage {
    let to = gm
        .to_recipients
        .as_ref()
        .map(|rs| rs.iter().map(graph_recipient_to_email_address).collect())
        .unwrap_or_default();
    let cc = gm
        .cc_recipients
        .as_ref()
        .map(|rs| rs.iter().map(graph_recipient_to_email_address).collect())
        .unwrap_or_default();

    let (body_text, body_html) = gm
        .body
        .as_ref()
        .map(|b| {
            let content = b.content.clone().unwrap_or_default();
            let ct = b.content_type.as_deref().unwrap_or("text");
            if ct.eq_ignore_ascii_case("html") {
                (String::new(), Some(content))
            } else {
                (content, None)
            }
        })
        .unwrap_or_default();

    DraftMessage {
        id: Some(gm.id.clone()),
        to,
        cc,
        bcc: vec![],
        subject: gm.subject.clone().unwrap_or_default(),
        body_text,
        body_html,
        in_reply_to: None,
    }
}

/// Parse an ISO 8601 datetime string (e.g., "2024-01-15T10:30:00Z") to Unix timestamp.
fn parse_graph_datetime(s: &str) -> Option<i64> {
    // Simple parser for ISO 8601 dates returned by Graph API.
    // Format: YYYY-MM-DDTHH:MM:SSZ or with fractional seconds.
    let s = s.trim().trim_end_matches('Z');
    let parts: Vec<&str> = s.split('T').collect();
    if parts.len() != 2 {
        return None;
    }
    let date_parts: Vec<i64> = parts[0].split('-').filter_map(|p| p.parse().ok()).collect();
    if date_parts.len() != 3 {
        return None;
    }
    let time_str = parts[1].split('.').next()?; // strip fractional seconds
    let time_parts: Vec<i64> = time_str.split(':').filter_map(|p| p.parse().ok()).collect();
    if time_parts.len() != 3 {
        return None;
    }

    let (year, month, day) = (date_parts[0], date_parts[1], date_parts[2]);
    let (hour, min, sec) = (time_parts[0], time_parts[1], time_parts[2]);

    // Days from year 0 to 1970-01-01 is not needed; use a simpler epoch calculation.
    // Calculate days since Unix epoch using a well-known algorithm.
    let m = if month <= 2 { month + 9 } else { month - 3 };
    let y = if month <= 2 { year - 1 } else { year };
    let days = 365 * y + y / 4 - y / 100 + y / 400 + (m * 306 + 5) / 10 + day - 1
        - 719468; // days from 0000-03-01 to 1970-01-01
    Some(days * 86400 + hour * 3600 + min * 60 + sec)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_well_known_name_to_role_inbox() {
        assert_eq!(well_known_name_to_role("inbox"), Some(FolderRole::Inbox));
    }

    #[test]
    fn test_well_known_name_to_role_sent() {
        assert_eq!(
            well_known_name_to_role("sentitems"),
            Some(FolderRole::Sent)
        );
    }

    #[test]
    fn test_well_known_name_to_role_drafts() {
        assert_eq!(well_known_name_to_role("drafts"), Some(FolderRole::Drafts));
    }

    #[test]
    fn test_well_known_name_to_role_trash() {
        assert_eq!(
            well_known_name_to_role("deleteditems"),
            Some(FolderRole::Trash)
        );
    }

    #[test]
    fn test_well_known_name_to_role_archive() {
        assert_eq!(
            well_known_name_to_role("archive"),
            Some(FolderRole::Archive)
        );
    }

    #[test]
    fn test_well_known_name_to_role_spam() {
        assert_eq!(well_known_name_to_role("junkemail"), Some(FolderRole::Spam));
    }

    #[test]
    fn test_well_known_name_to_role_unknown() {
        assert_eq!(well_known_name_to_role("customfolder"), None);
    }

    #[test]
    fn test_well_known_name_to_role_case_insensitive() {
        assert_eq!(well_known_name_to_role("Inbox"), Some(FolderRole::Inbox));
        assert_eq!(
            well_known_name_to_role("SentItems"),
            Some(FolderRole::Sent)
        );
        assert_eq!(
            well_known_name_to_role("JunkEmail"),
            Some(FolderRole::Spam)
        );
    }

    #[test]
    fn test_capabilities() {
        let provider = OutlookProvider::new("token".to_string(), "test-account".to_string());
        let caps = provider.capabilities();
        assert!(!caps.has_labels);
        assert!(caps.has_folders);
        assert!(caps.has_categories);
        assert!(!caps.has_push);
        assert!(caps.has_threads);
    }

    #[test]
    fn test_graph_recipient_to_email_address_with_name() {
        let r = GraphRecipient {
            email_address: GraphEmailAddress {
                name: Some("Alice".to_string()),
                address: Some("alice@example.com".to_string()),
            },
        };
        let addr = graph_recipient_to_email_address(&r);
        assert_eq!(addr.name, Some("Alice".to_string()));
        assert_eq!(addr.address, "alice@example.com");
    }

    #[test]
    fn test_graph_recipient_to_email_address_no_name() {
        let r = GraphRecipient {
            email_address: GraphEmailAddress {
                name: None,
                address: Some("bob@example.com".to_string()),
            },
        };
        let addr = graph_recipient_to_email_address(&r);
        assert_eq!(addr.name, None);
        assert_eq!(addr.address, "bob@example.com");
    }

    #[test]
    fn test_email_to_graph_recipient() {
        let addr = EmailAddress {
            name: Some("Charlie".to_string()),
            address: "charlie@example.com".to_string(),
        };
        let r = email_to_graph_recipient(&addr);
        assert_eq!(r.email_address.name, Some("Charlie".to_string()));
        assert_eq!(r.email_address.address, "charlie@example.com");
    }

    #[test]
    fn test_graph_category_to_category() {
        let gc = GraphCategory {
            id: Some("cat-1".to_string()),
            display_name: Some("Important".to_string()),
            color: Some("preset0".to_string()),
        };
        let cat = graph_category_to_category(&gc);
        assert_eq!(cat.id, "cat-1");
        assert_eq!(cat.name, "Important");
        assert_eq!(cat.color, Some("preset0".to_string()));
    }

    #[test]
    fn test_graph_category_to_category_minimal() {
        let gc = GraphCategory {
            id: None,
            display_name: None,
            color: None,
        };
        let cat = graph_category_to_category(&gc);
        assert_eq!(cat.id, "");
        assert_eq!(cat.name, "");
        assert_eq!(cat.color, None);
    }

    #[test]
    fn test_graph_folder_to_folder_inbox() {
        let gf = GraphFolder {
            id: "AAMkAD".to_string(),
            display_name: Some("Inbox".to_string()),
            total_item_count: Some(42),
            child_folder_count: Some(0),
            well_known_name: Some("inbox".to_string()),
        };
        let folder = graph_folder_to_folder(&gf);
        assert_eq!(folder.role, Some(FolderRole::Inbox));
        assert_eq!(folder.folder_type, FolderType::Folder);
        assert!(folder.is_system);
        assert_eq!(folder.remote_id, "AAMkAD");
        assert_eq!(folder.name, "Inbox");
    }

    #[test]
    fn test_graph_folder_to_folder_custom() {
        let gf = GraphFolder {
            id: "custom-id".to_string(),
            display_name: Some("My Folder".to_string()),
            total_item_count: Some(10),
            child_folder_count: Some(2),
            well_known_name: None,
        };
        let folder = graph_folder_to_folder(&gf);
        assert_eq!(folder.role, None);
        assert!(!folder.is_system);
        assert_eq!(folder.name, "My Folder");
    }

    #[test]
    fn test_parse_graph_datetime() {
        // 2024-01-15T10:30:00Z
        let ts = parse_graph_datetime("2024-01-15T10:30:00Z");
        assert!(ts.is_some());
        let ts = ts.unwrap();
        // 2024-01-15 10:30:00 UTC = 1705314600
        assert_eq!(ts, 1705314600);
    }

    #[test]
    fn test_parse_graph_datetime_with_fractional() {
        let ts = parse_graph_datetime("2024-01-15T10:30:00.123Z");
        assert!(ts.is_some());
        assert_eq!(ts.unwrap(), 1705314600);
    }

    #[test]
    fn test_parse_graph_datetime_invalid() {
        assert_eq!(parse_graph_datetime("not-a-date"), None);
        assert_eq!(parse_graph_datetime(""), None);
    }

    #[test]
    fn test_graph_message_to_draft() {
        let gm = GraphMessage {
            id: "draft-123".to_string(),
            subject: Some("Draft Subject".to_string()),
            body_preview: None,
            body: Some(GraphBody {
                content_type: Some("Text".to_string()),
                content: Some("Draft body".to_string()),
            }),
            from: None,
            to_recipients: Some(vec![GraphRecipient {
                email_address: GraphEmailAddress {
                    name: Some("Recipient".to_string()),
                    address: Some("recv@example.com".to_string()),
                },
            }]),
            cc_recipients: None,
            is_read: None,
            flag: None,
            is_draft: Some(true),
            received_date_time: None,
            internet_message_id: None,
            conversation_id: None,
            has_attachments: None,
            categories: None,
        };
        let draft = graph_message_to_draft(&gm);
        assert_eq!(draft.id, Some("draft-123".to_string()));
        assert_eq!(draft.subject, "Draft Subject");
        assert_eq!(draft.body_text, "Draft body");
        assert_eq!(draft.body_html, None);
        assert_eq!(draft.to.len(), 1);
        assert_eq!(draft.to[0].address, "recv@example.com");
    }

    #[test]
    fn test_set_access_token() {
        let provider = OutlookProvider::new("initial".to_string(), "test-account".to_string());
        assert_eq!(provider.token(), "initial");
        provider.set_access_token("updated".to_string());
        assert_eq!(provider.token(), "updated");
    }
}
