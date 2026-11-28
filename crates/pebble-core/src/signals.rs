//! Delivery signals captured at sync time for notification triage.
//!
//! These come from headers (or the provider's own classification) that are
//! not otherwise stored with a message. They are only collected for newly
//! synced mail; older messages have [`MailSignals::default`].

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailSignals {
    /// `Auto-Submitted` is present and not `no` (RFC 3834): auto-replies,
    /// out-of-office, system-generated mail.
    pub auto_submitted: bool,
    /// Mailing-list or bulk-sender markers: `List-Id`, `List-Unsubscribe`,
    /// or `Precedence: bulk | list | junk`.
    pub bulk: bool,
    /// The provider filed it as low priority (Gmail promotions / social /
    /// updates / forums categories, Outlook "Other").
    pub provider_low_priority: bool,
}

impl MailSignals {
    /// Pack into a small integer for storage.
    pub fn to_bits(self) -> i64 {
        (self.auto_submitted as i64)
            | ((self.bulk as i64) << 1)
            | ((self.provider_low_priority as i64) << 2)
    }

    pub fn from_bits(bits: i64) -> Self {
        Self {
            auto_submitted: bits & 1 != 0,
            bulk: bits & 2 != 0,
            provider_low_priority: bits & 4 != 0,
        }
    }

    pub fn is_empty(self) -> bool {
        self == Self::default()
    }

    /// Read the signals from header name/value pairs (case-insensitive names).
    pub fn from_headers<'a>(headers: impl IntoIterator<Item = (&'a str, &'a str)>) -> Self {
        let mut signals = Self::default();
        for (name, value) in headers {
            let value = value.trim();
            if name.eq_ignore_ascii_case("Auto-Submitted") {
                if !value.is_empty() && !value.eq_ignore_ascii_case("no") {
                    signals.auto_submitted = true;
                }
            } else if name.eq_ignore_ascii_case("List-Id")
                || name.eq_ignore_ascii_case("List-Unsubscribe")
            {
                if !value.is_empty() {
                    signals.bulk = true;
                }
            } else if name.eq_ignore_ascii_case("Precedence") {
                let v = value.to_ascii_lowercase();
                if v == "bulk" || v == "list" || v == "junk" {
                    signals.bulk = true;
                }
            } else if name.eq_ignore_ascii_case("X-Autoreply")
                || name.eq_ignore_ascii_case("X-Autorespond")
            {
                signals.auto_submitted = true;
            }
        }
        signals
    }

    /// Gmail category labels that mark low-priority mail.
    pub fn from_gmail_labels<'a>(labels: impl IntoIterator<Item = &'a str>) -> Self {
        let low = labels.into_iter().any(|label| {
            matches!(
                label,
                "CATEGORY_PROMOTIONS" | "CATEGORY_SOCIAL" | "CATEGORY_UPDATES" | "CATEGORY_FORUMS"
            )
        });
        Self {
            provider_low_priority: low,
            ..Self::default()
        }
    }

    pub fn merge(self, other: Self) -> Self {
        Self {
            auto_submitted: self.auto_submitted || other.auto_submitted,
            bulk: self.bulk || other.bulk,
            provider_low_priority: self.provider_low_priority || other.provider_low_priority,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headers_are_read_case_insensitively() {
        let s = MailSignals::from_headers([
            ("auto-submitted", "auto-replied"),
            ("list-unsubscribe", "<mailto:x>"),
        ]);
        assert!(s.auto_submitted && s.bulk && !s.provider_low_priority);
        assert!(!MailSignals::from_headers([("Auto-Submitted", "no")]).auto_submitted);
        assert!(MailSignals::from_headers([("Precedence", "Bulk")]).bulk);
        assert!(!MailSignals::from_headers([("Precedence", "first-class")]).bulk);
    }

    #[test]
    fn bits_round_trip() {
        for bits in 0..8 {
            assert_eq!(MailSignals::from_bits(bits).to_bits(), bits);
        }
    }

    #[test]
    fn gmail_categories() {
        assert!(
            MailSignals::from_gmail_labels(["INBOX", "CATEGORY_PROMOTIONS"]).provider_low_priority
        );
        assert!(
            !MailSignals::from_gmail_labels(["INBOX", "CATEGORY_PERSONAL"]).provider_low_priority
        );
    }
}
