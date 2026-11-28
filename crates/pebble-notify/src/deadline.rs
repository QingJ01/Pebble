//! Deadlines mentioned in mail ("by Friday 5pm", "请于12月5日前回复").
//!
//! Rule-based and conservative: a date only counts when a deadline word
//! ties it to the ask (`by`, `due`, `deadline`, `截止`, `之前` …), so
//! "see you Friday" or "明天开会" don't match. Dates are resolved against
//! the time the mail arrived, in local time.

use std::sync::OnceLock;

use chrono::{Datelike, Duration, NaiveDate, NaiveDateTime, NaiveTime, Weekday};
use regex::{Captures, Regex};

/// Characters of the body that are searched (deadlines sit near the top).
const SCAN_CHARS: usize = 2_000;
/// Further out than this is a plan, not a deadline to remind about.
const MAX_AHEAD_DAYS: i64 = 120;
/// Time used when only a day is given ("by Friday").
const END_OF_DAY: (u32, u32) = (18, 0);

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deadline {
    /// Local date and time it is due.
    pub due: NaiveDateTime,
    /// Only a day was given; `due` is the end of the working day.
    pub all_day: bool,
    /// The words it was found in, as written.
    pub phrase: String,
}

fn re(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).expect("valid deadline regex"))
}

const EN_MONTHS: &str = "jan(?:uary)?|feb(?:ruary)?|mar(?:ch)?|apr(?:il)?|may|june?|july?|aug(?:ust)?|sep(?:t(?:ember)?)?|oct(?:ober)?|nov(?:ember)?|dec(?:ember)?";
// "Mon", "Sat" and "Sun" alone are too often names or words ("Sun Wei").
const EN_DAYS: &str =
    "monday|tue(?:s(?:day)?)?|wed(?:nesday)?|thu(?:r(?:s(?:day)?)?)?|fri(?:day)?|saturday|sunday";

fn en_pattern() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    re(
        &CELL,
        &format!(
            r"(?ix)
            \b(?P<trigger>by|before|due(?:\s+(?:by|on|date:?))?|deadline(?:\s+is)?:?|no\s+later\s+than)\s+
            (?:the\s+)?(?:end\s+of\s+)?
            (?P<when>
                (?P<iso>\d{{4}}-\d{{1,2}}-\d{{1,2}})
              | (?P<eod>(?:today|tonight|eod|end\s+of\s+(?:the\s+)?day|cob|close\s+of\s+business)\b)
              | (?P<tomorrow>tomorrow\b)
              | (?P<eow>(?:this\s+|the\s+)?week\b)
              | (?:(?P<next>next|this)\s+)?(?P<weekday>{days})\b\.?
              | (?P<month1>{months})\.?\s+(?P<day1>\d{{1,2}})(?:st|nd|rd|th)?\b
              | (?P<day2>\d{{1,2}})(?:st|nd|rd|th)?\s+(?:of\s+)?(?P<month2>{months})\b\.?
            )
            (?:,?\s*(?:at\s+)?(?:
                (?P<h12>\d{{1,2}})(?::(?P<m12>\d{{2}}))?\s*(?P<ampm>am|pm|a\.m\.|p\.m\.)
              | (?P<h24>\d{{1,2}}):(?P<m24>\d{{2}})
              | (?P<noon>noon|midday)
            ))?",
            days = EN_DAYS,
            months = EN_MONTHS,
        ),
    )
}

fn zh_pattern() -> &'static Regex {
    static CELL: OnceLock<Regex> = OnceLock::new();
    re(
        &CELL,
        r"(?x)
        (?P<before>截止(?:日期|时间)?(?:为|是|到|至)?[:：]?\s*|截至\s*|最晚|最迟|ddl[:：]?\s*|DDL[:：]?\s*)?
        (?P<when>
            (?P<iso>(?:\d{4})[-/.年]\d{1,2}[-/.月]\d{1,2}[日号]?)
          | (?P<md>\d{1,2})月(?P<mdd>\d{1,2})[日号]
          | (?P<rel>今天|今日|今晚|明天|明日|明早|后天)
          | (?P<wk>(?P<wkp>本|这|下)?(?:个)?(?:周|星期|礼拜)(?P<wkd>[一二三四五六日天]))
          | (?P<eom>月底|本月底)
          | (?P<eow>本周|这周)
        )
        (?:\s*(?P<part>上午|中午|下午|晚上|傍晚|下班|早上))?
        (?:\s*(?P<hour>\d{1,2})\s*(?:[点时:：])\s*(?P<minute>\d{1,2}|半)?\s*(?:分)?)?
        (?P<after>\s*(?:之前|以前|前|为止|截止|截至))?
        (?P<follow>\S)?",
    )
}

fn month_from_en(name: &str) -> Option<u32> {
    let lower = name.to_ascii_lowercase();
    let months = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ];
    months
        .iter()
        .position(|m| lower.starts_with(m))
        .map(|i| i as u32 + 1)
}

fn weekday_from_en(name: &str) -> Option<Weekday> {
    let lower = name.to_ascii_lowercase();
    Some(match &lower[..lower.len().min(3)] {
        "mon" => Weekday::Mon,
        "tue" => Weekday::Tue,
        "wed" => Weekday::Wed,
        "thu" => Weekday::Thu,
        "fri" => Weekday::Fri,
        "sat" => Weekday::Sat,
        "sun" => Weekday::Sun,
        _ => return None,
    })
}

fn weekday_from_zh(c: &str) -> Option<Weekday> {
    Some(match c {
        "一" => Weekday::Mon,
        "二" => Weekday::Tue,
        "三" => Weekday::Wed,
        "四" => Weekday::Thu,
        "五" => Weekday::Fri,
        "六" => Weekday::Sat,
        "日" | "天" => Weekday::Sun,
        _ => return None,
    })
}

/// The day of `weekday` in the week (Mon–Sun) containing `from`, or the
/// next one when `next_week` is set.
fn weekday_in_week(from: NaiveDate, weekday: Weekday, next_week: bool) -> NaiveDate {
    let monday = from - Duration::days(i64::from(from.weekday().num_days_from_monday()));
    let day = monday + Duration::days(i64::from(weekday.num_days_from_monday()));
    if next_week {
        day + Duration::days(7)
    } else {
        day
    }
}

/// "Friday" with no qualifier: the coming one (today counts).
fn upcoming_weekday(from: NaiveDate, weekday: Weekday) -> NaiveDate {
    let this_week = weekday_in_week(from, weekday, false);
    if this_week < from {
        this_week + Duration::days(7)
    } else {
        this_week
    }
}

/// A month/day without a year: this year, or next year when it is well in
/// the past (a December mail about "January 5").
fn month_day(from: NaiveDate, month: u32, day: u32) -> Option<NaiveDate> {
    let this_year = NaiveDate::from_ymd_opt(from.year(), month, day)?;
    if this_year < from - Duration::days(30) {
        NaiveDate::from_ymd_opt(from.year() + 1, month, day)
    } else {
        Some(this_year)
    }
}

fn parse_iso(text: &str) -> Option<NaiveDate> {
    let parts: Vec<u32> = text
        .split(|c: char| !c.is_ascii_digit())
        .filter(|p| !p.is_empty())
        .filter_map(|p| p.parse().ok())
        .collect();
    match parts.as_slice() {
        [y, m, d, ..] => NaiveDate::from_ymd_opt(*y as i32, *m, *d),
        _ => None,
    }
}

fn at(date: NaiveDate, time: Option<(u32, u32)>) -> (NaiveDateTime, bool) {
    let (h, m) = time.unwrap_or(END_OF_DAY);
    let time = NaiveTime::from_hms_opt(h.min(23), m.min(59), 0).unwrap_or(NaiveTime::MIN);
    (date.and_time(time), false)
}

fn group<'t>(caps: &Captures<'t>, name: &str) -> Option<&'t str> {
    caps.name(name).map(|m| m.as_str())
}

fn en_time(caps: &Captures) -> Option<(u32, u32)> {
    if group(caps, "noon").is_some() {
        return Some((12, 0));
    }
    if let Some(h) = group(caps, "h12").and_then(|h| h.parse::<u32>().ok()) {
        let m = group(caps, "m12").and_then(|m| m.parse().ok()).unwrap_or(0);
        let pm = group(caps, "ampm").is_some_and(|s| s.to_ascii_lowercase().starts_with('p'));
        let h = match (h % 12, pm) {
            (h, true) => h + 12,
            (h, false) => h,
        };
        return (h < 24 && m < 60).then_some((h, m));
    }
    let h = group(caps, "h24")?.parse::<u32>().ok()?;
    let m = group(caps, "m24")?.parse::<u32>().ok()?;
    if h == 24 && m == 0 {
        return Some((23, 59));
    }
    // "by Friday 5:00": nobody means 5 a.m.
    let h = if (1..=7).contains(&h) { h + 12 } else { h };
    (h < 24 && m < 60).then_some((h, m))
}

fn en_match(caps: &Captures, received: NaiveDateTime) -> Option<(NaiveDateTime, bool)> {
    let today = received.date();
    let time = en_time(caps);
    let date = if let Some(iso) = group(caps, "iso") {
        parse_iso(iso)?
    } else if let Some(eod) = group(caps, "eod") {
        let tonight = eod.eq_ignore_ascii_case("tonight");
        return Some(at(
            today,
            time.or(Some(if tonight { (23, 0) } else { END_OF_DAY })),
        ))
        .map(|(due, _)| (due, time.is_none() && !tonight));
    } else if group(caps, "tomorrow").is_some() {
        today + Duration::days(1)
    } else if group(caps, "eow").is_some() {
        weekday_in_week(today, Weekday::Fri, false)
    } else if let Some(name) = group(caps, "weekday") {
        let weekday = weekday_from_en(name)?;
        match group(caps, "next").map(str::to_ascii_lowercase).as_deref() {
            Some("next") => weekday_in_week(today, weekday, true),
            Some(_) => weekday_in_week(today, weekday, false),
            None => upcoming_weekday(today, weekday),
        }
    } else if let (Some(month), Some(day)) = (group(caps, "month1"), group(caps, "day1")) {
        month_day(today, month_from_en(month)?, day.parse().ok()?)?
    } else if let (Some(day), Some(month)) = (group(caps, "day2"), group(caps, "month2")) {
        month_day(today, month_from_en(month)?, day.parse().ok()?)?
    } else {
        return None;
    };
    let (due, _) = at(date, time);
    Some((due, time.is_none()))
}

fn zh_time(caps: &Captures) -> Option<(u32, u32)> {
    let part = group(caps, "part");
    let Some(hour) = group(caps, "hour").and_then(|h| h.parse::<u32>().ok()) else {
        return match part {
            Some("中午") => Some((12, 0)),
            Some("下班") | Some("傍晚") => Some(END_OF_DAY),
            Some("晚上") => Some((21, 0)),
            Some("上午") | Some("早上") => Some((11, 0)),
            Some("下午") => Some((17, 0)),
            _ => None,
        };
    };
    let minute = match group(caps, "minute") {
        Some("半") => 30,
        Some(m) => m.parse().ok()?,
        None => 0,
    };
    // "今晚8点" is 20:00.
    let part = part.or_else(|| (group(caps, "rel") == Some("今晚")).then_some("晚上"));
    // 晚上12点 and 24点 are the end of the day, not noon or invalid.
    if hour == 24 || (hour == 12 && matches!(part, Some("晚上"))) {
        return Some((23, 59));
    }
    let hour = match part {
        Some("下午" | "晚上" | "傍晚") if hour < 12 => hour + 12,
        Some("中午") if hour < 6 => hour + 12,
        _ => hour,
    };
    (hour < 24 && minute < 60).then_some((hour, minute))
}

fn zh_match(caps: &Captures, received: NaiveDateTime) -> Option<(NaiveDateTime, bool)> {
    // A deadline word must tie the date to the ask.
    if group(caps, "before").is_none() && group(caps, "after").is_none() {
        return None;
    }
    let today = received.date();
    let time = zh_time(caps);
    let date = if let Some(iso) = group(caps, "iso") {
        parse_iso(iso)?
    } else if let (Some(m), Some(d)) = (group(caps, "md"), group(caps, "mdd")) {
        month_day(today, m.parse().ok()?, d.parse().ok()?)?
    } else if let Some(rel) = group(caps, "rel") {
        match rel {
            "今天" | "今日" => today,
            "今晚" => return Some(at(today, time.or(Some((23, 0))))).map(|(d, _)| (d, false)),
            "明天" | "明日" | "明早" => today + Duration::days(1),
            _ => today + Duration::days(2),
        }
    } else if group(caps, "wk").is_some() {
        let weekday = weekday_from_zh(group(caps, "wkd")?)?;
        match group(caps, "wkp") {
            Some("下") => weekday_in_week(today, weekday, true),
            Some(_) => weekday_in_week(today, weekday, false),
            None => upcoming_weekday(today, weekday),
        }
    } else if group(caps, "eom").is_some() {
        let first_next = if today.month() == 12 {
            NaiveDate::from_ymd_opt(today.year() + 1, 1, 1)?
        } else {
            NaiveDate::from_ymd_opt(today.year(), today.month() + 1, 1)?
        };
        first_next - Duration::days(1)
    } else if group(caps, "eow").is_some() {
        weekday_in_week(today, Weekday::Fri, false)
    } else {
        return None;
    };
    let (due, _) = at(date, time);
    Some((due, time.is_none()))
}

/// Body text without quoted replies, limited to the part that is searched.
fn scan_text(subject: &str, body: &str) -> String {
    let body = crate::awaiting::strip_quoted(body);
    let mut text = String::with_capacity(subject.len() + SCAN_CHARS + 1);
    text.push_str(subject);
    text.push('\n');
    text.extend(body.chars().take(SCAN_CHARS));
    text
}

fn plausible(due: NaiveDateTime, received: NaiveDateTime) -> bool {
    // Earlier the same day is fine ("by noon" sent at 11:00 then read late);
    // a date before today is a reference to the past.
    due.date() >= received.date() && due - received <= Duration::days(MAX_AHEAD_DAYS)
}

/// After "截至…" / "…为止" these mean a status report, not a deadline.
const AS_OF_FOLLOW: &str = "，,、已共我累计目总";

/// After a bare 前 these make a word (前往, 前来, 前面…), not "before".
const NOT_BEFORE: &str = "往来面台后天进去景列排夕";

/// "by Friday last week": the date is in the past.
fn en_refers_to_past(after: &str) -> bool {
    let after = after.trim_start().to_lowercase();
    ["last week", "last month", "last year"].iter().any(|p| {
        after.starts_with(p)
            && !after[p.len()..].starts_with("'s")
            && !after[p.len()..].starts_with("’s")
    })
}

/// "上周五前", "去年12月10日前", "已于周五前": the date is in the past.
/// "线上周三前" (online, by Wednesday) is not.
fn zh_refers_to_past(before: &str, caps: &Captures) -> bool {
    let tail: String = {
        let chars: Vec<char> = before.chars().collect();
        chars[chars.len().saturating_sub(3)..].iter().collect()
    };
    let unqualified_week = caps.name("wk").is_some() && caps.name("wkp").is_none();
    if unqualified_week || caps.name("eom").is_some() {
        let last_week = tail.ends_with("上个") || tail.ends_with('上');
        // 上 ending another word: 线上, 网上, 早上, 晚上, 马上…
        let word = tail
            .trim_end_matches('个')
            .trim_end_matches('上')
            .chars()
            .last()
            .is_some_and(|c| "线网早晚马路班会楼身手面台".contains(c));
        if last_week && !word {
            return true;
        }
    }
    if caps.name("md").is_some() && tail.ends_with("去年") {
        return true;
    }
    tail.ends_with('已') || tail.ends_with("已经") || tail.ends_with("已于")
}

/// The earliest deadline mentioned in a message, if any.
pub fn find_deadline(subject: &str, body: &str, received: NaiveDateTime) -> Option<Deadline> {
    let text = scan_text(subject, body);
    let mut found: Vec<Deadline> = Vec::new();
    for caps in en_pattern().captures_iter(&text) {
        let whole = caps.get(0).expect("match");
        let rest = &text[whole.end()..];
        // "by today's standards" is no deadline.
        if en_refers_to_past(rest) || rest.starts_with("'s") || rest.starts_with("’s") {
            continue;
        }
        if let Some((due, all_day)) = en_match(&caps, received) {
            found.push(Deadline {
                due,
                all_day,
                phrase: whole.as_str().trim().to_string(),
            });
        }
    }
    for caps in zh_pattern().captures_iter(&text) {
        let whole = caps.get(0).expect("match");
        // The character after a bare 前 tells "明天前往" (go tomorrow) from
        // "明天前" (by tomorrow).
        let follow = caps.name("follow");
        if group(&caps, "after").map(str::trim) == Some("前")
            && follow.is_some_and(|f| NOT_BEFORE.contains(f.as_str()))
        {
            continue;
        }
        if zh_refers_to_past(&text[..whole.start()], &caps) {
            continue;
        }
        // "截至今天，已完成80%", "到今天为止我们收到…": a status line ("as of"),
        // not something to do by then.
        let as_of = group(&caps, "before").is_some_and(|b| b.contains("截至"))
            || group(&caps, "after").is_some_and(|a| a.contains("为止") || a.contains("截至"));
        if as_of && follow.is_some_and(|f| AS_OF_FOLLOW.contains(f.as_str())) {
            continue;
        }
        if let Some((due, all_day)) = zh_match(&caps, received) {
            let end = follow.map_or(whole.end(), |f| f.start());
            found.push(Deadline {
                due,
                all_day,
                phrase: text[whole.start()..end].trim().to_string(),
            });
        }
    }
    found
        .into_iter()
        .filter(|d| plausible(d.due, received))
        .min_by_key(|d| d.due)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Wednesday 2026-12-02 10:00.
    fn wed() -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 12, 2)
            .unwrap()
            .and_hms_opt(10, 0, 0)
            .unwrap()
    }

    fn due(subject: &str, body: &str) -> Option<String> {
        find_deadline(subject, body, wed()).map(|d| d.due.format("%Y-%m-%d %H:%M").to_string())
    }

    #[test]
    fn english_deadlines() {
        assert_eq!(
            due("Budget", "Could you send it by Friday?").as_deref(),
            Some("2026-12-04 18:00")
        );
        assert_eq!(
            due("", "Please reply by tomorrow 3pm.").as_deref(),
            Some("2026-12-03 15:00")
        );
        assert_eq!(
            due("", "Need this by EOD").as_deref(),
            Some("2026-12-02 18:00")
        );
        assert_eq!(
            due("", "Deadline: Dec 10").as_deref(),
            Some("2026-12-10 18:00")
        );
        assert_eq!(
            due("", "due on 15th December at 09:30").as_deref(),
            Some("2026-12-15 09:30")
        );
        assert_eq!(
            due("", "no later than next Monday").as_deref(),
            Some("2026-12-07 18:00")
        );
        assert_eq!(
            due("", "Submit before 2026-12-20.").as_deref(),
            Some("2026-12-20 18:00")
        );
        assert_eq!(
            due("", "by the end of the week").as_deref(),
            Some("2026-12-04 18:00")
        );
        // Year rollover.
        assert_eq!(due("", "by January 5").as_deref(), Some("2027-01-05 18:00"));
    }

    #[test]
    fn chinese_deadlines() {
        assert_eq!(
            due("", "请于周五前回复").as_deref(),
            Some("2026-12-04 18:00")
        );
        assert_eq!(
            due("", "明天下班前给我").as_deref(),
            Some("2026-12-03 18:00")
        );
        assert_eq!(
            due("", "截止日期：12月10日").as_deref(),
            Some("2026-12-10 18:00")
        );
        assert_eq!(
            due("", "下周三下午3点前提交").as_deref(),
            Some("2026-12-09 15:00")
        );
        assert_eq!(due("", "最晚今晚").as_deref(), Some("2026-12-02 23:00"));
        assert_eq!(
            due("报名", "截止到2026-12-15").as_deref(),
            Some("2026-12-15 18:00")
        );
        assert_eq!(due("", "月底之前").as_deref(), Some("2026-12-31 18:00"));
        assert_eq!(
            due("", "请在12月8号中午12点半前确认").as_deref(),
            Some("2026-12-08 12:30")
        );
    }

    #[test]
    fn earliest_wins_and_flags_all_day() {
        let found = find_deadline("", "Draft by Friday, final by Dec 20", wed()).unwrap();
        assert_eq!(found.due.format("%m-%d").to_string(), "12-04");
        assert!(found.all_day);
        assert_eq!(found.phrase, "by Friday");
        assert!(!find_deadline("", "by tomorrow 3pm", wed()).unwrap().all_day);
    }

    #[test]
    fn ignores_dates_without_a_deadline_word_and_the_past() {
        assert_eq!(due("", "See you Friday!"), None);
        assert_eq!(due("", "明天开会讨论一下"), None);
        assert_eq!(due("", "周五见"), None);
        assert_eq!(due("", "by Nov 3"), None, "a month ago");
        assert_eq!(due("", "by the way, thanks"), None);
        assert_eq!(due("", "截止2027-09-01"), None, "too far ahead");
        assert_eq!(due("", "Thanks for sending it by Friday last week."), None);
        assert_eq!(due("", "我上周五前已提交"), None);
        assert_eq!(due("", "已于周五前提交"), None);
        assert_eq!(due("", "明天前往会场"), None);
        assert_eq!(due("", "请大家后天前来"), None);
        assert_eq!(due("", "Photo by Cobb"), None);
        assert_eq!(due("", "generated by weekly job"), None);
        assert_eq!(due("", "before weekend plans"), None);
        assert_eq!(
            due("", "请在线上周三前完成培训").as_deref(),
            Some("2026-12-02 18:00")
        );
        assert_eq!(
            due("", "今晚8点前提交").as_deref(),
            Some("2026-12-02 20:00")
        );
        assert_eq!(due("", "今晚12点前").as_deref(), Some("2026-12-02 23:59"));
        assert_eq!(due("", "明天24点前").as_deref(), Some("2026-12-03 23:59"));
        assert_eq!(
            due("", "by Friday 5:00").as_deref(),
            Some("2026-12-04 17:00")
        );
        assert_eq!(
            due("", "by Friday 09:30").as_deref(),
            Some("2026-12-04 09:30")
        );
        assert_eq!(due("", "截至今天，已完成80%"), None);
        assert_eq!(due("", "到今天为止我们收到了30份"), None);
        assert_eq!(
            due("", "报名截至12月10日").as_deref(),
            Some("2026-12-10 18:00")
        );
        assert_eq!(due("", "Reviewed by Sun Wei"), None);
        assert_eq!(due("", "even by today's standards"), None);
        assert_eq!(
            due("", "by Sunday please").as_deref(),
            Some("2026-12-06 18:00")
        );
        assert_eq!(
            due("", "Please send by Monday last month's invoices").as_deref(),
            Some("2026-12-07 18:00")
        );
        // Quoted history doesn't count.
        assert_eq!(
            due("Re: x", "Sounds good.\n\n> Please reply by Friday"),
            None
        );
    }
}
