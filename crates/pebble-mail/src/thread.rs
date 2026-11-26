use pebble_core::{new_id, Message};
use std::collections::HashMap;

pub use pebble_core::normalize_subject;

/// Namespace used for Gmail's provider-native `threadId`.
pub const GMAIL_THREAD_NAMESPACE: &str = "gmail";
/// Namespace used for Microsoft Graph's `conversationId`.
pub const OUTLOOK_THREAD_NAMESPACE: &str = "outlook";

/// Build a local thread id from a provider-native conversation id. The
/// namespace prefix keeps provider ids from colliding with Message-ID based
/// thread ids (which always look like `<...>`) or generated UUIDs.
pub fn provider_thread_id(namespace: &str, raw: Option<&str>) -> Option<String> {
    let raw = raw?.trim();
    (!raw.is_empty()).then(|| format!("{namespace}:{raw}"))
}

fn header_ids(header: Option<&String>) -> impl Iterator<Item = &str> {
    header
        .into_iter()
        .flat_map(|value| value.split_whitespace())
        .map(str::trim)
        .filter(|id| !id.is_empty())
}

/// Compute the thread ID for a message.
///
/// `existing_threads` maps `message_id_header → thread_id` for messages
/// already in the store. O(1) lookup per reference.
///
/// Logic:
/// 1. Check `in_reply_to` against existing `message_id_header`s.
/// 2. Check each ID in `references_header` (space-separated) against existing.
/// 3. No ancestor is stored yet: use the conversation root — the first
///    `References` entry, else the first `In-Reply-To` entry — so messages
///    of one conversation agree on a thread id regardless of arrival order.
/// 4. Fall back to the message's own `message_id_header`.
/// 5. If none, generate a new UUID.
pub fn compute_thread_id(message: &Message, existing_threads: &HashMap<String, String>) -> String {
    let lookup = |mid: &str| -> Option<String> { existing_threads.get(mid.trim()).cloned() };

    // 1. Check In-Reply-To
    for id in header_ids(message.in_reply_to.as_ref()) {
        if let Some(tid) = lookup(id) {
            return tid;
        }
    }

    // 2. Check References (check last to first for closest ancestor)
    let refs: Vec<&str> = header_ids(message.references_header.as_ref()).collect();
    for id in refs.iter().rev() {
        if let Some(tid) = lookup(id) {
            return tid;
        }
    }

    // 3. Conversation root from the headers
    if let Some(root) = refs
        .first()
        .copied()
        .or_else(|| header_ids(message.in_reply_to.as_ref()).next())
    {
        return root.to_string();
    }

    // 4. Use the message's own message_id_header as thread root
    if let Some(mid) = &message.message_id_header {
        return mid.clone();
    }

    // 5. Generate new UUID
    new_id()
}

/// Thread ids that other messages may have been filed under before
/// `message` arrived, and that therefore belong to `thread_id`.
///
/// When a child arrives before its ancestors it is filed under the root it
/// can see in its own headers (or under its parent's Message-ID when it only
/// carries `In-Reply-To`). Once `message` is stored with `thread_id`, any
/// thread named after `message`'s own Message-ID or one of its ancestors is
/// the same conversation and must be merged into `thread_id`.
pub fn thread_merge_aliases(message: &Message, thread_id: &str) -> Vec<String> {
    merge_aliases_from_headers(
        message.message_id_header.as_ref(),
        message.in_reply_to.as_ref(),
        message.references_header.as_ref(),
        thread_id,
    )
}

fn merge_aliases_from_headers(
    message_id_header: Option<&String>,
    in_reply_to: Option<&String>,
    references_header: Option<&String>,
    thread_id: &str,
) -> Vec<String> {
    let mut aliases: Vec<String> = Vec::new();
    let candidates = message_id_header
        .into_iter()
        .map(|id| id.trim())
        .chain(header_ids(in_reply_to))
        .chain(header_ids(references_header));
    for candidate in candidates {
        if candidate != thread_id && !aliases.iter().any(|alias| alias == candidate) {
            aliases.push(candidate.to_string());
        }
    }
    aliases
}

/// Mirror a thread merge in an in-memory `message_id_header → thread_id`
/// map so later messages of the same batch resolve to the merged thread.
pub fn apply_thread_merge_to_mappings(
    mappings: &mut HashMap<String, String>,
    thread_id: &str,
    aliases: &[String],
) {
    if aliases.is_empty() {
        return;
    }
    for value in mappings.values_mut() {
        if aliases.iter().any(|alias| alias == value) {
            *value = thread_id.to_string();
        }
    }
}

/// One pending re-parent: every message of the account filed under one of
/// `aliases` moves to `thread_id`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadMerge {
    pub thread_id: String,
    pub aliases: Vec<String>,
}

/// Apply thread merges in order inside one write transaction. Returns the
/// ids of the messages whose thread id changed.
pub async fn merge_thread_aliases(
    store: &std::sync::Arc<pebble_store::Store>,
    account_id: &str,
    merges: Vec<ThreadMerge>,
) -> pebble_core::Result<Vec<String>> {
    let merges: Vec<(String, Vec<String>)> = merges
        .into_iter()
        .filter(|merge| !merge.aliases.is_empty())
        .map(|merge| (merge.thread_id, merge.aliases))
        .collect();
    if merges.is_empty() {
        return Ok(Vec::new());
    }
    let store = std::sync::Arc::clone(store);
    let account_id = account_id.to_string();
    tokio::task::spawn_blocking(move || store.merge_thread_aliases(&account_id, &merges))
        .await
        .map_err(|e| pebble_core::PebbleError::Internal(format!("spawn_blocking: {e}")))?
}

/// Re-file stored messages under provider-native thread ids. Each entry is
/// `(local_message_id, thread_id)`. Returns the local ids that changed.
pub async fn assign_thread_ids(
    store: &std::sync::Arc<pebble_store::Store>,
    account_id: &str,
    assignments: Vec<(String, String)>,
) -> pebble_core::Result<Vec<String>> {
    if assignments.is_empty() {
        return Ok(Vec::new());
    }
    let store = std::sync::Arc::clone(store);
    let account_id = account_id.to_string();
    tokio::task::spawn_blocking(move || store.assign_thread_ids(&account_id, &assignments))
        .await
        .map_err(|e| pebble_core::PebbleError::Internal(format!("spawn_blocking: {e}")))?
}

/// Plan a re-merge of conversations stored before the live alias merge
/// existed: every message's thread is unioned with the threads named after
/// its own Message-ID and its ancestors (exactly what the live merge does
/// when a message arrives). Each merged group adopts the thread id of its
/// earliest message. Returns only the `(message_id, thread_id)` changes.
pub fn plan_rethread(rows: &[pebble_store::threads::ThreadHeaderRow]) -> Vec<(String, String)> {
    let mut order: Vec<&pebble_store::threads::ThreadHeaderRow> =
        rows.iter().filter(|row| row.thread_id.is_some()).collect();
    order.sort_by(|a, b| a.date.cmp(&b.date).then_with(|| a.id.cmp(&b.id)));

    // Union-find over thread ids; rank = first-seen position (earliest date).
    let mut index: HashMap<&str, usize> = HashMap::new();
    let mut names: Vec<&str> = Vec::new();
    for row in &order {
        let thread_id = row.thread_id.as_deref().unwrap_or_default();
        if !index.contains_key(thread_id) {
            index.insert(thread_id, names.len());
            names.push(thread_id);
        }
    }
    let mut parent: Vec<usize> = (0..names.len()).collect();
    fn find(parent: &mut [usize], mut x: usize) -> usize {
        while parent[x] != x {
            parent[x] = parent[parent[x]];
            x = parent[x];
        }
        x
    }
    for row in &order {
        let thread_id = row.thread_id.as_deref().unwrap_or_default();
        let aliases = merge_aliases_from_headers(
            row.message_id_header.as_ref(),
            row.in_reply_to.as_ref(),
            row.references_header.as_ref(),
            thread_id,
        );
        for alias in aliases {
            let Some(&alias_idx) = index.get(alias.as_str()) else {
                continue;
            };
            let a = find(&mut parent, index[thread_id]);
            let b = find(&mut parent, alias_idx);
            if a != b {
                // Earlier-seen thread id wins.
                let (root, child) = if a < b { (a, b) } else { (b, a) };
                parent[child] = root;
            }
        }
    }
    let mut changes = Vec::new();
    for row in &order {
        let thread_id = row.thread_id.as_deref().unwrap_or_default();
        let root = find(&mut parent, index[thread_id]);
        if root != index[thread_id] {
            changes.push((row.id.clone(), names[root].to_string()));
        }
    }
    changes
}

/// Re-merge an account's stored conversations (see [`plan_rethread`]) and
/// return the ids of the messages that moved.
pub async fn rethread_account(
    store: &std::sync::Arc<pebble_store::Store>,
    account_id: &str,
) -> pebble_core::Result<Vec<String>> {
    let store = std::sync::Arc::clone(store);
    let account_id = account_id.to_string();
    tokio::task::spawn_blocking(move || {
        let rows = store.list_thread_headers(&account_id)?;
        let changes = plan_rethread(&rows);
        store.assign_thread_ids(&account_id, &changes)
    })
    .await
    .map_err(|e| pebble_core::PebbleError::Internal(format!("spawn_blocking: {e}")))?
}

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_core::{new_id, now_timestamp, EmailAddress, Message};

    fn make_message(
        message_id_header: Option<&str>,
        in_reply_to: Option<&str>,
        references_header: Option<&str>,
    ) -> Message {
        let now = now_timestamp();
        Message {
            id: new_id(),
            account_id: "acct1".to_string(),
            remote_id: "1".to_string(),
            message_id_header: message_id_header.map(|s| s.to_string()),
            in_reply_to: in_reply_to.map(|s| s.to_string()),
            references_header: references_header.map(|s| s.to_string()),
            thread_id: None,
            subject: "Test".to_string(),
            snippet: "".to_string(),
            from_address: "a@b.com".to_string(),
            from_name: "A".to_string(),
            to_list: vec![EmailAddress {
                name: None,
                address: "b@c.com".to_string(),
            }],
            cc_list: vec![],
            bcc_list: vec![],
            body_text: "".to_string(),
            body_html_raw: "".to_string(),
            has_attachments: false,
            is_read: false,
            is_starred: false,
            is_draft: false,
            date: now,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: now,
            updated_at: now,
        }
    }

    #[test]
    fn test_normalize_subject() {
        assert_eq!(normalize_subject("Re: Hello"), "Hello");
        assert_eq!(normalize_subject("re: Hello"), "Hello");
        assert_eq!(normalize_subject("RE: Hello"), "Hello");
        assert_eq!(normalize_subject("Fwd: Hello"), "Hello");
        assert_eq!(normalize_subject("FWD: Hello"), "Hello");
        assert_eq!(normalize_subject("Fw: Hello"), "Hello");
        assert_eq!(normalize_subject("Re: Re: Hello"), "Hello");
        assert_eq!(normalize_subject("Re: Fwd: Hello"), "Hello");
        assert_eq!(normalize_subject("回复: Hello"), "Hello");
        assert_eq!(normalize_subject("转发: Hello"), "Hello");
        assert_eq!(normalize_subject("Hello"), "Hello");
        assert_eq!(normalize_subject("  Hello  "), "Hello");
        // Edge case: multi-byte char whose lowercase form has different byte length
        assert_eq!(normalize_subject("\u{0130}re: Test"), "\u{0130}re: Test");
        assert_eq!(normalize_subject("Re: \u{0130}test"), "\u{0130}test");
    }

    #[test]
    fn test_compute_thread_id_new_thread() {
        let msg = make_message(Some("<new@example.com>"), None, None);
        let existing: HashMap<String, String> = HashMap::new();
        let tid = compute_thread_id(&msg, &existing);
        assert_eq!(tid, "<new@example.com>");
    }

    #[test]
    fn test_compute_thread_id_reply() {
        let existing: HashMap<String, String> = [(
            "<original@example.com>".to_string(),
            "thread-abc".to_string(),
        )]
        .into_iter()
        .collect();
        let msg = make_message(
            Some("<reply@example.com>"),
            Some("<original@example.com>"),
            Some("<original@example.com>"),
        );
        let tid = compute_thread_id(&msg, &existing);
        assert_eq!(tid, "thread-abc");
    }

    #[test]
    fn test_compute_thread_id_via_references() {
        let existing: HashMap<String, String> =
            [("<root@example.com>".to_string(), "thread-xyz".to_string())]
                .into_iter()
                .collect();
        let msg = make_message(
            Some("<reply2@example.com>"),
            Some("<nonexistent@example.com>"),
            Some("<root@example.com> <intermediate@example.com>"),
        );
        let tid = compute_thread_id(&msg, &existing);
        assert_eq!(tid, "thread-xyz");
    }

    #[test]
    fn unknown_parent_files_reply_under_the_references_root() {
        let existing = HashMap::new();
        let child = make_message(
            Some("<child@example.com>"),
            Some("<parent@example.com>"),
            Some("<root@example.com> <parent@example.com>"),
        );
        assert_eq!(compute_thread_id(&child, &existing), "<root@example.com>");

        let in_reply_to_only = make_message(
            Some("<child@example.com>"),
            Some("<parent@example.com>"),
            None,
        );
        assert_eq!(
            compute_thread_id(&in_reply_to_only, &existing),
            "<parent@example.com>"
        );
    }

    #[test]
    fn provider_thread_ids_are_namespaced() {
        assert_eq!(
            provider_thread_id(GMAIL_THREAD_NAMESPACE, Some("18c2")).as_deref(),
            Some("gmail:18c2")
        );
        assert_eq!(
            provider_thread_id(OUTLOOK_THREAD_NAMESPACE, Some("  ")),
            None
        );
        assert_eq!(provider_thread_id(OUTLOOK_THREAD_NAMESPACE, None), None);
    }

    #[test]
    fn merge_aliases_exclude_the_target_and_duplicates() {
        let message = make_message(
            Some("<p@example.com>"),
            Some("<r@example.com>"),
            Some("<r@example.com>"),
        );
        assert_eq!(
            thread_merge_aliases(&message, "<r@example.com>"),
            vec!["<p@example.com>".to_string()]
        );

        let mut mappings: HashMap<String, String> = [
            ("<c@example.com>".to_string(), "<p@example.com>".to_string()),
            ("<x@example.com>".to_string(), "other".to_string()),
        ]
        .into_iter()
        .collect();
        apply_thread_merge_to_mappings(
            &mut mappings,
            "<r@example.com>",
            &["<p@example.com>".to_string()],
        );
        assert_eq!(mappings["<c@example.com>"], "<r@example.com>");
        assert_eq!(mappings["<x@example.com>"], "other");
    }

    fn thread_test_store() -> (std::sync::Arc<pebble_store::Store>, String) {
        let store = std::sync::Arc::new(pebble_store::Store::open_in_memory().unwrap());
        let now = now_timestamp();
        store
            .insert_account(&pebble_core::Account {
                id: "acct1".to_string(),
                email: "me@example.com".to_string(),
                display_name: "Me".to_string(),
                account_label: None,
                provider_display_name: None,
                color: None,
                provider: pebble_core::ProviderType::Imap,
                created_at: now,
                updated_at: now,
            })
            .unwrap();
        let folder = pebble_core::Folder {
            id: "inbox".to_string(),
            account_id: "acct1".to_string(),
            remote_id: "INBOX".to_string(),
            name: "Inbox".to_string(),
            folder_type: pebble_core::FolderType::Folder,
            role: Some(pebble_core::FolderRole::Inbox),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 0,
        };
        store.insert_folder(&folder).unwrap();
        (store, folder.id)
    }

    /// Mirror the IMAP/POP3 store path: resolve the thread from stored
    /// ancestors, store, then merge any thread filed under an alias.
    async fn arrive(
        store: &std::sync::Arc<pebble_store::Store>,
        folder_id: &str,
        mut message: Message,
    ) -> String {
        let refs: Vec<String> = header_ids(message.in_reply_to.as_ref())
            .chain(header_ids(message.references_header.as_ref()))
            .map(str::to_string)
            .collect();
        let mappings = store.get_thread_mappings_for_refs("acct1", &refs).unwrap();
        let thread_id = compute_thread_id(&message, &mappings);
        message.thread_id = Some(thread_id.clone());
        message.remote_id = message.id.clone();
        store
            .insert_message(&message, &[folder_id.to_string()])
            .unwrap();
        let aliases = thread_merge_aliases(&message, &thread_id);
        merge_thread_aliases(store, "acct1", vec![ThreadMerge { thread_id, aliases }])
            .await
            .unwrap();
        message.id
    }

    fn stored_thread(store: &pebble_store::Store, id: &str) -> String {
        store.get_message(id).unwrap().unwrap().thread_id.unwrap()
    }

    #[tokio::test]
    async fn out_of_order_arrival_converges_on_one_thread() {
        let (store, folder_id) = thread_test_store();
        // Grandchild arrives first, carrying only In-Reply-To.
        let grandchild = arrive(
            &store,
            &folder_id,
            make_message(Some("<c@example.com>"), Some("<p@example.com>"), None),
        )
        .await;
        // Unrelated conversation must stay separate.
        let unrelated = arrive(
            &store,
            &folder_id,
            make_message(Some("<u@example.com>"), None, None),
        )
        .await;
        // Parent arrives next, referencing the (still missing) root.
        let parent = arrive(
            &store,
            &folder_id,
            make_message(
                Some("<p@example.com>"),
                Some("<r@example.com>"),
                Some("<r@example.com>"),
            ),
        )
        .await;
        // Root arrives last.
        let root = arrive(
            &store,
            &folder_id,
            make_message(Some("<r@example.com>"), None, None),
        )
        .await;

        let thread = stored_thread(&store, &root);
        assert_eq!(stored_thread(&store, &parent), thread);
        assert_eq!(stored_thread(&store, &grandchild), thread);
        assert_ne!(stored_thread(&store, &unrelated), thread);
    }

    #[tokio::test]
    async fn merges_are_scoped_to_the_account() {
        let (store, folder_id) = thread_test_store();
        let now = now_timestamp();
        store
            .insert_account(&pebble_core::Account {
                id: "acct2".to_string(),
                email: "other@example.com".to_string(),
                display_name: "Other".to_string(),
                account_label: None,
                provider_display_name: None,
                color: None,
                provider: pebble_core::ProviderType::Imap,
                created_at: now,
                updated_at: now,
            })
            .unwrap();
        let mut foreign = make_message(Some("<c2@example.com>"), Some("<p@example.com>"), None);
        foreign.account_id = "acct2".to_string();
        foreign.remote_id = foreign.id.clone();
        foreign.thread_id = Some("<p@example.com>".to_string());
        store.insert_message(&foreign, &[]).unwrap();

        arrive(
            &store,
            &folder_id,
            make_message(
                Some("<p@example.com>"),
                Some("<r@example.com>"),
                Some("<r@example.com>"),
            ),
        )
        .await;

        assert_eq!(stored_thread(&store, &foreign.id), "<p@example.com>");
    }

    #[tokio::test]
    async fn provider_thread_assignment_only_reports_changes() {
        let (store, folder_id) = thread_test_store();
        let id = arrive(
            &store,
            &folder_id,
            make_message(Some("<g@example.com>"), None, None),
        )
        .await;
        let changed =
            assign_thread_ids(&store, "acct1", vec![(id.clone(), "gmail:t1".to_string())])
                .await
                .unwrap();
        assert_eq!(changed, vec![id.clone()]);
        assert_eq!(stored_thread(&store, &id), "gmail:t1");
        let unchanged = assign_thread_ids(&store, "acct1", vec![(id, "gmail:t1".to_string())])
            .await
            .unwrap();
        assert!(unchanged.is_empty());
    }

    /// Store a message with the thread id the pre-merge code would have given
    /// it (compute only, no alias merge).
    fn store_without_merge(
        store: &pebble_store::Store,
        folder_id: &str,
        mut message: Message,
        date: i64,
    ) -> String {
        let refs: Vec<String> = header_ids(message.in_reply_to.as_ref())
            .chain(header_ids(message.references_header.as_ref()))
            .map(str::to_string)
            .collect();
        let mappings = store.get_thread_mappings_for_refs("acct1", &refs).unwrap();
        message.thread_id = Some(compute_thread_id(&message, &mappings));
        message.remote_id = message.id.clone();
        message.date = date;
        store
            .insert_message(&message, &[folder_id.to_string()])
            .unwrap();
        message.id
    }

    #[tokio::test]
    async fn rethread_pass_merges_legacy_split_conversations_once() {
        let (store, folder_id) = thread_test_store();
        // Out-of-order arrival without the live merge leaves three threads.
        let grandchild = store_without_merge(
            &store,
            &folder_id,
            make_message(Some("<c@example.com>"), Some("<p@example.com>"), None),
            300,
        );
        let parent = store_without_merge(
            &store,
            &folder_id,
            make_message(
                Some("<p@example.com>"),
                Some("<r@example.com>"),
                Some("<r@example.com>"),
            ),
            200,
        );
        let unrelated = store_without_merge(
            &store,
            &folder_id,
            make_message(Some("<u@example.com>"), None, None),
            250,
        );
        let root = store_without_merge(
            &store,
            &folder_id,
            make_message(Some("<r@example.com>"), None, None),
            100,
        );
        assert_ne!(
            stored_thread(&store, &grandchild),
            stored_thread(&store, &root)
        );

        let mut changed = rethread_account(&store, "acct1").await.unwrap();
        changed.sort();
        let thread = stored_thread(&store, &root);
        assert_eq!(stored_thread(&store, &parent), thread);
        assert_eq!(stored_thread(&store, &grandchild), thread);
        assert_eq!(stored_thread(&store, &unrelated), "<u@example.com>");
        assert!(changed.contains(&grandchild));
        assert!(!changed.contains(&unrelated));

        // Idempotent: a second pass changes nothing.
        assert!(rethread_account(&store, "acct1").await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn live_merge_reports_the_messages_it_moved() {
        let (store, folder_id) = thread_test_store();
        let child = store_without_merge(
            &store,
            &folder_id,
            make_message(Some("<c@example.com>"), Some("<p@example.com>"), None),
            200,
        );
        let mut parent = make_message(Some("<p@example.com>"), None, None);
        parent.thread_id = Some("<p-thread>".to_string());
        let aliases = thread_merge_aliases(&parent, "<p-thread>");
        let moved = merge_thread_aliases(
            &store,
            "acct1",
            vec![ThreadMerge {
                thread_id: "<p-thread>".to_string(),
                aliases,
            }],
        )
        .await
        .unwrap();
        assert_eq!(moved, vec![child.clone()]);
        assert_eq!(stored_thread(&store, &child), "<p-thread>");
    }

    #[tokio::test]
    async fn worker_rethread_pass_runs_once_and_announces_refresh() {
        let (store, folder_id) = thread_test_store();
        let child = store_without_merge(
            &store,
            &folder_id,
            make_message(Some("<c@example.com>"), Some("<p@example.com>"), None),
            200,
        );
        store_without_merge(
            &store,
            &folder_id,
            make_message(
                Some("<p@example.com>"),
                Some("<r@example.com>"),
                Some("<r@example.com>"),
            ),
            100,
        );
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
        let base = crate::sync::SyncWorkerBase {
            account_id: "acct1".to_string(),
            store: std::sync::Arc::clone(&store),
            attachments_dir: std::env::temp_dir(),
            error_tx: None,
            message_tx: None,
            runtime_status_tx: None,
            progress_tx: None,
            thread_refresh_tx: Some(tx),
        };

        base.rethread_legacy_threads_once().await;
        let refresh = rx.try_recv().expect("re-thread must announce a refresh");
        assert_eq!(refresh.account_id, "acct1");
        assert_eq!(refresh.folder_ids, vec![folder_id.clone()]);
        assert_eq!(refresh.message_ids, vec![child.clone()]);
        assert_eq!(
            store
                .get_sync_state("acct1")
                .unwrap()
                .unwrap()
                .rethread_version,
            1
        );

        // Tracked per account: the second start does nothing.
        base.rethread_legacy_threads_once().await;
        assert!(rx.try_recv().is_err());
    }
}
