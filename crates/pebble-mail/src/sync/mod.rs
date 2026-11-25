use std::collections::HashSet;
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

use crate::imap::{
    plan_imap_body_batches, IMAP_BODY_BATCH_MAX_BYTES, IMAP_BODY_BATCH_MAX_MESSAGES,
    MAX_IMAP_MESSAGE_BYTES,
};
use crate::parser::{parse_raw_email, AttachmentData, ParsedMessage};
use crate::provider::imap_provider::ImapMailProvider;
use crate::realtime_policy::{RealtimePollPolicy, RealtimeRuntimeState, SyncTrigger};
use crate::reconcile;
use crate::thread::{
    apply_thread_merge_to_mappings, compute_thread_id, merge_thread_aliases, thread_merge_aliases,
    ThreadMerge,
};

mod attachments;
mod fetch;
mod folder_sync;
mod imap_cursor;
mod poll;
mod run;
#[cfg(test)]
mod tests;

pub(crate) use attachments::*;
use imap_cursor::*;

/// Run a blocking store operation on Tokio's blocking pool so sync workers
/// never stall the async runtime on SQLite I/O.
pub(crate) async fn blocking_store<F, T>(store: &Arc<Store>, f: F) -> Result<T>
where
    F: FnOnce(&Store) -> Result<T> + Send + 'static,
    T: Send + 'static,
{
    let store = Arc::clone(store);
    tokio::task::spawn_blocking(move || f(&store))
        .await
        .map_err(|e| PebbleError::Internal(format!("spawn_blocking: {e}")))?
}

/// Message IDs among `message_ids` with queued local operations that have
/// not reached the server yet. Sync must not overwrite their flags or
/// folder membership with (stale) server state.
pub(crate) async fn message_ids_with_pending_local_changes(
    store: &Arc<Store>,
    account_id: &str,
    message_ids: Vec<String>,
) -> Result<HashSet<String>> {
    if message_ids.is_empty() {
        return Ok(HashSet::new());
    }
    let account_id = account_id.to_string();
    blocking_store(store, move |store| {
        store.message_ids_with_active_pending_ops(&account_id, &message_ids)
    })
    .await
}

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
    /// Reconciliation updates refresh search only. They must not be surfaced
    /// as new mail or run user rules again.
    pub reconciliation: bool,
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
}
