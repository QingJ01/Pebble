const KNOWN_TRACKERS: &[&str] = &[
    "mailchimp.com",
    "list-manage.com",
    "hubspot.com",
    "sendgrid.net",
    "mailgun.org",
    "constantcontact.com",
    "campaign-archive.com",
    "exacttarget.com",
    "sailthru.com",
    "returnpath.net",
    "litmus.com",
    "bananatag.com",
    "yesware.com",
    "mailtrack.io",
    "getnotify.com",
    "streak.com",
    "cirrusinsight.com",
    "boomeranggmail.com",
    "mixmax.com",
    "superhuman.com",
    "facebook.com",
    "google-analytics.com",
    "doubleclick.net",
    "pixel.wp.com",
    "open.convertkit.com",
    "cmail19.com",
    "cmail20.com",
    "createsend.com",
    "intercom.io",
    "drip.com",
    "mandrillapp.com",
];

pub fn is_known_tracker(domain: &str) -> bool {
    let domain_lower = domain.to_lowercase();
    KNOWN_TRACKERS.iter().any(|t| domain_lower.contains(t))
}

pub fn is_tracking_pixel(width: Option<&str>, height: Option<&str>) -> bool {
    match (width, height) {
        // Both dimensions explicitly set to <= 1 — classic tracking pixel
        (Some(w), Some(h)) => {
            let w = w.parse::<u32>().unwrap_or(0);
            let h = h.parse::<u32>().unwrap_or(0);
            w <= 1 && h <= 1
        }
        // One dimension present and <= 1, other absent — likely a pixel
        (Some(v), None) | (None, Some(v)) => v.parse::<u32>().unwrap_or(0) <= 1,
        // Both absent — cannot determine from dimensions alone; don't flag here.
        // The caller should use additional heuristics (URL pattern, known domains).
        (None, None) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_tracker_domains() {
        assert!(is_known_tracker("tracking.mailchimp.com"));
        assert!(is_known_tracker("t.hubspot.com"));
        assert!(is_known_tracker("email.sendgrid.net"));
        assert!(!is_known_tracker("example.com"));
        assert!(!is_known_tracker("google.com"));
    }

    #[test]
    fn test_tracking_pixel_detection() {
        assert!(is_tracking_pixel(Some("1"), Some("1")));
        assert!(is_tracking_pixel(Some("0"), Some("0")));
        assert!(!is_tracking_pixel(Some("100"), Some("50")));
        assert!(!is_tracking_pixel(None, None)); // missing both dimensions — inconclusive, checked by other heuristics
        assert!(is_tracking_pixel(Some("1"), None)); // one dimension <= 1, other absent
        assert!(is_tracking_pixel(None, Some("0"))); // one dimension <= 1, other absent
    }
}
