//! Evaluation sets and scoring for smart notifications (#107).
//!
//! The synthetic datasets live in `crates/pebble-notify/eval/`. The same
//! code scores the shipped rules (a regression test runs it on every
//! build) and, through the `notify-eval` tool, the rules plus a model the
//! developer supplies. Everything evaluated is the shipped logic:
//! [`crate::decide`], [`crate::apply_model_verdict`],
//! [`crate::deadline::find_deadline`] and [`crate::awaiting`].

use std::collections::BTreeMap;

use chrono::NaiveDateTime;
use pebble_core::MailSignals;
use serde::Deserialize;

use crate::awaiting::{awaiting_score, request_strength, AwaitingFeatures, AWAITING_THRESHOLD};
use crate::{decide, Decision, MessageFacts, Mode, Prefs, SenderPref, Tier};

/// Local minute used for triage (no quiet hours in the sets).
pub const EVAL_MINUTE: u16 = 10 * 60;

// ------------------------------------------------------------------ triage

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CaseMail {
    pub from_address: String,
    pub from_name: String,
    pub subject: String,
    pub body: String,
    pub to_me: bool,
}

/// What the store would know about the message (all off by default).
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(default)]
pub struct CaseFacts {
    pub sender_known: bool,
    pub times_user_wrote: u32,
    pub favorite: bool,
    pub vip: bool,
    pub muted: bool,
    pub reply_to_me: bool,
    pub watched: bool,
    pub auto_submitted: bool,
    pub bulk: bool,
    pub low_priority: bool,
    pub rule_read: bool,
    pub rule_moved: bool,
    pub spam: bool,
    pub from_self: bool,
    pub backfill: bool,
}

#[derive(Debug, Clone, Deserialize)]
pub struct NotifyCase {
    pub id: String,
    pub lang: String,
    pub category: String,
    /// What a careful person would want in Smart mode.
    pub expected: Tier,
    #[serde(default)]
    pub note: String,
    pub mail: CaseMail,
    #[serde(default)]
    pub facts: CaseFacts,
}

impl NotifyCase {
    pub fn facts(&self) -> MessageFacts {
        let f = &self.facts;
        MessageFacts {
            is_new: !f.backfill,
            in_inbox: !f.spam && !f.rule_moved,
            in_spam_or_trash: f.spam,
            rule_marked_read: f.rule_read,
            rule_moved: f.rule_moved,
            from_address: self.mail.from_address.clone(),
            from_name: self.mail.from_name.clone(),
            from_self: f.from_self,
            to_me: self.mail.to_me,
            sender_pref: if f.muted {
                Some(SenderPref::Mute)
            } else if f.vip {
                Some(SenderPref::Vip)
            } else {
                None
            },
            sender_is_favorite: f.favorite,
            sender_known: f.sender_known || f.times_user_wrote > 0,
            reply_to_me: f.reply_to_me,
            watched_thread: f.watched,
            signals: MailSignals {
                auto_submitted: f.auto_submitted,
                bulk: f.bulk,
                provider_low_priority: f.low_priority,
            },
        }
    }

    /// The rule decision in `mode`.
    pub fn decide(&self, prefs: &Prefs) -> Decision {
        decide(&self.facts(), prefs, EVAL_MINUTE)
    }
}

pub fn smart_prefs(model_reviews_bulk: bool) -> Prefs {
    Prefs {
        mode: Mode::Smart,
        quiet_hours: None,
        model_reviews_bulk,
    }
}

fn tier_index(tier: Tier) -> usize {
    match tier {
        Tier::Interrupt => 0,
        Tier::Digest => 1,
        Tier::Silent => 2,
    }
}

pub fn tier_name(tier: Tier) -> &'static str {
    ["interrupt", "digest", "silent"][tier_index(tier)]
}

/// Confusion counts and the derived numbers that matter for notifications.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TriageScore {
    /// `[expected][got]`, in interrupt / digest / silent order.
    pub confusion: [[u32; 3]; 3],
}

impl TriageScore {
    pub fn add(&mut self, expected: Tier, got: Tier) {
        self.confusion[tier_index(expected)][tier_index(got)] += 1;
    }

    pub fn total(&self) -> u32 {
        self.confusion.iter().flatten().sum()
    }

    pub fn correct(&self) -> u32 {
        (0..3).map(|i| self.confusion[i][i]).sum()
    }

    pub fn accuracy(&self) -> f64 {
        ratio(self.correct(), self.total())
    }

    /// Of the notifications shown, how many were wanted.
    pub fn interrupt_precision(&self) -> f64 {
        let shown: u32 = (0..3).map(|e| self.confusion[e][0]).sum();
        ratio(self.confusion[0][0], shown)
    }

    /// Of the mail that deserved a notification, how much got one.
    pub fn interrupt_recall(&self) -> f64 {
        ratio(self.confusion[0][0], self.confusion[0].iter().sum())
    }

    pub fn interrupt_f1(&self) -> f64 {
        let (p, r) = (self.interrupt_precision(), self.interrupt_recall());
        if p + r == 0.0 {
            0.0
        } else {
            2.0 * p * r / (p + r)
        }
    }

    /// Wanted a notification, didn't get one.
    pub fn missed(&self) -> u32 {
        self.confusion[0][1] + self.confusion[0][2]
    }

    /// Notified although it could wait.
    pub fn noisy(&self) -> u32 {
        self.confusion[1][0] + self.confusion[2][0]
    }

    /// Mail that should reach the user (interrupt or digest) went silent.
    pub fn lost(&self) -> u32 {
        self.confusion[0][2] + self.confusion[1][2]
    }
}

pub fn ratio(n: u32, d: u32) -> f64 {
    if d == 0 {
        0.0
    } else {
        f64::from(n) / f64::from(d)
    }
}

/// Score `decisions` (one per case, same order) overall and per category.
pub fn score_triage(
    cases: &[NotifyCase],
    decisions: &[Tier],
) -> (TriageScore, BTreeMap<String, TriageScore>) {
    let mut overall = TriageScore::default();
    let mut by_category: BTreeMap<String, TriageScore> = BTreeMap::new();
    for (case, got) in cases.iter().zip(decisions) {
        overall.add(case.expected, *got);
        by_category
            .entry(case.category.clone())
            .or_default()
            .add(case.expected, *got);
    }
    (overall, by_category)
}

// --------------------------------------------------------------- deadlines

#[derive(Debug, Clone, Deserialize)]
pub struct DeadlineSet {
    /// Local time the mail arrived ("YYYY-MM-DD HH:MM").
    pub received: String,
    pub cases: Vec<DeadlineCase>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DeadlineCase {
    pub id: String,
    pub lang: String,
    pub subject: String,
    pub body: String,
    /// "YYYY-MM-DD HH:MM", or none when the mail sets no deadline.
    pub expected: Option<String>,
}

impl DeadlineSet {
    pub fn received_at(&self) -> NaiveDateTime {
        parse_minute(&self.received).expect("valid received time in the deadline set")
    }
}

impl DeadlineCase {
    pub fn expected_at(&self) -> Option<NaiveDateTime> {
        self.expected.as_deref().and_then(parse_minute)
    }
}

pub fn parse_minute(text: &str) -> Option<NaiveDateTime> {
    NaiveDateTime::parse_from_str(text.trim(), "%Y-%m-%d %H:%M").ok()
}

/// How a found deadline compares with the expected one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeadlineOutcome {
    /// Same minute.
    Exact,
    /// Right day, different time.
    SameDay,
    /// A deadline on the wrong day.
    WrongDay,
    /// Expected one, found none.
    Missed,
    /// Found one where there is none.
    FalsePositive,
    /// Correctly found none.
    CorrectNone,
}

pub fn deadline_outcome(
    expected: Option<NaiveDateTime>,
    got: Option<NaiveDateTime>,
) -> DeadlineOutcome {
    match (expected, got) {
        (None, None) => DeadlineOutcome::CorrectNone,
        (None, Some(_)) => DeadlineOutcome::FalsePositive,
        (Some(_), None) => DeadlineOutcome::Missed,
        (Some(e), Some(g)) if e == g => DeadlineOutcome::Exact,
        (Some(e), Some(g)) if e.date() == g.date() => DeadlineOutcome::SameDay,
        _ => DeadlineOutcome::WrongDay,
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeadlineScore {
    pub exact: u32,
    pub same_day: u32,
    pub wrong_day: u32,
    pub missed: u32,
    pub false_positive: u32,
    pub correct_none: u32,
}

impl DeadlineScore {
    pub fn add(&mut self, outcome: DeadlineOutcome) {
        match outcome {
            DeadlineOutcome::Exact => self.exact += 1,
            DeadlineOutcome::SameDay => self.same_day += 1,
            DeadlineOutcome::WrongDay => self.wrong_day += 1,
            DeadlineOutcome::Missed => self.missed += 1,
            DeadlineOutcome::FalsePositive => self.false_positive += 1,
            DeadlineOutcome::CorrectNone => self.correct_none += 1,
        }
    }

    pub fn with_deadline(&self) -> u32 {
        self.exact + self.same_day + self.wrong_day + self.missed
    }

    /// Deadlines found on the right day, of those that exist.
    pub fn day_recall(&self) -> f64 {
        ratio(self.exact + self.same_day, self.with_deadline())
    }

    /// Found deadlines that are on the right day.
    pub fn day_precision(&self) -> f64 {
        ratio(
            self.exact + self.same_day,
            self.exact + self.same_day + self.wrong_day + self.false_positive,
        )
    }
}

/// The shipped rules on one case.
pub fn rule_deadline(case: &DeadlineCase, received: NaiveDateTime) -> Option<NaiveDateTime> {
    crate::deadline::find_deadline(&case.subject, &case.body, received).map(|d| d.due)
}

// ---------------------------------------------------------------- awaiting

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AwaitingCase {
    pub id: String,
    pub lang: String,
    pub subject: String,
    pub body: String,
    pub times_user_wrote: u32,
    pub direct: bool,
    pub only_recipient: bool,
    pub vip: bool,
    pub age_hours: f32,
    /// The sender is waiting on the user; a nudge is welcome.
    pub expected: bool,
    #[serde(default)]
    pub note: String,
}

impl AwaitingCase {
    pub fn score(&self) -> f32 {
        awaiting_score(&AwaitingFeatures {
            request: Some(request_strength(&self.subject, &self.body)),
            sent_count: self.times_user_wrote,
            direct: self.direct,
            only_recipient: self.only_recipient,
            vip: self.vip,
            age_hours: self.age_hours,
        })
    }

    pub fn nudges(&self) -> bool {
        self.score() >= AWAITING_THRESHOLD
    }
}

/// True/false positives and negatives of a yes/no decision.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BinaryScore {
    pub tp: u32,
    pub fp: u32,
    pub tn: u32,
    pub fn_: u32,
}

impl BinaryScore {
    pub fn add(&mut self, expected: bool, got: bool) {
        match (expected, got) {
            (true, true) => self.tp += 1,
            (false, true) => self.fp += 1,
            (false, false) => self.tn += 1,
            (true, false) => self.fn_ += 1,
        }
    }
    pub fn precision(&self) -> f64 {
        ratio(self.tp, self.tp + self.fp)
    }
    pub fn recall(&self) -> f64 {
        ratio(self.tp, self.tp + self.fn_)
    }
}

// ----------------------------------------------------------------- loading

/// The datasets shipped in the crate.
pub fn bundled_notify_cases() -> Vec<NotifyCase> {
    serde_json::from_str(include_str!("../eval/notify_cases.json"))
        .expect("valid notify_cases.json")
}

pub fn bundled_deadline_set() -> DeadlineSet {
    serde_json::from_str(include_str!("../eval/deadline_cases.json"))
        .expect("valid deadline_cases.json")
}

pub fn bundled_awaiting_cases() -> Vec<AwaitingCase> {
    serde_json::from_str(include_str!("../eval/awaiting_cases.json"))
        .expect("valid awaiting_cases.json")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scores_count_the_errors_that_matter() {
        let mut s = TriageScore::default();
        s.add(Tier::Interrupt, Tier::Interrupt);
        s.add(Tier::Interrupt, Tier::Digest);
        s.add(Tier::Digest, Tier::Interrupt);
        s.add(Tier::Digest, Tier::Silent);
        assert_eq!((s.missed(), s.noisy(), s.lost()), (1, 1, 1));
        assert_eq!(s.interrupt_precision(), 0.5);
        assert_eq!(s.interrupt_recall(), 0.5);
        assert_eq!(s.accuracy(), 0.25);

        let at = |t: &str| parse_minute(t);
        assert_eq!(
            deadline_outcome(at("2026-12-04 18:00"), at("2026-12-04 17:00")),
            DeadlineOutcome::SameDay
        );
        assert_eq!(
            deadline_outcome(None, at("2026-12-04 17:00")),
            DeadlineOutcome::FalsePositive
        );
    }

    #[test]
    fn bundled_sets_load() {
        assert!(bundled_notify_cases().len() >= 70);
        assert!(bundled_deadline_set().cases.len() >= 30);
        assert!(bundled_awaiting_cases().len() >= 20);
    }
}
