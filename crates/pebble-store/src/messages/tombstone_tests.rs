use crate::Store;
use pebble_core::*;

fn setup_store_with_message(is_deleted: bool, deleted_at: Option<i64>) -> (Store, String) {
    let store = Store::open_in_memory().unwrap();
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
    let msg = Message {
        id: new_id(),
        account_id: account.id.clone(),
        remote_id: "999".to_string(),
        message_id_header: None,
        in_reply_to: None,
        references_header: None,
        thread_id: None,
        subject: "Test".to_string(),
        snippet: "test".to_string(),
        from_address: "a@b.com".to_string(),
        from_name: "A".to_string(),
        to_list: vec![],
        cc_list: vec![],
        bcc_list: vec![],
        body_text: "body".to_string(),
        body_html_raw: "<p>body</p>".to_string(),
        has_attachments: false,
        is_read: false,
        is_starred: false,
        is_draft: false,
        date: now,
        remote_version: None,
        is_deleted,
        deleted_at,
        created_at: now,
        updated_at: now,
    };
    store
        .insert_message(&msg, std::slice::from_ref(&folder.id))
        .unwrap();
    (store, msg.id)
}

#[test]
fn test_purge_old_tombstone() {
    let thirty_one_days_ago = pebble_core::now_timestamp() - (31 * 24 * 3600);
    let (store, msg_id) = setup_store_with_message(true, Some(thirty_one_days_ago));
    let purged = store.purge_old_tombstones(30 * 24 * 3600).unwrap();
    assert_eq!(purged, 1);
    // Verify message is physically gone
    let fetched = store.get_message(&msg_id).unwrap();
    assert!(fetched.is_none());
}

#[test]
fn test_recent_tombstone_not_purged() {
    let one_day_ago = pebble_core::now_timestamp() - (24 * 3600);
    let (store, msg_id) = setup_store_with_message(true, Some(one_day_ago));
    let purged = store.purge_old_tombstones(30 * 24 * 3600).unwrap();
    assert_eq!(purged, 0);
    let fetched = store.get_message(&msg_id).unwrap();
    assert!(fetched.is_some());
}

#[test]
fn test_non_deleted_message_not_purged() {
    let (store, msg_id) = setup_store_with_message(false, None);
    let purged = store.purge_old_tombstones(30 * 24 * 3600).unwrap();
    assert_eq!(purged, 0);
    let fetched = store.get_message(&msg_id).unwrap();
    assert!(fetched.is_some());
}
