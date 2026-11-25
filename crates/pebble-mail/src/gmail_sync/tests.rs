use super::*;
use pebble_core::{Account, Folder, FolderRole, FolderType, Message, ProviderType};
use std::collections::VecDeque;

fn gmail_test_message(id: &str, remote_id: &str) -> Message {
    let now = now_timestamp();
    Message {
        id: id.to_string(),
        account_id: "gmail-account".to_string(),
        remote_id: remote_id.to_string(),
        message_id_header: None,
        in_reply_to: None,
        references_header: None,
        thread_id: None,
        subject: id.to_string(),
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
        is_starred: true,
        is_draft: false,
        date: now,
        remote_version: None,
        is_deleted: false,
        deleted_at: None,
        created_at: now,
        updated_at: now,
    }
}

fn gmail_test_folder(id: &str, remote_id: &str) -> Folder {
    Folder {
        id: id.to_string(),
        account_id: "gmail-account".to_string(),
        remote_id: remote_id.to_string(),
        name: remote_id.to_string(),
        folder_type: FolderType::Label,
        role: if remote_id == "INBOX" {
            Some(FolderRole::Inbox)
        } else {
            None
        },
        parent_id: None,
        color: None,
        is_system: remote_id == "INBOX",
        sort_order: 0,
    }
}

fn gmail_test_account() -> Account {
    let now = now_timestamp();
    Account {
        account_label: None,
        provider_display_name: None,
        id: "gmail-account".to_string(),
        email: "gmail@example.com".to_string(),
        display_name: "Gmail".to_string(),
        color: None,
        provider: ProviderType::Gmail,
        created_at: now,
        updated_at: now,
    }
}

struct GmailTestServer {
    endpoint: String,
    requests: mpsc::UnboundedReceiver<String>,
    responses: mpsc::UnboundedSender<(u16, String)>,
    task: tokio::task::JoinHandle<()>,
}

impl GmailTestServer {
    async fn next_request(&mut self) -> String {
        tokio::time::timeout(std::time::Duration::from_secs(10), self.requests.recv())
            .await
            .expect("timed out waiting for Gmail request")
            .expect("Gmail test server stopped")
    }

    fn respond(&self, status: u16, body: &str) {
        self.responses
            .send((status, body.to_string()))
            .expect("Gmail test server stopped");
    }

    fn stop(self) {
        self.task.abort();
    }
}

async fn start_gmail_test_server() -> GmailTestServer {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tokio::net::TcpListener;

    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("http://{}", listener.local_addr().unwrap());
    let (request_tx, requests) = mpsc::unbounded_channel();
    let (responses, mut response_rx) = mpsc::unbounded_channel::<(u16, String)>();
    let task = tokio::spawn(async move {
        loop {
            let Ok((mut stream, _)) = listener.accept().await else {
                break;
            };
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            loop {
                let Ok(count) = stream.read(&mut buffer).await else {
                    return;
                };
                if count == 0 {
                    return;
                }
                request.extend_from_slice(&buffer[..count]);
                if request.windows(4).any(|window| window == b"\r\n\r\n") {
                    break;
                }
            }
            let request_line = String::from_utf8_lossy(&request)
                .lines()
                .next()
                .unwrap_or_default()
                .to_string();
            let request_path = request_line
                .split_whitespace()
                .nth(1)
                .unwrap_or_default()
                .to_string();
            if request_tx.send(request_path).is_err() {
                break;
            }
            let Some((status, body)) = response_rx.recv().await else {
                break;
            };
            let response = format!(
                    "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            if stream.write_all(response.as_bytes()).await.is_err() {
                break;
            }
        }
    });
    GmailTestServer {
        endpoint,
        requests,
        responses,
        task,
    }
}

fn prepare_gmail_sync_store(saved_since: i64, current_since: i64) -> Arc<Store> {
    let store = Arc::new(Store::open_in_memory().unwrap());
    store.insert_account(&gmail_test_account()).unwrap();
    for folder in [
        gmail_test_folder("gmail-inbox", "INBOX"),
        gmail_test_folder("gmail-projects", "Label_Projects"),
        gmail_test_folder("gmail-keep", "Label_Keep"),
    ] {
        store.insert_folder(&folder).unwrap();
    }
    store
        .update_sync_state("gmail-account", |state| {
            state.last_sync_cursor = Some("history-old".to_string());
            state.sync_since = Some(current_since);
            state.gmail_sync_since = Some(saved_since);
            state.has_gmail_sync_scope = true;
        })
        .unwrap();
    store
}

fn gmail_test_worker(store: Arc<Store>, endpoint: String) -> GmailSyncWorker {
    let provider = Arc::new(GmailProvider::new("test-token".to_string()));
    provider.set_test_api_endpoint(endpoint);
    let (_stop_tx, stop_rx) = watch::channel(false);
    GmailSyncWorker::new(
        "gmail-account",
        provider,
        store,
        stop_rx,
        std::env::temp_dir().join(new_id()),
    )
}

fn gmail_labels_response() -> &'static str {
    r#"{"labels":[{"id":"INBOX","name":"Inbox","type":"system"},{"id":"Label_Projects","name":"Projects","type":"user"},{"id":"Label_Keep","name":"Keep","type":"user"}]}"#
}

async fn respond_to_request(
    server: &mut GmailTestServer,
    observed: &mut Vec<String>,
    expected: &str,
    status: u16,
    body: &str,
) -> String {
    let request = server.next_request().await;
    assert!(
        request.contains(expected),
        "expected {expected:?} in request {request:?}"
    );
    observed.push(request.clone());
    server.respond(status, body);
    request
}

async fn respond_to_empty_bootstrap_labels(
    server: &mut GmailTestServer,
    observed: &mut Vec<String>,
    count: usize,
) {
    for _ in 0..count {
        respond_to_request(
            server,
            observed,
            "/messages?labelIds=",
            200,
            r#"{"messages":[]}"#,
        )
        .await;
    }
}

#[tokio::test]
async fn scope_mismatch_poll_replays_history_before_bootstrap_checkpoint() {
    let store = prepare_gmail_sync_store(100, 200);
    let inbox_id = "gmail-inbox".to_string();
    let projects_id = "gmail-projects".to_string();
    let keep_id = "gmail-keep".to_string();
    let flag_message = gmail_test_message("flag-local", "flag-message");
    let deleted_message = gmail_test_message("deleted-local", "deleted-message");
    let removed_label_message = gmail_test_message("removed-local", "removed-label");
    let added_label_message = gmail_test_message("added-local", "added-label");
    for message in [
        &flag_message,
        &deleted_message,
        &removed_label_message,
        &added_label_message,
    ] {
        store
            .insert_message(message, std::slice::from_ref(&inbox_id))
            .unwrap();
    }
    store
        .add_message_to_folder(&removed_label_message.id, &projects_id)
        .unwrap();

    let mut server = start_gmail_test_server().await;
    let worker = gmail_test_worker(Arc::clone(&store), server.endpoint.clone());
    let worker_task = tokio::spawn(async move { worker.poll_changes().await });
    let mut observed = Vec::new();

    respond_to_request(
        &mut server,
        &mut observed,
        "/labels",
        200,
        gmail_labels_response(),
    )
    .await;
    respond_to_request(
        &mut server,
        &mut observed,
        "/profile",
        200,
        r#"{"emailAddress":"gmail@example.com","historyId":"profile-checkpoint"}"#,
    )
    .await;
    respond_to_request(
            &mut server,
            &mut observed,
            "/history?startHistoryId=history-old",
            200,
            r#"{"history":[{"labelsAdded":[{"message":{"id":"flag-message"},"labelIds":["UNREAD"]},{"message":{"id":"added-label"},"labelIds":["Label_Keep"]}]},{"labelsRemoved":[{"message":{"id":"flag-message"},"labelIds":["STARRED"]},{"message":{"id":"removed-label"},"labelIds":["Label_Projects"]}]},{"messagesDeleted":[{"message":{"id":"deleted-message"}}]}],"historyId":"history-replayed"}"#,
        )
        .await;

    let bootstrap_request = server.next_request().await;
    assert!(bootstrap_request.contains("/messages?labelIds="));
    observed.push(bootstrap_request.clone());
    let state = store.get_sync_state("gmail-account").unwrap().unwrap();
    assert_eq!(state.last_sync_cursor.as_deref(), Some("history-old"));
    assert_eq!(state.gmail_sync_since, Some(100));
    assert_eq!(state.sync_since, Some(200));

    let flag = store.get_message(&flag_message.id).unwrap().unwrap();
    assert!(!flag.is_read);
    assert!(!flag.is_starred);
    assert!(
        store
            .get_message(&deleted_message.id)
            .unwrap()
            .unwrap()
            .is_deleted
    );
    let removed_folders = store
        .get_message_folder_ids(&removed_label_message.id)
        .unwrap();
    assert!(!removed_folders.contains(&projects_id));
    let added_folders = store
        .get_message_folder_ids(&added_label_message.id)
        .unwrap();
    assert!(added_folders.contains(&keep_id));

    server.respond(200, r#"{"messages":[]}"#);
    respond_to_empty_bootstrap_labels(&mut server, &mut observed, 2).await;
    worker_task.await.unwrap().unwrap();

    assert!(observed[1].contains("/profile"));
    assert!(observed[2].contains("/history?startHistoryId=history-old"));
    let state = store.get_sync_state("gmail-account").unwrap().unwrap();
    assert_eq!(
        state.last_sync_cursor.as_deref(),
        Some("profile-checkpoint")
    );
    assert_eq!(state.gmail_sync_since, Some(200));
    assert!(state.has_gmail_sync_scope);
    server.stop();
}

#[tokio::test]
async fn expired_history_initial_sync_refreshes_folderless_metadata_without_bodies() {
    let store = prepare_gmail_sync_store(100, 200);
    let folderless = gmail_test_message("folderless-local", "folderless-cached");
    let deleted = gmail_test_message("deleted-local", "deleted-cached");
    store.insert_message(&folderless, &[]).unwrap();
    store
        .insert_message(&deleted, &["gmail-inbox".to_string()])
        .unwrap();

    let mut server = start_gmail_test_server().await;
    let worker = gmail_test_worker(Arc::clone(&store), server.endpoint.clone());
    let worker_task = tokio::spawn(async move { worker.poll_changes().await });
    let mut observed = Vec::new();

    respond_to_request(
        &mut server,
        &mut observed,
        "/labels",
        200,
        gmail_labels_response(),
    )
    .await;
    respond_to_request(
        &mut server,
        &mut observed,
        "/profile",
        200,
        r#"{"emailAddress":"gmail@example.com","historyId":"profile-checkpoint"}"#,
    )
    .await;
    respond_to_request(
        &mut server,
        &mut observed,
        "/history?startHistoryId=history-old",
        404,
        r#"{"error":"history expired"}"#,
    )
    .await;

    for _ in 0..2 {
        let request = server.next_request().await;
        assert!(request.contains("?format=minimal"), "{request}");
        observed.push(request.clone());
        if request.contains("/messages/folderless-cached?") {
            server.respond(
                200,
                r#"{"id":"folderless-cached","labelIds":["UNREAD","STARRED","Label_Keep"]}"#,
            );
        } else if request.contains("/messages/deleted-cached?") {
            server.respond(404, r#"{"error":"not found"}"#);
        } else {
            panic!("unexpected metadata request: {request}");
        }
    }

    let bootstrap_request = server.next_request().await;
    assert!(bootstrap_request.contains("/messages?labelIds="));
    observed.push(bootstrap_request.clone());
    let state = store.get_sync_state("gmail-account").unwrap().unwrap();
    assert_eq!(state.last_sync_cursor.as_deref(), Some("history-old"));
    assert_eq!(state.gmail_sync_since, Some(100));

    server.respond(200, r#"{"messages":[]}"#);
    respond_to_empty_bootstrap_labels(&mut server, &mut observed, 2).await;
    worker_task.await.unwrap().unwrap();

    let folderless = store.get_message(&folderless.id).unwrap().unwrap();
    assert!(!folderless.is_read);
    assert!(folderless.is_starred);
    assert!(store
        .get_message_folder_ids(&folderless.id)
        .unwrap()
        .contains(&"gmail-keep".to_string()));
    assert!(store.get_message(&deleted.id).unwrap().unwrap().is_deleted);
    assert!(observed
        .iter()
        .all(|request| !request.contains("format=full") && !request.contains("/attachments/")));
    assert_eq!(
        store
            .get_sync_state("gmail-account")
            .unwrap()
            .unwrap()
            .last_sync_cursor
            .as_deref(),
        Some("profile-checkpoint")
    );
    server.stop();
}

#[tokio::test]
async fn expired_history_metadata_failure_keeps_old_cursor_and_scope() {
    let store = prepare_gmail_sync_store(100, 200);
    let folderless = gmail_test_message("folderless-local", "folderless-failure");
    store.insert_message(&folderless, &[]).unwrap();

    let mut server = start_gmail_test_server().await;
    let worker = gmail_test_worker(Arc::clone(&store), server.endpoint.clone());
    let worker_task = tokio::spawn(async move { worker.poll_changes().await });
    let mut observed = Vec::new();

    respond_to_request(
        &mut server,
        &mut observed,
        "/labels",
        200,
        gmail_labels_response(),
    )
    .await;
    respond_to_request(
        &mut server,
        &mut observed,
        "/profile",
        200,
        r#"{"emailAddress":"gmail@example.com","historyId":"profile-checkpoint"}"#,
    )
    .await;
    respond_to_request(
        &mut server,
        &mut observed,
        "/history?startHistoryId=history-old",
        404,
        r#"{"error":"history expired"}"#,
    )
    .await;
    respond_to_request(
        &mut server,
        &mut observed,
        "/messages/folderless-failure?format=minimal",
        500,
        r#"{"error":"temporarily unavailable"}"#,
    )
    .await;

    assert!(worker_task.await.unwrap().is_err());
    let state = store.get_sync_state("gmail-account").unwrap().unwrap();
    assert_eq!(state.last_sync_cursor.as_deref(), Some("history-old"));
    assert_eq!(state.gmail_sync_since, Some(100));
    assert_eq!(state.sync_since, Some(200));
    assert!(!observed
        .iter()
        .any(|request| request.contains("/messages?labelIds=")));
    server.stop();
}

#[tokio::test]
async fn expired_history_bootstrap_failure_keeps_old_cursor_and_scope() {
    let store = prepare_gmail_sync_store(100, 200);
    let folderless = gmail_test_message("folderless-local", "folderless-bootstrap");
    store.insert_message(&folderless, &[]).unwrap();

    let mut server = start_gmail_test_server().await;
    let worker = gmail_test_worker(Arc::clone(&store), server.endpoint.clone());
    let worker_task = tokio::spawn(async move { worker.initial_sync().await });
    let mut observed = Vec::new();

    respond_to_request(
        &mut server,
        &mut observed,
        "/labels",
        200,
        gmail_labels_response(),
    )
    .await;
    respond_to_request(
        &mut server,
        &mut observed,
        "/profile",
        200,
        r#"{"emailAddress":"gmail@example.com","historyId":"profile-checkpoint"}"#,
    )
    .await;
    respond_to_request(
        &mut server,
        &mut observed,
        "/history?startHistoryId=history-old",
        404,
        r#"{"error":"history expired"}"#,
    )
    .await;
    respond_to_request(
        &mut server,
        &mut observed,
        "/messages/folderless-bootstrap?format=minimal",
        200,
        r#"{"id":"folderless-bootstrap","labelIds":["INBOX"]}"#,
    )
    .await;
    respond_to_request(
        &mut server,
        &mut observed,
        "/messages?labelIds=",
        500,
        r#"{"error":"bootstrap failed"}"#,
    )
    .await;
    respond_to_empty_bootstrap_labels(&mut server, &mut observed, 2).await;

    assert!(worker_task.await.unwrap().is_err());
    let state = store.get_sync_state("gmail-account").unwrap().unwrap();
    assert_eq!(state.last_sync_cursor.as_deref(), Some("history-old"));
    assert_eq!(state.gmail_sync_since, Some(100));
    assert_eq!(state.sync_since, Some(200));
    server.stop();
}

#[test]
fn test_build_sync_label_ids_includes_visible_custom_labels() {
    let folders = vec![
        Folder {
            id: "inbox".to_string(),
            account_id: "acct".to_string(),
            remote_id: "INBOX".to_string(),
            name: "Inbox".to_string(),
            folder_type: FolderType::Label,
            role: Some(FolderRole::Inbox),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 0,
        },
        Folder {
            id: "starred".to_string(),
            account_id: "acct".to_string(),
            remote_id: "STARRED".to_string(),
            name: "Starred".to_string(),
            folder_type: FolderType::Label,
            role: None,
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 1,
        },
        Folder {
            id: "custom".to_string(),
            account_id: "acct".to_string(),
            remote_id: "Label_Projects".to_string(),
            name: "Projects".to_string(),
            folder_type: FolderType::Label,
            role: None,
            parent_id: None,
            color: None,
            is_system: false,
            sort_order: 5,
        },
        Folder {
            id: "trash".to_string(),
            account_id: "acct".to_string(),
            remote_id: "TRASH".to_string(),
            name: "Trash".to_string(),
            folder_type: FolderType::Label,
            role: Some(FolderRole::Trash),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 6,
        },
        Folder {
            id: "local-archive".to_string(),
            account_id: "acct".to_string(),
            remote_id: "__local_archive__".to_string(),
            name: "Archive".to_string(),
            folder_type: FolderType::Folder,
            role: Some(FolderRole::Archive),
            parent_id: None,
            color: None,
            is_system: true,
            sort_order: 7,
        },
    ];

    let label_ids = build_sync_label_ids(&folders);
    assert_eq!(
        label_ids,
        vec![
            "INBOX".to_string(),
            "TRASH".to_string(),
            "Label_Projects".to_string(),
        ]
    );
}

#[test]
fn gmail_cursor_does_not_advance_with_failures() {
    assert!(!can_advance_gmail_cursor(1));
}

#[test]
fn gmail_cursor_advances_without_failures() {
    assert!(can_advance_gmail_cursor(0));
}

#[test]
fn gmail_history_additions_exclude_messages_deleted_in_same_batch() {
    let added = vec![
        "deleted-message".to_string(),
        "surviving-message".to_string(),
    ];
    let deleted = vec!["deleted-message".to_string()];

    assert_eq!(
        filter_deleted_history_additions(added, &deleted),
        vec!["surviving-message".to_string()]
    );
}

#[test]
fn gmail_history_fetch_404_does_not_count_as_cursor_blocking_failure() {
    let error = PebbleError::Network(
        "Failed to fetch message gone-message (status 404 Not Found): {}".to_string(),
    );

    assert!(!should_count_gmail_history_fetch_failure(&error));
}

#[test]
fn gmail_history_fetch_non_404_counts_as_cursor_blocking_failure() {
    let error = PebbleError::Network(
        "Failed to fetch message rate-limited-message (status 429 Too Many Requests): {}"
            .to_string(),
    );

    assert!(should_count_gmail_history_fetch_failure(&error));
}

#[test]
fn gmail_startup_fetch_notifies_only_when_history_cursor_exists() {
    assert!(!should_notify_gmail_startup_fetch(None));
    assert!(!should_notify_gmail_startup_fetch(Some("")));
    assert!(should_notify_gmail_startup_fetch(Some("history-1")));
}

#[test]
fn scope_mismatch_bootstrap_replays_pending_flags_deletions_and_labels() {
    let sync_state = pebble_store::accounts::SyncState {
        last_sync_cursor: Some("history-old".to_string()),
        sync_since: Some(86_400),
        gmail_sync_since: Some(0),
        has_gmail_sync_scope: true,
        ..Default::default()
    };
    assert_eq!(
        gmail_sync_dispatch(&sync_state),
        GmailSyncDispatch::Bootstrap
    );
    let history_cursor = saved_gmail_history_cursor(&sync_state);
    assert_eq!(history_cursor.as_deref(), Some("history-old"));

    let store = Store::open_in_memory().unwrap();
    store.insert_account(&gmail_test_account()).unwrap();
    let inbox = gmail_test_folder("gmail-inbox", "INBOX");
    let projects = gmail_test_folder("gmail-projects", "Label_Projects");
    let keep = gmail_test_folder("gmail-keep", "Label_Keep");
    for folder in [&inbox, &projects, &keep] {
        store.insert_folder(folder).unwrap();
    }
    let flag_message = gmail_test_message("local-flag", "flag-message");
    let deleted_message = gmail_test_message("local-deleted", "deleted-message");
    let removed_label_message = gmail_test_message("local-removed-label", "removed-label");
    let added_label_message = gmail_test_message("local-added-label", "added-label");
    for message in [
        &flag_message,
        &deleted_message,
        &removed_label_message,
        &added_label_message,
    ] {
        store
            .insert_message(message, std::slice::from_ref(&inbox.id))
            .unwrap();
    }
    store
        .add_message_to_folder(&removed_label_message.id, &projects.id)
        .unwrap();

    let page = serde_json::from_value::<GmailHistoryPage>(serde_json::json!({
        "history": [
            {"labelsAdded": [
                {"message": {"id": "flag-message"}, "labelIds": ["UNREAD"]},
                {"message": {"id": "added-label"}, "labelIds": ["Label_Keep"]}
            ]},
            {"labelsRemoved": [
                {"message": {"id": "flag-message"}, "labelIds": ["STARRED"]},
                {"message": {"id": "removed-label"}, "labelIds": ["Label_Projects"]}
            ]},
            {"messagesDeleted": [{"message": {"id": "deleted-message"}}]}
        ]
    }))
    .unwrap();
    let folders_by_remote = HashMap::from([
        ("INBOX".to_string(), inbox.id.clone()),
        ("Label_Projects".to_string(), projects.id.clone()),
        ("Label_Keep".to_string(), keep.id.clone()),
    ]);

    apply_gmail_history_mutations(
        &store,
        "gmail-account",
        &folders_by_remote,
        &page.history.unwrap(),
    )
    .unwrap();

    let updated_flag = store.get_message(&flag_message.id).unwrap().unwrap();
    assert!(!updated_flag.is_read);
    assert!(!updated_flag.is_starred);
    assert!(
        store
            .get_message(&deleted_message.id)
            .unwrap()
            .unwrap()
            .is_deleted
    );
    assert!(!store
        .get_message_folder_ids(&removed_label_message.id)
        .unwrap()
        .contains(&projects.id));
    assert!(store
        .get_message_folder_ids(&added_label_message.id)
        .unwrap()
        .contains(&keep.id));
}

#[test]
fn gmail_history_does_not_clobber_messages_with_pending_local_ops() {
    let store = Store::open_in_memory().unwrap();
    store.insert_account(&gmail_test_account()).unwrap();
    let inbox = gmail_test_folder("gmail-inbox", "INBOX");
    let keep = gmail_test_folder("gmail-keep", "Label_Keep");
    for folder in [&inbox, &keep] {
        store.insert_folder(folder).unwrap();
    }
    let mut optimistic = gmail_test_message("local-optimistic", "optimistic");
    optimistic.is_read = true;
    let plain = gmail_test_message("local-plain", "plain");
    for message in [&optimistic, &plain] {
        store
            .insert_message(message, std::slice::from_ref(&inbox.id))
            .unwrap();
    }
    // The user marked the message read offline; the op is still queued.
    store
        .insert_pending_mail_op("gmail-account", &optimistic.id, "update_flags", "{}")
        .unwrap();

    let page = serde_json::from_value::<GmailHistoryPage>(serde_json::json!({
        "history": [{"labelsAdded": [
            {"message": {"id": "optimistic"}, "labelIds": ["UNREAD", "Label_Keep"]},
            {"message": {"id": "plain"}, "labelIds": ["STARRED", "Label_Keep"]}
        ]}]
    }))
    .unwrap();
    let folders_by_remote = HashMap::from([
        ("INBOX".to_string(), inbox.id.clone()),
        ("Label_Keep".to_string(), keep.id.clone()),
    ]);
    let refreshed = apply_gmail_history_mutations(
        &store,
        "gmail-account",
        &folders_by_remote,
        &page.history.unwrap(),
    )
    .unwrap();

    let kept = store.get_message(&optimistic.id).unwrap().unwrap();
    assert!(kept.is_read, "queued local read state must survive sync");
    assert!(!store
        .get_message_folder_ids(&optimistic.id)
        .unwrap()
        .contains(&keep.id));
    assert!(!refreshed.contains(&optimistic.id));
    assert!(store.get_message(&plain.id).unwrap().unwrap().is_starred);
    assert!(store
        .get_message_folder_ids(&plain.id)
        .unwrap()
        .contains(&keep.id));
}

#[test]
fn gmail_label_progress_round_trips_and_respects_scope() {
    let progress = GmailLabelProgress {
        history_id: "123".to_string(),
        sync_since: Some(86_400),
        page_token: Some("page-2".to_string()),
        complete: false,
    };
    let state = serialize_gmail_label_progress(Some(&progress));
    assert_eq!(
        parse_gmail_label_progress(Some(&state), Some(86_400)),
        Some(progress.clone())
    );
    // A different sync range restarts the label.
    assert_eq!(parse_gmail_label_progress(Some(&state), None), None);
    // Cleared or foreign state means no resume.
    assert_eq!(
        parse_gmail_label_progress(Some(&serialize_gmail_label_progress(None)), None),
        None
    );
    assert_eq!(parse_gmail_label_progress(Some("not json"), None), None);
    assert_eq!(parse_gmail_label_progress(None, None), None);

    // Resumed bootstraps keep the older checkpoint so finished labels'
    // later changes are replayed from history.
    assert_eq!(gmail_bootstrap_checkpoint([&progress], "999"), "123");
    assert_eq!(gmail_bootstrap_checkpoint([], "999"), "999");
}

#[tokio::test]
async fn failed_gmail_history_replay_does_not_checkpoint_cursor() {
    let mut checkpoint_count = 0;
    let result = replay_gmail_history_and_checkpoint(
        || async {
            Err(PebbleError::Network(
                "history event application failed".to_string(),
            ))
        },
        |_| {
            checkpoint_count += 1;
            Ok(())
        },
    )
    .await;

    assert!(result.is_err());
    assert_eq!(checkpoint_count, 0);
}

#[tokio::test]
async fn expired_gmail_history_replay_does_not_checkpoint_cursor() {
    let mut checkpoint_count = 0;
    let outcome = replay_gmail_history_and_checkpoint(
        || async { Ok(GmailHistoryReplayOutcome::CursorExpired) },
        |_| {
            checkpoint_count += 1;
            Ok(())
        },
    )
    .await
    .unwrap();

    assert!(matches!(outcome, GmailHistoryReplayOutcome::CursorExpired));
    assert_eq!(checkpoint_count, 0);
}

#[tokio::test]
async fn expired_gmail_metadata_failure_is_retryable_without_checkpoint() {
    let store = Store::open_in_memory().unwrap();
    store.insert_account(&gmail_test_account()).unwrap();
    store
        .set_sync_cursor("gmail-account", "history-old")
        .unwrap();
    store
        .update_sync_state("gmail-account", |state| {
            state.gmail_sync_since = Some(86_400);
            state.has_gmail_sync_scope = true;
        })
        .unwrap();

    let error = fetch_cached_gmail_metadata(vec!["cached-message".to_string()], |_| async {
        Err(PebbleError::Network(
            "metadata service unavailable".to_string(),
        ))
    })
    .await
    .unwrap_err();

    assert!(error.to_string().contains("metadata service unavailable"));
    let state = store.get_sync_state("gmail-account").unwrap().unwrap();
    assert_eq!(state.last_sync_cursor.as_deref(), Some("history-old"));
    assert_eq!(state.gmail_sync_since, Some(86_400));
}

#[test]
fn gmail_scope_dispatch_retries_partial_backfill_for_expansion_and_all_mail() {
    let mut state = pebble_store::accounts::SyncState {
        last_sync_cursor: Some("history-old".to_string()),
        sync_since: Some(0),
        gmail_sync_since: Some(86_400),
        has_gmail_sync_scope: true,
        ..Default::default()
    };

    assert_eq!(gmail_sync_dispatch(&state), GmailSyncDispatch::Bootstrap);
    assert!(!can_advance_gmail_cursor(1));
    assert_eq!(gmail_sync_dispatch(&state), GmailSyncDispatch::Bootstrap);

    state.sync_since = None;
    assert_eq!(gmail_sync_dispatch(&state), GmailSyncDispatch::Bootstrap);

    state.gmail_sync_since = None;
    state.last_sync_cursor = Some("history-new".to_string());
    assert_eq!(
        gmail_sync_dispatch(&state),
        GmailSyncDispatch::History("history-new".to_string())
    );
}

#[tokio::test]
async fn collect_paginated_gmail_refs_fetches_until_next_page_is_empty() {
    let mut pages = VecDeque::from([
        (
            vec![crate::provider::gmail::GmailMessageRef {
                id: "gmail-1".to_string(),
                thread_id: None,
            }],
            Some("page-2".to_string()),
        ),
        (
            vec![crate::provider::gmail::GmailMessageRef {
                id: "gmail-2".to_string(),
                thread_id: None,
            }],
            None,
        ),
    ]);
    let mut requested_tokens = Vec::new();

    let refs = collect_paginated_gmail_refs("INBOX", 2, |label_id, limit, page_token| {
        requested_tokens.push((label_id, limit, page_token));
        let page = pages.pop_front().expect("expected a page request");
        async move { Ok(page) }
    })
    .await
    .unwrap();

    let ids: Vec<_> = refs.into_iter().map(|r| r.id).collect();
    assert_eq!(ids, vec!["gmail-1".to_string(), "gmail-2".to_string()]);
    assert_eq!(
        requested_tokens,
        vec![
            ("INBOX".to_string(), 2, None),
            ("INBOX".to_string(), 2, Some("page-2".to_string())),
        ]
    );
}

#[tokio::test]
async fn collect_paginated_gmail_history_fetches_every_page() {
    let mut pages = VecDeque::from([
        serde_json::from_value::<GmailHistoryPage>(serde_json::json!({
            "history": [{"messagesAdded": [{"message": {"id": "gmail-1"}}]}],
            "historyId": "history-2",
            "nextPageToken": "page-2"
        }))
        .unwrap(),
        serde_json::from_value::<GmailHistoryPage>(serde_json::json!({
            "history": [{"messagesAdded": [{"message": {"id": "gmail-2"}}]}],
            "historyId": "history-2"
        }))
        .unwrap(),
    ]);
    let mut requested_tokens = Vec::new();

    let batch = collect_paginated_gmail_history("history-1", |page_token| {
        requested_tokens.push(page_token);
        let page = pages.pop_front().expect("expected a history page request");
        async move { Ok(GmailHistoryPageFetch::Page(page)) }
    })
    .await
    .unwrap();

    assert_eq!(
        batch
            .entries
            .iter()
            .flat_map(|entry| entry.messages_added.iter().flatten())
            .map(|message| message.message.id.as_str())
            .collect::<Vec<_>>(),
        vec!["gmail-1", "gmail-2"]
    );
    assert_eq!(batch.history_id.as_deref(), Some("history-2"));
    assert_eq!(requested_tokens, vec![None, Some("page-2".to_string())]);
}

#[tokio::test]
async fn collect_paginated_gmail_history_reports_expired_cursor() {
    let batch = collect_paginated_gmail_history("expired-history", |_| async {
        Ok(GmailHistoryPageFetch::CursorExpired)
    })
    .await
    .unwrap();

    assert!(batch.cursor_expired);
    assert!(batch.entries.is_empty());
    assert!(batch.history_id.is_none());
}

#[test]
fn gmail_history_404_means_cursor_expired() {
    assert!(gmail_history_cursor_expired(reqwest::StatusCode::NOT_FOUND));
    assert!(!gmail_history_cursor_expired(
        reqwest::StatusCode::INTERNAL_SERVER_ERROR
    ));
}

#[test]
fn gmail_history_labels_update_read_and_starred_flags() {
    let page = serde_json::from_value::<GmailHistoryPage>(serde_json::json!({
        "history": [
            {"labelsAdded": [{
                "message": {"id": "gmail-1"},
                "labelIds": ["UNREAD", "STARRED"]
            }]},
            {"labelsRemoved": [{
                "message": {"id": "gmail-2"},
                "labelIds": ["UNREAD", "STARRED"]
            }]}
        ],
        "historyId": "history-2"
    }))
    .unwrap();

    let changes = gmail_history_flag_changes(&page.history.unwrap());

    assert_eq!(changes.len(), 2);
    assert_eq!(changes[0].remote_id, "gmail-1");
    assert_eq!(changes[0].is_read, Some(false));
    assert_eq!(changes[0].is_starred, Some(true));
    assert_eq!(changes[1].remote_id, "gmail-2");
    assert_eq!(changes[1].is_read, Some(true));
    assert_eq!(changes[1].is_starred, Some(false));
}

#[test]
fn gmail_history_label_add_then_remove_finishes_without_label() {
    let page = serde_json::from_value::<GmailHistoryPage>(serde_json::json!({
        "history": [
            {"labelsAdded": [{
                "message": {"id": "gmail-1"},
                "labelIds": ["Label_Projects"]
            }]},
            {"labelsRemoved": [{
                "message": {"id": "gmail-1"},
                "labelIds": ["Label_Projects"]
            }]}
        ]
    }))
    .unwrap();

    assert_eq!(
        gmail_history_label_mutations(&page.history.unwrap()),
        vec![GmailHistoryLabelMutation {
            remote_id: "gmail-1".to_string(),
            label_id: "Label_Projects".to_string(),
            action: GmailHistoryLabelAction::Remove,
        }]
    );
}

#[test]
fn gmail_history_label_remove_then_add_finishes_with_label() {
    let page = serde_json::from_value::<GmailHistoryPage>(serde_json::json!({
        "history": [
            {"labelsRemoved": [{
                "message": {"id": "gmail-1"},
                "labelIds": ["Label_Projects"]
            }]},
            {"labelsAdded": [{
                "message": {"id": "gmail-1"},
                "labelIds": ["Label_Projects"]
            }]}
        ]
    }))
    .unwrap();

    assert_eq!(
        gmail_history_label_mutations(&page.history.unwrap()),
        vec![GmailHistoryLabelMutation {
            remote_id: "gmail-1".to_string(),
            label_id: "Label_Projects".to_string(),
            action: GmailHistoryLabelAction::Add,
        }]
    );
}

#[test]
fn gmail_history_label_failures_keep_cursor_for_retry() {
    assert!(!can_advance_gmail_cursor(1));
    assert!(can_advance_gmail_cursor(0));
}
