use super::*;

impl GmailSyncWorker {
    /// Perform initial sync: list folders, fetch messages for each label.
    pub(super) async fn initial_sync(&self) -> Result<()> {
        info!(
            "Starting Gmail initial sync for account {}",
            self.base.account_id
        );

        self.refresh_folders().await?;

        // Get the stored sync cursor (historyId) if any
        let sync_state = self
            .base
            .store
            .get_sync_state(&self.base.account_id)?
            .unwrap_or_default();
        let sync_since = sync_state.sync_since;
        let stored_cursor = saved_gmail_history_cursor(&sync_state);
        let notify_new = should_notify_gmail_startup_fetch(stored_cursor.as_deref());

        // Capture the history checkpoint before replay or bootstrap so changes
        // arriving during either operation are replayed on the next poll.
        let (_email, profile_history_id) = self.provider.get_profile().await?;
        if let Some(cursor) = stored_cursor.as_deref() {
            match self.replay_gmail_history(cursor).await? {
                GmailHistoryReplayOutcome::Applied(_) => {}
                GmailHistoryReplayOutcome::CursorExpired => {
                    self.reconcile_cached_gmail_metadata().await?;
                }
            }
        }

        // Sync every visible remote label, prioritizing system folders first.
        let all_folders = self.base.store.list_folders(&self.base.account_id)?;
        let labels_to_sync = build_sync_label_ids(&all_folders);
        let folders_by_remote: HashMap<String, String> = all_folders
            .into_iter()
            .map(|f| (f.remote_id, f.id))
            .collect();
        let limit = if stored_cursor.is_some() {
            50
        } else {
            GMAIL_BOOTSTRAP_PAGE_SIZE
        };
        let mut failure_count = 0usize;

        // Resume state of an interrupted bootstrap, per label folder.
        let label_folder_ids: Vec<String> = labels_to_sync
            .iter()
            .filter_map(|label| folders_by_remote.get(label).cloned())
            .collect();
        let account_id = self.base.account_id.clone();
        let label_progress: HashMap<String, GmailLabelProgress> =
            blocking_store(&self.base.store, move |store| {
                let mut progress = HashMap::new();
                for folder_id in label_folder_ids {
                    let state = store.get_folder_sync_state(&account_id, &folder_id)?;
                    if let Some(saved) = parse_gmail_label_progress(state.as_deref(), sync_since) {
                        progress.insert(folder_id, saved);
                    }
                }
                Ok(progress)
            })
            .await?;
        let profile_history_id =
            gmail_bootstrap_checkpoint(label_progress.values(), &profile_history_id);

        for label_id in &labels_to_sync {
            let progress = folders_by_remote
                .get(label_id)
                .and_then(|folder_id| label_progress.get(folder_id))
                .cloned();
            match self
                .sync_label(
                    label_id,
                    limit,
                    &folders_by_remote,
                    notify_new,
                    sync_since,
                    progress,
                    &profile_history_id,
                )
                .await
            {
                Ok(outcome) => {
                    failure_count += outcome.failure_count;
                }
                Err(e) => {
                    failure_count += 1;
                    warn!("Gmail sync label {} failed: {}", label_id, e);
                }
            }
        }

        // Store the historyId as sync cursor for future delta syncs
        if !profile_history_id.is_empty() && can_advance_gmail_cursor(failure_count) {
            self.base
                .store
                .update_sync_state(&self.base.account_id, |state| {
                    state.last_sync_cursor = Some(profile_history_id.clone());
                    state.gmail_sync_since = sync_since;
                    state.has_gmail_sync_scope = true;
                })?;
            // The bootstrap is complete; drop per-label resume state.
            {
                let account_id = self.base.account_id.clone();
                let folder_ids: Vec<String> = labels_to_sync
                    .iter()
                    .filter_map(|label| folders_by_remote.get(label).cloned())
                    .collect();
                blocking_store(&self.base.store, move |store| {
                    for folder_id in folder_ids {
                        store.set_folder_sync_state(
                            &account_id,
                            &folder_id,
                            &serialize_gmail_label_progress(None),
                        )?;
                    }
                    Ok(())
                })
                .await?;
            }
        } else if failure_count > 0 {
            warn!(
                "Gmail initial sync had {} failures; keeping previous history cursor",
                failure_count
            );
            return Err(PebbleError::Network(format!(
                "Gmail initial sync had {failure_count} failed label syncs"
            )));
        }

        info!(
            "Gmail initial sync completed for account {}",
            self.base.account_id
        );
        Ok(())
    }

    pub(super) async fn reconcile_cached_gmail_metadata(&self) -> Result<()> {
        let folders = self.base.store.list_folders(&self.base.account_id)?;
        let visible_folders = folders
            .iter()
            .filter(|folder| {
                !folder.remote_id.starts_with("__local_")
                    && !visible_label_ids(std::slice::from_ref(&folder.remote_id)).is_empty()
            })
            .map(|folder| (folder.remote_id.clone(), folder.id.clone()))
            .collect::<HashMap<_, _>>();
        let folders_by_id = folders
            .iter()
            .map(|folder| (folder.id.clone(), folder.clone()))
            .collect::<HashMap<_, _>>();

        let remote_ids = self
            .base
            .store
            .list_cached_remote_ids_by_account(&self.base.account_id)?;
        let provider = Arc::clone(&self.provider);
        let fetched = fetch_cached_gmail_metadata(remote_ids, move |remote_id| {
            let provider = Arc::clone(&provider);
            async move { provider.fetch_message_metadata(&remote_id).await }
        })
        .await?;

        let account_id = self.base.account_id.clone();
        let refreshed = blocking_store(&self.base.store, move |store| {
            let existing = store.get_existing_message_map_by_remote_ids(
                &account_id,
                &fetched.iter().map(|(id, _)| id.clone()).collect::<Vec<_>>(),
            )?;
            let pending = store.message_ids_with_active_pending_ops(
                &account_id,
                &existing.values().cloned().collect::<Vec<_>>(),
            )?;
            let mut refreshed = Vec::new();
            for (remote_id, metadata) in fetched {
                let Some(local_id) = existing.get(&remote_id).cloned() else {
                    continue;
                };
                let Some(metadata) = metadata else {
                    store.soft_delete_message(&local_id)?;
                    refreshed.push(local_id);
                    continue;
                };
                if metadata.id != remote_id {
                    return Err(PebbleError::Network(format!(
                        "Mismatched Gmail metadata for {remote_id}"
                    )));
                }
                // Optimistic local changes win until their queued op lands.
                if pending.contains(&local_id) {
                    continue;
                }

                store.update_message_flags(
                    &local_id,
                    Some(!metadata.label_ids.iter().any(|label| label == "UNREAD")),
                    Some(metadata.label_ids.iter().any(|label| label == "STARRED")),
                )?;
                let desired_labels: HashSet<_> =
                    visible_label_ids(&metadata.label_ids).into_iter().collect();
                let existing_folder_ids = store.get_message_folder_ids(&local_id)?;
                for folder_id in existing_folder_ids {
                    if folders_by_id.get(&folder_id).is_some_and(|folder| {
                        !folder.remote_id.starts_with("__local_")
                            && !visible_label_ids(std::slice::from_ref(&folder.remote_id))
                                .is_empty()
                            && !desired_labels.contains(&folder.remote_id)
                    }) {
                        store.remove_message_from_folder(&local_id, &folder_id)?;
                    }
                }
                for label_id in &desired_labels {
                    if let Some(folder_id) = visible_folders.get(label_id) {
                        store.add_message_to_folder(&local_id, folder_id)?;
                    }
                }
                refreshed.push(local_id);
            }
            Ok(refreshed)
        })
        .await?;
        for local_id in refreshed {
            self.emit_message_refresh(&local_id);
        }
        Ok(())
    }

    pub(super) async fn save_label_progress(&self, folder_id: &str, progress: &GmailLabelProgress) {
        let account_id = self.base.account_id.clone();
        let folder_id = folder_id.to_string();
        let state = serialize_gmail_label_progress(Some(progress));
        if let Err(e) = blocking_store(&self.base.store, move |store| {
            store.set_folder_sync_state(&account_id, &folder_id, &state)
        })
        .await
        {
            warn!("Failed to checkpoint Gmail label progress: {e}");
        }
    }

    /// Sync messages for a specific Gmail label, one list page at a time.
    ///
    /// Each page is stored before the next is listed, so memory stays
    /// bounded by one page, and the next page token is checkpointed after
    /// every page stored without failures so an interrupted bootstrap
    /// resumes there.
    #[allow(clippy::too_many_arguments)]
    pub(super) async fn sync_label(
        &self,
        label_id: &str,
        limit: u32,
        folders_by_remote: &HashMap<String, String>,
        notify_new: bool,
        sync_since: Option<i64>,
        progress: Option<GmailLabelProgress>,
        checkpoint_history_id: &str,
    ) -> Result<GmailLabelSyncOutcome> {
        let folder_id = match folders_by_remote.get(label_id) {
            Some(id) => id.clone(),
            None => {
                debug!("No local folder found for label {}, skipping", label_id);
                return Ok(GmailLabelSyncOutcome::default());
            }
        };
        if progress.as_ref().is_some_and(|progress| progress.complete) {
            debug!(
                "Gmail label {} finished in an earlier interrupted bootstrap; skipping",
                label_id
            );
            return Ok(GmailLabelSyncOutcome::default());
        }

        let mut outcome = GmailLabelSyncOutcome::default();
        let mut page_token = progress.and_then(|progress| progress.page_token);
        let mut resumed = page_token.is_some();
        let mut checkpointing = true;
        loop {
            let (refs, next_page) = match self
                .provider
                .list_message_ids(label_id, limit, page_token.as_deref(), sync_since)
                .await
            {
                Ok(page) => page,
                Err(e) if resumed => {
                    warn!(
                        "Saved Gmail page token for label {} was rejected ({}); restarting the label",
                        label_id, e
                    );
                    resumed = false;
                    page_token = None;
                    continue;
                }
                Err(e) => return Err(e),
            };
            resumed = false;

            let page_failures = self
                .sync_label_page(
                    label_id,
                    &folder_id,
                    refs,
                    folders_by_remote,
                    notify_new,
                    &mut outcome,
                )
                .await?;
            if page_failures > 0 {
                // Keep the last good checkpoint so failed messages are
                // listed again on the next attempt.
                checkpointing = false;
            }

            match next_page {
                Some(token) if !token.is_empty() => {
                    if checkpointing {
                        self.save_label_progress(
                            &folder_id,
                            &GmailLabelProgress {
                                history_id: checkpoint_history_id.to_string(),
                                sync_since,
                                page_token: Some(token.clone()),
                                complete: false,
                            },
                        )
                        .await;
                    }
                    page_token = Some(token);
                }
                _ => break,
            }
        }

        if outcome.failure_count == 0 && !checkpoint_history_id.is_empty() {
            self.save_label_progress(
                &folder_id,
                &GmailLabelProgress {
                    history_id: checkpoint_history_id.to_string(),
                    sync_since,
                    page_token: None,
                    complete: true,
                },
            )
            .await;
        }

        if outcome.stored_count > 0 {
            info!(
                "Stored {} messages for label {}",
                outcome.stored_count, label_id
            );
        }
        Ok(outcome)
    }

    /// Store one listed page of a label. Returns the number of failures on
    /// this page (also added to `outcome`).
    pub(super) async fn sync_label_page(
        &self,
        label_id: &str,
        folder_id: &str,
        msg_refs: Vec<GmailMessageRef>,
        folders_by_remote: &HashMap<String, String>,
        notify_new: bool,
        outcome: &mut GmailLabelSyncOutcome,
    ) -> Result<usize> {
        if msg_refs.is_empty() {
            return Ok(0);
        }
        let mut page_failures = 0usize;

        let remote_ids: Vec<String> = msg_refs.iter().map(|r| r.id.clone()).collect();
        let account_id = self.base.account_id.clone();
        let (existing, incomplete) = blocking_store(&self.base.store, move |store| {
            Ok((
                store.get_existing_message_map_by_remote_ids(&account_id, &remote_ids)?,
                store.get_incomplete_attachment_message_map_by_remote_ids(
                    &account_id,
                    None,
                    &remote_ids,
                )?,
            ))
        })
        .await?;

        // Separate already-existing messages (label membership / thread
        // refresh only) from new ones to fetch.
        let mut to_fetch = Vec::new();
        let mut existing_refs: Vec<(String, Option<String>)> = Vec::new();
        for msg_ref in msg_refs {
            match existing.get(&msg_ref.id) {
                Some(local_id)
                    if !remote_message_requires_fetch(
                        true,
                        incomplete.contains_key(&msg_ref.id),
                    ) =>
                {
                    existing_refs.push((
                        local_id.clone(),
                        provider_thread_id(GMAIL_THREAD_NAMESPACE, msg_ref.thread_id.as_deref()),
                    ));
                }
                _ => to_fetch.push(msg_ref.id.clone()),
            }
        }

        if !existing_refs.is_empty() {
            let local_ids: Vec<String> = existing_refs.iter().map(|(id, _)| id.clone()).collect();
            let pending = message_ids_with_pending_local_changes(
                &self.base.store,
                &self.base.account_id,
                local_ids.clone(),
            )
            .await?;
            let folder_memberships = {
                let local_ids = local_ids.clone();
                blocking_store(&self.base.store, move |store| {
                    store.get_message_folder_ids_batch(&local_ids)
                })
                .await?
            };
            let to_label: Vec<String> = local_ids
                .iter()
                .filter(|id| !pending.contains(*id))
                .filter(|id| {
                    !folder_memberships
                        .get(*id)
                        .is_some_and(|folders| folders.iter().any(|f| f == folder_id))
                })
                .cloned()
                .collect();
            let mut refreshed: HashSet<String> = HashSet::new();
            if !to_label.is_empty() {
                let folder = folder_id.to_string();
                let labelled = blocking_store(&self.base.store, move |store| {
                    let mut results = Vec::with_capacity(to_label.len());
                    for local_id in to_label {
                        let result = store.add_message_to_folder(&local_id, &folder);
                        results.push((local_id, result.err().map(|e| e.to_string())));
                    }
                    Ok(results)
                })
                .await?;
                for (local_id, error) in labelled {
                    match error {
                        Some(e) => {
                            outcome.failure_count += 1;
                            page_failures += 1;
                            warn!(
                                "Failed to add Gmail label {} to existing message {}: {}",
                                label_id, local_id, e
                            );
                        }
                        None => {
                            refreshed.insert(local_id);
                        }
                    }
                }
            }

            // Move stored messages onto Gmail's own thread ids as we see them.
            let assignments: Vec<(String, String)> = existing_refs
                .into_iter()
                .filter_map(|(local_id, thread_id)| thread_id.map(|tid| (local_id, tid)))
                .collect();
            match assign_thread_ids(&self.base.store, &self.base.account_id, assignments).await {
                Ok(changed) => refreshed.extend(changed),
                Err(e) => warn!("Failed to update Gmail thread ids: {e}"),
            }
            for local_id in refreshed {
                self.emit_message_refresh(&local_id);
            }
        }

        // Fetch new messages concurrently (up to 10 in flight) and store each
        // as soon as it arrives, so at most one page is held in memory.
        use futures::stream::{self, StreamExt};
        let mut fetched_results = stream::iter(to_fetch.into_iter().map(|gmail_id| {
            let provider = Arc::clone(&self.provider);
            let account_id = self.base.account_id.clone();
            async move {
                let result = provider.fetch_sync_message(&gmail_id, &account_id).await;
                (gmail_id, result)
            }
        }))
        .buffer_unordered(10);

        while let Some((gmail_id, result)) = fetched_results.next().await {
            match result {
                Ok(fetched) => match self
                    .store_fetched_message(fetched, folder_id, folders_by_remote, notify_new)
                    .await
                {
                    Ok(true) => outcome.stored_count += 1,
                    Ok(false) => {}
                    Err(e) => {
                        outcome.failure_count += 1;
                        page_failures += 1;
                        error!("Failed to store Gmail message {}: {}", gmail_id, e);
                    }
                },
                Err(e) => {
                    outcome.failure_count += 1;
                    page_failures += 1;
                    warn!("Failed to fetch Gmail message {}: {}", gmail_id, e);
                }
            }
        }

        Ok(page_failures)
    }
}
