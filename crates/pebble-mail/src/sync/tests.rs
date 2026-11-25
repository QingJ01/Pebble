use super::*;
use pebble_core::{Folder, FolderRole};
use std::io::{self, Write};

struct FailingAttachmentWriter {
    fail_write: bool,
    fail_flush: bool,
}

impl Write for FailingAttachmentWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.fail_write {
            Err(io::Error::other("injected write failure"))
        } else {
            Ok(buf.len())
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        if self.fail_flush {
            Err(io::Error::other("injected flush failure"))
        } else {
            Ok(())
        }
    }
}

fn test_attachment(filename: &str, data: &[u8]) -> AttachmentData {
    AttachmentData {
        meta: crate::parser::AttachmentMeta {
            filename: filename.to_string(),
            mime_type: "application/octet-stream".to_string(),
            size: data.len(),
            content_id: None,
            is_inline: false,
        },
        data: data.to_vec(),
    }
}

fn test_store_with_message(message_id: &str) -> Arc<Store> {
    let store = Arc::new(Store::open_in_memory().unwrap());
    let now = now_timestamp();
    let account = pebble_core::Account {
        account_label: None,
        provider_display_name: None,
        id: "account-1".to_string(),
        email: "user@example.com".to_string(),
        display_name: "User".to_string(),
        color: None,
        provider: pebble_core::ProviderType::Imap,
        created_at: now,
        updated_at: now,
    };
    let folder = Folder {
        id: "folder-1".to_string(),
        account_id: account.id.clone(),
        remote_id: "INBOX".to_string(),
        name: "Inbox".to_string(),
        folder_type: pebble_core::FolderType::Folder,
        role: Some(FolderRole::Inbox),
        parent_id: None,
        color: None,
        is_system: true,
        sort_order: 0,
    };
    let message = Message {
        id: message_id.to_string(),
        account_id: account.id.clone(),
        remote_id: "remote-1".to_string(),
        message_id_header: None,
        in_reply_to: None,
        references_header: None,
        thread_id: None,
        subject: "Subject".to_string(),
        snippet: String::new(),
        from_address: String::new(),
        from_name: String::new(),
        to_list: Vec::new(),
        cc_list: Vec::new(),
        bcc_list: Vec::new(),
        body_text: String::new(),
        body_html_raw: String::new(),
        has_attachments: true,
        is_read: false,
        is_starred: false,
        is_draft: false,
        date: now,
        remote_version: None,
        is_deleted: false,
        deleted_at: None,
        created_at: now,
        updated_at: now,
    };
    store.insert_account(&account).unwrap();
    store.insert_folder(&folder).unwrap();
    store
        .insert_message(&message, std::slice::from_ref(&folder.id))
        .unwrap();
    store
}

#[test]
fn attachment_db_failure_is_returned_and_removes_written_file() {
    let store = Store::open_in_memory().unwrap();
    let root = std::env::temp_dir().join(format!("pebble-attachment-test-{}", new_id()));
    let message_id = "missing-message";

    let result = persist_message_attachments(
        &store,
        &root,
        message_id,
        vec![test_attachment("report.bin", b"attachment")],
    );

    assert!(result.is_err());
    assert!(!root.join(message_id).join("report.bin").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn attachment_directory_creation_failure_is_returned() {
    let store = Store::open_in_memory().unwrap();
    let root = std::env::temp_dir().join(format!("pebble-attachment-test-{}", new_id()));
    std::fs::write(&root, b"not a directory").unwrap();

    let result = persist_message_attachments(
        &store,
        &root,
        "message-1",
        vec![test_attachment("report.bin", b"attachment")],
    );

    assert!(result.is_err());
    let _ = std::fs::remove_file(root);
}

#[test]
fn attachment_write_failure_is_returned() {
    let mut writer = FailingAttachmentWriter {
        fail_write: true,
        fail_flush: false,
    };

    let result = write_attachment_bytes(&mut writer, b"attachment", Path::new("report.bin"));

    assert!(result.is_err());
}

#[test]
fn attachment_flush_failure_is_returned() {
    let mut writer = FailingAttachmentWriter {
        fail_write: false,
        fail_flush: true,
    };

    let result = write_attachment_bytes(&mut writer, b"attachment", Path::new("report.bin"));

    assert!(result.is_err());
}

#[tokio::test]
async fn attachment_task_join_failure_is_returned() {
    let task = tokio::task::spawn_blocking(|| -> Result<()> {
        panic!("injected attachment worker panic");
    });

    let result = finish_attachment_task(task).await;

    assert!(result.is_err());
}

#[tokio::test]
async fn atomic_new_message_db_failure_keeps_no_half_state_and_cleans_staged_files() {
    let message_id = "message-1";
    let store = test_store_with_message(message_id);
    let message = store.get_message(message_id).unwrap().unwrap();
    store
        .hard_delete_messages(&[message_id.to_string()])
        .unwrap();
    let root = std::env::temp_dir().join(format!("pebble-attachment-test-{}", new_id()));

    let result = store_new_message_with_attachments_atomically(
        Arc::clone(&store),
        root.clone(),
        message,
        vec!["missing-folder".to_string()],
        vec![test_attachment("report.bin", b"attachment")],
    )
    .await;

    assert!(result.is_err());
    assert!(store.get_message(message_id).unwrap().is_none());
    assert!(!root.join(message_id).join("report.bin").exists());
    let _ = std::fs::remove_dir_all(root);
}

#[test]
fn test_sanitize_filename_rejects_windows_reserved_names() {
    assert_eq!(sanitize_filename("CON.txt"), "unnamed_attachment");
    assert_eq!(sanitize_filename("aux"), "unnamed_attachment");
    assert_eq!(sanitize_filename("LPT1.log"), "unnamed_attachment");
}

#[test]
fn test_sanitize_filename_removes_windows_unsafe_characters() {
    assert_eq!(
        sanitize_filename("quarterly:report*final?.pdf"),
        "quarterly_report_final_.pdf",
    );
    assert_eq!(sanitize_filename("report. "), "report");
}

#[test]
fn zero_poll_interval_is_manual_only() {
    let config = SyncConfig {
        poll_interval_secs: 0,
        ..Default::default()
    };

    assert!(config.manual_only());
}

#[test]
fn imap_startup_fetch_notifies_only_when_cursor_exists() {
    assert!(!should_notify_imap_startup_fetch(None));
    assert!(should_notify_imap_startup_fetch(Some(42)));
}

#[test]
fn imap_folder_cursor_roundtrips() {
    let cursor = ImapFolderCursor {
        uidvalidity: Some(1234),
        last_uid: Some(987),
        highest_modseq: Some(4567),
        ..Default::default()
    };

    let json = serde_json::to_string(&cursor).unwrap();
    let decoded: ImapFolderCursor = serde_json::from_str(&json).unwrap();

    assert_eq!(decoded, cursor);
}

#[test]
fn imap_sync_scope_change_preserves_uidvalidity_but_restarts_history() {
    let stored = ImapFolderCursor {
        uidvalidity: Some(1234),
        last_uid: Some(987),
        sync_since: Some(86_400),
        has_sync_scope: true,
        ..Default::default()
    };
    let updated = prepare_imap_folder_cursor_for_sync_scope(stored, Some(0));

    assert_eq!(updated.uidvalidity, Some(1234));
    assert_eq!(updated.last_uid, Some(987));
    assert_eq!(updated.sync_since, Some(0));
    assert!(updated.has_sync_scope);
    assert!(updated.scope_backfill_required);
}

#[test]
fn imap_empty_search_watermark_does_not_skip_a_later_uid() {
    let high_water = completed_imap_history_high_water(None, Some(51), None);
    assert_eq!(high_water, Some(50));
    assert!(51 > high_water.unwrap());
}

#[test]
fn first_unscoped_imap_backfill_cannot_use_local_max_as_a_cursor() {
    let cursor = prepare_imap_folder_cursor_for_sync_scope(ImapFolderCursor::default(), None);
    assert!(cursor.scope_backfill_required);
    assert_eq!(imap_incremental_cursor_uid(&cursor), None);
}

#[test]
fn imap_partial_backfill_and_modseq_write_keep_pending_scope_after_restart() {
    let stored = ImapFolderCursor {
        uidvalidity: Some(1234),
        last_uid: Some(987),
        sync_since: Some(86_400),
        has_sync_scope: true,
        ..Default::default()
    };
    let mut cursor = prepare_imap_folder_cursor_for_sync_scope(stored, Some(0));
    mark_imap_backfill_complete(&mut cursor, None);
    set_imap_cursor_modseq(&mut cursor, 4567);

    let after_modseq_write = serialize_imap_folder_cursor(&cursor).unwrap();
    let restarted = parse_imap_folder_cursor(Some(&after_modseq_write));
    let restarted = prepare_imap_folder_cursor_for_sync_scope(restarted, Some(0));

    assert!(restarted.scope_backfill_required);
    assert_eq!(restarted.highest_modseq, Some(4567));
    assert_eq!(imap_incremental_cursor_uid(&restarted), None);
    assert_eq!(restarted.last_uid, Some(987));
}

#[test]
fn imap_high_water_never_regresses_to_a_smaller_local_max() {
    assert_eq!(
        completed_imap_history_high_water(Some(700), Some(501), Some(600)),
        Some(700)
    );
}

#[test]
fn legacy_half_written_attachment_rows_are_forced_to_refetch() {
    assert!(remote_message_requires_fetch(false, false));
    assert!(!remote_message_requires_fetch(true, false));
    assert!(remote_message_requires_fetch(true, true));
}

#[test]
fn attachment_repair_preserves_local_flags_created_at_and_folder_relations() {
    let store = test_store_with_message("repair-message");
    store
        .update_message_flags("repair-message", Some(true), Some(true))
        .unwrap();
    let archive = Folder {
        id: "folder-archive".to_string(),
        account_id: "account-1".to_string(),
        remote_id: "Archive".to_string(),
        name: "Archive".to_string(),
        folder_type: pebble_core::FolderType::Folder,
        role: Some(FolderRole::Archive),
        parent_id: None,
        color: None,
        is_system: true,
        sort_order: 1,
    };
    store.insert_folder(&archive).unwrap();
    store
        .add_message_to_folder("repair-message", &archive.id)
        .unwrap();
    let existing = store.get_message("repair-message").unwrap().unwrap();
    let mut replacement = existing.clone();
    replacement.is_read = false;
    replacement.is_starred = false;
    replacement.created_at += 999;
    let mut folder_ids = vec!["folder-1".to_string()];

    preserve_local_state_for_attachment_repair(&store, &mut replacement, &mut folder_ids).unwrap();

    assert!(replacement.is_read);
    assert!(replacement.is_starred);
    assert_eq!(replacement.created_at, existing.created_at);
    folder_ids.sort();
    assert_eq!(
        folder_ids,
        vec!["folder-1".to_string(), "folder-archive".to_string()]
    );
}

#[test]
fn imap_folder_cursor_resets_last_uid_when_uidvalidity_changes() {
    let stored = ImapFolderCursor {
        uidvalidity: Some(1234),
        last_uid: Some(987),
        highest_modseq: Some(4567),
        ..Default::default()
    };

    let prepared = prepare_imap_folder_cursor_for_status(stored, Some(9999), Some(7000));

    assert_eq!(prepared.uidvalidity, Some(9999));
    assert_eq!(prepared.last_uid, None);
    assert_eq!(prepared.highest_modseq, None);
}

#[test]
fn imap_folder_cursor_preserves_last_uid_when_uidvalidity_matches() {
    let stored = ImapFolderCursor {
        uidvalidity: Some(1234),
        last_uid: Some(987),
        highest_modseq: Some(4567),
        ..Default::default()
    };

    let prepared = prepare_imap_folder_cursor_for_status(stored, Some(1234), Some(7000));

    assert_eq!(prepared.uidvalidity, Some(1234));
    assert_eq!(prepared.last_uid, Some(987));
    // Only reconcile may advance the stored MODSEQ; copying the server's
    // value here would make reconcile skip unapplied flag changes.
    assert_eq!(prepared.highest_modseq, Some(4567));
}

#[test]
fn interrupted_imap_backfill_resumes_instead_of_switching_to_incremental() {
    let stored = ImapFolderCursor {
        uidvalidity: Some(7),
        has_sync_scope: true,
        scope_backfill_required: true,
        backfill_floor_uid: Some(5_000),
        backfill_uid_next: Some(9_001),
        ..Default::default()
    };
    let json = serialize_imap_folder_cursor(&stored).unwrap();
    let restarted =
        prepare_imap_folder_cursor_for_sync_scope(parse_imap_folder_cursor(Some(&json)), None);
    assert_eq!(restarted.backfill_floor_uid, Some(5_000));
    assert_eq!(imap_incremental_cursor_uid(&restarted), None);

    // Even a legacy last_uid must not short-circuit an unfinished walk.
    let mut with_last = restarted.clone();
    with_last.scope_backfill_required = false;
    with_last.last_uid = Some(9_000);
    assert_eq!(imap_incremental_cursor_uid(&with_last), None);

    let mut done = restarted;
    mark_imap_backfill_complete(&mut done, Some(9_001));
    assert_eq!(done.backfill_floor_uid, None);
    assert_eq!(done.backfill_uid_next, None);
    assert!(!done.scope_backfill_required);
    assert_eq!(
        completed_imap_history_high_water(done.last_uid, Some(9_001), Some(8_000)),
        Some(9_000)
    );
}

#[test]
fn legacy_imap_cursor_json_still_parses_and_new_fields_stay_optional() {
    let legacy = r#"{"uidvalidity":1,"last_uid":2,"highest_modseq":3}"#;
    let cursor = parse_imap_folder_cursor(Some(legacy));
    assert_eq!(cursor.last_uid, Some(2));
    assert!(cursor.retry_uids.is_empty());
    assert_eq!(cursor.deletion_baseline, None);
    let json = serialize_imap_folder_cursor(&cursor).unwrap();
    assert!(!json.contains("retry_uids"));
    assert!(!json.contains("backfill_floor_uid"));
}

#[test]
fn imap_retry_list_is_sorted_deduplicated_and_bounded() {
    let mut cursor = ImapFolderCursor::default();
    add_imap_retry_uids(&mut cursor, &[9, 3, 9]);
    add_imap_retry_uids(&mut cursor, &[3, 1]);
    assert_eq!(cursor.retry_uids, vec![1, 3, 9]);

    let many: Vec<u32> = (1..=(MAX_IMAP_RETRY_UIDS as u32 + 5)).collect();
    add_imap_retry_uids(&mut cursor, &many);
    assert_eq!(cursor.retry_uids.len(), MAX_IMAP_RETRY_UIDS);
    assert_eq!(cursor.retry_uids.last().copied(), many.last().copied());
}

#[test]
fn imap_retry_uids_reset_on_uidvalidity_change() {
    let stored = ImapFolderCursor {
        uidvalidity: Some(1),
        retry_uids: vec![4, 5],
        backfill_floor_uid: Some(10),
        deletion_baseline: Some(ImapDeletionBaseline {
            exists: 3,
            uid_next: 11,
        }),
        ..Default::default()
    };
    let prepared = prepare_imap_folder_cursor_for_status(stored, Some(2), None);
    assert!(prepared.retry_uids.is_empty());
    assert_eq!(prepared.backfill_floor_uid, None);
    assert_eq!(prepared.deletion_baseline, None);
}

#[test]
fn imap_deletion_scan_is_skipped_only_when_counts_prove_no_expunge() {
    let baseline = Some(ImapDeletionBaseline {
        exists: 100,
        uid_next: 500,
    });
    // No baseline or no UIDNEXT: must scan.
    assert!(imap_deletion_check_needed(None, 100, Some(500)));
    assert!(imap_deletion_check_needed(baseline, 100, None));
    // Nothing changed.
    assert!(!imap_deletion_check_needed(baseline, 100, Some(500)));
    // Two new messages, both present.
    assert!(!imap_deletion_check_needed(baseline, 102, Some(502)));
    // One expunged, one added: EXISTS unchanged but UIDNEXT moved.
    assert!(imap_deletion_check_needed(baseline, 100, Some(501)));
    // Pure expunge.
    assert!(imap_deletion_check_needed(baseline, 99, Some(500)));
    // UIDNEXT regressed (server reset): scan.
    assert!(imap_deletion_check_needed(baseline, 100, Some(400)));
}

#[test]
fn oversized_imap_placeholder_keeps_identity_headers_and_drops_mime_structure() {
    let header = b"From: Alice <alice@example.com>\r\n\
Subject: Huge\r\n\
Message-ID: <huge@example.com>\r\n\
In-Reply-To: <parent@example.com>\r\n\
Content-Type: multipart/mixed;\r\n\
 boundary=\"abc\"\r\n\
MIME-Version: 1.0\r\n\
\r\n";
    let placeholder = oversized_placeholder_message(header, 60 * 1024 * 1024);
    let parsed = parse_raw_email(&placeholder).unwrap();
    assert_eq!(parsed.subject, "Huge");
    assert_eq!(parsed.from_address, "alice@example.com");
    assert_eq!(
        parsed.message_id_header.as_deref(),
        Some("<huge@example.com>")
    );
    assert_eq!(parsed.in_reply_to.as_deref(), Some("<parent@example.com>"));
    assert!(parsed.body_text.contains("too large to download (60.0 MB)"));
    assert!(!parsed.has_attachments);
}

#[test]
fn imap_uidvalidity_change_keeps_old_generation_until_snapshot_commits() {
    let store = Store::open_in_memory().unwrap();
    let now = now_timestamp();
    let account = pebble_core::Account {
        account_label: None,
        provider_display_name: None,
        id: "uidvalidity-account".to_string(),
        email: "uidvalidity@example.com".to_string(),
        display_name: "UIDVALIDITY".to_string(),
        color: None,
        provider: pebble_core::ProviderType::Imap,
        created_at: now,
        updated_at: now,
    };
    let folder = Folder {
        id: "uidvalidity-folder".to_string(),
        account_id: account.id.clone(),
        remote_id: "INBOX".to_string(),
        name: "Inbox".to_string(),
        folder_type: pebble_core::FolderType::Folder,
        role: Some(FolderRole::Inbox),
        parent_id: None,
        color: None,
        is_system: true,
        sort_order: 0,
    };
    let message = Message {
        id: "old-generation-message".to_string(),
        account_id: account.id.clone(),
        remote_id: "42".to_string(),
        message_id_header: None,
        in_reply_to: None,
        references_header: None,
        thread_id: None,
        subject: "old generation".to_string(),
        snippet: String::new(),
        from_address: String::new(),
        from_name: String::new(),
        to_list: Vec::new(),
        cc_list: Vec::new(),
        bcc_list: Vec::new(),
        body_text: String::new(),
        body_html_raw: String::new(),
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
    };
    store.insert_account(&account).unwrap();
    store.insert_folder(&folder).unwrap();
    store
        .insert_message(&message, std::slice::from_ref(&folder.id))
        .unwrap();
    store
        .upsert_sync_failure(
            &account.id,
            &folder.id,
            &message.remote_id,
            "imap",
            "old generation failure",
        )
        .unwrap();

    let prepared = prepare_imap_folder_cursor_for_status(
        ImapFolderCursor {
            uidvalidity: Some(100),
            last_uid: Some(42),
            highest_modseq: Some(900),
            ..Default::default()
        },
        Some(200),
        Some(1000),
    );

    assert_eq!(prepared.last_uid, None);
    assert_eq!(prepared.highest_modseq, None);
    assert!(store.get_message(&message.id).unwrap().is_some());
    assert!(store
        .has_sync_failures_for_folder(&account.id, &folder.id)
        .unwrap());
}

#[tokio::test]
async fn imap_uidvalidity_backfill_failure_keeps_the_previous_cursor() {
    let store = test_store_with_message("cursor-message");
    let folder = store
        .list_folders("account-1")
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let previous = ImapFolderCursor {
        uidvalidity: Some(100),
        last_uid: Some(42),
        highest_modseq: Some(900),
        ..Default::default()
    };
    let previous_state = serialize_imap_folder_cursor(&previous).unwrap();
    store
        .set_folder_sync_state("account-1", &folder.id, &previous_state)
        .unwrap();
    store
        .upsert_sync_failure(
            "account-1",
            &folder.id,
            "42",
            "imap",
            "new generation parse failure",
        )
        .unwrap();
    let provider = Arc::new(crate::provider::imap_provider::ImapMailProvider::new(
        crate::imap::ImapConfig {
            host: "localhost".to_string(),
            port: 993,
            username: "user".to_string(),
            password: "password".to_string(),
            security: crate::imap::ConnectionSecurity::Tls,
            accept_invalid_certs: false,
            proxy: None,
        },
    ));
    let (_stop_tx, stop_rx) = watch::channel(false);
    let worker = SyncWorker::new(
        "account-1",
        provider,
        Arc::clone(&store),
        stop_rx,
        std::env::temp_dir(),
    );
    let current_generation = ImapFolderCursor {
        uidvalidity: Some(200),
        last_uid: None,
        highest_modseq: None,
        ..Default::default()
    };

    worker
        .persist_imap_folder_cursor_after_sync(&folder, current_generation, None)
        .await
        .unwrap();

    assert_eq!(
        store
            .get_folder_sync_state("account-1", &folder.id)
            .unwrap()
            .as_deref(),
        Some(previous_state.as_str())
    );
}

#[test]
fn imap_folder_cursor_does_not_advance_with_unresolved_failures() {
    assert!(!can_advance_imap_folder_cursor(true));
}

#[test]
fn imap_folder_cursor_advances_without_unresolved_failures() {
    assert!(can_advance_imap_folder_cursor(false));
}

#[test]
fn imap_deletion_diff_runs_when_server_and_local_counts_match() {
    assert!(should_run_imap_deletion_diff(2, 2));
}

#[test]
fn imap_deletion_diff_skips_empty_local_state() {
    assert!(!should_run_imap_deletion_diff(10, 0));
}

#[test]
fn imap_polling_fallback_uses_realtime_policy_delays() {
    let config = SyncConfig {
        poll_interval_secs: 60,
        ..Default::default()
    };
    let policy = imap_poll_policy(&config);

    assert_eq!(
        policy.next_delay(crate::realtime_policy::RealtimeContext {
            app_foreground: true,
            recent_activity: true,
            consecutive_failures: 0,
        }),
        std::time::Duration::from_secs(60)
    );
    assert_eq!(
        policy.next_delay(crate::realtime_policy::RealtimeContext {
            app_foreground: false,
            recent_activity: false,
            consecutive_failures: 0,
        }),
        std::time::Duration::from_secs(120)
    );
}

#[test]
fn startup_baseline_seed_requires_successful_initial_sync() {
    assert!(can_seed_imap_polling_baseline_after_startup(true));
    assert!(!can_seed_imap_polling_baseline_after_startup(false));
}

#[test]
fn idle_fallback_baseline_refresh_requires_successful_catch_up() {
    assert!(can_refresh_imap_polling_baseline_after_idle_fallback(true));
    assert!(!can_refresh_imap_polling_baseline_after_idle_fallback(
        false
    ));
}

#[test]
fn imap_polling_baseline_refuses_unresolved_inbox_failures() {
    let mut last_exists = Some(crate::idle::MailboxUidState {
        uidvalidity: Some(42),
        highest_uid: 7,
    });

    let seeded = apply_local_inbox_uid_baseline(&mut last_exists, Some(42), Some(12), true);

    assert!(!seeded);
    assert_eq!(
        last_exists,
        Some(crate::idle::MailboxUidState {
            uidvalidity: Some(42),
            highest_uid: 7,
        })
    );
}

#[test]
fn imap_polling_baseline_seeds_local_max_uid_without_unresolved_inbox_failures() {
    let mut last_exists = Some(crate::idle::MailboxUidState {
        uidvalidity: Some(42),
        highest_uid: 7,
    });

    let seeded = apply_local_inbox_uid_baseline(&mut last_exists, Some(43), Some(12), false);

    assert!(seeded);
    assert_eq!(
        last_exists,
        Some(crate::idle::MailboxUidState {
            uidvalidity: Some(43),
            highest_uid: 12,
        })
    );
}

#[test]
fn imap_polling_baseline_seeds_zero_for_clean_empty_local_inbox() {
    let mut last_exists = Some(crate::idle::MailboxUidState {
        uidvalidity: Some(42),
        highest_uid: 7,
    });

    let seeded = apply_local_inbox_uid_baseline(&mut last_exists, Some(43), None, false);

    assert!(seeded);
    assert_eq!(
        last_exists,
        Some(crate::idle::MailboxUidState {
            uidvalidity: Some(43),
            highest_uid: 0,
        })
    );
}

#[test]
fn inbox_missing_mailbox_is_not_skipped_during_initial_sync() {
    assert!(!should_skip_missing_imap_mailbox_during_initial_sync(Some(
        pebble_core::FolderRole::Inbox
    )));
}

#[test]
fn non_inbox_missing_mailbox_can_be_skipped_during_initial_sync() {
    assert!(should_skip_missing_imap_mailbox_during_initial_sync(Some(
        pebble_core::FolderRole::Sent
    )));
}

#[test]
fn inbox_initial_sync_folder_failure_fails_initial_sync() {
    assert!(should_fail_initial_sync_for_folder_error(
        Some(pebble_core::FolderRole::Inbox),
        false
    ));
}

#[test]
fn non_inbox_non_retryable_initial_sync_folder_failure_does_not_fail_initial_sync() {
    assert!(!should_fail_initial_sync_for_folder_error(
        Some(pebble_core::FolderRole::Sent),
        false
    ));
}

#[test]
fn non_inbox_retryable_initial_sync_folder_failure_fails_initial_sync() {
    assert!(should_fail_initial_sync_for_folder_error(None, true));
}

#[test]
fn first_reconcile_deadline_is_delayed_by_interval() {
    let now = Instant::now();
    let interval = Duration::from_secs(900);

    assert_eq!(first_reconcile_deadline(now, interval), now + interval);
}

#[test]
fn idle_check_disconnect_does_not_surface_when_recovery_succeeds() {
    let message = idle_check_recovery_user_error(None, None);

    assert!(message.is_none());
}

#[test]
fn idle_check_reconnect_failure_surfaces_connection_error() {
    let message =
        idle_check_recovery_user_error(Some("Network error: os error 10053".to_string()), None);

    assert_eq!(
        message,
        Some((
            "connection",
            "IMAP reconnect after idle check failed: Network error: os error 10053".to_string()
        ))
    );
}

#[test]
fn imap_windows_connection_abort_is_retryable_for_polling() {
    let error = pebble_core::PebbleError::Network(
        "SELECT failed: io: 你的主机中的软件中止了一个已建立的连接。 (os error 10053)".to_string(),
    );

    assert!(is_retryable_imap_connection_error(&error));
}

#[test]
fn imap_rustls_unexpected_eof_is_retryable_for_polling() {
    let error = pebble_core::PebbleError::Network(
            "SELECT failed: io: peer closed connection without sending TLS close_notify: https://docs.rs/rustls/latest/rustls/manual/_03_howto/index.html#unexpected-eof"
                .to_string(),
        );

    assert!(is_retryable_imap_connection_error(&error));
}

#[test]
fn imap_missing_folder_select_error_is_not_retryable_for_polling() {
    let error = pebble_core::PebbleError::Network(
        "SELECT failed: no response: code: None, info: Some(\"SELECT Folder not exist\")"
            .to_string(),
    );

    assert!(!is_retryable_imap_connection_error(&error));
}

#[test]
fn imap_missing_folder_select_error_is_detected_for_suppression() {
    let error = pebble_core::PebbleError::Network(
        "SELECT failed: no response: code: None, info: Some(\"SELECT Folder not exist\")"
            .to_string(),
    );

    assert!(is_missing_imap_mailbox_error(&error));
}

#[test]
fn imap_local_archive_is_not_a_remote_sync_target() {
    let folder = pebble_core::Folder {
        id: "folder-1".to_string(),
        account_id: "account-1".to_string(),
        remote_id: "__local_archive__".to_string(),
        name: "Archive".to_string(),
        folder_type: pebble_core::FolderType::Folder,
        role: Some(pebble_core::FolderRole::Archive),
        parent_id: None,
        color: None,
        is_system: true,
        sort_order: 3,
    };

    assert!(!should_attempt_imap_remote_folder(&folder));
}

fn test_folder(role: Option<pebble_core::FolderRole>, remote_id: &str) -> pebble_core::Folder {
    pebble_core::Folder {
        id: format!("folder-{remote_id}"),
        account_id: "account-1".to_string(),
        remote_id: remote_id.to_string(),
        name: remote_id.to_string(),
        folder_type: pebble_core::FolderType::Folder,
        role,
        parent_id: None,
        color: None,
        is_system: true,
        sort_order: 0,
    }
}

#[test]
fn imap_realtime_poll_targets_inbox_only() {
    let inbox = test_folder(Some(pebble_core::FolderRole::Inbox), "INBOX");
    let sent = test_folder(Some(pebble_core::FolderRole::Sent), "Sent");
    let spam = test_folder(Some(pebble_core::FolderRole::Spam), "Spam");
    let custom = test_folder(None, "Newsletters");

    assert!(should_poll_imap_folder_for_realtime(&inbox));
    assert!(!should_poll_imap_folder_for_realtime(&sent));
    assert!(!should_poll_imap_folder_for_realtime(&spam));
    assert!(!should_poll_imap_folder_for_realtime(&custom));
}

#[test]
fn explicit_imap_selection_always_includes_inbox() {
    let inbox = test_folder(Some(pebble_core::FolderRole::Inbox), "INBOX");
    let sent = test_folder(Some(pebble_core::FolderRole::Sent), "Sent");
    let custom = test_folder(None, "Newsletters");
    let selected = HashSet::from(["Newsletters".to_string()]);

    assert!(should_include_discovered_imap_folder(
        &inbox,
        Some(&selected)
    ));
    assert!(!should_include_discovered_imap_folder(
        &sent,
        Some(&selected)
    ));
    assert!(should_include_discovered_imap_folder(
        &custom,
        Some(&selected)
    ));
}

#[test]
fn missing_imap_selection_preserves_legacy_all_folder_sync() {
    let sent = test_folder(Some(pebble_core::FolderRole::Sent), "Sent");
    let custom = test_folder(None, "Newsletters");

    assert!(should_include_discovered_imap_folder(&sent, None));
    assert!(should_include_discovered_imap_folder(&custom, None));
}

#[test]
fn history_page_ranges_empty_mailbox_has_no_pages() {
    assert_eq!(history_page_ranges(0, 200), Vec::<(u32, u32)>::new());
}

#[test]
fn history_page_ranges_single_page_when_mailbox_fits() {
    assert_eq!(history_page_ranges(1, 200), vec![(1, 1)]);
    assert_eq!(history_page_ranges(200, 200), vec![(1, 200)]);
}

#[test]
fn history_page_ranges_pages_newest_to_oldest() {
    assert_eq!(history_page_ranges(201, 200), vec![(2, 201), (1, 1)]);
    assert_eq!(
        history_page_ranges(450, 200),
        vec![(251, 450), (51, 250), (1, 50)]
    );
}

#[test]
fn bounded_uidvalidity_recovery_selects_complete_snapshot_and_retains_local_relations() {
    let store = test_store_with_message("uidvalidity-stable");
    let mut old = store.get_message("uidvalidity-stable").unwrap().unwrap();
    old.remote_id = "42".to_string();
    old.message_id_header = Some("<stable@example.com>".to_string());
    old.date = now_timestamp() - 365 * 86_400;
    store
        .replace_message_with_attachments(&old, &["folder-1".to_string()], &[])
        .unwrap();
    store.add_label(&old.id, "Keep").unwrap();
    store
        .update_sync_state("account-1", |state| {
            state.sync_since = Some(now_timestamp() - 30 * 86_400);
        })
        .unwrap();

    let (ranges, uid_next) = imap_uidvalidity_snapshot_plan(2, Some(100));
    let selected_uids = ranges
        .into_iter()
        .flat_map(|(start, end)| start..=end)
        .collect::<Vec<_>>();
    assert_eq!(selected_uids, vec![1, 2]);
    assert_eq!(uid_next, Some(100));

    let snapshot = selected_uids
        .into_iter()
        .map(|uid| {
            let mut fresh = old.clone();
            fresh.id = new_id();
            fresh.remote_id = uid.to_string();
            fresh.message_id_header = Some(if uid == 1 {
                "<stable@example.com>".to_string()
            } else {
                "<new@example.com>".to_string()
            });
            fresh.subject = format!("replacement {uid}");
            pebble_store::messages::ImapFolderSnapshotMessage {
                message: fresh,
                attachments: Vec::new(),
            }
        })
        .collect();
    store
        .reconcile_imap_folder_uidvalidity("account-1", "folder-1", snapshot)
        .unwrap();

    let retained = store.get_message(&old.id).unwrap().unwrap();
    assert_eq!(retained.remote_id, "1");
    assert_eq!(
        store.get_message_folder_ids(&old.id).unwrap(),
        vec!["folder-1".to_string()]
    );
    assert_eq!(store.get_message_labels(&old.id).unwrap().len(), 1);
}

#[test]
fn imap_full_poll_targets_all_remote_folders() {
    let inbox = test_folder(Some(pebble_core::FolderRole::Inbox), "INBOX");
    let sent = test_folder(Some(pebble_core::FolderRole::Sent), "Sent");
    let spam = test_folder(Some(pebble_core::FolderRole::Spam), "Spam");
    let custom = test_folder(None, "Newsletters");
    let local = test_folder(Some(pebble_core::FolderRole::Archive), "__local_archive__");

    assert!(should_poll_imap_folder_for_scope(
        &inbox,
        ImapPollScope::Full
    ));
    assert!(should_poll_imap_folder_for_scope(
        &sent,
        ImapPollScope::Full
    ));
    assert!(should_poll_imap_folder_for_scope(
        &spam,
        ImapPollScope::Full
    ));
    assert!(should_poll_imap_folder_for_scope(
        &custom,
        ImapPollScope::Full
    ));
    assert!(!should_poll_imap_folder_for_scope(
        &local,
        ImapPollScope::Full
    ));
}
