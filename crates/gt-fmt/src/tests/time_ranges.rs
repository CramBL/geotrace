use chrono::{DateTime, Utc};

fn dt(s: &str) -> DateTime<Utc> {
    DateTime::parse_from_rfc3339(s)
        .expect("valid rfc3339")
        .with_timezone(&Utc)
}

#[test]
fn a_time_range_within_one_day_omits_the_end_date() {
    assert_eq!(
        crate::format_time_range(dt("2024-01-05T12:00:00Z"), dt("2024-01-05T12:30:45Z")),
        "2024-01-05 12:00:00 – 12:30:45"
    );
}

#[test]
fn a_time_range_across_midnight_states_the_end_date() {
    assert_eq!(
        crate::format_time_range(dt("2024-01-05T23:50:00Z"), dt("2024-01-06T00:10:00Z")),
        "2024-01-05 23:50:00 – 2024-01-06 00:10:00"
    );
}
