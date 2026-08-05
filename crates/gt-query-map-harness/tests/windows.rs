//! Windowed queries, whose matches are stretches rather than single points, and
//! the other per-point metrics a recording carries.

use gt_query_map_harness::{Dataset, MapScenario, PointSpec, TrackSpec, track};

/// A window match bands every point of the window, not just the one the
/// predicate happened to be evaluated at.
#[test]
fn a_window_match_bands_the_whole_window() {
    let mut scenario = MapScenario::new(Dataset::single_track(TrackSpec::from_speeds_kmh(&[
        5.0, 5.0, 40.0, 40.0, 40.0, 5.0,
    ])));
    scenario.run("points | window 3 | where min(velocity) > 30 km/h | draw");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  ..000.
    counts: shown 6, halos 1
    ");
    assert_eq!(scenario.panel_matches(0), [(track(0, 0), 2..5)]);
}

/// Heading spread over a window - the multipath indicator from the examples
/// list - bands the swinging stretch.
#[test]
fn a_heading_spread_window_bands_the_swinging_stretch() {
    let headings = [10.0, 10.0, 10.0, 200.0, 20.0, 190.0, 10.0, 10.0];
    let mut scenario = MapScenario::new(Dataset::single_track(TrackSpec::from_points(
        headings
            .iter()
            .enumerate()
            .map(|(i, &heading)| {
                PointSpec::at_secs(i as i64)
                    .speed_kmh(40.0)
                    .heading_deg(heading)
            })
            .collect(),
    )));
    scenario.run("points | window 3 | where spread(heading) > 90 deg | draw");
    // Every window from point 1 on holds one of the swung headings, so the band
    // reaches from there to the end; only the opening window is calm.
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .0000000
    counts: shown 8, halos 1
    ");
}

/// Accuracy is a plain per-point metric, so its matches are single points.
#[test]
fn an_accuracy_filter_matches_single_points() {
    let mut scenario = MapScenario::new(Dataset::single_track(TrackSpec::from_points(
        [2.0_f32, 25.0, 3.0, 30.0]
            .iter()
            .enumerate()
            .map(|(i, &eph)| PointSpec::at_secs(i as i64).eph_m(eph))
            .collect(),
    )));
    scenario.run("points | where eph > 20 m | draw");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .0.0
    counts: shown 4, halos 2
    ");
}

/// A window cannot span a point an earlier query hid: the pipeline evaluates
/// each stage over the surviving runs, so the window restarts at every gap.
#[test]
fn a_window_never_spans_a_hidden_point() {
    // Fast everywhere except point 3, which the first query hides, leaving runs
    // of three and two - only the first is long enough for a 3-point window.
    let mut scenario = MapScenario::new(Dataset::single_track(TrackSpec::from_speeds_kmh(&[
        40.0, 40.0, 40.0, 1.0, 40.0, 40.0,
    ])));
    scenario.run(
        "points | where velocity < 10 km/h | hide\n\n\
         points | window 3 | where min(velocity) > 30 km/h | draw",
    );
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  000x..
    counts: shown 5, halos 1
    ");
}

/// Interference values ride alongside the recording rather than in it, and
/// resolve as a percentage.
#[test]
fn an_interference_filter_reads_the_supplied_series() {
    let mut scenario = MapScenario::new(Dataset::single_track(
        TrackSpec::steady(4, 40.0).jamming(vec![Some(2.0), Some(40.0), Some(60.0), None]),
    ));
    scenario.run("points | where jamming > 30 % | draw");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .00.
    counts: shown 4, halos 1
    ");
}
