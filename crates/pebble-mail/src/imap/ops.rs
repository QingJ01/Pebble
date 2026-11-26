use super::*;

impl ImapProvider {
    /// Set flags on a message identified by UID.
    pub async fn set_flags(
        &self,
        mailbox: &str,
        uid: u32,
        is_read: Option<bool>,
        is_starred: Option<bool>,
    ) -> Result<()> {
        let uid_str = uid.to_string();

        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;

        macro_rules! do_store {
            ($s:expr) => {{
                with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, $s.select(mailbox)).await?;

                if let Some(read) = is_read {
                    let flag_cmd = if read {
                        format!("+FLAGS (\\Seen)")
                    } else {
                        format!("-FLAGS (\\Seen)")
                    };
                    let store_result = with_imap_timeout(
                        "STORE \\Seen",
                        IMAP_COMMAND_TIMEOUT_SECS,
                        $s.uid_store(&uid_str, &flag_cmd),
                    )
                    .await?;
                    let _: Vec<async_imap::types::Fetch> = with_imap_timeout(
                        "STORE \\Seen collect",
                        IMAP_COMMAND_TIMEOUT_SECS,
                        store_result.try_collect(),
                    )
                    .await?;
                }

                if let Some(starred) = is_starred {
                    let flag_cmd = if starred {
                        format!("+FLAGS (\\Flagged)")
                    } else {
                        format!("-FLAGS (\\Flagged)")
                    };
                    let store_result = with_imap_timeout(
                        "STORE \\Flagged",
                        IMAP_COMMAND_TIMEOUT_SECS,
                        $s.uid_store(&uid_str, &flag_cmd),
                    )
                    .await?;
                    let _: Vec<async_imap::types::Fetch> = with_imap_timeout(
                        "STORE \\Flagged collect",
                        IMAP_COMMAND_TIMEOUT_SECS,
                        store_result.try_collect(),
                    )
                    .await?;
                }
            }};
        }

        do_store!(sess);

        Ok(())
    }

    /// Move a message by UID from one mailbox to another.
    ///
    /// Tries IMAP MOVE (uid_mv) first, falls back to UID COPY + UID STORE \Deleted + EXPUNGE.
    /// One-shot wrapper around [`move_message_with_dedup`] with retry-dedup disabled.
    pub async fn move_message(
        &self,
        source_mailbox: &str,
        uid: u32,
        dest_mailbox: &str,
    ) -> Result<()> {
        self.move_message_with_dedup(source_mailbox, uid, dest_mailbox, None)
            .await
    }

    /// Move a message by UID with an idempotent COPY fallback.
    ///
    /// Pass the message's `Message-ID` header so that, on retry after a partial
    /// COPY/EXPUNGE failure, the destination is searched for a matching copy
    /// before COPY runs — preventing duplicate messages. `None` or an empty
    /// header disables the check (one-shot move). `STORE \Deleted` + `EXPUNGE`
    /// are always run and are themselves idempotent.
    pub async fn move_message_with_dedup(
        &self,
        source_mailbox: &str,
        uid: u32,
        dest_mailbox: &str,
        message_id_header: Option<&str>,
    ) -> Result<()> {
        let uid_str = uid.to_string();

        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;

        macro_rules! do_move {
            ($s:expr) => {{
                with_imap_timeout(
                    "SELECT",
                    IMAP_COMMAND_TIMEOUT_SECS,
                    $s.select(source_mailbox),
                )
                .await?;

                // Try MOVE extension first
                match with_imap_timeout(
                    "UID MOVE",
                    IMAP_COMMAND_TIMEOUT_SECS,
                    $s.uid_mv(&uid_str, dest_mailbox),
                )
                .await
                {
                    Ok(_) => {
                        debug!(
                            "MOVE UID {} from {} to {} succeeded",
                            uid, source_mailbox, dest_mailbox
                        );
                    }
                    Err(_move_err) => {
                        // Fallback: COPY + flag Deleted + EXPUNGE
                        debug!(
                            "MOVE not supported, falling back to COPY+DELETE for UID {}",
                            uid
                        );

                        // Idempotency: on retry after a partial COPY/EXPUNGE
                        // failure the destination may already hold a copy (COPY
                        // preserves the Message-ID header). Search the dest and
                        // skip COPY if a match is found, so a retried move does
                        // not create duplicates. Any error here is treated as
                        // "no match" (fall through to COPY) — never block the move.
                        let mut already_copied = false;
                        if let Some(h) = message_id_header {
                            if !h.is_empty() {
                                let criteria = format!(
                                    "HEADER \"Message-ID\" \"{}\"",
                                    h.replace('\\', "\\\\").replace('"', "\\\"")
                                );
                                let dest_selected = with_imap_timeout(
                                    "SELECT",
                                    IMAP_COMMAND_TIMEOUT_SECS,
                                    $s.select(dest_mailbox),
                                )
                                .await
                                .is_ok();
                                if dest_selected {
                                    already_copied = match with_imap_timeout(
                                        "UID SEARCH",
                                        IMAP_COMMAND_TIMEOUT_SECS,
                                        $s.uid_search(criteria.as_str()),
                                    )
                                    .await
                                    {
                                        Ok(hits) => hits.into_iter().next().is_some(),
                                        Err(_) => false,
                                    };
                                }
                            }
                        }

                        // Re-select source (restores selection after the dest
                        // search above and in case the MOVE attempt changed state).
                        with_imap_timeout(
                            "SELECT",
                            IMAP_COMMAND_TIMEOUT_SECS,
                            $s.select(source_mailbox),
                        )
                        .await?;

                        if !already_copied {
                            with_imap_timeout(
                                "UID COPY",
                                IMAP_COMMAND_TIMEOUT_SECS,
                                $s.uid_copy(&uid_str, dest_mailbox),
                            )
                            .await?;
                        }

                        let store_result = with_imap_timeout(
                            "STORE \\Deleted",
                            IMAP_COMMAND_TIMEOUT_SECS,
                            $s.uid_store(&uid_str, "+FLAGS (\\Deleted)"),
                        )
                        .await?;
                        let _: Vec<async_imap::types::Fetch> = with_imap_timeout(
                            "STORE \\Deleted collect",
                            IMAP_COMMAND_TIMEOUT_SECS,
                            store_result.try_collect(),
                        )
                        .await?;

                        let expunge_result =
                            with_imap_timeout("EXPUNGE", IMAP_COMMAND_TIMEOUT_SECS, $s.expunge())
                                .await?;
                        let _: Vec<u32> = with_imap_timeout(
                            "EXPUNGE collect",
                            IMAP_COMMAND_TIMEOUT_SECS,
                            expunge_result.try_collect(),
                        )
                        .await?;

                        debug!(
                            "COPY+DELETE UID {} from {} to {} succeeded",
                            uid, source_mailbox, dest_mailbox
                        );
                    }
                }
            }};
        }

        do_move!(sess);

        Ok(())
    }

    /// Resolve a Message-ID to its mailbox-local UID, including after MOVE.
    pub async fn find_uid_by_message_id(
        &self,
        mailbox: &str,
        message_id_header: &str,
    ) -> Result<Option<u32>> {
        if message_id_header.trim().is_empty() {
            return Ok(None);
        }
        if message_id_header.contains(['\r', '\n']) {
            return Err(PebbleError::Validation("Invalid Message-ID".into()));
        }
        let criteria = format!(
            "HEADER \"Message-ID\" \"{}\"",
            message_id_header.replace('\\', "\\\\").replace('"', "\\\"")
        );
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;
        with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, sess.select(mailbox)).await?;
        let hits = with_imap_timeout(
            "UID SEARCH",
            IMAP_COMMAND_TIMEOUT_SECS,
            sess.uid_search(criteria.as_str()),
        )
        .await?;
        if hits.len() > 1 {
            return Err(PebbleError::Sync(
                "Message-ID matches multiple IMAP messages".into(),
            ));
        }
        Ok(hits.into_iter().next())
    }

    /// Delete a message by UID: flag as \Deleted and EXPUNGE.
    pub async fn delete_message(&self, mailbox: &str, uid: u32) -> Result<()> {
        let uid_str = uid.to_string();

        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;

        macro_rules! do_delete {
            ($s:expr) => {{
                with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, $s.select(mailbox)).await?;

                let store_result = with_imap_timeout(
                    "STORE \\Deleted",
                    IMAP_COMMAND_TIMEOUT_SECS,
                    $s.uid_store(&uid_str, "+FLAGS (\\Deleted)"),
                )
                .await?;
                let _: Vec<async_imap::types::Fetch> = with_imap_timeout(
                    "STORE \\Deleted collect",
                    IMAP_COMMAND_TIMEOUT_SECS,
                    store_result.try_collect(),
                )
                .await?;

                let expunge_result =
                    with_imap_timeout("EXPUNGE", IMAP_COMMAND_TIMEOUT_SECS, $s.expunge()).await?;
                let _: Vec<u32> = with_imap_timeout(
                    "EXPUNGE collect",
                    IMAP_COMMAND_TIMEOUT_SECS,
                    expunge_result.try_collect(),
                )
                .await?;

                debug!("Deleted UID {} from {}", uid, mailbox);
            }};
        }

        do_delete!(sess);

        Ok(())
    }

    /// Fetch all UIDs in a mailbox via UID SEARCH ALL.
    pub async fn fetch_all_uids(&self, mailbox: &str) -> Result<Vec<u32>> {
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;

        macro_rules! do_search {
            ($s:expr) => {{
                with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, $s.select(mailbox)).await?;

                let uids: Vec<u32> = with_imap_timeout(
                    "UID SEARCH ALL",
                    IMAP_COMMAND_TIMEOUT_SECS,
                    $s.uid_search("ALL"),
                )
                .await?
                .into_iter()
                .collect();
                uids
            }};
        }

        let results = do_search!(sess);

        Ok(results)
    }

    /// SELECT a mailbox and return the EXISTS count without fetching UIDs.
    pub async fn select_exists(&self, mailbox: &str) -> Result<u32> {
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;

        let mbox =
            with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, sess.select(mailbox)).await?;

        Ok(mbox.exists)
    }

    /// Check if the server advertises the CONDSTORE capability (RFC 4551).
    pub async fn supports_condstore(&self) -> bool {
        let mut guard = self.session.lock().await;
        let sess = match guard.as_mut() {
            Some(s) => s,
            None => return false,
        };

        match with_imap_timeout("CAPABILITY", IMAP_COMMAND_TIMEOUT_SECS, sess.capabilities()).await
        {
            Ok(caps) => caps.has_str("CONDSTORE"),
            Err(_) => false,
        }
    }

    /// SELECT a mailbox and return the HIGHESTMODSEQ value if the server supports CONDSTORE.
    /// Returns `Ok(Some(modseq))` if available, `Ok(None)` otherwise.
    pub async fn get_highest_modseq(&self, mailbox: &str) -> Result<Option<u64>> {
        Ok(self.get_mailbox_status(mailbox).await?.highest_modseq)
    }

    /// SELECT a mailbox and return the UIDVALIDITY/HIGHESTMODSEQ values advertised by the server.
    pub async fn get_mailbox_status(&self, mailbox: &str) -> Result<ImapMailboxStatus> {
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;

        macro_rules! do_select {
            ($s:expr) => {{
                let mailbox_info =
                    with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, $s.select(mailbox))
                        .await?;
                ImapMailboxStatus {
                    uid_validity: mailbox_info.uid_validity,
                    highest_modseq: mailbox_info.highest_modseq,
                    uid_next: mailbox_info.uid_next,
                    exists: mailbox_info.exists,
                }
            }};
        }

        let result = do_select!(sess);

        Ok(result)
    }

    /// Fetch flags for a set of UIDs along with per-message MODSEQ values.
    /// Returns `(flags_vec, highest_modseq)` where highest_modseq is the maximum
    /// MODSEQ seen across all fetched messages (or 0 if the server did not return any).
    ///
    /// The UID set is compressed into ranges and split across several
    /// commands so each command line stays below server limits. MODSEQ is
    /// only requested when the server advertised HIGHESTMODSEQ on SELECT.
    pub async fn fetch_flags_with_modseq(
        &self,
        mailbox: &str,
        uids: &[u32],
    ) -> Result<(Vec<(u32, bool, bool)>, u64)> {
        if uids.is_empty() {
            return Ok((Vec::new(), 0));
        }

        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;
        let mailbox_info =
            with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, sess.select(mailbox)).await?;
        let query = if mailbox_info.highest_modseq.is_some() {
            "(UID FLAGS MODSEQ)"
        } else {
            "(UID FLAGS)"
        };

        let mut highest = 0u64;
        let mut results = Vec::with_capacity(uids.len());
        for uid_set in compress_uid_set(uids) {
            let fetches = Self::uid_fetch_collect(sess, &uid_set, query, "UID FETCH FLAGS").await?;
            for fetch in fetches {
                let Some(uid) = fetch.uid else {
                    tracing::warn!(
                        "Skipping flags for message without UID (seq={})",
                        fetch.message
                    );
                    continue;
                };
                if let Some(ms) = fetch.modseq {
                    highest = highest.max(ms);
                }
                let (is_read, is_starred) = parse_flags(fetch.flags());
                results.push((uid, is_read, is_starred));
            }
        }
        Ok((results, highest))
    }

    /// Check if the server advertises the IDLE capability (RFC 2177).
    pub async fn supports_idle(&self) -> bool {
        let mut guard = self.session.lock().await;
        let sess = match guard.as_mut() {
            Some(s) => s,
            None => return false,
        };

        match with_imap_timeout("CAPABILITY", IMAP_COMMAND_TIMEOUT_SECS, sess.capabilities()).await
        {
            Ok(caps) => caps.has_str("IDLE"),
            Err(_) => false,
        }
    }

    /// Enter IMAP IDLE mode and wait for server notifications or timeout.
    ///
    /// The session is temporarily taken out of `self.session` while IDLE is
    /// active and restored when the command completes (or on error).
    /// Timeout should be <= 29 minutes per RFC 2177 recommendation.
    pub async fn idle_wait(
        &self,
        mailbox: &str,
        timeout_dur: std::time::Duration,
    ) -> Result<crate::idle::IdleEvent> {
        // Take the session out so we can pass ownership to the idle handle.
        let sess = {
            let mut guard = self.session.lock().await;
            guard
                .take()
                .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?
        };

        // Select the mailbox first.
        let mut session = sess;
        if let Err(e) =
            with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, session.select(mailbox)).await
        {
            // Restore session before returning error.
            let mut guard = self.session.lock().await;
            *guard = Some(session);
            return Err(e);
        }

        let mut idle_handle = session.idle();
        if let Err(e) =
            with_imap_timeout("IDLE init", IMAP_COMMAND_TIMEOUT_SECS, idle_handle.init()).await
        {
            // init() failed; the handle still owns the session.
            // Call done() to recover the session.
            match idle_handle.done().await {
                Ok(recovered) => {
                    let mut guard = self.session.lock().await;
                    *guard = Some(recovered);
                }
                Err(_) => {
                    // Session is lost; caller will need to reconnect.
                }
            }
            return Err(e);
        }

        let (wait_fut, _stop_source) = idle_handle.wait_with_timeout(timeout_dur);
        let idle_result = wait_fut.await;

        // Recover the session by sending DONE.
        let event = match idle_result {
            Ok(resp) => {
                use async_imap::extensions::idle::IdleResponse;
                match resp {
                    IdleResponse::NewData(_) => crate::idle::IdleEvent::NewMail,
                    IdleResponse::Timeout => crate::idle::IdleEvent::Timeout,
                    IdleResponse::ManualInterrupt => crate::idle::IdleEvent::Timeout,
                }
            }
            Err(e) => crate::idle::IdleEvent::Error(format!("IDLE wait error: {e}")),
        };

        match idle_handle.done().await {
            Ok(recovered) => {
                let mut guard = self.session.lock().await;
                *guard = Some(recovered);
            }
            Err(_) => {
                // Session is lost; caller will need to reconnect.
                tracing::warn!("Failed to recover session after IDLE DONE");
            }
        }

        Ok(event)
    }

    /// Disconnect from the IMAP server.
    pub async fn disconnect(&self) -> Result<()> {
        let mut guard = self.session.lock().await;
        if let Some(sess) = guard.as_mut() {
            let _ = sess.logout().await;
            *guard = None;
        }
        Ok(())
    }
}
