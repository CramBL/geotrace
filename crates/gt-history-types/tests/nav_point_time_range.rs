//! The time range the history database indexes a stored recording by.

use gt_history_types::{DatabaseRef, NavPointTimeRange, PruneMode, RecordingEntry, RecordingMeta};
use rstest::rstest;

#[rstest]
#[case::in_time_order(&[1_000, 2_000, 3_000], 1_000, 3_000)]
#[case::backward_step(&[3_000, 4_000, 1_000, 2_000], 1_000, 4_000)]
#[case::one_nav_point(&[7_000], 7_000, 7_000)]
fn a_time_range_covers_the_earliest_and_the_latest_nav_point_time(
    #[case] nav_point_times: &[i64],
    #[case] expected_start_us: i64,
    #[case] expected_end_us: i64,
) {
    let range = NavPointTimeRange::covering(nav_point_times).expect("the recording has nav points");

    assert_eq!(range.start_us(), expected_start_us);
    assert_eq!(range.end_us(), expected_end_us);
}

#[test]
fn a_recording_with_no_nav_point_has_no_time_range() {
    assert_eq!(NavPointTimeRange::covering(&[]), None);
}

#[rstest]
#[case::in_time_order(10, 1_000, 5_000)]
#[case::one_nav_point(1, 7_000, 7_000)]
fn stored_attributes_give_the_time_range_they_bound(
    #[case] nav_point_count: u64,
    #[case] start_us: i64,
    #[case] end_us: i64,
) {
    let range = NavPointTimeRange::from_stored_attributes(nav_point_count, start_us..=end_us)
        .expect("the attributes bound a time range");

    assert_eq!(range.start_us(), start_us);
    assert_eq!(range.end_us(), end_us);
}

#[rstest]
#[case::inverted_bounds(10, 5_000, 1_000)]
#[case::no_nav_point(0, 0, 0)]
fn stored_attributes_give_no_time_range_when_inverted_or_the_recording_has_no_nav_point(
    #[case] nav_point_count: u64,
    #[case] start_us: i64,
    #[case] end_us: i64,
) {
    assert_eq!(
        NavPointTimeRange::from_stored_attributes(nav_point_count, start_us..=end_us),
        None
    );
}

fn entry(identity: &str, time_range: Option<NavPointTimeRange>) -> RecordingEntry {
    RecordingEntry {
        db_ref: DatabaseRef {
            identity: identity.to_owned(),
            group_name: format!("{identity}_group"),
        },
        meta: RecordingMeta {
            time_range,
            nav_point_count: 0,
            sat_report_count: 0,
            marker_count: 0,
            event_marker_count: 0,
            gtd_size_bytes: 0,
        },
        total_tracks: 0,
        hidden_tracks: 0,
        title: None,
        device: None,
        notes: None,
        travel_mode: None,
        channels: Vec::new(),
        log_attachments: Vec::new(),
    }
}

#[test]
fn pruning_by_age_keeps_a_recording_with_no_time_range() {
    let entries = [
        entry("epoch", NavPointTimeRange::covering(&[0, 1_000])),
        entry("no_time_range", None),
    ];

    let selected = PruneMode::ByAge { max_age_secs: 60 }.select(&entries);

    let identities: Vec<&str> = selected
        .iter()
        .map(|db_ref| db_ref.identity.as_str())
        .collect();
    assert_eq!(identities, ["epoch"]);
}
