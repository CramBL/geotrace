//! One query per display mode, and what the map does with it.

use gt_query_map_harness::MapScenario;
use rstest::rstest;

#[test]
fn draw_halos_the_matched_points_and_hides_nothing() {
    let mut scenario = MapScenario::of_speeds_kmh(&[5.0, 40.0, 40.0, 5.0]);
    scenario.run("points | where velocity > 30 km/h | draw");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .00.
    counts: shown 4, halos 1
    ");
}

#[test]
fn hide_removes_the_matched_points() {
    let mut scenario = MapScenario::of_speeds_kmh(&[5.0, 40.0, 40.0, 5.0]);
    scenario.run("points | where velocity > 30 km/h | hide");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .xx.
    counts: shown 2, halos 0
    ");
}

#[test]
fn keep_removes_everything_else() {
    let mut scenario = MapScenario::of_speeds_kmh(&[5.0, 40.0, 40.0, 5.0]);
    scenario.run("points | where velocity > 30 km/h | keep");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  x..x
    counts: shown 2, halos 0
    ");
}

/// A query matching nothing leaves the map alone in `draw` and `hide`, and
/// empties it in `keep`, which is the mode's whole point.
#[rstest]
#[case::draw("draw", "track.gtd#0  ....\ncounts: shown 4, halos 0")]
#[case::hide("hide", "track.gtd#0  ....\ncounts: shown 4, halos 0")]
#[case::keep("keep", "track.gtd#0  xxxx\ncounts: shown 0, halos 0")]
fn a_query_matching_nothing_reads_per_mode(#[case] mode: &str, #[case] expected: &str) {
    let mut scenario = MapScenario::of_speeds_kmh(&[5.0, 40.0, 40.0, 5.0]);
    scenario.run(&format!("points | where velocity > 500 km/h | {mode}"));
    assert_eq!(scenario.picture().to_string(), expected);
}

/// The default mode is `draw`, so a query without a mode stage halos.
#[test]
fn a_query_without_a_mode_stage_draws() {
    let mut scenario = MapScenario::of_speeds_kmh(&[5.0, 40.0, 40.0, 5.0]);
    scenario.run("points | where velocity > 30 km/h");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .00.
    counts: shown 4, halos 1
    ");
}
