//! The timestamp layouts a log line can start with, and reading one line of each.

use chrono::{DateTime, Datelike as _, Duration, NaiveDate, NaiveDateTime, NaiveTime, Utc};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogFormat {
    SyslogShort,
    SyslogShortMicro,
    Iso8601Space,
    Iso8601T,
}

pub fn detect_format(line: &str) -> Option<LogFormat> {
    // SyslogShortMicro before SyslogShort so the more-specific pattern wins
    if parse_syslog(line, true).is_some() {
        return Some(LogFormat::SyslogShortMicro);
    }
    if parse_syslog(line, false).is_some() {
        return Some(LogFormat::SyslogShort);
    }
    if parse_iso_space(line).is_some() {
        return Some(LogFormat::Iso8601Space);
    }
    if parse_iso_t(line).is_some() {
        return Some(LogFormat::Iso8601T);
    }
    None
}

/// Parses a space-padded 1–2 digit day such as `" 9"` or `"29"` at `pos` in
/// `bytes`. Returns `(day_value, end_pos)` where `end_pos == pos + 2`.
fn parse_day(bytes: &[u8], pos: usize) -> Option<(u32, usize)> {
    let end = pos + 2;
    let s = std::str::from_utf8(bytes.get(pos..end)?).ok()?;
    let day: u32 = s.trim().parse().ok()?;
    Some((day, end))
}

/// Parses the leading `"HH:MM:SS"` of `s` and returns `(hour, minute, second)`.
fn parse_hms(s: &str) -> Option<(u32, u32, u32)> {
    let b = s.as_bytes();
    if b.get(2).copied() != Some(b':') || b.get(5).copied() != Some(b':') {
        return None;
    }
    let hour: u32 = s.get(..2)?.parse().ok()?;
    let min: u32 = s.get(3..5)?.parse().ok()?;
    let sec: u32 = s.get(6..8)?.parse().ok()?;
    Some((hour, min, sec))
}

/// Parses a fractional-seconds suffix beginning with `'.'` (e.g. `".123456 rest"`).
/// Returns `(nanoseconds, remaining_trimmed)`.
fn parse_fractional_seconds(rest: &str) -> Option<(u32, &str)> {
    if rest.as_bytes().first().copied() != Some(b'.') {
        return None;
    }
    let frac_end = rest
        .get(1..)
        .unwrap_or("")
        .find(|c: char| !c.is_ascii_digit())
        .map_or(rest.len(), |i| i + 1);
    let frac_str = rest.get(1..frac_end)?;
    if frac_str.is_empty() {
        return None;
    }
    // Normalise to 9 digits (nanoseconds precision).
    let padded = format!("{:0<9}", frac_str);
    let nano: u32 = padded.get(..9)?.parse().ok()?;
    let after = rest.get(frac_end..).unwrap_or("").trim_start();
    Some((nano, after))
}

/// `"May 29 18:48:24"` or `"May 29 18:48:24.123456"` - returns `(NaiveDateTime, rest)`.
fn parse_syslog(line: &str, micro: bool) -> Option<(NaiveDateTime, &str)> {
    // Pattern: "MMM DD HH:MM:SS[.ffffff] rest"
    let bytes = line.as_bytes();
    if bytes.len() < 15 {
        return None;
    }
    let month = parse_month_abbrev(line.get(..3)?)?;
    if bytes.get(3).copied() != Some(b' ') {
        return None;
    }
    let (day, day_end) = parse_day(bytes, 4)?;
    if bytes.get(day_end).copied() != Some(b' ') {
        return None;
    }
    let time_start = day_end + 1;
    let (hour, min, sec) = parse_hms(line.get(time_start..)?)?;
    let time_end = time_start + 8;
    let (nano, after_time) = if micro {
        parse_fractional_seconds(line.get(time_end..)?)?
    } else {
        (0u32, line.get(time_end..).unwrap_or("").trim_start())
    };
    let dt = NaiveDateTime::new(
        NaiveDate::from_ymd_opt(2000, month, day)?,
        NaiveTime::from_hms_nano_opt(hour, min, sec, nano)?,
    );
    Some((dt, after_time))
}

fn parse_iso_space(line: &str) -> Option<(DateTime<Utc>, &str)> {
    // "YYYY-MM-DD HH:MM:SS rest"
    if line.len() < 19 {
        return None;
    }
    let ts = line.get(..19)?;
    let dt = NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S").ok()?;
    let utc = dt.and_utc();
    let rest = line.get(19..).unwrap_or("").trim_start();
    Some((utc, rest))
}

fn parse_iso_t(line: &str) -> Option<(DateTime<Utc>, &str)> {
    // RFC 3339: ends at first space (or end of line)
    let end = line.find(' ').unwrap_or(line.len());
    let ts = line.get(..end)?;
    let dt = DateTime::parse_from_rfc3339(ts).ok()?.to_utc();
    let rest = line.get(end..).unwrap_or("").trim_start();
    Some((dt, rest))
}

fn parse_month_abbrev(s: &str) -> Option<u32> {
    match s {
        "Jan" => Some(1),
        "Feb" => Some(2),
        "Mar" => Some(3),
        "Apr" => Some(4),
        "May" => Some(5),
        "Jun" => Some(6),
        "Jul" => Some(7),
        "Aug" => Some(8),
        "Sep" => Some(9),
        "Oct" => Some(10),
        "Nov" => Some(11),
        "Dec" => Some(12),
        _ => None,
    }
}

/// Resolves the year the year-less syslog formats leave out, reading a
/// timestamp more than an hour ahead of `now` as last year's.
pub fn infer_year(naive: NaiveDateTime, now: DateTime<Utc>) -> DateTime<Utc> {
    let current_year = now.year();
    let candidate = naive.with_year(current_year).unwrap_or(naive).and_utc();
    if candidate > now + Duration::hours(1) {
        naive.with_year(current_year - 1).unwrap_or(naive).and_utc()
    } else {
        candidate
    }
}

/// Reads `line` as `format`, returning its timestamp and the message left after
/// the timestamp. The message is a slice of `line`.
pub(crate) fn parse_line(
    line: &str,
    format: LogFormat,
    now: DateTime<Utc>,
) -> Option<(DateTime<Utc>, &str)> {
    match format {
        LogFormat::SyslogShort => {
            let (naive, rest) = parse_syslog(line, false)?;
            Some((infer_year(naive, now), rest))
        }
        LogFormat::SyslogShortMicro => {
            let (naive, rest) = parse_syslog(line, true)?;
            Some((infer_year(naive, now), rest))
        }
        LogFormat::Iso8601Space => parse_iso_space(line),
        LogFormat::Iso8601T => parse_iso_t(line),
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;
    use rstest::rstest;

    use super::*;

    fn utc(y: i32, mo: u32, d: u32, h: u32, m: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, m, s)
            .single()
            .expect("valid")
    }

    #[rstest]
    #[case("May 29 18:48:24 host proc: msg", Some(LogFormat::SyslogShort))]
    #[case(
        "May 29 18:48:24.123456 host proc: msg",
        Some(LogFormat::SyslogShortMicro)
    )]
    #[case("2026-05-29 18:48:24 host proc: msg", Some(LogFormat::Iso8601Space))]
    #[case("2026-05-29T18:48:24Z host proc: msg", Some(LogFormat::Iso8601T))]
    #[case("not a timestamp", None)]
    fn a_line_is_detected_as_the_format_it_starts_with(
        #[case] line: &str,
        #[case] expected: Option<LogFormat>,
    ) {
        assert_eq!(detect_format(line), expected);
    }

    #[rstest]
    #[case::december_read_in_january(12, 31, 23, 59, utc(2026, 1, 1, 0, 0, 0), 2025)]
    #[case::january_read_in_january(1, 1, 0, 1, utc(2026, 1, 15, 12, 0, 0), 2026)]
    #[case::two_hours_ahead(5, 23, 12, 0, utc(2026, 5, 23, 10, 0, 0), 2025)]
    #[case::half_an_hour_ahead(5, 23, 10, 30, utc(2026, 5, 23, 10, 0, 0), 2026)]
    fn a_year_less_timestamp_ahead_of_now_belongs_to_last_year(
        #[case] month: u32,
        #[case] day: u32,
        #[case] hour: u32,
        #[case] minute: u32,
        #[case] now: DateTime<Utc>,
        #[case] expected_year: i32,
    ) {
        let naive = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2000, month, day).expect("valid date"),
            NaiveTime::from_hms_opt(hour, minute, 0).expect("valid time"),
        );
        assert_eq!(infer_year(naive, now).year(), expected_year);
    }
}
