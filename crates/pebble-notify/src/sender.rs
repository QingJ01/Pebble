//! Address-based hints that a sender is a machine.
//!
//! Used alongside the header signals, which are missing for providers that
//! do not expose raw headers (Outlook) and for mail synced before signals
//! were recorded.

const AUTOMATED_LOCAL_PARTS: &[&str] = &[
    "noreply",
    "no-reply",
    "no_reply",
    "donotreply",
    "do-not-reply",
    "do_not_reply",
    "mailer-daemon",
    "postmaster",
    "bounce",
    "bounces",
    "notification",
    "notifications",
    "notify",
    "alert",
    "alerts",
    "newsletter",
    "news",
    "digest",
    "updates",
    "marketing",
    "promo",
    "promotions",
    "automated",
    "system",
    "robot",
];

const AUTOMATED_PREFIXES: &[&str] = &[
    "noreply",
    "no-reply",
    "donotreply",
    "bounce",
    "mailer-daemon",
    "notifications",
];

/// True when the address (or display name) reads like an automated sender.
pub fn is_automated_sender(address: &str, name: &str) -> bool {
    let address = address.trim().to_ascii_lowercase();
    let Some((local, domain)) = address.split_once('@') else {
        return false;
    };
    // Strip +tags: notifications+abc@github.com.
    let local = local.split('+').next().unwrap_or(local);
    if AUTOMATED_LOCAL_PARTS.contains(&local) {
        return true;
    }
    if AUTOMATED_PREFIXES
        .iter()
        .any(|prefix| local.starts_with(prefix))
    {
        return true;
    }
    // Bounce / VERP domains and common bulk-mail subdomains.
    if domain.starts_with("bounce.") || domain.starts_with("email.") && local.contains("reply") {
        return true;
    }
    let name = name.trim().to_ascii_lowercase();
    name.contains("no-reply") || name.contains("noreply") || name == "mailer-daemon"
}

#[cfg(test)]
mod tests {
    use super::is_automated_sender;

    #[test]
    fn recognises_common_machine_senders() {
        for address in [
            "noreply@example.com",
            "no-reply@accounts.example.com",
            "notifications+abc123@github.com",
            "MAILER-DAEMON@mx.example.com",
            "donotreply-billing@shop.example",
            "news@brand.example",
        ] {
            assert!(is_automated_sender(address, ""), "{address}");
        }
    }

    #[test]
    fn leaves_people_alone() {
        for address in [
            "ann@example.com",
            "j.smith@corp.example",
            "newsom@example.com",
            "support-team@example.com",
        ] {
            assert!(!is_automated_sender(address, "Ann"), "{address}");
        }
        assert!(!is_automated_sender("not-an-address", ""));
    }
}
