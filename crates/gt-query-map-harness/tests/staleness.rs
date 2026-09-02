//! Results whose inputs moved under them: the map keeps drawing the last run,
//! marked stale, until it runs again.

use gt_query_map_harness::{Dataset, FileSpec, MapScenario, TrackSpec, track};
use gt_types::FileIdx;

fn scenario() -> MapScenario {
    MapScenario::new(Dataset::single_track(TrackSpec::from_speeds_kmh(&[
        5.0, 40.0, 40.0, 5.0, 40.0, 5.0,
    ])))
}

/// Narrowing the time filter after a run marks the results stale, and the
/// points it excluded read as filtered out rather than hidden.
#[test]
fn narrowing_the_time_filter_grays_the_results_out() {
    let mut scenario = scenario();
    scenario.run("points | where velocity > 30 km/h | draw");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .00.0.
    counts: shown 6, halos 2
    ");

    scenario
        .set_time_filter_secs(Some(2), Some(4))
        .refresh_staleness();
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  --0.0-
    counts: shown 3, halos 2
    stale
    ");
    insta::assert_snapshot!(scenario.panel(), @"
    chunks: 1
      0..40 ok
    run: completed
      2 matches on 1 track
    stale
    ");
}

/// Disabling a track after a run marks the results stale; its points are off
/// the map entirely, halo or not.
#[test]
fn disabling_an_evaluated_track_grays_the_results_out() {
    let mut scenario = MapScenario::new(Dataset::of_files(&[
        FileSpec::with_tracks(
            "a.gtd",
            vec![TrackSpec::from_speeds_kmh(&[5.0, 40.0, 40.0])],
        ),
        FileSpec::with_tracks(
            "b.gtd",
            vec![TrackSpec::from_speeds_kmh(&[40.0, 5.0, 40.0])],
        ),
    ]));
    scenario.run("points | where velocity > 30 km/h | draw");
    insta::assert_snapshot!(scenario.picture(), @"
    a.gtd#0  .00
    b.gtd#0  0.0
    counts: shown 6, halos 3
    ");

    scenario
        .set_track_visible(track(0, 0), false)
        .refresh_staleness();
    insta::assert_snapshot!(scenario.picture(), @"
    a.gtd#0  ooo
    b.gtd#0  0.0
    counts: shown 3, halos 2
    stale
    ");
}

/// A run's point ranges address the points of the file it read. The halos here
/// stay on points 1 and 2, where the replacement holds 5 km/h.
#[test]
fn replacing_the_loaded_file_with_another_of_the_same_name_grays_the_results_out() {
    let mut scenario = MapScenario::new(Dataset::single_track(TrackSpec::from_speeds_kmh(&[
        5.0, 40.0, 40.0, 5.0,
    ])));
    scenario.run("points | where velocity > 30 km/h | draw");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .00.
    counts: shown 4, halos 1
    ");

    scenario.replace_file(
        FileIdx::new(0),
        &FileSpec::with_tracks(
            "track.gtd",
            vec![TrackSpec::from_speeds_kmh(&[40.0, 5.0, 5.0, 40.0])],
        ),
    );
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .00.
    counts: shown 4, halos 1
    stale
    ");
}

/// Running again over the changed inputs clears the staleness and re-slices the
/// matches to the new window.
#[test]
fn running_again_clears_the_staleness() {
    let mut scenario = scenario();
    scenario.run("points | where velocity > 30 km/h | draw");
    scenario.set_time_filter_secs(Some(2), Some(4));
    scenario.run_current();
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  --0.0-
    counts: shown 3, halos 2
    ");
}

/// Widening the filter back to what the run saw makes the results current
/// again: staleness compares inputs, it is not a one-way latch.
#[test]
fn restoring_the_inputs_makes_the_results_current_again() {
    let mut scenario = scenario();
    scenario.run("points | where velocity > 30 km/h | draw");
    scenario.set_time_filter_secs(Some(2), Some(4));
    assert!(
        scenario.picture().stale,
        "the narrowed window is not what the run saw"
    );
    scenario.set_time_filter_secs(None, None);
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .00.0.
    counts: shown 6, halos 2
    ");
}
