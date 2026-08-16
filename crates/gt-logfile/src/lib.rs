use chrono::{DateTime, Datelike, NaiveDateTime, Utc};
use gt_types::{AssociationConfig, CustomMarker, Latitude, Longitude, MarkerIcon, NavPoint};

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
        chrono::NaiveDate::from_ymd_opt(2000, month, day)?,
        chrono::NaiveTime::from_hms_nano_opt(hour, min, sec, nano)?,
    );
    Some((dt, after_time))
}

fn parse_iso_space(line: &str) -> Option<(DateTime<Utc>, &str)> {
    // "YYYY-MM-DD HH:MM:SS rest"
    if line.len() < 19 {
        return None;
    }
    let ts = line.get(..19)?;
    let dt = chrono::NaiveDateTime::parse_from_str(ts, "%Y-%m-%d %H:%M:%S").ok()?;
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

pub fn infer_year(naive: NaiveDateTime, now: DateTime<Utc>) -> DateTime<Utc> {
    let current_year = now.year();
    let candidate = naive.with_year(current_year).unwrap_or(naive).and_utc();
    if candidate > now + chrono::Duration::hours(1) {
        naive.with_year(current_year - 1).unwrap_or(naive).and_utc()
    } else {
        candidate
    }
}

fn parse_line(line: &str, format: LogFormat, now: DateTime<Utc>) -> Option<(DateTime<Utc>, &str)> {
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

#[expect(
    clippy::indexing_slicing,
    reason = "DP indices are in bounds by loop invariants: a/b indexed at i-1/j-1, vecs sized n+1"
)]
fn levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let m = a.len();
    let n = b.len();
    let mut prev: Vec<usize> = (0..=n).collect();
    let mut curr = vec![0usize; n + 1];
    for i in 1..=m {
        curr[0] = i;
        for j in 1..=n {
            let cost = usize::from(a[i - 1] != b[j - 1]);
            curr[j] = (prev[j] + 1).min(curr[j - 1] + 1).min(prev[j - 1] + cost);
        }
        std::mem::swap(&mut prev, &mut curr);
    }
    prev[n]
}

struct ColorGroupAssigner {
    representatives: Vec<String>,
}

impl ColorGroupAssigner {
    fn new() -> Self {
        Self {
            representatives: Vec::new(),
        }
    }

    fn assign(&mut self, s: &str) -> u32 {
        let best = self
            .representatives
            .iter()
            .enumerate()
            .map(|(id, rep)| (id, levenshtein(s, rep)))
            .min_by_key(|&(_, d)| d);

        if let Some((id, dist)) = best
            && dist <= 5
        {
            return id as u32;
        }

        let new_id = self.representatives.len() as u32;
        self.representatives.push(s.to_owned());
        new_id
    }
}

pub struct LogLoadResult {
    pub markers: Vec<CustomMarker>,
    pub unassociated: Vec<(DateTime<Utc>, String)>,
}

pub fn load_log(
    content: &str,
    nav_points: &[NavPoint],
    now: DateTime<Utc>,
    config: &AssociationConfig,
) -> LogLoadResult {
    let empty = LogLoadResult {
        markers: Vec::new(),
        unassociated: Vec::new(),
    };

    // Detect format from first 10 non-empty lines
    let format = {
        let mut found = None;
        let mut scanned = 0;
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if let Some(fmt) = detect_format(trimmed) {
                found = Some(fmt);
                break;
            }
            scanned += 1;
            if scanned >= 10 {
                break;
            }
        }
        let Some(f) = found else { return empty };
        f
    };

    // Parse all lines with consecutive-failure abort
    let mut entries: Vec<(DateTime<Utc>, String)> = Vec::new();
    let mut consecutive_failures: u32 = 0;

    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match parse_line(trimmed, format, now) {
            Some((ts, rest)) => {
                consecutive_failures = 0;
                entries.push((ts, rest.to_owned()));
            }
            None => {
                consecutive_failures += 1;
                if consecutive_failures >= 3 {
                    log::debug!("gt-log-marker: aborting parse after 3 consecutive failures");
                    return empty;
                }
            }
        }
    }

    entries.sort_by_key(|(ts, _)| *ts);

    // Assign color groups and associate with nav track
    let mut assigner = ColorGroupAssigner::new();
    let mut markers: Vec<CustomMarker> = Vec::new();
    let mut unassociated: Vec<(DateTime<Utc>, String)> = Vec::new();

    for (ts, log_str) in &entries {
        let group_id = assigner.assign(log_str);
        let Some((lat, lon)) = associate(ts, nav_points, config.log_marker_window_s as i64) else {
            unassociated.push((*ts, log_str.clone()));
            continue;
        };

        markers.push(CustomMarker::new(
            *ts,
            log_str.clone(),
            MarkerIcon::Log,
            Latitude::new(lat),
            Longitude::new(lon),
            Some(group_id),
        ));
    }

    LogLoadResult {
        markers,
        unassociated,
    }
}

fn associate(ts: &DateTime<Utc>, nav_points: &[NavPoint], window_s: i64) -> Option<(f64, f64)> {
    if nav_points.is_empty() {
        return None;
    }

    let idx = nav_points.partition_point(|p| p.tpv.time().utc() <= *ts);

    let before = idx.checked_sub(1).and_then(|i| nav_points.get(i));
    let after = nav_points.get(idx);

    let nearest_gap = match (before, after) {
        (Some(b), Some(a)) => {
            let gap_before = (*ts - b.tpv.time().utc()).abs();
            let gap_after = (a.tpv.time().utc() - *ts).abs();
            gap_before.min(gap_after)
        }
        (Some(b), None) => (*ts - b.tpv.time().utc()).abs(),
        (None, Some(a)) => (a.tpv.time().utc() - *ts).abs(),
        (None, None) => return None,
    };

    if nearest_gap > chrono::Duration::seconds(window_s) {
        return None;
    }

    let (lat, lon) = match (before, after) {
        (Some(b), Some(a)) => {
            let span = (a.tpv.time() - b.tpv.time())
                .num_microseconds()
                .unwrap_or(1);
            let elapsed = (*ts - b.tpv.time().utc()).num_microseconds().unwrap_or(0);
            let t = if span == 0 {
                0.0f64
            } else {
                elapsed as f64 / span as f64
            };
            let lat = b.tpv.lat().as_degrees() * (1.0 - t) + a.tpv.lat().as_degrees() * t;
            let lon = b.tpv.lon().as_degrees() * (1.0 - t) + a.tpv.lon().as_degrees() * t;
            (lat, lon)
        }
        (Some(b), None) => (b.tpv.lat().as_degrees(), b.tpv.lon().as_degrees()),
        (None, Some(a)) => (a.tpv.lat().as_degrees(), a.tpv.lon().as_degrees()),
        (None, None) => return None,
    };

    Some((lat, lon))
}

#[cfg(test)]
mod tests {
    use gt_test_utils::{assert_matches_sequence, nav_points_from};

    use super::*;
    use chrono::TimeZone;

    fn utc(y: i32, mo: u32, d: u32, h: u32, m: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, m, s)
            .single()
            .expect("valid")
    }

    #[test]
    fn lev_empty() {
        assert_eq!(levenshtein("", ""), 0);
    }

    #[test]
    fn lev_same() {
        assert_eq!(levenshtein("abc", "abc"), 0);
    }

    #[test]
    fn lev_kitten_sitting() {
        assert_eq!(levenshtein("kitten", "sitting"), 3);
    }

    #[test]
    fn lev_abc_empty() {
        assert_eq!(levenshtein("abc", ""), 3);
    }

    #[test]
    fn lev_empty_abc() {
        assert_eq!(levenshtein("", "abc"), 3);
    }

    #[test]
    fn detect_syslog_short() {
        assert_eq!(
            detect_format("May 29 18:48:24 host proc: msg"),
            Some(LogFormat::SyslogShort)
        );
    }

    #[test]
    fn detect_syslog_short_micro() {
        assert_eq!(
            detect_format("May 29 18:48:24.123456 host proc: msg"),
            Some(LogFormat::SyslogShortMicro)
        );
    }

    #[test]
    fn detect_iso_space() {
        assert_eq!(
            detect_format("2026-05-29 18:48:24 host proc: msg"),
            Some(LogFormat::Iso8601Space)
        );
    }

    #[test]
    fn detect_iso_t() {
        assert_eq!(
            detect_format("2026-05-29T18:48:24Z host proc: msg"),
            Some(LogFormat::Iso8601T)
        );
    }

    #[test]
    fn detect_none() {
        assert_eq!(detect_format("not a timestamp"), None);
    }

    #[test]
    fn detect_skips_blank_lines() {
        let content = "\n\nMay 29 18:48:24 host proc: msg\n";
        let fmt = {
            let mut found = None;
            let mut scanned = 0;
            for line in content.lines() {
                let trimmed = line.trim();
                if trimmed.is_empty() {
                    continue;
                }
                if let Some(f) = detect_format(trimmed) {
                    found = Some(f);
                    break;
                }
                scanned += 1;
                if scanned >= 10 {
                    break;
                }
            }
            found
        };
        assert_eq!(fmt, Some(LogFormat::SyslogShort));
    }

    #[test]
    fn year_infer_dec_loaded_in_jan() {
        let naive = chrono::NaiveDateTime::new(
            chrono::NaiveDate::from_ymd_opt(2000, 12, 31).unwrap(),
            chrono::NaiveTime::from_hms_opt(23, 59, 59).unwrap(),
        );
        let now = utc(2026, 1, 1, 0, 0, 0);
        let result = infer_year(naive, now);
        assert_eq!(result.year(), 2025);
    }

    #[test]
    fn year_infer_jan_loaded_in_jan() {
        let naive = chrono::NaiveDateTime::new(
            chrono::NaiveDate::from_ymd_opt(2000, 1, 1).unwrap(),
            chrono::NaiveTime::from_hms_opt(0, 1, 0).unwrap(),
        );
        let now = utc(2026, 1, 15, 12, 0, 0);
        let result = infer_year(naive, now);
        assert_eq!(result.year(), 2026);
    }

    #[test]
    fn year_infer_future_by_2h_decrements() {
        let now = utc(2026, 5, 23, 10, 0, 0);
        let naive = chrono::NaiveDateTime::new(
            chrono::NaiveDate::from_ymd_opt(2000, 5, 23).unwrap(),
            chrono::NaiveTime::from_hms_opt(12, 0, 0).unwrap(), // 2 h ahead
        );
        let result = infer_year(naive, now);
        assert_eq!(result.year(), 2025);
    }

    #[test]
    fn year_infer_future_by_30min_keeps_current() {
        let now = utc(2026, 5, 23, 10, 0, 0);
        let naive = chrono::NaiveDateTime::new(
            chrono::NaiveDate::from_ymd_opt(2000, 5, 23).unwrap(),
            chrono::NaiveTime::from_hms_opt(10, 30, 0).unwrap(), // 30 min ahead
        );
        let result = infer_year(naive, now);
        assert_eq!(result.year(), 2026);
    }

    #[test]
    fn color_group_single() {
        let mut a = ColorGroupAssigner::new();
        assert_eq!(a.assign("hello"), 0);
    }

    #[test]
    fn color_group_identical() {
        let mut a = ColorGroupAssigner::new();
        assert_eq!(a.assign("hello"), 0);
        assert_eq!(a.assign("hello"), 0);
        assert_eq!(a.representatives.len(), 1);
    }

    #[test]
    fn color_group_distance_exactly_5() {
        let mut a = ColorGroupAssigner::new();
        let s1 = "abcde";
        let s2 = "fghij"; // distance 5 from s1
        assert_eq!(levenshtein(s1, s2), 5);
        assert_eq!(a.assign(s1), 0);
        assert_eq!(a.assign(s2), 0);
    }

    #[test]
    fn color_group_distance_6() {
        let mut a = ColorGroupAssigner::new();
        let s1 = "abcdef";
        let s2 = "ghijkl"; // distance 6
        assert_eq!(levenshtein(s1, s2), 6);
        assert_eq!(a.assign(s1), 0);
        assert_eq!(a.assign(s2), 1);
    }

    #[test]
    fn color_group_10_distinct() {
        let mut a = ColorGroupAssigner::new();
        // Each string is 20 repetitions of a unique letter - Levenshtein distance 20 apart
        let strings: Vec<String> = (0u8..10u8)
            .map(|i| std::iter::repeat_n(char::from(b'a' + i), 20).collect())
            .collect();
        for (i, s) in strings.iter().enumerate() {
            assert_eq!(a.assign(s), i as u32);
        }
    }

    #[test]
    fn color_group_gpsd_same_group() {
        let mut a = ColorGroupAssigner::new();
        let s1 = "gpsd[1234]: offset 0.003";
        let s2 = "gpsd[1235]: offset 0.004";
        let d = levenshtein(s1, s2);
        assert!(d <= 5, "expected distance ≤ 5, got {d}");
        assert_eq!(a.assign(s1), 0);
        assert_eq!(a.assign(s2), 0);
    }

    #[test]
    fn associate_interpolates_midpoint() {
        let t0 = utc(2026, 1, 1, 0, 0, 0);
        let pts = nav_points_from(t0, 5, 1);
        let ts = t0 + chrono::Duration::milliseconds(500);
        let (lat, lon) = associate(&ts, &pts, 60).expect("should associate");
        let expected_lat = 55.0 + 0.0005;
        let expected_lon = 12.0 + 0.0005;
        assert!((lat - expected_lat).abs() < 1e-9);
        assert!((lon - expected_lon).abs() < 1e-9);
    }

    #[test]
    fn associate_exact_fix_time() {
        let t0 = utc(2026, 1, 1, 0, 0, 0);
        let pts = nav_points_from(t0, 5, 1);
        let (lat, lon) = associate(&t0, &pts, 60).expect("should associate");
        assert!((lat - 55.0).abs() < 1e-9);
        assert!((lon - 12.0).abs() < 1e-9);
    }

    #[test]
    fn associate_within_60s_after_last() {
        let t0 = utc(2026, 1, 1, 0, 0, 0);
        let pts = nav_points_from(t0, 5, 1); // last fix at t0+4s
        let ts = t0 + chrono::Duration::seconds(4 + 59);
        assert!(associate(&ts, &pts, 60).is_some());
    }

    #[test]
    fn associate_beyond_60s_after_last() {
        let t0 = utc(2026, 1, 1, 0, 0, 0);
        let pts = nav_points_from(t0, 5, 1); // last fix at t0+4s
        let ts = t0 + chrono::Duration::seconds(4 + 61);
        assert!(associate(&ts, &pts, 60).is_none());
    }

    #[test]
    fn associate_beyond_60s_before_first() {
        let t0 = utc(2026, 1, 1, 0, 0, 0);
        let pts = nav_points_from(t0, 5, 1);
        let ts = t0 - chrono::Duration::seconds(61);
        assert!(associate(&ts, &pts, 60).is_none());
    }

    #[test]
    fn associate_within_60s_before_first() {
        let t0 = utc(2026, 1, 1, 0, 0, 0);
        let pts = nav_points_from(t0, 5, 1);
        let ts = t0 - chrono::Duration::seconds(30);
        let (lat, lon) = associate(&ts, &pts, 60).expect("should associate");
        assert!((lat - 55.0).abs() < 1e-9);
        assert!((lon - 12.0).abs() < 1e-9);
    }

    #[test]
    fn associate_empty_nav() {
        let ts = utc(2026, 1, 1, 0, 0, 0);
        assert!(associate(&ts, &[], 60).is_none());
    }

    fn make_log_content(entries: &[(DateTime<Utc>, &str)]) -> String {
        entries
            .iter()
            .map(|(ts, msg)| format!("{} {msg}", ts.format("%Y-%m-%d %H:%M:%S")))
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn load_log_mix_associated_unassociated() {
        let t0 = utc(2026, 1, 1, 0, 0, 0);
        let pts = nav_points_from(t0, 5, 1);
        let entries = vec![
            (t0, "msg alpha"),
            (t0 + chrono::Duration::seconds(1), "msg beta"),
            (t0 + chrono::Duration::seconds(2), "msg gamma"),
            (t0 + chrono::Duration::seconds(200), "far future"), // unassociated
            (t0 - chrono::Duration::seconds(200), "far past"),   // unassociated
        ];
        let content = make_log_content(&entries);
        let result = load_log(
            &content,
            &pts,
            utc(2026, 5, 23, 0, 0, 0),
            &AssociationConfig::default(),
        );
        assert_eq!(result.markers.len(), 3);
        assert_eq!(result.unassociated.len(), 2);
    }

    #[test]
    fn load_log_empty_content() {
        let result = load_log(
            "",
            &[],
            utc(2026, 5, 23, 0, 0, 0),
            &AssociationConfig::default(),
        );
        assert!(result.markers.is_empty());
        assert!(result.unassociated.is_empty());
    }

    #[test]
    fn load_log_empty_nav() {
        let t0 = utc(2026, 1, 1, 0, 0, 0);
        let content = make_log_content(&[(t0, "msg")]);
        let result = load_log(
            &content,
            &[],
            utc(2026, 5, 23, 0, 0, 0),
            &AssociationConfig::default(),
        );
        assert!(result.markers.is_empty());
        assert_eq!(result.unassociated.len(), 1);
    }

    #[test]
    fn load_log_two_bad_lines_then_good_continues() {
        let t0 = utc(2026, 1, 1, 0, 0, 0);
        let pts = nav_points_from(t0, 5, 1);
        let content =
            "2026-01-01 00:00:00 msg0\nNOT_A_TIMESTAMP\nALSO_NOT\n2026-01-01 00:00:01 msg1\n";
        let result = load_log(
            content,
            &pts,
            utc(2026, 5, 23, 0, 0, 0),
            &AssociationConfig::default(),
        );
        assert_eq!(result.markers.len(), 2);
    }

    #[test]
    fn load_log_three_consecutive_bad_aborts() {
        let t0 = utc(2026, 1, 1, 0, 0, 0);
        let pts = nav_points_from(t0, 5, 1);
        let content = "2026-01-01 00:00:00 msg0\nBAD1\nBAD2\nBAD3\n2026-01-01 00:00:01 msg1\n";
        let result = load_log(
            content,
            &pts,
            utc(2026, 5, 23, 0, 0, 0),
            &AssociationConfig::default(),
        );
        assert!(result.markers.is_empty());
        assert!(result.unassociated.is_empty());
    }

    #[test]
    fn load_log_blank_lines_dont_count_toward_failures() {
        let t0 = utc(2026, 1, 1, 0, 0, 0);
        let pts = nav_points_from(t0, 5, 1);
        // bad, blank, bad, blank, bad - blanks don't count, so only 3 non-empty failures
        let content =
            "2026-01-01 00:00:00 good\nBAD\n\nBAD\n\nBAD\n2026-01-01 00:00:01 also_good\n";
        let result = load_log(
            content,
            &pts,
            utc(2026, 5, 23, 0, 0, 0),
            &AssociationConfig::default(),
        );
        // 3 consecutive bad → abort
        assert!(result.markers.is_empty());
        assert!(result.unassociated.is_empty());
    }

    #[test]
    fn load_log_markers_have_log_icon() {
        let t0 = utc(2026, 1, 1, 0, 0, 0);
        let pts = nav_points_from(t0, 5, 1);
        let content = make_log_content(&[(t0, "msg")]);
        let result = load_log(
            &content,
            &pts,
            utc(2026, 5, 23, 0, 0, 0),
            &AssociationConfig::default(),
        );
        assert!(result.markers.iter().all(|m| m.icon == MarkerIcon::Log));
    }

    #[test]
    fn load_log_color_groups_match_assigner() {
        let t0 = utc(2026, 1, 1, 0, 0, 0);
        let pts = nav_points_from(t0, 10, 1);
        let very_different = "zzzzzzzzzzzzzzzzzzzzzzzzzzzzzz";
        let entries = vec![
            (t0, "gpsd[1234]: offset 0.003"),
            (
                t0 + chrono::Duration::seconds(1),
                "gpsd[1235]: offset 0.004",
            ),
            (t0 + chrono::Duration::seconds(2), very_different),
        ];
        let content = make_log_content(&entries);
        let result = load_log(
            &content,
            &pts,
            utc(2026, 5, 23, 0, 0, 0),
            &AssociationConfig::default(),
        );
        let groups: Vec<_> = result.markers.iter().map(|m| m.color_group).collect();
        assert_matches_sequence!(groups, [Some(0), Some(0), Some(1)]);
    }

    #[test]
    fn load_log_microsecond_precision() {
        let t0 = utc(2026, 1, 1, 0, 0, 0);
        let pts = nav_points_from(t0, 5, 1);
        // SyslogShortMicro line
        let content = "Jan  1 00:00:00.500000 msg_micro\n";
        let result = load_log(
            content,
            &pts,
            utc(2026, 1, 1, 12, 0, 0),
            &AssociationConfig::default(),
        );
        assert_eq!(result.markers.len(), 1);
        let ts = result.markers[0].time;
        assert_eq!(ts.timestamp_subsec_micros(), 500_000);
    }
}
