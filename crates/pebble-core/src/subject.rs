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
        // Outlook and other clients in other languages.
        "答复:",
        "答复：",
        "回覆:",
        "回覆：",
        "轉寄:",
        "轉寄：",
        "转寄:",
        "转寄：",
        "aw:",
        "wg:",
        "sv:",
        "vs:",
        "antw:",
        "tr:",
        "rv:",
        "réf:",
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
        // "Re[2]:", "RE[3]：" and similar counters.
        if !matched && lower.starts_with("re[") {
            if let Some(end) = s.find("]:").or_else(|| s.find("]：")) {
                let close = s[end..].chars().take(2).map(char::len_utf8).sum::<usize>();
                s = s[end + close..].trim().to_string();
                matched = true;
            }
        }
        if !matched {
            break;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::normalize_subject;

    #[test]
    fn strips_reply_prefixes_in_many_languages() {
        for subject in [
            "Re: 合同",
            "答复: 合同",
            "回覆：合同",
            "AW: 合同",
            "SV: Re: 合同",
            "Re[2]: 合同",
            "RE[3]：合同",
            "转发: 回复: 合同",
        ] {
            assert_eq!(normalize_subject(subject), "合同", "{subject}");
        }
        assert_eq!(
            normalize_subject("Regarding the plan"),
            "Regarding the plan"
        );
    }
}
