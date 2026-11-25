use super::*;

impl SyncWorker {
    /// Download and store the messages among `uids` that are not stored yet.
    ///
    /// Existence is checked locally first so already-synced messages cost no
    /// network traffic. Remaining messages are sized with `RFC822.SIZE`,
    /// fetched in bounded batches, and anything above
    /// [`MAX_IMAP_MESSAGE_BYTES`] is stored as a header-only placeholder.
    pub(super) async fn sync_uid_page(
        &self,
        folder: &pebble_core::Folder,
        uids: &[u32],
        notify_new: bool,
    ) -> Result<ImapStoreOutcome> {
        if uids.is_empty() {
            return Ok(ImapStoreOutcome::default());
        }
        let account_id = self.base.account_id.clone();
        let folder_id = folder.id.clone();
        let remote_ids: Vec<String> = uids.iter().map(u32::to_string).collect();
        let (existing, incomplete, has_failures) = blocking_store(&self.base.store, move |store| {
            Ok((
                store.get_existing_remote_ids_in_folder(&account_id, &folder_id, &remote_ids)?,
                store.get_incomplete_attachment_message_map_by_remote_ids(
                    &account_id,
                    Some(&folder_id),
                    &remote_ids,
                )?,
                store.has_sync_failures_for_folder(&account_id, &folder_id)?,
            ))
        })
        .await?;

        let mut missing = Vec::new();
        let mut present = Vec::new();
        for uid in uids {
            let remote_id = uid.to_string();
            if remote_message_requires_fetch(
                existing.contains(&remote_id),
                incomplete.contains_key(&remote_id),
            ) {
                missing.push(*uid);
            } else {
                present.push(remote_id);
            }
        }
        if has_failures && !present.is_empty() {
            let account_id = self.base.account_id.clone();
            let folder_id = folder.id.clone();
            blocking_store(&self.base.store, move |store| {
                for remote_id in present {
                    store.clear_sync_failure(&account_id, &folder_id, &remote_id)?;
                }
                Ok(())
            })
            .await?;
        }
        if missing.is_empty() {
            return Ok(ImapStoreOutcome::default());
        }

        let sizes = self
            .provider
            .inner()
            .fetch_uid_sizes(&folder.remote_id, &missing)
            .await?;
        let plan = plan_imap_body_batches(
            &sizes,
            MAX_IMAP_MESSAGE_BYTES,
            IMAP_BODY_BATCH_MAX_MESSAGES,
            IMAP_BODY_BATCH_MAX_BYTES,
        );

        let mut outcome = ImapStoreOutcome::default();
        for batch in &plan.batches {
            let raw_messages = self
                .provider
                .inner()
                .fetch_bodies_by_uids(&folder.remote_id, batch)
                .await?;
            let stored = self
                .store_fetched_messages(folder, raw_messages, notify_new)
                .await?;
            outcome.stored += stored.stored;
            outcome.failed_uids.extend(stored.failed_uids);
        }

        if !plan.oversized.is_empty() {
            let oversized_uids: Vec<u32> = plan.oversized.iter().map(|(uid, _)| *uid).collect();
            let sizes: std::collections::HashMap<u32, u32> =
                plan.oversized.iter().copied().collect();
            let headers = self
                .provider
                .inner()
                .fetch_headers_by_uids(&folder.remote_id, &oversized_uids)
                .await?;
            let mut with_headers = HashSet::new();
            let placeholders: Vec<(u32, Vec<u8>)> = headers
                .into_iter()
                .map(|(uid, header)| {
                    with_headers.insert(uid);
                    let size = sizes.get(&uid).copied().unwrap_or(0);
                    warn!(
                        "IMAP message UID {} in {} is {} bytes (limit {}); keeping headers only",
                        uid, folder.name, size, MAX_IMAP_MESSAGE_BYTES
                    );
                    (uid, oversized_placeholder_message(&header, size))
                })
                .collect();
            let stored = self
                .store_fetched_messages(folder, placeholders, notify_new)
                .await?;
            outcome.stored += stored.stored;
            outcome.failed_uids.extend(stored.failed_uids);

            let headerless: Vec<(u32, u32)> = plan
                .oversized
                .iter()
                .copied()
                .filter(|(uid, _)| !with_headers.contains(uid))
                .collect();
            if !headerless.is_empty() {
                let account_id = self.base.account_id.clone();
                let folder_id = folder.id.clone();
                blocking_store(&self.base.store, move |store| {
                    for (uid, size) in headerless {
                        store.upsert_sync_failure(
                            &account_id,
                            &folder_id,
                            &uid.to_string(),
                            "imap",
                            &format!(
                                "Message is {size} bytes, above the {MAX_IMAP_MESSAGE_BYTES}-byte download limit"
                            ),
                        )?;
                    }
                    Ok(())
                })
                .await?;
            }
        }
        Ok(outcome)
    }

    /// Backfill the full history of a folder that has no sync cursor yet,
    /// e.g. a folder just re-added to the account's IMAP selection after
    /// being deselected (which clears its local data).
    pub async fn sync_folder_history(
        &self,
        folder: &pebble_core::Folder,
        notify_new: bool,
    ) -> Result<u32> {
        let mut cursor = self.stored_imap_folder_cursor_async(folder).await?;
        Ok(self
            .sync_folder_history_with_watermark(folder, &mut cursor, notify_new)
            .await?
            .count)
    }

    /// Walk the folder's UIDs from newest to oldest in pages, checkpointing
    /// the lowest finished UID after every page so an interrupted backfill
    /// resumes where it stopped. UIDs are listed first and only messages
    /// missing locally are downloaded.
    ///
    /// The returned `uid_next` is the UIDNEXT captured when this backfill
    /// first started; callers mark the backfill complete with it.
    pub(super) async fn sync_folder_history_with_watermark(
        &self,
        folder: &pebble_core::Folder,
        cursor: &mut ImapFolderCursor,
        notify_new: bool,
    ) -> Result<ImapHistorySyncOutcome> {
        if Self::is_local_folder(folder) {
            return Ok(ImapHistorySyncOutcome::default());
        }

        let account_id = self.base.account_id.clone();
        let sync_since = blocking_store(&self.base.store, move |store| {
            Ok(store
                .get_sync_state(&account_id)?
                .and_then(|state| state.sync_since))
        })
        .await?;
        let floor = cursor.backfill_floor_uid;

        let (mut uids, uid_next) = if let Some(cutoff) = sync_since {
            self.provider
                .inner()
                .search_uids_since(&folder.remote_id, cutoff, None)
                .await?
        } else {
            match floor {
                Some(floor) if floor <= 1 => {
                    let status = self
                        .provider
                        .inner()
                        .get_mailbox_status(&folder.remote_id)
                        .await?;
                    (Vec::new(), status.uid_next)
                }
                Some(floor) => {
                    let (uids, status) = self
                        .provider
                        .inner()
                        .search_uids(&folder.remote_id, &format!("UID 1:{}", floor - 1))
                        .await?;
                    (uids, status.uid_next)
                }
                None => {
                    let (uids, status) = self
                        .provider
                        .inner()
                        .search_uids(&folder.remote_id, "ALL")
                        .await?;
                    (uids, status.uid_next)
                }
            }
        };
        if let Some(floor) = floor {
            uids.retain(|uid| *uid < floor);
        }
        let fallback_uid_next = uids.last().and_then(|uid| uid.checked_add(1)).unwrap_or(1);
        let snapshot_uid_next = if floor.is_some() {
            cursor.backfill_uid_next.or(uid_next)
        } else {
            uid_next
        }
        .or(Some(fallback_uid_next));
        cursor.backfill_uid_next = snapshot_uid_next;
        // Keep the folder in backfill mode until the walk completes.
        cursor.scope_backfill_required = true;

        let mut total = 0u32;
        for (index, page) in uids.rchunks(HISTORY_PAGE_SIZE as usize).enumerate() {
            let outcome = self
                .sync_uid_page(folder, page, notify_new && index == 0)
                .await?;
            total += outcome.stored;
            add_imap_retry_uids(cursor, &outcome.failed_uids);
            cursor.backfill_floor_uid = page.first().copied();
            if let Err(e) = self.checkpoint_imap_folder_cursor(folder, cursor).await {
                warn!(
                    "Failed to checkpoint IMAP backfill for {}: {}",
                    folder.name, e
                );
            }
        }

        Ok(ImapHistorySyncOutcome {
            count: total,
            uid_next: snapshot_uid_next,
        })
    }

    /// UIDVALIDITY changes invalidate every numeric UID in one mailbox. Fetch
    /// and stage the complete replacement first, then swap generations in one
    /// SQLite transaction so a network, parse, file, or DB failure leaves the
    /// old mailbox and its user-owned relations untouched.
    pub(super) async fn reconcile_folder_after_uidvalidity_change(
        &self,
        folder: &pebble_core::Folder,
    ) -> Result<ImapHistorySyncOutcome> {
        let status = self
            .provider
            .inner()
            .get_mailbox_status(&folder.remote_id)
            .await?;
        let (ranges, uid_next) = imap_uidvalidity_snapshot_plan(status.exists, status.uid_next);
        let mut snapshot: Vec<pebble_store::messages::ImapFolderSnapshotMessage> = Vec::new();
        let mut thread_mappings = std::collections::HashMap::new();
        for (start_seq, end_seq) in ranges {
            let raw_page = match self
                .provider
                .inner()
                .fetch_messages_page(&folder.remote_id, start_seq, end_seq)
                .await
            {
                Ok(page) => page,
                Err(error) => {
                    for entry in &snapshot {
                        cleanup_staged_attachment_files(&entry.attachments);
                    }
                    return Err(error);
                }
            };
            let page_uids: Vec<u32> = raw_page.iter().map(|(uid, _)| *uid).collect();
            let page_flags = self
                .fetch_initial_flags(&folder.remote_id, &page_uids)
                .await;
            for (uid, raw) in raw_page {
                let (page_is_read, page_is_starred) =
                    page_flags.get(&uid).copied().unwrap_or((false, false));
                let parsed = match parse_raw_email(&raw) {
                    Ok(parsed) => parsed,
                    Err(error) => {
                        for entry in &snapshot {
                            cleanup_staged_attachment_files(&entry.attachments);
                        }
                        return Err(PebbleError::Internal(format!(
                            "Failed to parse replacement IMAP message UID {uid}: {error}"
                        )));
                    }
                };
                drop(raw);

                let mut ref_ids = HashSet::new();
                for header in [&parsed.in_reply_to, &parsed.references_header]
                    .into_iter()
                    .flatten()
                {
                    for id in header.split_whitespace() {
                        ref_ids.insert(id.trim().to_string());
                    }
                }
                let ref_ids: Vec<_> = ref_ids.into_iter().collect();
                match self
                    .base
                    .store
                    .get_thread_mappings_for_refs(&self.base.account_id, &ref_ids)
                {
                    Ok(existing) => thread_mappings.extend(existing),
                    Err(error) => {
                        for entry in &snapshot {
                            cleanup_staged_attachment_files(&entry.attachments);
                        }
                        return Err(error);
                    }
                }

                let now = now_timestamp();
                let mut message = Message {
                    id: new_id(),
                    account_id: self.base.account_id.clone(),
                    remote_id: uid.to_string(),
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
                    is_read: page_is_read,
                    is_starred: page_is_starred,
                    is_draft: false,
                    date: parsed.date,
                    remote_version: None,
                    is_deleted: false,
                    deleted_at: None,
                    created_at: now,
                    updated_at: now,
                };
                message.thread_id = Some(compute_thread_id(&message, &thread_mappings));
                if let (Some(message_id), Some(thread_id)) =
                    (&message.message_id_header, &message.thread_id)
                {
                    thread_mappings.insert(message_id.clone(), thread_id.clone());
                }

                let attachments = match stage_message_attachments_async(
                    self.base.attachments_dir.clone(),
                    message.id.clone(),
                    parsed.attachments,
                )
                .await
                {
                    Ok(attachments) => attachments,
                    Err(error) => {
                        for entry in &snapshot {
                            cleanup_staged_attachment_files(&entry.attachments);
                        }
                        return Err(error);
                    }
                };
                snapshot.push(pebble_store::messages::ImapFolderSnapshotMessage {
                    message,
                    attachments,
                });
            }
        }

        let fresh_count = snapshot.len() as u32;
        let staged_attachments: Vec<_> = snapshot
            .iter()
            .flat_map(|entry| entry.attachments.iter().cloned())
            .collect();
        let reconcile = match self.base.store.reconcile_imap_folder_uidvalidity(
            &self.base.account_id,
            &folder.id,
            snapshot,
        ) {
            Ok(result) => result,
            Err(error) => {
                cleanup_staged_attachment_files(&staged_attachments);
                return Err(error);
            }
        };

        for path in &reconcile.unreferenced_attachment_paths {
            if let Err(error) = std::fs::remove_file(path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    warn!("Failed to remove superseded IMAP attachment {path}: {error}");
                }
            }
        }
        for message_id in &reconcile.indexed_message_ids {
            if let Some(message) = self.base.store.get_message(message_id)? {
                self.base.emit_message(StoredMessage {
                    folder_ids: self.base.store.get_message_folder_ids(message_id)?,
                    message,
                    notify: false,
                    reconciliation: true,
                });
            }
        }
        for message in reconcile.removed_messages {
            self.base.emit_message(StoredMessage {
                message,
                folder_ids: Vec::new(),
                notify: false,
                reconciliation: true,
            });
        }

        Ok(ImapHistorySyncOutcome {
            count: fresh_count,
            uid_next,
        })
    }

    /// Store a batch of raw fetched messages, resolving threads and
    /// attachments. Returns the number of newly stored messages and the UIDs
    /// that failed to parse or store (also recorded in `sync_failures`).
    /// Best-effort `(is_read, is_starred)` for freshly downloaded UIDs, so new
    /// messages start with the server's read/star state instead of waiting
    /// for the next flag reconcile. On error every UID falls back to
    /// unread/unstarred (the reconcile pass corrects it later).
    async fn fetch_initial_flags(
        &self,
        mailbox: &str,
        uids: &[u32],
    ) -> std::collections::HashMap<u32, (bool, bool)> {
        if uids.is_empty() {
            return std::collections::HashMap::new();
        }
        match self.provider.inner().fetch_flags(mailbox, uids).await {
            Ok(flags) => flags
                .into_iter()
                .map(|(uid, is_read, is_starred)| (uid, (is_read, is_starred)))
                .collect(),
            Err(error) => {
                warn!("Failed to fetch initial IMAP flags for {mailbox}: {error}");
                std::collections::HashMap::new()
            }
        }
    }

    pub(super) async fn store_fetched_messages(
        &self,
        folder: &pebble_core::Folder,
        raw_messages: Vec<(u32, Vec<u8>)>,
        notify_new: bool,
    ) -> Result<ImapStoreOutcome> {
        if raw_messages.is_empty() {
            return Ok(ImapStoreOutcome::default());
        }

        // Bulk-check which UIDs already exist to avoid N+1 queries
        let all_remote_ids: Vec<String> = raw_messages
            .iter()
            .map(|(uid, _)| uid.to_string())
            .collect();
        let account_id = self.base.account_id.clone();
        let folder_id = folder.id.clone();
        let (existing_ids, incomplete_attachments) =
            blocking_store(&self.base.store, move |store| {
                Ok((
                    store
                        .get_existing_remote_ids_in_folder(&account_id, &folder_id, &all_remote_ids)
                        .unwrap_or_default(),
                    store
                        .get_incomplete_attachment_message_map_by_remote_ids(
                            &account_id,
                            Some(&folder_id),
                            &all_remote_ids,
                        )
                        .unwrap_or_default(),
                ))
            })
            .await?;

        // Parse all raw messages upfront so we can collect In-Reply-To / References
        // before querying thread mappings (avoids loading the full account mapping).
        let parsed_messages: Vec<(u32, Result<crate::parser::ParsedMessage>)> = raw_messages
            .into_iter()
            .filter(|(uid, _)| {
                let remote_id = uid.to_string();
                if !remote_message_requires_fetch(
                    existing_ids.contains(&remote_id),
                    incomplete_attachments.contains_key(&remote_id),
                ) {
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

        let new_uids: Vec<u32> = parsed_messages.iter().map(|(uid, _)| *uid).collect();
        let initial_flags = self.fetch_initial_flags(&folder.remote_id, &new_uids).await;

        // Collect all referenced message-ID headers from this batch.
        let ref_ids = collect_ref_ids_from_parsed(&parsed_messages);

        // Load thread mappings only for the IDs referenced by this batch.
        // This is mutable so we can extend it as we store new messages within the batch,
        // ensuring intra-batch replies find their parent's thread.
        let account_id = self.base.account_id.clone();
        let mut thread_mappings = blocking_store(&self.base.store, move |store| {
            Ok(store
                .get_thread_mappings_for_refs(&account_id, &ref_ids)
                .unwrap_or_default())
        })
        .await?;

        let mut outcome = ImapStoreOutcome::default();
        let mut thread_merges = Vec::new();

        for (uid, parse_result) in parsed_messages {
            let remote_id = uid.to_string();
            let repairing_incomplete = incomplete_attachments.contains_key(&remote_id);

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
                    outcome.failed_uids.push(uid);
                    continue;
                }
            };

            let now = now_timestamp();

            // Build a temporary message to compute thread_id
            let mut msg = Message {
                id: incomplete_attachments
                    .get(&remote_id)
                    .cloned()
                    .unwrap_or_else(new_id),
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
                is_read: initial_flags.get(&uid).is_some_and(|(read, _)| *read),
                is_starred: initial_flags.get(&uid).is_some_and(|(_, starred)| *starred),
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
            let mut folder_ids = vec![folder.id.clone()];
            if repairing_incomplete {
                if let Err(error) = preserve_local_state_for_attachment_repair(
                    &self.base.store,
                    &mut msg,
                    &mut folder_ids,
                ) {
                    let reason = error.to_string();
                    let _ = self.base.store.upsert_sync_failure(
                        &self.base.account_id,
                        &folder.id,
                        &remote_id,
                        "imap",
                        &reason,
                    );
                    error!("Failed to prepare incomplete IMAP message repair: {error}");
                    outcome.failed_uids.push(uid);
                    continue;
                }
            }

            match store_new_message_with_attachments_atomically(
                Arc::clone(&self.base.store),
                self.base.attachments_dir.clone(),
                msg.clone(),
                folder_ids.clone(),
                parsed.attachments,
            )
            .await
            {
                Ok(()) => {
                    outcome.stored += 1;
                    let _ = self.base.store.clear_sync_failure(
                        &self.base.account_id,
                        &folder.id,
                        &remote_id,
                    );
                    // Update in-memory thread mappings so later messages in this batch
                    // can find this message as a thread parent, and re-parent any
                    // thread that was filed under this message or its ancestors
                    // before they arrived.
                    if let Some(tid) = &msg.thread_id {
                        if let Some(mid) = &msg.message_id_header {
                            thread_mappings.insert(mid.clone(), tid.clone());
                        }
                        let aliases = thread_merge_aliases(&msg, tid);
                        apply_thread_merge_to_mappings(&mut thread_mappings, tid, &aliases);
                        thread_merges.push(ThreadMerge {
                            thread_id: tid.clone(),
                            aliases,
                        });
                    }

                    // Notify listeners (e.g. search indexer) about the new message
                    self.base.emit_message(StoredMessage {
                        message: msg.clone(),
                        folder_ids,
                        notify: notify_new && !repairing_incomplete,
                        reconciliation: repairing_incomplete,
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
                    outcome.failed_uids.push(uid);
                }
            }
        }

        match merge_thread_aliases(&self.base.store, &self.base.account_id, thread_merges).await {
            Ok(moved) if moved.is_empty() => {}
            Ok(moved) => {
                debug!(
                    "Re-parented {} messages into their conversations in {}",
                    moved.len(),
                    folder.name
                );
                self.base.emit_thread_refresh(moved).await;
            }
            Err(e) => warn!("Failed to merge IMAP threads in {}: {}", folder.name, e),
        }

        Ok(outcome)
    }
}
