use std::path::{Path, PathBuf};
use std::sync::Arc;

use pebble_core::{Message, Result, new_id, now_timestamp};
use pebble_store::Store;
use tokio::sync::{mpsc, watch};
use tracing::{debug, error, info, warn};

/// Structured error info emitted by the sync worker.
#[derive(Debug, Clone, serde::Serialize)]
pub struct SyncError {
    pub error_type: String,
    pub message: String,
    pub timestamp: u64,
}

use crate::parser::{AttachmentData, parse_raw_email};
use crate::provider::imap_provider::ImapMailProvider;
use crate::reconcile;
use crate::thread::compute_thread_id;

/// Sanitize a filename to prevent path traversal attacks.
/// Removes path separators, `..` sequences, and trims leading dots/spaces.
pub(crate) fn sanitize_filename(name: &str) -> String {
    fn is_windows_reserved(stem: &str) -> bool {
        let upper = stem.trim().to_ascii_uppercase();
        matches!(
            upper.as_str(),
            "CON"
                | "PRN"
                | "AUX"
                | "NUL"
                | "COM1"
                | "COM2"
                | "COM3"
                | "COM4"
                | "COM5"
                | "COM6"
                | "COM7"
                | "COM8"
                | "COM9"
                | "LPT1"
                | "LPT2"
                | "LPT3"
                | "LPT4"
                | "LPT5"
                | "LPT6"
                | "LPT7"
                | "LPT8"
                | "LPT9"
        )
    }

    // Take only the last component if there are path separators
    let base = name
        .rsplit(|c: char| c == '/' || c == '\\')
        .next()
        .unwrap_or(name);
    // Reject if the component is exactly ".."
    if base == ".." || base == "." {
        return "unnamed_attachment".to_string();
    }
    // Remove `..` sequences repeatedly until none remain, then strip unsafe chars.
    let mut cleaned = base.to_string();
    while cleaned.contains("..") {
        cleaned = cleaned.replace("..", ".");
    }
    let sanitized: String = cleaned
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
            _ => c,
        })
        .filter(|c| !c.is_control())
        .collect();

    // Windows disallows names ending with dots/spaces and hidden/path-like prefixes are unsafe.
    let trimmed = sanitized.trim().trim_matches(|c: char| c == '.' || c == ' ');
    if trimmed.is_empty() {
        return "unnamed_attachment".to_string();
    }

    let stem = trimmed.split('.').next().unwrap_or(trimmed);
    if is_windows_reserved(stem) {
        return "unnamed_attachment".to_string();
    }

    trimmed.to_string()
}

pub(crate) fn persist_message_attachments(
    store: &Store,
    attachments_root: &Path,
    message_id: &str,
    attachments: &[AttachmentData],
) {
    for att_data in attachments {
        let att_dir = attachments_root.join(message_id);
        if std::fs::create_dir_all(&att_dir).is_err() {
            warn!("Failed to create attachment dir for message {}", message_id);
            continue;
        }

        let safe_filename = sanitize_filename(&att_data.meta.filename);
        if safe_filename.is_empty() {
            warn!("Attachment has empty filename after sanitization, skipping");
            continue;
        }

        let file_path = att_dir.join(&safe_filename);
        if std::fs::write(&file_path, &att_data.data).is_ok() {
            let attachment = pebble_core::Attachment {
                id: new_id(),
                message_id: message_id.to_string(),
                filename: att_data.meta.filename.clone(),
                mime_type: att_data.meta.mime_type.clone(),
                size: att_data.meta.size as i64,
                local_path: Some(file_path.to_string_lossy().to_string()),
                content_id: att_data.meta.content_id.clone(),
                is_inline: att_data.meta.is_inline,
            };
            if let Err(e) = store.insert_attachment(&attachment) {
                warn!("Failed to store attachment record: {}", e);
            }
        } else {
            warn!("Failed to write attachment file: {}", file_path.display());
        }
    }
}

// ---------------------------------------------------------------------------
// Sync cursor helpers
// ---------------------------------------------------------------------------

/// Parse a sync cursor string into its components.
/// Supports two formats:
/// - `"12345"` (UID only, backward compatible) -> `(Some(12345), None)`
/// - `"12345;modseq=67890"` -> `(Some(12345), Some(67890))`
fn parse_cursor(cursor: &str) -> (Option<u32>, Option<u64>) {
    let parts: Vec<&str> = cursor.splitn(2, ';').collect();
    let uid = parts.first().and_then(|s| s.parse::<u32>().ok());
    let modseq = parts.get(1).and_then(|s| {
        s.strip_prefix("modseq=")
            .and_then(|v| v.parse::<u64>().ok())
    });
    (uid, modseq)
}

/// Build a sync cursor string from UID and optional MODSEQ.
fn build_cursor(uid: u32, modseq: Option<u64>) -> String {
    match modseq {
        Some(m) => format!("{};modseq={}", uid, m),
        None => uid.to_string(),
    }
}

/// Extract the MODSEQ value from a cursor string, if present.
fn extract_modseq(cursor: &str) -> Option<u64> {
    parse_cursor(cursor).1
}

/// Configuration for the sync worker.
#[derive(Debug, Clone)]
pub struct SyncConfig {
    /// How often to poll for new messages, in seconds.
    pub poll_interval_secs: u64,
    /// How often to do a full reconcile, in seconds.
    pub reconcile_interval_secs: u64,
    /// How many messages to fetch on initial sync.
    pub initial_fetch_limit: u32,
}

impl Default for SyncConfig {
    fn default() -> Self {
        Self {
            poll_interval_secs: 15,
            reconcile_interval_secs: 900,
            initial_fetch_limit: 200,
        }
    }
}

/// A newly stored message along with the folder IDs it belongs to.
/// Emitted via `message_tx` so callers (e.g. the search indexer) can react.
#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub message: Message,
    pub folder_ids: Vec<String>,
}

/// A worker that syncs mail for one account.
pub struct SyncWorker {
    account_id: String,
    provider: Arc<ImapMailProvider>,
    store: Arc<Store>,
    stop_rx: watch::Receiver<bool>,
    attachments_dir: PathBuf,
    error_tx: Option<mpsc::UnboundedSender<SyncError>>,
    message_tx: Option<mpsc::UnboundedSender<StoredMessage>>,
}

impl SyncWorker {
    /// Create a new sync worker.
    pub fn new(
        account_id: impl Into<String>,
        provider: Arc<ImapMailProvider>,
        store: Arc<Store>,
        stop_rx: watch::Receiver<bool>,
        attachments_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            account_id: account_id.into(),
            provider,
            store,
            stop_rx,
            attachments_dir: attachments_dir.into(),
            error_tx: None,
            message_tx: None,
        }
    }

    /// Set the error channel for emitting structured sync errors.
    pub fn with_error_tx(mut self, tx: mpsc::UnboundedSender<SyncError>) -> Self {
        self.error_tx = Some(tx);
        self
    }

    /// Set the channel for emitting newly stored messages (used for search indexing).
    pub fn with_message_tx(mut self, tx: mpsc::UnboundedSender<StoredMessage>) -> Self {
        self.message_tx = Some(tx);
        self
    }

    /// Emit a structured error through the error channel.
    fn emit_error(&self, error_type: &str, message: &str) {
        if let Some(tx) = &self.error_tx {
            let _ = tx.send(SyncError {
                error_type: error_type.to_string(),
                message: message.to_string(),
                timestamp: now_timestamp() as u64,
            });
        }
    }

    /// Perform the initial full sync: list folders and fetch all of them.
    pub async fn initial_sync(&self) -> Result<()> {
        info!("Starting initial sync for account {}", self.account_id);

        let folders = self.provider.inner().list_folders(&self.account_id).await?;

        for folder in &folders {
            // Upsert folder into store
            // Ignore "already exists" errors
            let _ = self.store.insert_folder(folder);
        }

        // Ensure an Archive folder exists locally (even if the IMAP server doesn't have one)
        let has_archive = folders.iter().any(|f| f.role == Some(pebble_core::FolderRole::Archive));
        if !has_archive {
            let archive = pebble_core::Folder {
                id: new_id(),
                account_id: self.account_id.clone(),
                remote_id: "__local_archive__".to_string(),
                name: "Archive".to_string(),
                folder_type: pebble_core::FolderType::Folder,
                role: Some(pebble_core::FolderRole::Archive),
                parent_id: None,
                color: None,
                is_system: true,
                sort_order: 3,
            };
            let _ = self.store.insert_folder(&archive);
            info!("Created local archive folder for account {}", self.account_id);
        }

        // Sync all folders, prioritising Inbox first
        let mut ordered: Vec<&pebble_core::Folder> = Vec::with_capacity(folders.len());
        if let Some(inbox) = folders.iter().find(|f| f.role == Some(pebble_core::FolderRole::Inbox)) {
            ordered.push(inbox);
        }
        for f in &folders {
            if f.role != Some(pebble_core::FolderRole::Inbox) {
                ordered.push(f);
            }
        }

        // Use persisted cursor (inbox-level) for the inbox; other folders use their own max UID
        let cursor = self
            .store
            .get_sync_cursor(&self.account_id)
            .ok()
            .flatten();
        let (inbox_since_uid, prev_modseq) = cursor
            .as_deref()
            .map(parse_cursor)
            .unwrap_or((None, None));

        for folder in &ordered {
            let is_inbox = folder.role == Some(pebble_core::FolderRole::Inbox);

            let since_uid = if is_inbox {
                inbox_since_uid
            } else {
                // For non-inbox folders, resume from the last known UID
                self.store
                    .get_max_remote_id(&self.account_id, &folder.id)
                    .ok()
                    .flatten()
                    .and_then(|s| s.parse::<u32>().ok())
            };

            let limit = if since_uid.is_some() { 50 } else { 200 };
            match self.sync_folder(folder, since_uid, limit).await {
                Ok(count) => {
                    if count > 0 {
                        info!("Initial sync: fetched {} messages from {}", count, folder.name);
                    }
                    // Update inbox cursor
                    if is_inbox && count > 0 {
                        if let Ok(Some(max_uid_str)) =
                            self.store.get_max_remote_id(&self.account_id, &folder.id)
                        {
                            if let Ok(max_uid) = max_uid_str.parse::<u32>() {
                                let new_cursor = build_cursor(max_uid, prev_modseq);
                                let _ = self.store.set_sync_cursor(&self.account_id, &new_cursor);
                            }
                        }
                    }
                }
                Err(e) => warn!("Initial sync folder {} failed: {}", folder.name, e),
            }
        }

        Ok(())
    }

    /// Check if a folder is local-only (not backed by IMAP).
    fn is_local_folder(folder: &pebble_core::Folder) -> bool {
        folder.remote_id.starts_with("__local_")
    }

    /// Sync a folder: fetch raw messages, parse, compute threads, store.
    /// Returns the number of new messages stored.
    pub async fn sync_folder(
        &self,
        folder: &pebble_core::Folder,
        since_uid: Option<u32>,
        limit: u32,
    ) -> Result<u32> {
        // Skip local-only folders (not backed by IMAP)
        if Self::is_local_folder(folder) {
            return Ok(0);
        }

        let raw_messages = self
            .provider
            .inner()
            .fetch_messages_raw(&folder.remote_id, since_uid, limit)
            .await?;

        if raw_messages.is_empty() {
            return Ok(0);
        }

        // Load existing thread mappings for thread ID computation.
        // This is mutable so we can extend it as we store new messages within the batch,
        // ensuring intra-batch replies find their parent's thread.
        let mut thread_mappings = self
            .store
            .get_thread_mappings(&self.account_id)
            .unwrap_or_default();

        // Bulk-check which UIDs already exist to avoid N+1 queries
        let all_remote_ids: Vec<String> = raw_messages.iter().map(|(uid, _)| uid.to_string()).collect();
        let existing_ids = self
            .store
            .get_existing_remote_ids(&self.account_id, &all_remote_ids)
            .unwrap_or_default();

        let mut stored_count = 0u32;

        for (uid, raw) in raw_messages {
            let remote_id = uid.to_string();

            // Skip if already stored (checked via bulk query above)
            if existing_ids.contains(&remote_id) {
                debug!("Message UID {} already stored, skipping", uid);
                continue;
            }

            let parsed = match parse_raw_email(&raw) {
                Ok(p) => p,
                Err(e) => {
                    warn!("Failed to parse message UID {}: {}", uid, e);
                    continue;
                }
            };

            let now = now_timestamp();

            // Build a temporary message to compute thread_id
            let mut msg = Message {
                id: new_id(),
                account_id: self.account_id.clone(),
                remote_id: remote_id.clone(),
                message_id_header: parsed.message_id_header.clone(),
                in_reply_to: parsed.in_reply_to.clone(),
                references_header: parsed.references_header.clone(),
                thread_id: None,
                subject: parsed.subject.clone(),
                snippet: parsed.snippet.clone(),
                from_address: parsed.from_address.clone(),
                from_name: parsed.from_name.clone(),
                to_list: parsed.to_list.clone(),
                cc_list: parsed.cc_list.clone(),
                bcc_list: parsed.bcc_list.clone(),
                body_text: parsed.body_text.clone(),
                body_html_raw: parsed.body_html.clone(),
                has_attachments: parsed.has_attachments,
                is_read: false,
                is_starred: false,
                is_draft: false,
                date: parsed.date,
                remote_version: None,
                is_deleted: false,
                deleted_at: None,
                created_at: now,
                updated_at: now,
            };

            let thread_id = compute_thread_id(&msg, &thread_mappings);
            msg.thread_id = Some(thread_id);

            match self
                .store
                .insert_message(&msg, std::slice::from_ref(&folder.id))
            {
                Ok(()) => {
                    stored_count += 1;
                    // Update in-memory thread mappings so later messages in this batch
                    // can find this message as a thread parent.
                    if let (Some(mid), Some(tid)) = (&msg.message_id_header, &msg.thread_id) {
                        thread_mappings.push((mid.clone(), tid.clone()));
                    }

                    // Notify listeners (e.g. search indexer) about the new message
                    if let Some(tx) = &self.message_tx {
                        let _ = tx.send(StoredMessage {
                            message: msg.clone(),
                            folder_ids: vec![folder.id.clone()],
                        });
                    }

                    persist_message_attachments(
                        &self.store,
                        &self.attachments_dir,
                        &msg.id,
                        &parsed.attachments,
                    );
                }
                Err(e) => {
                    error!("Failed to store message UID {}: {}", uid, e);
                }
            }
        }

        Ok(stored_count)
    }

    /// Poll all folders for new messages since the highest known UID.
    pub async fn poll_new_messages(&self) -> Result<()> {
        let folders = self.store.list_folders(&self.account_id)?;
        if folders.is_empty() {
            return Ok(());
        }

        // Preserve existing modseq value when updating the cursor
        let prev_modseq = self
            .store
            .get_sync_cursor(&self.account_id)
            .ok()
            .flatten()
            .and_then(|s| extract_modseq(&s));

        for folder in &folders {
            let since_uid = self
                .store
                .get_max_remote_id(&self.account_id, &folder.id)
                .ok()
                .flatten()
                .and_then(|s| s.parse::<u32>().ok());

            match self.sync_folder(folder, since_uid, 50).await {
                Ok(count) if count > 0 => {
                    info!("Polled {} new messages from {} for account {}", count, folder.name, self.account_id);
                    // Update inbox cursor
                    if folder.role == Some(pebble_core::FolderRole::Inbox) {
                        if let Ok(Some(max_uid_str)) =
                            self.store.get_max_remote_id(&self.account_id, &folder.id)
                        {
                            if let Ok(max_uid) = max_uid_str.parse::<u32>() {
                                let new_cursor = build_cursor(max_uid, prev_modseq);
                                let _ = self.store.set_sync_cursor(&self.account_id, &new_cursor);
                            }
                        }
                    }
                }
                Ok(_) => {}
                Err(e) => warn!("Poll failed for folder {} account {}: {}", folder.name, self.account_id, e),
            }
        }

        Ok(())
    }

    /// Reconcile a folder: detect flag changes and server-side deletions.
    ///
    /// When the server supports CONDSTORE (RFC 4551), this method first checks
    /// the mailbox HIGHESTMODSEQ against the stored value. If they match, no
    /// flags have changed and the expensive full flag fetch is skipped entirely.
    /// When they differ (or on the first sync), a full flag fetch is performed
    /// and the new MODSEQ is persisted in the cursor.
    async fn reconcile_folder(&self, folder: &pebble_core::Folder) -> Result<()> {
        // Skip local-only folders
        if Self::is_local_folder(folder) {
            return Ok(());
        }

        // Step 1: Get local state
        let local_state = self
            .store
            .list_remote_ids_by_folder(&self.account_id, &folder.id)?;
        if local_state.is_empty() {
            return Ok(());
        }

        // Read stored MODSEQ from cursor
        let stored_modseq = self
            .store
            .get_sync_cursor(&self.account_id)
            .ok()
            .flatten()
            .and_then(|s| extract_modseq(&s))
            .unwrap_or(0);

        // Step 2: Try CONDSTORE optimisation — check HIGHESTMODSEQ
        let condstore_skip = match self
            .provider
            .inner()
            .get_highest_modseq(&folder.remote_id)
            .await
        {
            Ok(Some(server_modseq)) => {
                if reconcile::can_skip_reconcile(stored_modseq, server_modseq) {
                    debug!(
                        "CONDSTORE: HIGHESTMODSEQ unchanged ({}), skipping flag reconcile for {}",
                        server_modseq, folder.name
                    );
                    true
                } else {
                    debug!(
                        "CONDSTORE: HIGHESTMODSEQ changed ({} -> {}), doing full flag reconcile for {}",
                        stored_modseq, server_modseq, folder.name
                    );
                    false
                }
            }
            Ok(None) => {
                // Server does not support CONDSTORE — fall through to full reconcile
                false
            }
            Err(e) => {
                warn!("CONDSTORE HIGHESTMODSEQ check failed for {}: {}", folder.name, e);
                false
            }
        };

        if !condstore_skip {
            // Step 3: Fetch remote flags (with MODSEQ when possible)
            let uids: Vec<u32> = local_state
                .iter()
                .filter_map(|(_, remote_id, _, _)| remote_id.parse().ok())
                .collect();

            // Try fetching flags with MODSEQ to update the stored value
            let (remote_flags, new_modseq) = match self
                .provider
                .inner()
                .fetch_flags_with_modseq(&folder.remote_id, &uids)
                .await
            {
                Ok(result) => result,
                Err(e) => {
                    // Fall back to plain flag fetch if MODSEQ fetch fails
                    warn!("fetch_flags_with_modseq failed, falling back: {}", e);
                    let flags = self
                        .provider
                        .inner()
                        .fetch_flags(&folder.remote_id, &uids)
                        .await?;
                    (flags, 0)
                }
            };

            // Step 4: Compute and apply flag diff
            let flag_changes = reconcile::compute_flag_diff(&local_state, &remote_flags);
            if !flag_changes.is_empty() {
                info!(
                    "Applying {} flag changes for folder {}",
                    flag_changes.len(),
                    folder.name
                );
                self.store.bulk_update_flags(&flag_changes)?;
            }

            // Step 5: Persist new MODSEQ in cursor if we got one
            if new_modseq > 0 {
                self.update_cursor_modseq(new_modseq);
            }
        }

        // Step 6: Detect deletions (always — CONDSTORE doesn't cover expunges)
        let server_uids = self
            .provider
            .inner()
            .fetch_all_uids(&folder.remote_id)
            .await?;
        let local_remote_ids: Vec<(String, String)> = local_state
            .iter()
            .map(|(id, rid, _, _)| (id.clone(), rid.clone()))
            .collect();
        let deleted = reconcile::detect_deletions(&local_remote_ids, &server_uids);
        if !deleted.is_empty() {
            info!(
                "Soft-deleting {} server-removed messages from {}",
                deleted.len(),
                folder.name
            );
            self.store.bulk_soft_delete(&deleted)?;
        }

        Ok(())
    }

    /// Update the MODSEQ portion of the sync cursor without changing the UID part.
    fn update_cursor_modseq(&self, new_modseq: u64) {
        let cursor = self
            .store
            .get_sync_cursor(&self.account_id)
            .ok()
            .flatten();
        let (uid, _old_modseq) = cursor
            .as_deref()
            .map(parse_cursor)
            .unwrap_or((None, None));
        if let Some(uid_val) = uid {
            let new_cursor = build_cursor(uid_val, Some(new_modseq));
            let _ = self.store.set_sync_cursor(&self.account_id, &new_cursor);
        }
    }

    /// Run the sync worker loop until the stop signal is received.
    pub async fn run(&self, config: SyncConfig) {
        let poll_interval = tokio::time::Duration::from_secs(config.poll_interval_secs);
        let reconcile_interval =
            tokio::time::Duration::from_secs(config.reconcile_interval_secs);

        let mut poll_ticker = tokio::time::interval(poll_interval);
        let mut reconcile_ticker = tokio::time::interval(reconcile_interval);

        // Connect and do initial sync
        if let Err(e) = self.provider.connect().await {
            error!("Failed to connect for account {}: {}", self.account_id, e);
            self.emit_error("connection", &format!("Failed to connect: {}", e));
            return;
        }

        if let Err(e) = self.initial_sync().await {
            error!("Initial sync failed for account {}: {}", self.account_id, e);
            self.emit_error("sync", &format!("Initial sync failed: {}", e));
        }

        let supports_idle = self.provider.inner().supports_idle().await;
        if supports_idle {
            info!("IMAP IDLE supported for account {}", self.account_id);
        } else {
            info!("IMAP IDLE not supported for account {}, using polling", self.account_id);
        }

        let supports_condstore = self.provider.inner().supports_condstore().await;
        if supports_condstore {
            info!("CONDSTORE supported for account {}", self.account_id);
        } else {
            debug!("CONDSTORE not supported for account {}", self.account_id);
        }

        let mut stop_rx = self.stop_rx.clone();
        let mut last_exists: u32 = 0;

        loop {
            tokio::select! {
                _ = poll_ticker.tick() => {
                    // Quick check if mailbox has changes before doing full poll
                    let folders = match self.store.list_folders(&self.account_id) {
                        Ok(f) => f,
                        Err(_) => continue,
                    };
                    if let Some(inbox) = folders.iter().find(|f| f.role == Some(pebble_core::FolderRole::Inbox)) {
                        match crate::idle::check_for_changes_with_idle(self.provider.inner(), &inbox.remote_id, &mut last_exists, supports_idle).await {
                            Ok(crate::idle::IdleEvent::NewMail) => {
                                if let Err(e) = self.poll_new_messages().await {
                                    warn!("Poll error for account {}: {}", self.account_id, e);
                                    self.emit_error("poll", &format!("Poll error: {}", e));
                                }
                            }
                            Ok(crate::idle::IdleEvent::Timeout) => {
                                debug!("No changes detected for account {}", self.account_id);
                            }
                            Ok(crate::idle::IdleEvent::Error(e)) => {
                                warn!("IDLE check error for account {}: {}", self.account_id, e);
                                self.emit_error("idle", &format!("IDLE check error: {}", e));
                                // Fall back to regular poll on error
                                if let Err(e) = self.poll_new_messages().await {
                                    warn!("Poll error for account {}: {}", self.account_id, e);
                                    self.emit_error("poll", &format!("Poll error: {}", e));
                                }
                            }
                            Err(e) => {
                                warn!("IDLE check failed for account {}: {}", self.account_id, e);
                                self.emit_error("idle", &format!("IDLE check failed: {}", e));
                            }
                        }
                    }
                }
                _ = reconcile_ticker.tick() => {
                    // Full reconcile: poll new messages + flag diff + deletion detection
                    if let Err(e) = self.poll_new_messages().await {
                        warn!("Reconcile poll error for account {}: {}", self.account_id, e);
                        self.emit_error("reconcile", &format!("Reconcile poll error: {}", e));
                    }
                    let folders = match self.store.list_folders(&self.account_id) {
                        Ok(f) => f,
                        Err(e) => {
                            warn!("Reconcile list folders error: {}", e);
                            self.emit_error("reconcile", &format!("List folders error: {}", e));
                            continue;
                        }
                    };
                    for folder in &folders {
                        if let Err(e) = self.reconcile_folder(folder).await {
                            warn!("Reconcile folder {} error: {}", folder.name, e);
                            self.emit_error("reconcile", &format!("Reconcile {} error: {}", folder.name, e));
                        }
                    }
                }
                Ok(()) = stop_rx.changed() => {
                    if *stop_rx.borrow() {
                        info!("Sync worker stopping for account {}", self.account_id);
                        let _ = self.provider.disconnect().await;
                        break;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_cursor_uid_only() {
        let (uid, modseq) = parse_cursor("12345");
        assert_eq!(uid, Some(12345));
        assert_eq!(modseq, None);
    }

    #[test]
    fn test_parse_cursor_with_modseq() {
        let (uid, modseq) = parse_cursor("12345;modseq=67890");
        assert_eq!(uid, Some(12345));
        assert_eq!(modseq, Some(67890));
    }

    #[test]
    fn test_parse_cursor_invalid_uid() {
        let (uid, modseq) = parse_cursor("abc");
        assert_eq!(uid, None);
        assert_eq!(modseq, None);
    }

    #[test]
    fn test_parse_cursor_invalid_modseq() {
        let (uid, modseq) = parse_cursor("123;modseq=abc");
        assert_eq!(uid, Some(123));
        assert_eq!(modseq, None);
    }

    #[test]
    fn test_sanitize_filename_rejects_windows_reserved_names() {
        assert_eq!(sanitize_filename("CON.txt"), "unnamed_attachment");
        assert_eq!(sanitize_filename("aux"), "unnamed_attachment");
        assert_eq!(sanitize_filename("LPT1.log"), "unnamed_attachment");
    }

    #[test]
    fn test_sanitize_filename_removes_windows_unsafe_characters() {
        assert_eq!(
            sanitize_filename("quarterly:report*final?.pdf"),
            "quarterly_report_final_.pdf",
        );
        assert_eq!(sanitize_filename("report. "), "report");
    }

    #[test]
    fn test_build_cursor_without_modseq() {
        assert_eq!(build_cursor(100, None), "100");
    }

    #[test]
    fn test_build_cursor_with_modseq() {
        assert_eq!(build_cursor(100, Some(200)), "100;modseq=200");
    }

    #[test]
    fn test_build_cursor_roundtrip() {
        let cursor = build_cursor(999, Some(12345));
        let (uid, modseq) = parse_cursor(&cursor);
        assert_eq!(uid, Some(999));
        assert_eq!(modseq, Some(12345));
    }

    #[test]
    fn test_extract_modseq_present() {
        assert_eq!(extract_modseq("100;modseq=500"), Some(500));
    }

    #[test]
    fn test_extract_modseq_absent() {
        assert_eq!(extract_modseq("100"), None);
    }
}
