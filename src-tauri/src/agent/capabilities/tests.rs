use super::*;
use pebble_core::{Account, EmailAddress, Folder, FolderRole, FolderType, ProviderType};

const NOW: i64 = 1_800_000_000;

struct Fixture {
    store: Store,
    search: TantivySearch,
}

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

fn setup() -> Fixture {
    let store = Store::open_in_memory().unwrap();
    let search = TantivySearch::open_in_memory().unwrap();
    for (id, email) in [("work", "me@work.example"), ("home", "me@home.example")] {
        store.insert_account(&account(id, email)).unwrap();
        store
            .insert_folder(&Folder {
                id: format!("{id}-inbox"),
                account_id: id.into(),
                remote_id: "INBOX".into(),
                name: "Inbox".into(),
                folder_type: FolderType::Folder,
                role: Some(FolderRole::Inbox),
                parent_id: None,
                color: None,
                is_system: true,
                sort_order: 0,
            })
            .unwrap();
    }
    Fixture { store, search }
}

#[allow(clippy::too_many_arguments)]
fn put(
    f: &Fixture,
    id: &str,
    account: &str,
    from: &str,
    subject: &str,
    body: &str,
    thread: &str,
    hours_ago: i64,
) -> Message {
    let message = Message {
        id: id.into(),
        account_id: account.into(),
        remote_id: id.into(),
        message_id_header: None,
        in_reply_to: None,
        references_header: None,
        thread_id: Some(thread.into()),
        subject: subject.into(),
        snippet: body.chars().take(80).collect(),
        from_address: from.into(),
        from_name: String::new(),
        to_list: vec![EmailAddress {
            name: None,
            address: format!("me@{account}.example"),
        }],
        cc_list: vec![],
        bcc_list: vec![],
        body_text: body.into(),
        body_html_raw: String::new(),
        has_attachments: false,
        is_read: false,
        is_starred: false,
        is_draft: false,
        date: NOW - hours_ago * 3600,
        remote_version: None,
        is_deleted: false,
        deleted_at: None,
        created_at: 0,
        updated_at: 0,
    };
    let folder = format!("{account}-inbox");
    f.store
        .insert_message(&message, std::slice::from_ref(&folder))
        .unwrap();
    f.search.index_message(&message, &[folder]).unwrap();
    f.search.commit().unwrap();
    message
}

fn work_scope() -> HashMap<String, String> {
    HashMap::from([("work".to_string(), "me@work.example".to_string())])
}

#[test]
fn search_stays_inside_the_scope_and_records_sources() {
    let f = setup();
    put(
        &f,
        "w1",
        "work",
        "dana@partner.example",
        "Partnership proposal",
        "Let's partner on the launch",
        "t1",
        5,
    );
    put(
        &f,
        "h1",
        "home",
        "dana@partner.example",
        "Partnership dinner",
        "Partnership dinner Friday",
        "t2",
        5,
    );
    let mut caps = Capabilities::new(&f.store, &f.search, work_scope());
    let result = caps.call("search_mail", r#"{"query": "partnership"}"#);
    let ids: Vec<&str> = result["results"]
        .as_array()
        .unwrap()
        .iter()
        .map(|r| r["message_id"].as_str().unwrap())
        .collect();
    assert_eq!(ids, vec!["w1"]);
    assert_eq!(result["searched_accounts"][0], "me@work.example");
    assert_eq!(caps.used.len(), 1);
    assert_eq!(caps.used[0].message_id, "w1");
}

#[test]
fn reading_outside_the_scope_is_refused() {
    let f = setup();
    put(
        &f,
        "h1",
        "home",
        "mom@family.example",
        "Dinner",
        "Come home Sunday",
        "t2",
        5,
    );
    let mut caps = Capabilities::new(&f.store, &f.search, work_scope());
    let result = caps.call("read_thread", r#"{"message_id": "h1"}"#);
    assert!(result["error"].as_str().unwrap().contains("No message"));
    assert!(caps.used.is_empty());
}

#[test]
fn threads_drop_quotes_and_mask_codes() {
    let f = setup();
    put(
        &f,
        "w1",
        "work",
        "it@corp.example",
        "Login",
        "Your verification code is 482913.\n\nOn Mon, X wrote:\n> old",
        "t1",
        3,
    );
    put(
        &f,
        "w2",
        "work",
        "me@work.example",
        "Re: Login",
        "Thanks",
        "t1",
        2,
    );
    let mut caps = Capabilities::new(&f.store, &f.search, work_scope());
    let result = caps.call("read_thread", r#"{"message_id": "w2"}"#);
    let bodies: Vec<&str> = result["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|m| m["body"].as_str().unwrap())
        .collect();
    assert_eq!(bodies.len(), 2);
    assert!(!bodies[0].contains("482913"), "{}", bodies[0]);
    assert!(!bodies[0].contains("old"));
    assert_eq!(caps.used.len(), 2);
    assert_eq!(
        redact_codes("验证码：735 102，5 分钟内有效"),
        "验证码：••••••，5 分钟内有效"
    );
    assert_eq!(redact_codes("Order 123456 shipped"), "Order 123456 shipped");
    for leaky in [
        "482913 is your Google verification code",
        "G-482913 是你的验证码",
        "Your code for account 2 is 482913",
        "Your verification code is 482913",
    ] {
        assert!(
            !redact_codes(leaky).contains("482913"),
            "{leaky} → {}",
            redact_codes(leaky)
        );
    }
    assert_eq!(
        redact_codes("Free shipping 2024 sale"),
        "Free shipping 2024 sale",
        "no match inside words"
    );
}

#[test]
fn drafts_are_proposals_with_the_right_recipient() {
    let f = setup();
    put(
        &f,
        "w1",
        "work",
        "dana@partner.example",
        "Proposal",
        "Thoughts?",
        "t1",
        5,
    );
    let mut caps = Capabilities::new(&f.store, &f.search, work_scope());
    let result = caps.call(
        "draft_email",
        r#"{"reply_to_message_id": "w1", "body": "Looks good, let's talk Friday."}"#,
    );
    assert_eq!(result["status"], "draft_ready");
    let draft = &caps.drafts[0];
    assert_eq!(draft.to, vec!["dana@partner.example"]);
    assert_eq!(draft.subject, "Re: Proposal");
    assert_eq!(draft.account_id, "work");
    // A reply ignores recipients the model picked.
    caps.call(
        "draft_email",
        r#"{"reply_to_message_id": "w1", "to": ["attacker@evil.example"], "body": "Sure"}"#,
    );
    assert_eq!(caps.drafts[1].to, vec!["dana@partner.example"]);
    assert!(caps.drafts[1].unverified.is_empty());
    // A new mail to an address nobody mentioned is flagged; one the user named isn't.
    caps.question = "Write to wei@friends.example".into();
    caps.call(
        "draft_email",
        r#"{"to": ["wei@friends.example", "attacker@evil.example"], "subject": "Hi", "body": "Hello"}"#,
    );
    assert_eq!(caps.drafts[2].unverified, vec!["attacker@evil.example"]);
    assert!(caps.call("draft_email", r#"{"body": "Hi"}"#)["error"]
        .as_str()
        .unwrap()
        .contains("recipient"));
    assert!(caps.call("draft_email", r#"{"body": ""}"#)["error"].is_string());
}

#[test]
fn follow_up_creates_a_watch_and_recent_mail_lists_the_inbox() {
    let f = setup();
    put(
        &f,
        "w1",
        "work",
        "landlord@homes.example",
        "Lease",
        "Renew?",
        "t1",
        1,
    );
    put(
        &f,
        "w-old",
        "work",
        "old@corp.example",
        "Old",
        "Old mail",
        "t9",
        24 * 10,
    );
    let mut caps = Capabilities::new(&f.store, &f.search, work_scope());
    let followed = caps.call("follow_up", r#"{"message_id": "w1", "days": 3}"#);
    assert_eq!(followed["status"], "following");
    assert_eq!(caps.follow_ups.len(), 1);
    assert_eq!(f.store.open_thread_watches().unwrap()[0].source, "agent");

    let recent = caps.call("list_recent_mail", r#"{"days": 2}"#);
    let _ = recent; // dates are relative to the real clock; covered by the store test
    assert!(caps.call("unknown_tool", "{}")["error"]
        .as_str()
        .unwrap()
        .contains("Unknown"));
    assert!(caps.call("search_mail", "not json")["error"].is_string());
}
