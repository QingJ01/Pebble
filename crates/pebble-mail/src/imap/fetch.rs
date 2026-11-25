use super::*;

impl ImapProvider {
    /// List folders for the given account, returning `Folder` structs.
    pub async fn list_folders(&self, account_id: &str) -> Result<Vec<Folder>> {
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;

        let mailboxes: Vec<(String, Option<String>, Vec<NameAttribute<'static>>)> = {
            let stream = sess
                .list(None, Some("*"))
                .await
                .map_err(|e| PebbleError::Network(format!("LIST failed: {e}")))?;
            stream
                .map_ok(|n| {
                    let attributes = n
                        .attributes()
                        .iter()
                        .cloned()
                        .map(NameAttribute::into_owned)
                        .collect();
                    (
                        n.name().to_string(),
                        n.delimiter().map(str::to_owned),
                        attributes,
                    )
                })
                .try_collect()
                .await
                .map_err(|e| PebbleError::Network(format!("LIST collect: {e}")))?
        };

        let mut folders: Vec<Folder> = mailboxes
            .into_iter()
            .filter(|(_, _, attributes)| should_sync_listed_mailbox(attributes))
            .map(|(raw_name, delimiter, attributes)| {
                // Decode IMAP Modified UTF-7 folder name to UTF-8
                let display_name = utf7_imap::decode_utf7_imap(raw_name.clone());
                let role = detect_folder_role_from_attributes(&attributes)
                    .or_else(|| detect_folder_role(&raw_name))
                    .or_else(|| detect_folder_role(&display_name));
                let sort_order = folder_sort_order(&role);
                let parent_id = delimiter
                    .as_deref()
                    .filter(|d| !d.is_empty())
                    .and_then(|d| raw_name.rsplit_once(d).map(|(parent, _)| parent.to_owned()));
                let is_system = role.is_some()
                    || attributes.iter().any(|a| {
                        matches!(a, NameAttribute::All | NameAttribute::Flagged)
                            || matches!(a,
                    NameAttribute::Extension(value) if value.eq_ignore_ascii_case("\\Important"))
                    });
                Folder {
                    id: new_id(),
                    account_id: account_id.to_string(),
                    remote_id: raw_name,
                    name: display_name,
                    folder_type: FolderType::Folder,
                    role,
                    parent_id,
                    color: None,
                    is_system,
                    sort_order,
                }
            })
            .collect();

        folders.sort_by_key(|f| f.sort_order);
        Ok(folders)
    }

    /// Fetch raw message bytes from a mailbox.
    /// Returns a list of `(uid, raw_bytes)` pairs.
    pub async fn fetch_messages_raw(
        &self,
        mailbox: &str,
        since_uid: Option<u32>,
        limit: u32,
    ) -> Result<Vec<(u32, Vec<u8>)>> {
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;

        macro_rules! do_fetch {
            ($s:expr) => {{
                let mailbox_info =
                    with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, $s.select(mailbox))
                        .await?;

                let exists = mailbox_info.exists;
                if exists == 0 {
                    return Ok(Vec::new());
                }

                let mut results = Vec::new();

                if let Some(uid) = since_uid {
                    let next_uid = match uid.checked_add(1) {
                        Some(n) => n,
                        None => return Ok(Vec::new()),
                    };
                    if !should_search_incremental_uids(next_uid, mailbox_info.uid_next) {
                        return Ok(Vec::new());
                    }
                    let search_query = format!("UID {next_uid}:*");
                    let searched_uids: Vec<u32> = with_imap_timeout(
                        "UID SEARCH",
                        IMAP_COMMAND_TIMEOUT_SECS,
                        $s.uid_search(&search_query),
                    )
                    .await?
                    .into_iter()
                    .collect();
                    let fetch_uids = incremental_fetch_uids(Some(uid), &searched_uids);
                    if fetch_uids.is_empty() {
                        return Ok(Vec::new());
                    }
                    for uid_set in compress_uid_set(&fetch_uids) {
                        let fetches =
                            Self::uid_fetch_collect($s, &uid_set, "(UID BODY.PEEK[])", "UID FETCH")
                                .await?;
                        for fetch in fetches {
                            if let Some(uid) = fetch.uid {
                                if let Some(body) = fetch.body() {
                                    results.push((uid, body.to_vec()));
                                }
                            } else {
                                tracing::warn!(
                                    "Skipping message without UID (seq={})",
                                    fetch.message
                                );
                            }
                        }
                    }
                } else {
                    let start = if exists > limit {
                        exists - limit + 1
                    } else {
                        1
                    };
                    return Self::fetch_messages_sequence_range($s, start, exists).await;
                }

                results
            }};
        }

        let results = do_fetch!(sess);

        Ok(results)
    }

    /// Fetch the raw bytes of the messages whose sequence numbers fall within
    /// `[start_seq, end_seq]` (inclusive), after selecting `mailbox`.
    ///
    /// Sequence numbers are stable for the lifetime of a session; callers that
    /// page backwards through a mailbox must issue these fetches on the same
    /// connection. Returns `(uid, raw_bytes)` pairs ordered by the server
    /// response, which is ascending sequence number.
    pub async fn fetch_messages_page(
        &self,
        mailbox: &str,
        start_seq: u32,
        end_seq: u32,
    ) -> Result<Vec<(u32, Vec<u8>)>> {
        if start_seq == 0 || end_seq < start_seq {
            return Ok(Vec::new());
        }
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;

        macro_rules! do_fetch {
            ($s:expr) => {{
                let mailbox_info =
                    with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, $s.select(mailbox))
                        .await?;
                if mailbox_info.exists == 0 || start_seq > mailbox_info.exists {
                    return Ok(Vec::new());
                }
                let end_seq = end_seq.min(mailbox_info.exists);
                Self::fetch_messages_sequence_range($s, start_seq, end_seq).await?
            }};
        }

        let results = do_fetch!(sess);

        Ok(results)
    }

    pub async fn search_uids_since(
        &self,
        mailbox: &str,
        cutoff: i64,
        since_uid: Option<u32>,
    ) -> Result<(Vec<u32>, Option<u32>)> {
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;
        let mailbox_info =
            with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, sess.select(mailbox)).await?;
        let Some(criteria) = imap_since_search_criteria(cutoff, since_uid) else {
            return Ok((Vec::new(), mailbox_info.uid_next));
        };
        let mut uids = with_imap_timeout(
            "UID SEARCH",
            IMAP_COMMAND_TIMEOUT_SECS,
            sess.uid_search(&criteria),
        )
        .await?
        .into_iter()
        .collect::<Vec<_>>();
        uids.sort_unstable();
        uids.dedup();
        Ok((uids, mailbox_info.uid_next))
    }

    /// Fetch full message bodies for exactly `uids`; errors when the server
    /// does not return every requested UID.
    pub async fn fetch_messages_by_uids(
        &self,
        mailbox: &str,
        uids: &[u32],
    ) -> Result<Vec<(u32, Vec<u8>)>> {
        self.fetch_uid_sections(mailbox, uids, UidSection::Body, true)
            .await
    }

    /// Fetch full message bodies for `uids`, tolerating UIDs that were
    /// expunged in the meantime. Callers are expected to bound `uids` (see
    /// [`plan_imap_body_batches`]).
    pub async fn fetch_bodies_by_uids(
        &self,
        mailbox: &str,
        uids: &[u32],
    ) -> Result<Vec<(u32, Vec<u8>)>> {
        self.fetch_uid_sections(mailbox, uids, UidSection::Body, false)
            .await
    }

    /// Fetch only the header section of `uids` (used for oversized messages).
    pub async fn fetch_headers_by_uids(
        &self,
        mailbox: &str,
        uids: &[u32],
    ) -> Result<Vec<(u32, Vec<u8>)>> {
        self.fetch_uid_sections(mailbox, uids, UidSection::Header, false)
            .await
    }

    pub(super) async fn fetch_uid_sections(
        &self,
        mailbox: &str,
        uids: &[u32],
        section: UidSection,
        strict: bool,
    ) -> Result<Vec<(u32, Vec<u8>)>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let query = match section {
            UidSection::Body => "(UID BODY.PEEK[])",
            UidSection::Header => "(UID BODY.PEEK[HEADER])",
        };
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;
        let _ =
            with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, sess.select(mailbox)).await?;
        let requested: HashSet<_> = uids.iter().copied().collect();
        let mut fetched = Vec::with_capacity(requested.len());
        let mut fetched_uids = HashSet::with_capacity(requested.len());
        for uid_set in compress_uid_set(uids) {
            let fetches = Self::uid_fetch_collect(sess, &uid_set, query, "UID FETCH").await?;
            for fetch in fetches {
                let Some(uid) = fetch.uid else {
                    continue;
                };
                if !requested.contains(&uid) {
                    continue;
                }
                let bytes = match section {
                    UidSection::Body => fetch.body(),
                    UidSection::Header => fetch.header(),
                };
                let Some(bytes) = bytes else {
                    if strict {
                        return Err(PebbleError::Network(format!(
                            "IMAP UID FETCH returned no body for UID {uid}"
                        )));
                    }
                    continue;
                };
                if fetched_uids.insert(uid) {
                    fetched.push((uid, bytes.to_vec()));
                }
            }
        }
        if strict && fetched_uids != requested {
            return Err(PebbleError::Network(
                "IMAP UID FETCH did not return every requested UID".to_string(),
            ));
        }
        Ok(fetched)
    }

    /// Return `(uid, RFC822.SIZE)` for the requested UIDs without fetching
    /// any message content. UIDs no longer on the server are omitted.
    pub async fn fetch_uid_sizes(&self, mailbox: &str, uids: &[u32]) -> Result<Vec<(u32, u32)>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;
        with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, sess.select(mailbox)).await?;
        let mut sizes = Vec::with_capacity(uids.len());
        for uid_set in compress_uid_set(uids) {
            let fetches =
                Self::uid_fetch_collect(sess, &uid_set, "(UID RFC822.SIZE)", "UID FETCH SIZE")
                    .await?;
            for fetch in fetches {
                if let Some(uid) = fetch.uid {
                    sizes.push((uid, fetch.size.unwrap_or(0)));
                }
            }
        }
        Ok(sizes)
    }

    /// Run `UID SEARCH <criteria>` after selecting `mailbox`. Returns the
    /// sorted UIDs together with the SELECT status snapshot.
    pub async fn search_uids(
        &self,
        mailbox: &str,
        criteria: &str,
    ) -> Result<(Vec<u32>, ImapMailboxStatus)> {
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;
        let mailbox_info =
            with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, sess.select(mailbox)).await?;
        let status = ImapMailboxStatus {
            uid_validity: mailbox_info.uid_validity,
            highest_modseq: mailbox_info.highest_modseq,
            uid_next: mailbox_info.uid_next,
            exists: mailbox_info.exists,
        };
        if status.exists == 0 {
            return Ok((Vec::new(), status));
        }
        let mut uids = with_imap_timeout(
            "UID SEARCH",
            IMAP_COMMAND_TIMEOUT_SECS,
            sess.uid_search(criteria),
        )
        .await?
        .into_iter()
        .collect::<Vec<_>>();
        uids.sort_unstable();
        uids.dedup();
        Ok((uids, status))
    }

    /// UIDs strictly greater than `since_uid`, ascending. Skips the search
    /// entirely when UIDNEXT shows nothing new arrived.
    pub async fn search_uids_after(&self, mailbox: &str, since_uid: u32) -> Result<Vec<u32>> {
        let Some(next_uid) = since_uid.checked_add(1) else {
            return Ok(Vec::new());
        };
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;
        let mailbox_info =
            with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, sess.select(mailbox)).await?;
        if mailbox_info.exists == 0
            || !should_search_incremental_uids(next_uid, mailbox_info.uid_next)
        {
            return Ok(Vec::new());
        }
        let searched: Vec<u32> = with_imap_timeout(
            "UID SEARCH",
            IMAP_COMMAND_TIMEOUT_SECS,
            sess.uid_search(format!("UID {next_uid}:*")),
        )
        .await?
        .into_iter()
        .collect();
        Ok(incremental_fetch_uids(Some(since_uid), &searched))
    }

    /// Return the UIDs among `uids` that still exist on the server. Commands
    /// are issued per compressed chunk so they stay below line-length limits.
    pub async fn existing_uids_among(&self, mailbox: &str, uids: &[u32]) -> Result<Vec<u32>> {
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;
        let mailbox_info =
            with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, sess.select(mailbox)).await?;
        if mailbox_info.exists == 0 {
            return Ok(Vec::new());
        }
        let mut existing = Vec::new();
        for uid_set in compress_uid_set(uids) {
            let hits = with_imap_timeout(
                "UID SEARCH",
                IMAP_COMMAND_TIMEOUT_SECS,
                sess.uid_search(format!("UID {uid_set}")),
            )
            .await?;
            existing.extend(hits);
        }
        existing.sort_unstable();
        existing.dedup();
        Ok(existing)
    }

    /// CONDSTORE flag delta: `UID FETCH 1:* (FLAGS) (CHANGEDSINCE modseq)`.
    /// Returns `(uid, is_read, is_starred)` for every message whose flags
    /// changed after `modseq`, plus the highest MODSEQ observed (0 if none).
    pub async fn fetch_flags_changed_since(
        &self,
        mailbox: &str,
        modseq: u64,
    ) -> Result<(Vec<(u32, bool, bool)>, u64)> {
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;
        let mailbox_info =
            with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, sess.select(mailbox)).await?;
        if mailbox_info.exists == 0 {
            return Ok((Vec::new(), 0));
        }
        let query = format!("(UID FLAGS) (CHANGEDSINCE {modseq})");
        let fetches =
            Self::uid_fetch_collect(sess, "1:*", &query, "UID FETCH CHANGEDSINCE").await?;
        let mut highest = 0u64;
        let mut results = Vec::with_capacity(fetches.len());
        for fetch in fetches {
            let Some(uid) = fetch.uid else {
                continue;
            };
            if let Some(ms) = fetch.modseq {
                highest = highest.max(ms);
            }
            let (is_read, is_starred) = parse_flags(fetch.flags());
            results.push((uid, is_read, is_starred));
        }
        Ok((results, highest))
    }

    /// UID of the newest message in the mailbox (0 when empty), together
    /// with the SELECT status. Uses `FETCH * (UID)` instead of listing every
    /// UID in the mailbox.
    pub async fn highest_uid(&self, mailbox: &str) -> Result<(ImapMailboxStatus, u32)> {
        let mut guard = self.session.lock().await;
        let sess = guard
            .as_mut()
            .ok_or_else(|| PebbleError::Network("Not connected".to_string()))?;
        let mailbox_info =
            with_imap_timeout("SELECT", IMAP_COMMAND_TIMEOUT_SECS, sess.select(mailbox)).await?;
        let status = ImapMailboxStatus {
            uid_validity: mailbox_info.uid_validity,
            highest_modseq: mailbox_info.highest_modseq,
            uid_next: mailbox_info.uid_next,
            exists: mailbox_info.exists,
        };
        if status.exists == 0 {
            return Ok((status, 0));
        }
        let fetches = with_imap_timeout(
            "FETCH * (UID)",
            IMAP_COMMAND_TIMEOUT_SECS,
            sess.fetch("*", "(UID)"),
        )
        .await?;
        let fetches: Vec<async_imap::types::Fetch> = with_imap_timeout(
            "FETCH * (UID) collect",
            IMAP_COMMAND_TIMEOUT_SECS,
            fetches.try_collect(),
        )
        .await?;
        let highest = fetches
            .iter()
            .filter_map(|fetch| fetch.uid)
            .max()
            .unwrap_or(0);
        Ok((status, highest))
    }

    /// Run one `UID FETCH` and collect its responses.
    pub(super) async fn uid_fetch_collect(
        sess: &mut ImapSession,
        uid_set: &str,
        query: &str,
        operation: &str,
    ) -> Result<Vec<async_imap::types::Fetch>> {
        let fetches = with_imap_timeout(
            operation,
            IMAP_COMMAND_TIMEOUT_SECS,
            sess.uid_fetch(uid_set, query),
        )
        .await?;
        with_imap_timeout(
            &format!("{operation} collect"),
            IMAP_COMMAND_TIMEOUT_SECS,
            fetches.try_collect(),
        )
        .await
    }

    /// Fetch the raw bytes of the messages whose sequence numbers fall within
    /// `[start_seq, end_seq]` (inclusive). The caller must have already
    /// selected the mailbox on the same session.
    pub(super) async fn fetch_messages_sequence_range(
        sess: &mut ImapSession,
        start_seq: u32,
        end_seq: u32,
    ) -> Result<Vec<(u32, Vec<u8>)>> {
        let seq_set = if start_seq == end_seq {
            start_seq.to_string()
        } else {
            format!("{start_seq}:{end_seq}")
        };
        let fetches = with_imap_timeout(
            "FETCH",
            IMAP_COMMAND_TIMEOUT_SECS,
            sess.fetch(&seq_set, "(UID BODY.PEEK[])"),
        )
        .await?;
        let fetches: Vec<async_imap::types::Fetch> = with_imap_timeout(
            "FETCH collect",
            IMAP_COMMAND_TIMEOUT_SECS,
            fetches.try_collect(),
        )
        .await?;
        let mut results = Vec::with_capacity(fetches.len());
        for fetch in fetches {
            if let Some(uid) = fetch.uid {
                if let Some(body) = fetch.body() {
                    results.push((uid, body.to_vec()));
                }
            } else {
                tracing::warn!("Skipping message without UID (seq={})", fetch.message);
            }
        }
        Ok(results)
    }

    /// Fetch flags for a set of UIDs. Returns `(uid, is_read, is_starred)`.
    pub async fn fetch_flags(&self, mailbox: &str, uids: &[u32]) -> Result<Vec<(u32, bool, bool)>> {
        Ok(self.fetch_flags_with_modseq(mailbox, uids).await?.0)
    }
}
