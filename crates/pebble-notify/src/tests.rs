use super::*;

fn person() -> MessageFacts {
    MessageFacts {
        is_new: true,
        in_inbox: true,
        from_address: "ann@example.com".into(),
        from_name: "Ann".into(),
        to_me: true,
        ..MessageFacts::default()
    }
}

fn prefs(mode: Mode) -> Prefs {
    Prefs {
        mode,
        ..Prefs::default()
    }
}

const NOON: u16 = 12 * 60;

#[test]
fn hard_rules_silence_in_every_mode() {
    for mode in [Mode::All, Mode::Smart, Mode::ImportantOnly] {
        let p = prefs(mode);
        let cases: Vec<(MessageFacts, Reason)> = vec![
            (
                MessageFacts {
                    is_new: false,
                    ..person()
                },
                Reason::Backfill,
            ),
            (
                MessageFacts {
                    in_spam_or_trash: true,
                    ..person()
                },
                Reason::Spam,
            ),
            (
                MessageFacts {
                    in_inbox: false,
                    ..person()
                },
                Reason::Moved,
            ),
            (
                MessageFacts {
                    rule_moved: true,
                    ..person()
                },
                Reason::Moved,
            ),
            (
                MessageFacts {
                    rule_marked_read: true,
                    ..person()
                },
                Reason::RuleRead,
            ),
            (
                MessageFacts {
                    sender_pref: Some(SenderPref::Mute),
                    ..person()
                },
                Reason::Muted,
            ),
            (
                MessageFacts {
                    from_self: true,
                    ..person()
                },
                Reason::SelfSent,
            ),
            // Mute wins over VIP-like signals and follow-ups.
            (
                MessageFacts {
                    sender_pref: Some(SenderPref::Mute),
                    watched_thread: true,
                    sender_is_favorite: true,
                    ..person()
                },
                Reason::Muted,
            ),
        ];
        for (facts, reason) in cases {
            let d = decide(&facts, &p, NOON);
            assert_eq!(d.tier, Tier::Silent, "{mode:?} {reason:?}");
            assert_eq!(d.reason, reason);
            assert!(!d.model_candidate);
        }
    }
}

#[test]
fn important_mail_interrupts_in_every_mode() {
    for mode in [Mode::All, Mode::Smart, Mode::ImportantOnly] {
        let p = prefs(mode);
        assert_eq!(
            decide(
                &MessageFacts {
                    watched_thread: true,
                    ..person()
                },
                &p,
                NOON
            )
            .reason,
            Reason::Watch
        );
        assert_eq!(
            decide(
                &MessageFacts {
                    reply_to_me: true,
                    ..person()
                },
                &p,
                NOON
            )
            .reason,
            Reason::ReplyToMe
        );
        let vip = decide(
            &MessageFacts {
                sender_pref: Some(SenderPref::Vip),
                ..person()
            },
            &p,
            NOON,
        );
        assert_eq!((vip.tier, vip.reason), (Tier::Interrupt, Reason::Vip));
        assert_eq!(
            decide(
                &MessageFacts {
                    sender_is_favorite: true,
                    ..person()
                },
                &p,
                NOON
            )
            .reason,
            Reason::Vip
        );
    }
}

#[test]
fn automated_replies_on_followed_threads_do_not_count() {
    let auto_reply = MessageFacts {
        watched_thread: true,
        signals: MailSignals {
            auto_submitted: true,
            ..MailSignals::default()
        },
        ..person()
    };
    let d = decide(&auto_reply, &prefs(Mode::Smart), NOON);
    assert_eq!((d.tier, d.reason), (Tier::Digest, Reason::Bulk));
}

#[test]
fn a_person_replying_on_a_mailing_list_still_counts() {
    let list_reply = MessageFacts {
        reply_to_me: true,
        signals: MailSignals {
            bulk: true,
            provider_low_priority: true,
            ..MailSignals::default()
        },
        ..person()
    };
    let d = decide(&list_reply, &prefs(Mode::ImportantOnly), NOON);
    assert_eq!((d.tier, d.reason), (Tier::Interrupt, Reason::ReplyToMe));
    let watched = MessageFacts {
        reply_to_me: false,
        watched_thread: true,
        ..list_reply
    };
    assert_eq!(
        decide(&watched, &prefs(Mode::Smart), NOON).reason,
        Reason::Watch
    );
}

#[test]
fn modes_handle_ordinary_and_bulk_mail() {
    let bulk = MessageFacts {
        from_address: "news@shop.example".into(),
        ..person()
    };
    let unknown = person();
    let known = MessageFacts {
        sender_known: true,
        ..person()
    };
    let cc_known = MessageFacts {
        sender_known: true,
        to_me: false,
        ..person()
    };

    let all = prefs(Mode::All);
    for facts in [&bulk, &unknown, &known] {
        assert_eq!(decide(facts, &all, NOON).tier, Tier::Interrupt);
        assert!(!decide(facts, &all, NOON).model_candidate);
    }

    let smart = prefs(Mode::Smart);
    assert_eq!(decide(&bulk, &smart, NOON).tier, Tier::Digest);
    assert!(!decide(&bulk, &smart, NOON).model_candidate);
    let d = decide(&known, &smart, NOON);
    assert_eq!(
        (d.tier, d.reason.clone(), d.model_candidate),
        (Tier::Interrupt, Reason::Direct, true)
    );
    assert_eq!(decide(&unknown, &smart, NOON).tier, Tier::Digest);
    assert_eq!(decide(&cc_known, &smart, NOON).tier, Tier::Digest);

    let important = prefs(Mode::ImportantOnly);
    assert_eq!(decide(&known, &important, NOON).tier, Tier::Digest);
    assert_eq!(decide(&bulk, &important, NOON).tier, Tier::Digest);
}

#[test]
fn bulk_goes_to_the_model_only_when_asked() {
    let bulk = MessageFacts {
        signals: MailSignals {
            bulk: true,
            ..MailSignals::default()
        },
        ..person()
    };
    let mut p = prefs(Mode::Smart);
    assert!(!decide(&bulk, &p, NOON).model_candidate);
    p.model_reviews_bulk = true;
    assert!(decide(&bulk, &p, NOON).model_candidate);
}

#[test]
fn quiet_hours_wrap_midnight_and_respect_exceptions() {
    let quiet = QuietHours {
        start_minute: 22 * 60,
        end_minute: 7 * 60,
        allow_watch: true,
        allow_vip: false,
    };
    assert!(quiet.contains(23 * 60));
    assert!(quiet.contains(3 * 60));
    assert!(!quiet.contains(12 * 60));
    assert!(!QuietHours {
        start_minute: 60,
        end_minute: 60,
        allow_watch: true,
        allow_vip: true
    }
    .contains(60));

    let p = Prefs {
        mode: Mode::All,
        quiet_hours: Some(quiet),
        ..Prefs::default()
    };
    let night = 23 * 60;
    let d = decide(&person(), &p, night);
    assert_eq!((d.tier, d.base_tier), (Tier::Digest, Tier::Interrupt));
    assert_eq!(
        decide(
            &MessageFacts {
                watched_thread: true,
                ..person()
            },
            &p,
            night
        )
        .tier,
        Tier::Interrupt
    );
    assert_eq!(
        decide(
            &MessageFacts {
                sender_is_favorite: true,
                ..person()
            },
            &p,
            night
        )
        .tier,
        Tier::Digest
    );
    assert_eq!(decide(&person(), &p, NOON).tier, Tier::Interrupt);
}

#[test]
fn model_can_promote_or_demote_but_never_silence() {
    let p = prefs(Mode::Smart);
    let d = decide(&person(), &p, NOON);
    assert!(d.model_candidate);
    let yes = ModelVerdict {
        interrupt: true,
        label: ModelLabel::NeedsAction,
        confidence: 0.9,
        why: None,
    };
    let promoted = apply_model_verdict(&d, &yes, &p, NOON, 0.6);
    assert_eq!(
        (promoted.tier, promoted.reason.code()),
        (Tier::Interrupt, "model:needs_action".to_string())
    );

    let unsure = ModelVerdict {
        confidence: 0.3,
        ..yes.clone()
    };
    assert_eq!(
        apply_model_verdict(&d, &unsure, &p, NOON, 0.6).tier,
        Tier::Digest
    );
    // An unsure "digest" can't demote what the rules would notify.
    let direct = decide(
        &MessageFacts {
            sender_known: true,
            ..person()
        },
        &p,
        NOON,
    );
    assert_eq!(direct.tier, Tier::Interrupt);
    let unsure_no = ModelVerdict {
        interrupt: false,
        confidence: 0.2,
        ..yes.clone()
    };
    let kept = apply_model_verdict(&direct, &unsure_no, &p, NOON, 0.6);
    assert_eq!((kept.tier, kept.reason), (Tier::Interrupt, Reason::Direct));

    let no = ModelVerdict {
        interrupt: false,
        label: ModelLabel::Promotion,
        confidence: 0.99,
        why: None,
    };
    let known = decide(
        &MessageFacts {
            sender_known: true,
            ..person()
        },
        &p,
        NOON,
    );
    assert_eq!(
        apply_model_verdict(&known, &no, &p, NOON, 0.6).tier,
        Tier::Digest
    );

    // Not a candidate: unchanged.
    let vip = decide(
        &MessageFacts {
            sender_is_favorite: true,
            ..person()
        },
        &p,
        NOON,
    );
    assert_eq!(apply_model_verdict(&vip, &no, &p, NOON, 0.6), vip);
}

#[test]
fn reason_codes_are_stable() {
    assert_eq!(Reason::RuleRead.code(), "rule_read");
    assert_eq!(Reason::SelfSent.code(), "self_sent");
    assert_eq!(Reason::ReplyToMe.code(), "reply_to_me");
    assert_eq!(
        Reason::Model(ModelLabel::TimeSensitive).code(),
        "model:time_sensitive"
    );
}
