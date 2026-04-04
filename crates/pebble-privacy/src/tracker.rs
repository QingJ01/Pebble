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
    "facebook.com/tr",
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
    let w = width.and_then(|v| v.parse::<u32>().ok()).unwrap_or(u32::MAX);
    let h = height.and_then(|v| v.parse::<u32>().ok()).unwrap_or(u32::MAX);
    w <= 1 && h <= 1
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
        assert!(!is_tracking_pixel(None, None));
    }
}
