//! Validate downsampling, chunking, and gps_accuracy derivation.

mod support;

use rstest::rstest;
use support::points_with as points;

use gt_snap::request_plan::{
    self, CHUNK_OVERLAP_POINTS, CHUNK_POINTS, GPS_ACCURACY_OVERRIDE_RANGE_M, GPS_ACCURACY_RANGE_M,
    RequestPlan, SEARCH_RADIUS_RANGE_M, SnapParams, TURN_PENALTY_FACTOR_RANGE,
};
use gt_snap::wire::{Costing, TraceOptions};

/// Distinct sent-point track indices across a plan's chunks, in order,
/// counting overlap points once.
fn distinct_sent_indices(plan: &RequestPlan) -> Vec<usize> {
    plan.chunks
        .iter()
        .flat_map(|chunk| chunk.owned_sent().iter().map(|sent| sent.point.as_usize()))
        .collect()
}

/// Downsampling: input rate vs. expected kept count over a 60-point track.
#[rstest]
#[case::already_1hz(1000, 60)]
#[case::hz_10_keeps_every_tenth(100, 6)]
#[case::hz_2_keeps_every_second_point(500, 30)]
#[case::slower_than_1hz_keeps_all(2000, 60)]
fn downsampling_respects_min_interval(#[case] step_ms: i64, #[case] expected: usize) {
    let plan = request_plan::plan(&points(60, step_ms, |_| None));
    assert_eq!(plan.sent_point_count(), expected);
}

#[test]
fn empty_track_plans_no_chunks() {
    let plan = request_plan::plan(&[]);
    assert_eq!(plan.chunks.len(), 0);
    assert_eq!(plan.sent_point_count(), 0);
    assert_eq!(plan.gps_accuracy_m, None);
}

#[test]
fn single_point_plans_one_chunk() {
    let plan = request_plan::plan(&points(1, 1000, |_| None));
    assert_eq!(plan.chunks.len(), 1);
    assert_eq!(plan.sent_point_count(), 1);
}

/// Chunk boundaries: sent-point count vs. expected chunk count.
#[rstest]
#[case::exactly_one_chunk(CHUNK_POINTS, 1)]
#[case::one_over(CHUNK_POINTS + 1, 2)]
#[case::one_under(CHUNK_POINTS - 1, 1)]
#[case::two_full_steps(2 * CHUNK_POINTS - CHUNK_OVERLAP_POINTS, 2)]
#[case::just_past_two_steps(2 * CHUNK_POINTS - CHUNK_OVERLAP_POINTS + 1, 3)]
fn chunk_count_at_boundaries(#[case] count: usize, #[case] expected_chunks: usize) {
    let plan = request_plan::plan(&points(count, 1000, |_| None));
    assert_eq!(
        plan.chunks.len(),
        expected_chunks,
        "for {count} sent points"
    );
    assert_eq!(plan.sent_point_count(), count);
}

/// Every sent point is owned by exactly one chunk, ownership is contiguous
/// over the original indices, and consecutive chunks overlap by exactly
/// [`CHUNK_OVERLAP_POINTS`].
#[rstest]
#[case(CHUNK_POINTS + 1)]
#[case(3 * CHUNK_POINTS)]
fn ownership_partitions_sent_points(#[case] count: usize) {
    let plan = request_plan::plan(&points(count, 1000, |_| None));
    let indices = distinct_sent_indices(&plan);
    let expected: Vec<usize> = (0..count).collect();
    assert_eq!(indices, expected);

    for pair in plan.chunks.windows(2) {
        let (Some(a), Some(b)) = (pair.first(), pair.get(1)) else {
            continue;
        };
        let a_indices: Vec<usize> = a.sent.iter().map(|s| s.point.as_usize()).collect();
        let b_indices: Vec<usize> = b.sent.iter().map(|s| s.point.as_usize()).collect();
        let shared: Vec<&usize> = a_indices.iter().filter(|i| b_indices.contains(i)).collect();
        assert_eq!(shared.len(), CHUNK_OVERLAP_POINTS);
    }
}

/// Downsampled tracks keep their original `PointIdx` provenance: a 10 Hz
/// track's sent points map back to every 10th original index.
#[test]
fn sent_points_carry_original_indices() {
    let plan = request_plan::plan(&points(50, 100, |_| None));
    let indices = distinct_sent_indices(&plan);
    assert_eq!(indices, vec![0, 10, 20, 30, 40]);
}

#[test]
fn out_of_order_timestamps_are_not_kept() {
    let mut pts = points(5, 1000, |_| None);
    pts.reverse();
    let plan = request_plan::plan(&pts);
    // Only the first point qualifies; every later one goes back in time.
    assert_eq!(plan.sent_point_count(), 1);
}

/// gps_accuracy: eph distribution vs. expected derived value.
#[rstest]
#[case::no_eph_at_all(&[], None)]
#[case::single_value(&[12.0], Some(12.0))]
#[case::odd_count_takes_middle(&[8.0, 10.0, 24.0], Some(10.0))]
#[case::even_count_averages_middles(&[8.0, 10.0, 14.0, 24.0], Some(12.0))]
#[case::clamped_below(&[1.0, 2.0, 3.0], Some(*GPS_ACCURACY_RANGE_M.start()))]
#[case::clamped_above(&[80.0, 90.0, 120.0], Some(*GPS_ACCURACY_RANGE_M.end()))]
#[case::outlier_resistant(&[9.0, 10.0, 11.0, 900.0, 900.0], Some(11.0))]
fn gps_accuracy_is_clamped_median(#[case] ephs: &[f32], #[case] expected: Option<f64>) {
    let pts = points(ephs.len().max(1), 1000, |i| ephs.get(i).copied());
    let plan = request_plan::plan(&pts);
    assert_eq!(plan.gps_accuracy_m, expected);
}

/// Points with eph but thinned away by downsampling do not contribute:
/// a 10 Hz track where only unsampled points carry huge eph derives from
/// the sent points alone.
#[test]
fn gps_accuracy_derives_from_sent_points_only() {
    // Every 10th point (the kept ones) has eph 10; the rest carry 900.
    let pts = points(50, 100, |i| Some(if i % 10 == 0 { 10.0 } else { 900.0 }));
    let plan = request_plan::plan(&pts);
    assert_eq!(plan.gps_accuracy_m, Some(10.0));
}

/// Advanced options: raw override values vs. the trace options actually
/// sent - everything clamps to the server-accepted ranges (the server
/// rejects out-of-range options with error 158 instead of clamping).
#[rstest]
#[case::in_range_passes_through(Some(25.0), Some(25.0))]
#[case::above_cap_clamps(Some(250.0), Some(*SEARCH_RADIUS_RANGE_M.end()))]
#[case::negative_clamps_to_zero(Some(-5.0), Some(*SEARCH_RADIUS_RANGE_M.start()))]
#[case::unset_stays_server_default(None, None)]
fn search_radius_is_clamped_at_request_build(
    #[case] configured: Option<f64>,
    #[case] sent: Option<f64>,
) {
    let params = SnapParams {
        search_radius_m: configured,
        ..SnapParams::new(Costing::Auto)
    };
    let sent_options = params.trace_options(None);
    assert_eq!(sent_options.and_then(|o| o.search_radius), sent);
}

#[rstest]
#[case::in_range_passes_through(Some(500.0), Some(500.0))]
#[case::above_cap_clamps(Some(1.0e9), Some(*TURN_PENALTY_FACTOR_RANGE.end()))]
#[case::negative_clamps_to_zero(Some(-1.0), Some(*TURN_PENALTY_FACTOR_RANGE.start()))]
#[case::unset_stays_server_default(None, None)]
fn turn_penalty_factor_is_clamped_at_request_build(
    #[case] configured: Option<f64>,
    #[case] sent: Option<f64>,
) {
    let params = SnapParams {
        turn_penalty_factor: configured,
        ..SnapParams::new(Costing::Auto)
    };
    let sent_options = params.trace_options(None);
    assert_eq!(sent_options.and_then(|o| o.turn_penalty_factor), sent);
}

/// gps_accuracy precedence: the clamped override beats the derived value,
/// the derived value fills in when no override is set, and the override's
/// range is the server's full range, wider than the derivation clamp.
#[rstest]
#[case::override_beats_derived(Some(50.0), Some(12.0), Some(50.0))]
#[case::override_clamps_to_server_range(Some(150.0), Some(12.0), Some(*GPS_ACCURACY_OVERRIDE_RANGE_M.end()))]
#[case::derived_fills_absent_override(None, Some(12.0), Some(12.0))]
#[case::neither_stays_server_default(None, None, None)]
fn gps_accuracy_override_beats_derived(
    #[case] configured: Option<f64>,
    #[case] derived: Option<f64>,
    #[case] sent: Option<f64>,
) {
    let params = SnapParams {
        gps_accuracy_override_m: configured,
        ..SnapParams::new(Costing::Auto)
    };
    assert_eq!(params.gps_accuracy_sent_m(derived), sent);
}

/// All-default params send no trace_options at all; the chunk request
/// carries them only when something differs from the server default.
#[test]
fn default_params_send_no_trace_options() {
    assert_eq!(SnapParams::new(Costing::Auto).trace_options(None), None);
    assert_eq!(
        SnapParams::new(Costing::Auto).trace_options(Some(12.0)),
        Some(TraceOptions {
            gps_accuracy: Some(12.0),
            search_radius: None,
            turn_penalty_factor: None,
        })
    );
}
