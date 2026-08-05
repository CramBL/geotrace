//! One query per display mode, and what the map does with it.

use gt_query_map_harness::{Dataset, MapScenario, TrackSpec};
use rstest::rstest;

/// Four points: slow, fast, fast, slow. `30 km/h` splits them in the middle.
fn scenario() -> MapScenario {
    MapScenario::new(Dataset::single_track(TrackSpec::from_speeds_kmh(&[
        5.0, 40.0, 40.0, 5.0,
    ])))
}

#[test]
fn draw_halos_the_matched_points_and_hides_nothing() {
    let mut scenario = scenario();
    scenario.run("points | where velocity > 30 km/h | draw");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .00.
    counts: shown 4, halos 1
    ");
}

#[test]
fn hide_removes_the_matched_points() {
    let mut scenario = scenario();
    scenario.run("points | where velocity > 30 km/h | hide");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .xx.
    counts: shown 2, halos 0
    ");
}

#[test]
fn keep_removes_everything_else() {
    let mut scenario = scenario();
    scenario.run("points | where velocity > 30 km/h | keep");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  x..x
    counts: shown 2, halos 0
    ");
}

/// A query matching nothing leaves the map alone in `draw` and `hide`, and
/// empties it in `keep`, which is the mode's whole point.
#[rstest]
#[case("draw")]
#[case("keep")]
#[case("hide")]
fn a_query_matching_nothing_reads_per_mode(#[case] mode: &str) {
    let mut scenario = scenario();
    scenario.run(&format!("points | where velocity > 500 km/h | {mode}"));
    insta::assert_snapshot!(format!("no_match_{mode}"), scenario.picture());
}

/// The default mode is `draw`, so a query without a mode stage halos.
#[test]
fn a_query_without_a_mode_stage_draws() {
    let mut scenario = scenario();
    scenario.run("points | where velocity > 30 km/h");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .00.
    counts: shown 4, halos 1
    ");
}
