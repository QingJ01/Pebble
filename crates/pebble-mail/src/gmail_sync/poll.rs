use super::*;

impl GmailSyncWorker {
    pub(super) async fn replay_gmail_history(
        &self,
        history_id: &str,
    ) -> Result<GmailHistoryReplayOutcome> {
        let history_cursor = history_id.to_string();
        let provider = Arc::clone(&self.provider);
        let history = collect_paginated_gmail_history(history_id, move |page_token| {
            let provider = Arc::clone(&provider);
            let history_cursor = history_cursor.clone();
            async move {
                let mut url =
                    reqwest::Url::parse("https://www.googleapis.com/gmail/v1/users/me/history")
                        .map_err(|e| {
                            PebbleError::Network(format!("Build Gmail history URL: {e}"))
                        })?;
                {
                    let mut query = url.query_pairs_mut();
                    query.append_pair("startHistoryId", &history_cursor);
                    if let Some(page_token) = page_token {
                        query.append_pair("pageToken", &page_token);
                    }
                }

                let resp = provider.get(url.as_str()).await?;
                let status = resp.status();
                if gmail_history_cursor_expired(status) {
                    return Ok(GmailHistoryPageFetch::CursorExpired);
                }
                if !status.is_success() {
                    let body = resp.text().await.unwrap_or_default();
                    return Err(PebbleError::Network(format!(
                        "Gmail history request failed with status {status}: {body}"
                    )));
                }

                let page = resp.json::<GmailHistoryPage>().await.map_err(|e| {
                    PebbleError::Network(format!("Parse Gmail history response: {e}"))
                })?;
                Ok(GmailHistoryPageFetch::Page(page))
            }
        })
        .await?;

        if history.cursor_expired {
            return Ok(GmailHistoryReplayOutcome::CursorExpired);
        }

        let mut new_ids = Vec::new();
        let mut deleted_ids = Vec::new();
        let mut failure_count = 0usize;

        for entry in &history.entries {
            if let Some(ref added) = entry.messages_added {
                for m in added {
                    new_ids.push(m.message.id.clone());
                }
            }
            if let Some(ref deleted) = entry.messages_deleted {
                for m in deleted {
                    deleted_ids.push(m.message.id.clone());
                }
            }
        }

        let account_id = self.base.account_id.clone();
        let entries = history.entries;
        let (folders_by_remote, refreshed) = blocking_store(&self.base.store, move |store| {
            let folders_by_remote: HashMap<String, String> = store
                .list_folders(&account_id)?
                .into_iter()
                .map(|folder| (folder.remote_id, folder.id))
                .collect();
            let refreshed =
                apply_gmail_history_mutations(store, &account_id, &folders_by_remote, &entries)?;
            Ok((folders_by_remote, refreshed))
        })
        .await?;
        for local_id in refreshed {
            self.emit_message_refresh(&local_id);
        }
        if !deleted_ids.is_empty() {
            info!("Applied {} Gmail history deletions", deleted_ids.len());
        }

        let new_ids = filter_deleted_history_additions(new_ids, &deleted_ids);

        // Fetch new messages concurrently, then collect refs and store.
        if !new_ids.is_empty() {
            let existing = match self
                .base
                .store
                .get_existing_message_map_by_remote_ids(&self.base.account_id, &new_ids)
            {
                Ok(existing) => existing,
                Err(e) => {
                    failure_count += 1;
                    warn!("Failed to look up existing Gmail history messages: {}", e);
                    HashMap::new()
                }
            };
            let incomplete = match self
                .base
                .store
                .get_incomplete_attachment_message_map_by_remote_ids(
                    &self.base.account_id,
                    None,
                    &new_ids,
                ) {
                Ok(incomplete) => incomplete,
                Err(e) => {
                    failure_count += 1;
                    warn!("Failed to inspect incomplete Gmail history messages: {e}");
                    HashMap::new()
                }
            };

            // Emit refresh for already-known messages; collect truly new IDs.
            let to_fetch: Vec<String> = new_ids
                .into_iter()
                .filter(|gid| {
                    if remote_message_requires_fetch(
                        existing.contains_key(gid),
                        incomplete.contains_key(gid),
                    ) {
                        true
                    } else if let Some(local_id) = existing.get(gid) {
                        self.emit_message_refresh(local_id);
                        false
                    } else {
                        true
                    }
                })
                .collect();

            if !to_fetch.is_empty() {
                let inbox_folder_id = folders_by_remote.get("INBOX").cloned().unwrap_or_default();

                // Fetch concurrently and store each message as it arrives.
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
                        Ok(fetched) => {
                            if let Err(e) = self
                                .store_fetched_message(
                                    fetched,
                                    &inbox_folder_id,
                                    &folders_by_remote,
                                    true,
                                )
                                .await
                            {
                                failure_count += 1;
                                warn!("Failed to store history message {}: {}", gmail_id, e);
                            }
                        }
                        Err(e) => {
                            if should_count_gmail_history_fetch_failure(&e) {
                                failure_count += 1;
                                warn!("Failed to fetch history message {}: {}", gmail_id, e);
                            } else {
                                debug!(
                                    "Skipping stale Gmail history message {} after fetch returned not found",
                                    gmail_id
                                );
                            }
                        }
                    }
                }
            }
        }

        if failure_count > 0 {
            return Err(PebbleError::Network(format!(
                "Gmail history replay had {failure_count} failures"
            )));
        }

        Ok(GmailHistoryReplayOutcome::Applied(history.history_id))
    }

    /// Poll for new messages using the Gmail History API (delta sync).
    pub(super) async fn poll_changes(&self) -> Result<()> {
        let sync_state = self
            .base
            .store
            .get_sync_state(&self.base.account_id)?
            .unwrap_or_default();
        let history_id = match gmail_sync_dispatch(&sync_state) {
            GmailSyncDispatch::History(id) => id,
            GmailSyncDispatch::Bootstrap => {
                debug!("No history cursor or the scope changed; performing bootstrap sync");
                return self.initial_sync().await;
            }
        };

        match replay_gmail_history_and_checkpoint(
            || self.replay_gmail_history(&history_id),
            |cursor| {
                self.base
                    .store
                    .set_sync_cursor(&self.base.account_id, cursor)
            },
        )
        .await?
        {
            GmailHistoryReplayOutcome::CursorExpired => {
                warn!(
                    "Gmail history cursor expired for account {}; running a full resync",
                    self.base.account_id
                );
                self.initial_sync().await
            }
            GmailHistoryReplayOutcome::Applied(Some(_)) => Ok(()),
            GmailHistoryReplayOutcome::Applied(None) => Ok(()),
        }
    }

    pub(super) async fn run_poll_cycle(
        &self,
        backoff: &mut SyncBackoff,
        allow_circuit_attempt: bool,
    ) {
        if backoff.is_circuit_open() {
            warn!(
                "Circuit open for Gmail account {} ({} failures), current delay {:?}",
                self.base.account_id,
                backoff.failure_count(),
                backoff.current_delay()
            );
            if !allow_circuit_attempt {
                return;
            }
        }

        self.base.emit_sync_started("poll");
        if let Err(e) = self.ensure_valid_token().await {
            warn!("Token refresh failed: {}", e);
            self.base.emit_sync_error("poll", &e.to_string());
            let _ = backoff.record_failure();
            return;
        }
        match self.poll_changes().await {
            Ok(()) => {
                self.base.emit_sync_completed("poll");
                backoff.record_success()
            }
            Err(e) => {
                warn!(
                    "Gmail poll failed for account {}: {}",
                    self.base.account_id, e
                );
                self.base.emit_error("sync", &format!("Poll failed: {e}"));
                self.base.emit_sync_error("poll", &e.to_string());
                let _ = backoff.record_failure();
            }
        }
    }

    /// Main sync loop.
    pub async fn run(
        &self,
        config: SyncConfig,
        trigger_rx: Option<mpsc::UnboundedReceiver<SyncTrigger>>,
    ) {
        // Ensure token is valid before starting
        if let Err(e) = self.ensure_valid_token().await {
            error!(
                "Token validation failed for account {}: {}",
                self.base.account_id, e
            );
            self.base
                .emit_error("auth", &format!("Token validation failed: {e}"));
            self.base.emit_sync_error("auth", &e.to_string());
            return;
        }

        let startup_dispatch = self
            .base
            .store
            .get_sync_state(&self.base.account_id)
            .map(|state| gmail_sync_dispatch(&state.unwrap_or_default()));

        // Initial startup pass. Existing accounts must use Gmail History delta;
        // otherwise startup can skip older changes outside the latest-label fetch
        // window and then incorrectly advance the cursor.
        self.base.emit_sync_started("initial");
        let startup_result = match startup_dispatch {
            Ok(GmailSyncDispatch::History(_)) => match self.refresh_folders().await {
                Ok(()) => self.poll_changes().await,
                Err(e) => Err(e),
            },
            Ok(GmailSyncDispatch::Bootstrap) => self.initial_sync().await,
            Err(error) => Err(error),
        };

        if let Err(e) = startup_result {
            error!(
                "Gmail startup sync failed for account {}: {}",
                self.base.account_id, e
            );
            self.base
                .emit_error("sync", &format!("Startup sync failed: {e}"));
            self.base.emit_sync_error("initial", &e.to_string());
            // Don't return — still enter poll loop so we can retry
        } else {
            self.base.emit_sync_completed("initial");
        }

        if config.manual_only() {
            info!(
                "Gmail manual sync completed for account {}",
                self.base.account_id
            );
            return;
        }

        let policy = RealtimePollPolicy::from_foreground_interval_secs(config.poll_interval_secs);
        let mut stop_rx = self.stop_rx.clone();
        let mut backoff = SyncBackoff::new();
        let mut trigger_rx = trigger_rx;
        let mut runtime = RealtimeRuntimeState::new(Duration::from_secs(60), Instant::now());

        loop {
            let next_delay =
                policy.next_delay(runtime.context(backoff.failure_count(), Instant::now()));

            tokio::select! {
                _ = stop_rx.changed() => {
                    if *stop_rx.borrow() {
                        info!("Gmail sync stopped for account {}", self.base.account_id);
                        break;
                    }
                }
                _ = tokio::time::sleep(next_delay) => {
                    self.run_poll_cycle(&mut backoff, true).await;
                }
                trigger = recv_sync_trigger(&mut trigger_rx) => {
                    match trigger {
                        Some(trigger) => {
                            runtime.record_trigger(trigger, Instant::now());
                            if trigger.should_sync_now() {
                                self.run_poll_cycle(
                                    &mut backoff,
                                    trigger.bypasses_circuit_backoff(),
                                ).await;
                            }
                        }
                        None => {
                            trigger_rx = None;
                        }
                    }
                }
            }
        }
    }
}
