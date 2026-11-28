use super::*;
use pebble_core::{Account, EmailAddress, Folder, FolderType, ProviderType};

fn account(id: &str, email: &str) -> Account {
    Account {
        id: id.into(),
        email: email.into(),
        display_name: "Me".into(),
        color: None,
        account_label: None,
        provider_display_name: None,
        provider: ProviderType::Imap,
        created_at: 0,
        updated_at: 0,
    }
}

fn folder(id: &str, account: &str, role: FolderRole) -> Folder {
    Folder {
        id: id.into(),
        account_id: account.into(),
        remote_id: id.into(),
        name: format!("{role:?}"),
        folder_type: FolderType::Folder,
        role: Some(role),
        parent_id: None,
        color: None,
        is_system: true,
        sort_order: 0,
    }
}

#[allow(clippy::too_many_arguments)]
fn put(
    store: &Store,
    index: &TantivySearch,
    id: &str,
    account: &str,
    folder: &str,
    subject: &str,
    date: i64,
    read: bool,
) {
    let message = Message {
        id: id.into(),
        account_id: account.into(),
        remote_id: id.into(),
        message_id_header: None,
        in_reply_to: None,
        references_header: None,
        thread_id: Some(format!("t-{id}")),
        subject: subject.into(),
        snippet: subject.into(),
        from_address: "dana@partner.example".into(),
        from_name: "Dana".into(),
        to_list: vec![EmailAddress {
            name: None,
            address: "me@work.example".into(),
        }],
        cc_list: vec![],
        bcc_list: vec![],
        body_text: format!("{subject} body"),
        body_html_raw: String::new(),
        has_attachments: false,
        is_read: read,
        is_starred: false,
        is_draft: false,
        date,
        remote_version: None,
        is_deleted: false,
        deleted_at: None,
        created_at: 0,
        updated_at: 0,
    };
    store
        .insert_message(&message, &[folder.to_string()])
        .unwrap();
    index
        .index_message(&message, &[folder.to_string()])
        .unwrap();
}

fn fixture() -> (Store, TantivySearch) {
    let store = Store::open_in_memory().unwrap();
    let index = TantivySearch::open_in_memory().unwrap();
    store
        .insert_account(&account("work", "me@work.example"))
        .unwrap();
    store
        .insert_account(&account("home", "me@home.example"))
        .unwrap();
    for (id, acct, role) in [
        ("w-in", "work", FolderRole::Inbox),
        ("w-sent", "work", FolderRole::Sent),
        ("w-spam", "work", FolderRole::Spam),
        ("w-trash", "work", FolderRole::Trash),
        ("h-in", "home", FolderRole::Inbox),
    ] {
        store.insert_folder(&folder(id, acct, role)).unwrap();
    }
    put(
        &store,
        &index,
        "a",
        "work",
        "w-in",
        "Budget plan",
        1_000,
        false,
    );
    put(
        &store,
        &index,
        "b",
        "work",
        "w-sent",
        "Budget reply",
        2_000,
        true,
    );
    put(
        &store,
        &index,
        "c",
        "work",
        "w-spam",
        "Budget offer",
        3_000,
        false,
    );
    put(
        &store,
        &index,
        "d",
        "work",
        "w-trash",
        "Budget old",
        4_000,
        true,
    );
    put(
        &store,
        &index,
        "e",
        "home",
        "h-in",
        "Budget at home",
        5_000,
        false,
    );
    // Many matches in another account must not crowd out this one.
    for i in 0..40 {
        put(
            &store,
            &index,
            &format!("x{i}"),
            "home",
            "h-in",
            "Budget noise",
            6_000 + i,
            false,
        );
    }
    index.commit().unwrap();
    (store, index)
}

fn ids(page: &Page) -> Vec<&str> {
    page.items.iter().map(|f| f.message.id.as_str()).collect()
}

#[test]
fn searches_only_the_accounts_newest_first_without_spam_or_trash() {
    let (store, index) = fixture();
    let q = MailQuery {
        query: Some("budget".into()),
        ..Default::default()
    };
    let page = search(&store, &index, &["work".to_string()], &q).unwrap();
    assert_eq!(ids(&page), vec!["b", "a"]);
    assert_eq!(page.items[0].folders, vec!["Sent".to_string()]);
}

#[test]
fn filters_by_unread_folder_and_pages_stably() {
    let (store, index) = fixture();
    let work = ["work".to_string()];
    let unread = MailQuery {
        query: Some("budget".into()),
        unread: Some(true),
        ..Default::default()
    };
    assert_eq!(
        ids(&search(&store, &index, &work, &unread).unwrap()),
        vec!["a"]
    );
    let sent = MailQuery {
        query: Some("budget".into()),
        folder: Some("sent".into()),
        ..Default::default()
    };
    assert_eq!(
        ids(&search(&store, &index, &work, &sent).unwrap()),
        vec!["b"]
    );
    assert!(search(
        &store,
        &index,
        &work,
        &MailQuery {
            query: Some("x".into()),
            folder: Some("spam".into()),
            ..Default::default()
        }
    )
    .is_err());

    let home = ["home".to_string()];
    let first = search(
        &store,
        &index,
        &home,
        &MailQuery {
            query: Some("budget".into()),
            limit: Some(10),
            ..Default::default()
        },
    )
    .unwrap();
    assert_eq!(first.items.len(), 10);
    let cursor = first.next_cursor.clone().unwrap();
    let second = search(
        &store,
        &index,
        &home,
        &MailQuery {
            query: Some("budget".into()),
            limit: Some(10),
            cursor: Some(cursor),
            ..Default::default()
        },
    )
    .unwrap();
    assert!(second
        .items
        .iter()
        .all(|f| f.message.date < first.items[9].message.date));
}

#[test]
fn needs_something_to_search_for() {
    let (store, index) = fixture();
    assert!(search(&store, &index, &["work".to_string()], &MailQuery::default()).is_err());
    assert!(search(
        &store,
        &index,
        &[],
        &MailQuery {
            query: Some("budget".into()),
            ..Default::default()
        }
    )
    .unwrap()
    .items
    .is_empty());
    let messages = store
        .get_messages_batch(&["a".into(), "c".into(), "d".into()])
        .unwrap();
    let kept: Vec<String> = outside_spam_and_trash(&store, &messages)
        .into_iter()
        .map(|m| m.id)
        .collect();
    assert_eq!(kept, vec!["a".to_string()]);
}
