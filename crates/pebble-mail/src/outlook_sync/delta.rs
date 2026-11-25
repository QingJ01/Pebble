use super::*;

/// Delta pages processed per folder in one sync pass. Progress is
/// checkpointed per page, so a larger folder simply continues on the next
/// pass instead of failing.
pub(super) const MAX_OUTLOOK_DELTA_PAGES_PER_RUN: usize = MAX_GRAPH_CONTINUATION_PAGES;

/// Pagination state machine for Graph delta queries: validates every
/// continuation URL before it is requested, detects loops, and bounds the
/// number of pages per run.
pub(super) struct OutlookDeltaPager {
    pub(super) cursor: Option<String>,
    pub(super) visited: std::collections::HashSet<String>,
    pub(super) page_count: usize,
    pub(super) max_pages: usize,
}

pub(super) enum OutlookPageStep {
    Next(String),
    Done(Option<String>),
}

impl OutlookDeltaPager {
    pub(super) fn new(start: Option<&str>, max_pages: usize) -> Self {
        Self {
            cursor: start.map(ToOwned::to_owned),
            visited: std::collections::HashSet::new(),
            page_count: 0,
            max_pages,
        }
    }

    /// The cursor for the next request; `Ok(None)` when the per-run page
    /// budget is exhausted.
    pub(super) fn next_request(&mut self) -> Result<Option<Option<String>>> {
        if self.page_count == self.max_pages {
            return Ok(None);
        }
        if let Some(continuation) = self.cursor.as_deref() {
            validate_graph_continuation_url(continuation)?;
            if !self.visited.insert(continuation.to_string()) {
                return Err(PebbleError::Network(
                    "Outlook delta pagination returned a repeated continuation URL".to_string(),
                ));
            }
        }
        self.page_count += 1;
        Ok(Some(self.cursor.take()))
    }

    pub(super) fn advance(
        &mut self,
        next_link: Option<String>,
        delta_link: Option<String>,
    ) -> Result<OutlookPageStep> {
        if let Some(delta_link) = delta_link {
            validate_graph_continuation_url(&delta_link)?;
            return Ok(OutlookPageStep::Done(Some(delta_link)));
        }
        match next_link {
            Some(next_link) if !next_link.is_empty() => {
                self.cursor = Some(next_link.clone());
                Ok(OutlookPageStep::Next(next_link))
            }
            _ => Ok(OutlookPageStep::Done(None)),
        }
    }
}

pub(super) const OUTLOOK_RESUME_KEY: &str = "outlook_resume";

/// Continuation of an unfinished delta walk, stored next to the folder's
/// delta cursor so an interrupted initial sync resumes from the last page
/// that was fully persisted.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(super) struct OutlookResumeState {
    pub(super) next_link: String,
    /// The walk restarted after an expired cursor; stale messages cannot be
    /// reconciled after a resume because earlier pages are not remembered.
    #[serde(default)]
    pub(super) recovering: bool,
}

pub(super) fn parse_outlook_resume_state(state: Option<&str>) -> Option<OutlookResumeState> {
    let value: serde_json::Value = serde_json::from_str(state?.trim()).ok()?;
    serde_json::from_value(value.get(OUTLOOK_RESUME_KEY)?.clone()).ok()
}

pub(super) fn serialize_outlook_folder_state(
    folder_remote_id: &str,
    delta_link: Option<&str>,
    resume: Option<&OutlookResumeState>,
    sync_since: Option<i64>,
) -> String {
    let mut value = serde_json::json!({ "sync_since": sync_since });
    if let Some(delta_link) = delta_link {
        value[outlook_delta_cursor_key(folder_remote_id)] = serde_json::json!(delta_link);
    }
    if let Some(resume) = resume {
        value[OUTLOOK_RESUME_KEY] = serde_json::json!(resume);
    }
    value.to_string()
}

pub(super) enum SyncWaitOutcome {
    Stop,
    ReadyToPoll,
    ContextChanged,
}

pub(super) async fn wait_for_outlook_delay(
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

pub(super) async fn wait_for_outlook_policy_delay(
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

pub(super) fn should_sync_outlook_folder(folder: &Folder) -> bool {
    folder.role.is_some()
        || folder.remote_id.starts_with("__local_")
        || !should_hide_outlook_folder(Some(&folder.name), None)
}

pub(super) fn load_cached_outlook_ids(
    sync_since: Option<i64>,
    load: impl FnOnce() -> Result<std::collections::HashSet<String>>,
) -> Result<Arc<std::collections::HashSet<String>>> {
    match sync_since {
        Some(_) => load().map(Arc::new),
        None => Ok(Arc::new(std::collections::HashSet::new())),
    }
}

#[cfg(test)]
#[derive(Debug)]
pub(super) struct OutlookDeltaBatch {
    pub(super) messages: Vec<Message>,
    pub(super) deleted_remote_ids: Vec<String>,
    pub(super) delta_link: Option<String>,
}

#[cfg(test)]
#[derive(Debug)]
pub(super) struct OutlookDeltaRecoveryBatch {
    pub(super) batch: OutlookDeltaBatch,
    pub(super) recovered_from_expired_cursor: bool,
}

#[cfg(test)]
pub(super) async fn collect_outlook_delta_pages<F, Fut>(
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
    let mut pager = OutlookDeltaPager::new(stored_cursor, MAX_GRAPH_CONTINUATION_PAGES);
    loop {
        let Some(cursor) = pager.next_request()? else {
            return Err(PebbleError::Network(
                "Outlook delta pagination exceeded page limit".to_string(),
            ));
        };
        let page = fetch_page(folder_id.to_string(), cursor).await?;
        messages.extend(page.messages);
        deleted_remote_ids.extend(page.deleted_remote_ids);
        match pager.advance(page.next_link, page.delta_link)? {
            OutlookPageStep::Next(_) => {}
            OutlookPageStep::Done(delta_link) => {
                return Ok(OutlookDeltaBatch {
                    messages,
                    deleted_remote_ids,
                    delta_link,
                })
            }
        }
    }
}

#[cfg(test)]
pub(super) async fn collect_outlook_delta_pages_with_expiry_recovery<F, Fut>(
    folder_id: &str,
    stored_cursor: Option<&str>,
    mut fetch_page: F,
) -> Result<OutlookDeltaRecoveryBatch>
where
    F: FnMut(String, Option<String>) -> Fut,
    Fut: Future<Output = Result<OutlookDeltaPage>>,
{
    match collect_outlook_delta_pages(folder_id, stored_cursor, &mut fetch_page).await {
        Ok(batch) => Ok(OutlookDeltaRecoveryBatch {
            batch,
            recovered_from_expired_cursor: false,
        }),
        Err(PebbleError::SyncCursorExpired(_)) if stored_cursor.is_some() => {
            let batch = collect_outlook_delta_pages(folder_id, None, &mut fetch_page).await?;
            Ok(OutlookDeltaRecoveryBatch {
                batch,
                recovered_from_expired_cursor: true,
            })
        }
        Err(error) => Err(error),
    }
}

pub(super) fn outlook_delta_cursor_key(folder_remote_id: &str) -> String {
    format!("outlook_delta:{folder_remote_id}")
}

pub(super) fn parse_outlook_delta_cursor(
    folder_remote_id: &str,
    state: Option<&str>,
) -> Option<String> {
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
        .and_then(|value| {
            value
                .get(&key)
                .and_then(|cursor| cursor.as_str())
                .map(str::to_string)
        })
}

pub(super) fn serialize_outlook_delta_cursor_with_scope(
    folder_remote_id: &str,
    delta_link: &str,
    sync_since: Option<i64>,
) -> String {
    serde_json::json!({
        outlook_delta_cursor_key(folder_remote_id): delta_link,
        "sync_since": sync_since,
    })
    .to_string()
}

pub(super) fn outlook_delta_scope_matches(state: Option<&str>, sync_since: Option<i64>) -> bool {
    let Some(state) = state else {
        return true;
    };
    if state.trim_start().starts_with("https://") {
        return sync_since.is_none();
    }
    let Ok(value) = serde_json::from_str::<serde_json::Value>(state) else {
        return sync_since.is_none();
    };
    match value.get("sync_since") {
        Some(saved) => saved.as_i64() == sync_since,
        None => sync_since.is_none(),
    }
}

pub(super) fn can_advance_outlook_delta_cursor(failure_count: u32) -> bool {
    failure_count == 0
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(super) struct OutlookDeletionOutcome {
    pub(super) deleted_count: usize,
    pub(super) failure_count: u32,
}

pub(super) fn apply_outlook_deleted_remote_ids_from_existing<F>(
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

pub(super) fn apply_outlook_deleted_remote_ids(
    store: &Store,
    account_id: &str,
    deleted_remote_ids: &[String],
) -> Result<OutlookDeletionOutcome> {
    if deleted_remote_ids.is_empty() {
        return Ok(OutlookDeletionOutcome::default());
    }

    let mut existing =
        store.get_existing_message_map_by_remote_ids(account_id, deleted_remote_ids)?;
    // A queued local move/delete owns these messages until it reaches the
    // server; a tombstone from the source folder must not undo it.
    let pending = store.message_ids_with_active_pending_ops(
        account_id,
        &existing.values().cloned().collect::<Vec<_>>(),
    )?;
    existing.retain(|_, message_id| !pending.contains(message_id));
    Ok(apply_outlook_deleted_remote_ids_from_existing(
        &existing,
        deleted_remote_ids,
        |message_id| store.soft_delete_message(message_id),
    ))
}

pub(super) fn load_outlook_folder_snapshot(store: &Store, folder_id: &str) -> Result<Vec<Message>> {
    const PAGE_SIZE: u32 = 500;
    let mut snapshot = Vec::new();
    let mut offset = 0u32;
    loop {
        let mut page = store.list_full_messages_by_folder(folder_id, PAGE_SIZE, offset)?;
        let page_len = u32::try_from(page.len()).unwrap_or(PAGE_SIZE);
        snapshot.append(&mut page);
        if page_len < PAGE_SIZE {
            return Ok(snapshot);
        }
        offset = offset.saturating_add(page_len);
    }
}

#[cfg(test)]
pub(super) fn reconcile_outlook_fresh_snapshot(
    store: &Store,
    folder_id: &str,
    previous_snapshot: &[Message],
    fresh_remote_ids: &std::collections::HashSet<String>,
) -> OutlookDeletionOutcome {
    reconcile_outlook_fresh_snapshot_since(
        store,
        folder_id,
        previous_snapshot,
        fresh_remote_ids,
        None,
    )
}

pub(super) fn reconcile_outlook_fresh_snapshot_since(
    store: &Store,
    folder_id: &str,
    previous_snapshot: &[Message],
    fresh_remote_ids: &std::collections::HashSet<String>,
    sync_since: Option<i64>,
) -> OutlookDeletionOutcome {
    let mut outcome = OutlookDeletionOutcome::default();
    let snapshot_ids: Vec<String> = previous_snapshot.iter().map(|m| m.id.clone()).collect();
    let pending = previous_snapshot
        .first()
        .map(|message| {
            store.message_ids_with_active_pending_ops(&message.account_id, &snapshot_ids)
        })
        .transpose()
        .unwrap_or_default()
        .unwrap_or_default();
    for message in previous_snapshot {
        if fresh_remote_ids.contains(&message.remote_id)
            || sync_since.is_some_and(|cutoff| message.date < cutoff)
            || pending.contains(&message.id)
        {
            continue;
        }
        match store.remove_message_from_folder(&message.id, folder_id) {
            Ok(()) => outcome.deleted_count += 1,
            Err(error) => {
                warn!(
                    "Failed to reconcile stale Outlook folder message {}: {error}",
                    message.remote_id
                );
                outcome.failure_count += 1;
            }
        }
    }
    outcome
}

pub(super) fn update_outlook_backoff_after_sync(backoff: &mut SyncBackoff, failure_count: u32) {
    if failure_count == 0 {
        backoff.record_success();
    } else {
        let _ = backoff.record_failure();
    }
}
