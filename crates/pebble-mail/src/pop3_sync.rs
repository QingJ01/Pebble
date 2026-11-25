use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use pebble_core::{new_id, now_timestamp, Folder, FolderRole, FolderType, Message, Result};
use pebble_store::Store;
use tokio::sync::{mpsc, watch};
use tracing::{info, warn};

use crate::backoff::SyncBackoff;
use crate::parser::parse_raw_email;
use crate::pop3::{Pop3MessageRef, Pop3Provider};
use crate::realtime_policy::{RealtimePollPolicy, RealtimeRuntimeState, SyncTrigger};
use crate::sync::{
    preserve_local_state_for_attachment_repair, recv_sync_trigger, remote_message_requires_fetch,
    store_new_message_with_attachments_atomically, StoredMessage, SyncConfig, SyncError,
    SyncProgress, SyncWorkerBase, ThreadRefresh,
};
use crate::thread::{compute_thread_id, merge_thread_aliases, thread_merge_aliases, ThreadMerge};

const POP3_SEEN_UIDLS_KEY: &str = "pop3_seen_uidls";
const POP3_INBOX_REMOTE_ID: &str = "INBOX";

pub struct Pop3SyncWorker {
    base: SyncWorkerBase,
    provider: Arc<Pop3Provider>,
    stop_rx: watch::Receiver<bool>,
}

impl Pop3SyncWorker {
    pub fn new(
        account_id: String,
        provider: Arc<Pop3Provider>,
        store: Arc<Store>,
        stop_rx: watch::Receiver<bool>,
        attachments_dir: PathBuf,
    ) -> Self {
        Self {
            base: SyncWorkerBase {
                account_id,
                store,
                attachments_dir,
                error_tx: None,
                message_tx: None,
                runtime_status_tx: None,
                progress_tx: None,
                thread_refresh_tx: None,
            },
            provider,
            stop_rx,
        }
    }

    pub fn with_error_tx(mut self, error_tx: mpsc::UnboundedSender<SyncError>) -> Self {
        self.base.error_tx = Some(error_tx);
        self
    }

    pub fn with_progress_tx(mut self, progress_tx: mpsc::UnboundedSender<SyncProgress>) -> Self {
        self.base.progress_tx = Some(progress_tx);
        self
    }

    pub fn with_message_tx(mut self, message_tx: mpsc::UnboundedSender<StoredMessage>) -> Self {
        self.base.message_tx = Some(message_tx);
        self
    }

    /// Receive a [`ThreadRefresh`] whenever stored messages change thread.
    pub fn with_thread_refresh_tx(mut self, tx: mpsc::UnboundedSender<ThreadRefresh>) -> Self {
        self.base.thread_refresh_tx = Some(tx);
        self
    }

    pub async fn run(
        &self,
        config: SyncConfig,
        trigger_rx: Option<mpsc::UnboundedReceiver<SyncTrigger>>,
    ) {
        self.base.emit_sync_started("initial");
        let initial_result = self.sync_once(config.initial_fetch_limit, false).await;
        match initial_result {
            Ok(()) => self.base.emit_sync_completed("initial"),
            Err(e) => {
                warn!("POP3 initial sync failed for {}: {e}", self.base.account_id);
                self.base
                    .emit_error("sync", &format!("POP3 initial sync failed: {e}"));
                self.base.emit_sync_error("initial", &e.to_string());
            }
        }
        self.base.rethread_legacy_threads_once().await;

        if config.manual_only() {
            info!("POP3 manual sync completed for {}", self.base.account_id);
            return;
        }

        let policy = RealtimePollPolicy::from_foreground_interval_secs(config.poll_interval_secs);
        let mut backoff = SyncBackoff::new();
        let mut stop_rx = self.stop_rx.clone();
        let mut trigger_rx = trigger_rx;
        let mut runtime = RealtimeRuntimeState::new(Duration::from_secs(60), Instant::now());

        loop {
            let next_delay =
                policy.next_delay(runtime.context(backoff.failure_count(), Instant::now()));
            tokio::select! {
                changed = stop_rx.changed() => {
                    match changed {
                        Ok(()) if *stop_rx.borrow() => break,
                        _ => {}
                    }
                }
                _ = tokio::time::sleep(next_delay) => {
                    self.run_poll_cycle(&mut backoff).await;
                }
                trigger = recv_sync_trigger(&mut trigger_rx) => {
                    match trigger {
                        Some(trigger) => {
                            runtime.record_trigger(trigger, Instant::now());
                            if trigger.should_sync_now() {
                                self.run_poll_cycle(&mut backoff).await;
                            }
                        }
                        None => trigger_rx = None,
                    }
                }
            }
        }
        info!("POP3 sync task completed for {}", self.base.account_id);
    }

    async fn run_poll_cycle(&self, backoff: &mut SyncBackoff) {
        if backoff.is_circuit_open() {
            warn!(
                "Circuit open for POP3 account {} ({} failures), current delay {:?}",
                self.base.account_id,
                backoff.failure_count(),
                backoff.current_delay()
            );
            return;
        }

        self.base.emit_sync_started("poll");
        match self.sync_once(50, true).await {
            Ok(()) => {
                self.base.emit_sync_completed("poll");
                backoff.record_success();
            }
            Err(e) => {
                warn!("POP3 poll failed for {}: {e}", self.base.account_id);
                self.base
                    .emit_error("sync", &format!("POP3 poll failed: {e}"));
                self.base.emit_sync_error("poll", &e.to_string());
                let _ = backoff.record_failure();
            }
        }
    }

    async fn sync_once(&self, limit: u32, notify_new: bool) -> Result<()> {
        let inbox = self.ensure_pop3_folders()?;
        let (remote_messages, fetched_messages) = self
            .provider
            .list_and_retrieve_selected(|remote_messages| {
                let known_uidls = self.known_uidls(&inbox, remote_messages)?;
                Ok(select_pop3_messages_to_fetch(
                    remote_messages,
                    &known_uidls,
                    limit,
                ))
            })
            .await?;

        let mut fetched_uidls = Vec::with_capacity(fetched_messages.len());
        for (message_ref, raw) in fetched_messages {
            let fetched_uid = message_ref.uid.clone();
            self.store_message(&inbox, message_ref, raw, notify_new)
                .await?;
            fetched_uidls.push(fetched_uid);
        }

        let known_uidls = self.known_uidls(&inbox, &remote_messages)?;
        self.persist_seen_uidls(&known_uidls, &fetched_uidls)?;
        Ok(())
    }

    fn ensure_pop3_folders(&self) -> Result<Folder> {
        let existing = self.base.store.list_folders(&self.base.account_id)?;
        let mut inbox = existing
            .iter()
            .find(|folder| folder.role == Some(FolderRole::Inbox))
            .cloned();
        let specs = [
            (POP3_INBOX_REMOTE_ID, "Inbox", Some(FolderRole::Inbox), 0),
            ("__local_archive__", "Archive", Some(FolderRole::Archive), 3),
            ("__local_trash__", "Trash", Some(FolderRole::Trash), 5),
        ];

        for (remote_id, name, role, sort_order) in specs {
            if existing.iter().any(|folder| folder.role == role) {
                continue;
            }
            let folder = Folder {
                id: new_id(),
                account_id: self.base.account_id.clone(),
                remote_id: remote_id.to_string(),
                name: name.to_string(),
                folder_type: FolderType::Folder,
                role: role.clone(),
                parent_id: None,
                color: None,
                is_system: true,
                sort_order,
            };
            let id = self.base.store.insert_folder(&folder)?;
            if role == Some(FolderRole::Inbox) {
                inbox = Some(Folder { id, ..folder });
            }
        }

        inbox.ok_or_else(|| {
            pebble_core::PebbleError::Internal("Failed to create POP3 Inbox".to_string())
        })
    }

    fn known_uidls(
        &self,
        inbox: &Folder,
        remote_messages: &[Pop3MessageRef],
    ) -> Result<HashSet<String>> {
        let remote_ids = remote_messages
            .iter()
            .map(|message| message.uid.clone())
            .collect::<Vec<_>>();
        let mut known = self
            .base
            .store
            .get_existing_remote_ids_in_folder(&self.base.account_id, &inbox.id, &remote_ids)?
            .into_iter()
            .collect::<HashSet<_>>();
        let incomplete = self
            .base
            .store
            .get_incomplete_attachment_message_map_by_remote_ids(
                &self.base.account_id,
                Some(&inbox.id),
                &remote_ids,
            )?;

        if let Some(state) = self.base.store.get_sync_state(&self.base.account_id)? {
            if let Some(value) = state.extra.get(POP3_SEEN_UIDLS_KEY) {
                if let Some(uidls) = value.as_array() {
                    for uid in uidls.iter().filter_map(|uid| uid.as_str()) {
                        known.insert(uid.to_string());
                    }
                }
            }
        }
        known.retain(|remote_id| {
            !remote_message_requires_fetch(true, incomplete.contains_key(remote_id))
        });
        Ok(known)
    }

    fn persist_seen_uidls(
        &self,
        known_uidls: &HashSet<String>,
        fetched_uidls: &[String],
    ) -> Result<()> {
        let uidls = merge_seen_uidls(known_uidls, fetched_uidls);
        self.base
            .store
            .update_sync_state(&self.base.account_id, |state| {
                state.extra.insert(
                    POP3_SEEN_UIDLS_KEY.to_string(),
                    serde_json::Value::Array(
                        uidls.into_iter().map(serde_json::Value::String).collect(),
                    ),
                );
            })
    }

    async fn store_message(
        &self,
        inbox: &Folder,
        message_ref: Pop3MessageRef,
        raw: Vec<u8>,
        notify_new: bool,
    ) -> Result<()> {
        let parsed = parse_raw_email(&raw)?;
        let now = now_timestamp();
        let remote_ids = vec![message_ref.uid.clone()];
        let incomplete = self
            .base
            .store
            .get_incomplete_attachment_message_map_by_remote_ids(
                &self.base.account_id,
                Some(&inbox.id),
                &remote_ids,
            )?;
        let repaired_local_id = incomplete.get(&message_ref.uid).cloned();
        let repairing_incomplete = repaired_local_id.is_some();
        let mut msg = Message {
            id: repaired_local_id.unwrap_or_else(new_id),
            account_id: self.base.account_id.clone(),
            remote_id: message_ref.uid.clone(),
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
            is_read: false,
            is_starred: false,
            is_draft: false,
            date: parsed.date,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        };

        let refs = collect_message_reference_ids(&msg);
        let mappings = self
            .base
            .store
            .get_thread_mappings_for_refs(&self.base.account_id, &refs)
            .unwrap_or_default();
        msg.thread_id = Some(compute_thread_id(&msg, &mappings));
        let mut folder_ids = vec![inbox.id.clone()];
        if repairing_incomplete {
            preserve_local_state_for_attachment_repair(
                &self.base.store,
                &mut msg,
                &mut folder_ids,
            )?;
        }
        store_new_message_with_attachments_atomically(
            Arc::clone(&self.base.store),
            self.base.attachments_dir.clone(),
            msg.clone(),
            folder_ids.clone(),
            parsed.attachments,
        )
        .await?;
        if let Some(thread_id) = msg.thread_id.clone() {
            let aliases = thread_merge_aliases(&msg, &thread_id);
            match merge_thread_aliases(
                &self.base.store,
                &self.base.account_id,
                vec![ThreadMerge { thread_id, aliases }],
            )
            .await
            {
                Ok(moved) => self.base.emit_thread_refresh(moved).await,
                Err(e) => warn!("Failed to merge POP3 threads: {e}"),
            }
        }
        self.base.emit_message(StoredMessage {
            message: msg,
            folder_ids,
            notify: notify_new && !repairing_incomplete,
            reconciliation: repairing_incomplete,
        });
        Ok(())
    }
}

fn merge_seen_uidls(known_uidls: &HashSet<String>, fetched_uidls: &[String]) -> Vec<String> {
    let mut uidls = known_uidls.clone();
    uidls.extend(fetched_uidls.iter().cloned());
    let mut uidls = uidls.into_iter().collect::<Vec<_>>();
    uidls.sort();
    uidls
}

fn select_pop3_messages_to_fetch(
    messages: &[Pop3MessageRef],
    known_uidls: &HashSet<String>,
    limit: u32,
) -> Vec<Pop3MessageRef> {
    let mut selected = messages
        .iter()
        .filter(|message| !known_uidls.contains(&message.uid))
        .cloned()
        .collect::<Vec<_>>();
    selected.sort_by_key(|message| std::cmp::Reverse(message.number));
    selected.truncate(limit as usize);
    selected.sort_by_key(|message| message.number);
    selected
}

fn collect_message_reference_ids(message: &Message) -> Vec<String> {
    let mut refs = HashSet::new();
    for header in [&message.in_reply_to, &message.references_header]
        .into_iter()
        .flatten()
    {
        for id in header.split_whitespace() {
            let id = id.trim();
            if !id.is_empty() {
                refs.insert(id.to_string());
            }
        }
    }
    refs.into_iter().collect()
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use pebble_core::Message;

    use super::{
        collect_message_reference_ids, merge_seen_uidls, select_pop3_messages_to_fetch,
        Pop3MessageRef,
    };

    #[test]
    fn pop3_fetch_selection_skips_known_uidls_and_keeps_newest_limit() {
        let messages = vec![
            Pop3MessageRef {
                number: 1,
                uid: "old-a".to_string(),
                size: Some(100),
            },
            Pop3MessageRef {
                number: 2,
                uid: "old-b".to_string(),
                size: Some(100),
            },
            Pop3MessageRef {
                number: 3,
                uid: "new-a".to_string(),
                size: Some(100),
            },
            Pop3MessageRef {
                number: 4,
                uid: "new-b".to_string(),
                size: Some(100),
            },
        ];
        let known = HashSet::from(["old-a".to_string()]);

        let selected = select_pop3_messages_to_fetch(&messages, &known, 2);

        assert_eq!(
            selected.into_iter().map(|msg| msg.uid).collect::<Vec<_>>(),
            vec!["new-a".to_string(), "new-b".to_string()]
        );
    }

    #[test]
    fn pop3_seen_uidls_do_not_include_unfetched_remote_messages() {
        let known = HashSet::from(["known-a".to_string()]);
        let fetched = vec!["new-b".to_string()];

        assert_eq!(
            merge_seen_uidls(&known, &fetched),
            vec!["known-a".to_string(), "new-b".to_string()]
        );
    }

    #[test]
    fn pop3_reference_collection_uses_in_reply_to_and_references_header() {
        let mut message = Message {
            id: "message-1".to_string(),
            account_id: "account-1".to_string(),
            remote_id: "uidl-1".to_string(),
            message_id_header: Some("<current@example.com>".to_string()),
            in_reply_to: Some("<parent@example.com>".to_string()),
            references_header: Some("<root@example.com> <parent@example.com>".to_string()),
            thread_id: None,
            subject: "Re: hello".to_string(),
            snippet: String::new(),
            from_address: "a@example.com".to_string(),
            from_name: String::new(),
            to_list: vec![],
            cc_list: vec![],
            bcc_list: vec![],
            body_text: String::new(),
            body_html_raw: String::new(),
            has_attachments: false,
            is_read: false,
            is_starred: false,
            is_draft: false,
            date: 0,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: 0,
            updated_at: 0,
        };

        let refs = collect_message_reference_ids(&message)
            .into_iter()
            .collect::<HashSet<_>>();

        assert_eq!(
            refs,
            HashSet::from([
                "<root@example.com>".to_string(),
                "<parent@example.com>".to_string(),
            ])
        );

        message.in_reply_to = None;
        message.references_header = None;
        assert!(collect_message_reference_ids(&message).is_empty());
    }
}
