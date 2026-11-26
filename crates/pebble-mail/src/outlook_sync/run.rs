use super::*;

impl OutlookSyncWorker {
    /// Sync one folder through Graph delta, persisting every page before the
    /// next is requested and checkpointing the continuation link after each
    /// failure-free page. Returns the number of failures.
    pub(super) async fn sync_outlook_folder(
        &self,
        folder: &Folder,
        sync_since: Option<i64>,
    ) -> u32 {
        let account_id = self.base.account_id.clone();
        let folder_id = folder.id.clone();
        let state = match blocking_store(&self.base.store, move |store| {
            store.get_folder_sync_state(&account_id, &folder_id)
        })
        .await
        {
            Ok(state) => state,
            Err(error) => {
                warn!(
                    "Failed to read Outlook sync cursor for folder {}: {error}",
                    folder.name
                );
                return 1;
            }
        };
        let scope_matches = outlook_delta_scope_matches(state.as_deref(), sync_since);
        let delta_cursor = if scope_matches {
            parse_outlook_delta_cursor(&folder.remote_id, state.as_deref())
        } else {
            None
        };
        let resume = if scope_matches {
            parse_outlook_resume_state(state.as_deref())
        } else {
            None
        };

        let account_id = self.base.account_id.clone();
        let folder_id = folder.id.clone();
        let cached_remote_ids = match blocking_store(&self.base.store, move |store| {
            load_cached_outlook_ids(sync_since, || {
                store
                    .list_remote_ids_by_folder(&account_id, &folder_id)
                    .map(|messages| {
                        messages
                            .into_iter()
                            .map(|(_, remote_id, _, _, _)| remote_id)
                            .collect::<std::collections::HashSet<_>>()
                    })
            })
        })
        .await
        {
            Ok(cached_remote_ids) => cached_remote_ids,
            Err(error) => {
                warn!(
                    "Failed to read cached Outlook message IDs for folder {}: {error}",
                    folder.name
                );
                return 1;
            }
        };

        let start = resume
            .as_ref()
            .map(|resume| resume.next_link.clone())
            .or_else(|| delta_cursor.clone());
        let mut recovering = resume.as_ref().is_some_and(|resume| resume.recovering);
        // Only a walk from a stored delta link reports genuinely new mail.
        let mut notify_new = resume.is_none() && delta_cursor.is_some();
        let mut previous_snapshot: Option<Vec<Message>> = None;
        let mut fresh_remote_ids: Option<std::collections::HashSet<String>> = None;
        let mut pager = OutlookDeltaPager::new(start.as_deref(), MAX_OUTLOOK_DELTA_PAGES_PER_RUN);
        let mut failures = 0u32;
        let mut checkpointing = true;

        loop {
            let request = match pager.next_request() {
                Ok(Some(request)) => request,
                Ok(None) => {
                    info!(
                        "Outlook folder {} has more delta pages; continuing on the next sync",
                        folder.name
                    );
                    return failures;
                }
                Err(e) => {
                    warn!("Outlook delta sync failed for folder {}: {e}", folder.name);
                    return failures + 1;
                }
            };
            let first_page_with_cursor = pager.page_count == 1 && request.is_some();
            let page = match self
                .provider
                .fetch_delta_page_since_with_cached_ids(
                    &folder.remote_id,
                    request.as_deref(),
                    sync_since,
                    Arc::clone(&cached_remote_ids),
                )
                .await
            {
                Ok(page) => page,
                Err(error)
                    if first_page_with_cursor
                        && (matches!(error, PebbleError::SyncCursorExpired(_))
                            || resume.is_some()) =>
                {
                    warn!(
                        "Outlook delta cursor for folder {} is no longer usable ({error}); restarting a full walk",
                        folder.name
                    );
                    let folder_id = folder.id.clone();
                    match blocking_store(&self.base.store, move |store| {
                        load_outlook_folder_snapshot(store, &folder_id)
                    })
                    .await
                    {
                        Ok(snapshot) => previous_snapshot = Some(snapshot),
                        Err(error) => {
                            warn!(
                                "Failed to snapshot Outlook folder {} before expired-cursor recovery: {error}",
                                folder.name
                            );
                            return failures + 1;
                        }
                    }
                    fresh_remote_ids = Some(std::collections::HashSet::new());
                    recovering = true;
                    notify_new = false;
                    pager = OutlookDeltaPager::new(None, MAX_OUTLOOK_DELTA_PAGES_PER_RUN);
                    continue;
                }
                Err(error) => {
                    warn!(
                        "Outlook delta sync failed for folder {}: {error}",
                        folder.name
                    );
                    return failures + 1;
                }
            };

            let OutlookDeltaPage {
                messages,
                deleted_remote_ids,
                next_link,
                delta_link,
            } = page;
            if let Some(fresh) = fresh_remote_ids.as_mut() {
                fresh.extend(messages.iter().map(|message| message.remote_id.clone()));
            }
            let mut page_failures = self
                .persist_folder_messages(folder, messages, notify_new)
                .await;
            let account_id = self.base.account_id.clone();
            match blocking_store(&self.base.store, move |store| {
                apply_outlook_deleted_remote_ids(store, &account_id, &deleted_remote_ids)
            })
            .await
            {
                Ok(outcome) => page_failures += outcome.failure_count,
                Err(e) => {
                    warn!(
                        "Failed to load Outlook tombstones for folder {}: {e}",
                        folder.name
                    );
                    page_failures += 1;
                }
            }
            failures += page_failures;
            if page_failures > 0 {
                // Later pages are still applied, but the checkpoint stays on
                // the last fully persisted page so failures are retried.
                checkpointing = false;
            }

            match pager.advance(next_link, delta_link) {
                Err(e) => {
                    warn!("Outlook delta sync failed for folder {}: {e}", folder.name);
                    return failures + 1;
                }
                Ok(OutlookPageStep::Next(next_link)) => {
                    if checkpointing {
                        let state = serialize_outlook_folder_state(
                            &folder.remote_id,
                            delta_cursor.as_deref(),
                            Some(&OutlookResumeState {
                                next_link,
                                recovering,
                            }),
                            sync_since,
                        );
                        if let Err(e) = self.save_outlook_folder_state(folder, state).await {
                            warn!(
                                "Failed to checkpoint Outlook delta for folder {}: {e}",
                                folder.name
                            );
                        }
                    }
                }
                Ok(OutlookPageStep::Done(delta_link)) => {
                    if failures == 0 {
                        match (previous_snapshot.as_deref(), fresh_remote_ids.as_ref()) {
                            (Some(previous_snapshot), Some(fresh_remote_ids)) => {
                                let store = Arc::clone(&self.base.store);
                                let folder_id = folder.id.clone();
                                let previous_snapshot = previous_snapshot.to_vec();
                                let fresh_remote_ids = fresh_remote_ids.clone();
                                failures += blocking_store(&store, move |store| {
                                    Ok(reconcile_outlook_fresh_snapshot_since(
                                        store,
                                        &folder_id,
                                        &previous_snapshot,
                                        &fresh_remote_ids,
                                        sync_since,
                                    )
                                    .failure_count)
                                })
                                .await
                                .unwrap_or(1);
                            }
                            _ if recovering => debug!(
                                "Outlook folder {} finished a resumed recovery walk; stale messages are reconciled on the next cursor expiry",
                                folder.name
                            ),
                            _ => {}
                        }
                    }
                    if let Some(delta_link) = delta_link {
                        if can_advance_outlook_delta_cursor(failures) {
                            let state = serialize_outlook_delta_cursor_with_scope(
                                &folder.remote_id,
                                &delta_link,
                                sync_since,
                            );
                            if let Err(e) = self.save_outlook_folder_state(folder, state).await {
                                warn!(
                                    "Failed to persist Outlook delta cursor for folder {}: {e}",
                                    folder.name
                                );
                            }
                        } else {
                            warn!(
                                "Outlook delta sync for folder {} had {} failures; keeping previous checkpoint",
                                folder.name, failures
                            );
                        }
                    }
                    return failures;
                }
            }
        }
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
                    self.base.account_id,
                    backoff.failure_count(),
                    delay
                );
                loop {
                    match wait_for_outlook_delay(delay, &mut runtime, &mut stop_rx, &mut trigger_rx)
                        .await
                    {
                        SyncWaitOutcome::Stop => break 'sync_loop,
                        SyncWaitOutcome::ReadyToPoll => break,
                        SyncWaitOutcome::ContextChanged => continue,
                    }
                }
            }

            self.base.emit_sync_started("poll");

            // Refresh token if needed
            if let Err(e) = self.ensure_valid_token().await {
                warn!("Outlook token validation failed: {}", e);
                self.base
                    .emit_error("auth", &format!("Token validation failed: {e}"));
                self.base.emit_sync_error("poll", &e.to_string());
                let _ = backoff.record_failure();
                if backoff.is_circuit_open() {
                    let delay = backoff.current_delay();
                    loop {
                        match wait_for_outlook_delay(
                            delay,
                            &mut runtime,
                            &mut stop_rx,
                            &mut trigger_rx,
                        )
                        .await
                        {
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
                    self.base
                        .emit_error("sync", &format!("Outlook folder list failed: {e}"));
                    self.base.emit_sync_error("poll", &e.to_string());
                    let delay = backoff.record_failure();
                    if backoff.is_circuit_open() {
                        warn!(
                            "Circuit open for Outlook account {} ({} failures), waiting {:?}",
                            self.base.account_id,
                            backoff.failure_count(),
                            delay
                        );
                    }
                    if backoff.is_circuit_open() {
                        loop {
                            match wait_for_outlook_delay(
                                delay,
                                &mut runtime,
                                &mut stop_rx,
                                &mut trigger_rx,
                            )
                            .await
                            {
                                SyncWaitOutcome::Stop => break 'sync_loop,
                                SyncWaitOutcome::ReadyToPoll => break,
                                SyncWaitOutcome::ContextChanged => continue,
                            }
                        }
                    } else if wait_for_outlook_policy_delay(
                        &policy,
                        &backoff,
                        &mut runtime,
                        &mut stop_rx,
                        &mut trigger_rx,
                    )
                    .await
                    {
                        break 'sync_loop;
                    }
                    continue;
                }
            };

            for folder in &folders {
                // Persist folder
                let _ = self.base.store.insert_folder(folder);
            }

            // Re-read folders from DB so we use persisted IDs (upsert may keep old IDs)
            let db_folders = self
                .base
                .store
                .list_folders(&self.base.account_id)
                .unwrap_or_default()
                .into_iter()
                .filter(should_sync_outlook_folder)
                .collect::<Vec<_>>();
            let mut sync_failure_count = 0u32;
            let sync_since = match self.base.store.get_sync_state(&self.base.account_id) {
                Ok(state) => state.and_then(|state| state.sync_since),
                Err(error) => {
                    warn!("Failed to read Outlook sync scope: {error}");
                    self.base.emit_error(
                        "sync",
                        &format!("Failed to read Outlook sync scope: {error}"),
                    );
                    self.base.emit_sync_error("poll", &error.to_string());
                    let _ = backoff.record_failure();
                    if wait_for_outlook_policy_delay(
                        &policy,
                        &backoff,
                        &mut runtime,
                        &mut stop_rx,
                        &mut trigger_rx,
                    )
                    .await
                    {
                        break 'sync_loop;
                    }
                    continue 'sync_loop;
                }
            };

            for folder in &db_folders {
                sync_failure_count += self.sync_outlook_folder(folder, sync_since).await;
                if *stop_rx.borrow() {
                    break;
                }
            }

            if config.manual_only() {
                if sync_failure_count == 0 {
                    self.base.emit_sync_completed("poll");
                } else {
                    self.base.emit_sync_error(
                        "poll",
                        &format!("Outlook sync had {sync_failure_count} failure(s)"),
                    );
                }
                break;
            }

            update_outlook_backoff_after_sync(&mut backoff, sync_failure_count);
            if sync_failure_count == 0 {
                self.base.emit_sync_completed("poll");
            } else {
                self.base.emit_sync_error(
                    "poll",
                    &format!("Outlook sync had {sync_failure_count} failure(s)"),
                );
            }

            if wait_for_outlook_policy_delay(
                &policy,
                &backoff,
                &mut runtime,
                &mut stop_rx,
                &mut trigger_rx,
            )
            .await
            {
                break;
            }
        }

        info!(
            "Outlook sync task completed for account {}",
            self.base.account_id
        );
    }
}
