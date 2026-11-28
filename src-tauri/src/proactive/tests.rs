use super::*;
use pebble_core::{EmailAddress, Folder, FolderRole, FolderType, MailSignals, ProviderType};
use pebble_store::agent::NewThreadWatch;

const HOUR: i64 = 3600;
const NOW: i64 = 1_800_000_000;

fn setup() -> (Store, Account) {
    let store = Store::open_in_memory().unwrap();
    let account = Account {
        id: "acct".into(),
        email: "me@pebble.example".into(),
        display_name: "Me".into(),
        color: None,
        account_label: None,
        provider_display_name: None,
        provider: ProviderType::Imap,
        created_at: 0,
        updated_at: 0,
    };
    store.insert_account(&account).unwrap();
    store
        .insert_folder(&Folder {
            id: "inbox".into(),
            account_id: "acct".into(),
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
    (store, account)
}

fn put(
    store: &Store,
    from: &str,
    subject: &str,
    body: &str,
    hours_ago: i64,
    thread: &str,
) -> Message {
    put_to(
        store,
        from,
        subject,
        body,
        hours_ago,
        thread,
        &["me@pebble.example"],
        &[],
    )
}

#[allow(clippy::too_many_arguments)]
fn put_to(
    store: &Store,
    from: &str,
    subject: &str,
    body: &str,
    hours_ago: i64,
    thread: &str,
    to: &[&str],
    cc: &[&str],
) -> Message {
    let list = |items: &[&str]| -> Vec<EmailAddress> {
        items
            .iter()
            .map(|address| EmailAddress {
                name: None,
                address: address.to_string(),
            })
            .collect()
    };
    let message = Message {
        id: pebble_core::new_id(),
        account_id: "acct".into(),
        remote_id: pebble_core::new_id(),
        message_id_header: None,
        in_reply_to: None,
        references_header: None,
        thread_id: Some(thread.into()),
        subject: subject.into(),
        snippet: body.chars().take(60).collect(),
        from_address: from.into(),
        from_name: String::new(),
        to_list: list(to),
        cc_list: list(cc),
        bcc_list: vec![],
        body_text: body.into(),
        body_html_raw: String::new(),
        has_attachments: false,
        is_read: false,
        is_starred: false,
        is_draft: false,
        date: NOW - hours_ago * HOUR,
        remote_version: None,
        is_deleted: false,
        deleted_at: None,
        created_at: 0,
        updated_at: 0,
    };
    store
        .insert_message(&message, &["inbox".to_string()])
        .unwrap();
    message
}

fn known(store: &Store, address: &str) {
    let mut sent = put(
        store,
        "me@pebble.example",
        "hi",
        "hi",
        500,
        &format!("sent-{address}"),
    );
    sent.to_list = vec![EmailAddress {
        name: None,
        address: address.into(),
    }];
    store.note_sent_recipients(&sent).unwrap();
}

#[test]
fn finds_clear_personal_asks_and_skips_the_rest() {
    let (store, account) = setup();
    let prefs = NotifyPrefsRecord::default();
    known(&store, "ann@friends.example");
    let ask = put(
        &store,
        "ann@friends.example",
        "Dinner on Friday?",
        "Are you free at 7?",
        30,
        "t1",
    );
    put(
        &store,
        "sam@corp.example",
        "Slides",
        "For reference, nothing needed from you.",
        30,
        "t2",
    );
    put(
        &store,
        "fresh@corp.example",
        "Quick question",
        "Can you check this?",
        2,
        "t3",
    );
    put(
        &store,
        "noreply@shop.example",
        "Can you rate us?",
        "How did we do?",
        30,
        "t4",
    );
    let bulk = put(
        &store,
        "editor@weekly.example",
        "Your thoughts?",
        "What do you think?",
        30,
        "t5",
    );
    store
        .record_message_signals(
            &bulk.id,
            MailSignals {
                bulk: true,
                ..MailSignals::default()
            },
        )
        .unwrap();
    put(
        &store,
        "pushy@sales.example",
        "Can we talk?",
        "Do you have time?",
        30,
        "t6",
    );
    store
        .set_sender_pref("", "@sales.example", "mute", 1)
        .unwrap();
    // A question to a whole list, from someone the user never wrote to.
    put_to(
        &store,
        "someone@corp.example",
        "Ideas?",
        "Any ideas?",
        30,
        "t7",
        &["team@corp.example"],
        &["me@pebble.example"],
    );
    let snoozed = put(
        &store,
        "ann@friends.example",
        "Also: tickets?",
        "Can you buy the tickets?",
        30,
        "t8",
    );
    store
        .snooze_message(&pebble_core::SnoozedMessage {
            message_id: snoozed.id.clone(),
            snoozed_at: NOW,
            unsnoozed_at: NOW + 3600,
            return_to: "inbox".into(),
        })
        .unwrap();

    let found = find_awaiting_replies(&store, &account, &prefs, NOW);
    let ids: Vec<_> = found.iter().map(|item| item.message.id.clone()).collect();
    assert_eq!(ids, vec![ask.id.clone()]);
    assert!(found[0].score >= AWAITING_THRESHOLD);

    assert_eq!(record_awaiting_replies(&store, &found, NOW), 1);
    assert!(
        find_awaiting_replies(&store, &account, &prefs, NOW).is_empty(),
        "raised once"
    );
    let event = &store.list_agent_events(10, None).unwrap()[0];
    assert_eq!(event.push_state, "pending");
    assert_eq!(event.payload["snippet"], "Are you free at 7?");
}

#[test]
fn old_findings_stay_in_the_conversation() {
    let (store, account) = setup();
    known(&store, "ann@friends.example");
    put(
        &store,
        "ann@friends.example",
        "Contract?",
        "Could you sign the contract?",
        5 * 24,
        "t1",
    );
    let found = find_awaiting_replies(&store, &account, &NotifyPrefsRecord::default(), NOW);
    assert_eq!(found.len(), 1);
    record_awaiting_replies(&store, &found, NOW);
    assert_eq!(
        store.list_agent_events(10, None).unwrap()[0].push_state,
        "in_app"
    );
    assert!(store.pending_agent_pushes().unwrap().is_empty());
}

#[test]
fn a_reply_from_the_user_clears_the_candidate() {
    let (store, account) = setup();
    known(&store, "ann@friends.example");
    put(
        &store,
        "ann@friends.example",
        "Dinner on Friday?",
        "Are you free at 7?",
        30,
        "t1",
    );
    put(
        &store,
        "me@pebble.example",
        "Re: Dinner on Friday?",
        "Yes!",
        20,
        "t1",
    );
    assert!(find_awaiting_replies(&store, &account, &NotifyPrefsRecord::default(), NOW).is_empty());
}

#[test]
fn overdue_follow_ups_are_recorded_once_and_ask_to_be_pushed() {
    let (store, _) = setup();
    store
        .create_thread_watch(
            &NewThreadWatch {
                account_id: "acct".into(),
                thread_id: Some("t9".into()),
                subject: "Proposal".into(),
                deadline: Some(NOW - 10),
                ..Default::default()
            },
            NOW - 100,
        )
        .unwrap();
    assert_eq!(record_overdue_follow_ups(&store, NOW), 1);
    assert_eq!(record_overdue_follow_ups(&store, NOW + 60), 0);
    let events = store.list_agent_events(10, None).unwrap();
    assert_eq!(
        (events[0].kind.as_str(), events[0].push_state.as_str()),
        ("watch_overdue", "pending")
    );
    assert_eq!(events[0].payload["title"], "Proposal");
}

#[test]
fn briefing_is_made_once_in_its_window_and_only_pushed_with_news() {
    let (store, account) = setup();
    let prefs = NotifyPrefsRecord::default();
    assert!(
        !maybe_record_briefing(&store, &prefs, "2027-01-04", 8 * 60, NOW),
        "before 09:00"
    );
    assert!(maybe_record_briefing(
        &store,
        &prefs,
        "2027-01-04",
        9 * 60 + 5,
        NOW
    ));
    assert!(
        !maybe_record_briefing(&store, &prefs, "2027-01-04", 10 * 60, NOW),
        "once a day"
    );
    let quiet = store.list_agent_events(10, None).unwrap();
    assert_eq!(quiet[0].kind, "briefing");
    assert_eq!(
        quiet[0].push_state, "in_app",
        "nothing to say: no notification"
    );

    known(&store, "ann@friends.example");
    put(
        &store,
        "ann@friends.example",
        "Dinner on Friday?",
        "Are you free at 7?",
        30,
        "t1",
    );
    let found = find_awaiting_replies(&store, &account, &prefs, NOW);
    record_awaiting_replies(&store, &found, NOW);
    assert!(maybe_record_briefing(
        &store,
        &prefs,
        "2027-01-05",
        9 * 60,
        NOW + 86_400
    ));
    let latest = store
        .list_agent_events(10, None)
        .unwrap()
        .into_iter()
        .find(|event| event.kind == "briefing" && event.dedupe_key.ends_with("01-05"))
        .unwrap();
    assert_eq!(latest.push_state, "pending");
    assert_eq!(latest.payload["awaitingReply"].as_array().unwrap().len(), 1);

    assert!(
        !maybe_record_briefing(&store, &prefs, "2027-01-06", 15 * 60, NOW),
        "missed window"
    );
    let off = NotifyPrefsRecord {
        briefing_enabled: false,
        ..prefs
    };
    assert!(!maybe_record_briefing(
        &store,
        &off,
        "2027-01-07",
        9 * 60,
        NOW
    ));
}

#[test]
fn notification_text_summarises_groups() {
    let labels = crate::commands::notifications::NotificationLabels::default();
    let event = |kind: &str, payload: serde_json::Value| AgentEvent {
        id: "e".into(),
        kind: kind.into(),
        dedupe_key: "k".into(),
        account_id: None,
        message_id: None,
        thread_id: None,
        watch_id: None,
        conversation_id: None,
        payload,
        created_at: 0,
        read_at: None,
        dismissed_at: None,
        push_state: "pending".into(),
        pushed_at: None,
        resolution: None,
    };
    let a = event(
        "awaiting_reply",
        json!({"from": "Ann", "subject": "Dinner?"}),
    );
    let b = event("awaiting_reply", json!({"from": "Bo", "subject": "Lunch?"}));
    let o = event("watch_overdue", json!({"title": "Proposal"}));
    assert_eq!(
        notification_text(&[&a], &labels),
        "Ann is waiting for your reply: Dinner?"
    );
    assert_eq!(
        notification_text(&[&a, &b], &labels),
        "2 messages are waiting for your reply"
    );
    assert_eq!(
        notification_text(&[&a, &o], &labels),
        "2 new messages from Pebble"
    );
    assert_eq!(
        notification_text(&[&o], &labels),
        "Still no reply: Proposal"
    );
}

#[test]
fn remind_me_later_raises_unanswered_mail_again_once() {
    let (store, account) = setup();
    known(&store, "ann@friends.example");
    let ask = put(
        &store,
        "ann@friends.example",
        "Contract?",
        "Could you sign the contract?",
        30,
        "t1",
    );
    let found = find_awaiting_replies(&store, &account, &NotifyPrefsRecord::default(), NOW);
    record_awaiting_replies(&store, &found, NOW);
    let event = store.list_agent_events(10, None).unwrap().remove(0);
    store
        .resolve_agent_event(
            &event.id,
            &json!({"action": "later", "until": NOW + 86_400}),
            NOW,
        )
        .unwrap();
    assert_eq!(record_later_reminders(&store, NOW + 3600), 0, "not yet");
    assert_eq!(record_later_reminders(&store, NOW + 86_401), 1);
    assert_eq!(record_later_reminders(&store, NOW + 90_000), 0, "only once");
    let again = store.list_agent_events(10, None).unwrap().remove(0);
    assert_eq!(again.payload["again"], true);
    assert_eq!(again.message_id.as_deref(), Some(ask.id.as_str()));

    // Answered in the meantime: no second reminder.
    let (store, account) = setup();
    known(&store, "ann@friends.example");
    put(
        &store,
        "ann@friends.example",
        "Contract?",
        "Could you sign the contract?",
        30,
        "t1",
    );
    let found = find_awaiting_replies(&store, &account, &NotifyPrefsRecord::default(), NOW);
    record_awaiting_replies(&store, &found, NOW);
    let event = store.list_agent_events(10, None).unwrap().remove(0);
    store
        .resolve_agent_event(
            &event.id,
            &json!({"action": "later", "until": NOW + 10}),
            NOW,
        )
        .unwrap();
    put(
        &store,
        "me@pebble.example",
        "Re: Contract?",
        "Signed.",
        1,
        "t1",
    );
    assert_eq!(record_later_reminders(&store, NOW + 20), 0);
}

#[test]
fn answered_mail_settles_its_reminder() {
    let (store, account) = setup();
    known(&store, "ann@friends.example");
    put(
        &store,
        "ann@friends.example",
        "Contract?",
        "Could you sign the contract?",
        30,
        "t1",
    );
    let found = find_awaiting_replies(&store, &account, &NotifyPrefsRecord::default(), NOW);
    record_awaiting_replies(&store, &found, NOW);
    assert_eq!(settle_answered(&store, NOW), 0);
    put(
        &store,
        "me@pebble.example",
        "Re: Contract?",
        "Signed.",
        1,
        "t1",
    );
    assert_eq!(settle_answered(&store, NOW), 1);
    let event = store.list_agent_events(10, None).unwrap().remove(0);
    assert_eq!(event.resolution.unwrap()["action"], "answered");
    assert!(
        store.pending_agent_pushes().unwrap().is_empty(),
        "no push for settled mail"
    );
    assert_eq!(
        build_briefing(&store, NOW)["awaitingReply"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
}

#[test]
fn overdue_follow_ups_link_to_the_sent_mail() {
    let (store, _) = setup();
    let sent = put_to(
        &store,
        "me@pebble.example",
        "Proposal",
        "Here is the proposal.",
        60,
        "t-sent",
        &["dana@partner.example"],
        &[],
    );
    let watch = store
        .create_thread_watch(
            &NewThreadWatch {
                account_id: "acct".into(),
                subject: "Proposal".into(),
                participants: vec!["dana@partner.example".into()],
                anchor_message_id: Some("send:job".into()),
                deadline: Some(NOW - 10),
                ..Default::default()
            },
            NOW - 61 * HOUR,
        )
        .unwrap();
    assert_eq!(
        store.watch_link_message(&watch).unwrap().as_deref(),
        Some(sent.id.as_str())
    );
    record_overdue_follow_ups(&store, NOW);
    let event = store.list_agent_events(10, None).unwrap().remove(0);
    assert_eq!(event.message_id.as_deref(), Some(sent.id.as_str()));
    assert_eq!(event.payload["subject"], "Proposal");
}

#[test]
fn briefing_lists_important_unread_mail_to_open() {
    let (store, _) = setup();
    let important = put(
        &store,
        "boss@corp.example",
        "Board deck",
        "Need it by noon",
        2,
        "t1",
    );
    store
        .insert_notification_log(&pebble_store::notify::NewNotificationLog {
            account_id: "acct",
            message_key: "k1",
            message_id: &important.id,
            tier: "interrupt",
            reason: "vip",
            why: None,
            created_at: NOW - 3600,
        })
        .unwrap();
    let briefing = build_briefing(&store, NOW);
    assert_eq!(briefing["importantUnread"], 1);
    assert_eq!(briefing["important"][0]["messageId"], important.id.as_str());
    assert_eq!(briefing["important"][0]["subject"], "Board deck");
}
