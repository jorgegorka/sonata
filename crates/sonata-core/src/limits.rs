use chrono::{DateTime, Datelike, Duration, NaiveDate, NaiveTime, TimeZone, Utc};

use crate::envelope::CONTINUE_LINE;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResetTime {
    pub date: Option<(Option<i32>, u32, u32)>,
    pub time: NaiveTime,
}

#[derive(Debug, Clone, PartialEq)]
pub struct UsageLimit {
    pub reset: Option<ResetTime>,
    pub line: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Copy, Default)]
pub struct Skip<'a> {
    pub prompt: Option<&'a str>,
    pub waited: &'a [String],
}

pub fn turn_output(screen: &str) -> &str {
    let mut start = 0;
    let mut offset = 0;
    for line in screen.split_inclusive('\n') {
        offset += line.len();
        if line.contains("[Message from ") || line.contains("[Recurring task \"") || line.contains(CONTINUE_LINE) {
            start = offset;
        }
    }
    &screen[start..]
}

const CLAUDE_MARKS: &[char] = &['⏺', '⎿'];

const CODEX_MARKS: &[char] = &['■'];

const OPENCODE_MARKS: &[char] = &['┃', '│'];

fn own_text<'a>(line: &'a str, marks: Option<&[char]>) -> Option<&'a str> {
    let line = line.trim();
    let first = line.chars().next()?;
    let rest = line[first.len_utf8()..].trim_start();
    match marks {
        Some(marks) => marks.contains(&first).then_some(rest),
        None if first.is_alphanumeric() => Some(line),
        None => Some(rest),
    }
}

fn plain(text: &str) -> String {
    text.replace('’', "'")
}

fn read_limit(
    screen: &str,
    skip: &Skip,
    marks: Option<&[char]>,
    is_limit: fn(&str) -> bool,
    reset_of: fn(&str) -> Option<ResetTime>,
    next_line_reset: bool,
) -> Option<UsageLimit> {
    let prompt = skip.prompt.map(plain);
    let own: Vec<Option<&str>> = turn_output(screen).lines().map(|raw| own_text(raw, marks)).collect();
    let lines: Vec<(&str, String, Option<String>)> = own
        .iter()
        .enumerate()
        .filter_map(|(at, line)| {
            let line = (*line)?;
            let normal = plain(line);
            if !is_limit(&normal) || prompt.as_deref().is_some_and(|prompt| prompt.contains(normal.as_str())) {
                return None;
            }
            let next = own.get(at + 1).copied().flatten().filter(|_| next_line_reset).map(plain);
            Some((line, normal, next))
        })
        .collect();
    let mut fresh = lines.iter().filter(|(line, ..)| !skip.waited.iter().any(|waited| waited.as_str() == *line));
    let (first, normal, next) = fresh.next()?;
    let reset = reset_of(normal)
        .or_else(|| next.as_deref().and_then(reset_of))
        .or_else(|| fresh.find_map(|(_, n, _)| reset_of(n)));
    let lines = lines.iter().map(|(line, ..)| line.to_string()).collect();
    Some(UsageLimit { reset, line: first.to_string(), lines })
}

fn starts_with_any(line: &str, phrases: &[&str]) -> bool {
    phrases.iter().any(|p| line.starts_with(p))
}

const CLAUDE_HIT: &str = "You've hit your ";

const CLAUDE_PHRASES: &[&str] = &["You're out of usage credits", "Your org is out of usage", "Usage limit reached"];

fn claude_line(line: &str) -> bool {
    match line.strip_prefix(CLAUDE_HIT) {
        Some(rest) => {
            let what = rest.split(" · ").next().unwrap_or_default().trim();
            what.ends_with("limit") || what.ends_with("budget")
        }
        None => starts_with_any(line, CLAUDE_PHRASES),
    }
}

const CODEX_PHRASES: &[&str] =
    &["You've hit your usage limit", "Your workspace is out of credits", "You hit your spend cap"];

const GROK_PHRASES: &[&str] = &[
    "You've hit your team's API rate limit",
    "You've hit the rate limit for your plan",
    "You've reached your free Grok Build usage limit",
    "You hit your free usage limit",
    "You hit your weekly limit",
    "You've hit the credit limit for your plan",
    "You've hit your spending cap",
];

pub fn claude_limit(screen: &str, skip: &Skip) -> Option<UsageLimit> {
    read_limit(screen, skip, Some(CLAUDE_MARKS), claude_line, claude_reset, false)
}

pub fn codex_limit(screen: &str, skip: &Skip) -> Option<UsageLimit> {
    read_limit(screen, skip, Some(CODEX_MARKS), |line| starts_with_any(line, CODEX_PHRASES), codex_reset, false)
}

pub fn grok_limit(screen: &str, skip: &Skip) -> Option<UsageLimit> {
    read_limit(screen, skip, None, |line| starts_with_any(line, GROK_PHRASES), |_| None, false)
}

fn opencode_line(line: &str) -> bool {
    line == "Usage limit reached" || line.ends_with(" usage limit reached") || line.ends_with(" is out of credits")
}

fn opencode_notice_line(line: &str) -> bool {
    line.trim_start()
        .strip_prefix(OPENCODE_MARKS)
        .and_then(|rest| rest.strip_prefix(' '))
        .is_some_and(|text| text.starts_with(|c: char| !c.is_whitespace()))
}

pub fn opencode_limit(screen: &str, skip: &Skip) -> Option<UsageLimit> {
    let notice: String =
        turn_output(screen).lines().filter(|line| opencode_notice_line(line)).map(|line| format!("{line}\n")).collect();
    read_limit(&notice, skip, Some(OPENCODE_MARKS), opencode_line, opencode_reset, true)
}

fn after<'a>(line: &'a str, marker: &str) -> Option<&'a str> {
    let at = line.to_ascii_lowercase().find(marker)?;
    Some(&line[at + marker.len()..])
}

fn claude_reset(line: &str) -> Option<ResetTime> {
    let text = after(line, "resets ").or_else(|| after(line, "continuing automatically at "))?;
    let end = [" (", " ·"].iter().filter_map(|m| text.find(m)).min().unwrap_or(text.len());
    let text = text[..end].trim();
    match text.split_once(" at ") {
        Some((date, time)) => ResetTime::parse(Some(date), time),
        None => ResetTime::parse(None, text),
    }
}

fn codex_reset(line: &str) -> Option<ResetTime> {
    let text = after(line, "try again at ")?;
    let text = text.trim().trim_end_matches('.').trim();
    let tokens: Vec<&str> = text.split_whitespace().collect();
    let split = tokens.len().checked_sub(2)?;
    let time = tokens[split..].concat();
    let date = tokens[..split].join(" ");
    ResetTime::parse((!date.is_empty()).then_some(date.as_str()), &time)
}

fn opencode_reset(line: &str) -> Option<ResetTime> {
    let text = after(line, "resets ")?;
    let text = text[..text.find('.')?].trim();
    if let Some(time) = text.strip_suffix(" today") {
        return ResetTime::parse(None, time);
    }
    match text.split_once(", ") {
        Some((date, time)) => ResetTime::parse(Some(date), time),
        None => Some(ResetTime { date: Some(parse_date(text)?), time: NaiveTime::MIN }),
    }
}

impl ResetTime {
    fn parse(date: Option<&str>, time: &str) -> Option<ResetTime> {
        let time = parse_clock(time)?;
        let date = match date {
            Some(date) => Some(parse_date(date)?),
            None => None,
        };
        Some(ResetTime { date, time })
    }
}

fn parse_clock(text: &str) -> Option<NaiveTime> {
    let lower = text.trim().to_ascii_lowercase();
    let (digits, pm) =
        if let Some(d) = lower.strip_suffix("pm") { (d, true) } else { (lower.strip_suffix("am")?, false) };
    let (hour, minute) = match digits.trim().split_once(':') {
        Some((h, m)) => (h, m),
        None => (digits.trim(), "0"),
    };
    let hour: u32 = hour.parse().ok().filter(|h| (1..=12).contains(h))?;
    let minute: u32 = minute.parse().ok().filter(|m| *m < 60)?;
    NaiveTime::from_hms_opt(hour % 12 + if pm { 12 } else { 0 }, minute, 0)
}

fn parse_date(text: &str) -> Option<(Option<i32>, u32, u32)> {
    let mut tokens = text.split(|c: char| c.is_whitespace() || c == ',').filter(|t| !t.is_empty());
    let month = tokens.next()?.to_ascii_lowercase();
    let month = 1 + ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"]
        .iter()
        .position(|m| month.starts_with(m))? as u32;
    let day = tokens.next()?.to_ascii_lowercase();
    let day = day.trim_end_matches(|c: char| c.is_ascii_alphabetic());
    let day: u32 = day.parse().ok().filter(|d| (1..=31).contains(d))?;
    let year = match tokens.next() {
        Some(y) => Some(y.parse::<i32>().ok()?),
        None => None,
    };
    if tokens.next().is_some() {
        return None;
    }
    Some((year, month, day))
}

pub fn reset_instant<Tz: TimeZone>(reset: ResetTime, now: DateTime<Utc>, tz: &Tz) -> Option<DateTime<Utc>> {
    let local =
        |date: NaiveDate| tz.from_local_datetime(&date.and_time(reset.time)).earliest().map(|d| d.with_timezone(&Utc));
    let today = now.with_timezone(tz).date_naive();
    match reset.date {
        None => {
            let first = local(today)?;
            if first > now {
                Some(first)
            } else {
                local(today.succ_opt()?)
            }
        }
        Some((Some(year), month, day)) => local(NaiveDate::from_ymd_opt(year, month, day)?),
        Some((None, month, day)) => {
            let this_year = local(NaiveDate::from_ymd_opt(today.year(), month, day)?)?;
            if this_year < now - Duration::days(1) {
                local(NaiveDate::from_ymd_opt(today.year() + 1, month, day)?)
            } else {
                Some(this_year)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::FixedOffset;

    const CLAUDE: &str = include_str!("../tests/fixtures/screens/claude-limit.txt");
    const CODEX: &str = include_str!("../tests/fixtures/screens/codex-limit.txt");
    const GROK: &str = include_str!("../tests/fixtures/screens/grok-limit.txt");
    const OPENCODE: &str = include_str!("../tests/fixtures/screens/opencode-limit.txt");

    const NONE: Skip = Skip { prompt: None, waited: &[] };

    fn typed(prompt: &str) -> Skip<'_> {
        Skip { prompt: Some(prompt), waited: &[] }
    }

    fn t(h: u32, m: u32) -> NaiveTime {
        NaiveTime::from_hms_opt(h, m, 0).unwrap()
    }

    fn reset(date: Option<(Option<i32>, u32, u32)>, h: u32, m: u32) -> Option<ResetTime> {
        Some(ResetTime { date, time: t(h, m) })
    }

    fn claude(line: &str) -> Option<UsageLimit> {
        claude_limit(
            &format!("❯ [Message from user · thread you↔user · language: English]\n  work\n  ⎿  {line}\n"),
            &NONE,
        )
    }

    fn codex(line: &str) -> Option<UsageLimit> {
        codex_limit(
            &format!("› [Message from user · thread you↔user · language: English]\n  work\n\n■ {line}\n"),
            &NONE,
        )
    }

    #[test]
    fn claude_reads_its_limit_and_reset() {
        let limit = claude_limit(CLAUDE, &NONE).unwrap();
        assert_eq!(limit.reset, reset(None, 14, 10));
        assert_eq!(limit.line, "You've hit your 5-hour limit · resets 2:10pm (Europe/Madrid)");
    }

    #[test]
    fn claude_reads_every_limit_wording() {
        let cases = [
            ("You've hit your weekly limit · resets Oct 12 at 8pm (Europe/Madrid)", reset(Some((None, 10, 12)), 20, 0)),
            ("You've hit your session limit · resets 7pm (Europe/Madrid)", reset(None, 19, 0)),
            ("You've hit your 5-hour limit · resets 12am (Europe/Madrid)", reset(None, 0, 0)),
            ("You're out of usage credits · resets 3pm (Europe/Madrid)", reset(None, 15, 0)),
            ("Your org is out of usage · add funds to continue", None),
        ];
        for (line, expected) in cases {
            let limit = claude(line).unwrap_or_else(|| panic!("no limit in {line}"));
            assert_eq!(limit.reset, expected, "{line}");
            assert_eq!(limit.line, line);
        }
    }

    #[test]
    fn claude_reads_you_have_hit_your_only_for_a_limit_or_a_budget() {
        assert_eq!(claude_limit("❯ [Message from user · x]\n⏺ You've hit your target for the month\n", &NONE), None);
        assert_eq!(claude("You've hit your stride · resets nothing"), None);
        for line in [
            "You've hit your monthly spend limit · resets 3pm (Europe/Madrid)",
            "You've hit your team's shared budget · resets 3pm (Europe/Madrid)",
            "You've hit your limit · resets 3pm (Europe/Madrid)",
        ] {
            let limit = claude(line).unwrap_or_else(|| panic!("no limit in {line}"));
            assert_eq!(limit.reset, reset(None, 15, 0), "{line}");
            assert_eq!(limit.line, line);
        }
        for line in ["You've hit your usage limit", "You’ve hit your Opus limit", "You've hit your usage credit limit"]
        {
            assert!(claude(line).is_some(), "{line}");
        }
    }

    #[test]
    fn claude_takes_the_reset_of_a_later_line_when_the_first_has_none() {
        let screen = "❯ [Message from user · x]\n  ⎿  You've hit your usage limit\n⏺ Usage limit reached · continuing automatically at 2:10pm · esc to cancel\n";
        let limit = claude_limit(screen, &NONE).unwrap();
        assert_eq!(limit.line, "You've hit your usage limit");
        assert_eq!(limit.reset, reset(None, 14, 10));
    }

    #[test]
    fn codex_reads_its_limit_and_reset() {
        let limit = codex_limit(CODEX, &NONE).unwrap();
        assert_eq!(limit.reset, reset(None, 15, 5));
        assert!(limit.line.starts_with("You’ve hit your usage limit."));
        let dated = codex("You’ve hit your usage limit. Upgrade or try again at Nov 7th, 2026 3:05 PM.").unwrap();
        assert_eq!(dated.reset, reset(Some((Some(2026), 11, 7)), 15, 5));
        assert_eq!(codex("You’ve hit your usage limit. Try again later.").unwrap().reset, None);
        assert_eq!(codex("You've hit your usage limit. Try again at 3:05 PM.").unwrap().reset, reset(None, 15, 5));
        assert_eq!(codex("Your workspace is out of credits. Add credits to continue.").unwrap().reset, None);
        assert_eq!(codex("You hit your spend cap. Ask your admin.").unwrap().reset, None);
    }

    #[test]
    fn grok_reads_a_limit_without_a_reset() {
        assert_eq!(grok_limit(GROK, &NONE).unwrap().reset, None);
        for text in [
            "You’ve hit your team’s API rate limit. Try again later.",
            "You’ve hit the rate limit for your plan. Try again later.",
            "You’ve reached your free Grok Build usage limit for now. Upgrade.",
            "You hit your free usage limit. Try again later.",
            "You hit your weekly limit.",
            "You’ve hit the credit limit for your plan.",
            "You’ve hit your spending cap.",
            "You've hit your spending cap.",
        ] {
            let screen = format!("❯ [Message from user · x]\n\n     {text}\n");
            let limit = grok_limit(&screen, &NONE).unwrap_or_else(|| panic!("no limit in {text}"));
            assert_eq!(limit.reset, None, "{text}");
            assert_eq!(limit.line, text);
        }
    }

    #[test]
    fn a_quoted_limit_is_not_a_limit() {
        for quoted in [
            "+    \"You've hit your 5-hour limit · resets 2:10pm\"",
            "assert!(x.contains(\"You’ve hit your usage limit\"))",
            "assert!(x.contains(\"You’ve hit the rate limit for your plan\"))",
        ] {
            let screen = format!("❯ [Message from user · x]\n  {quoted}\n");
            assert_eq!(claude_limit(&screen, &NONE), None, "{quoted}");
            assert_eq!(codex_limit(&screen, &NONE), None, "{quoted}");
            assert_eq!(grok_limit(&screen, &NONE), None, "{quoted}");
        }
    }

    #[test]
    fn an_earlier_turns_limit_is_not_this_turns() {
        let next = format!("{CLAUDE}\n❯ [Message from user · thread you↔user · language: English]\n  more work\n");
        assert_eq!(claude_limit(&next, &NONE), None);
        let continued = format!("{CLAUDE}\n❯ The usage limit has reset. Continue where you stopped.\n");
        assert_eq!(claude_limit(&continued, &NONE), None);
        let task = format!("{CODEX}\n› [Recurring task \"x\"]\n  work\n");
        assert_eq!(codex_limit(&task, &NONE), None);
    }

    #[test]
    fn turn_output_without_a_marker_is_the_whole_screen() {
        let screen = "⏺ Reading\n  ⎿  You've hit your 5-hour limit · resets 2:10pm (Europe/Madrid)\n";
        assert_eq!(turn_output(screen), screen);
        assert_eq!(turn_output(""), "");
    }

    #[test]
    fn turn_output_starts_below_the_last_marker() {
        let screen = format!(
            "❯ [Message from user · x]\n  one\n⏺ A\n❯ [Recurring task \"Wiki\" from the user]\n  two\n⏺ B\n❯ {CONTINUE_LINE}\n⏺ C\n"
        );
        assert_eq!(turn_output(&screen), "⏺ C\n");
        let codex =
            "› [Message from user · x]\n  one\n• A\n› [Message from @writer · thread you↔@writer]\n  two\n• B\n";
        assert_eq!(turn_output(codex), "  two\n• B\n");
    }

    #[test]
    fn a_limit_in_the_echo_of_the_prompt_is_not_a_limit() {
        let body = "Why?\nYou've hit your weekly limit · resets Nov 9 at 8pm (Europe/Madrid)\n⎿  You've hit your 5-hour limit · resets 2:10pm (Europe/Madrid)";
        let prompt = format!("[Message from user · thread you↔user · language: English]\n{body}");
        let echo: String = body.lines().map(|line| format!("  {line}\n")).collect();
        let screen =
            format!("❯ [Message from user · thread you↔user · language: English]\n{echo}\n⏺ It is the 5-hour limit.\n");
        assert_eq!(claude_limit(&screen, &typed(&prompt)), None);
        assert!(claude_limit(&screen, &NONE).is_some(), "the marked line counts when the prompt is not known");

        let body = "Why?\nYou’ve hit your usage limit. Try again at 3:05 PM.\n■ You’ve hit your usage limit. Try again at 4:05 PM.";
        let prompt = format!("[Message from user · thread you↔user · language: English]\n{body}");
        let echo: String = body.lines().map(|line| format!("  {line}\n")).collect();
        let screen =
            format!("› [Message from user · thread you↔user · language: English]\n{echo}\n• It is codex's limit.\n");
        assert_eq!(codex_limit(&screen, &typed(&prompt)), None);
        assert!(codex_limit(&screen, &NONE).is_some(), "the marked line counts when the prompt is not known");

        let body = "You hit your weekly limit. What does that mean?";
        let prompt = format!("[Message from user · thread you↔user · language: English]\n{body}");
        let screen = format!(
            "     ❯ [Message from user · thread you↔user · language: English]   2:33 PM\n       {body}\n\n     It means your weekly usage is spent.\n"
        );
        assert_eq!(grok_limit(&screen, &typed(&prompt)), None);
        assert!(grok_limit(&screen, &NONE).is_some(), "the line counts when the prompt is not known");
    }

    #[test]
    fn a_limit_in_a_bots_brief_is_not_a_limit() {
        let body = "If your screen shows\n⏺ Usage limit reached · continuing automatically at 2:10pm · esc to cancel\nwait for it.";
        let prompt = format!("[Message from @master · thread you↔@master]\n{body}");
        let echo: String = body.lines().map(|line| format!("  {line}\n")).collect();
        let screen = format!("❯ [Message from @master · thread you↔@master]\n{echo}\n⏺ Noted.\n");
        assert_eq!(claude_limit(&screen, &typed(&prompt)), None);
    }

    #[test]
    fn a_claude_heredoc_line_is_not_a_limit() {
        let screen = "❯ [Message from user · x]\n  Brief @writer.\n\n⏺ Bash(sbot send @writer <<'EOF'\n      If you see these, wait:\n      You've hit your 5-hour limit · resets 2:10pm (Europe/Madrid)\n      Usage limit reached · continuing automatically at 2:10pm · esc to cancel\n      EOF)\n  ⎿  Sent to @writer.\n";
        assert_eq!(claude_limit(screen, &NONE), None);
    }

    #[test]
    fn a_line_without_its_clis_mark_is_not_its_limit() {
        for line in [
            "You've hit your 5-hour limit · resets 2:10pm (Europe/Madrid)",
            "* You've hit your 5-hour limit · resets 2:10pm (Europe/Madrid)",
            "• You’ve hit your usage limit. Try again at 3:05 PM.",
            "> Usage limit reached · continuing automatically at 2:10pm · esc to cancel",
        ] {
            let screen = format!("❯ [Message from user · x]\n  work\n\n{line}\n");
            assert_eq!(claude_limit(&screen, &NONE), None, "{line}");
            assert_eq!(codex_limit(&screen, &NONE), None, "{line}");
        }
    }

    #[test]
    fn a_real_limit_below_an_echoed_one_is_found() {
        let quoted = "⎿  You've hit your weekly limit · resets Nov 9 at 8pm (Europe/Madrid)";
        let prompt = format!("[Message from user · x]\nSee:\n{quoted}");
        let screen = format!(
            "❯ [Message from user · x]\n  See:\n  {quoted}\n\n⏺ Reading\n  ⎿  You've hit your 5-hour limit · resets 2:10pm (Europe/Madrid)\n"
        );
        let limit = claude_limit(&screen, &typed(&prompt)).unwrap();
        assert_eq!(limit.line, "You've hit your 5-hour limit · resets 2:10pm (Europe/Madrid)");
        assert_eq!(limit.reset, reset(None, 14, 10));
        assert_eq!(limit.lines, [limit.line.as_str()], "the echo is not among the lines waited on");
    }

    #[test]
    fn the_lines_a_session_waited_on_are_passed_over() {
        let first = claude_limit(CLAUDE, &NONE).unwrap();
        assert_eq!(
            first.lines,
            [
                "You've hit your 5-hour limit · resets 2:10pm (Europe/Madrid)",
                "Usage limit reached · continuing automatically at 2:10pm · esc to cancel"
            ]
        );
        let waited = Skip { prompt: None, waited: &first.lines };
        assert_eq!(claude_limit(CLAUDE, &waited), None);
        let weekly = format!(
            "{CLAUDE}\n⏺ Checking the build\n  ⎿  You've hit your weekly limit · resets Nov 9 at 8pm (Europe/Madrid)\n"
        );
        let limit = claude_limit(&weekly, &waited).unwrap();
        assert_eq!(limit.line, "You've hit your weekly limit · resets Nov 9 at 8pm (Europe/Madrid)");
        assert_eq!(limit.reset, reset(Some((None, 11, 9)), 20, 0));
        assert_eq!(limit.lines.len(), 3, "the new wait passes over all three afterwards");
    }

    #[test]
    fn a_reset_has_already_reset_message_is_not_a_limit() {
        let screen = "❯ [Message from user · x]\n⏺ Your usage limit has reset\n  Press enter to continue\n";
        assert_eq!(claude_limit(screen, &NONE), None);
        assert_eq!(codex_limit(screen, &NONE), None);
        assert_eq!(grok_limit(screen, &NONE), None);
    }

    #[test]
    fn odd_input_does_not_panic() {
        assert_eq!(claude_limit("", &NONE), None);
        assert_eq!(claude_limit("⎿", &NONE), None);
        assert_eq!(claude_limit("⎿  ", &NONE), None);
        assert_eq!(codex_limit("■", &NONE), None);
        let odd = claude("You've hit your 5-hour limit · resets 25pm (Europe/Madrid)").unwrap();
        assert_eq!(odd.reset, None);
        let odd = claude("You've hit your 5-hour limit · resets Feb 40 at 8pm").unwrap();
        assert_eq!(odd.reset, None);
        let odd = codex("You’ve hit your usage limit. Try again at 13:99 PM.").unwrap();
        assert_eq!(odd.reset, None);
        let odd = claude("You've hit your limit · resets é").unwrap();
        assert_eq!(odd.reset, None);
    }

    #[test]
    fn reset_instant_picks_the_next_occurrence() {
        let tz = FixedOffset::east_opt(3600).unwrap();
        let now = Utc.with_ymd_and_hms(2026, 11, 5, 11, 0, 0).unwrap();
        let utc = |y, mo, d, h, mi| Utc.with_ymd_and_hms(y, mo, d, h, mi, 0).unwrap();
        let at = |date, h, m| reset_instant(ResetTime { date, time: t(h, m) }, now, &tz);
        assert_eq!(at(None, 14, 10), Some(utc(2026, 11, 5, 13, 10)));
        assert_eq!(at(None, 9, 0), Some(utc(2026, 11, 6, 8, 0)));
        assert_eq!(at(Some((None, 10, 12)), 20, 0), Some(utc(2027, 10, 12, 19, 0)));
        assert_eq!(at(Some((None, 11, 5)), 23, 0), Some(utc(2026, 11, 5, 22, 0)));
        assert_eq!(at(Some((Some(2026), 11, 7)), 15, 5), Some(utc(2026, 11, 7, 14, 5)));
        assert_eq!(at(Some((None, 2, 30)), 9, 0), None);
    }

    fn opencode(notice: &str) -> Option<UsageLimit> {
        let boxed: String = notice.lines().map(|line| format!("  ┃ {line}\n")).collect();
        opencode_limit(
            &format!("  ┃  [Message from user · thread you↔user · language: English]\n  ┃  work\n\n  ┃\n{boxed}  ┃\n"),
            &NONE,
        )
    }

    #[test]
    fn opencode_reads_the_go_limit_and_its_reset_on_the_next_line() {
        let limit = opencode_limit(OPENCODE, &NONE).expect("a limit");
        assert_eq!(limit.line, "OpenCode Go 5-hour usage limit reached");
        assert_eq!(limit.reset, reset(None, 15, 5));
    }

    #[test]
    fn opencode_reads_each_notice_title() {
        for title in [
            "Usage limit reached",
            "OpenCode Go weekly usage limit reached",
            "Kimi K3 usage limit reached",
            "Acme is out of credits",
            "Your workspace is out of credits",
        ] {
            assert!(opencode(title).is_some(), "{title}");
        }
        assert!(opencode_limit("  │ Usage limit reached\n", &NONE).is_some(), "the box's other border");
    }

    #[test]
    fn opencode_reads_the_reset_forms_of_its_notice() {
        let title = "OpenCode Go weekly usage limit reached";
        let reset_of = |body: &str| opencode(&format!("{title}\n{body}")).expect(body).reset;
        assert_eq!(
            reset_of("Resets 3:05 PM today. Switch to a free model with /models to continue."),
            reset(None, 15, 5)
        );
        assert_eq!(
            reset_of("Resets Nov 14, 9:00 AM. Switch to a free model with /models to continue."),
            reset(Some((None, 11, 14)), 9, 0)
        );
        assert_eq!(
            reset_of("Resets Nov 14. Switch model with /models to continue."),
            reset(Some((None, 11, 14)), 0, 0)
        );
        assert_eq!(reset_of("Switch to a free model with /models to continue."), None, "no reset from the API");
    }

    #[test]
    fn opencode_prose_tool_lines_and_echo_are_no_limit() {
        let prompt = "  ┃  [Message from user · thread you↔user · language: English]\n  ┃  work\n";
        assert_eq!(
            opencode_limit(&format!("{prompt}\n     The usage limit reached\n"), &NONE),
            None,
            "the bot's prose"
        );
        let tool = format!(
            "{prompt}\n  ┃\n  ┃  $ cat notes.txt\n  ┃\n  ┃  Usage limit reached\n  ┃  Acme is out of credits\n"
        );
        assert_eq!(opencode_limit(&tool, &NONE), None, "a tool's output, two spaces in");
        assert_eq!(opencode("Usage limit reached…"), None, "a line cut short");
        let asked = "[Message from user · thread you↔user · language: English]\nUsage limit reached";
        let echo = "  ┃  [Message from user · thread you↔user · language: English]\n  ┃  Usage limit reached\n";
        assert_eq!(opencode_limit(echo, &typed(asked)), None, "the prompt's echo");
        let working = include_str!("../tests/fixtures/screens/opencode-working.txt");
        assert_eq!(opencode_limit(working, &NONE), None);
    }
}
