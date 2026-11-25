use super::ImapFolderSnapshotMessage;
use crate::Store;
use pebble_core::*;
use std::collections::HashMap;

fn make_account() -> Account {
    let now = now_timestamp();
    Account {
        account_label: None,
        provider_display_name: None,
        id: new_id(),
        email: "imap@example.com".to_string(),
        display_name: "IMAP".to_string(),
        color: None,
        provider: ProviderType::Imap,
        created_at: now,
        updated_at: now,
    }
}

fn make_folder(account_id: &str, remote_id: &str, role: FolderRole, sort_order: i32) -> Folder {
    Folder {
        id: new_id(),
        account_id: account_id.to_string(),
        remote_id: remote_id.to_string(),
        name: remote_id.to_string(),
        folder_type: FolderType::Folder,
        role: Some(role),
        parent_id: None,
        color: None,
        is_system: true,
        sort_order,
    }
}

fn make_message(account_id: &str, remote_id: &str) -> Message {
    let now = now_timestamp();
    Message {
        id: new_id(),
        account_id: account_id.to_string(),
        remote_id: remote_id.to_string(),
        message_id_header: None,
        in_reply_to: None,
        references_header: None,
        thread_id: None,
        subject: "Test".to_string(),
        snippet: "test".to_string(),
        from_address: "from@example.com".to_string(),
        from_name: "From".to_string(),
        to_list: vec![],
        cc_list: vec![],
        bcc_list: vec![],
        body_text: "body".to_string(),
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
    }
}

#[test]
fn existing_remote_ids_are_scoped_by_folder_for_imap() {
    let store = Store::open_in_memory().unwrap();
    let account = make_account();
    store.insert_account(&account).unwrap();

    let inbox = make_folder(&account.id, "INBOX", FolderRole::Inbox, 0);
    let sent = make_folder(&account.id, "Sent", FolderRole::Sent, 1);
    store.insert_folder(&inbox).unwrap();
    store.insert_folder(&sent).unwrap();

    let msg = make_message(&account.id, "123");
    store
        .insert_message(&msg, std::slice::from_ref(&inbox.id))
        .unwrap();

    let remote_ids = vec!["123".to_string()];
    let inbox_matches = store
        .get_existing_remote_ids_in_folder(&account.id, &inbox.id, &remote_ids)
        .unwrap();
    let sent_matches = store
        .get_existing_remote_ids_in_folder(&account.id, &sent.id, &remote_ids)
        .unwrap();

    assert!(inbox_matches.contains("123"));
    assert!(!sent_matches.contains("123"));
}

#[test]
fn cached_remote_ids_include_folderless_messages_and_are_account_scoped() {
    let store = Store::open_in_memory().unwrap();
    let mut account = make_account();
    account.provider = ProviderType::Gmail;
    let mut other_account = make_account();
    other_account.provider = ProviderType::Gmail;
    store.insert_account(&account).unwrap();
    store.insert_account(&other_account).unwrap();
    let inbox = make_folder(&account.id, "INBOX", FolderRole::Inbox, 0);
    store.insert_folder(&inbox).unwrap();

    let folderless = make_message(&account.id, "folderless");
    let in_folder = make_message(&account.id, "in-folder");
    let mut deleted = make_message(&account.id, "deleted");
    deleted.is_deleted = true;
    let empty_remote_id = make_message(&account.id, "");
    let other_account_message = make_message(&other_account.id, "other-account");

    store.insert_message(&folderless, &[]).unwrap();
    store
        .insert_message(&in_folder, std::slice::from_ref(&inbox.id))
        .unwrap();
    store.insert_message(&deleted, &[]).unwrap();
    store.insert_message(&empty_remote_id, &[]).unwrap();
    store.insert_message(&other_account_message, &[]).unwrap();

    let mut remote_ids = store
        .list_cached_remote_ids_by_account(&account.id)
        .unwrap();
    remote_ids.sort();
    assert_eq!(
        remote_ids,
        vec!["folderless".to_string(), "in-folder".to_string()]
    );
}

#[test]
fn numeric_imap_uids_can_repeat_in_different_folders() {
    let store = Store::open_in_memory().unwrap();
    let account = make_account();
    let inbox = make_folder(&account.id, "INBOX", FolderRole::Inbox, 0);
    let sent = make_folder(&account.id, "Sent", FolderRole::Sent, 1);
    let inbox_message = make_message(&account.id, "123");
    let mut sent_message = make_message(&account.id, "123");
    sent_message.subject = "different mailbox message".to_string();

    store.insert_account(&account).unwrap();
    store.insert_folder(&inbox).unwrap();
    store.insert_folder(&sent).unwrap();
    store
        .insert_message(&inbox_message, std::slice::from_ref(&inbox.id))
        .unwrap();
    store
        .insert_message(&sent_message, std::slice::from_ref(&sent.id))
        .expect("the same IMAP UID in another mailbox must be a distinct message");

    let inbox_state = store
        .list_remote_ids_by_folder(&account.id, &inbox.id)
        .unwrap();
    let sent_state = store
        .list_remote_ids_by_folder(&account.id, &sent.id)
        .unwrap();
    assert_eq!(inbox_state.len(), 1);
    assert_eq!(sent_state.len(), 1);
    assert_ne!(inbox_state[0].0, sent_state[0].0);
}

#[test]
fn move_records_destination_uid_without_source_folder_collision() {
    let store = Store::open_in_memory().unwrap();
    let account = make_account();
    let inbox = make_folder(&account.id, "INBOX", FolderRole::Inbox, 0);
    let archive = make_folder(&account.id, "Archive", FolderRole::Archive, 1);
    let moved = make_message(&account.id, "5");
    let neighbor = make_message(&account.id, "2");
    store.insert_account(&account).unwrap();
    store.insert_folder(&inbox).unwrap();
    store.insert_folder(&archive).unwrap();
    store
        .insert_message(&moved, std::slice::from_ref(&inbox.id))
        .unwrap();
    store
        .insert_message(&neighbor, std::slice::from_ref(&inbox.id))
        .unwrap();

    assert!(store.update_remote_id(&moved.id, "2").is_err());
    store
        .move_message_to_folder_with_remote_id(&moved.id, &archive.id, "2")
        .unwrap();
    assert_eq!(
        store.get_message(&moved.id).unwrap().unwrap().remote_id,
        "2"
    );
    assert_eq!(
        store.get_message_folder_ids(&moved.id).unwrap(),
        vec![archive.id]
    );
    assert_eq!(
        store.get_message(&neighbor.id).unwrap().unwrap().remote_id,
        "2"
    );
}

#[test]
fn move_with_duplicate_destination_uid_rolls_back() {
    let store = Store::open_in_memory().unwrap();
    let account = make_account();
    let inbox = make_folder(&account.id, "INBOX", FolderRole::Inbox, 0);
    let archive = make_folder(&account.id, "Archive", FolderRole::Archive, 1);
    let moved = make_message(&account.id, "5");
    let neighbor = make_message(&account.id, "2");
    store.insert_account(&account).unwrap();
    store.insert_folder(&inbox).unwrap();
    store.insert_folder(&archive).unwrap();
    store
        .insert_message(&moved, std::slice::from_ref(&inbox.id))
        .unwrap();
    store
        .insert_message(&neighbor, std::slice::from_ref(&archive.id))
        .unwrap();

    assert!(store
        .move_message_to_folder_with_remote_id(&moved.id, &archive.id, "2")
        .is_err());
    assert_eq!(
        store.get_message(&moved.id).unwrap().unwrap().remote_id,
        "5"
    );
    assert_eq!(
        store.get_message_folder_ids(&moved.id).unwrap(),
        vec![inbox.id]
    );
}

#[test]
fn non_imap_numeric_remote_ids_remain_account_wide_unique() {
    let store = Store::open_in_memory().unwrap();
    let mut account = make_account();
    account.provider = ProviderType::Gmail;
    let inbox = make_folder(&account.id, "INBOX", FolderRole::Inbox, 0);
    let sent = make_folder(&account.id, "Sent", FolderRole::Sent, 1);
    let inbox_message = make_message(&account.id, "123");
    let sent_message = make_message(&account.id, "123");

    store.insert_account(&account).unwrap();
    store.insert_folder(&inbox).unwrap();
    store.insert_folder(&sent).unwrap();
    store
        .insert_message(&inbox_message, std::slice::from_ref(&inbox.id))
        .unwrap();
    let duplicate = store.insert_message(&sent_message, std::slice::from_ref(&sent.id));

    assert!(duplicate.is_err());
}

#[test]
fn imap_uid_is_rejected_when_repeated_in_the_same_folder() {
    let store = Store::open_in_memory().unwrap();
    let account = make_account();
    let inbox = make_folder(&account.id, "INBOX", FolderRole::Inbox, 0);
    let first = make_message(&account.id, "123");
    let second = make_message(&account.id, "123");

    store.insert_account(&account).unwrap();
    store.insert_folder(&inbox).unwrap();
    store
        .insert_message(&first, std::slice::from_ref(&inbox.id))
        .unwrap();
    let duplicate = store.insert_message(&second, std::slice::from_ref(&inbox.id));

    assert!(duplicate.is_err());
    assert!(store.get_message(&second.id).unwrap().is_none());
}

#[test]
fn incomplete_attachment_rows_are_detected_for_forced_refetch() {
    let store = Store::open_in_memory().unwrap();
    let account = make_account();
    let inbox = make_folder(&account.id, "INBOX", FolderRole::Inbox, 0);
    store.insert_account(&account).unwrap();
    store.insert_folder(&inbox).unwrap();
    let mut message = make_message(&account.id, "42");
    message.has_attachments = true;
    store
        .insert_message(&message, std::slice::from_ref(&inbox.id))
        .unwrap();

    let remote_ids = vec![message.remote_id.clone()];
    let incomplete = store
        .get_incomplete_attachment_message_map_by_remote_ids(
            &account.id,
            Some(&inbox.id),
            &remote_ids,
        )
        .unwrap();
    assert_eq!(incomplete.get("42"), Some(&message.id));

    store
        .insert_attachment(&Attachment {
            id: new_id(),
            message_id: message.id.clone(),
            filename: "complete.bin".to_string(),
            mime_type: "application/octet-stream".to_string(),
            size: 1,
            local_path: None,
            content_id: None,
            is_inline: false,
        })
        .unwrap();
    assert!(store
        .get_incomplete_attachment_message_map_by_remote_ids(&account.id, None, &remote_ids,)
        .unwrap()
        .is_empty());
}

#[test]
fn update_remote_id_reports_missing_message() {
    let store = Store::open_in_memory().unwrap();

    let err = store
        .update_remote_id("missing-message", "new-remote-id")
        .expect_err("updating a missing message must fail");

    assert!(matches!(err, PebbleError::Internal(message) if message.contains("missing-message")));
}

#[test]
fn reconciling_imap_uidvalidity_preserves_only_unambiguous_stable_identity() {
    let store = Store::open_in_memory().unwrap();
    let account = make_account();
    let inbox = make_folder(&account.id, "INBOX", FolderRole::Inbox, 0);
    let sent = make_folder(&account.id, "Sent", FolderRole::Sent, 1);
    let mut stable = make_message(&account.id, "42");
    stable.message_id_header = Some("<stable@example.com>".to_string());
    stable.is_read = true;
    stable.is_starred = true;
    let mut different = make_message(&account.id, "50");
    different.message_id_header = Some("<old@example.com>".to_string());
    let mut shared = make_message(&account.id, "43");
    shared.message_id_header = Some("<shared@example.com>".to_string());
    let sent_only = make_message(&account.id, "44");

    store.insert_account(&account).unwrap();
    store.insert_folder(&inbox).unwrap();
    store.insert_folder(&sent).unwrap();
    store
        .insert_message(&stable, std::slice::from_ref(&inbox.id))
        .unwrap();
    store
        .insert_message(&different, std::slice::from_ref(&inbox.id))
        .unwrap();
    store
        .insert_message(&shared, &[inbox.id.clone(), sent.id.clone()])
        .unwrap();
    store
        .insert_message(&sent_only, std::slice::from_ref(&sent.id))
        .unwrap();
    store.add_label(&stable.id, "Keep").unwrap();
    store
        .upsert_kanban_card(&KanbanCard {
            message_id: stable.id.clone(),
            column: KanbanColumn::Waiting,
            position: 1,
            created_at: 1,
            updated_at: 1,
        })
        .unwrap();
    store
        .snooze_message(&SnoozedMessage {
            message_id: stable.id.clone(),
            snoozed_at: 1,
            unsnoozed_at: 2,
            return_to: "inbox".to_string(),
        })
        .unwrap();
    store.add_label(&different.id, "Must not inherit").unwrap();
    store
        .upsert_sync_failure(&account.id, &inbox.id, "42", "imap", "old generation")
        .unwrap();
    store
        .upsert_sync_failure(&account.id, &sent.id, "44", "imap", "other folder")
        .unwrap();
    store
        .insert_pending_mail_op(&account.id, &different.id, "mark_read", "{}")
        .unwrap();
    store
        .insert_pending_mail_op(&account.id, &shared.id, "mark_read", "{}")
        .unwrap();

    let mut fresh_stable = make_message(&account.id, "99");
    fresh_stable.message_id_header = Some("<stable@example.com>".to_string());
    fresh_stable.subject = "fresh stable".to_string();
    let fresh_stable_generated_id = fresh_stable.id.clone();
    let mut same_uid_different_identity = make_message(&account.id, "50");
    same_uid_different_identity.message_id_header = Some("<new@example.com>".to_string());
    let different_new_id = same_uid_different_identity.id.clone();

    let result = store
        .reconcile_imap_folder_uidvalidity(
            &account.id,
            &inbox.id,
            vec![
                ImapFolderSnapshotMessage {
                    message: fresh_stable,
                    attachments: Vec::new(),
                },
                ImapFolderSnapshotMessage {
                    message: same_uid_different_identity,
                    attachments: Vec::new(),
                },
            ],
        )
        .unwrap();

    assert!(result.unreferenced_attachment_paths.is_empty());
    assert!(store
        .get_message(&fresh_stable_generated_id)
        .unwrap()
        .is_none());
    let reconciled_stable = store.get_message(&stable.id).unwrap().unwrap();
    assert_eq!(reconciled_stable.remote_id, "99");
    assert_eq!(reconciled_stable.subject, "fresh stable");
    assert!(reconciled_stable.is_read);
    assert!(reconciled_stable.is_starred);
    assert_eq!(store.get_message_labels(&stable.id).unwrap().len(), 1);
    assert!(store
        .list_kanban_cards(None)
        .unwrap()
        .iter()
        .any(|card| card.message_id == stable.id));
    assert!(store.get_snoozed_message(&stable.id).unwrap().is_some());

    assert!(store.get_message(&different.id).unwrap().is_none());
    assert!(store.get_message(&different_new_id).unwrap().is_some());
    assert!(store
        .get_message_labels(&different_new_id)
        .unwrap()
        .is_empty());
    assert!(store.get_message(&shared.id).unwrap().is_some());
    assert_eq!(
        store.get_message_folder_ids(&shared.id).unwrap(),
        vec![sent.id.clone()]
    );
    assert!(store.get_message(&sent_only.id).unwrap().is_some());
    assert!(!store
        .has_sync_failures_for_folder(&account.id, &inbox.id)
        .unwrap());
    assert!(store
        .has_sync_failures_for_folder(&account.id, &sent.id)
        .unwrap());
    let pending_ops = store.list_pending_mail_ops(&account.id).unwrap();
    assert_eq!(pending_ops.len(), 1);
    assert_eq!(pending_ops[0].message_id, shared.id);

    let pending: HashMap<String, String> =
        store.list_search_pending().unwrap().into_iter().collect();
    assert_eq!(pending.get(&stable.id).map(String::as_str), Some("index"));
    assert_eq!(
        pending.get(&different.id).map(String::as_str),
        Some("remove")
    );
    assert_eq!(
        pending.get(&different_new_id).map(String::as_str),
        Some("index")
    );
    assert_eq!(pending.get(&shared.id).map(String::as_str), Some("index"));
    assert!(result
        .removed_messages
        .iter()
        .any(|message| message.id == different.id));
}

#[test]
fn failed_imap_uidvalidity_reconcile_keeps_the_old_generation_intact() {
    let store = Store::open_in_memory().unwrap();
    let account = make_account();
    let inbox = make_folder(&account.id, "INBOX", FolderRole::Inbox, 0);
    let sent = make_folder(&account.id, "Sent", FolderRole::Sent, 1);
    store.insert_account(&account).unwrap();
    store.insert_folder(&inbox).unwrap();
    store.insert_folder(&sent).unwrap();

    let mut old = make_message(&account.id, "42");
    old.message_id_header = Some("<stable@example.com>".to_string());
    old.subject = "old generation".to_string();
    store
        .insert_message(&old, std::slice::from_ref(&inbox.id))
        .unwrap();
    store.add_label(&old.id, "Keep").unwrap();
    store
        .upsert_sync_failure(&account.id, &inbox.id, "42", "imap", "keep until commit")
        .unwrap();

    let blocker = make_message(&account.id, "77");
    let duplicate_attachment_id = new_id();
    store
        .replace_message_with_attachments(
            &blocker,
            std::slice::from_ref(&sent.id),
            &[Attachment {
                id: duplicate_attachment_id.clone(),
                message_id: blocker.id.clone(),
                filename: "blocker.bin".to_string(),
                mime_type: "application/octet-stream".to_string(),
                size: 1,
                local_path: None,
                content_id: None,
                is_inline: false,
            }],
        )
        .unwrap();

    let mut fresh = make_message(&account.id, "99");
    fresh.message_id_header = old.message_id_header.clone();
    fresh.subject = "fresh generation".to_string();
    let result = store.reconcile_imap_folder_uidvalidity(
        &account.id,
        &inbox.id,
        vec![ImapFolderSnapshotMessage {
            message: fresh,
            attachments: vec![Attachment {
                id: duplicate_attachment_id,
                message_id: "temporary-id".to_string(),
                filename: "new.bin".to_string(),
                mime_type: "application/octet-stream".to_string(),
                size: 2,
                local_path: None,
                content_id: None,
                is_inline: false,
            }],
        }],
    );
    assert!(result.is_err());

    let retained = store.get_message(&old.id).unwrap().unwrap();
    assert_eq!(retained.remote_id, "42");
    assert_eq!(retained.subject, "old generation");
    assert_eq!(
        store.get_message_folder_ids(&old.id).unwrap(),
        vec![inbox.id.clone()]
    );
    assert_eq!(store.get_message_labels(&old.id).unwrap().len(), 1);
    assert!(store
        .has_sync_failures_for_folder(&account.id, &inbox.id)
        .unwrap());
    assert!(store.list_search_pending().unwrap().is_empty());
}

#[test]
fn imap_uidvalidity_reconcile_allows_stable_messages_to_swap_uids() {
    let store = Store::open_in_memory().unwrap();
    let account = make_account();
    let inbox = make_folder(&account.id, "INBOX", FolderRole::Inbox, 0);
    store.insert_account(&account).unwrap();
    store.insert_folder(&inbox).unwrap();

    let mut old_a = make_message(&account.id, "1");
    old_a.message_id_header = Some("<a@example.com>".to_string());
    let mut old_b = make_message(&account.id, "2");
    old_b.message_id_header = Some("<b@example.com>".to_string());
    store
        .insert_message(&old_a, std::slice::from_ref(&inbox.id))
        .unwrap();
    store
        .insert_message(&old_b, std::slice::from_ref(&inbox.id))
        .unwrap();

    let mut fresh_a = make_message(&account.id, "2");
    fresh_a.message_id_header = old_a.message_id_header.clone();
    let mut fresh_b = make_message(&account.id, "1");
    fresh_b.message_id_header = old_b.message_id_header.clone();
    store
        .reconcile_imap_folder_uidvalidity(
            &account.id,
            &inbox.id,
            vec![
                ImapFolderSnapshotMessage {
                    message: fresh_a,
                    attachments: Vec::new(),
                },
                ImapFolderSnapshotMessage {
                    message: fresh_b,
                    attachments: Vec::new(),
                },
            ],
        )
        .unwrap();

    assert_eq!(
        store.get_message(&old_a.id).unwrap().unwrap().remote_id,
        "2"
    );
    assert_eq!(
        store.get_message(&old_b.id).unwrap().unwrap().remote_id,
        "1"
    );
    assert_eq!(
        store.get_message_folder_ids(&old_a.id).unwrap(),
        vec![inbox.id.clone()]
    );
    assert_eq!(
        store.get_message_folder_ids(&old_b.id).unwrap(),
        vec![inbox.id]
    );
}

#[test]
fn replace_message_updates_non_imap_row_without_triggering_self_duplicate() {
    let store = Store::open_in_memory().unwrap();
    let mut account = make_account();
    account.provider = ProviderType::Outlook;
    account.email = "outlook@example.com".to_string();
    store.insert_account(&account).unwrap();
    let inbox = make_folder(&account.id, "Inbox", FolderRole::Inbox, 0);
    store.insert_folder(&inbox).unwrap();

    let message = make_message(&account.id, "graph-message-id");
    store
        .replace_message_with_attachments(&message, std::slice::from_ref(&inbox.id), &[])
        .unwrap();

    let mut updated = message.clone();
    updated.subject = "Updated by delta sync".to_string();
    store
        .replace_message_with_attachments(&updated, std::slice::from_ref(&inbox.id), &[])
        .unwrap();

    assert_eq!(
        store.get_message(&message.id).unwrap().unwrap().subject,
        "Updated by delta sync"
    );
}

#[test]
fn replace_message_with_attachments_replaces_old_attachment_set() {
    let store = Store::open_in_memory().unwrap();
    let account = make_account();
    store.insert_account(&account).unwrap();

    let drafts = make_folder(&account.id, "Drafts", FolderRole::Drafts, 0);
    let archive = make_folder(&account.id, "Archive", FolderRole::Archive, 1);
    store.insert_folder(&drafts).unwrap();
    store.insert_folder(&archive).unwrap();

    let mut msg = make_message(&account.id, "draft-1");
    msg.is_draft = true;
    msg.has_attachments = true;

    let old_attachment = Attachment {
        id: new_id(),
        message_id: msg.id.clone(),
        filename: "old.pdf".to_string(),
        mime_type: "application/pdf".to_string(),
        size: 10,
        local_path: Some("C:\\tmp\\old.pdf".to_string()),
        content_id: None,
        is_inline: false,
    };
    store
        .replace_message_with_attachments(&msg, std::slice::from_ref(&drafts.id), &[old_attachment])
        .unwrap();
    store.add_label(&msg.id, "Important").unwrap();
    store
        .upsert_kanban_card(&KanbanCard {
            message_id: msg.id.clone(),
            column: KanbanColumn::Waiting,
            position: 7,
            created_at: 1,
            updated_at: 2,
        })
        .unwrap();
    store
        .snooze_message(&SnoozedMessage {
            message_id: msg.id.clone(),
            snoozed_at: 3,
            unsnoozed_at: 4,
            return_to: "archive".to_string(),
        })
        .unwrap();

    let mut updated = msg.clone();
    updated.subject = "Updated draft".to_string();
    let new_attachment = Attachment {
        id: new_id(),
        message_id: updated.id.clone(),
        filename: "new.pdf".to_string(),
        mime_type: "application/pdf".to_string(),
        size: 20,
        local_path: Some("C:\\tmp\\new.pdf".to_string()),
        content_id: None,
        is_inline: false,
    };
    store
        .replace_message_with_attachments(
            &updated,
            std::slice::from_ref(&archive.id),
            &[new_attachment],
        )
        .unwrap();

    let fetched = store.get_message(&updated.id).unwrap().unwrap();
    assert_eq!(fetched.subject, "Updated draft");
    assert!(fetched.has_attachments);

    let attachments = store.list_attachments_by_message(&updated.id).unwrap();
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].filename, "new.pdf");
    assert_eq!(
        store.get_message_folder_ids(&updated.id).unwrap(),
        vec![archive.id]
    );
    assert_eq!(store.get_message_labels(&updated.id).unwrap().len(), 1);
    assert_eq!(store.list_kanban_cards(None).unwrap().len(), 1);
    assert!(store.get_snoozed_message(&updated.id).unwrap().is_some());
}

#[test]
fn replace_message_with_attachments_rolls_back_all_state_on_insert_failure() {
    let store = Store::open_in_memory().unwrap();
    let account = make_account();
    store.insert_account(&account).unwrap();
    let drafts = make_folder(&account.id, "Drafts", FolderRole::Drafts, 0);
    let archive = make_folder(&account.id, "Archive", FolderRole::Archive, 1);
    store.insert_folder(&drafts).unwrap();
    store.insert_folder(&archive).unwrap();

    let mut original = make_message(&account.id, "draft-original");
    original.subject = "Original".to_string();
    let old_attachment = Attachment {
        id: new_id(),
        message_id: original.id.clone(),
        filename: "old.pdf".to_string(),
        mime_type: "application/pdf".to_string(),
        size: 10,
        local_path: Some("C:\\tmp\\old.pdf".to_string()),
        content_id: None,
        is_inline: false,
    };
    store
        .replace_message_with_attachments(
            &original,
            std::slice::from_ref(&drafts.id),
            std::slice::from_ref(&old_attachment),
        )
        .unwrap();
    store.add_label(&original.id, "Keep").unwrap();
    store
        .upsert_kanban_card(&KanbanCard {
            message_id: original.id.clone(),
            column: KanbanColumn::Todo,
            position: 1,
            created_at: 1,
            updated_at: 1,
        })
        .unwrap();
    store
        .snooze_message(&SnoozedMessage {
            message_id: original.id.clone(),
            snoozed_at: 1,
            unsnoozed_at: 2,
            return_to: "inbox".to_string(),
        })
        .unwrap();

    let blocker = make_message(&account.id, "blocker");
    let duplicate_id = new_id();
    store
        .replace_message_with_attachments(
            &blocker,
            std::slice::from_ref(&archive.id),
            &[Attachment {
                id: duplicate_id.clone(),
                message_id: blocker.id.clone(),
                filename: "blocker.pdf".to_string(),
                mime_type: "application/pdf".to_string(),
                size: 1,
                local_path: None,
                content_id: None,
                is_inline: false,
            }],
        )
        .unwrap();

    let mut attempted = original.clone();
    attempted.subject = "Must roll back".to_string();
    let failure = store.replace_message_with_attachments(
        &attempted,
        std::slice::from_ref(&archive.id),
        &[Attachment {
            id: duplicate_id,
            message_id: attempted.id.clone(),
            filename: "new.pdf".to_string(),
            mime_type: "application/pdf".to_string(),
            size: 20,
            local_path: None,
            content_id: None,
            is_inline: false,
        }],
    );
    assert!(failure.is_err());

    assert_eq!(
        store.get_message(&original.id).unwrap().unwrap().subject,
        "Original"
    );
    assert_eq!(
        store.get_message_folder_ids(&original.id).unwrap(),
        vec![drafts.id]
    );
    let attachments = store.list_attachments_by_message(&original.id).unwrap();
    assert_eq!(attachments.len(), 1);
    assert_eq!(attachments[0].id, old_attachment.id);
    assert_eq!(store.get_message_labels(&original.id).unwrap().len(), 1);
    assert_eq!(store.list_kanban_cards(None).unwrap().len(), 1);
    assert!(store.get_snoozed_message(&original.id).unwrap().is_some());
}

#[test]
fn prepared_send_rolls_back_message_and_attachments_when_op_insert_fails() {
    let store = Store::open_in_memory().unwrap();
    let account = make_account();
    store.insert_account(&account).unwrap();
    let outbox = make_folder(&account.id, "Outbox", FolderRole::Inbox, 0);
    store.insert_folder(&outbox).unwrap();
    let mut message = make_message(&account.id, "local-outbox-rollback");
    message.has_attachments = true;
    let attachment = Attachment {
        id: new_id(),
        message_id: message.id.clone(),
        filename: "evidence.txt".to_string(),
        mime_type: "text/plain".to_string(),
        size: 8,
        local_path: Some("C:\\tmp\\evidence.txt".to_string()),
        content_id: None,
        is_inline: false,
    };
    store
        .with_write(|conn| {
            conn.execute_batch(
                "CREATE TRIGGER fail_prepared_send_op
                     BEFORE INSERT ON pending_mail_ops
                     WHEN NEW.op_type = 'send'
                     BEGIN
                         SELECT RAISE(ABORT, 'injected pending-op failure');
                     END;",
            )?;
            Ok(())
        })
        .unwrap();

    let result = store.prepare_outgoing_send(
        &message,
        std::slice::from_ref(&outbox.id),
        std::slice::from_ref(&attachment),
        r#"{"op":"send","payload":{}}"#,
    );

    assert!(result.is_err());
    assert!(store.get_message(&message.id).unwrap().is_none());
    assert!(store
        .list_attachments_by_message(&message.id)
        .unwrap()
        .is_empty());
    assert!(store.list_pending_mail_ops(&account.id).unwrap().is_empty());
}
