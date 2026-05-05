use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::backoff::SyncBackoff;
use pebble_core::{new_id, now_timestamp, Message, PebbleError, Result};
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

#[derive(Debug, Clone, serde::Serialize)]
pub struct SyncProgress {
    pub account_id: String,
    pub status: String,
    pub phase: String,
    pub message: Option<String>,
}

impl SyncProgress {
    fn started(account_id: &str, phase: &str) -> Self {
        Self {
            account_id: account_id.to_string(),
            status: "started".to_string(),
            phase: phase.to_string(),
            message: None,
        }
    }

    fn completed(account_id: &str, phase: &str) -> Self {
        Self {
            account_id: account_id.to_string(),
            status: "completed".to_string(),
            phase: phase.to_string(),
            message: None,
        }
    }

    fn error(account_id: &str, phase: &str, message: &str) -> Self {
        Self {
            account_id: account_id.to_string(),
            status: "error".to_string(),
            phase: phase.to_string(),
            message: Some(message.to_string()),
        }
    }
}

use crate::parser::{parse_raw_email, AttachmentData, ParsedMessage};
use crate::provider::imap_provider::ImapMailProvider;
use crate::realtime_policy::{RealtimePollPolicy, RealtimeRuntimeState, SyncTrigger};
use crate::reconcile;
use crate::thread::compute_thread_id;

/// Collect all message-ID references (In-Reply-To + References) from a batch of
/// pre-parsed messages. Used to limit the thread-mappings query to only the IDs
/// that are actually needed by this batch.
fn collect_ref_ids_from_parsed(
    parsed_messages: &[(u32, pebble_core::Result<ParsedMessage>)],
) -> Vec<String> {
    let mut refs = std::collections::HashSet::new();
    for (_, result) in parsed_messages {
        if let Ok(parsed) = result {
            if let Some(irt) = &parsed.in_reply_to {
                for id in irt.split_whitespace() {
                    refs.insert(id.trim().to_string());
                }
            }
            if let Some(r) = &parsed.references_header {
                for id in r.split_whitespace() {
                    refs.insert(id.trim().to_string());
                }
            }
        }
    }
    refs.into_iter().collect()
}

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
    let base = name.rsplit(['/', '\\']).next().unwrap_or(name);
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
    let trimmed = sanitized
        .trim()
        .trim_matches(|c: char| c == '.' || c == ' ');
    if trimmed.is_empty() {
        return "unnamed_attachment".to_string();
    }

    let stem = trimmed.split('.').next().unwrap_or(trimmed);
    if is_windows_reserved(stem) {
        return "unnamed_attachment".to_string();
    }

    trimmed.to_string()
}

/// Write attachments to disk and record them in the store.
///
/// Takes ownership of `attachments` so that each buffer can be freed the
/// moment it has been flushed — we don't keep every attachment's bytes
/// live in memory until the whole function returns. Writes use a buffered
/// writer with 64 KiB chunks so the working set stays bounded.
pub(crate) fn persist_message_attachments(
    store: &Store,
    attachments_root: &Path,
    message_id: &str,
    attachments: Vec<AttachmentData>,
) {
    use std::io::Write;
    const CHUNK_SIZE: usize = 64 * 1024;

    for att_data in attachments.into_iter() {
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

        let mut file_path = att_dir.join(&safe_filename);
        let mut counter = 1u32;
        while file_path.exists() {
            let stem = std::path::Path::new(&safe_filename)
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy();
            let ext = std::path::Path::new(&safe_filename)
                .extension()
                .map(|e| format!(".{}", e.to_string_lossy()))
                .unwrap_or_default();
            file_path = att_dir.join(format!("{stem}_{counter}{ext}"));
            counter += 1;
        }
        let file = match std::fs::File::create(&file_path) {
            Ok(f) => f,
            Err(e) => {
                warn!(
                    "Failed to create attachment file {}: {}",
                    file_path.display(),
                    e
                );
                continue;
            }
        };
        let mut writer = std::io::BufWriter::with_capacity(CHUNK_SIZE, file);

        let AttachmentData { meta, data } = att_data;
        let mut write_ok = true;
        for chunk in data.chunks(CHUNK_SIZE) {
            if let Err(e) = writer.write_all(chunk) {
                warn!(
                    "Failed to write attachment file {}: {}",
                    file_path.display(),
                    e
                );
                write_ok = false;
                break;
            }
        }
        // Release the attachment buffer as soon as bytes are flushed to the
        // buffered writer, before we touch the store — callers often invoke us
        // in a tight loop where peak memory matters.
        drop(data);

        if !write_ok {
            let _ = std::fs::remove_file(&file_path);
            continue;
        }
        if let Err(e) = writer.flush() {
            warn!(
                "Failed to flush attachment file {}: {}",
                file_path.display(),
                e
            );
            let _ = std::fs::remove_file(&file_path);
            continue;
        }

        let attachment = pebble_core::Attachment {
            id: new_id(),
            message_id: message_id.to_string(),
            filename: meta.filename,
            mime_type: meta.mime_type,
            size: meta.size as i64,
            local_path: Some(file_path.to_string_lossy().to_string()),
            content_id: meta.content_id,
            is_inline: meta.is_inline,
        };
        if let Err(e) = store.insert_attachment(&attachment) {
            warn!("Failed to store attachment record: {}", e);
        }
    }
}

/// Async wrapper that offloads attachment I/O to a blocking thread.
pub(crate) async fn persist_message_attachments_async(
    store: Arc<Store>,
    attachments_root: PathBuf,
    message_id: String,
    attachments: Vec<AttachmentData>,
) {
    if attachments.is_empty() {
        return;
    }
    let _ = tokio::task::spawn_blocking(move || {
        persist_message_attachments(&store, &attachments_root, &message_id, attachments);
    })
    .await;
}

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
struct ImapFolderCursor {
    uidvalidity: Option<u64>,
    last_uid: Option<u32>,
    highest_modseq: Option<u64>,
}

fn parse_imap_folder_cursor(state: Option<&str>) -> ImapFolderCursor {
    match state {
        Some(raw) => serde_json::from_str(raw).unwrap_or_default(),
        None => ImapFolderCursor::default(),
    }
}

fn prepare_imap_folder_cursor_for_status(
    mut cursor: ImapFolderCursor,
    uidvalidity: Option<u64>,
    highest_modseq: Option<u64>,
) -> ImapFolderCursor {
    if let (Some(stored), Some(current)) = (cursor.uidvalidity, uidvalidity) {
        if stored != current {
            cursor.last_uid = None;
        }
    }
    if uidvalidity.is_some() {
        cursor.uidvalidity = uidvalidity;
    }
    if highest_modseq.is_some() {
        cursor.highest_modseq = highest_modseq;
    }
    cursor
}

fn serialize_imap_folder_cursor(cursor: &ImapFolderCursor) -> Option<String> {
    serde_json::to_string(cursor).ok()
}

fn can_advance_imap_folder_cursor(has_unresolved_failures: bool) -> bool {
    !has_unresolved_failures
}

fn should_run_imap_deletion_diff(_server_exists: u32, local_count: usize) -> bool {
    local_count > 0
}

fn can_seed_imap_polling_baseline_after_startup(initial_sync_succeeded: bool) -> bool {
    initial_sync_succeeded
}

fn can_refresh_imap_polling_baseline_after_idle_fallback(catch_up_succeeded: bool) -> bool {
    catch_up_succeeded
}

fn apply_local_inbox_uid_baseline(
    last_exists: &mut Option<crate::idle::MailboxUidState>,
    uidvalidity: Option<u64>,
    local_max_uid: Option<u32>,
    has_unresolved_failures: bool,
) -> bool {
    if has_unresolved_failures {
        return false;
    }
    *last_exists = Some(crate::idle::MailboxUidState {
        uidvalidity,
        highest_uid: local_max_uid.unwrap_or(0),
    });
    true
}

fn should_skip_missing_imap_mailbox_during_initial_sync(
    folder_role: Option<pebble_core::FolderRole>,
) -> bool {
    folder_role != Some(pebble_core::FolderRole::Inbox)
}

fn should_fail_initial_sync_for_folder_error(
    folder_role: Option<pebble_core::FolderRole>,
    is_retryable: bool,
) -> bool {
    folder_role == Some(pebble_core::FolderRole::Inbox) || is_retryable
}

fn idle_check_recovery_user_error(
    reconnect_error: Option<String>,
    poll_error: Option<String>,
) -> Option<(&'static str, String)> {
    if let Some(error) = reconnect_error {
        return Some((
            "connection",
            format!("IMAP reconnect after idle check failed: {error}"),
        ));
    }
    if let Some(error) = poll_error {
        return Some((
            "poll",
            format!("Poll after idle check reconnect failed: {error}"),
        ));
    }
    None
}

fn is_retryable_imap_connection_error(error: &PebbleError) -> bool {
    let PebbleError::Network(message) = error else {
        return false;
    };
    let lower = message.to_ascii_lowercase();

    lower.contains("os error 10053")
        || lower.contains("connection reset")
        || lower.contains("connection aborted")
        || lower.contains("broken pipe")
        || lower.contains("connection closed")
        || lower.contains("unexpected eof")
        || lower.contains("timed out")
        || lower == "not connected"
}

fn is_missing_imap_mailbox_error(error: &PebbleError) -> bool {
    let PebbleError::Network(message) = error else {
        return false;
    };
    let lower = message.to_ascii_lowercase();

    lower.contains("folder not exist")
        || lower.contains("mailbox does not exist")
        || lower.contains("mailbox doesn't exist")
        || lower.contains("no such mailbox")
}

fn should_attempt_imap_remote_folder(folder: &pebble_core::Folder) -> bool {
    !folder.remote_id.starts_with("__local_")
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImapPollScope {
    Realtime,
    Full,
}

fn should_poll_imap_folder_for_scope(folder: &pebble_core::Folder, scope: ImapPollScope) -> bool {
    if !should_attempt_imap_remote_folder(folder) {
        return false;
    }

    match scope {
        ImapPollScope::Realtime => folder.role == Some(pebble_core::FolderRole::Inbox),
        ImapPollScope::Full => true,
    }
}

#[cfg(test)]
fn should_poll_imap_folder_for_realtime(folder: &pebble_core::Folder) -> bool {
    should_poll_imap_folder_for_scope(folder, ImapPollScope::Realtime)
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

impl SyncConfig {
    pub fn manual_only(&self) -> bool {
        self.poll_interval_secs == 0
    }
}

fn imap_poll_policy(config: &SyncConfig) -> RealtimePollPolicy {
    RealtimePollPolicy::from_foreground_interval_secs(config.poll_interval_secs)
}

fn first_reconcile_deadline(now: Instant, interval: Duration) -> Instant {
    now + interval
}

fn should_notify_imap_startup_fetch(since_uid: Option<u32>) -> bool {
    since_uid.is_some()
}

/// A newly stored message along with the folder IDs it belongs to.
/// Emitted via `message_tx` so callers (e.g. the search indexer) can react.
#[derive(Debug, Clone)]
pub struct StoredMessage {
    pub message: Message,
    pub folder_ids: Vec<String>,
    pub notify: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyncRuntimeStatus {
    ImapIdleAvailable,
    ImapPollingFallback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ImapWorkerTrigger {
    ProviderPush,
}

pub(crate) async fn recv_sync_trigger(
    trigger_rx: &mut Option<mpsc::UnboundedReceiver<SyncTrigger>>,
) -> Option<SyncTrigger> {
    match trigger_rx {
        Some(rx) => rx.recv().await,
        None => std::future::pending().await,
    }
}

/// Common fields shared by all sync workers.
pub(crate) struct SyncWorkerBase {
    pub(crate) account_id: String,
    pub(crate) store: Arc<Store>,
    pub(crate) attachments_dir: PathBuf,
    pub(crate) error_tx: Option<mpsc::UnboundedSender<SyncError>>,
    pub(crate) message_tx: Option<mpsc::UnboundedSender<StoredMessage>>,
    pub(crate) runtime_status_tx: Option<mpsc::UnboundedSender<SyncRuntimeStatus>>,
    pub(crate) progress_tx: Option<mpsc::UnboundedSender<SyncProgress>>,
}

impl SyncWorkerBase {
    /// Emit a structured error through the error channel.
    pub(crate) fn emit_error(&self, error_type: &str, message: &str) {
        if let Some(tx) = &self.error_tx {
            let _ = tx.send(SyncError {
                error_type: error_type.to_string(),
                message: message.to_string(),
                timestamp: now_timestamp() as u64,
            });
        }
    }

    /// Emit a newly stored message through the message channel.
    pub(crate) fn emit_message(&self, message: StoredMessage) {
        if let Some(tx) = &self.message_tx {
            let _ = tx.send(message);
        }
    }

    pub(crate) fn emit_runtime_status(&self, status: SyncRuntimeStatus) {
        if let Some(tx) = &self.runtime_status_tx {
            let _ = tx.send(status);
        }
    }

    pub(crate) fn emit_sync_started(&self, phase: &str) {
        if let Some(tx) = &self.progress_tx {
            let _ = tx.send(SyncProgress::started(&self.account_id, phase));
        }
    }

    pub(crate) fn emit_sync_completed(&self, phase: &str) {
        if let Some(tx) = &self.progress_tx {
            let _ = tx.send(SyncProgress::completed(&self.account_id, phase));
        }
    }

    pub(crate) fn emit_sync_error(&self, phase: &str, message: &str) {
        if let Some(tx) = &self.progress_tx {
            let _ = tx.send(SyncProgress::error(&self.account_id, phase, message));
        }
    }
}

/// A worker that syncs mail for one account.
pub struct SyncWorker {
    pub(crate) base: SyncWorkerBase,
    provider: Arc<ImapMailProvider>,
    idle_provider: Arc<ImapMailProvider>,
    stop_rx: watch::Receiver<bool>,
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
        let idle_provider = Arc::new(provider.clone_for_idle());
        Self {
            base: SyncWorkerBase {
                account_id: account_id.into(),
                store,
                attachments_dir: attachments_dir.into(),
                error_tx: None,
                message_tx: None,
                runtime_status_tx: None,
                progress_tx: None,
            },
            provider,
            idle_provider,
            stop_rx,
        }
    }

    /// Set the error channel for emitting structured sync errors.
    pub fn with_error_tx(mut self, tx: mpsc::UnboundedSender<SyncError>) -> Self {
        self.base.error_tx = Some(tx);
        self
    }

    /// Set the channel for emitting newly stored messages (used for search indexing).
    pub fn with_message_tx(mut self, tx: mpsc::UnboundedSender<StoredMessage>) -> Self {
        self.base.message_tx = Some(tx);
        self
    }

    pub fn with_runtime_status_tx(mut self, tx: mpsc::UnboundedSender<SyncRuntimeStatus>) -> Self {
        self.base.runtime_status_tx = Some(tx);
        self
    }

    pub fn with_progress_tx(mut self, tx: mpsc::UnboundedSender<SyncProgress>) -> Self {
        self.base.progress_tx = Some(tx);
        self
    }

    fn refresh_inbox_local_uid_baseline(
        &self,
        last_exists: &mut Option<crate::idle::MailboxUidState>,
    ) -> bool {
        let folders = match self.base.store.list_folders(&self.base.account_id) {
            Ok(folders) => folders,
            Err(e) => {
                warn!(
                    "Failed to load folders while refreshing IMAP polling baseline for account {}: {}",
                    self.base.account_id, e
                );
                return false;
            }
        };

        let Some(inbox) = folders
            .iter()
            .find(|folder| folder.role == Some(pebble_core::FolderRole::Inbox))
        else {
            warn!(
                "Failed to refresh IMAP polling baseline for account {}: no Inbox folder found",
                self.base.account_id
            );
            return false;
        };

        let has_unresolved_failures = match self
            .base
            .store
            .has_sync_failures_for_folder(&self.base.account_id, &inbox.id)
        {
            Ok(has_failures) => has_failures,
            Err(e) => {
                warn!(
                    "Failed to check Inbox sync failures while refreshing IMAP polling baseline for account {} folder {}: {}",
                    self.base.account_id, inbox.remote_id, e
                );
                return false;
            }
        };
        if has_unresolved_failures {
            debug!(
                "Skipping IMAP polling baseline refresh for account {} folder {} because Inbox has unresolved sync failures",
                self.base.account_id, inbox.remote_id
            );
            return false;
        }

        let folder_sync_state = match self
            .base
            .store
            .get_folder_sync_state(&self.base.account_id, &inbox.id)
        {
            Ok(state) => state,
            Err(e) => {
                warn!(
                    "Failed to load local Inbox cursor while refreshing IMAP polling baseline for account {} folder {}: {}",
                    self.base.account_id, inbox.remote_id, e
                );
                return false;
            }
        };
        let cursor = parse_imap_folder_cursor(folder_sync_state.as_deref());

        let local_max_uid = match self
            .base
            .store
            .get_max_remote_id(&self.base.account_id, &inbox.id)
        {
            Ok(remote_id) => remote_id.and_then(|uid| uid.parse::<u32>().ok()),
            Err(e) => {
                warn!(
                    "Failed to load local Inbox max UID while refreshing IMAP polling baseline for account {} folder {}: {}",
                    self.base.account_id, inbox.remote_id, e
                );
                return false;
            }
        };
        let refreshed = apply_local_inbox_uid_baseline(
            last_exists,
            cursor.uidvalidity,
            local_max_uid,
            has_unresolved_failures,
        );
        if refreshed {
            debug!(
                "Refreshed IMAP polling baseline for account {} folder {} to local max UID {}",
                self.base.account_id,
                inbox.remote_id,
                last_exists.map(|state| state.highest_uid).unwrap_or(0)
            );
        }
        refreshed
    }

    fn stored_imap_folder_cursor(&self, folder: &pebble_core::Folder) -> ImapFolderCursor {
        let state = self
            .base
            .store
            .get_folder_sync_state(&self.base.account_id, &folder.id)
            .ok()
            .flatten();
        let mut cursor = parse_imap_folder_cursor(state.as_deref());
        let has_failures = self
            .base
            .store
            .has_sync_failures_for_folder(&self.base.account_id, &folder.id)
            .unwrap_or(false);
        if cursor.last_uid.is_none() && can_advance_imap_folder_cursor(has_failures) {
            cursor.last_uid = self
                .base
                .store
                .get_max_remote_id(&self.base.account_id, &folder.id)
                .ok()
                .flatten()
                .and_then(|s| s.parse::<u32>().ok());
        }
        cursor
    }

    async fn try_imap_folder_cursor_for_sync(
        &self,
        folder: &pebble_core::Folder,
    ) -> Result<ImapFolderCursor> {
        let cursor = self.stored_imap_folder_cursor(folder);
        if !should_attempt_imap_remote_folder(folder) {
            return Ok(cursor);
        }
        let status = self
            .provider
            .inner()
            .get_mailbox_status(&folder.remote_id)
            .await?;
        Ok(prepare_imap_folder_cursor_for_status(
            cursor,
            status.uid_validity.map(u64::from),
            status.highest_modseq,
        ))
    }

    fn persist_imap_folder_cursor_after_sync(
        &self,
        folder: &pebble_core::Folder,
        mut cursor: ImapFolderCursor,
    ) -> Result<()> {
        if !can_advance_imap_folder_cursor(
            self.base
                .store
                .has_sync_failures_for_folder(&self.base.account_id, &folder.id)?,
        ) {
            debug!(
                "Keeping previous IMAP cursor for {} because unresolved sync failures exist",
                folder.name
            );
            return Ok(());
        }

        if let Some(max_uid) = self
            .base
            .store
            .get_max_remote_id(&self.base.account_id, &folder.id)?
            .and_then(|s| s.parse::<u32>().ok())
        {
            cursor.last_uid = Some(max_uid);
        }
        if let Some(state) = serialize_imap_folder_cursor(&cursor) {
            self.base
                .store
                .set_folder_sync_state(&self.base.account_id, &folder.id, &state)?;
        }
        Ok(())
    }

    /// Perform the initial full sync: list folders and fetch all of them.
    pub async fn initial_sync(&self) -> Result<()> {
        info!("Starting initial sync for account {}", self.base.account_id);

        let remote_folders = self
            .provider
            .inner()
            .list_folders(&self.base.account_id)
            .await?;

        for folder in &remote_folders {
            // Upsert folder into store
            let _ = self.base.store.insert_folder(folder);
        }

        // Ensure an Archive folder exists locally (even if the IMAP server doesn't have one)
        let has_archive = remote_folders
            .iter()
            .any(|f| f.role == Some(pebble_core::FolderRole::Archive));
        if !has_archive {
            let archive = pebble_core::Folder {
                id: new_id(),
                account_id: self.base.account_id.clone(),
                remote_id: "__local_archive__".to_string(),
                name: "Archive".to_string(),
                folder_type: pebble_core::FolderType::Folder,
                role: Some(pebble_core::FolderRole::Archive),
                parent_id: None,
                color: None,
                is_system: true,
                sort_order: 3,
            };
            let _ = self.base.store.insert_folder(&archive);
            info!(
                "Created local archive folder for account {}",
                self.base.account_id
            );
        }

        // Re-read folders from DB so we use the actual persisted IDs
        // (insert_folder upserts, so in-memory IDs from list_folders may differ).
        let folders = self.base.store.list_folders(&self.base.account_id)?;

        // Sync all folders, prioritising Inbox first
        let mut ordered: Vec<&pebble_core::Folder> = Vec::with_capacity(folders.len());
        if let Some(inbox) = folders
            .iter()
            .find(|f| f.role == Some(pebble_core::FolderRole::Inbox))
        {
            ordered.push(inbox);
        }
        for f in &folders {
            if f.role != Some(pebble_core::FolderRole::Inbox) {
                ordered.push(f);
            }
        }

        let mut first_initial_sync_error = None;
        for folder in &ordered {
            if let Err(e) = self.initial_sync_folder_with_reconnect(folder).await {
                warn!("Initial sync folder {} failed: {}", folder.name, e);
                let is_retryable = is_retryable_imap_connection_error(&e);
                if should_fail_initial_sync_for_folder_error(folder.role.clone(), is_retryable)
                    && first_initial_sync_error.is_none()
                {
                    first_initial_sync_error = Some(e);
                }
            }
        }

        if let Some(e) = first_initial_sync_error {
            return Err(e);
        }

        Ok(())
    }

    async fn initial_sync_folder_with_reconnect(&self, folder: &pebble_core::Folder) -> Result<()> {
        match self.initial_sync_folder_once(folder).await {
            Ok(()) => Ok(()),
            Err(e)
                if is_missing_imap_mailbox_error(&e)
                    && should_skip_missing_imap_mailbox_during_initial_sync(
                        folder.role.clone(),
                    ) =>
            {
                debug!(
                    "Skipping unavailable IMAP folder {} ({}) for account {}: {}",
                    folder.name, folder.remote_id, self.base.account_id, e
                );
                Ok(())
            }
            Err(e) if is_retryable_imap_connection_error(&e) => {
                warn!(
                    "IMAP connection failed during initial sync for folder {} account {}; reconnecting before retry: {}",
                    folder.name, self.base.account_id, e
                );
                let _ = self.provider.disconnect().await;
                self.provider.connect().await?;
                self.initial_sync_folder_once(folder).await
            }
            Err(e) => Err(e),
        }
    }

    async fn initial_sync_folder_once(&self, folder: &pebble_core::Folder) -> Result<()> {
        if !should_attempt_imap_remote_folder(folder) {
            debug!(
                "Skipping local-only IMAP folder {} ({}) during initial sync",
                folder.name, folder.remote_id
            );
            return Ok(());
        }

        let cursor = self.try_imap_folder_cursor_for_sync(folder).await?;
        let since_uid = cursor.last_uid;
        let limit = if since_uid.is_some() { 50 } else { 200 };
        let notify_new = should_notify_imap_startup_fetch(since_uid);
        let count = self
            .sync_folder(folder, since_uid, limit, notify_new)
            .await?;

        if count > 0 {
            info!(
                "Initial sync: fetched {} messages from {}",
                count, folder.name
            );
        }
        if let Err(e) = self.persist_imap_folder_cursor_after_sync(folder, cursor) {
            warn!("Failed to persist IMAP cursor for {}: {}", folder.name, e);
        }

        Ok(())
    }

    /// Check if a folder is local-only (not backed by IMAP).
    fn is_local_folder(folder: &pebble_core::Folder) -> bool {
        !should_attempt_imap_remote_folder(folder)
    }

    /// Sync a folder: fetch raw messages, parse, compute threads, store.
    /// Returns the number of new messages stored.
    pub async fn sync_folder(
        &self,
        folder: &pebble_core::Folder,
        since_uid: Option<u32>,
        limit: u32,
        notify_new: bool,
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

        // Bulk-check which UIDs already exist to avoid N+1 queries
        let all_remote_ids: Vec<String> = raw_messages
            .iter()
            .map(|(uid, _)| uid.to_string())
            .collect();
        let existing_ids = self
            .base
            .store
            .get_existing_remote_ids_in_folder(&self.base.account_id, &folder.id, &all_remote_ids)
            .unwrap_or_default();

        // Parse all raw messages upfront so we can collect In-Reply-To / References
        // before querying thread mappings (avoids loading the full account mapping).
        let parsed_messages: Vec<(u32, Result<crate::parser::ParsedMessage>)> = raw_messages
            .into_iter()
            .filter(|(uid, _)| {
                let remote_id = uid.to_string();
                if existing_ids.contains(&remote_id) {
                    let _ = self.base.store.clear_sync_failure(
                        &self.base.account_id,
                        &folder.id,
                        &remote_id,
                    );
                    debug!(
                        "Message UID {} already stored in {}, skipping",
                        uid, folder.name
                    );
                    false
                } else {
                    true
                }
            })
            .map(|(uid, raw)| {
                let parsed = parse_raw_email(&raw);
                (uid, parsed)
            })
            .collect();

        // Collect all referenced message-ID headers from this batch.
        let ref_ids = collect_ref_ids_from_parsed(&parsed_messages);

        // Load thread mappings only for the IDs referenced by this batch.
        // This is mutable so we can extend it as we store new messages within the batch,
        // ensuring intra-batch replies find their parent's thread.
        let mut thread_mappings = self
            .base
            .store
            .get_thread_mappings_for_refs(&self.base.account_id, &ref_ids)
            .unwrap_or_default();

        let mut stored_count = 0u32;

        for (uid, parse_result) in parsed_messages {
            let remote_id = uid.to_string();

            let parsed = match parse_result {
                Ok(p) => p,
                Err(e) => {
                    let reason = e.to_string();
                    let _ = self.base.store.upsert_sync_failure(
                        &self.base.account_id,
                        &folder.id,
                        &remote_id,
                        "imap",
                        &reason,
                    );
                    warn!("Failed to parse message UID {}: {}", uid, e);
                    continue;
                }
            };

            let now = now_timestamp();

            // Build a temporary message to compute thread_id
            let mut msg = Message {
                id: new_id(),
                account_id: self.base.account_id.clone(),
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
                .base
                .store
                .insert_message(&msg, std::slice::from_ref(&folder.id))
            {
                Ok(()) => {
                    stored_count += 1;
                    let _ = self.base.store.clear_sync_failure(
                        &self.base.account_id,
                        &folder.id,
                        &remote_id,
                    );
                    // Update in-memory thread mappings so later messages in this batch
                    // can find this message as a thread parent.
                    if let (Some(mid), Some(tid)) = (&msg.message_id_header, &msg.thread_id) {
                        thread_mappings.insert(mid.clone(), tid.clone());
                    }

                    persist_message_attachments_async(
                        Arc::clone(&self.base.store),
                        self.base.attachments_dir.clone(),
                        msg.id.clone(),
                        parsed.attachments,
                    )
                    .await;

                    // Notify listeners (e.g. search indexer) about the new message
                    self.base.emit_message(StoredMessage {
                        message: msg.clone(),
                        folder_ids: vec![folder.id.clone()],
                        notify: notify_new,
                    });
                }
                Err(e) => {
                    let reason = e.to_string();
                    let _ = self.base.store.upsert_sync_failure(
                        &self.base.account_id,
                        &folder.id,
                        &remote_id,
                        "imap",
                        &reason,
                    );
                    error!("Failed to store message UID {}: {}", uid, e);
                }
            }
        }

        Ok(stored_count)
    }

    /// Poll all folders for new messages since the highest known UID.
    pub async fn poll_new_messages(&self) -> Result<()> {
        self.poll_new_messages_for_scope("poll", ImapPollScope::Realtime)
            .await
    }

    async fn poll_all_new_messages(&self, phase: &str) -> Result<()> {
        self.poll_new_messages_for_scope(phase, ImapPollScope::Full)
            .await
    }

    async fn poll_new_messages_for_scope(&self, phase: &str, scope: ImapPollScope) -> Result<()> {
        self.base.emit_sync_started(phase);
        let result = self.poll_new_messages_inner(scope).await;
        match &result {
            Ok(()) => self.base.emit_sync_completed(phase),
            Err(e) => self.base.emit_sync_error(phase, &e.to_string()),
        }
        result
    }

    async fn poll_new_messages_inner(&self, scope: ImapPollScope) -> Result<()> {
        let folders = self.base.store.list_folders(&self.base.account_id)?;
        if folders.is_empty() {
            return Ok(());
        }

        let mut first_recovered_error = None;
        for folder in folders
            .iter()
            .filter(|folder| should_poll_imap_folder_for_scope(folder, scope))
        {
            if let Some(e) = self.poll_imap_folder_with_reconnect(folder).await {
                if first_recovered_error.is_none() {
                    first_recovered_error = Some(e);
                }
            }
        }

        if let Some(e) = first_recovered_error {
            return Err(e);
        }

        Ok(())
    }

    async fn poll_imap_folder_with_reconnect(
        &self,
        folder: &pebble_core::Folder,
    ) -> Option<PebbleError> {
        match self.poll_imap_folder_once(folder).await {
            Ok(()) => None,
            Err(e) if is_missing_imap_mailbox_error(&e) => {
                debug!(
                    "Skipping unavailable IMAP folder {} ({}) for account {}: {}",
                    folder.name, folder.remote_id, self.base.account_id, e
                );
                None
            }
            Err(e) if is_retryable_imap_connection_error(&e) => {
                warn!(
                    "IMAP connection failed while polling folder {} account {}; reconnecting before retry: {}",
                    folder.name, self.base.account_id, e
                );
                let _ = self.provider.disconnect().await;
                match self.provider.connect().await {
                    Ok(()) => {
                        if let Err(retry_error) = self.poll_imap_folder_once(folder).await {
                            warn!(
                                "Poll retry failed for folder {} account {} after reconnect: {}",
                                folder.name, self.base.account_id, retry_error
                            );
                            if is_retryable_imap_connection_error(&retry_error) {
                                return Some(retry_error);
                            }
                        }
                        None
                    }
                    Err(reconnect_error) => {
                        warn!(
                            "Reconnect failed while polling folder {} account {}: {}",
                            folder.name, self.base.account_id, reconnect_error
                        );
                        Some(reconnect_error)
                    }
                }
            }
            Err(e) => {
                warn!(
                    "Poll failed for folder {} account {}: {}",
                    folder.name, self.base.account_id, e
                );
                None
            }
        }
    }

    async fn poll_imap_folder_once(&self, folder: &pebble_core::Folder) -> Result<()> {
        if !should_attempt_imap_remote_folder(folder) {
            debug!(
                "Skipping local-only IMAP folder {} ({}) during poll",
                folder.name, folder.remote_id
            );
            return Ok(());
        }

        let cursor = self.try_imap_folder_cursor_for_sync(folder).await?;
        let since_uid = cursor.last_uid;

        let count = self.sync_folder(folder, since_uid, 50, true).await?;
        if count > 0 {
            info!(
                "Polled {} new messages from {} for account {}",
                count, folder.name, self.base.account_id
            );
        }
        if let Err(e) = self.persist_imap_folder_cursor_after_sync(folder, cursor) {
            warn!("Failed to persist IMAP cursor for {}: {}", folder.name, e);
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
        match self.reconcile_folder_once(folder).await {
            Ok(()) => Ok(()),
            Err(e) if is_missing_imap_mailbox_error(&e) => {
                debug!(
                    "Skipping unavailable IMAP folder {} ({}) during reconcile for account {}: {}",
                    folder.name, folder.remote_id, self.base.account_id, e
                );
                Ok(())
            }
            Err(e) if is_retryable_imap_connection_error(&e) => {
                warn!(
                    "IMAP connection failed while reconciling folder {} account {}; reconnecting before retry: {}",
                    folder.name, self.base.account_id, e
                );
                let _ = self.provider.disconnect().await;
                self.provider.connect().await?;
                self.reconcile_folder_once(folder).await
            }
            Err(e) => Err(e),
        }
    }

    async fn reconcile_folder_once(&self, folder: &pebble_core::Folder) -> Result<()> {
        // Skip local-only folders
        if Self::is_local_folder(folder) {
            return Ok(());
        }

        // Step 1: Get local state
        let local_state = self
            .base
            .store
            .list_remote_ids_by_folder(&self.base.account_id, &folder.id)?;
        if local_state.is_empty() {
            return Ok(());
        }

        // Read stored MODSEQ from this folder's cursor.
        let stored_modseq = self
            .stored_imap_folder_cursor(folder)
            .highest_modseq
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
                warn!(
                    "CONDSTORE HIGHESTMODSEQ check failed for {}: {}",
                    folder.name, e
                );
                false
            }
        };

        if !condstore_skip {
            // Step 3: Fetch remote flags (with MODSEQ when possible)
            let uids: Vec<u32> = local_state
                .iter()
                .filter_map(|(_, remote_id, _, _, _)| remote_id.parse().ok())
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
                self.base.store.bulk_update_flags(&flag_changes)?;
            }

            // Step 5: Persist new MODSEQ in cursor if we got one
            if new_modseq > 0 {
                self.update_folder_cursor_modseq(folder, new_modseq);
            }
        }

        // Step 6: Detect deletions (always — CONDSTORE doesn't cover expunges).
        // EXISTS can stay unchanged when one message is expunged and another
        // is added, so compare UID sets whenever local state exists.
        let server_exists = self
            .provider
            .inner()
            .select_exists(&folder.remote_id)
            .await?;
        if should_run_imap_deletion_diff(server_exists, local_state.len()) {
            let server_uids = self
                .provider
                .inner()
                .fetch_all_uids(&folder.remote_id)
                .await?;
            let local_remote_ids: Vec<(String, String)> = local_state
                .iter()
                .map(|(id, rid, _, _, _)| (id.clone(), rid.clone()))
                .collect();
            let deleted = reconcile::detect_deletions(&local_remote_ids, &server_uids);
            if !deleted.is_empty() {
                info!(
                    "Soft-deleting {} server-removed messages from {}",
                    deleted.len(),
                    folder.name
                );
                self.base.store.bulk_soft_delete(&deleted)?;
            }
        }

        Ok(())
    }

    /// Update the MODSEQ portion of one folder cursor without changing its UID.
    fn update_folder_cursor_modseq(&self, folder: &pebble_core::Folder, new_modseq: u64) {
        let mut cursor = self.stored_imap_folder_cursor(folder);
        cursor.highest_modseq = Some(new_modseq);
        if let Some(state) = serialize_imap_folder_cursor(&cursor) {
            let _ =
                self.base
                    .store
                    .set_folder_sync_state(&self.base.account_id, &folder.id, &state);
        }
    }

    fn spawn_idle_watcher(
        account_id: String,
        idle_provider: Arc<ImapMailProvider>,
        inbox_remote_id: String,
        configured_idle_wait_secs: u64,
        mut stop_rx: watch::Receiver<bool>,
        trigger_tx: mpsc::UnboundedSender<ImapWorkerTrigger>,
    ) -> tokio::task::JoinHandle<()> {
        tokio::spawn(async move {
            let idle_wait = tokio::time::Duration::from_secs(
                crate::idle::recommended_idle_wait_secs(configured_idle_wait_secs),
            );
            let mut backoff = SyncBackoff::new();
            let mut connected = false;

            loop {
                if *stop_rx.borrow() {
                    break;
                }

                if backoff.is_circuit_open() {
                    let delay = backoff.current_delay();
                    warn!(
                        "IMAP IDLE watcher circuit open for account {} ({} failures), waiting {:?}",
                        account_id,
                        backoff.failure_count(),
                        delay
                    );
                    match tokio::time::timeout(delay, stop_rx.changed()).await {
                        Ok(Ok(())) if *stop_rx.borrow() => break,
                        _ => {}
                    }
                    continue;
                }

                if !connected {
                    match idle_provider.connect().await {
                        Ok(()) => {
                            connected = true;
                            backoff.record_success();
                        }
                        Err(e) => {
                            warn!(
                                "Failed to connect IMAP IDLE watcher for account {account_id}: {e}"
                            );
                            let delay = backoff.record_failure();
                            match tokio::time::timeout(delay, stop_rx.changed()).await {
                                Ok(Ok(())) if *stop_rx.borrow() => break,
                                _ => {}
                            }
                            continue;
                        }
                    }
                }

                let idle_result = tokio::select! {
                    result = idle_provider.inner().idle_wait(&inbox_remote_id, idle_wait) => Some(result),
                    changed = stop_rx.changed() => {
                        match changed {
                            Ok(()) if *stop_rx.borrow() => None,
                            _ => continue,
                        }
                    }
                };

                let Some(idle_result) = idle_result else {
                    break;
                };

                match idle_result {
                    Ok(crate::idle::IdleEvent::NewMail) => {
                        let _ = trigger_tx.send(ImapWorkerTrigger::ProviderPush);
                        backoff.record_success();
                    }
                    Ok(crate::idle::IdleEvent::Timeout) => {
                        debug!("IMAP IDLE timeout for account {account_id}; re-entering IDLE");
                        backoff.record_success();
                    }
                    Ok(crate::idle::IdleEvent::Error(e)) => {
                        warn!("IMAP IDLE watcher error for account {account_id}: {e}");
                        let _ = idle_provider.disconnect().await;
                        connected = false;
                        let delay = backoff.record_failure();
                        match tokio::time::timeout(delay, stop_rx.changed()).await {
                            Ok(Ok(())) if *stop_rx.borrow() => break,
                            _ => {}
                        }
                    }
                    Err(e) => {
                        warn!("IMAP IDLE watcher failed for account {account_id}: {e}");
                        let _ = idle_provider.disconnect().await;
                        connected = false;
                        let delay = backoff.record_failure();
                        match tokio::time::timeout(delay, stop_rx.changed()).await {
                            Ok(Ok(())) if *stop_rx.borrow() => break,
                            _ => {}
                        }
                    }
                }
            }

            let _ = idle_provider.disconnect().await;
            info!("IMAP IDLE watcher stopped for account {account_id}");
        })
    }

    /// Run the sync worker loop until the stop signal is received.
    pub async fn run(
        &self,
        config: SyncConfig,
        trigger_rx: Option<mpsc::UnboundedReceiver<SyncTrigger>>,
    ) {
        // Connect and do initial sync
        if let Err(e) = self.provider.connect().await {
            error!(
                "Failed to connect for account {}: {}",
                self.base.account_id, e
            );
            self.base
                .emit_error("connection", &format!("Failed to connect: {}", e));
            self.base.emit_sync_error("connection", &e.to_string());
            return;
        }

        self.base.emit_sync_started("initial");
        let initial_sync_succeeded = match self.initial_sync().await {
            Ok(()) => {
                self.base.emit_sync_completed("initial");
                true
            }
            Err(e) => {
                error!(
                    "Initial sync failed for account {}: {}",
                    self.base.account_id, e
                );
                self.base
                    .emit_error("sync", &format!("Initial sync failed: {}", e));
                self.base.emit_sync_error("initial", &e.to_string());
                false
            }
        };

        if config.manual_only() {
            info!("Manual sync completed for account {}", self.base.account_id);
            let _ = self.provider.disconnect().await;
            return;
        }

        let poll_policy = imap_poll_policy(&config);
        let reconcile_interval = tokio::time::Duration::from_secs(config.reconcile_interval_secs);

        let mut reconcile_ticker = tokio::time::interval_at(
            tokio::time::Instant::from_std(first_reconcile_deadline(
                Instant::now(),
                reconcile_interval,
            )),
            reconcile_interval,
        );
        reconcile_ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);

        let supports_idle = self.provider.inner().supports_idle().await;
        if supports_idle {
            info!("IMAP IDLE supported for account {}", self.base.account_id);
            self.base
                .emit_runtime_status(SyncRuntimeStatus::ImapIdleAvailable);
        } else {
            info!(
                "IMAP IDLE not supported for account {}, using polling",
                self.base.account_id
            );
            self.base
                .emit_runtime_status(SyncRuntimeStatus::ImapPollingFallback);
        }

        let supports_condstore = self.provider.inner().supports_condstore().await;
        if supports_condstore {
            info!("CONDSTORE supported for account {}", self.base.account_id);
        } else {
            debug!(
                "CONDSTORE not supported for account {}",
                self.base.account_id
            );
        }

        let mut stop_rx = self.stop_rx.clone();
        let mut last_exists: Option<crate::idle::MailboxUidState> = None;
        let mut backoff = SyncBackoff::new();
        let mut trigger_rx = trigger_rx;
        let mut runtime = RealtimeRuntimeState::new(Duration::from_secs(60), Instant::now());
        let (idle_trigger_tx, mut idle_trigger_rx) = mpsc::unbounded_channel();
        let mut idle_watcher = None;

        if supports_idle {
            match self.base.store.list_folders(&self.base.account_id) {
                Ok(folders) => {
                    if let Some(inbox) = folders
                        .iter()
                        .find(|f| f.role == Some(pebble_core::FolderRole::Inbox))
                    {
                        idle_watcher = Some(Self::spawn_idle_watcher(
                            self.base.account_id.clone(),
                            Arc::clone(&self.idle_provider),
                            inbox.remote_id.clone(),
                            config.poll_interval_secs,
                            self.stop_rx.clone(),
                            idle_trigger_tx.clone(),
                        ));
                    } else {
                        warn!(
                            "IMAP IDLE supported for account {}, but no Inbox folder was available; using polling",
                            self.base.account_id
                        );
                    }
                }
                Err(e) => {
                    warn!(
                        "Failed to load folders before starting IMAP IDLE for account {}: {}; using polling",
                        self.base.account_id, e
                    );
                }
            }
        }
        drop(idle_trigger_tx);
        let mut idle_watcher_active = idle_watcher.is_some();
        let mut polling_baseline_trusted = false;
        if !idle_watcher_active
            && can_seed_imap_polling_baseline_after_startup(initial_sync_succeeded)
        {
            polling_baseline_trusted = self.refresh_inbox_local_uid_baseline(&mut last_exists);
        }

        loop {
            let next_poll_delay =
                poll_policy.next_delay(runtime.context(backoff.failure_count(), Instant::now()));

            tokio::select! {
                _ = tokio::time::sleep(next_poll_delay), if !idle_watcher_active => {
                    if backoff.is_circuit_open() {
                        warn!(
                            "Circuit open for account {} ({} consecutive failures); attempting half-open poll after scheduled delay",
                            self.base.account_id, backoff.failure_count()
                        );
                    }

                    if !polling_baseline_trusted {
                        match self.poll_new_messages().await {
                            Ok(()) => {
                                backoff.record_success();
                                polling_baseline_trusted =
                                    self.refresh_inbox_local_uid_baseline(&mut last_exists);
                            }
                            Err(e) => {
                                warn!("Catch-up poll before IMAP baseline refresh failed for account {}: {}", self.base.account_id, e);
                                self.base.emit_error("poll", &format!("Catch-up poll before IMAP baseline refresh failed: {}", e));
                                backoff.record_failure();
                            }
                        }
                        continue;
                    }

                    // Quick check if mailbox has changes before doing full poll
                    let folders = match self.base.store.list_folders(&self.base.account_id) {
                        Ok(f) => f,
                        Err(_) => {
                            backoff.record_failure();
                            continue;
                        }
                    };
                    if let Some(inbox) = folders.iter().find(|f| f.role == Some(pebble_core::FolderRole::Inbox)) {
                        match crate::idle::check_for_changes_with_idle(self.provider.inner(), &inbox.remote_id, &mut last_exists, false).await {
                            Ok(crate::idle::IdleEvent::NewMail) => {
                                if let Err(e) = self.poll_new_messages().await {
                                    warn!("Poll error for account {}: {}", self.base.account_id, e);
                                    self.base.emit_error("poll", &format!("Poll error: {}", e));
                                    backoff.record_failure();
                                } else {
                                    backoff.record_success();
                                    polling_baseline_trusted =
                                        self.refresh_inbox_local_uid_baseline(&mut last_exists);
                                }
                            }
                            Ok(crate::idle::IdleEvent::Timeout) => {
                                debug!("No changes detected for account {}", self.base.account_id);
                                backoff.record_success();
                            }
                            Ok(crate::idle::IdleEvent::Error(e)) => {
                                warn!("IDLE check error for account {}: {}", self.base.account_id, e);
                                let _ = self.provider.disconnect().await;
                                let recovery_error = match self.provider.connect().await {
                                    Ok(()) => match self.poll_new_messages().await {
                                        Ok(()) => {
                                            polling_baseline_trusted = self
                                                .refresh_inbox_local_uid_baseline(&mut last_exists);
                                            None
                                        }
                                        Err(poll_error) => {
                                            warn!(
                                                "Poll after idle check reconnect failed for account {}: {}",
                                                self.base.account_id, poll_error
                                            );
                                            idle_check_recovery_user_error(
                                                None,
                                                Some(poll_error.to_string()),
                                            )
                                        }
                                    },
                                    Err(reconnect_error) => {
                                        warn!(
                                            "Reconnect after idle check failed for account {}: {}",
                                            self.base.account_id, reconnect_error
                                        );
                                        idle_check_recovery_user_error(
                                            Some(reconnect_error.to_string()),
                                            None,
                                        )
                                    }
                                };
                                if let Some((error_type, message)) = recovery_error {
                                    self.base.emit_error(error_type, &message);
                                    backoff.record_failure();
                                } else {
                                    backoff.record_success();
                                }
                            }
                            Err(e) => {
                                warn!("IDLE check failed for account {}: {}", self.base.account_id, e);
                                self.base.emit_error("idle", &format!("IDLE check failed: {}", e));
                                backoff.record_failure();
                            }
                        }
                    }
                }
                trigger = idle_trigger_rx.recv(), if idle_watcher_active => {
                    match trigger {
                        Some(ImapWorkerTrigger::ProviderPush) => {
                            runtime.record_trigger(SyncTrigger::ProviderPush, Instant::now());
                            if backoff.is_circuit_open() {
                                debug!(
                                    "Ignoring IMAP provider push while circuit is open for account {}",
                                    self.base.account_id
                                );
                                continue;
                            }

                            if let Err(e) = self.poll_new_messages().await {
                                warn!("Provider push poll error for account {}: {}", self.base.account_id, e);
                                self.base.emit_error("poll", &format!("Provider push poll error: {}", e));
                                backoff.record_failure();
                            } else {
                                backoff.record_success();
                            }
                        }
                        None => {
                            warn!(
                                "IMAP IDLE watcher exited for account {}; falling back to polling",
                                self.base.account_id
                            );
                            idle_watcher_active = false;
                            let catch_up_succeeded = match self.poll_new_messages().await {
                                Ok(()) => {
                                    backoff.record_success();
                                    true
                                }
                                Err(e) => {
                                    warn!("Catch-up poll after IMAP IDLE watcher exit failed for account {}: {}", self.base.account_id, e);
                                    self.base.emit_error("poll", &format!("Catch-up poll after IMAP IDLE watcher exit failed: {}", e));
                                    backoff.record_failure();
                                    false
                                }
                            };
                            if can_refresh_imap_polling_baseline_after_idle_fallback(catch_up_succeeded) {
                                polling_baseline_trusted =
                                    self.refresh_inbox_local_uid_baseline(&mut last_exists);
                            } else {
                                polling_baseline_trusted = false;
                            }
                        }
                    }
                }
                trigger = recv_sync_trigger(&mut trigger_rx) => {
                    match trigger {
                        Some(trigger) => {
                            runtime.record_trigger(trigger, Instant::now());
                            if !trigger.should_sync_now() {
                                continue;
                            }
                            if backoff.is_circuit_open()
                                && !trigger.bypasses_circuit_backoff()
                            {
                                debug!(
                                    "Ignoring realtime trigger while circuit is open for account {}",
                                    self.base.account_id
                                );
                                continue;
                            }
                            let poll_result = if trigger == SyncTrigger::Manual {
                                self.poll_all_new_messages("manual").await
                            } else {
                                self.poll_new_messages().await
                            };
                            if let Err(e) = poll_result {
                                warn!("Triggered poll error for account {}: {}", self.base.account_id, e);
                                self.base.emit_error("poll", &format!("Triggered poll error: {}", e));
                                backoff.record_failure();
                            } else {
                                backoff.record_success();
                                if !idle_watcher_active {
                                    polling_baseline_trusted =
                                        self.refresh_inbox_local_uid_baseline(&mut last_exists);
                                }
                            }
                        }
                        None => {
                            trigger_rx = None;
                        }
                    }
                }
                _ = reconcile_ticker.tick() => {
                    // Full reconcile: poll new messages + flag diff + deletion detection
                    self.base.emit_sync_started("reconcile");
                    let mut reconcile_failed = false;
                    if let Err(e) = self.poll_new_messages_inner(ImapPollScope::Full).await {
                        warn!("Reconcile poll error for account {}: {}", self.base.account_id, e);
                        self.base.emit_error("reconcile", &format!("Reconcile poll error: {}", e));
                        self.base.emit_sync_error("reconcile", &e.to_string());
                        backoff.record_failure();
                        continue;
                    } else {
                        backoff.record_success();
                        if !idle_watcher_active {
                            polling_baseline_trusted =
                                self.refresh_inbox_local_uid_baseline(&mut last_exists);
                        }
                    }
                    let folders = match self.base.store.list_folders(&self.base.account_id) {
                        Ok(f) => f,
                        Err(e) => {
                            warn!("Reconcile list folders error: {}", e);
                            self.base.emit_error("reconcile", &format!("List folders error: {}", e));
                            self.base.emit_sync_error("reconcile", &e.to_string());
                            continue;
                        }
                    };
                    for folder in &folders {
                        if let Err(e) = self.reconcile_folder(folder).await {
                            warn!("Reconcile folder {} error: {}", folder.name, e);
                            self.base.emit_error("reconcile", &format!("Reconcile {} error: {}", folder.name, e));
                            reconcile_failed = true;
                        }
                    }
                    if reconcile_failed {
                        self.base.emit_sync_error("reconcile", "One or more folders failed to reconcile");
                    } else {
                        self.base.emit_sync_completed("reconcile");
                    }
                }
                Ok(()) = stop_rx.changed() => {
                    if *stop_rx.borrow() {
                        info!("Sync worker stopping for account {}", self.base.account_id);
                        break;
                    }
                }
            }
        }

        if let Some(handle) = idle_watcher.take() {
            handle.abort();
        }
        let _ = self.provider.disconnect().await;
        let _ = self.idle_provider.disconnect().await;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn zero_poll_interval_is_manual_only() {
        let config = SyncConfig {
            poll_interval_secs: 0,
            ..Default::default()
        };

        assert!(config.manual_only());
    }

    #[test]
    fn imap_startup_fetch_notifies_only_when_cursor_exists() {
        assert!(!should_notify_imap_startup_fetch(None));
        assert!(should_notify_imap_startup_fetch(Some(42)));
    }

    #[test]
    fn imap_folder_cursor_roundtrips() {
        let cursor = ImapFolderCursor {
            uidvalidity: Some(1234),
            last_uid: Some(987),
            highest_modseq: Some(4567),
        };

        let json = serde_json::to_string(&cursor).unwrap();
        let decoded: ImapFolderCursor = serde_json::from_str(&json).unwrap();

        assert_eq!(decoded, cursor);
    }

    #[test]
    fn imap_folder_cursor_resets_last_uid_when_uidvalidity_changes() {
        let stored = ImapFolderCursor {
            uidvalidity: Some(1234),
            last_uid: Some(987),
            highest_modseq: Some(4567),
        };

        let prepared = prepare_imap_folder_cursor_for_status(stored, Some(9999), Some(7000));

        assert_eq!(prepared.uidvalidity, Some(9999));
        assert_eq!(prepared.last_uid, None);
        assert_eq!(prepared.highest_modseq, Some(7000));
    }

    #[test]
    fn imap_folder_cursor_preserves_last_uid_when_uidvalidity_matches() {
        let stored = ImapFolderCursor {
            uidvalidity: Some(1234),
            last_uid: Some(987),
            highest_modseq: Some(4567),
        };

        let prepared = prepare_imap_folder_cursor_for_status(stored, Some(1234), Some(7000));

        assert_eq!(prepared.uidvalidity, Some(1234));
        assert_eq!(prepared.last_uid, Some(987));
        assert_eq!(prepared.highest_modseq, Some(7000));
    }

    #[test]
    fn imap_folder_cursor_does_not_advance_with_unresolved_failures() {
        assert!(!can_advance_imap_folder_cursor(true));
    }

    #[test]
    fn imap_folder_cursor_advances_without_unresolved_failures() {
        assert!(can_advance_imap_folder_cursor(false));
    }

    #[test]
    fn imap_deletion_diff_runs_when_server_and_local_counts_match() {
        assert!(should_run_imap_deletion_diff(2, 2));
    }

    #[test]
    fn imap_deletion_diff_skips_empty_local_state() {
        assert!(!should_run_imap_deletion_diff(10, 0));
    }

    #[test]
    fn imap_polling_fallback_uses_realtime_policy_delays() {
        let config = SyncConfig {
            poll_interval_secs: 60,
            ..Default::default()
        };
        let policy = imap_poll_policy(&config);

        assert_eq!(
            policy.next_delay(crate::realtime_policy::RealtimeContext {
                app_foreground: true,
                recent_activity: true,
                consecutive_failures: 0,
            }),
            std::time::Duration::from_secs(60)
        );
        assert_eq!(
            policy.next_delay(crate::realtime_policy::RealtimeContext {
                app_foreground: false,
                recent_activity: false,
                consecutive_failures: 0,
            }),
            std::time::Duration::from_secs(120)
        );
    }

    #[test]
    fn startup_baseline_seed_requires_successful_initial_sync() {
        assert!(can_seed_imap_polling_baseline_after_startup(true));
        assert!(!can_seed_imap_polling_baseline_after_startup(false));
    }

    #[test]
    fn idle_fallback_baseline_refresh_requires_successful_catch_up() {
        assert!(can_refresh_imap_polling_baseline_after_idle_fallback(true));
        assert!(!can_refresh_imap_polling_baseline_after_idle_fallback(
            false
        ));
    }

    #[test]
    fn imap_polling_baseline_refuses_unresolved_inbox_failures() {
        let mut last_exists = Some(crate::idle::MailboxUidState {
            uidvalidity: Some(42),
            highest_uid: 7,
        });

        let seeded = apply_local_inbox_uid_baseline(&mut last_exists, Some(42), Some(12), true);

        assert!(!seeded);
        assert_eq!(
            last_exists,
            Some(crate::idle::MailboxUidState {
                uidvalidity: Some(42),
                highest_uid: 7,
            })
        );
    }

    #[test]
    fn imap_polling_baseline_seeds_local_max_uid_without_unresolved_inbox_failures() {
        let mut last_exists = Some(crate::idle::MailboxUidState {
            uidvalidity: Some(42),
            highest_uid: 7,
        });

        let seeded = apply_local_inbox_uid_baseline(&mut last_exists, Some(43), Some(12), false);

        assert!(seeded);
        assert_eq!(
            last_exists,
            Some(crate::idle::MailboxUidState {
                uidvalidity: Some(43),
                highest_uid: 12,
            })
        );
    }

    #[test]
    fn imap_polling_baseline_seeds_zero_for_clean_empty_local_inbox() {
        let mut last_exists = Some(crate::idle::MailboxUidState {
            uidvalidity: Some(42),
            highest_uid: 7,
        });

        let seeded = apply_local_inbox_uid_baseline(&mut last_exists, Some(43), None, false);

        assert!(seeded);
        assert_eq!(
            last_exists,
            Some(crate::idle::MailboxUidState {
                uidvalidity: Some(43),
                highest_uid: 0,
            })
        );
    }

    #[test]
    fn inbox_missing_mailbox_is_not_skipped_during_initial_sync() {
        assert!(!should_skip_missing_imap_mailbox_during_initial_sync(Some(
            pebble_core::FolderRole::Inbox
        )));
    }

    #[test]
    fn non_inbox_missing_mailbox_can_be_skipped_during_initial_sync() {
        assert!(should_skip_missing_imap_mailbox_during_initial_sync(Some(
            pebble_core::FolderRole::Sent
        )));
    }

    #[test]
    fn inbox_initial_sync_folder_failure_fails_initial_sync() {
        assert!(should_fail_initial_sync_for_folder_error(
            Some(pebble_core::FolderRole::Inbox),
            false
        ));
    }

    #[test]
    fn non_inbox_non_retryable_initial_sync_folder_failure_does_not_fail_initial_sync() {
        assert!(!should_fail_initial_sync_for_folder_error(
            Some(pebble_core::FolderRole::Sent),
            false
        ));
    }

    #[test]
    fn non_inbox_retryable_initial_sync_folder_failure_fails_initial_sync() {
        assert!(should_fail_initial_sync_for_folder_error(None, true));
    }

    #[test]
    fn first_reconcile_deadline_is_delayed_by_interval() {
        let now = Instant::now();
        let interval = Duration::from_secs(900);

        assert_eq!(first_reconcile_deadline(now, interval), now + interval);
    }

    #[test]
    fn idle_check_disconnect_does_not_surface_when_recovery_succeeds() {
        let message = idle_check_recovery_user_error(None, None);

        assert!(message.is_none());
    }

    #[test]
    fn idle_check_reconnect_failure_surfaces_connection_error() {
        let message =
            idle_check_recovery_user_error(Some("Network error: os error 10053".to_string()), None);

        assert_eq!(
            message,
            Some((
                "connection",
                "IMAP reconnect after idle check failed: Network error: os error 10053".to_string()
            ))
        );
    }

    #[test]
    fn imap_windows_connection_abort_is_retryable_for_polling() {
        let error = pebble_core::PebbleError::Network(
            "SELECT failed: io: 你的主机中的软件中止了一个已建立的连接。 (os error 10053)"
                .to_string(),
        );

        assert!(is_retryable_imap_connection_error(&error));
    }

    #[test]
    fn imap_missing_folder_select_error_is_not_retryable_for_polling() {
        let error = pebble_core::PebbleError::Network(
            "SELECT failed: no response: code: None, info: Some(\"SELECT Folder not exist\")"
                .to_string(),
        );

        assert!(!is_retryable_imap_connection_error(&error));
    }

    #[test]
    fn imap_missing_folder_select_error_is_detected_for_suppression() {
        let error = pebble_core::PebbleError::Network(
            "SELECT failed: no response: code: None, info: Some(\"SELECT Folder not exist\")"
                .to_string(),
        );

        assert!(is_missing_imap_mailbox_error(&error));
    }

    #[test]
    fn imap_local_archive_is_not_a_remote_sync_target() {
        let folder = pebble_core::Folder {
            id: "folder-1".to_string(),
            account_id: "account-1".to_string(),
            remote_id: "__local_archive__".to_string(),
            name: "Archive".to_string(),
            folder_type: pebble_core::FolderType::Folder,
            role: Some(pebble_core::FolderRole::Archive),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 3,
        };

        assert!(!should_attempt_imap_remote_folder(&folder));
    }

    fn test_folder(role: Option<pebble_core::FolderRole>, remote_id: &str) -> pebble_core::Folder {
        pebble_core::Folder {
            id: format!("folder-{remote_id}"),
            account_id: "account-1".to_string(),
            remote_id: remote_id.to_string(),
            name: remote_id.to_string(),
            folder_type: pebble_core::FolderType::Folder,
            role,
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 0,
        }
    }

    #[test]
    fn imap_realtime_poll_targets_inbox_only() {
        let inbox = test_folder(Some(pebble_core::FolderRole::Inbox), "INBOX");
        let sent = test_folder(Some(pebble_core::FolderRole::Sent), "Sent");
        let spam = test_folder(Some(pebble_core::FolderRole::Spam), "Spam");
        let custom = test_folder(None, "Newsletters");

        assert!(should_poll_imap_folder_for_realtime(&inbox));
        assert!(!should_poll_imap_folder_for_realtime(&sent));
        assert!(!should_poll_imap_folder_for_realtime(&spam));
        assert!(!should_poll_imap_folder_for_realtime(&custom));
    }

    #[test]
    fn imap_full_poll_targets_all_remote_folders() {
        let inbox = test_folder(Some(pebble_core::FolderRole::Inbox), "INBOX");
        let sent = test_folder(Some(pebble_core::FolderRole::Sent), "Sent");
        let spam = test_folder(Some(pebble_core::FolderRole::Spam), "Spam");
        let custom = test_folder(None, "Newsletters");
        let local = test_folder(Some(pebble_core::FolderRole::Archive), "__local_archive__");

        assert!(should_poll_imap_folder_for_scope(
            &inbox,
            ImapPollScope::Full
        ));
        assert!(should_poll_imap_folder_for_scope(
            &sent,
            ImapPollScope::Full
        ));
        assert!(should_poll_imap_folder_for_scope(
            &spam,
            ImapPollScope::Full
        ));
        assert!(should_poll_imap_folder_for_scope(
            &custom,
            ImapPollScope::Full
        ));
        assert!(!should_poll_imap_folder_for_scope(
            &local,
            ImapPollScope::Full
        ));
    }
}
