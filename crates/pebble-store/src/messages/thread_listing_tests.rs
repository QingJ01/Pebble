use crate::Store;
use pebble_core::*;

#[test]
fn local_drafts_without_thread_ids_are_independent_threads_in_single_and_multi_folder_views() {
    let store = Store::open_in_memory().unwrap();
    let (account, folder) = seed_account_and_folder(&store);
    let (_, other_folder) = seed_account_and_folder(&store);
    let mut first = make_msg(&account, "unused", "sender@example.com", 20);
    first.thread_id = None;
    first.remote_id.clear();
    first.is_draft = true;
    first.in_reply_to = Some("<parent@example.com>".into());
    let mut second = first.clone();
    second.id = new_id();
    second.date = 10;
    second.snippet = "Second draft".into();
    store
        .insert_message(&first, &[folder.clone(), other_folder.clone()])
        .unwrap();
    store
        .insert_message(&second, std::slice::from_ref(&folder))
        .unwrap();
    let single = store.list_threads_by_folder(&folder, 50, 0).unwrap();
    assert_eq!(single.len(), 2);
    assert_eq!(single[0].thread_id, first.id);
    assert_eq!(single[0].message_count, 1);
    assert_eq!(single[1].thread_id, second.id);
    let multi = store
        .list_threads_by_folders(&[folder.clone(), other_folder], 50, 0)
        .unwrap();
    assert_eq!(multi.len(), 2);
    assert_eq!(multi[0].message_count, 1);
    let opened = store
        .list_messages_by_thread(Some(&account), &first.id)
        .unwrap();
    assert_eq!(opened.len(), 1);
    assert_eq!(opened[0].in_reply_to, first.in_reply_to);
    assert!(opened[0].thread_id.is_none());
    assert_eq!(
        store.list_threads_by_folder(&folder, 1, 1).unwrap()[0].thread_id,
        second.id
    );
    store
        .hard_delete_messages(std::slice::from_ref(&first.id))
        .unwrap();
    assert!(store
        .list_messages_by_thread(Some(&account), &first.id)
        .unwrap()
        .is_empty());
    assert_eq!(
        store.list_threads_by_folder(&folder, 50, 0).unwrap().len(),
        1
    );
}

fn seed_account_and_folder(store: &Store) -> (String, String) {
    let now = now_timestamp();
    let account = Account {
        account_label: None,
        provider_display_name: None,
        id: new_id(),
        email: "test@example.com".to_string(),
        display_name: "Test".to_string(),
        color: None,
        provider: ProviderType::Imap,
        created_at: now,
        updated_at: now,
    };
    store.insert_account(&account).unwrap();
    let folder = Folder {
        id: new_id(),
        account_id: account.id.clone(),
        remote_id: "INBOX".to_string(),
        name: "Inbox".to_string(),
        folder_type: FolderType::Folder,
        role: Some(FolderRole::Inbox),
        parent_id: None,
        color: None,
        is_system: true,
        sort_order: 0,
    };
    store.insert_folder(&folder).unwrap();
    (account.id, folder.id)
}

fn make_msg(account_id: &str, thread_id: &str, from: &str, date: i64) -> Message {
    Message {
        id: new_id(),
        account_id: account_id.to_string(),
        remote_id: new_id(),
        message_id_header: None,
        in_reply_to: None,
        references_header: None,
        thread_id: Some(thread_id.to_string()),
        subject: "Thread subject".to_string(),
        snippet: format!("snippet-{date}"),
        from_address: from.to_string(),
        from_name: String::new(),
        to_list: vec![],
        cc_list: vec![],
        bcc_list: vec![],
        body_text: "body".to_string(),
        body_html_raw: String::new(),
        has_attachments: false,
        is_read: false,
        is_starred: false,
        is_draft: false,
        date,
        remote_version: None,
        is_deleted: false,
        deleted_at: None,
        created_at: date,
        updated_at: date,
    }
}

#[test]
fn list_threads_aggregates_distinct_participants() {
    let store = Store::open_in_memory().unwrap();
    let (account_id, folder_id) = seed_account_and_folder(&store);
    let thread_id = new_id();

    let base = now_timestamp();
    // Three messages in same thread, two distinct senders (alice appears twice).
    let m1 = make_msg(&account_id, &thread_id, "alice@example.com", base - 200);
    let m2 = make_msg(&account_id, &thread_id, "bob@example.com", base - 100);
    let m3 = make_msg(&account_id, &thread_id, "alice@example.com", base);
    store
        .insert_message(&m1, std::slice::from_ref(&folder_id))
        .unwrap();
    store
        .insert_message(&m2, std::slice::from_ref(&folder_id))
        .unwrap();
    store
        .insert_message(&m3, std::slice::from_ref(&folder_id))
        .unwrap();

    let threads = store
        .list_threads_by_folder(&folder_id, 50, 0)
        .expect("list_threads_by_folder should succeed with distinct participants");
    assert_eq!(threads.len(), 1);
    let t = &threads[0];
    assert_eq!(t.message_count, 3);
    assert_eq!(t.unread_count, 3);
    let mut parts = t.participants.clone();
    parts.sort();
    assert_eq!(
        parts,
        vec![
            "alice@example.com".to_string(),
            "bob@example.com".to_string()
        ]
    );
}

#[test]
fn list_threads_by_folders_counts_messages_once_across_selected_folders() {
    let store = Store::open_in_memory().unwrap();
    let (account_id, inbox_id) = seed_account_and_folder(&store);
    let archive = Folder {
        id: new_id(),
        account_id: account_id.clone(),
        remote_id: "Archive".to_string(),
        name: "Archive".to_string(),
        folder_type: FolderType::Folder,
        role: Some(FolderRole::Archive),
        parent_id: None,
        color: None,
        is_system: true,
        sort_order: 1,
    };
    store.insert_folder(&archive).unwrap();

    let thread_id = new_id();
    let base = now_timestamp();
    let m1 = make_msg(&account_id, &thread_id, "alice@example.com", base - 60);
    let m2 = make_msg(&account_id, &thread_id, "bob@example.com", base);
    let m1_folder_ids = [inbox_id.clone(), archive.id.clone()];
    store.insert_message(&m1, &m1_folder_ids).unwrap();
    store
        .insert_message(&m2, std::slice::from_ref(&archive.id))
        .unwrap();

    let threads = store
        .list_threads_by_folders(&[inbox_id, archive.id], 50, 0)
        .expect("list_threads_by_folders should aggregate selected folders");

    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].message_count, 2);
    assert_eq!(threads[0].snippet, format!("snippet-{base}"));
}

#[test]
fn thread_subject_comes_from_the_earliest_message_not_the_largest_string() {
    let store = Store::open_in_memory().unwrap();
    let (account, folder) = seed_account_and_folder(&store);
    let mut original = make_msg(&account, "t-subject", "a@example.com", 10);
    original.subject = "Budget".into();
    let mut reply = make_msg(&account, "t-subject", "b@example.com", 20);
    reply.subject = "Re: Zebra crossing (was: Budget)".into();
    let mut later = make_msg(&account, "t-subject", "c@example.com", 30);
    later.subject = "RE: Budget".into();
    for msg in [&later, &reply, &original] {
        store
            .insert_message(msg, std::slice::from_ref(&folder))
            .unwrap();
    }

    for threads in [
        store.list_threads_by_folder(&folder, 50, 0).unwrap(),
        store
            .list_threads_by_folders(std::slice::from_ref(&folder), 50, 0)
            .unwrap(),
    ] {
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].subject, "Budget");
    }
}

#[test]
fn thread_subject_strips_reply_prefix_when_the_original_is_not_in_view() {
    let store = Store::open_in_memory().unwrap();
    let (account, folder) = seed_account_and_folder(&store);
    let mut reply = make_msg(&account, "t-reply", "b@example.com", 20);
    reply.subject = "Re: 回复：Quarterly plan".into();
    store
        .insert_message(&reply, std::slice::from_ref(&folder))
        .unwrap();

    let threads = store.list_threads_by_folder(&folder, 50, 0).unwrap();
    assert_eq!(threads[0].subject, "Quarterly plan");
}

#[test]
fn list_threads_shows_sender_names_once_per_address() {
    let store = Store::open_in_memory().unwrap();
    let (account_id, folder_id) = seed_account_and_folder(&store);
    let thread_id = new_id();
    let base = now_timestamp();

    let mut m1 = make_msg(&account_id, &thread_id, "dave@example.test", base - 200);
    m1.from_name = "Dave Second".to_string();
    // Same sender, different case, no name: still one participant.
    let m2 = make_msg(&account_id, &thread_id, "Dave@Example.test", base - 100);
    // No display name: falls back to the address.
    let m3 = make_msg(&account_id, &thread_id, "carol@example.test", base);
    for m in [&m1, &m2, &m3] {
        store
            .insert_message(m, std::slice::from_ref(&folder_id))
            .unwrap();
    }

    let threads = store.list_threads_by_folder(&folder_id, 50, 0).unwrap();
    assert_eq!(
        threads[0].participants,
        vec!["Dave Second".to_string(), "carol@example.test".to_string()]
    );
}

#[test]
fn unread_only_listing_keeps_unread_messages_and_whole_unread_threads() {
    let store = Store::open_in_memory().unwrap();
    let (account, folder) = seed_account_and_folder(&store);
    let mut read_old = make_msg(&account, "t-mixed", "a@example.com", 10);
    read_old.is_read = true;
    let unread_new = make_msg(&account, "t-mixed", "b@example.com", 20);
    let mut read_only = make_msg(&account, "t-read", "c@example.com", 30);
    read_only.is_read = true;
    for msg in [&read_old, &unread_new, &read_only] {
        store
            .insert_message(msg, std::slice::from_ref(&folder))
            .unwrap();
    }
    let folders = std::slice::from_ref(&folder);

    let all = store
        .list_messages_page_filtered(folders, None, 50, 0, false)
        .unwrap();
    assert_eq!(all.len(), 3);
    let unread = store
        .list_messages_page_filtered(folders, None, 50, 0, true)
        .unwrap();
    assert_eq!(
        unread.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec![unread_new.id.as_str()]
    );

    let threads = store
        .list_threads_by_folders_filtered(folders, 50, 0, true)
        .unwrap();
    assert_eq!(threads.len(), 1);
    assert_eq!(threads[0].thread_id, "t-mixed");
    // The matching thread is still shown whole, read messages included.
    assert_eq!(threads[0].message_count, 2);
    assert_eq!(threads[0].unread_count, 1);

    // Reading the last unread message drops it from the unread views.
    store
        .update_message_flags(&unread_new.id, Some(true), None)
        .unwrap();
    assert!(store
        .list_messages_page_filtered(folders, None, 50, 0, true)
        .unwrap()
        .is_empty());
    assert!(store
        .list_threads_by_folders_filtered(folders, 50, 0, true)
        .unwrap()
        .is_empty());
}

#[test]
fn unread_threads_rank_by_newest_message_not_newest_unread() {
    let store = Store::open_in_memory().unwrap();
    let (account, folder) = seed_account_and_folder(&store);
    // Thread A: unread on day 1, read reply on day 5.
    let a_unread = make_msg(&account, "t-a", "a@example.com", 100);
    let mut a_reply = make_msg(&account, "t-a", "b@example.com", 500);
    a_reply.is_read = true;
    // Thread B: unread on day 3.
    let b_unread = make_msg(&account, "t-b", "c@example.com", 300);
    for msg in [&a_unread, &a_reply, &b_unread] {
        store
            .insert_message(msg, std::slice::from_ref(&folder))
            .unwrap();
    }

    let threads = store
        .list_threads_by_folders_filtered(std::slice::from_ref(&folder), 50, 0, true)
        .unwrap();
    let order: Vec<_> = threads.iter().map(|t| t.thread_id.as_str()).collect();
    assert_eq!(order, vec!["t-a", "t-b"]);
    assert_eq!(threads[0].last_date, 500);
    assert_eq!(
        store
            .list_threads_by_folders_filtered(std::slice::from_ref(&folder), 1, 1, true)
            .unwrap()[0]
            .thread_id,
        "t-b"
    );
}
