//! Clearing between runs: nothing of the first run may reach the second.

use gt_query_map_harness::{Dataset, MapScenario, TrackSpec, track};

fn scenario() -> MapScenario {
    MapScenario::new(Dataset::single_track(
        TrackSpec::from_speeds_kmh(&[5.0, 40.0, 40.0, 5.0]).snap_error(vec![
            Some(0.5),
            Some(30.0),
            Some(0.5),
            Some(0.5),
        ]),
    ))
}

#[test]
fn clearing_returns_the_map_to_untouched() {
    let mut scenario = scenario();
    scenario.run("points | where velocity > 30 km/h | hide");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .xx.
    counts: shown 2, halos 0
    ");

    scenario.clear();
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  ....
    counts: shown 4, halos 0
    ");
    assert!(scenario.matches().is_none(), "no matches remain to draw");
}

/// A second run in another mode replaces the first: no leftover halo where the
/// hide-run had hidden points.
#[test]
fn a_second_run_in_another_mode_replaces_the_first() {
    let mut scenario = scenario();
    scenario.run("points | where velocity > 30 km/h | hide");
    scenario.clear();
    scenario.run("points | where velocity > 30 km/h | draw");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .00.
    counts: shown 4, halos 1
    ");
}

/// A second run on another metric replaces the first: the snap-error run's
/// halo sits on its own point, not on the velocity run's.
#[test]
fn a_second_run_on_another_metric_replaces_the_first() {
    let mut scenario = scenario();
    scenario.run("points | where velocity > 30 km/h | draw");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .00.
    counts: shown 4, halos 1
    ");

    scenario.clear();
    scenario.run("points | where snap_error > 10 m | draw");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .0..
    counts: shown 4, halos 1
    ");
}

/// Running again without clearing first also replaces everything - clearing is
/// a convenience, not a requirement.
#[test]
fn running_again_without_clearing_replaces_the_results() {
    let mut scenario = scenario();
    scenario.run("points | where velocity > 30 km/h | hide");
    scenario.run("points | where velocity > 30 km/h | draw");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .00.
    counts: shown 4, halos 1
    ");
}

/// The selection is not query state: clearing the results keeps the pinned
/// point, and so does the next run.
#[test]
fn the_selection_survives_clearing_and_the_next_run() {
    let mut scenario = scenario();
    scenario.run("points | where velocity > 30 km/h | draw");
    scenario.select_point(track(0, 0), 1);
    scenario.clear();
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  ....
         select   ^
    popup: drawn
    counts: shown 4, halos 0
    ");

    scenario.run("points | where velocity < 10 km/h | draw");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  0..0
         select   ^
    popup: drawn
    counts: shown 4, halos 2
    ");
}

/// Clearing the results drops the hovered match with them.
#[test]
fn clearing_drops_the_hovered_match() {
    let mut scenario = scenario();
    scenario.run("points | where velocity > 30 km/h | draw");
    scenario.hover_match(0, 0);
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .00.
          hover   ~~
    counts: shown 4, halos 1
    ");

    scenario.clear();
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  ....
    counts: shown 4, halos 0
    ");
}
