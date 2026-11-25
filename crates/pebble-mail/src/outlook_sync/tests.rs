use super::*;
use crate::parser::{AttachmentData, AttachmentMeta};
use pebble_core::{new_id, Account, Attachment, Folder, FolderRole, FolderType, ProviderType};
use std::collections::VecDeque;

#[test]
fn outlook_delta_cursor_key_is_folder_scoped() {
    assert_eq!(
        outlook_delta_cursor_key("folder-1"),
        "outlook_delta:folder-1"
    );
}

#[test]
fn outlook_delta_cursor_advances_only_without_failures() {
    assert!(can_advance_outlook_delta_cursor(0));
    assert!(!can_advance_outlook_delta_cursor(1));
}

#[test]
fn outlook_range_scope_mismatch_bootstraps_without_reusing_cursor() {
    let cursor = serialize_outlook_delta_cursor_with_scope("inbox", "opaque", Some(86_400));
    assert!(outlook_delta_scope_matches(Some(&cursor), Some(86_400)));
    assert!(!outlook_delta_scope_matches(Some(&cursor), Some(0)));
    assert!(!outlook_delta_scope_matches(
        Some("https://graph.microsoft.com/v1.0/me/messages/delta?$deltatoken=old"),
        Some(86_400)
    ));
}

fn make_message(remote_id: &str) -> Message {
    let now = now_timestamp();
    Message {
        id: format!("local-{remote_id}"),
        account_id: "account-1".to_string(),
        remote_id: remote_id.to_string(),
        message_id_header: None,
        in_reply_to: None,
        references_header: None,
        thread_id: None,
        subject: remote_id.to_string(),
        snippet: String::new(),
        from_address: String::new(),
        from_name: String::new(),
        to_list: Vec::new(),
        cc_list: Vec::new(),
        bcc_list: Vec::new(),
        body_text: String::new(),
        body_html_raw: String::new(),
        has_attachments: false,
        is_read: true,
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
fn expired_outlook_snapshot_does_not_prune_cached_mail_before_cutoff() {
    let store = Store::open_in_memory().unwrap();
    let account = make_account();
    let folder = make_folder("inbox");
    let cutoff = 1_704_067_200;
    let mut old = make_message("old");
    old.date = cutoff - 1;
    let mut recent = make_message("recent");
    recent.date = cutoff + 1;
    store.insert_account(&account).unwrap();
    store.insert_folder(&folder).unwrap();
    store
        .insert_message(&old, std::slice::from_ref(&folder.id))
        .unwrap();
    store
        .insert_message(&recent, std::slice::from_ref(&folder.id))
        .unwrap();

    let outcome = reconcile_outlook_fresh_snapshot_since(
        &store,
        &folder.id,
        &[old.clone(), recent.clone()],
        &std::collections::HashSet::new(),
        Some(cutoff),
    );

    assert_eq!(outcome.failure_count, 0);
    assert!(!store.get_message(&old.id).unwrap().unwrap().is_deleted);
    assert!(store.get_message(&recent.id).unwrap().unwrap().is_deleted);
}

fn make_account() -> Account {
    Account {
        account_label: None,
        provider_display_name: None,
        id: "account-1".to_string(),
        email: "user@example.com".to_string(),
        display_name: "User".to_string(),
        color: None,
        provider: ProviderType::Outlook,
        created_at: now_timestamp(),
        updated_at: now_timestamp(),
    }
}

fn make_folder(remote_id: &str) -> Folder {
    Folder {
        id: format!("folder-{remote_id}"),
        account_id: "account-1".to_string(),
        remote_id: remote_id.to_string(),
        name: remote_id.to_string(),
        folder_type: FolderType::Folder,
        role: Some(FolderRole::Inbox),
        parent_id: None,
        color: None,
        is_system: true,
        sort_order: 0,
    }
}

fn make_attachment_data(filename: &str, data: &[u8]) -> AttachmentData {
    AttachmentData {
        meta: AttachmentMeta {
            filename: filename.to_string(),
            mime_type: "text/plain".to_string(),
            size: data.len(),
            content_id: None,
            is_inline: false,
        },
        data: data.to_vec(),
    }
}

#[test]
fn outlook_sync_skips_hidden_service_folders_from_existing_store_rows() {
    let mut conversation_history = make_folder("conversation-history-id");
    conversation_history.name = "对话历史记录".to_string();
    conversation_history.role = None;

    let mut remote_outbox = make_folder("remote-outbox-id");
    remote_outbox.name = "发件箱".to_string();
    remote_outbox.role = None;

    let mut local_outbox = make_folder("__local_outbox__");
    local_outbox.name = "Outbox".to_string();
    local_outbox.role = None;

    assert!(!should_sync_outlook_folder(&conversation_history));
    assert!(!should_sync_outlook_folder(&remote_outbox));
    assert!(should_sync_outlook_folder(&local_outbox));
    assert!(should_sync_outlook_folder(&make_folder("inbox-id")));
}

#[test]
fn unbounded_outlook_sync_does_not_load_cached_remote_ids() {
    let mut load_count = 0;
    let cached = load_cached_outlook_ids(None, || {
        load_count += 1;
        Ok(std::collections::HashSet::from(["cached".to_string()]))
    })
    .unwrap();

    assert!(cached.is_empty());
    assert_eq!(load_count, 0);

    let bounded = load_cached_outlook_ids(Some(1), || {
        load_count += 1;
        Ok(std::collections::HashSet::from(["cached".to_string()]))
    })
    .unwrap();
    assert!(bounded.contains("cached"));
    assert_eq!(load_count, 1);
}

#[tokio::test]
async fn collect_outlook_delta_pages_follows_next_link_until_delta_link() {
    let mut pages = VecDeque::from([
            OutlookDeltaPage {
                messages: vec![make_message("outlook-1")],
                deleted_remote_ids: vec!["deleted-1".to_string()],
                next_link: Some(
                    "https://graph.microsoft.com/v1.0/me/mailFolders/folder-1/messages/delta?$skiptoken=next"
                        .to_string(),
                ),
                delta_link: None,
            },
            OutlookDeltaPage {
                messages: vec![make_message("outlook-2")],
                deleted_remote_ids: vec!["deleted-2".to_string()],
                next_link: None,
                delta_link: Some(
                    "https://graph.microsoft.com/v1.0/me/mailFolders/folder-1/messages/delta?$deltatoken=done"
                        .to_string(),
                ),
            },
        ]);
    let mut requested = Vec::new();

    let batch =
            collect_outlook_delta_pages(
                "folder-1",
                Some("https://graph.microsoft.com/v1.0/me/mailFolders/folder-1/messages/delta?$deltatoken=cursor-0"),
                |folder_id, cursor| {
                requested.push((folder_id, cursor));
                let page = pages.pop_front().expect("expected a delta page request");
                async move { Ok(page) }
            },
            )
            .await
            .unwrap();

    let remote_ids: Vec<_> = batch.messages.into_iter().map(|m| m.remote_id).collect();
    assert_eq!(
        remote_ids,
        vec!["outlook-1".to_string(), "outlook-2".to_string()]
    );
    assert_eq!(
        batch.deleted_remote_ids,
        vec!["deleted-1".to_string(), "deleted-2".to_string()]
    );
    assert_eq!(
            batch.delta_link.as_deref(),
            Some("https://graph.microsoft.com/v1.0/me/mailFolders/folder-1/messages/delta?$deltatoken=done")
        );
    assert_eq!(
            requested,
            vec![
                (
                    "folder-1".to_string(),
                    Some("https://graph.microsoft.com/v1.0/me/mailFolders/folder-1/messages/delta?$deltatoken=cursor-0".to_string())
                ),
                (
                    "folder-1".to_string(),
                    Some("https://graph.microsoft.com/v1.0/me/mailFolders/folder-1/messages/delta?$skiptoken=next".to_string())
                ),
            ]
        );
}

#[tokio::test]
async fn outlook_pagination_reuses_shared_cached_remote_ids() {
    let cached = Arc::new(std::collections::HashSet::from(["cached".to_string()]));
    let mut allocations = Vec::new();
    let mut page_number = 0;

    collect_outlook_delta_pages("inbox", None, |_, cursor| {
        allocations.push(Arc::as_ptr(&cached));
        let cached = Arc::clone(&cached);
        page_number += 1;
        async move {
            assert!(cached.contains("cached"));
            if cursor.is_none() {
                Ok(OutlookDeltaPage {
                    messages: Vec::new(),
                    deleted_remote_ids: Vec::new(),
                    next_link: Some(
                        "https://graph.microsoft.com/v1.0/me/messages/delta?$skiptoken=page-2"
                            .to_string(),
                    ),
                    delta_link: None,
                })
            } else {
                Ok(OutlookDeltaPage {
                    messages: Vec::new(),
                    deleted_remote_ids: Vec::new(),
                    next_link: None,
                    delta_link: Some(
                        "https://graph.microsoft.com/v1.0/me/messages/delta?$deltatoken=done"
                            .to_string(),
                    ),
                })
            }
        }
    })
    .await
    .unwrap();

    assert_eq!(page_number, 2);
    assert_eq!(allocations.len(), 2);
    assert_eq!(allocations[0], allocations[1]);
}

#[tokio::test]
async fn collect_outlook_delta_pages_rejects_untrusted_stored_cursor_before_fetch() {
    let mut fetch_count = 0;

    let error = collect_outlook_delta_pages(
        "folder-1",
        Some("https://evil.example/v1.0/me/messages/delta?$deltatoken=secret"),
        |_, _| {
            fetch_count += 1;
            async { unreachable!("untrusted cursor must be rejected before a credentialed GET") }
        },
    )
    .await
    .unwrap_err();

    assert!(error.to_string().contains("untrusted Graph continuation"));
    assert_eq!(fetch_count, 0);
}

#[tokio::test]
async fn collect_outlook_delta_pages_rejects_malicious_second_page_before_fetch() {
    let mut fetch_count = 0;

    let error = collect_outlook_delta_pages("folder-1", None, |_, _| {
        fetch_count += 1;
        async {
            Ok(OutlookDeltaPage {
                messages: Vec::new(),
                deleted_remote_ids: Vec::new(),
                next_link: Some(
                    "https://graph.microsoft.com.evil.example/v1.0/me/messages/delta".to_string(),
                ),
                delta_link: None,
            })
        }
    })
    .await
    .unwrap_err();

    assert!(error.to_string().contains("untrusted Graph continuation"));
    assert_eq!(fetch_count, 1);
}

#[tokio::test]
async fn collect_outlook_delta_pages_rejects_repeated_next_link() {
    let repeated =
        "https://graph.microsoft.com/v1.0/me/messages/delta?$skiptoken=repeated".to_string();
    let mut fetch_count = 0;

    let error = collect_outlook_delta_pages("folder-1", None, |_, _| {
        fetch_count += 1;
        let repeated = repeated.clone();
        async move {
            Ok(OutlookDeltaPage {
                messages: Vec::new(),
                deleted_remote_ids: Vec::new(),
                next_link: Some(repeated),
                delta_link: None,
            })
        }
    })
    .await
    .unwrap_err();

    assert!(error.to_string().contains("repeated"));
    assert_eq!(fetch_count, 2);
}

#[tokio::test]
async fn collect_outlook_delta_pages_caps_infinite_unique_continuations() {
    let mut fetch_count = 0usize;

    let error = collect_outlook_delta_pages("folder-1", None, |_, _| {
        fetch_count += 1;
        let next =
            format!("https://graph.microsoft.com/v1.0/me/messages/delta?$skiptoken={fetch_count}");
        async move {
            Ok(OutlookDeltaPage {
                messages: Vec::new(),
                deleted_remote_ids: Vec::new(),
                next_link: Some(next),
                delta_link: None,
            })
        }
    })
    .await
    .unwrap_err();

    assert!(error.to_string().contains("page limit"));
    assert_eq!(fetch_count, MAX_GRAPH_CONTINUATION_PAGES);
}

#[tokio::test]
async fn expired_outlook_cursor_restarts_fresh_pagination_and_reconciles_ghosts() {
    let stored_cursor = "https://graph.microsoft.com/v1.0/me/messages/delta?$deltatoken=expired";
    let next_link = "https://graph.microsoft.com/v1.0/me/messages/delta?$skiptoken=fresh-2";
    let delta_link = "https://graph.microsoft.com/v1.0/me/messages/delta?$deltatoken=fresh-done";
    let mut calls = 0;
    let recovered = collect_outlook_delta_pages_with_expiry_recovery(
        "inbox",
        Some(stored_cursor),
        |_, cursor| {
            calls += 1;
            async move {
                match calls {
                    1 => Err(PebbleError::SyncCursorExpired("expired".to_string())),
                    2 => {
                        assert!(cursor.is_none());
                        Ok(OutlookDeltaPage {
                            messages: vec![make_message("current-1")],
                            deleted_remote_ids: Vec::new(),
                            next_link: Some(next_link.to_string()),
                            delta_link: None,
                        })
                    }
                    3 => Ok(OutlookDeltaPage {
                        messages: vec![make_message("current-2")],
                        deleted_remote_ids: Vec::new(),
                        next_link: None,
                        delta_link: Some(delta_link.to_string()),
                    }),
                    _ => unreachable!(),
                }
            }
        },
    )
    .await
    .unwrap();
    assert!(recovered.recovered_from_expired_cursor);
    assert_eq!(recovered.batch.messages.len(), 2);

    let store = Store::open_in_memory().unwrap();
    let account = make_account();
    let folder = make_folder("inbox");
    let current = make_message("current-1");
    let ghost = make_message("ghost");
    store.insert_account(&account).unwrap();
    store.insert_folder(&folder).unwrap();
    store
        .insert_message(&current, std::slice::from_ref(&folder.id))
        .unwrap();
    store
        .insert_message(&ghost, std::slice::from_ref(&folder.id))
        .unwrap();
    let snapshot = load_outlook_folder_snapshot(&store, &folder.id).unwrap();
    let fresh_remote_ids = recovered
        .batch
        .messages
        .iter()
        .map(|message| message.remote_id.clone())
        .collect::<std::collections::HashSet<_>>();

    let outcome =
        reconcile_outlook_fresh_snapshot(&store, &folder.id, &snapshot, &fresh_remote_ids);
    assert_eq!(outcome.failure_count, 0);
    assert!(store.get_message(&ghost.id).unwrap().unwrap().is_deleted);
    assert!(!store.get_message(&current.id).unwrap().unwrap().is_deleted);
    assert_eq!(recovered.batch.delta_link.as_deref(), Some(delta_link));
    assert!(can_advance_outlook_delta_cursor(outcome.failure_count));
}

#[tokio::test]
async fn failed_fresh_page_after_cursor_expiry_keeps_snapshot_retryable() {
    let stored_cursor = "https://graph.microsoft.com/v1.0/me/messages/delta?$deltatoken=expired";
    let next_link = "https://graph.microsoft.com/v1.0/me/messages/delta?$skiptoken=fresh-2";
    let mut calls = 0;

    let error =
        collect_outlook_delta_pages_with_expiry_recovery("inbox", Some(stored_cursor), |_, _| {
            calls += 1;
            async move {
                match calls {
                    1 => Err(PebbleError::SyncCursorExpired("expired".to_string())),
                    2 => Ok(OutlookDeltaPage {
                        messages: vec![make_message("partial")],
                        deleted_remote_ids: Vec::new(),
                        next_link: Some(next_link.to_string()),
                        delta_link: None,
                    }),
                    3 => Err(PebbleError::Network("fresh page failed".to_string())),
                    _ => unreachable!(),
                }
            }
        })
        .await
        .unwrap_err();

    assert!(error.to_string().contains("fresh page failed"));
}

#[test]
fn outlook_tombstones_soft_delete_existing_messages() {
    let store = Store::open_in_memory().unwrap();
    let account = make_account();
    let folder = make_folder("inbox");
    let message = make_message("deleted-1");

    store.insert_account(&account).unwrap();
    store.insert_folder(&folder).unwrap();
    store
        .insert_message(&message, std::slice::from_ref(&folder.id))
        .unwrap();

    let outcome = apply_outlook_deleted_remote_ids(
        &store,
        "account-1",
        &["deleted-1".to_string(), "missing".to_string()],
    )
    .unwrap();

    assert_eq!(outcome.deleted_count, 1);
    assert_eq!(outcome.failure_count, 0);
    assert!(store.get_message(&message.id).unwrap().unwrap().is_deleted);
}

#[test]
fn outlook_tombstones_continue_after_per_message_delete_failure() {
    let existing = [
        ("deleted-1".to_string(), "message-1".to_string()),
        ("deleted-2".to_string(), "message-2".to_string()),
    ]
    .into_iter()
    .collect();
    let mut attempted = Vec::new();

    let outcome = apply_outlook_deleted_remote_ids_from_existing(
        &existing,
        &["deleted-1".to_string(), "deleted-2".to_string()],
        |message_id: &str| {
            attempted.push(message_id.to_string());
            if message_id == "message-1" {
                Err(PebbleError::Storage("delete failed".to_string()))
            } else {
                Ok(())
            }
        },
    );

    assert_eq!(
        attempted,
        vec!["message-1".to_string(), "message-2".to_string()]
    );
    assert_eq!(outcome.deleted_count, 1);
    assert_eq!(outcome.failure_count, 1);
}

#[test]
fn outlook_backoff_records_failure_after_folder_sync_failures() {
    let mut backoff = SyncBackoff::new();

    update_outlook_backoff_after_sync(&mut backoff, 2);
    assert_eq!(backoff.failure_count(), 1);

    update_outlook_backoff_after_sync(&mut backoff, 0);
    assert_eq!(backoff.failure_count(), 0);
}

#[test]
fn outlook_pager_stops_at_the_page_budget_without_error() {
    let mut pager = OutlookDeltaPager::new(None, 2);
    assert_eq!(pager.next_request().unwrap(), Some(None));
    let next = "https://graph.microsoft.com/v1.0/me/messages/delta?$skiptoken=2".to_string();
    assert!(matches!(
        pager.advance(Some(next.clone()), None).unwrap(),
        OutlookPageStep::Next(link) if link == next
    ));
    assert_eq!(pager.next_request().unwrap(), Some(Some(next)));
    pager
        .advance(
            Some("https://graph.microsoft.com/v1.0/me/messages/delta?$skiptoken=3".into()),
            None,
        )
        .unwrap();
    assert_eq!(pager.next_request().unwrap(), None);
}

#[test]
fn outlook_resume_state_round_trips_next_to_the_delta_cursor() {
    let delta = "https://graph.microsoft.com/v1.0/me/messages/delta?$deltatoken=old";
    let resume = OutlookResumeState {
        next_link: "https://graph.microsoft.com/v1.0/me/messages/delta?$skiptoken=p7".to_string(),
        recovering: true,
    };
    let state = serialize_outlook_folder_state("inbox", Some(delta), Some(&resume), Some(86_400));
    assert_eq!(parse_outlook_resume_state(Some(&state)), Some(resume));
    assert_eq!(
        parse_outlook_delta_cursor("inbox", Some(&state)).as_deref(),
        Some(delta)
    );
    assert!(outlook_delta_scope_matches(Some(&state), Some(86_400)));
    assert!(!outlook_delta_scope_matches(Some(&state), None));

    // Finishing the walk writes a plain delta cursor without resume data.
    let done = serialize_outlook_delta_cursor_with_scope("inbox", delta, None);
    assert_eq!(parse_outlook_resume_state(Some(&done)), None);
    let fresh = serialize_outlook_folder_state("inbox", None, None, None);
    assert_eq!(parse_outlook_delta_cursor("inbox", Some(&fresh)), None);
}

#[tokio::test]
async fn outlook_delta_keeps_queued_local_flags_and_ignores_tombstones() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let account = make_account();
    let folder = make_folder("inbox");
    let archive = make_folder("archive");
    store.insert_account(&account).unwrap();
    store.insert_folder(&folder).unwrap();
    store.insert_folder(&archive).unwrap();
    let mut local = make_message("outlook-pending");
    local.is_read = true;
    store
        .insert_message(&local, std::slice::from_ref(&archive.id))
        .unwrap();
    store
        .insert_pending_mail_op(&local.account_id, &local.id, "move", "{}")
        .unwrap();

    let mut server_view = make_message("outlook-pending");
    server_view.is_read = false;
    server_view.subject = "Server subject".to_string();
    let worker = OutlookSyncWorker::new(
        "account-1",
        Arc::new(OutlookProvider::new("token".into(), "account-1".into())),
        Arc::clone(&store),
        std::env::temp_dir(),
    );
    let failures = worker
        .persist_folder_messages(&folder, vec![server_view], false)
        .await;
    assert_eq!(failures, 0);
    let stored = store.get_message(&local.id).unwrap().unwrap();
    assert!(stored.is_read);
    assert_eq!(stored.subject, "Server subject");
    assert_eq!(
        store.get_message_folder_ids(&local.id).unwrap(),
        vec![archive.id.clone()]
    );

    let outcome = apply_outlook_deleted_remote_ids(
        &store,
        &local.account_id,
        &["outlook-pending".to_string()],
    )
    .unwrap();
    assert_eq!(outcome.deleted_count, 0);
    assert!(!store.get_message(&local.id).unwrap().unwrap().is_deleted);
}

#[tokio::test]
async fn outlook_delta_updates_flags_for_existing_messages() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let account = make_account();
    let folder = make_folder("inbox");
    let mut existing = make_message("outlook-1");
    existing.is_read = false;
    existing.is_starred = false;

    store.insert_account(&account).unwrap();
    store.insert_folder(&folder).unwrap();
    store
        .insert_message(&existing, std::slice::from_ref(&folder.id))
        .unwrap();

    let mut changed = make_message("outlook-1");
    changed.id = "newly-generated-delta-id".to_string();
    changed.is_read = true;
    changed.is_starred = true;
    changed.subject = "Updated subject".to_string();
    changed.body_text = "Updated body".to_string();
    let worker = OutlookSyncWorker::new(
        "account-1",
        Arc::new(OutlookProvider::new(
            "token".to_string(),
            "account-1".to_string(),
        )),
        Arc::clone(&store),
        std::env::temp_dir(),
    );

    let failures = worker
        .persist_folder_messages(&folder, vec![changed], false)
        .await;

    let stored = store.get_message(&existing.id).unwrap().unwrap();
    assert_eq!(failures, 0);
    assert!(stored.is_read);
    assert!(stored.is_starred);
    assert_eq!(stored.subject, "Updated subject");
    assert_eq!(stored.body_text, "Updated body");
}

fn complete_graph_message(body_type: &str) -> serde_json::Value {
    serde_json::json!({
        "id": "outlook-partial-delta",
        "subject": "Keep this subject",
        "bodyPreview": "Keep this preview",
        "body": { "contentType": body_type, "content": "Keep this body" },
        "from": { "emailAddress": { "name": "Sender", "address": "sender@example.com" } },
        "toRecipients": [{ "emailAddress": { "name": null, "address": "user@example.com" } }],
        "ccRecipients": [{ "emailAddress": { "name": null, "address": "cc@example.com" } }],
        "isRead": false,
        "flag": { "flagStatus": "flagged" },
        "isDraft": false,
        "receivedDateTime": "2026-08-24T10:00:00Z",
        "internetMessageId": "<original@example.com>",
        "conversationId": "original-conversation",
        "hasAttachments": true,
        "categories": []
    })
}

#[tokio::test]
async fn outlook_partial_delta_preserves_stored_content_metadata_and_attachments() {
    use crate::provider::outlook::resolve_outlook_delta_page;

    for body_type in ["text", "html"] {
        let store = Arc::new(Store::open_in_memory().unwrap());
        let folder = make_folder("inbox");
        store.insert_account(&make_account()).unwrap();
        store.insert_folder(&folder).unwrap();
        let attachments_dir =
            std::env::temp_dir().join(format!("pebble-outlook-partial-delta-{}", new_id()));
        let (message_tx, mut message_rx) = mpsc::unbounded_channel();
        let worker = OutlookSyncWorker::new(
            "account-1",
            Arc::new(OutlookProvider::new("token".into(), "account-1".into())),
            Arc::clone(&store),
            attachments_dir.clone(),
        )
        .with_message_tx(message_tx);

        let original_json = complete_graph_message(body_type);
        let initial_page = resolve_outlook_delta_page(
            serde_json::from_value(serde_json::json!({ "value": [original_json] })).unwrap(),
            "account-1",
            |_| async { panic!("complete message must not be fetched again") },
        )
        .await
        .unwrap();
        let mut original = initial_page.messages.into_iter().next().unwrap();
        original.created_at = 123;
        let failures = worker
            .persist_folder_messages_with_attachment_fetch(
                &folder,
                vec![original.clone()],
                false,
                |_| async {
                    Ok(vec![make_attachment_data(
                        "keep.txt",
                        b"attachment contents",
                    )])
                },
            )
            .await;
        assert_eq!(failures, 0);
        message_rx.try_recv().unwrap();
        let original = store.get_message(&original.id).unwrap().unwrap();

        // A read change omits the star; a star change omits the read state.
        let mut fetched = complete_graph_message(body_type);
        let partial = if body_type == "text" {
            fetched["isRead"] = serde_json::json!(true);
            serde_json::json!({ "id": original.remote_id, "isRead": true })
        } else {
            fetched["flag"]["flagStatus"] = serde_json::json!("notFlagged");
            serde_json::json!({ "id": original.remote_id, "flag": { "flagStatus": "notFlagged" } })
        };
        let page = resolve_outlook_delta_page(
            serde_json::from_value(serde_json::json!({ "value": [partial] })).unwrap(),
            "account-1",
            |remote_id| {
                assert_eq!(remote_id, original.remote_id);
                let fetched = fetched.clone();
                async move { Ok(Some(fetched)) }
            },
        )
        .await
        .unwrap();
        let failures = worker
            .persist_folder_messages_with_attachment_fetch(
                &folder,
                page.messages,
                true,
                |_| async {
                    Ok(vec![make_attachment_data(
                        "keep.txt",
                        b"attachment contents",
                    )])
                },
            )
            .await;
        assert_eq!(failures, 0);

        let stored = store.get_message(&original.id).unwrap().unwrap();
        assert_eq!(stored.subject, original.subject);
        assert_eq!(stored.snippet, original.snippet);
        assert_eq!(stored.body_text, original.body_text);
        assert_eq!(stored.body_html_raw, original.body_html_raw);
        assert_eq!(stored.from_name, original.from_name);
        assert_eq!(stored.from_address, original.from_address);
        assert_eq!(
            serde_json::to_value(&stored.to_list).unwrap(),
            serde_json::to_value(&original.to_list).unwrap()
        );
        assert_eq!(
            serde_json::to_value(&stored.cc_list).unwrap(),
            serde_json::to_value(&original.cc_list).unwrap()
        );
        assert_eq!(stored.date, original.date);
        assert_eq!(stored.message_id_header, original.message_id_header);
        // Existing-message updates use the server's (namespaced) conversation ID.
        assert_eq!(
            stored.thread_id.as_deref(),
            Some("outlook:original-conversation")
        );
        assert_eq!(stored.created_at, original.created_at);
        assert_eq!(stored.is_read, body_type == "text");
        assert_eq!(stored.is_starred, body_type == "text");
        assert!(!stored.is_draft);
        assert!(stored.has_attachments);
        let attachments = store.list_attachments_by_message(&original.id).unwrap();
        assert_eq!(attachments.len(), 1);
        assert_eq!(attachments[0].filename, "keep.txt");
        assert_eq!(
            std::fs::read(attachments[0].local_path.as_ref().unwrap()).unwrap(),
            b"attachment contents"
        );
        let event = message_rx.try_recv().unwrap();
        assert_eq!(event.message.subject, original.subject);
        assert!(
            !event.notify,
            "updating existing mail must not notify as new mail"
        );
        std::fs::remove_dir_all(attachments_dir).unwrap();
    }
}

#[tokio::test]
async fn outlook_complete_delta_can_clear_content_and_flags() {
    use crate::provider::outlook::resolve_outlook_delta_page;

    let store = Arc::new(Store::open_in_memory().unwrap());
    let folder = make_folder("inbox");
    store.insert_account(&make_account()).unwrap();
    store.insert_folder(&folder).unwrap();
    let mut original = make_message("outlook-partial-delta");
    original.subject = "Previous subject".into();
    original.body_text = "Previous body".into();
    original.is_read = true;
    original.is_starred = true;
    store
        .insert_message(&original, std::slice::from_ref(&folder.id))
        .unwrap();

    let mut cleared = complete_graph_message("text");
    cleared["subject"] = serde_json::json!("");
    cleared["bodyPreview"] = serde_json::json!("");
    cleared["body"]["content"] = serde_json::json!("");
    cleared["toRecipients"] = serde_json::json!([]);
    cleared["ccRecipients"] = serde_json::json!([]);
    cleared["flag"]["flagStatus"] = serde_json::json!("notFlagged");
    cleared["hasAttachments"] = serde_json::json!(false);
    let page = resolve_outlook_delta_page(
        serde_json::from_value(serde_json::json!({ "value": [cleared] })).unwrap(),
        "account-1",
        |_| async { panic!("explicit empty values are a complete message") },
    )
    .await
    .unwrap();
    let worker = OutlookSyncWorker::new(
        "account-1",
        Arc::new(OutlookProvider::new("token".into(), "account-1".into())),
        Arc::clone(&store),
        std::env::temp_dir(),
    );
    assert_eq!(
        worker
            .persist_folder_messages(&folder, page.messages, false)
            .await,
        0
    );
    let stored = store.get_message(&original.id).unwrap().unwrap();
    assert!(stored.subject.is_empty());
    assert!(stored.snippet.is_empty());
    assert!(stored.body_text.is_empty());
    assert!(stored.body_html_raw.is_empty());
    assert!(stored.to_list.is_empty());
    assert!(stored.cc_list.is_empty());
    assert!(!stored.is_read);
    assert!(!stored.is_starred);
}

#[tokio::test]
async fn outlook_delta_removes_attachments_deleted_from_an_existing_message() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let account = make_account();
    let folder = make_folder("inbox");
    let mut existing = make_message("outlook-attachment-delete");
    existing.has_attachments = true;
    let attachments_dir = std::env::temp_dir().join(format!(
        "pebble-outlook-existing-attachment-delete-{}",
        new_id()
    ));
    let old_path = attachments_dir.join("old.txt");
    std::fs::create_dir_all(&attachments_dir).unwrap();
    std::fs::write(&old_path, b"old attachment").unwrap();

    store.insert_account(&account).unwrap();
    store.insert_folder(&folder).unwrap();
    store
        .insert_message(&existing, std::slice::from_ref(&folder.id))
        .unwrap();
    store
        .insert_attachment(&Attachment {
            id: new_id(),
            message_id: existing.id.clone(),
            filename: "old.txt".to_string(),
            mime_type: "text/plain".to_string(),
            size: 14,
            local_path: Some(old_path.to_string_lossy().into_owned()),
            content_id: None,
            is_inline: false,
        })
        .unwrap();

    let mut changed = make_message("outlook-attachment-delete");
    changed.has_attachments = false;
    let worker = OutlookSyncWorker::new(
        "account-1",
        Arc::new(OutlookProvider::new(
            "token".to_string(),
            "account-1".to_string(),
        )),
        Arc::clone(&store),
        attachments_dir.clone(),
    );

    let failures = worker
        .persist_folder_messages(&folder, vec![changed], false)
        .await;

    assert_eq!(failures, 0);
    assert!(store
        .list_attachments_by_message(&existing.id)
        .unwrap()
        .is_empty());
    assert!(!old_path.exists());
    let _ = std::fs::remove_dir_all(attachments_dir);
}

#[tokio::test]
async fn outlook_delta_adds_attachments_to_an_existing_message() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let account = make_account();
    let folder = make_folder("inbox");
    let existing = make_message("outlook-attachment-add");
    let attachments_dir = std::env::temp_dir().join(format!(
        "pebble-outlook-existing-attachment-add-{}",
        new_id()
    ));

    store.insert_account(&account).unwrap();
    store.insert_folder(&folder).unwrap();
    store
        .insert_message(&existing, std::slice::from_ref(&folder.id))
        .unwrap();

    let mut changed = make_message("outlook-attachment-add");
    changed.has_attachments = true;
    let worker = OutlookSyncWorker::new(
        "account-1",
        Arc::new(OutlookProvider::new(
            "token".to_string(),
            "account-1".to_string(),
        )),
        Arc::clone(&store),
        attachments_dir.clone(),
    );

    let failures = worker
        .persist_folder_messages_with_attachment_fetch(
            &folder,
            vec![changed],
            false,
            |remote_id| async move {
                assert_eq!(remote_id, "outlook-attachment-add");
                Ok(vec![make_attachment_data("new.txt", b"new attachment")])
            },
        )
        .await;

    let attachments = store.list_attachments_by_message(&existing.id).unwrap();
    assert_eq!(failures, 0);
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].filename, "new.txt");
    let path = attachments[0].local_path.as_ref().unwrap();
    assert_eq!(std::fs::read(path).unwrap(), b"new attachment");
    let _ = std::fs::remove_dir_all(attachments_dir);
}

#[tokio::test]
async fn outlook_delta_replaces_attachments_on_an_existing_message() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let account = make_account();
    let folder = make_folder("inbox");
    let mut existing = make_message("outlook-attachment-replace");
    existing.has_attachments = true;
    let attachments_dir = std::env::temp_dir().join(format!(
        "pebble-outlook-existing-attachment-replace-{}",
        new_id()
    ));
    let old_path = attachments_dir.join("old.txt");
    std::fs::create_dir_all(&attachments_dir).unwrap();
    std::fs::write(&old_path, b"old attachment").unwrap();

    store.insert_account(&account).unwrap();
    store.insert_folder(&folder).unwrap();
    store
        .insert_message(&existing, std::slice::from_ref(&folder.id))
        .unwrap();
    store
        .insert_attachment(&Attachment {
            id: new_id(),
            message_id: existing.id.clone(),
            filename: "old.txt".to_string(),
            mime_type: "text/plain".to_string(),
            size: 14,
            local_path: Some(old_path.to_string_lossy().into_owned()),
            content_id: None,
            is_inline: false,
        })
        .unwrap();

    let mut changed = make_message("outlook-attachment-replace");
    changed.has_attachments = true;
    let worker = OutlookSyncWorker::new(
        "account-1",
        Arc::new(OutlookProvider::new(
            "token".to_string(),
            "account-1".to_string(),
        )),
        Arc::clone(&store),
        attachments_dir.clone(),
    );

    let failures = worker
        .persist_folder_messages_with_attachment_fetch(&folder, vec![changed], false, |_| async {
            Ok(vec![make_attachment_data("new.txt", b"replacement")])
        })
        .await;

    let attachments = store.list_attachments_by_message(&existing.id).unwrap();
    assert_eq!(failures, 0);
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].filename, "new.txt");
    assert_eq!(
        std::fs::read(attachments[0].local_path.as_ref().unwrap()).unwrap(),
        b"replacement"
    );
    assert!(!old_path.exists());
    let _ = std::fs::remove_dir_all(attachments_dir);
}

#[tokio::test]
async fn outlook_invalid_attachment_base64_preserves_existing_state_and_blocks_cursor() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let account = make_account();
    let folder = make_folder("inbox");
    let mut existing = make_message("outlook-attachment-fetch-failure");
    existing.has_attachments = true;
    let attachments_dir = std::env::temp_dir().join(format!(
        "pebble-outlook-attachment-fetch-failure-{}",
        new_id()
    ));
    let old_path = attachments_dir.join("old.txt");
    std::fs::create_dir_all(&attachments_dir).unwrap();
    std::fs::write(&old_path, b"old attachment").unwrap();

    store.insert_account(&account).unwrap();
    store.insert_folder(&folder).unwrap();
    store
        .insert_message(&existing, std::slice::from_ref(&folder.id))
        .unwrap();
    let old_attachment = Attachment {
        id: new_id(),
        message_id: existing.id.clone(),
        filename: "old.txt".to_string(),
        mime_type: "text/plain".to_string(),
        size: 14,
        local_path: Some(old_path.to_string_lossy().into_owned()),
        content_id: None,
        is_inline: false,
    };
    store.insert_attachment(&old_attachment).unwrap();

    let mut changed = make_message("outlook-attachment-fetch-failure");
    changed.has_attachments = true;
    changed.subject = "must not commit".to_string();
    let worker = OutlookSyncWorker::new(
        "account-1",
        Arc::new(OutlookProvider::new(
            "token".to_string(),
            "account-1".to_string(),
        )),
        Arc::clone(&store),
        attachments_dir.clone(),
    );

    let failures = worker
        .persist_folder_messages_with_attachment_fetch(&folder, vec![changed], false, |_| async {
            crate::provider::outlook::base64_standard_decode_outlook("TQ$=").map(|_| Vec::new())
        })
        .await;

    assert_eq!(failures, 1);
    assert!(!can_advance_outlook_delta_cursor(failures));
    assert_eq!(
        store.get_message(&existing.id).unwrap().unwrap().subject,
        existing.subject
    );
    let stored_attachments = store.list_attachments_by_message(&existing.id).unwrap();
    assert_eq!(stored_attachments.len(), 1);
    assert_eq!(stored_attachments[0].id, old_attachment.id);
    assert_eq!(stored_attachments[0].local_path, old_attachment.local_path);
    assert!(old_path.exists());
    let _ = std::fs::remove_dir_all(attachments_dir);
}

#[tokio::test]
async fn outlook_attachment_write_failure_preserves_existing_state_and_blocks_cursor() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let account = make_account();
    let folder = make_folder("inbox");
    let mut existing = make_message("outlook-attachment-write-failure");
    existing.has_attachments = true;
    let test_dir = std::env::temp_dir().join(format!(
        "pebble-outlook-attachment-write-failure-{}",
        new_id()
    ));
    std::fs::create_dir_all(&test_dir).unwrap();
    let attachments_root_file = test_dir.join("not-a-directory");
    std::fs::write(&attachments_root_file, b"block directory creation").unwrap();

    store.insert_account(&account).unwrap();
    store.insert_folder(&folder).unwrap();
    store
        .insert_message(&existing, std::slice::from_ref(&folder.id))
        .unwrap();

    let mut changed = make_message("outlook-attachment-write-failure");
    changed.has_attachments = true;
    changed.subject = "must not commit".to_string();
    let worker = OutlookSyncWorker::new(
        "account-1",
        Arc::new(OutlookProvider::new(
            "token".to_string(),
            "account-1".to_string(),
        )),
        Arc::clone(&store),
        attachments_root_file,
    );

    let failures = worker
        .persist_folder_messages_with_attachment_fetch(&folder, vec![changed], false, |_| async {
            Ok(vec![make_attachment_data("new.txt", b"new")])
        })
        .await;

    assert_eq!(failures, 1);
    assert!(!can_advance_outlook_delta_cursor(failures));
    assert_eq!(
        store.get_message(&existing.id).unwrap().unwrap().subject,
        existing.subject
    );
    assert!(store
        .list_attachments_by_message(&existing.id)
        .unwrap()
        .is_empty());
    let _ = std::fs::remove_dir_all(test_dir);
}

#[tokio::test]
async fn outlook_attachment_db_failure_rolls_back_staged_files_and_blocks_cursor() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let account = make_account();
    let stored_folder = make_folder("inbox");
    let missing_folder = make_folder("missing");
    let existing = make_message("outlook-attachment-db-failure");
    let attachments_dir =
        std::env::temp_dir().join(format!("pebble-outlook-attachment-db-failure-{}", new_id()));

    store.insert_account(&account).unwrap();
    store.insert_folder(&stored_folder).unwrap();
    store
        .insert_message(&existing, std::slice::from_ref(&stored_folder.id))
        .unwrap();

    let mut changed = make_message("outlook-attachment-db-failure");
    changed.has_attachments = true;
    changed.subject = "must not commit".to_string();
    let worker = OutlookSyncWorker::new(
        "account-1",
        Arc::new(OutlookProvider::new(
            "token".to_string(),
            "account-1".to_string(),
        )),
        Arc::clone(&store),
        attachments_dir.clone(),
    );

    let failures = worker
        .persist_folder_messages_with_attachment_fetch(
            &missing_folder,
            vec![changed],
            false,
            |_| async { Ok(vec![make_attachment_data("new.txt", b"new")]) },
        )
        .await;

    assert_eq!(failures, 1);
    assert!(!can_advance_outlook_delta_cursor(failures));
    assert_eq!(
        store.get_message(&existing.id).unwrap().unwrap().subject,
        existing.subject
    );
    assert!(store
        .list_attachments_by_message(&existing.id)
        .unwrap()
        .is_empty());
    assert!(!attachments_dir.join(&existing.id).join("new.txt").exists());
    let _ = std::fs::remove_dir_all(attachments_dir);
}

#[tokio::test]
async fn outlook_delta_does_not_delete_an_attachment_file_still_referenced_elsewhere() {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let account = make_account();
    let folder = make_folder("inbox");
    let mut existing = make_message("outlook-shared-attachment-first");
    existing.has_attachments = true;
    let mut other = make_message("outlook-shared-attachment-second");
    other.id = "local-shared-attachment-second".to_string();
    other.has_attachments = true;
    let attachments_dir =
        std::env::temp_dir().join(format!("pebble-outlook-shared-attachment-{}", new_id()));
    let shared_path = attachments_dir.join("shared.txt");
    std::fs::create_dir_all(&attachments_dir).unwrap();
    std::fs::write(&shared_path, b"shared attachment").unwrap();

    store.insert_account(&account).unwrap();
    store.insert_folder(&folder).unwrap();
    store
        .insert_message(&existing, std::slice::from_ref(&folder.id))
        .unwrap();
    store
        .insert_message(&other, std::slice::from_ref(&folder.id))
        .unwrap();
    for message_id in [&existing.id, &other.id] {
        store
            .insert_attachment(&Attachment {
                id: new_id(),
                message_id: message_id.clone(),
                filename: "shared.txt".to_string(),
                mime_type: "text/plain".to_string(),
                size: 17,
                local_path: Some(shared_path.to_string_lossy().into_owned()),
                content_id: None,
                is_inline: false,
            })
            .unwrap();
    }

    let changed = make_message("outlook-shared-attachment-first");
    let worker = OutlookSyncWorker::new(
        "account-1",
        Arc::new(OutlookProvider::new(
            "token".to_string(),
            "account-1".to_string(),
        )),
        Arc::clone(&store),
        attachments_dir.clone(),
    );

    let failures = worker
        .persist_folder_messages(&folder, vec![changed], false)
        .await;

    assert_eq!(failures, 0);
    assert!(store
        .list_attachments_by_message(&existing.id)
        .unwrap()
        .is_empty());
    assert_eq!(
        store.list_attachments_by_message(&other.id).unwrap().len(),
        1
    );
    assert!(shared_path.exists());
    let _ = std::fs::remove_dir_all(attachments_dir);
}
