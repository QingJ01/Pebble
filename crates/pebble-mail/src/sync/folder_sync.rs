use super::*;

impl SyncWorker {
    pub(super) fn refresh_inbox_local_uid_baseline(
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

    pub(super) async fn try_imap_folder_cursor_for_sync(
        &self,
        folder: &pebble_core::Folder,
    ) -> Result<(ImapFolderCursor, bool)> {
        let cursor = self.stored_imap_folder_cursor_async(folder).await?;
        if !should_attempt_imap_remote_folder(folder) {
            return Ok((cursor, false));
        }
        let status = self
            .provider
            .inner()
            .get_mailbox_status(&folder.remote_id)
            .await?;
        let current_uidvalidity = status.uid_validity.map(u64::from);
        let changed = imap_uidvalidity_changed(cursor.uidvalidity, current_uidvalidity);
        Ok((
            prepare_imap_folder_cursor_for_status(
                cursor,
                current_uidvalidity,
                status.highest_modseq,
            ),
            changed,
        ))
    }

    pub(super) async fn stored_imap_folder_cursor_async(
        &self,
        folder: &pebble_core::Folder,
    ) -> Result<ImapFolderCursor> {
        let base_store = Arc::clone(&self.base.store);
        let account_id = self.base.account_id.clone();
        let folder = folder.clone();
        tokio::task::spawn_blocking(move || {
            let worker_view = ImapCursorReader {
                store: &base_store,
                account_id: &account_id,
            };
            worker_view.stored_cursor(&folder)
        })
        .await
        .map_err(|e| PebbleError::Internal(format!("spawn_blocking: {e}")))?
    }

    /// Persist an in-progress cursor (backfill floor, advanced `last_uid`,
    /// retry list) so an interrupted sync resumes from here.
    pub(super) async fn checkpoint_imap_folder_cursor(
        &self,
        folder: &pebble_core::Folder,
        cursor: &mut ImapFolderCursor,
    ) -> Result<()> {
        let account_id = self.base.account_id.clone();
        let sync_since = blocking_store(&self.base.store, move |store| {
            Ok(store
                .get_sync_state(&account_id)?
                .and_then(|state| state.sync_since))
        })
        .await?;
        cursor.sync_since = sync_since;
        cursor.has_sync_scope = true;
        let state = serialize_imap_folder_cursor(cursor).ok_or_else(|| {
            PebbleError::Internal("Failed to serialize IMAP folder sync cursor".to_string())
        })?;
        let account_id = self.base.account_id.clone();
        let folder_id = folder.id.clone();
        blocking_store(&self.base.store, move |store| {
            store.set_folder_sync_state(&account_id, &folder_id, &state)
        })
        .await
    }

    pub(super) async fn persist_imap_folder_cursor_after_sync(
        &self,
        folder: &pebble_core::Folder,
        mut cursor: ImapFolderCursor,
        snapshot_uid_next: Option<u32>,
    ) -> Result<()> {
        let account_id = self.base.account_id.clone();
        let folder_id = folder.id.clone();
        let (has_failures, stored_state, sync_since, local_max_uid) =
            blocking_store(&self.base.store, move |store| {
                Ok((
                    store.has_sync_failures_for_folder(&account_id, &folder_id)?,
                    store.get_folder_sync_state(&account_id, &folder_id)?,
                    store
                        .get_sync_state(&account_id)?
                        .and_then(|state| state.sync_since),
                    store
                        .get_max_remote_id(&account_id, &folder_id)?
                        .and_then(|s| s.parse::<u32>().ok()),
                ))
            })
            .await?;

        // Within one UIDVALIDITY generation, failed UIDs live in the cursor's
        // retry list and must not hold the cursor back. A new generation is
        // only committed when it was stored without unresolved failures.
        let stored_cursor = parse_imap_folder_cursor(stored_state.as_deref());
        let generation_changed =
            imap_uidvalidity_changed(stored_cursor.uidvalidity, cursor.uidvalidity);
        if generation_changed && !can_advance_imap_folder_cursor(has_failures) {
            debug!(
                "Keeping previous IMAP cursor for {} because the new UIDVALIDITY generation has unresolved sync failures",
                folder.name
            );
            return Ok(());
        }

        cursor.sync_since = sync_since;
        cursor.has_sync_scope = true;
        mark_imap_backfill_complete(&mut cursor, snapshot_uid_next);
        cursor.last_uid =
            completed_imap_history_high_water(cursor.last_uid, snapshot_uid_next, local_max_uid);
        if let Some(state) = serialize_imap_folder_cursor(&cursor) {
            let account_id = self.base.account_id.clone();
            let folder_id = folder.id.clone();
            blocking_store(&self.base.store, move |store| {
                store.set_folder_sync_state(&account_id, &folder_id, &state)
            })
            .await?;
        }
        Ok(())
    }

    /// Perform the initial full sync: list folders and fetch all of them.
    pub async fn initial_sync(&self) -> Result<()> {
        info!("Starting initial sync for account {}", self.base.account_id);

        let sync_state = self.base.store.get_sync_state(&self.base.account_id)?;
        let defaults_pending = sync_state
            .as_ref()
            .is_some_and(|state| state.imap_default_folders_pending);
        let mut selected_remote_ids = sync_state
            .and_then(|state| state.selected_imap_folder_remote_ids)
            .map(|ids| ids.into_iter().collect::<HashSet<_>>());

        let discovered_folders = self
            .provider
            .inner()
            .list_folders(&self.base.account_id)
            .await?;

        if defaults_pending {
            let defaults = default_imap_folder_remote_ids(&discovered_folders);
            if let Some(selected) = selected_remote_ids.as_mut() {
                selected.extend(defaults.iter().cloned());
            }
            self.base
                .store
                .update_sync_state(&self.base.account_id, |state| {
                    if let Some(ids) = state.selected_imap_folder_remote_ids.as_mut() {
                        for id in &defaults {
                            if !ids.contains(id) {
                                ids.push(id.clone());
                            }
                        }
                    }
                    state.imap_default_folders_pending = false;
                })?;
            info!(
                "Added {} default IMAP folders for new account {}",
                defaults.len(),
                self.base.account_id
            );
        }
        let remote_folders: Vec<_> = discovered_folders
            .into_iter()
            .filter(|folder| {
                should_include_discovered_imap_folder(folder, selected_remote_ids.as_ref())
            })
            .collect();

        if selected_remote_ids.is_some() {
            info!(
                "Applying explicit IMAP folder selection for account {} ({} folders including Inbox)",
                self.base.account_id,
                remote_folders.len()
            );
        }

        for folder in &remote_folders {
            // Upsert folder into store
            let _ = self.base.store.insert_folder(folder);
        }

        // Ensure an Archive folder exists locally (even if the IMAP server doesn't have one)
        let has_archive = remote_folders
            .iter()
            .any(|f| f.role == Some(pebble_core::FolderRole::Archive));
        if has_archive {
            if self.base.store.remove_empty_local_fallback_folder(
                &self.base.account_id,
                "__local_archive__",
                pebble_core::FolderRole::Archive,
            )? {
                info!(
                    "Removed local archive fallback for account {} now that the server has one",
                    self.base.account_id
                );
            }
        } else {
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

    pub(super) async fn initial_sync_folder_with_reconnect(
        &self,
        folder: &pebble_core::Folder,
    ) -> Result<()> {
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

    pub(super) async fn initial_sync_folder_once(
        &self,
        folder: &pebble_core::Folder,
    ) -> Result<()> {
        if !should_attempt_imap_remote_folder(folder) {
            debug!(
                "Skipping local-only IMAP folder {} ({}) during initial sync",
                folder.name, folder.remote_id
            );
            return Ok(());
        }

        let (count, _) = self.sync_imap_folder_with_cursor(folder, None).await?;
        if count > 0 {
            info!(
                "Initial sync: fetched {} messages from {}",
                count, folder.name
            );
        }
        Ok(())
    }

    /// Shared body of initial sync and polling for one folder: handle a
    /// UIDVALIDITY change, resume or run the history backfill, or fetch new
    /// UIDs (retrying earlier failures first), then persist the cursor.
    ///
    /// `notify_override` forces new-mail notifications on/off; `None` means
    /// "notify only when the folder already had a cursor".
    pub(super) async fn sync_imap_folder_with_cursor(
        &self,
        folder: &pebble_core::Folder,
        notify_override: Option<bool>,
    ) -> Result<(u32, Option<u32>)> {
        let (mut cursor, uidvalidity_changed) =
            self.try_imap_folder_cursor_for_sync(folder).await?;
        let since_uid = imap_incremental_cursor_uid(&cursor);
        let notify_new =
            notify_override.unwrap_or_else(|| should_notify_imap_startup_fetch(since_uid));
        let (count, snapshot_uid_next) = if uidvalidity_changed {
            let outcome = self
                .reconcile_folder_after_uidvalidity_change(folder)
                .await?;
            (outcome.count, outcome.uid_next)
        } else if let Some(uid) = since_uid {
            let retried = self.retry_failed_imap_uids(folder, &mut cursor).await?;
            let fetched = self
                .sync_new_imap_uids(folder, &mut cursor, uid, notify_new)
                .await?;
            (retried + fetched, None)
        } else {
            // No cursor yet (fresh account, folder whose local data was
            // cleared, or an interrupted backfill): backfill the mailbox,
            // resuming below the last checkpoint when there is one.
            let outcome = self
                .sync_folder_history_with_watermark(folder, &mut cursor, notify_new)
                .await?;
            (outcome.count, outcome.uid_next)
        };

        if let Err(e) = self
            .persist_imap_folder_cursor_after_sync(folder, cursor, snapshot_uid_next)
            .await
        {
            warn!("Failed to persist IMAP cursor for {}: {}", folder.name, e);
        }
        Ok((count, snapshot_uid_next))
    }

    /// Check if a folder is local-only (not backed by IMAP).
    pub(super) fn is_local_folder(folder: &pebble_core::Folder) -> bool {
        !should_attempt_imap_remote_folder(folder)
    }

    /// Sync a folder: fetch raw messages, parse, compute threads, store.
    /// Returns the number of new messages stored.
    ///
    /// This entry point does not maintain the folder cursor; the worker's
    /// own polling uses [`Self::sync_imap_folder_with_cursor`].
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

        let uids = match since_uid {
            Some(uid) => {
                self.provider
                    .inner()
                    .search_uids_after(&folder.remote_id, uid)
                    .await?
            }
            None => {
                let (mut uids, _) = self
                    .provider
                    .inner()
                    .search_uids(&folder.remote_id, "ALL")
                    .await?;
                let keep = usize::try_from(limit).unwrap_or(usize::MAX);
                if uids.len() > keep {
                    uids.drain(..uids.len() - keep);
                }
                uids
            }
        };
        let mut total = 0;
        for page in uids.chunks(HISTORY_PAGE_SIZE as usize) {
            total += self.sync_uid_page(folder, page, notify_new).await?.stored;
        }
        Ok(total)
    }

    /// Fetch UIDs newer than `since_uid` page by page, advancing and
    /// checkpointing `cursor.last_uid` after each page. UIDs that fail are
    /// queued in the retry list instead of blocking the cursor.
    pub(super) async fn sync_new_imap_uids(
        &self,
        folder: &pebble_core::Folder,
        cursor: &mut ImapFolderCursor,
        since_uid: u32,
        notify_new: bool,
    ) -> Result<u32> {
        let uids = self
            .provider
            .inner()
            .search_uids_after(&folder.remote_id, since_uid)
            .await?;
        let mut total = 0;
        for page in uids.chunks(HISTORY_PAGE_SIZE as usize) {
            let outcome = self.sync_uid_page(folder, page, notify_new).await?;
            total += outcome.stored;
            add_imap_retry_uids(cursor, &outcome.failed_uids);
            if let Some(&max_uid) = page.last() {
                cursor.last_uid = Some(cursor.last_uid.map_or(max_uid, |uid| uid.max(max_uid)));
            }
            if let Err(e) = self.checkpoint_imap_folder_cursor(folder, cursor).await {
                warn!(
                    "Failed to checkpoint IMAP cursor for {}: {}",
                    folder.name, e
                );
            }
        }
        Ok(total)
    }

    /// Retry UIDs that previously failed, keeping only those that still fail
    /// (and have retry attempts left) in the retry list.
    pub(super) async fn retry_failed_imap_uids(
        &self,
        folder: &pebble_core::Folder,
        cursor: &mut ImapFolderCursor,
    ) -> Result<u32> {
        if cursor.retry_uids.is_empty() {
            return Ok(0);
        }
        let pending = std::mem::take(&mut cursor.retry_uids);
        let account_id = self.base.account_id.clone();
        let folder_id = folder.id.clone();
        let candidates = pending.clone();
        let retryable: Vec<u32> = blocking_store(&self.base.store, move |store| {
            let mut retryable = Vec::with_capacity(candidates.len());
            for uid in candidates {
                let attempts = store
                    .get_sync_failure(&account_id, &folder_id, &uid.to_string())?
                    .map_or(0, |failure| failure.attempts);
                if attempts < MAX_IMAP_UID_RETRY_ATTEMPTS {
                    retryable.push(uid);
                }
            }
            Ok(retryable)
        })
        .await
        .unwrap_or(pending);

        let mut stored = 0;
        let mut still_failed = Vec::new();
        let pages: Vec<&[u32]> = retryable.chunks(HISTORY_PAGE_SIZE as usize).collect();
        for (index, page) in pages.iter().enumerate() {
            match self.sync_uid_page(folder, page, false).await {
                Ok(outcome) => {
                    stored += outcome.stored;
                    still_failed.extend(outcome.failed_uids);
                }
                Err(error) => {
                    for remaining in &pages[index..] {
                        still_failed.extend_from_slice(remaining);
                    }
                    add_imap_retry_uids(cursor, &still_failed);
                    return Err(error);
                }
            }
        }
        add_imap_retry_uids(cursor, &still_failed);
        Ok(stored)
    }
}
