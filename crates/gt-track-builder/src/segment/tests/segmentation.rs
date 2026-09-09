use std::ops::Range;

use rstest::rstest;

use crate::segment::{self, TrackLayoutConfig, TrackSplitRule};
use crate::test_util;

#[test]
fn segment_tracks_empty_input() {
    assert!(segment::segment_tracks(&[], &TrackLayoutConfig::default()).is_empty());
}

#[test]
fn segment_tracks_single_point() {
    let pts = vec![test_util::fix_without_a_satellite_report(0)];
    let ranges = segment::segment_tracks(&pts, &TrackLayoutConfig::default());
    assert_eq!(ranges, vec![0..1]);
}

#[rstest]
#[case::forward_step_below_the_split_gap(TrackSplitRule::StepInEitherDirection, 299, vec![0..2])]
#[case::forward_step_at_the_split_gap(TrackSplitRule::StepInEitherDirection, 300, vec![0..1, 1..2])]
#[case::backward_step_below_the_split_gap(TrackSplitRule::StepInEitherDirection, -299, vec![0..2])]
#[case::backward_step_at_the_split_gap(TrackSplitRule::StepInEitherDirection, -300, vec![0..1, 1..2])]
#[case::forward_step_at_the_split_gap_under_forward_gaps_only(TrackSplitRule::ForwardGapOnly, 300, vec![0..1, 1..2])]
#[case::backward_step_at_the_split_gap_under_forward_gaps_only(TrackSplitRule::ForwardGapOnly, -300, vec![0..2])]
fn segment_tracks_applies_the_configured_split_rule(
    #[case] track_split_rule: TrackSplitRule,
    #[case] step_seconds: i64,
    #[case] expected_ranges: Vec<Range<usize>>,
) {
    let pts = vec![
        test_util::fix_without_a_satellite_report(1_000),
        test_util::fix_without_a_satellite_report(1_000 + step_seconds),
    ];
    let config = TrackLayoutConfig {
        track_split_rule,
        ..TrackLayoutConfig::default()
    };

    let ranges = segment::segment_tracks(&pts, &config);

    assert_eq!(ranges, expected_ranges);
}

#[test]
fn segment_tracks_multiple_gaps() {
    let pts = vec![
        test_util::fix_without_a_satellite_report(0),
        test_util::fix_without_a_satellite_report(3600), // first gap
        test_util::fix_without_a_satellite_report(7200), // second gap
    ];
    let ranges = segment::segment_tracks(&pts, &TrackLayoutConfig::default());
    assert_eq!(ranges, vec![0..1, 1..2, 2..3]);
}
