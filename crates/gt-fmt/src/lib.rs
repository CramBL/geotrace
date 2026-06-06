use std::fmt::Write;

use uom::si::{
    f64,
    length::{kilometer, meter},
};

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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    fn dur(h: i64, m: i64, s: i64) -> Duration {
        Duration::seconds(h * 3600 + m * 60 + s)
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
}
