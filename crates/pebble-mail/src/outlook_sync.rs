use std::future::Future;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::Mutex as StdMutex;
use std::time::{Duration, Instant};

use pebble_core::traits::FolderProvider;
use pebble_core::{now_timestamp, Folder, Message, PebbleError, Result};
use pebble_store::Store;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};

use crate::backoff::SyncBackoff;
use crate::provider::outlook::{OutlookDeltaPage, OutlookProvider};
use crate::gmail_sync::TokenRefresher;
use crate::realtime_policy::{RealtimePollPolicy, RealtimeRuntimeState, SyncTrigger};
use crate::sync::{StoredMessage, SyncConfig, SyncError, SyncWorkerBase, persist_message_attachments_async, recv_sync_trigger};
use crate::thread::compute_thread_id;

struct OutlookDeltaBatch {
    messages: Vec<Message>,
    deleted_remote_ids: Vec<String>,
    delta_link: Option<String>,
}

enum SyncWaitOutcome {
    Stop,
    ReadyToPoll,
    ContextChanged,
}

async fn wait_for_outlook_delay(
    wait: Duration,
    runtime: &mut RealtimeRuntimeState,
    stop_rx: &mut watch::Receiver<bool>,
    trigger_rx: &mut Option<mpsc::UnboundedReceiver<SyncTrigger>>,
) -> SyncWaitOutcome {
    tokio::select! {
        _ = tokio::time::sleep(wait) => SyncWaitOutcome::ReadyToPoll,
        changed = stop_rx.changed() => {
            match changed {
                Ok(()) if *stop_rx.borrow() => SyncWaitOutcome::Stop,
                _ => SyncWaitOutcome::ContextChanged,
            }
        }
        trigger = recv_sync_trigger(trigger_rx) => {
            match trigger {
                Some(trigger) => {
                    runtime.record_trigger(trigger, Instant::now());
                    if trigger.should_sync_now() {
                        SyncWaitOutcome::ReadyToPoll
                    } else {
                        SyncWaitOutcome::ContextChanged
                    }
                }
                None => {
                    *trigger_rx = None;
                    SyncWaitOutcome::ContextChanged
                }
            }
        }
    }
}

async fn wait_for_outlook_policy_delay(
    policy: &RealtimePollPolicy,
    backoff: &SyncBackoff,
    runtime: &mut RealtimeRuntimeState,
    stop_rx: &mut watch::Receiver<bool>,
    trigger_rx: &mut Option<mpsc::UnboundedReceiver<SyncTrigger>>,
) -> bool {
    loop {
        let wait = policy.next_delay(runtime.context(backoff.failure_count(), Instant::now()));
        match wait_for_outlook_delay(wait, runtime, stop_rx, trigger_rx).await {
            SyncWaitOutcome::Stop => return true,
            SyncWaitOutcome::ReadyToPoll => return false,
            SyncWaitOutcome::ContextChanged => continue,
        }
    }
}

async fn collect_outlook_delta_pages<F, Fut>(
    folder_id: &str,
    stored_cursor: Option<&str>,
    mut fetch_page: F,
) -> Result<OutlookDeltaBatch>
where
    F: FnMut(String, Option<String>) -> Fut,
    Fut: Future<Output = Result<OutlookDeltaPage>>,
{
    let mut messages = Vec::new();
    let mut deleted_remote_ids = Vec::new();
    let mut cursor = stored_cursor.map(ToOwned::to_owned);

    loop {
        let page = fetch_page(folder_id.to_string(), cursor.take()).await?;
        messages.extend(page.messages);
        deleted_remote_ids.extend(page.deleted_remote_ids);

        if let Some(delta_link) = page.delta_link {
            return Ok(OutlookDeltaBatch {
                messages,
                deleted_remote_ids,
                delta_link: Some(delta_link),
            });
        }

        match page.next_link {
            Some(next_link) if !next_link.is_empty() => cursor = Some(next_link),
            _ => {
                return Ok(OutlookDeltaBatch {
                    messages,
                    deleted_remote_ids,
                    delta_link: None,
                });
            }
        }
    }
}

fn outlook_delta_cursor_key(folder_remote_id: &str) -> String {
    format!("outlook_delta:{folder_remote_id}")
}

fn parse_outlook_delta_cursor(folder_remote_id: &str, state: Option<&str>) -> Option<String> {
    let state = state?.trim();
    if state.is_empty() {
        return None;
    }
    if state.starts_with("https://") {
        return Some(state.to_string());
    }
    let key = outlook_delta_cursor_key(folder_remote_id);
    serde_json::from_str::<serde_json::Value>(state)
        .ok()
        .and_then(|value| value.get(&key).and_then(|cursor| cursor.as_str()).map(str::to_string))
}

fn serialize_outlook_delta_cursor(folder_remote_id: &str, delta_link: &str) -> String {
    serde_json::json!({
        outlook_delta_cursor_key(folder_remote_id): delta_link,
    })
    .to_string()
}

fn can_advance_outlook_delta_cursor(failure_count: u32) -> bool {
    failure_count == 0
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
struct OutlookDeletionOutcome {
    deleted_count: usize,
    failure_count: u32,
}

fn apply_outlook_deleted_remote_ids_from_existing<F>(
    existing: &std::collections::HashMap<String, String>,
    deleted_remote_ids: &[String],
    mut soft_delete: F,
) -> OutlookDeletionOutcome
where
    F: FnMut(&str) -> Result<()>,
{
    let mut outcome = OutlookDeletionOutcome::default();
    for remote_id in deleted_remote_ids {
        if let Some(message_id) = existing.get(remote_id) {
            match soft_delete(message_id) {
                Ok(()) => outcome.deleted_count += 1,
                Err(e) => {
                    warn!("Failed to apply Outlook tombstone for message {message_id}: {e}");
                    outcome.failure_count += 1;
                }
            }
        }
    }
    outcome
}

fn apply_outlook_deleted_remote_ids(
    store: &Store,
    account_id: &str,
    deleted_remote_ids: &[String],
) -> Result<OutlookDeletionOutcome> {
    if deleted_remote_ids.is_empty() {
        return Ok(OutlookDeletionOutcome::default());
    }

    let existing = store.get_existing_message_map_by_remote_ids(account_id, deleted_remote_ids)?;
    Ok(apply_outlook_deleted_remote_ids_from_existing(
        &existing,
        deleted_remote_ids,
        |message_id| store.soft_delete_message(message_id),
    ))
}

fn update_outlook_backoff_after_sync(backoff: &mut SyncBackoff, failure_count: u32) {
    if failure_count == 0 {
        backoff.record_success();
    } else {
        let _ = backoff.record_failure();
    }
}

/// A sync worker for Outlook accounts using the Microsoft Graph API.
pub struct OutlookSyncWorker {
    pub(crate) base: SyncWorkerBase,
    provider: Arc<OutlookProvider>,
    token_refresher: Option<Arc<TokenRefresher>>,
    /// Last known token expiry (unix timestamp).
    token_expires_at: StdMutex<Option<i64>>,
}

impl OutlookSyncWorker {
    pub fn new(
        account_id: impl Into<String>,
        provider: Arc<OutlookProvider>,
        store: Arc<Store>,
        attachments_dir: impl Into<PathBuf>,
    ) -> Self {
        Self {
            base: SyncWorkerBase {
                account_id: account_id.into(),
                store,
                attachments_dir: attachments_dir.into(),
                error_tx: None,
                message_tx: None,
                runtime_status_tx: None,
            },
            provider,
            token_refresher: None,
            token_expires_at: StdMutex::new(None),
        }
    }

    pub fn with_error_tx(mut self, tx: mpsc::UnboundedSender<SyncError>) -> Self {
        self.base.error_tx = Some(tx);
        self
    }

    pub fn with_message_tx(mut self, tx: mpsc::UnboundedSender<StoredMessage>) -> Self {
        self.base.message_tx = Some(tx);
        self
    }

    pub fn with_token_refresher(mut self, refresher: TokenRefresher, expires_at: Option<i64>) -> Self {
        self.token_refresher = Some(Arc::new(refresher));
        *self.token_expires_at.lock().unwrap_or_else(|e| e.into_inner()) = expires_at;
        self
    }

    pub fn with_token_expires_at(self, expires_at: Option<i64>) -> Self {
        *self.token_expires_at.lock().unwrap_or_else(|e| e.into_inner()) = expires_at;
        self
    }

    /// Ensure the access token is still valid; refresh if needed.
    async fn ensure_valid_token(&self) -> Result<()> {
        let now = now_timestamp();
        let needs_refresh = {
            let expires = self.token_expires_at.lock().unwrap_or_else(|e| e.into_inner());
            match *expires {
                Some(exp) => now >= exp - 60,
                None => false,
            }
        };

        if needs_refresh {
            if let Some(ref refresher) = self.token_refresher {
                match refresher().await {
                    Ok((new_token, new_expires_at)) => {
                        self.provider.set_access_token(new_token);
                        let mut expires = self.token_expires_at.lock().unwrap_or_else(|e| e.into_inner());
                        *expires = new_expires_at.or(Some(now + 3600));
                        info!("Outlook OAuth token refreshed for account {}", self.base.account_id);
                    }
                    Err(e) => {
                        warn!("Failed to refresh Outlook OAuth token: {}", e);
                        self.base.emit_error("token_refresh", &format!("Outlook token refresh failed: {e}"));
                        return Err(PebbleError::Auth(format!("Outlook token refresh failed: {e}")));
                    }
                }
            }
        }
        Ok(())
    }

    async fn persist_folder_messages(&self, folder: &Folder, messages: Vec<Message>, notify_new: bool) -> u32 {
        let remote_ids: Vec<String> = messages.iter().map(|m| m.remote_id.clone()).collect();
        let existing = self
            .base.store
            .get_existing_remote_ids(&self.base.account_id, &remote_ids)
            .unwrap_or_default();

        let ref_ids: Vec<String> = {
            let mut refs = std::collections::HashSet::new();
            for msg in &messages {
                if let Some(irt) = &msg.in_reply_to {
                    for id in irt.split_whitespace() {
                        refs.insert(id.trim().to_string());
                    }
                }
                if let Some(r) = &msg.references_header {
                    for id in r.split_whitespace() {
                        refs.insert(id.trim().to_string());
                    }
                }
            }
            refs.into_iter().collect()
        };

        let mut thread_mappings = self
            .base
            .store
            .get_thread_mappings_for_refs(&self.base.account_id, &ref_ids)
            .unwrap_or_default();
        let mut failure_count = 0;

        for msg in &messages {
            if existing.contains(&msg.remote_id) {
                continue;
            }

            let mut msg = msg.clone();
            let thread_id = compute_thread_id(&msg, &thread_mappings);
            msg.thread_id = Some(thread_id);

            let folder_ids = vec![folder.id.clone()];
            if let Err(e) = self.base.store.insert_message(&msg, &folder_ids) {
                warn!("Failed to store Outlook message: {e}");
                failure_count += 1;
                continue;
            }

            if let (Some(mid), Some(tid)) = (&msg.message_id_header, &msg.thread_id) {
                thread_mappings.insert(mid.clone(), tid.clone());
            }

            if msg.has_attachments {
                match self.provider.list_message_attachments(&msg.remote_id).await {
                    Ok(attachments) if !attachments.is_empty() => {
                        persist_message_attachments_async(
                            Arc::clone(&self.base.store),
                            self.base.attachments_dir.clone(),
                            msg.id.clone(),
                            attachments,
                        )
                        .await;
                    }
                    Ok(_) => {}
                    Err(e) => {
                        warn!(
                            "Failed to fetch Outlook attachments for {}: {e}",
                            msg.remote_id
                        );
                    }
                }
            }

            self.base.emit_message(StoredMessage {
                message: msg.clone(),
                folder_ids,
                notify: notify_new,
            });
        }

        failure_count
    }

    /// Main sync loop.
    pub async fn run(
        &self,
        config: SyncConfig,
        mut stop_rx: watch::Receiver<bool>,
        trigger_rx: Option<mpsc::UnboundedReceiver<SyncTrigger>>,
    ) {
        let policy = RealtimePollPolicy::from_foreground_interval_secs(config.poll_interval_secs);
        let mut backoff = SyncBackoff::new();
        let mut trigger_rx = trigger_rx;
        let mut runtime = RealtimeRuntimeState::new(Duration::from_secs(60), Instant::now());

        'sync_loop: loop {
            if *stop_rx.borrow() {
                break;
            }

            // Check circuit breaker at start of each iteration
            if backoff.is_circuit_open() {
                let delay = backoff.current_delay();
                warn!(
                    "Circuit open for Outlook account {} ({} failures), waiting {:?}",
                    self.base.account_id, backoff.failure_count(), delay
                );
                loop {
                    match wait_for_outlook_delay(delay, &mut runtime, &mut stop_rx, &mut trigger_rx).await {
                        SyncWaitOutcome::Stop => break 'sync_loop,
                        SyncWaitOutcome::ReadyToPoll => break,
                        SyncWaitOutcome::ContextChanged => continue,
                    }
                }
                continue;
            }

            // Refresh token if needed
            if let Err(e) = self.ensure_valid_token().await {
                warn!("Outlook token validation failed: {}", e);
                self.base.emit_error("auth", &format!("Token validation failed: {e}"));
                let _ = backoff.record_failure();
                if backoff.is_circuit_open() {
                    let delay = backoff.current_delay();
                    loop {
                        match wait_for_outlook_delay(delay, &mut runtime, &mut stop_rx, &mut trigger_rx).await {
                            SyncWaitOutcome::Stop => break 'sync_loop,
                            SyncWaitOutcome::ReadyToPoll => break,
                            SyncWaitOutcome::ContextChanged => continue,
                        }
                    }
                }
                continue;
            }

            // List folders and fetch messages per folder
            let folders = match self.provider.list_folders().await {
                Ok(f) => f,
                Err(e) => {
                    warn!("Outlook folder list failed: {e}");
                    self.base.emit_error("sync", &format!("Outlook folder list failed: {e}"));
                    let delay = backoff.record_failure();
                    if backoff.is_circuit_open() {
                        warn!(
                            "Circuit open for Outlook account {} ({} failures), waiting {:?}",
                            self.base.account_id, backoff.failure_count(), delay
                        );
                    }
                    if backoff.is_circuit_open() {
                        loop {
                            match wait_for_outlook_delay(delay, &mut runtime, &mut stop_rx, &mut trigger_rx).await {
                                SyncWaitOutcome::Stop => break 'sync_loop,
                                SyncWaitOutcome::ReadyToPoll => break,
                                SyncWaitOutcome::ContextChanged => continue,
                            }
                        }
                    } else {
                        if wait_for_outlook_policy_delay(&policy, &backoff, &mut runtime, &mut stop_rx, &mut trigger_rx).await {
                            break 'sync_loop;
                        }
                    }
                    continue;
                }
            };

            for folder in &folders {
                // Persist folder
                let _ = self.base.store.insert_folder(folder);
            }

            // Re-read folders from DB so we use persisted IDs (upsert may keep old IDs)
            let db_folders = self.base.store.list_folders(&self.base.account_id).unwrap_or_default();
            let mut sync_failure_count = 0u32;

            for folder in &db_folders {
                let state = self
                    .base
                    .store
                    .get_folder_sync_state(&self.base.account_id, &folder.id)
                    .ok()
                    .flatten();
                let cursor = parse_outlook_delta_cursor(&folder.remote_id, state.as_deref());
                let notify_new = cursor.is_some();

                match collect_outlook_delta_pages(&folder.remote_id, cursor.as_deref(), |folder_id, cursor| {
                    let provider = Arc::clone(&self.provider);
                    async move { provider.fetch_delta_page(&folder_id, cursor.as_deref()).await }
                })
                .await
                {
                    Ok(batch) => {
                        let mut failure_count = self.persist_folder_messages(folder, batch.messages, notify_new).await;

                        match apply_outlook_deleted_remote_ids(
                            &self.base.store,
                            &self.base.account_id,
                            &batch.deleted_remote_ids,
                        ) {
                            Ok(outcome) => {
                                failure_count += outcome.failure_count;
                            }
                            Err(e) => {
                                warn!("Failed to load Outlook tombstones for folder {}: {e}", folder.name);
                                failure_count += 1;
                            }
                        }
                        sync_failure_count += failure_count;

                        if let Some(delta_link) = batch.delta_link {
                            if can_advance_outlook_delta_cursor(failure_count) {
                                let state = serialize_outlook_delta_cursor(&folder.remote_id, &delta_link);
                                if let Err(e) = self.base.store.set_folder_sync_state(
                                    &self.base.account_id,
                                    &folder.id,
                                    &state,
                                ) {
                                    warn!("Failed to persist Outlook delta cursor for folder {}: {e}", folder.name);
                                }
                            } else {
                                warn!(
                                    "Outlook delta sync for folder {} had {} failures; keeping previous cursor",
                                    folder.name, failure_count
                                );
                            }
                        }
                    }
                    Err(e) => {
                        warn!("Outlook delta sync failed for folder {}: {e}", folder.name);
                        sync_failure_count += 1;
                    }
                }

                if *stop_rx.borrow() {
                    break;
                }
            }

            if config.manual_only() {
                break;
            }

            update_outlook_backoff_after_sync(&mut backoff, sync_failure_count);

            if wait_for_outlook_policy_delay(&policy, &backoff, &mut runtime, &mut stop_rx, &mut trigger_rx).await {
                break;
            }
        }

        info!("Outlook sync task completed for account {}", self.base.account_id);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_core::{Account, Folder, FolderRole, FolderType, ProviderType};
    use std::collections::VecDeque;

    #[test]
    fn outlook_delta_cursor_key_is_folder_scoped() {
        assert_eq!(
            outlook_delta_cursor_key("folder-1"),
            "outlook_delta:folder-1"
        );
    }

    #[test]
    fn outlook_delta_cursor_advances_only_without_failures() {
        assert!(can_advance_outlook_delta_cursor(0));
        assert!(!can_advance_outlook_delta_cursor(1));
    }

    fn make_message(remote_id: &str) -> Message {
        let now = now_timestamp();
        Message {
            id: format!("local-{remote_id}"),
            account_id: "account-1".to_string(),
            remote_id: remote_id.to_string(),
            message_id_header: None,
            in_reply_to: None,
            references_header: None,
            thread_id: None,
            subject: remote_id.to_string(),
            snippet: String::new(),
            from_address: String::new(),
            from_name: String::new(),
            to_list: Vec::new(),
            cc_list: Vec::new(),
            bcc_list: Vec::new(),
            body_text: String::new(),
            body_html_raw: String::new(),
            has_attachments: false,
            is_read: true,
            is_starred: false,
            is_draft: false,
            date: now,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    fn make_account() -> Account {
        Account {
            id: "account-1".to_string(),
            email: "user@example.com".to_string(),
            display_name: "User".to_string(),
            provider: ProviderType::Outlook,
            created_at: now_timestamp(),
            updated_at: now_timestamp(),
        }
    }

    fn make_folder(remote_id: &str) -> Folder {
        Folder {
            id: format!("folder-{remote_id}"),
            account_id: "account-1".to_string(),
            remote_id: remote_id.to_string(),
            name: remote_id.to_string(),
            folder_type: FolderType::Folder,
            role: Some(FolderRole::Inbox),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 0,
        }
    }

    #[tokio::test]
    async fn collect_outlook_delta_pages_follows_next_link_until_delta_link() {
        let mut pages = VecDeque::from([
            OutlookDeltaPage {
                messages: vec![make_message("outlook-1")],
                deleted_remote_ids: vec!["deleted-1".to_string()],
                next_link: Some("https://graph.example/next".to_string()),
                delta_link: None,
            },
            OutlookDeltaPage {
                messages: vec![make_message("outlook-2")],
                deleted_remote_ids: vec!["deleted-2".to_string()],
                next_link: None,
                delta_link: Some("https://graph.example/delta".to_string()),
            },
        ]);
        let mut requested = Vec::new();

        let batch = collect_outlook_delta_pages("folder-1", Some("cursor-0"), |folder_id, cursor| {
            requested.push((folder_id, cursor));
            let page = pages.pop_front().expect("expected a delta page request");
            async move { Ok(page) }
        })
        .await
        .unwrap();

        let remote_ids: Vec<_> = batch.messages.into_iter().map(|m| m.remote_id).collect();
        assert_eq!(remote_ids, vec!["outlook-1".to_string(), "outlook-2".to_string()]);
        assert_eq!(
            batch.deleted_remote_ids,
            vec!["deleted-1".to_string(), "deleted-2".to_string()]
        );
        assert_eq!(batch.delta_link.as_deref(), Some("https://graph.example/delta"));
        assert_eq!(
            requested,
            vec![
                ("folder-1".to_string(), Some("cursor-0".to_string())),
                (
                    "folder-1".to_string(),
                    Some("https://graph.example/next".to_string())
                ),
            ]
        );
    }

    #[test]
    fn outlook_tombstones_soft_delete_existing_messages() {
        let store = Store::open_in_memory().unwrap();
        let account = make_account();
        let folder = make_folder("inbox");
        let message = make_message("deleted-1");

        store.insert_account(&account).unwrap();
        store.insert_folder(&folder).unwrap();
        store.insert_message(&message, &[folder.id.clone()]).unwrap();

        let outcome = apply_outlook_deleted_remote_ids(
            &store,
            "account-1",
            &["deleted-1".to_string(), "missing".to_string()],
        )
        .unwrap();

        assert_eq!(outcome.deleted_count, 1);
        assert_eq!(outcome.failure_count, 0);
        assert!(store.get_message(&message.id).unwrap().unwrap().is_deleted);
    }

    #[test]
    fn outlook_tombstones_continue_after_per_message_delete_failure() {
        let existing = [
            ("deleted-1".to_string(), "message-1".to_string()),
            ("deleted-2".to_string(), "message-2".to_string()),
        ]
        .into_iter()
        .collect();
        let mut attempted = Vec::new();

        let outcome = apply_outlook_deleted_remote_ids_from_existing(
            &existing,
            &["deleted-1".to_string(), "deleted-2".to_string()],
            |message_id: &str| {
                attempted.push(message_id.to_string());
                if message_id == "message-1" {
                    Err(PebbleError::Storage("delete failed".to_string()))
                } else {
                    Ok(())
                }
            },
        );

        assert_eq!(attempted, vec!["message-1".to_string(), "message-2".to_string()]);
        assert_eq!(outcome.deleted_count, 1);
        assert_eq!(outcome.failure_count, 1);
    }

    #[test]
    fn outlook_backoff_records_failure_after_folder_sync_failures() {
        let mut backoff = SyncBackoff::new();

        update_outlook_backoff_after_sync(&mut backoff, 2);
        assert_eq!(backoff.failure_count(), 1);

        update_outlook_backoff_after_sync(&mut backoff, 0);
        assert_eq!(backoff.failure_count(), 0);
    }

}
