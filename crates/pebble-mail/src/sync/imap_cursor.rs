use super::*;

#[derive(Debug, Clone, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub(super) struct ImapFolderCursor {
    pub(super) uidvalidity: Option<u64>,
    pub(super) last_uid: Option<u32>,
    pub(super) highest_modseq: Option<u64>,
    #[serde(default)]
    pub(super) sync_since: Option<i64>,
    #[serde(default)]
    pub(super) has_sync_scope: bool,
    #[serde(default)]
    pub(super) scope_backfill_required: bool,
    /// Lowest UID already handled by an unfinished history backfill. All
    /// UIDs in `[backfill_floor_uid, backfill_uid_next)` are done, so an
    /// interrupted backfill resumes below this UID instead of restarting.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) backfill_floor_uid: Option<u32>,
    /// UIDNEXT captured when the unfinished backfill started; becomes the
    /// incremental high-water mark once the backfill completes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) backfill_uid_next: Option<u32>,
    /// UIDs at or below the cursor that failed to download/parse/store.
    /// They are retried on later syncs without holding the cursor back.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(super) retry_uids: Vec<u32>,
    /// `(EXISTS, UIDNEXT)` observed when the local UID set was last verified
    /// against the server. Lets reconcile skip the deletion scan when the
    /// counts prove nothing was expunged since.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub(super) deletion_baseline: Option<ImapDeletionBaseline>,
}

#[derive(Debug, Clone, Copy, Default, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub(super) struct ImapDeletionBaseline {
    pub(super) exists: u32,
    pub(super) uid_next: u32,
}

/// Retry list bound; beyond it the oldest failures are only tracked in the
/// `sync_failures` table.
pub(super) const MAX_IMAP_RETRY_UIDS: usize = 10_000;
/// Failures retried this many times are left to the `sync_failures` table.
pub(super) const MAX_IMAP_UID_RETRY_ATTEMPTS: i64 = 5;

pub(super) fn add_imap_retry_uids(cursor: &mut ImapFolderCursor, failed: &[u32]) {
    if failed.is_empty() {
        return;
    }
    cursor.retry_uids.extend_from_slice(failed);
    cursor.retry_uids.sort_unstable();
    cursor.retry_uids.dedup();
    if cursor.retry_uids.len() > MAX_IMAP_RETRY_UIDS {
        let excess = cursor.retry_uids.len() - MAX_IMAP_RETRY_UIDS;
        cursor.retry_uids.drain(..excess);
    }
}

/// Whether the local UID set must be re-verified against the server.
///
/// Since the baseline, `added = UIDNEXT' - UIDNEXT` UIDs were assigned and
/// `EXISTS' - EXISTS = present_new - expunged_old`. Because
/// `present_new <= added`, `EXISTS' - EXISTS == added` implies no message
/// that existed at the baseline was expunged, so the scan can be skipped.
pub(super) fn imap_deletion_check_needed(
    baseline: Option<ImapDeletionBaseline>,
    exists: u32,
    uid_next: Option<u32>,
) -> bool {
    let (Some(baseline), Some(uid_next)) = (baseline, uid_next) else {
        return true;
    };
    if uid_next < baseline.uid_next {
        return true;
    }
    let added = i64::from(uid_next - baseline.uid_next);
    i64::from(exists) - i64::from(baseline.exists) != added
}

/// Turn the header section of a message too large to download into a small
/// placeholder message: original headers (minus MIME structure) plus a
/// plain-text notice, so the message is listed with sender, subject and
/// threading intact.
pub(super) fn oversized_placeholder_message(header: &[u8], size: u32) -> Vec<u8> {
    let text = String::from_utf8_lossy(header);
    let mut kept = String::with_capacity(text.len() + 256);
    let mut skipping = false;
    for line in text.split_inclusive('\n') {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.is_empty() {
            break;
        }
        let is_continuation = line.starts_with([' ', '\t']);
        if !is_continuation {
            let name = trimmed
                .split(':')
                .next()
                .unwrap_or_default()
                .trim()
                .to_ascii_lowercase();
            skipping = name == "content-type"
                || name == "content-transfer-encoding"
                || name == "mime-version";
        }
        if !skipping {
            kept.push_str(trimmed);
            kept.push_str("\r\n");
        }
    }
    let megabytes = f64::from(size) / (1024.0 * 1024.0);
    kept.push_str("MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\n\r\n");
    kept.push_str(&format!(
        "This message is too large to download ({megabytes:.1} MB). Open it in your mail provider's web interface to read it.\r\n"
    ));
    kept.into_bytes()
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(super) struct ImapStoreOutcome {
    pub(super) stored: u32,
    pub(super) failed_uids: Vec<u32>,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(super) struct ImapHistorySyncOutcome {
    pub(super) count: u32,
    pub(super) uid_next: Option<u32>,
}

pub(super) fn imap_incremental_cursor_uid(cursor: &ImapFolderCursor) -> Option<u32> {
    (!cursor.scope_backfill_required && cursor.backfill_floor_uid.is_none())
        .then_some(cursor.last_uid)
        .flatten()
}

pub(super) fn completed_imap_history_high_water(
    previous_uid: Option<u32>,
    uid_next: Option<u32>,
    local_max_uid: Option<u32>,
) -> Option<u32> {
    [
        previous_uid,
        uid_next.and_then(|next| next.checked_sub(1)),
        local_max_uid,
    ]
    .into_iter()
    .flatten()
    .max()
}

pub(super) fn mark_imap_backfill_complete(
    cursor: &mut ImapFolderCursor,
    snapshot_uid_next: Option<u32>,
) {
    if snapshot_uid_next.is_some() {
        cursor.scope_backfill_required = false;
        cursor.backfill_floor_uid = None;
        cursor.backfill_uid_next = None;
    }
}

pub(super) fn set_imap_cursor_modseq(cursor: &mut ImapFolderCursor, new_modseq: u64) {
    cursor.highest_modseq = Some(new_modseq);
}

pub(super) fn parse_imap_folder_cursor(state: Option<&str>) -> ImapFolderCursor {
    match state {
        Some(raw) => serde_json::from_str(raw).unwrap_or_default(),
        None => ImapFolderCursor::default(),
    }
}

pub(super) fn imap_uidvalidity_changed(stored: Option<u64>, current: Option<u64>) -> bool {
    matches!((stored, current), (Some(stored), Some(current)) if stored != current)
}

pub(super) fn prepare_imap_folder_cursor_for_status(
    mut cursor: ImapFolderCursor,
    uidvalidity: Option<u64>,
    highest_modseq: Option<u64>,
) -> ImapFolderCursor {
    let uidvalidity_changed = imap_uidvalidity_changed(cursor.uidvalidity, uidvalidity);
    if uidvalidity_changed {
        cursor.last_uid = None;
        cursor.highest_modseq = None;
        cursor.scope_backfill_required = true;
        cursor.backfill_floor_uid = None;
        cursor.backfill_uid_next = None;
        cursor.retry_uids.clear();
        cursor.deletion_baseline = None;
    }
    if uidvalidity.is_some() {
        cursor.uidvalidity = uidvalidity;
    }
    // HIGHESTMODSEQ is deliberately not copied from SELECT: the stored value
    // means "flags applied up to here" and only reconcile may advance it.
    // Copying the server value would make reconcile skip flag changes that
    // were never applied locally.
    let _ = highest_modseq;
    cursor
}

pub(super) fn prepare_imap_folder_cursor_for_sync_scope(
    mut cursor: ImapFolderCursor,
    sync_since: Option<i64>,
) -> ImapFolderCursor {
    let scope_matches = if cursor.has_sync_scope {
        cursor.sync_since == sync_since
    } else {
        sync_since.is_none()
    };
    if !scope_matches {
        cursor.sync_since = sync_since;
        cursor.has_sync_scope = true;
        cursor.scope_backfill_required = true;
        cursor.backfill_floor_uid = None;
        cursor.backfill_uid_next = None;
    } else if !cursor.has_sync_scope && cursor.last_uid.is_none() {
        cursor.scope_backfill_required = true;
    }
    cursor
}

pub(super) fn serialize_imap_folder_cursor(cursor: &ImapFolderCursor) -> Option<String> {
    serde_json::to_string(cursor).ok()
}

pub(super) fn can_advance_imap_folder_cursor(has_unresolved_failures: bool) -> bool {
    !has_unresolved_failures
}

pub(super) fn should_run_imap_deletion_diff(_server_exists: u32, local_count: usize) -> bool {
    local_count > 0
}

pub(super) fn can_seed_imap_polling_baseline_after_startup(initial_sync_succeeded: bool) -> bool {
    initial_sync_succeeded
}

pub(super) fn can_refresh_imap_polling_baseline_after_idle_fallback(
    catch_up_succeeded: bool,
) -> bool {
    catch_up_succeeded
}

pub(super) fn apply_local_inbox_uid_baseline(
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

pub(super) fn should_skip_missing_imap_mailbox_during_initial_sync(
    folder_role: Option<pebble_core::FolderRole>,
) -> bool {
    folder_role != Some(pebble_core::FolderRole::Inbox)
}

pub(super) fn should_fail_initial_sync_for_folder_error(
    folder_role: Option<pebble_core::FolderRole>,
    is_retryable: bool,
) -> bool {
    folder_role == Some(pebble_core::FolderRole::Inbox) || is_retryable
}

pub(super) fn idle_check_recovery_user_error(
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

pub(super) fn is_retryable_imap_connection_error(error: &PebbleError) -> bool {
    let PebbleError::Network(message) = error else {
        return false;
    };
    let lower = message.to_ascii_lowercase();

    lower.contains("os error 10053")
        || lower.contains("connection reset")
        || lower.contains("connection aborted")
        || lower.contains("broken pipe")
        || lower.contains("connection closed")
        || lower.contains("closed connection")
        || lower.contains("tls close_notify")
        || lower.contains("unexpected eof")
        || lower.contains("unexpected-eof")
        || lower.contains("timed out")
        || lower == "not connected"
}

pub(super) fn is_missing_imap_mailbox_error(error: &PebbleError) -> bool {
    let PebbleError::Network(message) = error else {
        return false;
    };
    let lower = message.to_ascii_lowercase();

    lower.contains("folder not exist")
        || lower.contains("mailbox does not exist")
        || lower.contains("mailbox doesn't exist")
        || lower.contains("no such mailbox")
}

pub(super) fn should_attempt_imap_remote_folder(folder: &pebble_core::Folder) -> bool {
    !folder.remote_id.starts_with("__local_")
}

/// `(uid, is_read, is_starred)` as reported by the server.
pub(super) type RemoteFlagState = (u32, bool, bool);

pub(super) fn local_uids(
    local_state: &[pebble_store::messages::FolderRemoteMessageState],
) -> Vec<u32> {
    local_state
        .iter()
        .filter_map(|(_, remote_id, _, _, _)| remote_id.parse().ok())
        .collect()
}

/// Page size used when backfilling a folder's full history.
pub(super) const HISTORY_PAGE_SIZE: u32 = 200;

/// Inclusive sequence-number ranges covering a mailbox from newest to
/// oldest, each at most `page_size` messages long. `exists == 0` yields no
/// ranges.
pub(super) fn history_page_ranges(exists: u32, page_size: u32) -> Vec<(u32, u32)> {
    let mut ranges = Vec::new();
    let mut end = exists;
    while end >= 1 {
        let start = if end > page_size {
            end - page_size + 1
        } else {
            1
        };
        ranges.push((start, end));
        if start == 1 {
            break;
        }
        end = start - 1;
    }
    ranges
}

pub(super) fn imap_uidvalidity_snapshot_plan(
    exists: u32,
    uid_next: Option<u32>,
) -> (Vec<(u32, u32)>, Option<u32>) {
    (history_page_ranges(exists, HISTORY_PAGE_SIZE), uid_next)
}

pub(super) fn should_include_discovered_imap_folder(
    folder: &pebble_core::Folder,
    selected_remote_ids: Option<&HashSet<String>>,
) -> bool {
    let Some(selected_remote_ids) = selected_remote_ids else {
        // Existing accounts without an explicit preference retain the legacy
        // behaviour of syncing every selectable mailbox.
        return true;
    };

    folder.role == Some(pebble_core::FolderRole::Inbox)
        || folder.remote_id.eq_ignore_ascii_case("INBOX")
        || selected_remote_ids.contains(&folder.remote_id)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ImapPollScope {
    Realtime,
    Full,
}

pub(super) fn should_poll_imap_folder_for_scope(
    folder: &pebble_core::Folder,
    scope: ImapPollScope,
) -> bool {
    if !should_attempt_imap_remote_folder(folder) {
        return false;
    }

    match scope {
        ImapPollScope::Realtime => folder.role == Some(pebble_core::FolderRole::Inbox),
        ImapPollScope::Full => true,
    }
}

#[cfg(test)]
pub(super) fn should_poll_imap_folder_for_realtime(folder: &pebble_core::Folder) -> bool {
    should_poll_imap_folder_for_scope(folder, ImapPollScope::Realtime)
}

/// Reads a folder's IMAP cursor; usable from blocking threads.
pub(super) struct ImapCursorReader<'a> {
    pub(super) store: &'a Store,
    pub(super) account_id: &'a str,
}

impl ImapCursorReader<'_> {
    pub(super) fn stored_cursor(&self, folder: &pebble_core::Folder) -> Result<ImapFolderCursor> {
        let state = self
            .store
            .get_folder_sync_state(self.account_id, &folder.id)?;
        let sync_since = self
            .store
            .get_sync_state(self.account_id)?
            .and_then(|state| state.sync_since);
        let mut cursor = prepare_imap_folder_cursor_for_sync_scope(
            parse_imap_folder_cursor(state.as_deref()),
            sync_since,
        );
        let has_failures = self
            .store
            .has_sync_failures_for_folder(self.account_id, &folder.id)?;
        if cursor.last_uid.is_none()
            && !cursor.scope_backfill_required
            && cursor.backfill_floor_uid.is_none()
            && can_advance_imap_folder_cursor(has_failures)
        {
            cursor.last_uid = self
                .store
                .get_max_remote_id(self.account_id, &folder.id)?
                .and_then(|s| s.parse::<u32>().ok());
        }
        Ok(cursor)
    }
}
