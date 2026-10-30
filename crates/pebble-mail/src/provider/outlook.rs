use std::future::Future;
use std::sync::RwLock;

use async_trait::async_trait;
use reqwest::Client;
use serde::{Deserialize, Serialize};
use tracing::debug;

use super::http_client_with_proxy;
use pebble_core::traits::*;
use pebble_core::{
    new_id, now_timestamp, Category, DraftMessage, EmailAddress, Folder, FolderRole, FolderType,
    HttpProxyConfig, Message, PebbleError, ProviderCapabilities, Result,
};

const GRAPH_API_BASE: &str = "https://graph.microsoft.com/v1.0/me";
const GRAPH_MESSAGE_SELECT: &str = "id,subject,bodyPreview,body,from,toRecipients,ccRecipients,isRead,flag,isDraft,receivedDateTime,internetMessageId,conversationId,hasAttachments,categories";
pub(crate) const MAX_GRAPH_CONTINUATION_PAGES: usize = 1_000;

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

#[derive(Deserialize)]
pub(crate) struct GraphDeltaList {
    value: Vec<serde_json::Value>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
    #[serde(rename = "@odata.deltaLink")]
    delta_link: Option<String>,
}

#[derive(Deserialize)]
struct GraphRemoved {
    #[allow(dead_code)]
    reason: Option<String>,
}

#[allow(dead_code)]
#[derive(Deserialize)]
struct GraphMessage {
    id: String,
    #[serde(rename = "@removed")]
    removed: Option<GraphRemoved>,
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

pub struct OutlookDeltaPage {
    pub messages: Vec<Message>,
    pub deleted_remote_ids: Vec<String>,
    pub next_link: Option<String>,
    pub delta_link: Option<String>,
}

pub(crate) async fn resolve_outlook_delta_page<F, Fut>(
    list: GraphDeltaList,
    account_id: &str,
    mut fetch_message: F,
) -> Result<OutlookDeltaPage>
where
    F: FnMut(String) -> Fut,
    Fut: Future<Output = Result<Option<serde_json::Value>>>,
{
    let mut messages = Vec::new();
    let mut deleted_remote_ids = Vec::new();
    for mut value in list.value {
        let remote_id = value
            .get("id")
            .and_then(serde_json::Value::as_str)
            .filter(|id| !id.trim().is_empty())
            .ok_or_else(|| PebbleError::Network("Delta message did not include an ID".to_string()))?
            .to_string();
        if let Some(removed) = value.get("@removed").filter(|removed| !removed.is_null()) {
            serde_json::from_value::<GraphRemoved>(removed.clone()).map_err(|error| {
                PebbleError::Network(format!("Invalid delta tombstone: {error}"))
            })?;
            deleted_remote_ids.push(remote_id);
            continue;
        }

        // Delta updates may omit unchanged properties even when they were selected.
        // Fetch a complete current snapshot before converting missing fields to defaults.
        if !graph_message_has_complete_fields(&value) {
            let Some(complete) = fetch_message(remote_id.clone()).await? else {
                // The message can disappear between the delta response and this GET.
                deleted_remote_ids.push(remote_id);
                continue;
            };
            if complete.get("id").and_then(serde_json::Value::as_str) != Some(remote_id.as_str())
                || complete
                    .get("@removed")
                    .is_some_and(|removed| !removed.is_null())
                || !graph_message_has_complete_fields(&complete)
            {
                return Err(PebbleError::Network(format!(
                    "Incomplete or mismatched Graph message response for {remote_id}"
                )));
            }
            value = complete;
        }

        let gm: GraphMessage = serde_json::from_value(value)
            .map_err(|error| PebbleError::Network(format!("Invalid delta message: {error}")))?;
        messages.push(OutlookProvider::graph_message_to_message(&gm, account_id));
    }
    Ok(OutlookDeltaPage {
        messages,
        deleted_remote_ids,
        next_link: list.next_link,
        delta_link: list.delta_link,
    })
}

fn graph_message_has_complete_fields(value: &serde_json::Value) -> bool {
    let Some(fields) = value.as_object() else {
        return false;
    };
    if !GRAPH_MESSAGE_SELECT
        .split(',')
        // Categories are selected for Graph but are not written into Message.
        .filter(|field| *field != "categories")
        .all(|field| fields.contains_key(field))
    {
        return false;
    }

    fn has_fields_or_null(value: &serde_json::Value, required: &[&str]) -> bool {
        value.is_null()
            || value
                .as_object()
                .is_some_and(|fields| required.iter().all(|field| fields.contains_key(*field)))
    }

    fn complete_recipient(value: &serde_json::Value) -> bool {
        value.is_null()
            || value
                .get("emailAddress")
                .is_some_and(|address| has_fields_or_null(address, &["name", "address"]))
    }

    // Explicit nulls, empty strings/lists, and false values are intentional values.
    has_fields_or_null(&fields["body"], &["content", "contentType"])
        && has_fields_or_null(&fields["flag"], &["flagStatus"])
        && complete_recipient(&fields["from"])
        && ["toRecipients", "ccRecipients"].iter().all(|field| {
            fields[*field].is_null()
                || fields[*field]
                    .as_array()
                    .is_some_and(|recipients| recipients.iter().all(complete_recipient))
        })
}

fn graph_message_fetch_url(remote_id: &str) -> Result<String> {
    let mut url = reqwest::Url::parse(&format!("{GRAPH_API_BASE}/messages/"))
        .map_err(|error| PebbleError::Network(format!("Invalid Graph message URL: {error}")))?;
    url.path_segments_mut()
        .map_err(|_| PebbleError::Network("Invalid Graph message URL base".to_string()))?
        .pop_if_empty()
        .push(remote_id);
    url.query_pairs_mut()
        .append_pair("$select", GRAPH_MESSAGE_SELECT);
    Ok(url.to_string())
}

fn outlook_delta_page_to_changes(page: OutlookDeltaPage) -> ChangeSet {
    ChangeSet {
        new_messages: page.messages,
        flag_changes: vec![],
        moved: vec![],
        deleted: page.deleted_remote_ids,
        cursor: SyncCursor {
            value: page.delta_link.or(page.next_link).unwrap_or_default(),
        },
    }
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
    from: GraphOutgoingRecipient,
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
    #[serde(skip_serializing_if = "Vec::is_empty")]
    attachments: Vec<GraphFileAttachment>,
}

#[derive(Serialize)]
struct GraphFileAttachment {
    #[serde(rename = "@odata.type")]
    odata_type: String,
    name: String,
    #[serde(rename = "contentType")]
    content_type: String,
    #[serde(rename = "contentBytes")]
    content_bytes: String,
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

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct GraphAttachmentItem {
    #[allow(dead_code)]
    id: String,
    name: Option<String>,
    content_type: Option<String>,
    size: Option<i64>,
    #[serde(default)]
    is_inline: bool,
    content_id: Option<String>,
    /// Present only on fileAttachment resources.
    content_bytes: Option<String>,
    #[serde(rename = "@odata.type")]
    odata_type: Option<String>,
}

#[derive(Deserialize)]
struct GraphAttachmentList {
    value: Vec<GraphAttachmentItem>,
    #[serde(rename = "@odata.nextLink")]
    next_link: Option<String>,
}

#[async_trait]
#[allow(
    clippy::double_must_use,
    reason = "async_trait adds must_use to an already must-use boxed future"
)]
trait GraphAttachmentPageOperations {
    async fn fetch_attachment_page(&mut self, url: &str) -> Result<GraphAttachmentList>;
}

pub(crate) fn validate_graph_continuation_url(url: &str) -> Result<reqwest::Url> {
    let parsed = reqwest::Url::parse(url).map_err(|error| {
        PebbleError::Network(format!(
            "Refusing untrusted Graph continuation URL: invalid URL ({error})"
        ))
    })?;
    let is_trusted = parsed.scheme() == "https"
        && parsed
            .host_str()
            .is_some_and(|host| host.eq_ignore_ascii_case("graph.microsoft.com"))
        && parsed.port_or_known_default() == Some(443)
        && parsed.username().is_empty()
        && parsed.password().is_none()
        && parsed.path().starts_with("/v1.0/me/");
    if !is_trusted {
        return Err(PebbleError::Network(
            "Refusing untrusted Graph continuation URL".to_string(),
        ));
    }
    Ok(parsed)
}

fn outlook_delta_response_error(status: reqwest::StatusCode, body: &str) -> PebbleError {
    if status == reqwest::StatusCode::GONE
        || (status.is_client_error() && body.to_ascii_lowercase().contains("syncstatenotfound"))
    {
        PebbleError::SyncCursorExpired(format!(
            "Outlook delta cursor was rejected (status {status})"
        ))
    } else {
        PebbleError::Network(format!(
            "Failed to fetch Outlook delta (status {status}): {body}"
        ))
    }
}

fn outlook_delete_draft_status_result(status: reqwest::StatusCode, body: &str) -> Result<()> {
    if status.is_success() || status == reqwest::StatusCode::NOT_FOUND {
        Ok(())
    } else {
        Err(PebbleError::Network(format!(
            "Failed to delete draft (status {status}): {body}"
        )))
    }
}

fn validate_graph_attachment_next_link(next_link: &str) -> Result<()> {
    validate_graph_continuation_url(next_link)
        .map(|_| ())
        .map_err(|_| {
            PebbleError::Network("Refusing untrusted Graph attachment nextLink".to_string())
        })
}

async fn collect_graph_attachment_pages<O: GraphAttachmentPageOperations>(
    operations: &mut O,
    initial_url: &str,
) -> Result<Vec<GraphAttachmentItem>> {
    let mut url = initial_url.to_string();
    let mut items = Vec::new();
    let mut visited = std::collections::HashSet::new();
    let mut page_count = 0usize;

    loop {
        if page_count == MAX_GRAPH_CONTINUATION_PAGES {
            return Err(PebbleError::Network(
                "Graph attachment pagination exceeded page limit".to_string(),
            ));
        }
        page_count += 1;
        if !visited.insert(url.clone()) {
            return Err(PebbleError::Network(
                "Graph attachment pagination returned a repeated nextLink".to_string(),
            ));
        }
        let page = operations.fetch_attachment_page(&url).await?;
        items.extend(page.value);
        let Some(next_link) = page.next_link else {
            return Ok(items);
        };
        validate_graph_attachment_next_link(&next_link)?;
        url = next_link;
    }
}

#[async_trait]
#[allow(
    clippy::double_must_use,
    reason = "async_trait adds must_use to an already must-use boxed future"
)]
trait DraftAttachmentOperations {
    async fn upload(&mut self, attachment: &GraphFileAttachment) -> Result<String>;
    async fn delete(&mut self, attachment_id: &str) -> Result<()>;
}

#[async_trait]
#[allow(
    clippy::double_must_use,
    reason = "async_trait adds must_use to an already must-use boxed future"
)]
trait CreatedDraftCleanupOperations {
    async fn delete_created_draft(&mut self, draft_id: &str) -> Result<()>;
}

async fn finish_created_draft_attachments<O: CreatedDraftCleanupOperations>(
    cleanup: &mut O,
    draft_id: &str,
    attachment_result: Result<()>,
) -> Result<()> {
    let Err(attachment_error) = attachment_result else {
        return Ok(());
    };
    match cleanup.delete_created_draft(draft_id).await {
        Ok(()) => Err(attachment_error),
        Err(cleanup_error) => Err(PebbleError::Network(format!(
            "{attachment_error}; additionally failed to delete newly created draft {draft_id}: {cleanup_error}"
        ))),
    }
}

async fn replace_draft_attachment_set<O: DraftAttachmentOperations>(
    operations: &mut O,
    new_attachments: &[GraphFileAttachment],
    old_attachment_ids: &[String],
) -> Result<()> {
    let mut uploaded_ids = Vec::with_capacity(new_attachments.len());
    for attachment in new_attachments {
        match operations.upload(attachment).await {
            Ok(id) => uploaded_ids.push(id),
            Err(upload_error) => {
                let mut rollback_failures = Vec::new();
                for id in uploaded_ids.iter().rev() {
                    if let Err(error) = operations.delete(id).await {
                        rollback_failures.push(format!("{id}: {error}"));
                    }
                }
                if rollback_failures.is_empty() {
                    return Err(upload_error);
                }
                return Err(PebbleError::Network(format!(
                    "{upload_error}; additionally failed to roll back uploaded draft attachments: {}",
                    rollback_failures.join(", ")
                )));
            }
        }
    }

    for id in old_attachment_ids {
        operations.delete(id).await?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// OutlookProvider
// ---------------------------------------------------------------------------

pub struct OutlookProvider {
    client: Client,
    access_token: RwLock<String>,
    account_id: String,
}

struct OutlookGraphAttachmentPageOperations<'a> {
    provider: &'a OutlookProvider,
}

#[async_trait]
impl GraphAttachmentPageOperations for OutlookGraphAttachmentPageOperations<'_> {
    async fn fetch_attachment_page(&mut self, url: &str) -> Result<GraphAttachmentList> {
        let resp = self.provider.get(url).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to list attachments (status {status}): {text}"
            )));
        }
        resp.json().await.map_err(|error| {
            PebbleError::Network(format!("Failed to parse attachment list: {error}"))
        })
    }
}

struct OutlookDraftAttachmentOperations<'a> {
    provider: &'a OutlookProvider,
    draft_id: &'a str,
}

struct OutlookCreatedDraftCleanupOperations<'a> {
    provider: &'a OutlookProvider,
}

#[async_trait]
impl CreatedDraftCleanupOperations for OutlookCreatedDraftCleanupOperations<'_> {
    async fn delete_created_draft(&mut self, draft_id: &str) -> Result<()> {
        let url = format!("{GRAPH_API_BASE}/messages/{draft_id}");
        let resp = self.provider.delete(&url).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to delete newly created draft (status {status}): {text}"
            )));
        }
        Ok(())
    }
}

#[async_trait]
impl DraftAttachmentOperations for OutlookDraftAttachmentOperations<'_> {
    async fn upload(&mut self, attachment: &GraphFileAttachment) -> Result<String> {
        let url = format!("{GRAPH_API_BASE}/messages/{}/attachments", self.draft_id);
        let resp = self.provider.post_json(&url, attachment).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to add draft attachment (status {status}): {text}"
            )));
        }
        let uploaded: GraphDraftResponse = resp.json().await.map_err(|e| {
            PebbleError::Network(format!("Failed to parse uploaded draft attachment: {e}"))
        })?;
        if uploaded.id.trim().is_empty() {
            return Err(PebbleError::Network(
                "Uploaded draft attachment response did not include an ID".to_string(),
            ));
        }
        Ok(uploaded.id)
    }

    async fn delete(&mut self, attachment_id: &str) -> Result<()> {
        let url = format!(
            "{GRAPH_API_BASE}/messages/{}/attachments/{attachment_id}",
            self.draft_id
        );
        let resp = self.provider.delete(&url).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to delete draft attachment (status {status}): {text}"
            )));
        }
        Ok(())
    }
}

impl OutlookProvider {
    pub fn new(access_token: String, account_id: String) -> Self {
        Self {
            client: http_client_with_proxy(None).expect("failed to build Outlook HTTP client"),
            access_token: RwLock::new(access_token),
            account_id,
        }
    }

    pub fn new_with_proxy(
        access_token: String,
        account_id: String,
        proxy: Option<HttpProxyConfig>,
    ) -> Result<Self> {
        Ok(Self {
            client: http_client_with_proxy(proxy.as_ref())?,
            access_token: RwLock::new(access_token),
            account_id,
        })
    }

    pub fn set_access_token(&self, token: String) {
        *self.access_token.write().unwrap_or_else(|e| e.into_inner()) = token;
    }

    pub fn token(&self) -> String {
        self.access_token
            .read()
            .unwrap_or_else(|e| e.into_inner())
            .clone()
    }

    async fn get(&self, url: &str) -> Result<reqwest::Response> {
        self.client
            .get(url)
            .bearer_auth(self.token())
            .send()
            .await
            .map_err(|e| PebbleError::Network(format!("Graph API GET failed: {e}")))
    }

    pub async fn fetch_messages_page(
        &self,
        folder_id: &str,
        limit: u32,
        cursor: Option<&str>,
    ) -> Result<FetchResult> {
        let url = match cursor {
            Some(cursor) if !cursor.is_empty() => {
                validate_graph_continuation_url(cursor)?.to_string()
            }
            _ => format!(
                "{GRAPH_API_BASE}/mailFolders/{folder_id}/messages?$top={limit}&$select={GRAPH_MESSAGE_SELECT}"
            ),
        };
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
        if let Some(next_link) = list.next_link.as_deref() {
            validate_graph_continuation_url(next_link)?;
        }

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

    pub async fn fetch_delta_page(
        &self,
        folder_id: &str,
        cursor: Option<&str>,
    ) -> Result<OutlookDeltaPage> {
        let url = match cursor {
            Some(cursor) if !cursor.is_empty() => {
                validate_graph_continuation_url(cursor)?.to_string()
            }
            _ => format!(
                "{GRAPH_API_BASE}/mailFolders/{folder_id}/messages/delta?$top=50&$select={GRAPH_MESSAGE_SELECT}"
            ),
        };

        let resp = self.get(&url).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(outlook_delta_response_error(status, &text));
        }

        let list: GraphDeltaList = resp
            .json()
            .await
            .map_err(|e| PebbleError::Network(format!("Failed to parse delta response: {e}")))?;
        for continuation in [list.next_link.as_deref(), list.delta_link.as_deref()]
            .into_iter()
            .flatten()
        {
            validate_graph_continuation_url(continuation)?;
        }

        resolve_outlook_delta_page(list, &self.account_id, |remote_id| async move {
            self.fetch_graph_message(&remote_id).await
        })
        .await
    }

    async fn fetch_graph_message(&self, remote_id: &str) -> Result<Option<serde_json::Value>> {
        let resp = self.get(&graph_message_fetch_url(remote_id)?).await?;
        if resp.status() == reqwest::StatusCode::NOT_FOUND {
            return Ok(None);
        }
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to fetch Outlook message (status {status}): {text}"
            )));
        }
        resp.json().await.map(Some).map_err(|error| {
            PebbleError::Network(format!("Failed to parse Outlook message: {error}"))
        })
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

#[cfg(test)]
mod proxy_tests {
    use super::*;
    use pebble_core::HttpProxyConfig;

    #[tokio::test]
    async fn outlook_provider_new_ignores_all_proxy() {
        crate::provider::proxy_test_support::assert_client_builder_ignores_all_proxy(|| {
            OutlookProvider::new("access-token".to_string(), "account-id".to_string()).client
        })
        .await;
    }

    #[test]
    fn outlook_provider_accepts_socks5_proxy() {
        let provider = OutlookProvider::new_with_proxy(
            "access-token".to_string(),
            "account-id".to_string(),
            Some(HttpProxyConfig {
                host: "127.0.0.1".to_string(),
                port: 7890,
            }),
        );

        assert!(provider.is_ok());
    }

    #[test]
    fn outlook_provider_rejects_invalid_proxy() {
        let err = OutlookProvider::new_with_proxy(
            "access-token".to_string(),
            "account-id".to_string(),
            Some(HttpProxyConfig {
                host: " ".to_string(),
                port: 0,
            }),
        )
        .err()
        .unwrap();

        assert!(err.to_string().contains("Proxy host"));
    }
}

// ---------------------------------------------------------------------------
// Trait implementations
// ---------------------------------------------------------------------------

#[async_trait]
impl MailTransport for OutlookProvider {
    async fn authenticate(&mut self, credentials: &AuthCredentials) -> Result<()> {
        if let Some(token) = credentials
            .data
            .get("access_token")
            .and_then(|v| v.as_str())
        {
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
        self.fetch_messages_page(&query.folder_id, limit, None)
            .await
    }

    async fn send_message(&self, message: &OutgoingMessage) -> Result<()> {
        let (content_type, content) = if let Some(ref html) = message.body_html {
            ("HTML".to_string(), html.clone())
        } else {
            ("Text".to_string(), message.body_text.clone())
        };

        // Build attachment list
        let mut attachments = Vec::new();
        for path_str in &message.attachment_paths {
            attachments.push(graph_file_attachment_from_path(path_str)?);
        }

        let body = GraphSendMail {
            message: GraphOutgoingMessage {
                // Exchange controls the name. Only submit the verified mailbox address.
                from: email_to_graph_recipient(&EmailAddress {
                    name: None,
                    address: message.from.address.clone(),
                }),
                subject: message.subject.clone(),
                body: GraphOutgoingBody {
                    content_type,
                    content,
                },
                to_recipients: message.to.iter().map(email_to_graph_recipient).collect(),
                cc_recipients: message.cc.iter().map(email_to_graph_recipient).collect(),
                bcc_recipients: message.bcc.iter().map(email_to_graph_recipient).collect(),
                reply_to: None,
                attachments,
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
        let page = if since.value.starts_with("https://") {
            self.fetch_delta_page("", Some(&since.value)).await?
        } else {
            self.fetch_delta_page(&since.value, None).await?
        };
        Ok(outlook_delta_page_to_changes(page))
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

        let account_id = &self.account_id;
        Ok(folder_list
            .value
            .iter()
            .filter(|gf| {
                !should_hide_outlook_folder(
                    gf.display_name.as_deref(),
                    gf.well_known_name.as_deref(),
                )
            })
            .map(|gf| graph_folder_to_folder(gf, account_id))
            .collect())
    }

    async fn move_message(&self, remote_id: &str, to_folder_id: &str) -> Result<String> {
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
        // Graph API assigns a new message ID on every folder move.
        // Parse the response to capture it, otherwise subsequent
        // operations (delete, restore) would use a stale ID.
        let moved: serde_json::Value = resp
            .json()
            .await
            .map_err(|e| PebbleError::Network(format!("Failed to parse move response: {e}")))?;
        let new_id = moved["id"]
            .as_str()
            .ok_or_else(|| PebbleError::Network("Move response missing id field".into()))?
            .to_string();
        Ok(new_id)
    }
}

// ---------------------------------------------------------------------------
// Graph API write-back methods (flags, trash, delete, restore)
// ---------------------------------------------------------------------------

impl OutlookProvider {
    /// Update the read status of a message via Graph API.
    pub async fn update_read_status(&self, remote_id: &str, is_read: bool) -> Result<()> {
        let url = format!("{GRAPH_API_BASE}/messages/{remote_id}");
        let body = serde_json::json!({ "isRead": is_read });
        let resp = self.patch_json(&url, &body).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to update read status (status {status}): {text}"
            )));
        }
        Ok(())
    }

    /// Update the flag (starred) status of a message via Graph API.
    pub async fn update_flag_status(&self, remote_id: &str, is_starred: bool) -> Result<()> {
        let url = format!("{GRAPH_API_BASE}/messages/{remote_id}");
        let flag_status = if is_starred { "flagged" } else { "notFlagged" };
        let body = serde_json::json!({ "flag": { "flagStatus": flag_status } });
        let resp = self.patch_json(&url, &body).await?;
        if !resp.status().is_success() {
            let status = resp.status();
            let text = resp.text().await.unwrap_or_default();
            return Err(PebbleError::Network(format!(
                "Failed to update flag status (status {status}): {text}"
            )));
        }
        Ok(())
    }

    /// Move a message to the Deleted Items folder (trash) via Graph API.
    /// Returns the new remote message ID assigned by Graph after the move.
    pub async fn trash_message(&self, remote_id: &str) -> Result<String> {
        self.move_message(remote_id, "deleteditems").await
    }

    /// Move a message from trash back to inbox via Graph API.
    /// Returns the new remote message ID assigned by Graph after the move.
    pub async fn restore_message(&self, remote_id: &str) -> Result<String> {
        self.move_message(remote_id, "inbox").await
    }

    /// Permanently delete a message via Graph API.
    pub async fn delete_message_permanently(&self, remote_id: &str) -> Result<()> {
        let url = format!("{GRAPH_API_BASE}/messages/{remote_id}");
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

    /// Fetch all file attachments for a message via Graph API.
    /// Returns parsed `AttachmentData` suitable for `persist_message_attachments`.
    /// Inline and non-file attachments (itemAttachment, referenceAttachment) are skipped.
    async fn list_graph_attachment_items(
        &self,
        remote_id: &str,
    ) -> Result<Vec<GraphAttachmentItem>> {
        let url = format!("{GRAPH_API_BASE}/messages/{remote_id}/attachments");
        let mut operations = OutlookGraphAttachmentPageOperations { provider: self };
        collect_graph_attachment_pages(&mut operations, &url).await
    }

    async fn replace_draft_attachments(
        &self,
        draft_id: &str,
        new_attachments: &[GraphFileAttachment],
        remove_existing: bool,
    ) -> Result<()> {
        let old_attachment_ids = if remove_existing {
            self.list_graph_attachment_items(draft_id)
                .await?
                .into_iter()
                .filter(is_graph_file_attachment)
                .map(|item| item.id)
                .collect()
        } else {
            Vec::new()
        };
        let mut operations = OutlookDraftAttachmentOperations {
            provider: self,
            draft_id,
        };
        replace_draft_attachment_set(&mut operations, new_attachments, &old_attachment_ids).await
    }

    pub async fn list_message_attachments(
        &self,
        remote_id: &str,
    ) -> Result<Vec<crate::parser::AttachmentData>> {
        graph_attachment_items_to_data(self.list_graph_attachment_items(remote_id).await?)
    }
}

fn graph_attachment_items_to_data(
    items: Vec<GraphAttachmentItem>,
) -> Result<Vec<crate::parser::AttachmentData>> {
    let mut out = Vec::new();
    for item in items {
        // Only fileAttachment carries inline content bytes; skip others for now.
        if !is_graph_file_attachment(&item) {
            continue;
        }
        let b64 = item.content_bytes.ok_or_else(|| {
            PebbleError::Network(format!(
                "Outlook file attachment {} did not include contentBytes",
                item.id
            ))
        })?;
        let data = base64_standard_decode_outlook(&b64)?;
        let filename = item.name.unwrap_or_else(|| "attachment".to_string());
        let mime_type = item
            .content_type
            .unwrap_or_else(|| "application/octet-stream".to_string());
        let size = item.size.unwrap_or(data.len() as i64).max(0) as usize;
        out.push(crate::parser::AttachmentData {
            meta: crate::parser::AttachmentMeta {
                filename,
                mime_type,
                size,
                content_id: item.content_id,
                is_inline: item.is_inline,
            },
            data,
        });
    }
    Ok(out)
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
        let attachments = graph_file_attachments_from_paths(&draft.attachment_paths)?;
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
        let attachment_result = if attachments.is_empty() {
            Ok(())
        } else {
            self.replace_draft_attachments(&draft_resp.id, &attachments, false)
                .await
        };
        let mut cleanup = OutlookCreatedDraftCleanupOperations { provider: self };
        finish_created_draft_attachments(&mut cleanup, &draft_resp.id, attachment_result).await?;
        Ok(draft_resp.id)
    }

    async fn update_draft(&self, draft_id: &str, draft: &DraftMessage) -> Result<()> {
        let attachments = graph_file_attachments_from_paths(&draft.attachment_paths)?;
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
        self.replace_draft_attachments(draft_id, &attachments, true)
            .await?;
        Ok(())
    }

    async fn delete_draft(&self, draft_id: &str) -> Result<()> {
        let url = format!("{GRAPH_API_BASE}/messages/{draft_id}");
        let resp = self.delete(&url).await?;
        let status = resp.status();
        if status.is_success() || status == reqwest::StatusCode::NOT_FOUND {
            return Ok(());
        }
        let text = resp.text().await.unwrap_or_default();
        outlook_delete_draft_status_result(status, &text)
    }

    async fn list_drafts(&self) -> Result<Vec<DraftMessage>> {
        let select = "id,subject,body,toRecipients,ccRecipients,isDraft";
        let url = format!("{GRAPH_API_BASE}/mailFolders/Drafts/messages?$select={select}");
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

        Ok(list.value.iter().map(graph_message_to_draft).collect())
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

fn is_graph_file_attachment(item: &GraphAttachmentItem) -> bool {
    item.odata_type
        .as_deref()
        .map(|t| t.eq_ignore_ascii_case("#microsoft.graph.fileAttachment"))
        .unwrap_or(false)
}

fn graph_file_attachment_from_path(path_str: &str) -> Result<GraphFileAttachment> {
    let path = std::path::Path::new(path_str);
    let filename = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("attachment")
        .to_string();
    let data = std::fs::read(path)
        .map_err(|e| PebbleError::Internal(format!("Failed to read attachment {path_str}: {e}")))?;
    let mime = guess_outlook_mime(&filename);
    Ok(GraphFileAttachment {
        odata_type: "#microsoft.graph.fileAttachment".to_string(),
        name: filename,
        content_type: mime.to_string(),
        content_bytes: base64_standard_encode_outlook(&data),
    })
}

fn graph_file_attachments_from_paths(paths: &[String]) -> Result<Vec<GraphFileAttachment>> {
    paths
        .iter()
        .map(|path| graph_file_attachment_from_path(path))
        .collect()
}

fn base64_standard_encode_outlook(data: &[u8]) -> String {
    const ALPHABET: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = if chunk.len() > 1 { chunk[1] as u32 } else { 0 };
        let b2 = if chunk.len() > 2 { chunk[2] as u32 } else { 0 };
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 0x3F) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((n >> 6) & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(n & 0x3F) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

pub(crate) fn base64_standard_decode_outlook(input: &str) -> Result<Vec<u8>> {
    fn sextet(byte: u8) -> Option<u8> {
        match byte {
            b'A'..=b'Z' => Some(byte - b'A'),
            b'a'..=b'z' => Some(byte - b'a' + 26),
            b'0'..=b'9' => Some(byte - b'0' + 52),
            b'+' => Some(62),
            b'/' => Some(63),
            _ => None,
        }
    }

    if input.is_empty() {
        return Ok(Vec::new());
    }
    let bytes = input.as_bytes();
    if !bytes.len().is_multiple_of(4) {
        return Err(PebbleError::Network(
            "Invalid Outlook attachment base64 content: length is not a multiple of four"
                .to_string(),
        ));
    }
    let padding = bytes.iter().rev().take_while(|byte| **byte == b'=').count();
    if padding > 2 || bytes[..bytes.len() - padding].contains(&b'=') {
        return Err(PebbleError::Network(
            "Invalid Outlook attachment base64 content: invalid padding".to_string(),
        ));
    }

    let mut out = Vec::with_capacity(bytes.len() / 4 * 3 - padding);
    for (chunk_index, chunk) in bytes.as_chunks::<4>().0.iter().enumerate() {
        let is_last = chunk_index + 1 == bytes.len() / 4;
        let chunk_padding = if is_last { padding } else { 0 };
        let a = sextet(chunk[0]);
        let b = sextet(chunk[1]);
        let c = if chunk_padding == 2 {
            Some(0)
        } else {
            sextet(chunk[2])
        };
        let d = if chunk_padding >= 1 {
            Some(0)
        } else {
            sextet(chunk[3])
        };
        let (Some(a), Some(b), Some(c), Some(d)) = (a, b, c, d) else {
            return Err(PebbleError::Network(
                "Invalid Outlook attachment base64 content: non-standard alphabet character"
                    .to_string(),
            ));
        };
        if (chunk_padding == 2 && b & 0x0f != 0) || (chunk_padding == 1 && c & 0x03 != 0) {
            return Err(PebbleError::Network(
                "Invalid Outlook attachment base64 content: non-canonical trailing bits"
                    .to_string(),
            ));
        }
        let n = ((a as u32) << 18) | ((b as u32) << 12) | ((c as u32) << 6) | d as u32;
        out.push((n >> 16) as u8);
        if chunk_padding < 2 {
            out.push((n >> 8) as u8);
        }
        if chunk_padding == 0 {
            out.push(n as u8);
        }
    }
    Ok(out)
}

fn guess_outlook_mime(filename: &str) -> &'static str {
    let ext = filename.rsplit('.').next().unwrap_or("").to_lowercase();
    match ext.as_str() {
        "pdf" => "application/pdf",
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "txt" => "text/plain",
        "html" | "htm" => "text/html",
        "zip" => "application/zip",
        "doc" => "application/msword",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xls" => "application/vnd.ms-excel",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "csv" => "text/csv",
        "eml" => "message/rfc822",
        _ => "application/octet-stream",
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

pub fn should_hide_outlook_folder(
    display_name: Option<&str>,
    well_known_name: Option<&str>,
) -> bool {
    let well_known = well_known_name.unwrap_or_default().trim().to_lowercase();
    if matches!(well_known.as_str(), "outbox" | "conversationhistory") {
        return true;
    }

    let display_name = display_name.unwrap_or_default().trim().to_lowercase();
    matches!(
        display_name.as_str(),
        "outbox" | "发件箱" | "conversation history" | "对话历史记录"
    )
}

fn graph_folder_to_folder(gf: &GraphFolder, account_id: &str) -> Folder {
    let role = gf
        .well_known_name
        .as_deref()
        .and_then(well_known_name_to_role)
        .or_else(|| {
            gf.display_name
                .as_deref()
                .and_then(crate::imap::detect_folder_role)
        });
    let is_system = role.is_some();
    let sort_order = crate::imap::folder_sort_order(&role);
    Folder {
        id: new_id(),
        account_id: account_id.to_string(),
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
        from: None,
        id: Some(gm.id.clone()),
        to,
        cc,
        bcc: vec![],
        subject: gm.subject.clone().unwrap_or_default(),
        body_text,
        body_html,
        in_reply_to: None,
        attachment_paths: Vec::new(),
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
    let days = 365 * y + y / 4 - y / 100 + y / 400 + (m * 306 + 5) / 10 + day - 1 - 719468; // days from 0000-03-01 to 1970-01-01
    Some(days * 86400 + hour * 3600 + min * 60 + sec)
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn complete_delta_message(id: &str) -> serde_json::Value {
        serde_json::json!({
            "id": id,
            "subject": "Preserved subject",
            "bodyPreview": "Preserved preview",
            "body": {"contentType": "html", "content": "<p>Preserved body</p>"},
            "from": {"emailAddress": {"name": "Alice", "address": "alice@example.com"}},
            "toRecipients": [{"emailAddress": {"name": null, "address": "to@example.com"}}],
            "ccRecipients": [],
            "isRead": false,
            "flag": {"flagStatus": "flagged"},
            "isDraft": false,
            "receivedDateTime": "2024-01-15T10:30:00Z",
            "internetMessageId": "<preserved@example.com>",
            "conversationId": "conversation-1",
            "hasAttachments": true
        })
    }

    fn delta_list(values: Vec<serde_json::Value>) -> GraphDeltaList {
        serde_json::from_value(serde_json::json!({
            "value": values,
            "@odata.deltaLink": "https://graph.microsoft.com/v1.0/me/mailFolders/inbox/messages/delta?$deltatoken=next"
        }))
        .unwrap()
    }

    #[tokio::test]
    async fn thin_read_delta_fetches_complete_message_and_uses_its_latest_state() {
        let list = delta_list(vec![serde_json::json!({"id": "message-1", "isRead": true})]);
        let mut requested = Vec::new();
        let page = resolve_outlook_delta_page(list, "account-1", |id| {
            requested.push(id.clone());
            std::future::ready(Ok(Some(complete_delta_message(&id))))
        })
        .await
        .unwrap();

        let message = &page.messages[0];
        assert_eq!(message.subject, "Preserved subject");
        assert_eq!(message.body_html_raw, "<p>Preserved body</p>");
        assert_eq!(message.snippet, "Preserved preview");
        assert_eq!(message.from_address, "alice@example.com");
        assert_eq!(message.to_list[0].address, "to@example.com");
        assert_eq!(message.date, 1705314600);
        assert_eq!(message.thread_id.as_deref(), Some("conversation-1"));
        assert!(!message.is_read, "the full fetch is newer than the delta");
        assert!(message.is_starred);
        assert!(message.has_attachments);
        assert_eq!(requested, ["message-1"]);
    }

    #[tokio::test]
    async fn delta_hydrates_when_any_persisted_field_is_absent() {
        for field in [
            "subject",
            "bodyPreview",
            "body",
            "from",
            "toRecipients",
            "ccRecipients",
            "isRead",
            "flag",
            "isDraft",
            "receivedDateTime",
            "internetMessageId",
            "conversationId",
            "hasAttachments",
        ] {
            let mut partial = complete_delta_message("message-1");
            partial.as_object_mut().unwrap().remove(field);
            let mut requested = Vec::new();
            let page = resolve_outlook_delta_page(delta_list(vec![partial]), "account-1", |id| {
                requested.push(id.clone());
                std::future::ready(Ok(Some(complete_delta_message(&id))))
            })
            .await
            .unwrap();
            assert_eq!(requested, ["message-1"], "missing {field}");
            assert_eq!(page.messages[0].subject, "Preserved subject");
        }
    }

    #[tokio::test]
    async fn delta_hydrates_when_nested_content_or_metadata_is_absent() {
        for (parent, field) in [
            ("/body", "content"),
            ("/body", "contentType"),
            ("/flag", "flagStatus"),
            ("/from/emailAddress", "name"),
            ("/from/emailAddress", "address"),
            ("/toRecipients/0/emailAddress", "name"),
            ("/toRecipients/0/emailAddress", "address"),
        ] {
            let mut partial = complete_delta_message("message-1");
            partial
                .pointer_mut(parent)
                .unwrap()
                .as_object_mut()
                .unwrap()
                .remove(field);
            let mut requested = Vec::new();
            resolve_outlook_delta_page(delta_list(vec![partial]), "account-1", |id| {
                requested.push(id.clone());
                std::future::ready(Ok(Some(complete_delta_message(&id))))
            })
            .await
            .unwrap();
            assert_eq!(requested, ["message-1"], "missing {parent}/{field}");
        }
    }

    #[tokio::test]
    async fn complete_delta_accepts_explicit_empty_content_and_nullable_metadata_without_fetching()
    {
        let mut complete = complete_delta_message("message-1");
        complete["subject"] = serde_json::json!("");
        complete["bodyPreview"] = serde_json::json!("");
        complete["body"]["content"] = serde_json::json!("");
        complete["from"] = serde_json::Value::Null;
        complete["toRecipients"] = serde_json::json!([]);
        complete["internetMessageId"] = serde_json::Value::Null;
        complete["conversationId"] = serde_json::Value::Null;
        complete["hasAttachments"] = serde_json::json!(false);
        let page = resolve_outlook_delta_page(delta_list(vec![complete]), "account-1", |_| async {
            panic!("a complete delta must not trigger an additional request")
        })
        .await
        .unwrap();

        assert_eq!(page.messages.len(), 1);
        assert_eq!(page.messages[0].subject, "");
        assert_eq!(page.messages[0].body_html_raw, "");
        assert!(page.messages[0].to_list.is_empty());
        assert!(!page.messages[0].has_attachments);
    }

    #[tokio::test]
    async fn delta_tombstones_do_not_fetch_message_content() {
        let page = resolve_outlook_delta_page(
            delta_list(vec![
                serde_json::json!({"id": "deleted-1", "@removed": {"reason": "deleted"}}),
            ]),
            "account-1",
            |_| async { panic!("a tombstone must not trigger a content request") },
        )
        .await
        .unwrap();
        assert!(page.messages.is_empty());
        assert_eq!(page.deleted_remote_ids, ["deleted-1"]);
    }

    #[tokio::test]
    async fn delta_treats_message_removed_before_hydration_as_deleted() {
        let page = resolve_outlook_delta_page(
            delta_list(vec![serde_json::json!({"id": "gone-1", "isRead": true})]),
            "account-1",
            |_| async { Ok(None) },
        )
        .await
        .unwrap();
        assert!(page.messages.is_empty());
        assert_eq!(page.deleted_remote_ids, ["gone-1"]);
        assert!(page.delta_link.is_some());
    }

    #[tokio::test]
    async fn delta_hydration_failure_returns_no_page_to_persist() {
        let result = resolve_outlook_delta_page(
            delta_list(vec![
                complete_delta_message("complete-1"),
                serde_json::json!({"id": "thin-1", "isRead": true}),
            ]),
            "account-1",
            |_| async { Err(PebbleError::Network("Graph unavailable".to_string())) },
        )
        .await;
        assert!(
            result.is_err(),
            "failed hydration must prevent committing a delta cursor"
        );
    }

    #[tokio::test]
    async fn delta_rejects_incomplete_mismatched_and_removed_hydration_responses() {
        for invalid in [
            serde_json::json!({"id": "message-1", "isRead": true}),
            complete_delta_message("different-message"),
            serde_json::json!({"id": "message-1", "@removed": {"reason": "deleted"}}),
        ] {
            let result = resolve_outlook_delta_page(
                delta_list(vec![serde_json::json!({"id": "message-1", "isRead": true})]),
                "account-1",
                |_| std::future::ready(Ok(Some(invalid.clone()))),
            )
            .await;
            assert!(
                result.is_err(),
                "invalid full-message response was accepted: {invalid}"
            );
        }
    }
    use std::collections::VecDeque;

    #[test]
    fn outlook_attachment_base64_rejects_invalid_standard_alphabet_character() {
        assert!(base64_standard_decode_outlook("TQ$=").is_err());
        assert_eq!(
            base64_standard_decode_outlook("TQ==").unwrap(),
            b"M".to_vec()
        );
    }

    #[test]
    fn outlook_attachment_base64_rejects_invalid_padding() {
        assert!(base64_standard_decode_outlook("TQ=").is_err());
        assert!(base64_standard_decode_outlook("TR==").is_err());
    }

    #[test]
    fn outlook_attachment_base64_rejects_truncated_quartet() {
        assert!(base64_standard_decode_outlook("TQ").is_err());
    }

    #[test]
    fn outlook_file_attachment_without_content_bytes_is_an_error() {
        let page = graph_attachment_page(serde_json::json!({
            "value": [{
                "id": "file-1",
                "name": "missing.txt",
                "@odata.type": "#microsoft.graph.fileAttachment"
            }]
        }));

        let error = graph_attachment_items_to_data(page.value).unwrap_err();
        assert!(error.to_string().contains("contentBytes"));
    }

    #[test]
    fn outlook_non_file_attachment_without_content_bytes_is_skipped() {
        let page = graph_attachment_page(serde_json::json!({
            "value": [{
                "id": "reference-1",
                "name": "link",
                "@odata.type": "#microsoft.graph.referenceAttachment"
            }]
        }));

        assert!(graph_attachment_items_to_data(page.value)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn outlook_delta_gone_and_sync_state_not_found_are_dedicated_cursor_expiry_errors() {
        assert!(matches!(
            outlook_delta_response_error(reqwest::StatusCode::GONE, "gone"),
            PebbleError::SyncCursorExpired(_)
        ));
        assert!(matches!(
            outlook_delta_response_error(
                reqwest::StatusCode::BAD_REQUEST,
                r#"{"error":{"code":"syncStateNotFound"}}"#,
            ),
            PebbleError::SyncCursorExpired(_)
        ));
        assert!(matches!(
            outlook_delta_response_error(reqwest::StatusCode::BAD_REQUEST, "other"),
            PebbleError::Network(_)
        ));
    }

    #[test]
    fn outlook_delete_draft_treats_not_found_as_idempotent_success() {
        assert!(outlook_delete_draft_status_result(reqwest::StatusCode::NO_CONTENT, "").is_ok());
        assert!(outlook_delete_draft_status_result(reqwest::StatusCode::NOT_FOUND, "gone").is_ok());
        assert!(outlook_delete_draft_status_result(
            reqwest::StatusCode::INTERNAL_SERVER_ERROR,
            "failed"
        )
        .is_err());
    }

    #[derive(Default)]
    struct FakeGraphAttachmentPageOperations {
        pages: VecDeque<GraphAttachmentList>,
        requested_urls: Vec<String>,
    }

    #[async_trait::async_trait]
    impl GraphAttachmentPageOperations for FakeGraphAttachmentPageOperations {
        async fn fetch_attachment_page(&mut self, url: &str) -> Result<GraphAttachmentList> {
            self.requested_urls.push(url.to_string());
            self.pages.pop_front().ok_or_else(|| {
                PebbleError::Network("unexpected attachment page request".to_string())
            })
        }
    }

    fn graph_attachment_page(value: serde_json::Value) -> GraphAttachmentList {
        serde_json::from_value(value).unwrap()
    }

    #[tokio::test]
    async fn graph_attachment_listing_follows_safe_next_link_across_two_pages() {
        let next_link =
            "https://graph.microsoft.com/v1.0/me/messages/draft-1/attachments?$skiptoken=next";
        let mut operations = FakeGraphAttachmentPageOperations {
            pages: VecDeque::from([
                graph_attachment_page(serde_json::json!({
                    "value": [{
                        "id": "old-1",
                        "name": "one.txt",
                        "@odata.type": "#microsoft.graph.fileAttachment"
                    }],
                    "@odata.nextLink": next_link
                })),
                graph_attachment_page(serde_json::json!({
                    "value": [{
                        "id": "old-2",
                        "name": "two.txt",
                        "@odata.type": "#microsoft.graph.fileAttachment"
                    }]
                })),
            ]),
            requested_urls: Vec::new(),
        };

        let items = collect_graph_attachment_pages(
            &mut operations,
            "https://graph.microsoft.com/v1.0/me/messages/draft-1/attachments",
        )
        .await
        .unwrap();

        assert_eq!(
            items.into_iter().map(|item| item.id).collect::<Vec<_>>(),
            vec!["old-1", "old-2"]
        );
        assert_eq!(operations.requested_urls.len(), 2);
        assert_eq!(operations.requested_urls[1], next_link);
    }

    #[tokio::test]
    async fn two_page_attachment_listing_deletes_every_old_file_during_replacement() {
        let mut page_operations = FakeGraphAttachmentPageOperations {
            pages: VecDeque::from([
                graph_attachment_page(serde_json::json!({
                    "value": [{
                        "id": "old-1",
                        "@odata.type": "#microsoft.graph.fileAttachment"
                    }],
                    "@odata.nextLink": "https://graph.microsoft.com/v1.0/me/messages/draft-1/attachments?$skiptoken=next"
                })),
                graph_attachment_page(serde_json::json!({
                    "value": [{
                        "id": "old-2",
                        "@odata.type": "#microsoft.graph.fileAttachment"
                    }]
                })),
            ]),
            requested_urls: Vec::new(),
        };
        let old_ids = collect_graph_attachment_pages(
            &mut page_operations,
            "https://graph.microsoft.com/v1.0/me/messages/draft-1/attachments",
        )
        .await
        .unwrap()
        .into_iter()
        .filter(is_graph_file_attachment)
        .map(|item| item.id)
        .collect::<Vec<_>>();
        let mut replacement_operations = FakeDraftAttachmentOperations::default();

        replace_draft_attachment_set(&mut replacement_operations, &[], &old_ids)
            .await
            .unwrap();

        assert_eq!(
            replacement_operations.events,
            vec!["delete:old-1", "delete:old-2"]
        );
    }

    #[tokio::test]
    async fn graph_attachment_listing_rejects_untrusted_next_link_before_sending_token() {
        for next_link in [
            "http://graph.microsoft.com/v1.0/me/messages/1/attachments?$skiptoken=secret",
            "https://graph.microsoft.com.evil.example/v1.0/me/messages/1/attachments?$skiptoken=secret",
            "https://evil.example/v1.0/me/messages/1/attachments?$skiptoken=secret",
            "https://graph.microsoft.com:444/v1.0/me/messages/1/attachments?$skiptoken=secret",
            "https://attacker@graph.microsoft.com/v1.0/me/messages/1/attachments?$skiptoken=secret",
            "https://graph.microsoft.com/v1.0/users/other/messages/1/attachments?$skiptoken=secret",
        ] {
            let mut operations = FakeGraphAttachmentPageOperations {
                pages: VecDeque::from([graph_attachment_page(serde_json::json!({
                    "value": [],
                    "@odata.nextLink": next_link
                }))]),
                requested_urls: Vec::new(),
            };

            let error = collect_graph_attachment_pages(
                &mut operations,
                "https://graph.microsoft.com/v1.0/me/messages/1/attachments",
            )
            .await
            .unwrap_err();

            assert!(error.to_string().contains("untrusted Graph attachment nextLink"));
            assert_eq!(operations.requested_urls.len(), 1);
        }
    }

    #[test]
    fn test_well_known_name_to_role_inbox() {
        assert_eq!(well_known_name_to_role("inbox"), Some(FolderRole::Inbox));
    }

    #[test]
    fn test_well_known_name_to_role_sent() {
        assert_eq!(well_known_name_to_role("sentitems"), Some(FolderRole::Sent));
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
        assert_eq!(well_known_name_to_role("SentItems"), Some(FolderRole::Sent));
        assert_eq!(well_known_name_to_role("JunkEmail"), Some(FolderRole::Spam));
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
    fn test_graph_file_attachment_from_path() {
        let path = std::env::temp_dir().join(format!("pebble-outlook-{}.txt", new_id()));
        std::fs::write(&path, b"hello").unwrap();
        let path_string = path.to_string_lossy().into_owned();

        let attachment = graph_file_attachment_from_path(&path_string).unwrap();
        let _ = std::fs::remove_file(path);

        assert_eq!(attachment.odata_type, "#microsoft.graph.fileAttachment");
        assert!(attachment.name.starts_with("pebble-outlook-"));
        assert_eq!(attachment.content_type, "text/plain");
        assert_eq!(attachment.content_bytes, "aGVsbG8=");
    }

    #[derive(Default)]
    struct FakeDraftAttachmentOperations {
        events: Vec<String>,
        upload_attempt: usize,
        fail_upload_at: Option<usize>,
        fail_delete_id: Option<String>,
    }

    #[async_trait::async_trait]
    impl DraftAttachmentOperations for FakeDraftAttachmentOperations {
        async fn upload(&mut self, attachment: &GraphFileAttachment) -> Result<String> {
            let attempt = self.upload_attempt;
            self.upload_attempt += 1;
            self.events.push(format!("upload:{}", attachment.name));
            if self.fail_upload_at == Some(attempt) {
                return Err(PebbleError::Network(format!(
                    "upload failed at attempt {attempt}"
                )));
            }
            Ok(format!("new-{attempt}"))
        }

        async fn delete(&mut self, attachment_id: &str) -> Result<()> {
            self.events.push(format!("delete:{attachment_id}"));
            if self.fail_delete_id.as_deref() == Some(attachment_id) {
                return Err(PebbleError::Network(format!(
                    "delete failed for {attachment_id}"
                )));
            }
            Ok(())
        }
    }

    fn fake_graph_attachment(name: &str) -> GraphFileAttachment {
        GraphFileAttachment {
            odata_type: "#microsoft.graph.fileAttachment".to_string(),
            name: name.to_string(),
            content_type: "text/plain".to_string(),
            content_bytes: "YQ==".to_string(),
        }
    }

    #[tokio::test]
    async fn draft_attachment_replacement_uploads_every_new_item_before_deleting_old_items() {
        let mut operations = FakeDraftAttachmentOperations::default();
        let new_attachments = vec![
            fake_graph_attachment("a.txt"),
            fake_graph_attachment("b.txt"),
        ];
        let old_ids = vec!["old-1".to_string(), "old-2".to_string()];

        replace_draft_attachment_set(&mut operations, &new_attachments, &old_ids)
            .await
            .unwrap();

        assert_eq!(
            operations.events,
            [
                "upload:a.txt",
                "upload:b.txt",
                "delete:old-1",
                "delete:old-2"
            ]
        );
    }

    #[tokio::test]
    async fn draft_attachment_upload_failure_rolls_back_new_items_without_deleting_old_items() {
        let mut operations = FakeDraftAttachmentOperations {
            fail_upload_at: Some(1),
            ..Default::default()
        };
        let new_attachments = vec![
            fake_graph_attachment("a.txt"),
            fake_graph_attachment("b.txt"),
        ];
        let old_ids = vec!["old-1".to_string()];

        let error = replace_draft_attachment_set(&mut operations, &new_attachments, &old_ids)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("upload failed at attempt 1"));
        assert_eq!(
            operations.events,
            ["upload:a.txt", "upload:b.txt", "delete:new-0"]
        );
    }

    #[tokio::test]
    async fn draft_attachment_old_delete_failure_is_returned() {
        let mut operations = FakeDraftAttachmentOperations {
            fail_delete_id: Some("old-2".to_string()),
            ..Default::default()
        };
        let new_attachments = vec![fake_graph_attachment("a.txt")];
        let old_ids = vec!["old-1".to_string(), "old-2".to_string()];

        let error = replace_draft_attachment_set(&mut operations, &new_attachments, &old_ids)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("delete failed for old-2"));
        assert_eq!(
            operations.events,
            ["upload:a.txt", "delete:old-1", "delete:old-2"]
        );
    }

    #[test]
    fn draft_attachment_preparation_rejects_a_missing_file() {
        let path = std::env::temp_dir().join(format!("pebble-outlook-missing-{}", new_id()));

        let error = graph_file_attachments_from_paths(&[path.to_string_lossy().into_owned()])
            .err()
            .expect("missing attachment must be rejected")
            .to_string();

        assert!(error.contains("Failed to read attachment"));
    }

    #[derive(Default)]
    struct FakeCreatedDraftCleanupOperations {
        deleted_draft_ids: Vec<String>,
        fail_delete: bool,
    }

    #[async_trait::async_trait]
    impl CreatedDraftCleanupOperations for FakeCreatedDraftCleanupOperations {
        async fn delete_created_draft(&mut self, draft_id: &str) -> Result<()> {
            self.deleted_draft_ids.push(draft_id.to_string());
            if self.fail_delete {
                return Err(PebbleError::Network("draft cleanup failed".to_string()));
            }
            Ok(())
        }
    }

    #[tokio::test]
    async fn created_draft_attachment_failure_deletes_the_new_draft_and_returns_original_error() {
        let mut cleanup = FakeCreatedDraftCleanupOperations::default();
        let attachment_result = Err(PebbleError::Network("attachment upload failed".to_string()));

        let error = finish_created_draft_attachments(&mut cleanup, "draft-1", attachment_result)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("attachment upload failed"));
        assert_eq!(cleanup.deleted_draft_ids, ["draft-1"]);
    }

    #[tokio::test]
    async fn created_draft_cleanup_failure_preserves_both_errors() {
        let mut cleanup = FakeCreatedDraftCleanupOperations {
            fail_delete: true,
            ..Default::default()
        };
        let attachment_result = Err(PebbleError::Network("attachment upload failed".to_string()));

        let error = finish_created_draft_attachments(&mut cleanup, "draft-1", attachment_result)
            .await
            .unwrap_err()
            .to_string();

        assert!(error.contains("attachment upload failed"));
        assert!(error.contains("draft cleanup failed"));
        assert_eq!(cleanup.deleted_draft_ids, ["draft-1"]);
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
        let folder = graph_folder_to_folder(&gf, "test-account-id");
        assert_eq!(folder.role, Some(FolderRole::Inbox));
        assert_eq!(folder.folder_type, FolderType::Folder);
        assert!(folder.is_system);
        assert_eq!(folder.remote_id, "AAMkAD");
        assert_eq!(folder.name, "Inbox");
        assert_eq!(folder.account_id, "test-account-id");
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
        let folder = graph_folder_to_folder(&gf, "acct-123");
        assert_eq!(folder.role, None);
        assert!(!folder.is_system);
        assert_eq!(folder.name, "My Folder");
        assert_eq!(folder.account_id, "acct-123");
    }

    #[test]
    fn test_graph_folder_to_folder_detects_localized_system_names() {
        let spam = GraphFolder {
            id: "junk-id".to_string(),
            display_name: Some("垃圾邮件".to_string()),
            total_item_count: Some(0),
            child_folder_count: Some(0),
            well_known_name: None,
        };
        let archive = GraphFolder {
            id: "archive-id".to_string(),
            display_name: Some("存档".to_string()),
            total_item_count: Some(0),
            child_folder_count: Some(0),
            well_known_name: None,
        };
        let conversation_history = GraphFolder {
            id: "conversation-history-id".to_string(),
            display_name: Some("对话历史记录".to_string()),
            total_item_count: Some(0),
            child_folder_count: Some(0),
            well_known_name: None,
        };

        let spam_folder = graph_folder_to_folder(&spam, "acct-123");
        let archive_folder = graph_folder_to_folder(&archive, "acct-123");
        let history_folder = graph_folder_to_folder(&conversation_history, "acct-123");

        assert_eq!(spam_folder.role, Some(FolderRole::Spam));
        assert!(spam_folder.is_system);
        assert_eq!(archive_folder.role, Some(FolderRole::Archive));
        assert!(archive_folder.is_system);
        assert_eq!(history_folder.role, None);
        assert!(!history_folder.is_system);
    }

    #[test]
    fn test_should_hide_outlook_folder_skips_non_mail_system_folders() {
        assert!(should_hide_outlook_folder(Some("Outbox"), Some("outbox")));
        assert!(should_hide_outlook_folder(Some("发件箱"), None));
        assert!(should_hide_outlook_folder(
            Some("Conversation History"),
            Some("conversationhistory")
        ));
        assert!(should_hide_outlook_folder(Some("对话历史记录"), None));

        assert!(!should_hide_outlook_folder(Some("Inbox"), Some("inbox")));
        assert!(!should_hide_outlook_folder(Some("垃圾邮件"), None));
        assert!(!should_hide_outlook_folder(Some("Project"), None));
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
            removed: None,
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
    fn outlook_delta_page_to_changes_preserves_messages_deletions_and_cursor() {
        let gm: GraphMessage = serde_json::from_value(complete_delta_message("message-1")).unwrap();
        let page = OutlookDeltaPage {
            messages: vec![OutlookProvider::graph_message_to_message(&gm, "account-1")],
            deleted_remote_ids: vec!["deleted-1".to_string()],
            next_link: None,
            delta_link: Some("delta-link".to_string()),
        };

        let changes = outlook_delta_page_to_changes(page);

        assert_eq!(changes.new_messages.len(), 1);
        assert_eq!(changes.new_messages[0].remote_id, "message-1");
        assert_eq!(changes.deleted, vec!["deleted-1".to_string()]);
        assert_eq!(changes.cursor.value, "delta-link");
    }

    #[test]
    fn graph_message_fetch_url_encodes_the_remote_id_as_one_path_segment() {
        let url = reqwest::Url::parse(&graph_message_fetch_url("AAMk/a?b#c+d=").unwrap()).unwrap();
        assert_eq!(url.host_str(), Some("graph.microsoft.com"));
        assert_eq!(url.path(), "/v1.0/me/messages/AAMk%2Fa%3Fb%23c+d=");
        assert_eq!(url.fragment(), None);
        assert_eq!(
            url.query_pairs().collect::<Vec<_>>(),
            [("$select".into(), GRAPH_MESSAGE_SELECT.into())]
        );
    }

    #[test]
    fn test_set_access_token() {
        let provider = OutlookProvider::new("initial".to_string(), "test-account".to_string());
        assert_eq!(provider.token(), "initial");
        provider.set_access_token("updated".to_string());
        assert_eq!(provider.token(), "updated");
    }
}
