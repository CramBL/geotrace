use std::path::PathBuf;

use chrono::Duration;
use gt_types::nav_point::NavPoint;
use gt_types::track::FileSource;
use rstest::rstest;

use crate::segment::{self, FileMeta, SegmentationConfig};
use crate::test_util;

#[rstest]
#[case::no_fixes(vec![])]
#[case::no_satellite_reports(vec![test_util::fix_without_a_satellite_report(0), test_util::fix_without_a_satellite_report(60)])]
#[case::one_fix_with_a_report(vec![test_util::fix_with_a_satellite_in_fix(0)])]
fn compute_fix_stats_needs_two_fixes_with_a_satellite_report(#[case] pts: Vec<NavPoint>) {
    assert!(segment::compute_fix_stats(&pts).is_none());
}

#[test]
fn compute_fix_stats_all_with_fix() {
    // Two consecutive sat points, both in fix → all `time_with_fix`, no losses
    let pts = vec![
        test_util::fix_with_a_satellite_in_fix(0),
        test_util::fix_with_a_satellite_in_fix(60),
    ];
    let stats = segment::compute_fix_stats(&pts).expect("has satellite data");
    assert_eq!(stats.time_with_fix, Duration::seconds(60));
    assert_eq!(stats.time_without_fix, Duration::zero());
    assert_eq!(stats.fix_loss_count, 0);
    assert_eq!(stats.max_continuous_no_fix, Duration::zero());
}

#[test]
fn compute_fix_stats_all_without_fix() {
    let pts = vec![
        test_util::fix_with_a_satellite_in_view_only(0),
        test_util::fix_with_a_satellite_in_view_only(120),
    ];
    let stats = segment::compute_fix_stats(&pts).expect("has satellite data");
    assert_eq!(stats.time_with_fix, Duration::zero());
    assert_eq!(stats.time_without_fix, Duration::seconds(120));
    assert_eq!(stats.fix_loss_count, 0);
    assert_eq!(stats.max_continuous_no_fix, Duration::seconds(120));
}

#[test]
fn compute_fix_stats_fix_then_lost() {
    // fix 0→60, lost 60→180 → one loss, 120s without fix
    let pts = vec![
        test_util::fix_with_a_satellite_in_fix(0),
        test_util::fix_with_a_satellite_in_view_only(60),
        test_util::fix_with_a_satellite_in_view_only(180),
    ];
    let stats = segment::compute_fix_stats(&pts).expect("has satellite data");
    assert_eq!(stats.time_with_fix, Duration::seconds(60));
    assert_eq!(stats.time_without_fix, Duration::seconds(120));
    assert_eq!(stats.fix_loss_count, 1);
    assert_eq!(stats.max_continuous_no_fix, Duration::seconds(120));
}

#[test]
fn compute_fix_stats_multiple_losses() {
    // fix→lost→fix→lost pattern. Two separate no-fix stretches
    let pts = vec![
        test_util::fix_with_a_satellite_in_fix(0),         // fix
        test_util::fix_with_a_satellite_in_view_only(100), // lost (100s with fix)
        test_util::fix_with_a_satellite_in_view_only(200), // still lost (100s without fix, streak=100)
        test_util::fix_with_a_satellite_in_fix(300), // regained (100s more without fix, streak=200)
        test_util::fix_with_a_satellite_in_view_only(400), // lost again (100s with fix)
        test_util::fix_with_a_satellite_in_view_only(450), // still lost (50s without fix, streak=50)
    ];
    let stats = segment::compute_fix_stats(&pts).expect("has satellite data");
    assert_eq!(stats.time_with_fix, Duration::seconds(200));
    assert_eq!(stats.time_without_fix, Duration::seconds(250));
    assert_eq!(stats.fix_loss_count, 2);
    assert_eq!(stats.max_continuous_no_fix, Duration::seconds(200));
}

#[test]
fn compute_fix_stats_ignores_points_without_sat_data() {
    // Gaps between sat-report points (no satellite data) are not counted in either bucket
    let pts = vec![
        test_util::fix_with_a_satellite_in_fix(0),
        test_util::fix_without_a_satellite_report(30), // no satellite data - ignored
        test_util::fix_without_a_satellite_report(60), // no satellite data - ignored
        test_util::fix_with_a_satellite_in_fix(90),
    ];
    let stats = segment::compute_fix_stats(&pts).expect("has satellite data");
    // Interval 0→90 attributed to first sat point (has fix) = 90s with fix
    assert_eq!(stats.time_with_fix, Duration::seconds(90));
    assert_eq!(stats.time_without_fix, Duration::zero());
    assert_eq!(stats.fix_loss_count, 0);
}

#[test]
fn build_loaded_file_file_fix_stats_aggregates_tracks() {
    // Default split gap is 300 s, so consecutive points must be < 300 s apart to stay in
    // the same track. Track 1: t=0 (fix)→t=60 (no-fix)→t=180 (no-fix). One loss, max=120s.
    // Track 2 (after a 9820s gap): t=10000 (fix)→t=10060 (no-fix)→t=10120 (fix). One loss,
    // max=60s.
    // sum(120, 60) = 180s != max(120, 60) = 120s, so the assertion below distinguishes
    // "max across tracks" from "sum across tracks".
    let pts = vec![
        test_util::fix_with_a_satellite_in_fix(0),
        test_util::fix_with_a_satellite_in_view_only(60),
        test_util::fix_with_a_satellite_in_view_only(180), // end of track 1
        test_util::fix_with_a_satellite_in_fix(10_000),    // large gap → new track 2
        test_util::fix_with_a_satellite_in_view_only(10_060),
        test_util::fix_with_a_satellite_in_fix(10_120),
    ];
    let f = segment::build_loaded_file(
        "test.gtd".to_owned(),
        &pts,
        &[],
        vec![],
        vec![],
        &[],
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("test.gtd")),
        FileMeta::default(),
        vec![],
    );
    assert_eq!(f.tracks.len(), 2, "expected two tracks");
    let stats = f.metadata.fix_stats.expect("fix stats should be present");
    assert_eq!(stats.time_with_fix, Duration::seconds(60 + 60));
    assert_eq!(stats.time_without_fix, Duration::seconds(120 + 60));
    assert_eq!(stats.fix_loss_count, 2);
    // max taken across tracks, not summed
    assert_eq!(stats.max_continuous_no_fix, Duration::seconds(120));
}

proptest::proptest! {
    /// Invariant: `time_with_fix + time_without_fix` equals the sum of all
    /// intervals between consecutive satellite-report points, regardless of
    /// fix pattern or gap sizes.
    #[test]
    fn fix_stats_durations_sum_to_total_interval(
        deltas_and_fixes in proptest::collection::vec(
            (1i64..300i64, proptest::bool::ANY),
            2..20usize,
        )
    ) {
        let mut t: i64 = 0;
        let points: Vec<NavPoint> = deltas_and_fixes
            .iter()
            .map(|(dt, has_fix)| {
                t += dt;
                if *has_fix {
                    test_util::fix_with_a_satellite_in_fix(t)
                } else {
                    test_util::fix_with_a_satellite_in_view_only(t)
                }
            })
            .collect();

        // All points have satellite data, so fix stats must be Some.
        let stats = segment::compute_fix_stats(&points).expect("all points have satellite data");

        // Compute expected total: sum of intervals between consecutive sat-report points.
        let expected_total = points
            .windows(2)
            .map(|pair| {
                if let [a, b] = pair {
                    b.tpv.time() - a.tpv.time()
                } else {
                    Duration::zero()
                }
            })
            .fold(Duration::zero(), |acc, d| acc + d);

        proptest::prop_assert_eq!(
            stats.time_with_fix + stats.time_without_fix,
            expected_total,
        );
    }
}
