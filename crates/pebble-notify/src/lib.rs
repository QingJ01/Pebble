//! Notification triage for new mail.
//!
//! [`decide`] is a pure function: given what is known about one newly
//! synced message (after the user's rules ran) and the user's preferences,
//! it returns whether to interrupt now, add the message to the digest, or
//! stay silent, plus a stable reason code. The app gathers the inputs from
//! the store; the evaluation harness feeds synthetic ones. Keeping it pure
//! means the evaluated logic is exactly the shipped logic.

pub mod awaiting;
pub mod deadline;
pub mod delivery;
pub mod sender;
pub mod triage_model;

use pebble_core::MailSignals;
use serde::{Deserialize, Serialize};

pub use sender::is_automated_sender;

/// What to do with a new message.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Tier {
    /// Show a system notification now.
    Interrupt,
    /// No interruption; list it in the in-app digest.
    Digest,
    /// Neither. The message is still in the mailbox as usual.
    Silent,
}

/// Why a [`Tier`] was chosen. Serialised as a stable snake_case code that is
/// stored in the decision log and shown (translated) in settings.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Reason {
    /// Not new mail: first sync, history backfill, repair, draft, deleted.
    Backfill,
    /// Not in the inbox (anymore): a rule or the server moved it.
    Moved,
    /// In spam or trash.
    Spam,
    /// A user rule marked it read.
    RuleRead,
    /// Sender or domain is muted.
    Muted,
    /// Sent by this account.
    SelfSent,
    /// A reply on a thread the user is following.
    Watch,
    /// A reply to mail the user sent.
    ReplyToMe,
    /// Sender is marked important (VIP or favourite contact).
    Vip,
    /// Automated, mailing-list or provider low-priority mail.
    Bulk,
    /// Personal mail sent directly to the user by someone they know.
    Direct,
    /// Everything else: the account's notification mode decided.
    Default,
    /// The model judged it (label from [`ModelLabel`]).
    Model(ModelLabel),
    /// Model assist was on but unavailable; rule result used.
    ModelFallback,
    /// Waiting for the model's answer (the notification is held).
    ModelPending,
}

impl Reason {
    pub fn code(&self) -> String {
        match self {
            Reason::Model(label) => format!("model:{}", label.code()),
            other => serde_json::to_value(other)
                .ok()
                .and_then(|value| value.as_str().map(str::to_string))
                .unwrap_or_else(|| "default".into()),
        }
    }
}

/// Categories a model may assign (fixed list, never free text).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelLabel {
    NeedsAction,
    TimeSensitive,
    Personal,
    Notification,
    Newsletter,
    Promotion,
    Other,
}

impl ModelLabel {
    pub fn code(self) -> &'static str {
        match self {
            ModelLabel::NeedsAction => "needs_action",
            ModelLabel::TimeSensitive => "time_sensitive",
            ModelLabel::Personal => "personal",
            ModelLabel::Notification => "notification",
            ModelLabel::Newsletter => "newsletter",
            ModelLabel::Promotion => "promotion",
            ModelLabel::Other => "other",
        }
    }

    pub fn parse(code: &str) -> Option<Self> {
        Some(match code.trim().to_ascii_lowercase().as_str() {
            "needs_action" => ModelLabel::NeedsAction,
            "time_sensitive" => ModelLabel::TimeSensitive,
            "personal" => ModelLabel::Personal,
            "notification" => ModelLabel::Notification,
            "newsletter" => ModelLabel::Newsletter,
            "promotion" => ModelLabel::Promotion,
            "other" => ModelLabel::Other,
            _ => return None,
        })
    }
}

/// Per-account notification mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    /// Every new inbox message interrupts (behaviour before smart
    /// notifications; the default for upgraded installs).
    #[default]
    All,
    /// Direct personal mail interrupts; bulk and unknown go to the digest.
    Smart,
    /// Only followed threads, replies to the user and VIPs interrupt.
    ImportantOnly,
}

/// Quiet hours in local minutes since midnight. `start == end` disables.
/// A range may wrap midnight (22:00–07:00).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct QuietHours {
    pub start_minute: u16,
    pub end_minute: u16,
    /// Followed-thread replies may still interrupt.
    #[serde(default = "yes")]
    pub allow_watch: bool,
    /// VIP senders may still interrupt.
    #[serde(default = "yes")]
    pub allow_vip: bool,
}

fn yes() -> bool {
    true
}

impl QuietHours {
    pub fn contains(&self, minute_of_day: u16) -> bool {
        let (s, e) = (self.start_minute, self.end_minute);
        if s == e {
            false
        } else if s < e {
            minute_of_day >= s && minute_of_day < e
        } else {
            minute_of_day >= s || minute_of_day < e
        }
    }
}

/// Explicit per-sender preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SenderPref {
    Vip,
    Mute,
}

/// Everything known about one new message when deciding.
#[derive(Debug, Clone, Default)]
pub struct MessageFacts {
    /// The sync marked it as new mail (not first sync / backfill / repair),
    /// and it is not a draft or deleted.
    pub is_new: bool,
    /// Still filed in the inbox after the user's rules ran.
    pub in_inbox: bool,
    pub in_spam_or_trash: bool,
    /// A rule marked it read.
    pub rule_marked_read: bool,
    /// A rule moved or archived it.
    pub rule_moved: bool,
    pub from_address: String,
    pub from_name: String,
    /// The sender is this account (or one of its addresses).
    pub from_self: bool,
    /// The account address is in To (not only Cc/Bcc/list).
    pub to_me: bool,
    pub sender_pref: Option<SenderPref>,
    pub sender_is_favorite: bool,
    /// The user has written to this sender before.
    pub sender_known: bool,
    /// It answers a message this account sent.
    pub reply_to_me: bool,
    /// It belongs to a thread with an active follow-up.
    pub watched_thread: bool,
    pub signals: MailSignals,
}

/// User preferences that apply to the message's account.
#[derive(Debug, Clone, Default)]
pub struct Prefs {
    pub mode: Mode,
    pub quiet_hours: Option<QuietHours>,
    /// Also send bulk mail (rule 9) to the model when model assist is on.
    pub model_reviews_bulk: bool,
}

/// The rule-based decision plus whether the model may refine it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Decision {
    pub tier: Tier,
    pub reason: Reason,
    /// Tier before quiet hours were applied.
    pub base_tier: Tier,
    /// The rules could not tell; model assist (if enabled) may decide
    /// between interrupt and digest.
    pub model_candidate: bool,
}

/// Whether a message looks machine-sent (headers, provider category or a
/// no-reply style address).
pub fn is_automated(facts: &MessageFacts) -> bool {
    facts.signals.auto_submitted
        || facts.signals.bulk
        || facts.signals.provider_low_priority
        || is_automated_sender(&facts.from_address, &facts.from_name)
}

/// Decide what to do with one new message. Rules are checked in order and
/// the first match wins; see the design in issue #107.
pub fn decide(facts: &MessageFacts, prefs: &Prefs, local_minute_of_day: u16) -> Decision {
    let base = |tier: Tier, reason: Reason| Decision {
        tier,
        reason,
        base_tier: tier,
        model_candidate: false,
    };

    // 1–5: never interrupt, never sent to a model.
    if !facts.is_new {
        return base(Tier::Silent, Reason::Backfill);
    }
    if facts.in_spam_or_trash {
        return base(Tier::Silent, Reason::Spam);
    }
    if !facts.in_inbox || facts.rule_moved {
        return base(Tier::Silent, Reason::Moved);
    }
    if facts.rule_marked_read {
        return base(Tier::Silent, Reason::RuleRead);
    }
    if facts.sender_pref == Some(SenderPref::Mute) {
        return base(Tier::Silent, Reason::Muted);
    }
    if facts.from_self {
        return base(Tier::Silent, Reason::SelfSent);
    }

    let automated = is_automated(facts);
    // A person's reply on a mailing list still carries list headers; only
    // auto-replies and bots disqualify a reply.
    let machine_reply =
        facts.signals.auto_submitted || is_automated_sender(&facts.from_address, &facts.from_name);

    // 6–8: important regardless of mode.
    let important = if facts.watched_thread && !machine_reply {
        Some(Reason::Watch)
    } else if facts.reply_to_me && !machine_reply {
        Some(Reason::ReplyToMe)
    } else if facts.sender_pref == Some(SenderPref::Vip) || facts.sender_is_favorite {
        Some(Reason::Vip)
    } else {
        None
    };

    let mut decision = match important {
        Some(reason) => base(Tier::Interrupt, reason),
        None if automated => {
            // 9: bulk.
            let tier = if prefs.mode == Mode::All {
                Tier::Interrupt
            } else {
                Tier::Digest
            };
            Decision {
                tier,
                reason: Reason::Bulk,
                base_tier: tier,
                model_candidate: prefs.mode != Mode::All && prefs.model_reviews_bulk,
            }
        }
        None => {
            // 10: the account's mode.
            match prefs.mode {
                Mode::All => base(Tier::Interrupt, Reason::Default),
                Mode::ImportantOnly => Decision {
                    tier: Tier::Digest,
                    reason: Reason::Default,
                    base_tier: Tier::Digest,
                    model_candidate: true,
                },
                Mode::Smart => {
                    let direct = facts.to_me && facts.sender_known;
                    let tier = if direct {
                        Tier::Interrupt
                    } else {
                        Tier::Digest
                    };
                    Decision {
                        tier,
                        reason: if direct {
                            Reason::Direct
                        } else {
                            Reason::Default
                        },
                        base_tier: tier,
                        model_candidate: true,
                    }
                }
            }
        }
    };

    apply_quiet_hours(&mut decision, prefs, local_minute_of_day);
    decision
}

/// Quiet hours turn an interruption into a digest entry, except for the
/// categories the user allowed.
pub fn apply_quiet_hours(decision: &mut Decision, prefs: &Prefs, local_minute_of_day: u16) {
    let Some(quiet) = prefs.quiet_hours else {
        return;
    };
    if decision.tier != Tier::Interrupt || !quiet.contains(local_minute_of_day) {
        return;
    }
    let allowed = match decision.reason {
        Reason::Watch | Reason::ReplyToMe => quiet.allow_watch,
        Reason::Vip => quiet.allow_vip,
        _ => false,
    };
    if !allowed {
        decision.tier = Tier::Digest;
    }
}

/// Apply a model verdict to a rule decision that was a model candidate.
/// The model can only choose between interrupt and digest; it can never
/// silence a message. Low confidence falls back to the digest.
pub fn apply_model_verdict(
    decision: &Decision,
    verdict: &ModelVerdict,
    prefs: &Prefs,
    local_minute_of_day: u16,
    min_confidence: f32,
) -> Decision {
    if !decision.model_candidate {
        return decision.clone();
    }
    // An unsure model changes nothing: the rules decide. A sure one may
    // promote or demote (never silence).
    if verdict.confidence < min_confidence {
        return Decision {
            model_candidate: false,
            ..decision.clone()
        };
    }
    let tier = if verdict.interrupt {
        Tier::Interrupt
    } else {
        Tier::Digest
    };
    let mut next = Decision {
        tier,
        reason: Reason::Model(verdict.label),
        base_tier: tier,
        model_candidate: false,
    };
    apply_quiet_hours(&mut next, prefs, local_minute_of_day);
    next
}

/// A parsed model answer for one message.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelVerdict {
    pub interrupt: bool,
    pub label: ModelLabel,
    pub confidence: f32,
    /// Short "why now" line (≤ 40 chars after trimming), shown as AI text.
    pub why: Option<String>,
}

#[cfg(test)]
mod tests;
