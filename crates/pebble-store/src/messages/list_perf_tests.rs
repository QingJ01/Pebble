use super::{
    insert_message_row, MessageCursor, FOLDER_PAGE_AFTER_SQL, FOLDER_PAGE_SQL,
    FOLDER_UNREAD_PAGE_AFTER_SQL, FOLDER_UNREAD_PAGE_SQL,
};
use crate::Store;
use pebble_core::*;
use rusqlite::params;

const MESSAGES: usize = 20_000;

fn account(id: &str) -> Account {
    Account {
        account_label: None,
        provider_display_name: None,
        id: id.to_string(),
        email: format!("{id}@example.com"),
        display_name: id.to_string(),
        color: None,
        provider: ProviderType::Imap,
        created_at: 1,
        updated_at: 1,
    }
}

fn folder(id: &str, account_id: &str) -> Folder {
    Folder {
        id: id.to_string(),
        account_id: account_id.to_string(),
        remote_id: id.to_string(),
        name: id.to_string(),
        folder_type: FolderType::Folder,
        role: None,
        parent_id: None,
        color: None,
        is_system: false,
        sort_order: 0,
    }
}

fn message(id: String, account_id: &str, thread: Option<String>, date: i64) -> Message {
    Message {
        id,
        account_id: account_id.to_string(),
        remote_id: String::new(),
        message_id_header: None,
        in_reply_to: None,
        references_header: None,
        thread_id: thread,
        subject: format!("subject {date}"),
        snippet: format!("snippet {date}"),
        from_address: format!("sender{}@example.com", date % 7),
        from_name: String::new(),
        to_list: vec![],
        cc_list: vec![],
        bcc_list: vec![],
        body_text: "x".repeat(6000),
        body_html_raw: String::new(),
        has_attachments: false,
        is_read: date % 3 == 0,
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

/// 20k messages in "inbox" (dates collide in pairs to exercise the
/// id tie-break), every 4th also in "archive", 3 messages per thread.
fn big_store() -> Store {
    let store = Store::open_in_memory().unwrap();
    store.insert_account(&account("acct")).unwrap();
    store.insert_folder(&folder("inbox", "acct")).unwrap();
    store.insert_folder(&folder("archive", "acct")).unwrap();
    store
            .with_write(|conn| {
                let tx = conn.unchecked_transaction()?;
                for i in 0..MESSAGES {
                    let msg = message(
                        format!("m{i:05}"),
                        "acct",
                        Some(format!("t{:05}", i / 3)),
                        (i / 2) as i64,
                    );
                    insert_message_row(&tx, &msg, "[]", "[]", "[]")?;
                    tx.execute(
                        "INSERT INTO message_folders (message_id, folder_id) VALUES (?1, 'inbox')",
                        params![msg.id],
                    )?;
                    if i % 4 == 0 {
                        tx.execute(
                            "INSERT INTO message_folders (message_id, folder_id) VALUES (?1, 'archive')",
                            params![msg.id],
                        )?;
                    }
                }
                tx.commit()?;
                Ok(())
            })
            .unwrap();
    store
}

fn plan(store: &Store, sql: &str, args: &[&dyn rusqlite::ToSql]) -> String {
    store
        .with_read(|conn| {
            let mut stmt = conn.prepare(&format!("EXPLAIN QUERY PLAN {sql}"))?;
            let rows = stmt.query_map(args, |row| row.get::<_, String>(3))?;
            Ok(rows.collect::<rusqlite::Result<Vec<_>>>()?.join(" | "))
        })
        .unwrap()
}

#[test]
fn folder_page_queries_use_the_date_index_without_sorting() {
    let store = big_store();
    let checks: Vec<(&str, String, &str)> = vec![
            (
                "first page",
                plan(&store, FOLDER_PAGE_SQL, &[&"inbox", &50, &0]),
                "idx_mf_folder_date",
            ),
            (
                "keyset page",
                plan(
                    &store,
                    FOLDER_PAGE_AFTER_SQL,
                    &[&"inbox", &50, &0, &5000i64, &"m10000"],
                ),
                "idx_mf_folder_date",
            ),
            (
                "unread first page",
                plan(&store, FOLDER_UNREAD_PAGE_SQL, &[&"inbox", &50, &0]),
                "idx_mf_folder_unread_date",
            ),
            (
                "unread keyset page",
                plan(
                    &store,
                    FOLDER_UNREAD_PAGE_AFTER_SQL,
                    &[&"inbox", &50, &0, &5000i64, &"m10000"],
                ),
                "idx_mf_folder_unread_date",
            ),
            (
                "thread walk",
                plan(
                    &store,
                    "SELECT m.account_id, mf.thread_key, mf.date
                     FROM message_folders mf JOIN messages m ON m.id = mf.message_id
                     WHERE mf.folder_id = ?1 AND mf.is_deleted = 0
                     ORDER BY mf.date DESC, mf.message_id DESC",
                    &[&"inbox"],
                ),
                "idx_mf_folder_date",
            ),
            (
                "thread aggregate",
                plan(
                    &store,
                    "SELECT m.account_id, mf.thread_key, m.id
                     FROM message_folders mf JOIN messages m ON m.id = mf.message_id
                     WHERE mf.folder_id IN (?1) AND mf.thread_key IN (?2, ?3)
                       AND mf.is_deleted = 0",
                    &[&"inbox", &"t00001", &"t00002"],
                ),
                "idx_mf_folder_thread",
            ),
            (
                "unread counts",
                plan(
                    &store,
                    "SELECT mf.folder_id, COUNT(*) FROM folders f
                     JOIN message_folders mf
                       ON mf.folder_id = f.id AND mf.is_read = 0 AND mf.is_deleted = 0
                     WHERE f.account_id = ?1 GROUP BY mf.folder_id",
                    &[&"acct"],
                ),
                "idx_mf_folder_unread",
            ),
            (
                "tombstone purge",
                plan(
                    &store,
                    "DELETE FROM messages WHERE is_deleted = 1 AND deleted_at IS NOT NULL AND deleted_at < ?1",
                    &[&10i64],
                ),
                "idx_messages_tombstones",
            ),
        ];
    for (name, plan, index) in checks {
        eprintln!("{name}: {plan}");
        assert!(plan.contains(index), "{name} must use {index}: {plan}");
        assert!(
            !plan.contains("TEMP B-TREE FOR ORDER BY"),
            "{name} must not sort: {plan}"
        );
        assert!(
            !plan.contains("SCAN mf") && !plan.contains("SCAN message_folders"),
            "{name} must not scan the folder table: {plan}"
        );
    }
}

#[test]
fn keyset_pages_match_offset_pages_and_cover_the_folder_once() {
    let store = big_store();
    let mut cursor: Option<MessageCursor> = None;
    let mut seen = Vec::new();
    loop {
        let page = store
            .list_messages_page(&["inbox".to_string()], cursor.as_ref(), 997, 0)
            .unwrap();
        if page.is_empty() {
            break;
        }
        if seen.is_empty() {
            let offset_page = store.list_messages_by_folder("inbox", 997, 0).unwrap();
            assert_eq!(
                offset_page.iter().map(|m| &m.id).collect::<Vec<_>>(),
                page.iter().map(|m| &m.id).collect::<Vec<_>>()
            );
        }
        let last = page.last().unwrap();
        cursor = Some(MessageCursor {
            date: last.date,
            id: last.id.clone(),
        });
        seen.extend(page.into_iter().map(|m| (m.date, m.id)));
    }
    assert_eq!(seen.len(), MESSAGES);
    let mut expected = seen.clone();
    expected.sort_by(|a, b| b.cmp(a));
    assert_eq!(seen, expected, "pages must be strictly newest-first");

    // Multi-folder pages deduplicate messages present in both folders.
    let both = store
        .list_messages_page(&["inbox".to_string(), "archive".to_string()], None, 10, 0)
        .unwrap();
    let ids: Vec<_> = both.iter().map(|m| m.id.as_str()).collect();
    assert_eq!(
        ids,
        vec![
            "m19999", "m19998", "m19997", "m19996", "m19995", "m19994", "m19993", "m19992",
            "m19991", "m19990"
        ]
    );
}

#[test]
fn thread_pages_and_unread_counts_follow_flag_changes() {
    let store = big_store();
    let page = store.list_threads_by_folder("inbox", 3, 0).unwrap();
    assert_eq!(
        page.iter()
            .map(|t| t.thread_id.as_str())
            .collect::<Vec<_>>(),
        vec!["t06666", "t06665", "t06664"]
    );
    assert_eq!(page[0].message_count, 2); // m19998, m19999
    assert_eq!(page[1].message_count, 3);
    assert!(page.iter().all(|t| t.account_id == "acct"));
    let second = store.list_threads_by_folder("inbox", 3, 3).unwrap();
    assert_eq!(second[0].thread_id, "t06663");

    let before = store.get_folder_unread_counts("acct").unwrap()["inbox"];
    // m19996/m19997 (date 9998) start unread.
    store
        .update_message_flags("m19996", Some(true), None)
        .unwrap();
    store
        .bulk_update_flags(&[("m19997".to_string(), Some(true), None)])
        .unwrap();
    let after = store.get_folder_unread_counts("acct").unwrap()["inbox"];
    assert_eq!(after, before - 2);
    store.bulk_soft_delete(&["m19998".to_string()]).unwrap();
    let page = store.list_messages_by_folder("inbox", 3, 0).unwrap();
    assert_eq!(
        page.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
        vec!["m19999", "m19997", "m19996"]
    );
    assert!(page[1].is_read);
    // Bodies are still returned for full reads.
    let full = store.get_message("m00001").unwrap().unwrap();
    assert_eq!(full.body_text.len(), 6000);
}

#[test]
fn thread_messages_are_scoped_to_one_account() {
    let store = Store::open_in_memory().unwrap();
    for id in ["a1", "a2"] {
        store.insert_account(&account(id)).unwrap();
        store
            .insert_folder(&folder(&format!("{id}-inbox"), id))
            .unwrap();
    }
    // The same conversation delivered to both local accounts.
    for (idx, acct) in ["a1", "a2"].iter().enumerate() {
        for n in 0..2 {
            let msg = message(
                format!("{acct}-m{n}"),
                acct,
                Some("shared-thread".to_string()),
                10 + n as i64 + idx as i64,
            );
            store
                .insert_message(&msg, &[format!("{acct}-inbox")])
                .unwrap();
        }
    }
    let a1 = store
        .list_messages_by_thread(Some("a1"), "shared-thread")
        .unwrap();
    assert_eq!(a1.len(), 2);
    assert!(a1.iter().all(|m| m.account_id == "a1"));
    assert_eq!(
        store
            .list_messages_by_thread(None, "shared-thread")
            .unwrap()
            .len(),
        4
    );

    let unified = store
        .list_threads_by_folders(&["a1-inbox".to_string(), "a2-inbox".to_string()], 50, 0)
        .unwrap();
    assert_eq!(unified.len(), 2, "one thread per account");
    assert_eq!(unified[0].account_id, "a2");
    assert_eq!(unified[1].account_id, "a1");
    assert!(unified.iter().all(|t| t.message_count == 2));
}

#[test]
fn rowid_walk_visits_every_live_message_once() {
    let store = big_store();
    store.bulk_soft_delete(&["m00010".to_string()]).unwrap();
    let mut after = 0i64;
    let mut total = 0usize;
    loop {
        let batch = store.list_full_messages_after_rowid(after, 1500).unwrap();
        let Some((last, _)) = batch.last() else { break };
        after = *last;
        assert!(batch.iter().all(|(_, m)| m.body_text.len() == 6000));
        total += batch.len();
    }
    assert_eq!(total, MESSAGES - 1);
}
