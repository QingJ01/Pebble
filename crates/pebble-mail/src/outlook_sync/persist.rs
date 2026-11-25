use super::*;

impl OutlookSyncWorker {
    pub(super) async fn persist_folder_messages_with_attachment_fetch<F, Fut>(
        &self,
        folder: &Folder,
        messages: Vec<Message>,
        notify_new: bool,
        mut fetch_attachments: F,
    ) -> u32
    where
        F: FnMut(String) -> Fut,
        Fut: Future<Output = Result<Vec<crate::parser::AttachmentData>>>,
    {
        let remote_ids: Vec<String> = messages.iter().map(|m| m.remote_id.clone()).collect();
        let account_id = self.base.account_id.clone();
        let existing = match blocking_store(&self.base.store, move |store| {
            store.get_existing_message_map_by_remote_ids(&account_id, &remote_ids)
        })
        .await
        {
            Ok(existing) => existing,
            Err(e) => {
                warn!("Failed to look up existing Outlook messages: {e}");
                return u32::try_from(messages.len()).unwrap_or(u32::MAX);
            }
        };
        // Existing messages with queued local operations keep their local
        // flags and folder membership until the operation reaches Graph.
        let pending = match message_ids_with_pending_local_changes(
            &self.base.store,
            &self.base.account_id,
            existing.values().cloned().collect(),
        )
        .await
        {
            Ok(pending) => pending,
            Err(e) => {
                warn!("Failed to look up pending Outlook operations: {e}");
                return u32::try_from(messages.len()).unwrap_or(u32::MAX);
            }
        };

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

        let account_id = self.base.account_id.clone();
        let mut thread_mappings = blocking_store(&self.base.store, move |store| {
            Ok(store
                .get_thread_mappings_for_refs(&account_id, &ref_ids)
                .unwrap_or_default())
        })
        .await
        .unwrap_or_default();
        let mut failure_count = 0;

        for msg in &messages {
            if let Some(local_id) = existing.get(&msg.remote_id) {
                let current = match self.base.store.get_message(local_id) {
                    Ok(Some(current)) => current,
                    Ok(None) => {
                        failure_count += 1;
                        warn!(
                            "Existing Outlook message {} disappeared before update",
                            msg.remote_id
                        );
                        continue;
                    }
                    Err(e) => {
                        failure_count += 1;
                        warn!("Failed to load Outlook message {}: {e}", msg.remote_id);
                        continue;
                    }
                };
                let attachments = match self.base.store.list_attachments_by_message(local_id) {
                    Ok(attachments) => attachments,
                    Err(e) => {
                        failure_count += 1;
                        warn!(
                            "Failed to load attachments before updating Outlook message {}: {e}",
                            msg.remote_id
                        );
                        continue;
                    }
                };

                let mut updated = msg.clone();
                updated.id = local_id.clone();
                updated.created_at = current.created_at;
                updated.thread_id =
                    provider_thread_id(OUTLOOK_THREAD_NAMESPACE, msg.thread_id.as_deref())
                        .or(current.thread_id.clone());
                let folder_ids = if pending.contains(local_id) {
                    updated.is_read = current.is_read;
                    updated.is_starred = current.is_starred;
                    match self.base.store.get_message_folder_ids(local_id) {
                        Ok(folder_ids) => folder_ids,
                        Err(e) => {
                            failure_count += 1;
                            warn!(
                                "Failed to load folders of Outlook message {}: {e}",
                                msg.remote_id
                            );
                            continue;
                        }
                    }
                } else {
                    vec![folder.id.clone()]
                };
                let remote_attachments = if updated.has_attachments {
                    match fetch_attachments(msg.remote_id.clone()).await {
                        Ok(attachments) => attachments,
                        Err(e) => {
                            failure_count += 1;
                            warn!(
                                "Failed to fetch Outlook attachments for existing message {}: {e}",
                                msg.remote_id
                            );
                            continue;
                        }
                    }
                } else {
                    Vec::new()
                };
                let replacement_attachments = match stage_message_attachments_async(
                    self.base.attachments_dir.clone(),
                    local_id.clone(),
                    remote_attachments,
                )
                .await
                {
                    Ok(attachments) => attachments,
                    Err(e) => {
                        failure_count += 1;
                        warn!(
                            "Failed to stage Outlook attachments for existing message {}: {e}",
                            msg.remote_id
                        );
                        continue;
                    }
                };
                if let Err(e) = self.base.store.replace_message_with_attachments(
                    &updated,
                    &folder_ids,
                    &replacement_attachments,
                ) {
                    cleanup_staged_attachment_files(&replacement_attachments);
                    failure_count += 1;
                    warn!("Failed to update Outlook message {}: {e}", msg.remote_id);
                    continue;
                }

                for attachment in &attachments {
                    let Some(path) = attachment.local_path.as_deref() else {
                        continue;
                    };
                    match self.base.store.is_attachment_local_path_referenced(path) {
                        Ok(false) => {
                            if let Err(e) = std::fs::remove_file(path) {
                                if e.kind() != std::io::ErrorKind::NotFound {
                                    warn!(
                                        "Failed to remove obsolete Outlook attachment {path}: {e}"
                                    );
                                }
                            }
                        }
                        Ok(true) => {}
                        Err(e) => {
                            warn!("Failed to check Outlook attachment path reference {path}: {e}")
                        }
                    }
                }

                if let (Some(mid), Some(tid)) = (&updated.message_id_header, &updated.thread_id) {
                    thread_mappings.insert(mid.clone(), tid.clone());
                }
                // Update of an already-stored message: re-index only; never
                // surface as new mail or re-run user rules on it.
                let updated_id = updated.id.clone();
                self.base.emit_message(StoredMessage {
                    message: updated,
                    folder_ids: folder_ids.clone(),
                    notify: false,
                    reconciliation: true,
                });
                self.base.emit_list_refresh(folder_ids, vec![updated_id]);
                continue;
            }

            let mut msg = msg.clone();
            // Prefer Graph's conversationId so arrival order cannot split a
            // conversation; fall back to header threading without one.
            msg.thread_id = Some(
                provider_thread_id(OUTLOOK_THREAD_NAMESPACE, msg.thread_id.as_deref())
                    .unwrap_or_else(|| compute_thread_id(&msg, &thread_mappings)),
            );

            let folder_ids = vec![folder.id.clone()];
            let attachments = if msg.has_attachments {
                match fetch_attachments(msg.remote_id.clone()).await {
                    Ok(attachments) => attachments,
                    Err(e) => {
                        failure_count += 1;
                        warn!(
                            "Failed to fetch Outlook attachments for {}: {e}",
                            msg.remote_id
                        );
                        continue;
                    }
                }
            } else {
                Vec::new()
            };
            if let Err(e) = store_new_message_with_attachments_atomically(
                Arc::clone(&self.base.store),
                self.base.attachments_dir.clone(),
                msg.clone(),
                folder_ids.clone(),
                attachments,
            )
            .await
            {
                warn!("Failed to store Outlook message atomically: {e}");
                failure_count += 1;
                continue;
            }

            if let (Some(mid), Some(tid)) = (&msg.message_id_header, &msg.thread_id) {
                thread_mappings.insert(mid.clone(), tid.clone());
            }

            self.base.emit_message(StoredMessage {
                message: msg.clone(),
                folder_ids,
                notify: notify_new,
                reconciliation: false,
            });
        }

        failure_count
    }

    pub(super) async fn save_outlook_folder_state(
        &self,
        folder: &Folder,
        state: String,
    ) -> Result<()> {
        let account_id = self.base.account_id.clone();
        let folder_id = folder.id.clone();
        blocking_store(&self.base.store, move |store| {
            store.set_folder_sync_state(&account_id, &folder_id, &state)
        })
        .await
    }
}
