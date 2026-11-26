use crate::Store;

#[test]
fn large_in_batches_are_chunked_below_sqlite_variable_limit() {
    let store = Store::open_in_memory().unwrap();
    let ids: Vec<String> = (0..33_000).map(|i| i.to_string()).collect();

    assert!(store.get_message_folder_ids_batch(&ids).unwrap().is_empty());
    assert!(store.get_messages_batch(&ids).unwrap().is_empty());
    assert!(store
        .get_existing_remote_ids("account", &ids)
        .unwrap()
        .is_empty());
    assert!(store
        .get_existing_remote_ids_in_folder("account", "folder", &ids)
        .unwrap()
        .is_empty());
    assert!(store
        .get_existing_message_map_by_remote_ids("account", &ids)
        .unwrap()
        .is_empty());
    assert!(store
        .get_incomplete_attachment_message_map_by_remote_ids("account", None, &ids)
        .unwrap()
        .is_empty());
    store.bulk_soft_delete(&ids).unwrap();
}

#[test]
fn bulk_soft_delete_rolls_back_all_chunks_when_a_later_chunk_fails() {
    let store = Store::open_in_memory().unwrap();
    let ids: Vec<String> = (0..501).map(|i| format!("message-{i}")).collect();
    store
        .with_write(|conn| {
            conn.execute(
                "INSERT INTO accounts (id, email, display_name, provider, created_at, updated_at)
                 VALUES ('account', 'a@example.com', 'A', 'imap', 0, 0)",
                [],
            )?;
            for id in &ids {
                conn.execute(
                    "INSERT INTO messages (id, account_id, remote_id, date, created_at, updated_at)
                     VALUES (?1, 'account', ?1, 0, 0, 0)",
                    [id],
                )?;
            }
            conn.execute_batch(
                "CREATE TRIGGER fail_later_delete_chunk BEFORE UPDATE ON messages
                 WHEN NEW.id = 'message-500'
                 BEGIN SELECT RAISE(ABORT, 'forced delete failure'); END;",
            )?;
            Ok(())
        })
        .unwrap();

    assert!(store.bulk_soft_delete(&ids).is_err());
    let deleted_count = store
        .with_read(|conn| {
            Ok(conn.query_row(
                "SELECT COUNT(*) FROM messages WHERE is_deleted = 1",
                [],
                |row| row.get::<_, i64>(0),
            )?)
        })
        .unwrap();
    assert_eq!(deleted_count, 0);
}

#[test]
fn thread_ids_in_folders_are_scoped_to_selected_folder() {
    let store = Store::open_in_memory().unwrap();
    store.with_write(|conn| {
        conn.execute_batch(
            "INSERT INTO accounts (id, email, display_name, provider, created_at, updated_at) VALUES
             ('a1','a1@example.com','A1','imap',0,0),('a2','a2@example.com','A2','imap',0,0);
             INSERT INTO folders (id, account_id, remote_id, name, folder_type, is_system, sort_order) VALUES
             ('f1','a1','INBOX','Inbox','folder',1,0),('f2','a2','INBOX','Inbox','folder',1,0);
             INSERT INTO messages (id, account_id, remote_id, thread_id, date, created_at, updated_at) VALUES
             ('m1','a1','m1','t',0,0,0),('m2','a2','m2','t',0,0,0);
             INSERT INTO message_folders (message_id, folder_id) VALUES ('m1','f1'),('m2','f2');"
        )?;
        Ok(())
    }).unwrap();
    assert_eq!(
        store
            .list_message_ids_by_thread_in_folders("t", &["f1".into()])
            .unwrap(),
        vec!["m1"]
    );
    assert_eq!(
        store
            .list_message_ids_by_thread_in_folders("t", &["f2".into()])
            .unwrap(),
        vec!["m2"]
    );
}

#[test]
fn duplicate_ids_across_chunks_do_not_duplicate_folder_memberships() {
    let store = Store::open_in_memory().unwrap();
    let target = "target-message".to_string();
    let mut ids = vec![target.clone()];
    for index in 1..500 {
        ids.push(format!("filler-{index}"));
    }
    ids.push(target.clone());

    store
        .with_write(|conn| {
            conn.execute(
                "INSERT INTO accounts (id, email, display_name, provider, created_at, updated_at)
                 VALUES ('account', 'a@example.com', 'A', 'imap', 0, 0)",
                [],
            )?;
            conn.execute(
                "INSERT INTO folders (id, account_id, remote_id, name, folder_type, is_system, sort_order)
                 VALUES ('folder-1', 'account', 'INBOX', 'Inbox', 'folder', 1, 0)",
                [],
            )?;
            conn.execute(
                "INSERT INTO messages (id, account_id, remote_id, date, created_at, updated_at)
                 VALUES (?1, 'account', ?1, 0, 0, 0)",
                [&target],
            )?;
            conn.execute(
                "INSERT INTO message_folders (message_id, folder_id) VALUES (?1, 'folder-1')",
                [&target],
            )?;
            Ok(())
        })
        .unwrap();

    let result = store.get_message_folder_ids_batch(&ids).unwrap();
    assert_eq!(result.get(&target).map(Vec::len), Some(1));
}
