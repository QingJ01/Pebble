//! Does a message ask the user for something? Used to find mail that is
//! waiting for the user's reply (#107 proactive nudges). Rule-based; the
//! model (when enabled) refines it.

/// Phrases that ask the reader to do or answer something.
const REQUEST_PHRASES_EN: &[&str] = &[
    "can you",
    "could you",
    "would you",
    "will you",
    "are you able",
    "please confirm",
    "please let me know",
    "please reply",
    "please review",
    "please send",
    "please advise",
    "let me know",
    "get back to me",
    "your thoughts",
    "what do you think",
    "do you have time",
    "are you free",
    "rsvp",
    "awaiting your",
    "need your",
    "waiting for your",
    "by tomorrow",
    "by end of day",
    "by eod",
    "asap",
    "when can you",
    "could we",
    "shall we",
];

const REQUEST_PHRASES_ZH: &[&str] = &[
    "请确认",
    "请回复",
    "请尽快",
    "请帮忙",
    "请问",
    "麻烦",
    "能否",
    "能不能",
    "可不可以",
    "可以吗",
    "行吗",
    "好吗",
    "是否",
    "有没有空",
    "有空吗",
    "给我回",
    "回个话",
    "回复一下",
    "等你回复",
    "确认一下",
    "看一下",
    "帮我",
    "你觉得",
    "怎么看",
    "什么时候",
];

/// Closings that mean no answer is expected.
const NO_REPLY_PHRASES: &[&str] = &[
    "no need to reply",
    "no reply needed",
    "nothing needed from you",
    "no action needed",
    "no action required",
    "fyi",
    "for your reference",
    "不需要回复",
    "无需回复",
    "不用回复",
    "供参考",
    "仅供参考",
];

/// Short thank-you / acknowledgement messages.
const ACK_ONLY: &[&str] = &[
    "thanks",
    "thank you",
    "got it",
    "received",
    "收到",
    "谢谢",
    "好的",
    "ok",
];

/// Strip quoted history (`>` lines and everything after a typical
/// "On … wrote:" / "-----Original Message-----" separator).
pub fn strip_quoted(body: &str) -> String {
    let mut out = Vec::new();
    for line in body.lines() {
        let trimmed = line.trim();
        let lower = trimmed.to_lowercase();
        if trimmed.starts_with('>') {
            continue;
        }
        if (lower.starts_with("on ") && lower.ends_with("wrote:"))
            || lower.starts_with("-----original message")
            || trimmed.starts_with("在") && trimmed.ends_with("写道：")
            || lower.starts_with("from:") && !out.is_empty()
        {
            break;
        }
        out.push(line);
    }
    out.join("\n")
}

/// How clearly a message asks the reader for something.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum RequestStrength {
    /// FYI, thanks, closings.
    None,
    /// Only a question mark.
    Question,
    /// An explicit ask ("could you", "请确认", "let me know").
    Explicit,
    /// An explicit ask with urgency ("today", "ASAP", "今天下班前").
    Urgent,
}

const URGENT_PHRASES: &[&str] = &[
    "asap",
    "urgent",
    "today",
    "tonight",
    "by tomorrow",
    "by end of day",
    "by eod",
    "this afternoon",
    "right away",
    "as soon as possible",
    "尽快",
    "今天",
    "今晚",
    "明天",
    "下班前",
    "马上",
    "急",
    "截止",
];

/// Classify how strongly the (unquoted) text asks for an answer or action.
pub fn request_strength(subject: &str, body: &str) -> RequestStrength {
    let body = strip_quoted(body);
    let text = format!("{subject}\n{body}");
    let lower = text.to_lowercase();
    if NO_REPLY_PHRASES.iter().any(|p| lower.contains(p)) {
        return RequestStrength::None;
    }
    let compact: String = body
        .trim()
        .trim_end_matches(['!', '！', '.', '。', '~'])
        .to_lowercase();
    if compact.chars().count() <= 24
        && ACK_ONLY.iter().any(|p| compact.contains(p))
        && !compact.contains('?')
        && !compact.contains('？')
    {
        return RequestStrength::None;
    }
    let explicit = REQUEST_PHRASES_EN.iter().any(|p| lower.contains(p))
        || REQUEST_PHRASES_ZH.iter().any(|p| text.contains(p));
    let question = body.contains('?')
        || body.contains('？')
        || subject.contains('?')
        || subject.contains('？');
    let urgent = URGENT_PHRASES.iter().any(|p| lower.contains(p));
    match (explicit || question, explicit, urgent) {
        (false, _, _) => RequestStrength::None,
        (true, _, true) => RequestStrength::Urgent,
        (true, true, false) => RequestStrength::Explicit,
        (true, false, false) => RequestStrength::Question,
    }
}

/// Whether the (unquoted) text asks the reader for an answer or action.
pub fn looks_like_request(subject: &str, body: &str) -> bool {
    request_strength(subject, body) != RequestStrength::None
}

/// What is known about a message that may be waiting for the user's reply.
#[derive(Debug, Clone, Default)]
pub struct AwaitingFeatures {
    pub request: Option<RequestStrength>,
    /// How many times the user has written to the sender.
    pub sent_count: u32,
    /// The user is in To (not only Cc).
    pub direct: bool,
    /// The user is the only person in To and Cc.
    pub only_recipient: bool,
    pub vip: bool,
    /// Hours since it arrived.
    pub age_hours: f32,
}

/// Score in [0, 1]: how likely the sender is actually waiting on the user.
/// Nudges go out at [`AWAITING_THRESHOLD`] and above.
pub fn awaiting_score(f: &AwaitingFeatures) -> f32 {
    let request = match f.request.unwrap_or(RequestStrength::None) {
        RequestStrength::None => return 0.0,
        RequestStrength::Question => 0.30,
        RequestStrength::Explicit => 0.45,
        RequestStrength::Urgent => 0.55,
    };
    let relationship = if f.vip {
        0.25
    } else if f.sent_count >= 3 {
        0.20
    } else if f.sent_count >= 1 {
        0.12
    } else {
        0.0
    };
    let addressing = if f.only_recipient {
        0.20
    } else if f.direct {
        0.12
    } else {
        0.0
    };
    // Waiting longer makes it a little more pressing, up to three days.
    let age = (f.age_hours / 72.0).clamp(0.0, 1.0) * 0.05;
    (request + relationship + addressing + age).min(1.0)
}

/// Minimum [`awaiting_score`] for a nudge.
pub const AWAITING_THRESHOLD: f32 = 0.55;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn questions_and_requests_count() {
        assert!(looks_like_request(
            "Dinner on Friday?",
            "Are you free around 7?"
        ));
        assert!(looks_like_request(
            "Contract",
            "Could you review section 3 by tomorrow"
        ));
        assert!(looks_like_request(
            "合同",
            "你看一下能不能接受，今天给我回个话。"
        ));
        assert!(looks_like_request(
            "签证材料",
            "还缺一张在职证明，麻烦这周内补寄。"
        ));
    }

    #[test]
    fn fyi_and_thanks_do_not() {
        assert!(!looks_like_request(
            "Slides",
            "Attached for reference. Nothing needed from you."
        ));
        assert!(!looks_like_request(
            "Re: photos",
            "Got them, thank you so much!"
        ));
        assert!(!looks_like_request("Re: 照片", "收到了，谢谢！"));
        assert!(!looks_like_request(
            "周会纪要",
            "这是本周周会的纪要，供参考，不需要回复。"
        ));
    }

    #[test]
    fn strength_grades_asks() {
        assert_eq!(
            request_strength("Re: photos", "Got them, thanks!"),
            RequestStrength::None
        );
        assert_eq!(
            request_strength("Lunch", "Lunch today?"),
            RequestStrength::Urgent
        );
        assert_eq!(
            request_strength("Book", "What was the name of that book?"),
            RequestStrength::Question
        );
        assert_eq!(
            request_strength("Review", "Could you review the draft"),
            RequestStrength::Explicit
        );
        assert_eq!(
            request_strength("合同", "请确认第三条，今天下班前回我"),
            RequestStrength::Urgent
        );
    }

    #[test]
    fn scores_need_an_ask_and_reward_relationship_and_direct_mail() {
        let ask = AwaitingFeatures {
            request: Some(RequestStrength::Explicit),
            ..Default::default()
        };
        assert!(
            awaiting_score(&ask) < AWAITING_THRESHOLD,
            "an ask from a stranger in a list alone is not enough"
        );
        let friend = AwaitingFeatures {
            sent_count: 4,
            direct: true,
            ..ask.clone()
        };
        assert!(awaiting_score(&friend) >= AWAITING_THRESHOLD);
        let stranger_direct = AwaitingFeatures {
            only_recipient: true,
            direct: true,
            ..ask.clone()
        };
        assert!(
            awaiting_score(&stranger_direct) >= AWAITING_THRESHOLD,
            "a personal ask addressed only to you"
        );
        let cc_question = AwaitingFeatures {
            request: Some(RequestStrength::Question),
            sent_count: 1,
            ..Default::default()
        };
        assert!(awaiting_score(&cc_question) < AWAITING_THRESHOLD);
        let none = AwaitingFeatures {
            request: Some(RequestStrength::None),
            vip: true,
            only_recipient: true,
            ..Default::default()
        };
        assert_eq!(awaiting_score(&none), 0.0);
    }

    #[test]
    fn quoted_history_is_ignored() {
        let body = "Sounds good, see you then.\n\nOn Mon, Ann wrote:\n> Can you make it at 7?";
        assert!(!looks_like_request("Re: Dinner", body));
        assert_eq!(strip_quoted("hi\n> quoted\nbye"), "hi\nbye");
    }
}
