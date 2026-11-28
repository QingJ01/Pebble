//! Regression gate for the rule-based parts of smart notifications, on the
//! synthetic sets in `eval/`. The floors are the measured numbers when the
//! sets were written; raise them when the rules improve, never lower them
//! to make a change pass. `cargo run -p pebble-notify-eval` prints the full
//! report (and compares a model when one is configured).

use pebble_notify::eval::{
    bundled_awaiting_cases, bundled_deadline_set, bundled_notify_cases, deadline_outcome,
    rule_deadline, score_triage, smart_prefs, BinaryScore, DeadlineScore,
};
use pebble_notify::Tier;

#[test]
fn triage_never_loses_mail_and_keeps_its_accuracy() {
    let cases = bundled_notify_cases();
    let prefs = smart_prefs(false);
    let tiers: Vec<Tier> = cases.iter().map(|c| c.decide(&prefs).tier).collect();
    let (overall, by_category) = score_triage(&cases, &tiers);

    assert_eq!(
        overall.lost(),
        0,
        "mail that should reach the user went silent"
    );
    for certain in [
        "rule_silent",
        "rule_interrupt",
        "bulk_noise",
        "known_ask",
        "unknown_cold",
        "injection",
    ] {
        let s = &by_category[certain];
        assert_eq!(
            s.correct(),
            s.total(),
            "{certain}: the rules should get every case right"
        );
    }
    assert!(
        overall.accuracy() >= 0.64,
        "accuracy fell to {:.2}",
        overall.accuracy()
    );
    assert!(
        overall.noisy() <= 8,
        "{} noisy notifications",
        overall.noisy()
    );
    assert!(
        overall.missed() <= 20,
        "{} missed notifications",
        overall.missed()
    );
}

#[test]
fn model_candidates_cover_everything_the_rules_get_wrong() {
    // The model can only help where it is asked; with bulk review on, every
    // rule mistake in the set must be a model candidate.
    let cases = bundled_notify_cases();
    let prefs = smart_prefs(true);
    for case in &cases {
        let decision = case.decide(&prefs);
        if decision.tier != case.expected && case.category != "cc" {
            assert!(
                decision.model_candidate,
                "{} is wrong and not sent to the model",
                case.id
            );
        }
    }
}

#[test]
fn deadlines_are_precise() {
    let set = bundled_deadline_set();
    let received = set.received_at();
    let mut score = DeadlineScore::default();
    for case in &set.cases {
        score.add(deadline_outcome(
            case.expected_at(),
            rule_deadline(case, received),
        ));
    }
    assert_eq!(
        score.false_positive, 0,
        "a reminder for a deadline that isn't there"
    );
    assert_eq!(score.wrong_day, 0);
    // Exact, not just the right day: a wrong time of day moves the reminder.
    assert_eq!(score.same_day, 0, "deadlines at the wrong time of day");
    assert!(score.exact >= 18, "found {} deadlines exactly", score.exact);
}

#[test]
fn awaiting_reply_nudges_stay_useful() {
    let mut score = BinaryScore::default();
    for case in bundled_awaiting_cases() {
        score.add(case.expected, case.nudges());
    }
    assert!(
        score.precision() >= 0.85,
        "precision {:.2}",
        score.precision()
    );
    assert!(score.recall() >= 0.9, "recall {:.2}", score.recall());
}
