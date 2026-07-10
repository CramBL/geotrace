use std::fmt::Write;

use chrono::{DateTime, Utc};
use gt_types::track::FixStats;

pub mod name_template;
pub use name_template::{NameFields, render_name_template};
use uom::si::{
    f64,
    length::{kilometer, meter},
};

/// Two spaces, U+00B7 MIDDLE DOT, two spaces, joins fields inside tooltip strings.
const TOOLTIP_JOINER: &str = "  ·  ";

/// Returns the percentage of time spent with a satellite fix, rounded to the
/// nearest integer in `[0, 100]`.
///
/// Returns `0` when `time_with_fix + time_without_fix` is zero or negative.
pub fn fix_percentage(stats: FixStats) -> u32 {
    let total_ms = (stats.time_with_fix + stats.time_without_fix).num_milliseconds();
    if total_ms <= 0 {
        return 0;
    }
    let fix_ms = stats.time_with_fix.num_milliseconds().max(0);
    u32::try_from((fix_ms * 100 + total_ms / 2) / total_ms).unwrap_or(100)
}

/// Formats just the fix-percentage portion of the tooltip, e.g. `"85% fix"`.
pub fn format_fix_percentage(stats: FixStats) -> String {
    format!("{}% fix", fix_percentage(stats))
}

/// Formats the remaining tooltip details (time without fix, loss count, max
/// continuous gap), each prefixed by `TOOLTIP_JOINER`.
///
/// Returns an empty string when there is nothing to add beyond the
/// percentage, e.g. for `"100% fix"` with no losses.
pub fn format_fix_tooltip_details(stats: FixStats) -> String {
    let mut parts: Vec<String> = Vec::new();
    if stats.time_without_fix > chrono::Duration::zero() {
        parts.push(format!(
            "{} w/o fix",
            format_human_terse_duration(stats.time_without_fix)
        ));
    }
    if stats.fix_loss_count == 1 {
        parts.push("1 loss".to_owned());
    } else if stats.fix_loss_count > 1 {
        parts.push(format!("{} losses", stats.fix_loss_count));
    }
    if stats.max_continuous_no_fix > chrono::Duration::zero() {
        parts.push(format!(
            "max gap {}",
            format_human_terse_duration(stats.max_continuous_no_fix)
        ));
    }
    if parts.is_empty() {
        String::new()
    } else {
        format!("{TOOLTIP_JOINER}{}", parts.join(TOOLTIP_JOINER))
    }
}

/// Format a distance as a compact human-readable string.
///
/// Uses metres for distances under 1 km, kilometres otherwise.
pub fn format_distance(d: f64::Length) -> String {
    let km = d.get::<kilometer>();
    if km < 1.0 {
        format!("{:.0} m", d.get::<meter>())
    } else {
        format!("{km:.1} km")
    }
}

/// Format a duration as a compact human-readable string.
///
/// Rules:
/// - Durations ≥ 48 h are shown as `"Xd"` or `"XdYh"` (e.g. `"2d5h"`).
/// - Seconds are shown only when the total duration is under 2 minutes.
/// - Minutes are shown only when the whole hours is less than 3.
/// - Zero-valued components are omitted entirely.
/// - Zero duration returns `"0s"` to avoid an empty string.
///
/// Examples: `"20m"`, `"1h28m"`, `"3h"`, `"1m30s"`, `"45s"`, `"2d5h"`.
#[expect(
    clippy::let_underscore_must_use,
    reason = "writing to String cannot fail"
)]
pub fn format_human_terse_duration(d: chrono::Duration) -> String {
    let total_secs = d.num_seconds();

    if total_secs == 0 {
        return "0s".to_owned();
    }

    let h = d.num_hours();
    let m = d.num_minutes() % 60;
    let s = total_secs % 60;

    if h >= 48 {
        let days = h / 24;
        let rem_h = h % 24;
        return if rem_h > 0 {
            format!("{days}d{rem_h}h")
        } else {
            format!("{days}d")
        };
    }

    let show_s = total_secs < 120;
    let show_m = h < 3;

    // Pre-allocate enough capacity to avoid reallocations.
    let mut out = String::with_capacity(16);

    if h > 0 {
        let _ = write!(out, "{h}h");
    }
    if show_m && m > 0 {
        let _ = write!(out, "{m}m");
    }
    if show_s && s > 0 {
        let _ = write!(out, "{s}s");
    }

    out
}

/// Formats a time span for a tooltip header: the start date and time, then the
/// end time - including the end date as well when the span crosses midnight
/// into a different day. Times are UTC, matching the rest of the UI.
pub fn format_time_range(start: DateTime<Utc>, end: DateTime<Utc>) -> String {
    let start_str = start.format("%Y-%m-%d %H:%M:%S");
    if start.date_naive() == end.date_naive() {
        format!("{start_str} – {}", end.format("%H:%M:%S"))
    } else {
        format!("{start_str} – {}", end.format("%Y-%m-%d %H:%M:%S"))
    }
}

/// Picks the singular or plural word for a count: `singular` when `count == 1`,
/// otherwise `plural`. Keeps count-dependent labels consistent across the UI.
pub fn pluralize<'a>(count: usize, singular: &'a str, plural: &'a str) -> &'a str {
    if count == 1 { singular } else { plural }
}

/// Format a count with comma thousands separators (`8,940`).
pub fn format_count(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn format_count_adds_thousands_separators() {
        for (n, expected) in [
            (0, "0"),
            (12, "12"),
            (999, "999"),
            (1000, "1,000"),
            (8940, "8,940"),
            (1_000_000, "1,000,000"),
        ] {
            assert_eq!(format_count(n), expected);
        }
    }

    #[test]
    fn pluralize_picks_singular_only_for_one() {
        assert_eq!(pluralize(1, "recording", "recordings"), "recording");
        assert_eq!(pluralize(0, "recording", "recordings"), "recordings");
        assert_eq!(pluralize(2, "recording", "recordings"), "recordings");
    }

    fn dur(h: i64, m: i64, s: i64) -> Duration {
        Duration::seconds(h * 3600 + m * 60 + s)
    }

    fn fix_stats(
        with_fix_s: i64,
        without_fix_s: i64,
        fix_loss_count: u32,
        max_no_fix_s: i64,
    ) -> FixStats {
        FixStats {
            time_with_fix: Duration::seconds(with_fix_s),
            time_without_fix: Duration::seconds(without_fix_s),
            fix_loss_count,
            max_continuous_no_fix: Duration::seconds(max_no_fix_s),
        }
    }

    #[test]
    fn fix_percentage_rounds_to_nearest() {
        assert_eq!(fix_percentage(fix_stats(85, 15, 0, 15)), 85);
        assert_eq!(fix_percentage(fix_stats(4800, 0, 0, 0)), 100);
        assert_eq!(fix_percentage(fix_stats(0, 0, 0, 0)), 0);
    }

    #[test]
    fn format_fix_percentage_appends_suffix() {
        assert_eq!(format_fix_percentage(fix_stats(85, 15, 0, 15)), "85% fix");
    }

    #[test]
    fn format_fix_tooltip_details_empty_for_full_fix() {
        assert_eq!(format_fix_tooltip_details(fix_stats(4800, 0, 0, 0)), "");
    }

    #[test]
    fn format_fix_tooltip_details_includes_all_parts() {
        let d = format_fix_tooltip_details(fix_stats(4800, 900, 3, 480));
        assert_eq!(d, "  ·  15m w/o fix  ·  3 losses  ·  max gap 8m");
    }

    #[test]
    fn percentage_and_details_combine_into_expected_tooltip() {
        let stats = fix_stats(4800, 900, 3, 480);
        let combined = format!(
            "{}{}",
            format_fix_percentage(stats),
            format_fix_tooltip_details(stats)
        );
        assert_eq!(
            combined,
            "84% fix  ·  15m w/o fix  ·  3 losses  ·  max gap 8m"
        );
    }

    #[test]
    fn zero_duration() {
        assert_eq!(format_human_terse_duration(dur(0, 0, 0)), "0s");
    }

    #[test]
    fn seconds_only_under_two_minutes() {
        assert_eq!(format_human_terse_duration(dur(0, 0, 45)), "45s");
    }

    #[test]
    fn minutes_and_seconds_under_two_minutes() {
        assert_eq!(format_human_terse_duration(dur(0, 1, 30)), "1m30s");
    }

    #[test]
    fn minutes_only_no_seconds_above_threshold() {
        assert_eq!(format_human_terse_duration(dur(0, 20, 0)), "20m");
    }

    #[test]
    fn hours_and_minutes_seconds_omitted() {
        assert_eq!(format_human_terse_duration(dur(1, 28, 15)), "1h28m");
    }

    #[test]
    fn hours_only_zero_minutes() {
        assert_eq!(format_human_terse_duration(dur(2, 0, 0)), "2h");
    }

    #[test]
    fn three_or_more_hours_minutes_omitted() {
        assert_eq!(format_human_terse_duration(dur(3, 45, 10)), "3h");
    }

    #[test]
    fn exactly_two_minutes_no_seconds() {
        // 120 s is not < 120, so seconds are omitted
        assert_eq!(format_human_terse_duration(dur(0, 2, 0)), "2m");
    }

    #[test]
    fn just_under_two_minutes_shows_seconds() {
        assert_eq!(format_human_terse_duration(dur(0, 1, 59)), "1m59s");
    }

    #[test]
    fn forty_eight_hours_shows_days_no_hours() {
        assert_eq!(format_human_terse_duration(dur(48, 0, 0)), "2d");
    }

    #[test]
    fn above_forty_eight_hours_shows_days_and_hours() {
        assert_eq!(format_human_terse_duration(dur(53, 0, 0)), "2d5h");
        assert_eq!(format_human_terse_duration(dur(119, 0, 0)), "4d23h");
    }

    #[test]
    fn below_forty_eight_hours_shows_plain_hours() {
        assert_eq!(format_human_terse_duration(dur(47, 0, 0)), "47h");
    }

    fn dt(s: &str) -> DateTime<Utc> {
        DateTime::parse_from_rfc3339(s)
            .expect("valid rfc3339")
            .with_timezone(&Utc)
    }

    #[test]
    fn time_range_same_day_omits_end_date() {
        assert_eq!(
            format_time_range(dt("2024-01-05T12:00:00Z"), dt("2024-01-05T12:30:45Z")),
            "2024-01-05 12:00:00 – 12:30:45"
        );
    }

    #[test]
    fn time_range_across_midnight_includes_end_date() {
        assert_eq!(
            format_time_range(dt("2024-01-05T23:50:00Z"), dt("2024-01-06T00:10:00Z")),
            "2024-01-05 23:50:00 – 2024-01-06 00:10:00"
        );
    }
}
