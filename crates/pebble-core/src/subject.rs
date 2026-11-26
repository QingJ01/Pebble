/// Strip common reply/forward prefixes from a subject line, recursively.
/// Handles Re:, Fwd:, Fw:, 回复:, 转发: (case-insensitive, ASCII or
/// full-width colon).
pub fn normalize_subject(subject: &str) -> String {
    let mut s = subject.trim().to_string();
    let prefixes = [
        "re:",
        "fwd:",
        "fw:",
        "回复:",
        "转发:",
        "re：",
        "fwd：",
        "fw：",
        "回复：",
        "转发：",
    ];
    loop {
        let lower = s.to_lowercase();
        let mut matched = false;
        for prefix in &prefixes {
            if lower.starts_with(prefix) {
                // Count chars in the prefix, then advance by the same number of
                // chars in the *original* string to avoid byte-boundary panics
                // when to_lowercase() changes byte length.
                let char_count = prefix.chars().count();
                let byte_offset = s
                    .char_indices()
                    .nth(char_count)
                    .map(|(i, _)| i)
                    .unwrap_or(s.len());
                s = s[byte_offset..].trim().to_string();
                matched = true;
                break;
            }
        }
        if !matched {
            break;
        }
    }
    s
}
