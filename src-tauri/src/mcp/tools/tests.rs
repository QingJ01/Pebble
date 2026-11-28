//! Access control for the MCP tools: every way a call may be refused, and
//! what each level returns.

use std::collections::BTreeMap;

use super::*;
use crate::mcp::{server::handle_call, token_hash};
use pebble_core::{Account, EmailAddress, Folder, FolderType};

struct Fixture {
    store: Store,
    search: TantivySearch,
}

fn account(id: &str, email: &str, provider: ProviderType) -> Account {
    Account {
        id: id.into(),
        email: email.into(),
        display_name: "Me".into(),
        color: None,
        account_label: None,
        provider_display_name: None,
        provider,
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

fn put(f: &Fixture, id: &str, account: &str, folder: &str, subject: &str, body: &str) {
    let message = Message {
        id: id.into(),
        account_id: account.into(),
        remote_id: id.into(),
        message_id_header: None,
        in_reply_to: None,
        references_header: None,
        thread_id: Some(format!("t-{id}")),
        subject: subject.into(),
        snippet: body.chars().take(80).collect(),
        from_address: "dana@partner.example".into(),
        from_name: "Dana".into(),
        to_list: vec![EmailAddress {
            name: None,
            address: "me@work.example".into(),
        }],
        cc_list: vec![],
        bcc_list: vec![],
        body_text: body.into(),
        body_html_raw: String::new(),
        has_attachments: false,
        is_read: false,
        is_starred: false,
        is_draft: false,
        date: 1_800_000_000,
        remote_version: None,
        is_deleted: false,
        deleted_at: None,
        created_at: 0,
        updated_at: 0,
    };
    f.store
        .insert_message(&message, &[folder.to_string()])
        .unwrap();
    f.search
        .index_message(&message, &[folder.to_string()])
        .unwrap();
}

fn setup() -> Fixture {
    let f = Fixture {
        store: Store::open_in_memory().unwrap(),
        search: TantivySearch::open_in_memory().unwrap(),
    };
    f.store
        .insert_account(&account("work", "me@work.example", ProviderType::Imap))
        .unwrap();
    f.store
        .insert_account(&account("home", "me@home.example", ProviderType::Imap))
        .unwrap();
    f.store
        .insert_account(&account("gm", "me@gmail.com", ProviderType::Gmail))
        .unwrap();
    for (id, acct, role) in [
        ("w-in", "work", FolderRole::Inbox),
        ("w-spam", "work", FolderRole::Spam),
        ("h-in", "home", FolderRole::Inbox),
        ("g-in", "gm", FolderRole::Inbox),
    ] {
        f.store.insert_folder(&folder(id, acct, role)).unwrap();
    }
    put(
        &f,
        "w1",
        "work",
        "w-in",
        "Partnership proposal",
        "Shall we partner? Your code is 482913.",
    );
    put(
        &f,
        "w2",
        "work",
        "w-spam",
        "Partnership spam",
        "Buy partnership leads",
    );
    put(
        &f,
        "h1",
        "home",
        "h-in",
        "Partnership dinner",
        "Dinner on Friday?",
    );
    put(&f, "g1", "gm", "g-in", "Partnership gmail", "Gmail mail");
    f.search.commit().unwrap();
    f
}

fn pair(f: &Fixture, grants: &[(&str, &str)]) -> String {
    let token = crate::mcp::new_token();
    let grants: BTreeMap<String, String> = grants
        .iter()
        .map(|(a, l)| (a.to_string(), l.to_string()))
        .collect();
    f.store
        .insert_mcp_client(
            &pebble_core::new_id(),
            "Claude Desktop",
            &token_hash(&token),
            &grants,
            1,
        )
        .unwrap();
    token
}

fn call(f: &Fixture, token: &str, tool: &str, args: Value) -> pebble_mcp::protocol::BridgeResponse {
    handle_call(&f.store, &f.search, token, tool, args)
}

#[test]
fn calls_without_a_valid_token_or_grant_are_refused() {
    let f = setup();
    let r = call(
        &f,
        "pmcp_nope",
        TOOL_SEARCH,
        json!({"query": "partnership"}),
    );
    assert_eq!(r.code, Some(ErrorCode::Unauthorized));

    let token = pair(&f, &[("work", "metadata")]);
    assert_eq!(
        call(&f, &token, "mail_send", json!({})).code,
        Some(ErrorCode::Invalid)
    );
    // Metadata can't read threads or propose drafts.
    assert_eq!(
        call(&f, &token, TOOL_GET_THREAD, json!({"message_id": "w1"})).code,
        Some(ErrorCode::Forbidden)
    );
    assert_eq!(
        call(
            &f,
            &token,
            TOOL_PROPOSE_DRAFT,
            json!({"body": "x", "to": ["a@b.example"]})
        )
        .code,
        Some(ErrorCode::Forbidden)
    );
    // An account outside the grant.
    assert_eq!(
        call(
            &f,
            &token,
            TOOL_SEARCH,
            json!({"query": "partnership", "account": "me@home.example"})
        )
        .code,
        Some(ErrorCode::Forbidden)
    );

    let client = f.store.list_mcp_clients().unwrap()[0].clone();
    f.store.revoke_mcp_client(&client.id, 5).unwrap();
    assert_eq!(
        call(&f, &token, TOOL_SEARCH, json!({})).code,
        Some(ErrorCode::Unauthorized),
        "revoked"
    );
}

#[test]
fn gmail_is_never_reachable_even_if_granted() {
    let f = setup();
    let token = pair(&f, &[("gm", "body")]);
    assert_eq!(
        call(&f, &token, TOOL_SEARCH, json!({"query": "partnership"})).code,
        Some(ErrorCode::Forbidden)
    );
    let both = pair(&f, &[("gm", "body"), ("work", "body")]);
    let r = call(&f, &both, TOOL_SEARCH, json!({"query": "partnership"}));
    let text = r.result.unwrap().to_string();
    assert!(!text.contains("gmail"), "{text}");
    assert_eq!(
        call(&f, &both, TOOL_GET_THREAD, json!({"message_id": "g1"})).code,
        Some(ErrorCode::NotFound)
    );
    let grants = BTreeMap::from([
        ("gm".to_string(), "body".to_string()),
        ("work".to_string(), "root".to_string()),
    ]);
    assert!(sanitize_grants(&f.store, &grants).is_empty());
}

#[test]
fn search_respects_levels_scope_spam_and_pages() {
    let f = setup();
    let token = pair(&f, &[("work", "metadata"), ("home", "summary")]);
    let r = call(&f, &token, TOOL_SEARCH, json!({"query": "partnership"}))
        .result
        .unwrap();
    let results = r["results"].as_array().unwrap();
    let ids: Vec<&str> = results
        .iter()
        .map(|m| m["message_id"].as_str().unwrap())
        .collect();
    assert!(ids.contains(&"w1") && ids.contains(&"h1"));
    assert!(!ids.contains(&"w2"), "spam is left out");
    assert!(!ids.contains(&"g1"));
    let w1 = results.iter().find(|m| m["message_id"] == "w1").unwrap();
    assert!(w1.get("snippet").is_none(), "metadata only");
    let h1 = results.iter().find(|m| m["message_id"] == "h1").unwrap();
    assert!(h1["snippet"].as_str().unwrap().contains("Dinner"));
    assert_eq!(r["coverage"]["excluded"], json!(["spam", "trash"]));

    let page = call(
        &f,
        &token,
        TOOL_SEARCH,
        json!({"query": "partnership", "limit": 1}),
    )
    .result
    .unwrap();
    assert_eq!(page["results"].as_array().unwrap().len(), 1);
    let next = page["next_cursor"].as_str().unwrap();
    let rest = call(
        &f,
        &token,
        TOOL_SEARCH,
        json!({"query": "partnership", "limit": 1, "cursor": next}),
    )
    .result
    .unwrap();
    assert_ne!(
        rest["results"][0]["message_id"],
        page["results"][0]["message_id"]
    );
}

#[test]
fn threads_mask_codes_and_drafts_become_proposals() {
    let f = setup();
    let token = pair(&f, &[("work", "drafts")]);
    let thread = call(&f, &token, TOOL_GET_THREAD, json!({"message_id": "w1"}))
        .result
        .unwrap();
    let body = thread["messages"][0]["body"].as_str().unwrap();
    assert!(
        body.contains("partner") && !body.contains("482913"),
        "{body}"
    );
    assert!(thread["messages"][0]["attachments"].is_array());
    assert_eq!(
        call(&f, &token, TOOL_GET_THREAD, json!({"message_id": "h1"})).code,
        Some(ErrorCode::NotFound)
    );

    let r = call(
        &f,
        &token,
        TOOL_PROPOSE_DRAFT,
        json!({"reply_to_message_id": "w1", "to": ["evil@x.example"], "body": "Yes, let's."}),
    );
    assert!(r.ok, "{:?}", r.error);
    let event = &f.store.list_agent_events(10, None).unwrap()[0];
    assert_eq!(event.kind, "draft_proposal");
    assert_eq!(event.payload["client"], "Claude Desktop");
    assert_eq!(
        event.payload["draft"]["to"],
        json!(["dana@partner.example"]),
        "a reply goes to the original sender"
    );
    assert_eq!(event.push_state, "pending");
}

#[test]
fn the_audit_log_has_no_content_tokens_or_codes() {
    let f = setup();
    let token = pair(&f, &[("work", "body")]);
    call(&f, &token, TOOL_SEARCH, json!({"query": "partnership"}));
    call(&f, &token, TOOL_GET_THREAD, json!({"message_id": "w1"}));
    call(
        &f,
        &token,
        TOOL_PROPOSE_DRAFT,
        json!({"body": "x", "to": ["a@b.example"]}),
    );
    let audit = f.store.list_mcp_audit(None, 10).unwrap();
    assert_eq!(audit.len(), 3);
    assert_eq!(audit[0].outcome, "forbidden");
    assert_eq!(audit[2].accounts, "work");
    let dump = format!("{audit:?}");
    for secret in [token.as_str(), "482913", "Shall we partner", "partnership"] {
        assert!(!dump.contains(secret), "audit leaks {secret}");
    }
}

#[test]
fn calls_are_rate_limited_per_client() {
    let f = setup();
    let token = pair(&f, &[("work", "metadata")]);
    let mut limited = 0;
    for _ in 0..65 {
        if call(&f, &token, TOOL_SEARCH, json!({})).code == Some(ErrorCode::RateLimited) {
            limited += 1;
        }
    }
    assert_eq!(limited, 5);
}
