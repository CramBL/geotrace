/// Format a distance in km as a compact human-readable string.
///
/// Uses metres for distances under 1 km, kilometres otherwise.
pub fn format_distance(km: f64) -> String {
    if km < 1.0 {
        format!("{:.0} m", km * 1_000.0)
    } else {
        format!("{km:.1} km")
    }
}

/// Format a duration as a compact human-readable string.
///
/// Rules:
/// - Seconds are shown only when the total duration is under 2 minutes.
/// - Minutes are shown only when the total whole hours is less than 3.
/// - Zero-valued components are omitted entirely.
/// - Zero duration returns `"0s"` to avoid an empty string.
///
/// Examples: `"20m"`, `"1h28m"`, `"3h"`, `"1m30s"`, `"45s"`.
pub fn format_human_terse_duration(d: chrono::Duration) -> String {
    let total_secs = d.num_seconds();
    let h = d.num_hours();
    let m = d.num_minutes() % 60;
    let s = total_secs % 60;

    let show_s = total_secs < 120;
    let show_m = h < 3;

    if h == 0 && m == 0 && s == 0 {
        return "0s".to_owned();
    }

    let mut out = String::new();
    if h > 0 {
        out.push_str(&h.to_string());
        out.push('h');
    }
    if show_m && m > 0 {
        out.push_str(&m.to_string());
        out.push('m');
    }
    if show_s && s > 0 {
        out.push_str(&s.to_string());
        out.push('s');
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
}
