use super::*;

impl SyncWorker {
    /// Poll all folders for new messages since the highest known UID.
    pub async fn poll_new_messages(&self) -> Result<()> {
        self.poll_new_messages_for_scope("poll", ImapPollScope::Realtime)
            .await
    }

    pub(super) async fn poll_all_new_messages(&self, phase: &str) -> Result<()> {
        self.poll_new_messages_for_scope(phase, ImapPollScope::Full)
            .await
    }

    pub(super) async fn poll_new_messages_for_scope(
        &self,
        phase: &str,
        scope: ImapPollScope,
    ) -> Result<()> {
        self.base.emit_sync_started(phase);
        let result = self.poll_new_messages_inner(scope).await;
        match &result {
            Ok(()) => self.base.emit_sync_completed(phase),
            Err(e) => self.base.emit_sync_error(phase, &e.to_string()),
        }
        result
    }

    pub(super) async fn poll_new_messages_inner(&self, scope: ImapPollScope) -> Result<()> {
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

    pub(super) async fn poll_imap_folder_with_reconnect(
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
                            self.base.emit_error(
                                "poll",
                                &format!(
                                    "Poll retry failed for folder {}: {}",
                                    folder.name, retry_error
                                ),
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
                self.base.emit_error(
                    "poll",
                    &format!("Poll failed for folder {}: {}", folder.name, e),
                );
                None
            }
        }
    }

    pub(super) async fn poll_imap_folder_once(&self, folder: &pebble_core::Folder) -> Result<()> {
        if !should_attempt_imap_remote_folder(folder) {
            debug!(
                "Skipping local-only IMAP folder {} ({}) during poll",
                folder.name, folder.remote_id
            );
            return Ok(());
        }

        let (count, _) = self
            .sync_imap_folder_with_cursor(folder, Some(true))
            .await?;
        if count > 0 {
            info!(
                "Polled {} new messages from {} for account {}",
                count, folder.name, self.base.account_id
            );
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
    pub(super) async fn reconcile_folder(&self, folder: &pebble_core::Folder) -> Result<()> {
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

    pub(super) async fn reconcile_folder_once(&self, folder: &pebble_core::Folder) -> Result<()> {
        // Skip local-only folders
        if Self::is_local_folder(folder) {
            return Ok(());
        }

        // Step 1: Get local state
        let account_id = self.base.account_id.clone();
        let folder_id = folder.id.clone();
        let local_state = blocking_store(&self.base.store, move |store| {
            store.list_remote_ids_by_folder(&account_id, &folder_id)
        })
        .await?;
        if !should_run_imap_deletion_diff(0, local_state.len()) {
            return Ok(());
        }

        let cursor = self.stored_imap_folder_cursor_async(folder).await?;
        let stored_modseq = cursor.highest_modseq.unwrap_or(0);

        // One SELECT gives UIDVALIDITY, HIGHESTMODSEQ, EXISTS and UIDNEXT.
        let status = self
            .provider
            .inner()
            .get_mailbox_status(&folder.remote_id)
            .await?;
        if imap_uidvalidity_changed(cursor.uidvalidity, status.uid_validity.map(u64::from)) {
            debug!(
                "UIDVALIDITY changed for {}; leaving reconcile to the next poll",
                folder.name
            );
            return Ok(());
        }

        // Step 2: flags. With CONDSTORE only messages changed since the
        // stored MODSEQ are fetched; otherwise flags for the local UIDs are
        // fetched using compressed UID ranges.
        let remote_flags: Option<(Vec<RemoteFlagState>, u64)> = match status.highest_modseq {
            Some(server_modseq) if reconcile::can_skip_reconcile(stored_modseq, server_modseq) => {
                debug!(
                    "CONDSTORE: HIGHESTMODSEQ unchanged ({}), skipping flag reconcile for {}",
                    server_modseq, folder.name
                );
                None
            }
            Some(server_modseq) if stored_modseq > 0 => {
                match self
                    .provider
                    .inner()
                    .fetch_flags_changed_since(&folder.remote_id, stored_modseq)
                    .await
                {
                    Ok((changed, _)) => {
                        debug!(
                            "CONDSTORE: {} flag changes since MODSEQ {} in {}",
                            changed.len(),
                            stored_modseq,
                            folder.name
                        );
                        Some((changed, server_modseq))
                    }
                    Err(e) => {
                        warn!(
                            "CHANGEDSINCE fetch failed for {}, falling back to a full flag fetch: {}",
                            folder.name, e
                        );
                        let uids = local_uids(&local_state);
                        let (flags, _) = self
                            .provider
                            .inner()
                            .fetch_flags_with_modseq(&folder.remote_id, &uids)
                            .await?;
                        Some((flags, server_modseq))
                    }
                }
            }
            server_modseq => {
                let uids = local_uids(&local_state);
                let (flags, _) = self
                    .provider
                    .inner()
                    .fetch_flags_with_modseq(&folder.remote_id, &uids)
                    .await?;
                Some((flags, server_modseq.unwrap_or(0)))
            }
        };

        let mut new_modseq = None;
        if let Some((remote_flags, server_modseq)) = remote_flags {
            // Step 3: Compute and apply flag diff, leaving messages with
            // queued local changes alone until they reach the server.
            let flag_changes = reconcile::compute_flag_diff(&local_state, &remote_flags);
            let pending = message_ids_with_pending_local_changes(
                &self.base.store,
                &self.base.account_id,
                flag_changes.iter().map(|(id, _, _)| id.clone()).collect(),
            )
            .await?;
            let flag_changes: Vec<_> = flag_changes
                .into_iter()
                .filter(|(id, _, _)| !pending.contains(id))
                .collect();
            if !flag_changes.is_empty() {
                info!(
                    "Applying {} flag changes for folder {}",
                    flag_changes.len(),
                    folder.name
                );
                blocking_store(&self.base.store, move |store| {
                    store.bulk_update_flags(&flag_changes)
                })
                .await?;
            }
            // Advance MODSEQ only when every server change was applied, so
            // skipped messages are looked at again on the next pass.
            if pending.is_empty() && server_modseq > 0 {
                new_modseq = Some(server_modseq);
            }
        }

        // Step 4: Detect deletions. Skipped when EXISTS/UIDNEXT prove that
        // nothing was expunged since the last verification; otherwise only
        // the local UIDs are checked, in compressed chunks.
        let mut new_baseline = None;
        if imap_deletion_check_needed(cursor.deletion_baseline, status.exists, status.uid_next) {
            let uids = local_uids(&local_state);
            let server_uids = self
                .provider
                .inner()
                .existing_uids_among(&folder.remote_id, &uids)
                .await?;
            let local_remote_ids: Vec<(String, String)> = local_state
                .iter()
                .map(|(id, rid, _, _, _)| (id.clone(), rid.clone()))
                .collect();
            let deleted = reconcile::detect_deletions(&local_remote_ids, &server_uids);
            let pending = message_ids_with_pending_local_changes(
                &self.base.store,
                &self.base.account_id,
                deleted.clone(),
            )
            .await?;
            let deleted: Vec<String> = deleted
                .into_iter()
                .filter(|id| !pending.contains(id))
                .collect();
            if !deleted.is_empty() {
                info!(
                    "Soft-deleting {} server-removed messages from {}",
                    deleted.len(),
                    folder.name
                );
                blocking_store(&self.base.store, move |store| {
                    store.bulk_soft_delete(&deleted)
                })
                .await?;
            }
            if pending.is_empty() {
                new_baseline = status.uid_next.map(|uid_next| ImapDeletionBaseline {
                    exists: status.exists,
                    uid_next,
                });
            }
        } else if let Some(uid_next) = status.uid_next {
            new_baseline = Some(ImapDeletionBaseline {
                exists: status.exists,
                uid_next,
            });
        }

        // Step 5: Persist reconcile progress without touching fields owned by
        // polling (last_uid, backfill, retry list).
        if new_modseq.is_some() || new_baseline.is_some() {
            self.update_folder_cursor_reconcile_state(folder, new_modseq, new_baseline)
                .await?;
        }

        Ok(())
    }

    /// Update the reconcile-owned parts of one folder cursor.
    pub(super) async fn update_folder_cursor_reconcile_state(
        &self,
        folder: &pebble_core::Folder,
        new_modseq: Option<u64>,
        new_baseline: Option<ImapDeletionBaseline>,
    ) -> Result<()> {
        let mut cursor = self.stored_imap_folder_cursor_async(folder).await?;
        if let Some(modseq) = new_modseq {
            set_imap_cursor_modseq(&mut cursor, modseq);
        }
        if new_baseline.is_some() {
            cursor.deletion_baseline = new_baseline;
        }
        let state = serialize_imap_folder_cursor(&cursor).ok_or_else(|| {
            PebbleError::Internal("Failed to serialize IMAP folder sync cursor".to_string())
        })?;
        let account_id = self.base.account_id.clone();
        let folder_id = folder.id.clone();
        blocking_store(&self.base.store, move |store| {
            store.set_folder_sync_state(&account_id, &folder_id, &state)
        })
        .await
    }
}
