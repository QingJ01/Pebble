//! Storage for smart notifications (#107): sender preferences, notification
//! preferences, sync-time signals, sent-recipient history and the decision log.

use pebble_core::{MailSignals, Message, Result};
use rusqlite::{params, OptionalExtension};
use serde::{Deserialize, Serialize};

use crate::Store;

/// A VIP / mute entry. `account_id` is `""` for all accounts; `pattern` is a
/// lower-case address or `@domain`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SenderPrefRow {
    pub account_id: String,
    pub pattern: String,
    pub kind: String,
    pub created_at: i64,
}

/// One triage decision, newest first in listings.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NotificationLogRow {
    pub account_id: String,
    pub message_key: String,
    pub message_id: String,
    pub tier: String,
    pub reason: String,
    pub why: Option<String>,
    pub delivered: bool,
    pub created_at: i64,
    pub seen_at: Option<i64>,
}

/// A new decision to record.
#[derive(Debug, Clone)]
pub struct NewNotificationLog<'a> {
    pub account_id: &'a str,
    pub message_key: &'a str,
    pub message_id: &'a str,
    pub tier: &'a str,
    pub reason: &'a str,
    pub why: Option<&'a str>,
    pub created_at: i64,
}

/// Normalise a VIP / mute pattern: a full address, or `@domain`.
pub fn normalize_sender_pattern(raw: &str) -> Option<String> {
    let value = raw
        .trim()
        .trim_matches(|c| c == '<' || c == '>')
        .to_lowercase();
    if value.is_empty() || value.contains(char::is_whitespace) {
        return None;
    }
    if let Some(domain) = value.strip_prefix('@') {
        return (!domain.is_empty() && domain.contains('.') && !domain.contains('@'))
            .then(|| format!("@{domain}"));
    }
    let (local, domain) = value.split_once('@')?;
    (!local.is_empty() && domain.contains('.') && !domain.contains('@')).then_some(value)
}

/// Key used to deduplicate notifications for one message: the RFC 5322
/// Message-ID when present (stable across re-syncs), else the local id.
pub fn notification_message_key(message: &Message) -> String {
    match message.message_id_header.as_deref().map(str::trim) {
        Some(header) if !header.is_empty() => format!("mid:{}", header.to_lowercase()),
        _ => format!("id:{}", message.id),
    }
}

fn row_to_log(row: &rusqlite::Row) -> rusqlite::Result<NotificationLogRow> {
    Ok(NotificationLogRow {
        account_id: row.get(0)?,
        message_key: row.get(1)?,
        message_id: row.get(2)?,
        tier: row.get(3)?,
        reason: row.get(4)?,
        why: row.get(5)?,
        delivered: row.get::<_, i64>(6)? != 0,
        created_at: row.get(7)?,
        seen_at: row.get(8)?,
    })
}

const LOG_COLUMNS: &str =
    "account_id, message_key, message_id, tier, reason, why, delivered, created_at, seen_at";

impl Store {
    pub fn record_message_signals(&self, message_id: &str, signals: MailSignals) -> Result<()> {
        if signals.is_empty() {
            return Ok(());
        }
        self.with_write(|conn| {
            conn.execute(
                "INSERT INTO message_signals (message_id, bits) VALUES (?1, ?2)
                 ON CONFLICT(message_id) DO UPDATE SET bits = bits | excluded.bits",
                params![message_id, signals.to_bits()],
            )?;
            Ok(())
        })
    }

    pub fn message_signals(&self, message_id: &str) -> Result<MailSignals> {
        self.with_read(|conn| {
            let bits: Option<i64> = conn
                .query_row(
                    "SELECT bits FROM message_signals WHERE message_id = ?1",
                    params![message_id],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(bits.map(MailSignals::from_bits).unwrap_or_default())
        })
    }

    /// Remember the recipients of a message the account sent.
    pub fn note_sent_recipients(&self, message: &Message) -> Result<()> {
        if message.is_deleted || message.is_draft {
            return Ok(());
        }
        self.with_write(|conn| {
            let mut stmt = conn.prepare_cached(
                "INSERT INTO sent_recipients (account_id, normalized_address, sent_count, last_sent_at)
                 VALUES (?1, ?2, 1, ?3)
                 ON CONFLICT(account_id, normalized_address) DO UPDATE SET
                     sent_count = sent_count + 1,
                     last_sent_at = MAX(last_sent_at, excluded.last_sent_at)",
            )?;
            for address in message.to_list.iter().chain(&message.cc_list) {
                let normalized = address.address.trim().to_lowercase();
                if normalized.contains('@') {
                    stmt.execute(params![message.account_id, normalized, message.date])?;
                }
            }
            Ok(())
        })
    }

    /// How many times the account has written to `address`.
    pub fn sent_count_to(&self, account_id: &str, address: &str) -> Result<u32> {
        let normalized = address.trim().to_lowercase();
        self.with_read(|conn| {
            let count: Option<i64> = conn
                .query_row(
                    "SELECT sent_count FROM sent_recipients
                     WHERE account_id = ?1 AND normalized_address = ?2",
                    params![account_id, normalized],
                    |row| row.get(0),
                )
                .optional()?;
            Ok(count.unwrap_or(0).clamp(0, u32::MAX as i64) as u32)
        })
    }

    /// Whether `in_reply_to` names a message this account sent.
    pub fn is_reply_to_account(
        &self,
        account_id: &str,
        account_email: &str,
        in_reply_to: &str,
    ) -> Result<bool> {
        let ids: Vec<String> = in_reply_to
            .split_whitespace()
            .map(|id| id.trim().to_string())
            .filter(|id| !id.is_empty())
            .take(8)
            .collect();
        if ids.is_empty() {
            return Ok(false);
        }
        let email = account_email.trim().to_lowercase();
        self.with_read(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT 1 FROM messages
                 WHERE message_id_header = ?1 AND account_id = ?2
                   AND lower(trim(from_address)) = ?3
                 LIMIT 1",
            )?;
            for id in &ids {
                if stmt.exists(params![id, account_id, email])? {
                    return Ok(true);
                }
            }
            Ok(false)
        })
    }

    pub fn list_sender_prefs(&self) -> Result<Vec<SenderPrefRow>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(
                "SELECT account_id, pattern, kind, created_at FROM sender_prefs
                 ORDER BY kind, pattern, account_id",
            )?;
            let rows = stmt
                .query_map([], |row| {
                    Ok(SenderPrefRow {
                        account_id: row.get(0)?,
                        pattern: row.get(1)?,
                        kind: row.get(2)?,
                        created_at: row.get(3)?,
                    })
                })?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// Insert or replace a VIP / mute entry. Returns the stored pattern.
    pub fn set_sender_pref(
        &self,
        account_id: &str,
        raw_pattern: &str,
        kind: &str,
        now: i64,
    ) -> Result<String> {
        let pattern = normalize_sender_pattern(raw_pattern).ok_or_else(|| {
            pebble_core::PebbleError::Validation(format!(
                "Not an email address or @domain: {raw_pattern}"
            ))
        })?;
        if kind != "vip" && kind != "mute" {
            return Err(pebble_core::PebbleError::Validation(format!(
                "Unknown sender preference: {kind}"
            )));
        }
        self.with_write(|conn| {
            conn.execute(
                "INSERT INTO sender_prefs (account_id, pattern, kind, created_at) VALUES (?1, ?2, ?3, ?4)
                 ON CONFLICT(account_id, pattern) DO UPDATE SET kind = excluded.kind",
                params![account_id, pattern, kind, now],
            )?;
            Ok(())
        })?;
        Ok(pattern)
    }

    pub fn remove_sender_pref(&self, account_id: &str, pattern: &str) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "DELETE FROM sender_prefs WHERE account_id = ?1 AND pattern = ?2",
                params![account_id, pattern.trim().to_lowercase()],
            )?;
            Ok(())
        })
    }

    /// The preference that applies to `address` on `account_id`. Specific
    /// beats general: account address, account domain, global address,
    /// global domain. `None` if nothing matches.
    pub fn sender_pref_for(&self, account_id: &str, address: &str) -> Result<Option<String>> {
        let address = address.trim().to_lowercase();
        let Some((_, domain)) = address.rsplit_once('@') else {
            return Ok(None);
        };
        let domain = format!("@{domain}");
        self.with_read(|conn| {
            let mut stmt = conn.prepare_cached(
                "SELECT kind FROM sender_prefs WHERE account_id = ?1 AND pattern = ?2",
            )?;
            for (scope, pattern) in [
                (account_id, address.as_str()),
                (account_id, domain.as_str()),
                ("", address.as_str()),
                ("", domain.as_str()),
            ] {
                if let Some(kind) = stmt
                    .query_row(params![scope, pattern], |row| row.get::<_, String>(0))
                    .optional()?
                {
                    return Ok(Some(kind));
                }
            }
            Ok(None)
        })
    }

    /// Raw preference JSON for an account (`""` = defaults).
    pub fn get_notify_prefs_json(&self, account_id: &str) -> Result<Option<String>> {
        self.with_read(|conn| {
            Ok(conn
                .query_row(
                    "SELECT prefs_json FROM notify_prefs WHERE account_id = ?1",
                    params![account_id],
                    |row| row.get(0),
                )
                .optional()?)
        })
    }

    pub fn list_notify_prefs_json(&self) -> Result<Vec<(String, String)>> {
        self.with_read(|conn| {
            let mut stmt = conn
                .prepare("SELECT account_id, prefs_json FROM notify_prefs ORDER BY account_id")?;
            let rows = stmt
                .query_map([], |row| Ok((row.get(0)?, row.get(1)?)))?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn set_notify_prefs_json(&self, account_id: &str, json: &str, now: i64) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "INSERT INTO notify_prefs (account_id, prefs_json, updated_at) VALUES (?1, ?2, ?3)
                 ON CONFLICT(account_id) DO UPDATE SET prefs_json = excluded.prefs_json,
                                                       updated_at = excluded.updated_at",
                params![account_id, json, now],
            )?;
            Ok(())
        })
    }

    pub fn delete_notify_prefs(&self, account_id: &str) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "DELETE FROM notify_prefs WHERE account_id = ?1",
                params![account_id],
            )?;
            Ok(())
        })
    }

    /// Record a decision. Returns `false` when this message was already
    /// decided (dedupe across retries, re-syncs and restarts).
    ///
    /// The key is shared by every copy of a message (IMAP keeps one per
    /// folder). A silent decision for one copy (All Mail, a folder a server
    /// filter chose) must not block the inbox copy, so a later non-silent
    /// decision replaces an undelivered silent one.
    pub fn insert_notification_log(&self, entry: &NewNotificationLog<'_>) -> Result<bool> {
        self.with_write(|conn| {
            let inserted = conn.execute(
                "INSERT INTO notification_log
                     (account_id, message_key, message_id, tier, reason, why, delivered, created_at)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, 0, ?7)
                 ON CONFLICT(account_id, message_key) DO UPDATE SET
                     message_id = excluded.message_id, tier = excluded.tier,
                     reason = excluded.reason, why = excluded.why, created_at = excluded.created_at
                 WHERE notification_log.tier = 'silent' AND notification_log.delivered = 0
                   AND excluded.tier <> 'silent'",
                params![
                    entry.account_id,
                    entry.message_key,
                    entry.message_id,
                    entry.tier,
                    entry.reason,
                    entry.why,
                    entry.created_at
                ],
            )?;
            Ok(inserted == 1)
        })
    }

    /// Replace the decision of a logged message (model refinement).
    pub fn update_notification_decision(
        &self,
        account_id: &str,
        message_key: &str,
        tier: &str,
        reason: &str,
        why: Option<&str>,
    ) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "UPDATE notification_log SET tier = ?3, reason = ?4, why = ?5
                 WHERE account_id = ?1 AND message_key = ?2",
                params![account_id, message_key, tier, reason, why],
            )?;
            Ok(())
        })
    }

    pub fn mark_notification_delivered(&self, account_id: &str, message_key: &str) -> Result<()> {
        self.with_write(|conn| {
            conn.execute(
                "UPDATE notification_log SET delivered = 1 WHERE account_id = ?1 AND message_key = ?2",
                params![account_id, message_key],
            )?;
            Ok(())
        })
    }

    pub fn list_notification_log(&self, limit: u32) -> Result<Vec<NotificationLogRow>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {LOG_COLUMNS} FROM notification_log ORDER BY created_at DESC LIMIT ?1"
            ))?;
            let rows = stmt
                .query_map(params![limit.min(1_000)], row_to_log)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    /// Digest entries not yet seen, newest first, for live messages only.
    pub fn list_unseen_digest(&self, limit: u32) -> Result<Vec<NotificationLogRow>> {
        self.with_read(|conn| {
            let mut stmt = conn.prepare(&format!(
                "SELECT {} FROM notification_log l
                 JOIN messages m ON m.id = l.message_id AND m.is_deleted = 0
                 WHERE l.tier = 'digest' AND l.seen_at IS NULL
                 ORDER BY l.created_at DESC LIMIT ?1",
                LOG_COLUMNS
                    .split(", ")
                    .map(|column| format!("l.{column}"))
                    .collect::<Vec<_>>()
                    .join(", ")
            ))?;
            let rows = stmt
                .query_map(params![limit.min(1_000)], row_to_log)?
                .collect::<rusqlite::Result<Vec<_>>>()?;
            Ok(rows)
        })
    }

    pub fn mark_digest_seen(&self, now: i64) -> Result<usize> {
        self.with_write(|conn| {
            Ok(conn.execute(
                "UPDATE notification_log SET seen_at = ?1 WHERE tier = 'digest' AND seen_at IS NULL",
                params![now],
            )?)
        })
    }

    /// Drop decisions older than `before` (keeps the log small).
    pub fn prune_notification_log(&self, before: i64) -> Result<usize> {
        self.with_write(|conn| {
            Ok(conn.execute(
                "DELETE FROM notification_log WHERE created_at < ?1",
                params![before],
            )?)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use pebble_core::{Account, EmailAddress, ProviderType};

    fn account(email: &str) -> Account {
        Account {
            id: format!("acc-{email}"),
            email: email.to_string(),
            display_name: "Me".into(),
            color: None,
            account_label: None,
            provider_display_name: None,
            provider: ProviderType::Imap,
            created_at: 0,
            updated_at: 0,
        }
    }

    fn message(id: &str, account: &Account, from: &str, to: &[&str]) -> Message {
        Message {
            id: id.into(),
            account_id: account.id.clone(),
            remote_id: id.into(),
            message_id_header: Some(format!("<{id}@example.com>")),
            in_reply_to: None,
            references_header: None,
            thread_id: None,
            subject: "s".into(),
            snippet: String::new(),
            from_address: from.into(),
            from_name: String::new(),
            to_list: to
                .iter()
                .map(|address| EmailAddress {
                    name: None,
                    address: address.to_string(),
                })
                .collect(),
            cc_list: vec![],
            bcc_list: vec![],
            body_text: String::new(),
            body_html_raw: String::new(),
            has_attachments: false,
            is_read: false,
            is_starred: false,
            is_draft: false,
            date: 100,
            remote_version: None,
            is_deleted: false,
            deleted_at: None,
            created_at: 0,
            updated_at: 0,
        }
    }

    #[test]
    fn sender_patterns_normalise() {
        assert_eq!(
            normalize_sender_pattern(" Ann@Example.com "),
            Some("ann@example.com".into())
        );
        assert_eq!(
            normalize_sender_pattern("@Example.com"),
            Some("@example.com".into())
        );
        assert_eq!(
            normalize_sender_pattern("<bob@x.org>"),
            Some("bob@x.org".into())
        );
        for bad in ["", "ann", "@", "@localhost", "a b@x.com", "a@@x.com"] {
            assert_eq!(normalize_sender_pattern(bad), None, "{bad}");
        }
    }

    #[test]
    fn specific_sender_prefs_beat_general_ones() {
        let store = Store::open_in_memory().unwrap();
        store
            .set_sender_pref("", "@shop.example", "mute", 1)
            .unwrap();
        assert_eq!(
            store
                .sender_pref_for("a1", "deals@shop.example")
                .unwrap()
                .as_deref(),
            Some("mute")
        );
        store
            .set_sender_pref("a1", "boss@shop.example", "vip", 2)
            .unwrap();
        assert_eq!(
            store
                .sender_pref_for("a1", "Boss@Shop.example")
                .unwrap()
                .as_deref(),
            Some("vip")
        );
        assert_eq!(
            store
                .sender_pref_for("a2", "boss@shop.example")
                .unwrap()
                .as_deref(),
            Some("mute")
        );
        assert_eq!(
            store.sender_pref_for("a1", "ann@else.example").unwrap(),
            None
        );
        store.remove_sender_pref("", "@shop.example").unwrap();
        assert_eq!(
            store.sender_pref_for("a2", "boss@shop.example").unwrap(),
            None
        );
        assert!(store.set_sender_pref("", "nope", "vip", 3).is_err());
        assert!(store.set_sender_pref("", "a@b.co", "maybe", 3).is_err());
        assert_eq!(store.list_sender_prefs().unwrap().len(), 1);
    }

    #[test]
    fn sent_recipients_and_replies_are_tracked() {
        let store = Store::open_in_memory().unwrap();
        let me = account("me@example.com");
        store.insert_account(&me).unwrap();
        let sent = message("s1", &me, "Me@Example.com", &["Ann@example.com"]);
        store.insert_message(&sent, &[]).unwrap();
        store.note_sent_recipients(&sent).unwrap();
        store.note_sent_recipients(&sent).unwrap();
        assert_eq!(store.sent_count_to(&me.id, "ann@EXAMPLE.com").unwrap(), 2);
        assert_eq!(store.sent_count_to(&me.id, "bob@example.com").unwrap(), 0);

        assert!(store
            .is_reply_to_account(&me.id, "me@example.com", "<s1@example.com>")
            .unwrap());
        assert!(!store
            .is_reply_to_account(&me.id, "me@example.com", "<other@example.com>")
            .unwrap());
        let theirs = message("t1", &me, "ann@example.com", &["me@example.com"]);
        store.insert_message(&theirs, &[]).unwrap();
        assert!(!store
            .is_reply_to_account(&me.id, "me@example.com", "<t1@example.com>")
            .unwrap());
    }

    #[test]
    fn notification_log_dedupes_and_tracks_digest() {
        let store = Store::open_in_memory().unwrap();
        let me = account("me@example.com");
        store.insert_account(&me).unwrap();
        let m = message("m1", &me, "ann@example.com", &["me@example.com"]);
        store.insert_message(&m, &[]).unwrap();
        let key = notification_message_key(&m);
        let entry = NewNotificationLog {
            account_id: &me.id,
            message_key: &key,
            message_id: &m.id,
            tier: "digest",
            reason: "default",
            why: None,
            created_at: 10,
        };
        assert!(store.insert_notification_log(&entry).unwrap());
        assert!(!store.insert_notification_log(&entry).unwrap());
        // A silent copy elsewhere doesn't block, and doesn't replace, a
        // non-silent decision; a non-silent one replaces a silent one.
        let silent = NewNotificationLog {
            tier: "silent",
            reason: "moved",
            ..entry
        };
        assert!(!store.insert_notification_log(&silent).unwrap());
        assert_eq!(store.list_notification_log(10).unwrap()[0].tier, "digest");
        assert_eq!(store.list_unseen_digest(10).unwrap().len(), 1);
        store
            .update_notification_decision(
                &me.id,
                &key,
                "interrupt",
                "model:needs_action",
                Some("Asks for a reply"),
            )
            .unwrap();
        assert!(store.list_unseen_digest(10).unwrap().is_empty());
        let log = store.list_notification_log(10).unwrap();
        assert_eq!(log[0].reason, "model:needs_action");
        assert_eq!(log[0].why.as_deref(), Some("Asks for a reply"));
        store.mark_notification_delivered(&me.id, &key).unwrap();
        assert!(store.list_notification_log(10).unwrap()[0].delivered);
        assert_eq!(store.prune_notification_log(11).unwrap(), 1);
    }

    #[test]
    fn signals_and_prefs_round_trip() {
        let store = Store::open_in_memory().unwrap();
        let me = account("me@example.com");
        store.insert_account(&me).unwrap();
        let m = message("m1", &me, "news@shop.example", &["me@example.com"]);
        store.insert_message(&m, &[]).unwrap();
        assert!(store.message_signals("m1").unwrap().is_empty());
        store
            .record_message_signals(
                "m1",
                MailSignals {
                    bulk: true,
                    ..MailSignals::default()
                },
            )
            .unwrap();
        assert!(store.message_signals("m1").unwrap().bulk);

        store
            .set_notify_prefs_json("", r#"{"mode":"smart"}"#, 1)
            .unwrap();
        store
            .set_notify_prefs_json(&me.id, r#"{"mode":"all"}"#, 2)
            .unwrap();
        assert_eq!(
            store.get_notify_prefs_json("").unwrap().as_deref(),
            Some(r#"{"mode":"smart"}"#)
        );
        assert_eq!(store.list_notify_prefs_json().unwrap().len(), 2);
        store.delete_notify_prefs(&me.id).unwrap();
        assert_eq!(store.get_notify_prefs_json(&me.id).unwrap(), None);
    }
}
