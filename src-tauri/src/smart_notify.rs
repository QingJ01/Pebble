//! Smart notifications (#107): gathers what the store knows about a newly
//! synced message, runs [`pebble_notify::decide`] after the user's rules,
//! and records the decision so a message is notified at most once.

use std::collections::HashMap;

use pebble_core::{Account, FolderRole, Message};
use pebble_notify::{Decision, MessageFacts, Mode, Prefs, QuietHours, SenderPref, Tier};
use pebble_rules::types::RuleAction;
use pebble_store::Store;
use serde::{Deserialize, Serialize};
use tracing::warn;

/// Stored per-account (and default, `account_id = ""`) preferences.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct NotifyPrefsRecord {
    pub mode: Mode,
    pub quiet_hours: Option<QuietHours>,
    /// Let the configured model decide mail the rules can't (#107 §2.7).
    pub model_assist: bool,
    /// Also send bulk mail to the model.
    pub model_reviews_bulk: bool,
    /// Send the start of the body to the model (otherwise metadata only).
    pub model_send_body: bool,
}

impl Default for NotifyPrefsRecord {
    fn default() -> Self {
        Self {
            mode: Mode::All,
            quiet_hours: None,
            model_assist: false,
            model_reviews_bulk: false,
            model_send_body: true,
        }
    }
}

impl NotifyPrefsRecord {
    pub fn to_prefs(&self) -> Prefs {
        Prefs {
            mode: self.mode,
            quiet_hours: self.quiet_hours,
            model_reviews_bulk: self.model_reviews_bulk,
        }
    }
}

/// Load an account's preferences: its own row, else the defaults row,
/// else [`NotifyPrefsRecord::default`].
pub fn load_prefs(store: &Store, account_id: &str) -> NotifyPrefsRecord {
    let parse = |json: Option<String>| {
        json.and_then(|j| serde_json::from_str::<NotifyPrefsRecord>(&j).ok())
    };
    let defaults = parse(store.get_notify_prefs_json("").ok().flatten()).unwrap_or_default();
    if account_id.is_empty() {
        return defaults;
    }
    // An account row overrides only what is set per account (the mode and
    // AI help); quiet hours and the rest always follow the defaults, so a
    // later change there applies to every account.
    match parse(store.get_notify_prefs_json(account_id).ok().flatten()) {
        Some(own) => NotifyPrefsRecord {
            mode: own.mode,
            model_assist: own.model_assist,
            model_reviews_bulk: own.model_reviews_bulk,
            model_send_body: own.model_send_body,
            ..defaults
        },
        None => defaults,
    }
}

/// Minutes since local midnight.
pub fn local_minute_of_day() -> u16 {
    use chrono::Timelike;
    let now = chrono::Local::now();
    (now.hour() * 60 + now.minute()) as u16
}

/// Per-flush caches so a batch does not re-read accounts and folders.
#[derive(Default)]
pub struct TriageCache {
    accounts: HashMap<String, Option<Account>>,
    folder_roles: HashMap<String, HashMap<String, Option<FolderRole>>>,
    prefs: HashMap<String, NotifyPrefsRecord>,
}

impl TriageCache {
    pub fn account(&mut self, store: &Store, account_id: &str) -> Option<Account> {
        self.accounts
            .entry(account_id.to_string())
            .or_insert_with(|| store.get_account(account_id).ok().flatten())
            .clone()
    }

    fn roles(&mut self, store: &Store, account_id: &str) -> &HashMap<String, Option<FolderRole>> {
        self.folder_roles
            .entry(account_id.to_string())
            .or_insert_with(|| {
                store
                    .list_folders(account_id)
                    .map(|folders| folders.into_iter().map(|f| (f.id, f.role)).collect())
                    .unwrap_or_default()
            })
    }

    pub fn prefs(&mut self, store: &Store, account_id: &str) -> NotifyPrefsRecord {
        self.prefs
            .entry(account_id.to_string())
            .or_insert_with(|| load_prefs(store, account_id))
            .clone()
    }
}

/// Whether the message was sent by the account itself.
pub fn is_from_account(message: &Message, account: &Account) -> bool {
    message
        .from_address
        .trim()
        .eq_ignore_ascii_case(account.email.trim())
}

/// What the user's rules did, for the decision.
#[derive(Debug, Clone, Copy, Default)]
pub struct RuleEffect {
    pub marked_read: bool,
    pub moved: bool,
}

impl RuleEffect {
    pub fn from_actions(actions: &[RuleAction]) -> Self {
        let mut effect = Self::default();
        for action in actions {
            match action {
                RuleAction::MarkRead => effect.marked_read = true,
                RuleAction::Archive | RuleAction::MoveToFolder(_) => effect.moved = true,
                RuleAction::AddLabel(_) | RuleAction::SetKanbanColumn(_) => {}
            }
        }
        effect
    }
}

/// Collect the facts for one new message.
pub fn gather_facts(
    store: &Store,
    cache: &mut TriageCache,
    stored: &pebble_mail::StoredMessage,
    rules: RuleEffect,
    watched_thread: bool,
) -> MessageFacts {
    let message = &stored.message;
    let account = cache.account(store, &message.account_id);
    let roles = cache.roles(store, &message.account_id);
    let role_of = |id: &String| roles.get(id).cloned().flatten();
    let in_inbox = stored
        .folder_ids
        .iter()
        .any(|id| role_of(id) == Some(FolderRole::Inbox));
    let in_spam_or_trash = stored
        .folder_ids
        .iter()
        .any(|id| matches!(role_of(id), Some(FolderRole::Spam | FolderRole::Trash)));

    let account_email = account
        .as_ref()
        .map(|a| a.email.trim().to_lowercase())
        .unwrap_or_default();
    let from_self = account
        .as_ref()
        .is_some_and(|a| is_from_account(message, a));
    let to_me = !account_email.is_empty()
        && message
            .to_list
            .iter()
            .any(|addr| addr.address.trim().eq_ignore_ascii_case(&account_email));

    let sender_pref = match store.sender_pref_for(&message.account_id, &message.from_address) {
        Ok(Some(kind)) if kind == "vip" => Some(SenderPref::Vip),
        Ok(Some(kind)) if kind == "mute" => Some(SenderPref::Mute),
        Ok(_) => None,
        Err(e) => {
            warn!("Sender preference lookup failed: {e}");
            None
        }
    };
    let sender_is_favorite = store
        .get_contact_by_email(&message.from_address)
        .ok()
        .flatten()
        .is_some_and(|contact| contact.is_favorite);
    let sender_known = store
        .sent_count_to(&message.account_id, &message.from_address)
        .map(|count| count > 0)
        .unwrap_or(false);
    let reply_to_me = match (&message.in_reply_to, account_email.is_empty()) {
        (Some(parent), false) => store
            .is_reply_to_account(&message.account_id, &account_email, parent)
            .unwrap_or(false),
        _ => false,
    };

    MessageFacts {
        is_new: stored.notify && !message.is_draft && !message.is_deleted,
        in_inbox,
        in_spam_or_trash,
        rule_marked_read: rules.marked_read,
        rule_moved: rules.moved,
        from_address: message.from_address.clone(),
        from_name: message.from_name.clone(),
        from_self,
        to_me,
        sender_pref,
        sender_is_favorite,
        sender_known,
        reply_to_me,
        watched_thread,
        signals: stored.signals,
    }
}

/// A decided message that should be logged (and maybe shown).
#[derive(Debug, Clone)]
pub struct Triaged {
    pub account_id: String,
    pub message_id: String,
    pub message_key: String,
    pub decision: Decision,
}

/// Record a decision. Returns `true` if this is the first decision for the
/// message (so it may be shown); `false` if it was already handled.
pub fn record(store: &Store, triaged: &Triaged, now: i64) -> bool {
    let tier = tier_code(triaged.decision.tier);
    let reason = triaged.decision.reason.code();
    match store.insert_notification_log(&pebble_store::notify::NewNotificationLog {
        account_id: &triaged.account_id,
        message_key: &triaged.message_key,
        message_id: &triaged.message_id,
        tier,
        reason: &reason,
        why: None,
        created_at: now,
    }) {
        Ok(inserted) => inserted,
        Err(e) => {
            // Never lose a notification because the log failed; worst case a
            // retry shows it twice.
            warn!("Failed to record notification decision: {e}");
            true
        }
    }
}

pub fn tier_code(tier: Tier) -> &'static str {
    match tier {
        Tier::Interrupt => "interrupt",
        Tier::Digest => "digest",
        Tier::Silent => "silent",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prefs_default_and_parse() {
        let store = Store::open_in_memory().unwrap();
        // Fresh databases get "smart" from the migration; an explicit row wins.
        assert_eq!(load_prefs(&store, "a1").mode, Mode::Smart);
        store
            .set_notify_prefs_json("a1", r#"{"mode":"importantOnly"}"#, 1)
            .unwrap();
        assert_eq!(
            load_prefs(&store, "a1").mode,
            Mode::Smart,
            "camelCase mode names are rejected"
        );
        store
            .set_notify_prefs_json(
                "a1",
                r#"{"mode":"important_only","quietHours":{"startMinute":1320,"endMinute":420}}"#,
                1,
            )
            .unwrap();
        let prefs = load_prefs(&store, "a1");
        assert_eq!(prefs.mode, Mode::ImportantOnly);
        assert!(
            prefs.quiet_hours.is_none(),
            "quiet hours come from the defaults"
        );
        assert!(prefs.model_send_body);
        store
            .set_notify_prefs_json(
                "",
                r#"{"mode":"all","quietHours":{"startMinute":1320,"endMinute":420}}"#,
                2,
            )
            .unwrap();
        let prefs = load_prefs(&store, "a1");
        assert_eq!(prefs.mode, Mode::ImportantOnly, "the account's mode wins");
        assert!(
            prefs.quiet_hours.unwrap().contains(23 * 60),
            "later default quiet hours apply"
        );
        // Garbage falls back to the defaults row.
        store.set_notify_prefs_json("a2", "not json", 1).unwrap();
        assert_eq!(load_prefs(&store, "a2").mode, Mode::All);
    }

    #[test]
    fn rule_effects() {
        let effect =
            RuleEffect::from_actions(&[RuleAction::MarkRead, RuleAction::AddLabel("x".into())]);
        assert!(effect.marked_read && !effect.moved);
        assert!(RuleEffect::from_actions(&[RuleAction::Archive]).moved);
        assert!(RuleEffect::from_actions(&[RuleAction::MoveToFolder("F".into())]).moved);
    }
}
