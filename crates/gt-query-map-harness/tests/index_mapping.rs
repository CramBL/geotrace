//! Point indices across a time-filtered slice and across several tracks: the
//! evaluator sees slice-relative indices, the map needs absolute ones.

use gt_query_map_harness::{Dataset, FileSpec, MapScenario, TrackSpec, track};

/// Six points: slow, fast, fast, slow, fast, fast.
fn sliced_scenario() -> MapScenario {
    let mut scenario = MapScenario::new(Dataset::single_track(TrackSpec::from_speeds_kmh(&[
        5.0, 40.0, 40.0, 5.0, 40.0, 40.0,
    ])));
    // Only points 2..=5 are in the window, so a run over the slice must shift
    // its matches back by two to land on the map.
    scenario.set_time_filter_secs(Some(2), Some(5));
    scenario
}

#[test]
fn a_draw_over_a_slice_halos_the_absolute_points() {
    let mut scenario = sliced_scenario();
    scenario.run("points | where velocity > 30 km/h | draw");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  --0.00
    counts: shown 4, halos 2
    ");
}

#[test]
fn a_hide_over_a_slice_hides_the_absolute_points() {
    let mut scenario = sliced_scenario();
    scenario.run("points | where velocity > 30 km/h | hide");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  --x.xx
    counts: shown 1, halos 0
    ");
}

/// `keep` hides the non-matching points *of the slice*. Points outside the
/// window are never evaluated, so `keep` never hides them.
#[test]
fn a_keep_over_a_slice_only_hides_inside_the_window() {
    let mut scenario = sliced_scenario();
    scenario.run("points | where velocity > 30 km/h | keep");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  --.x..
    counts: shown 3, halos 0
    ");
}

/// The panel's match ranges are absolute point indices, the same ones the map
/// bands - so hovering a table row highlights the points it lists.
#[test]
fn panel_match_ranges_are_absolute() {
    let mut scenario = sliced_scenario();
    scenario.run("points | where velocity > 30 km/h | draw");
    let matches = scenario.panel_matches(0);
    assert_eq!(matches, [(track(0, 0), 2..3), (track(0, 0), 4..6)]);
    scenario.hover_match(0, 1);
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  --0.00
          hover      ~~
    counts: shown 4, halos 2
    ");
}

/// Narrowing the time filter can change what a windowed query matches near the
/// cut.
///
/// A window is only ever evaluated over points the run has, so trimming the
/// slice withdraws the windows that reached past the cut. Here point 1 survives
/// only because the window over points 1 and 2 averages slowly enough. Drop
/// point 2 and the only window left is the fast one. The counterexample a
/// property test shrank to, kept as the documented semantics.
#[test]
fn narrowing_the_window_can_change_a_windowed_match() {
    let dataset = || Dataset::single_track(TrackSpec::from_speeds_kmh(&[40.0, 10.0, 10.0]));
    let query = "points | window 2 | where avg(velocity) < 20 km/h | keep";

    let mut wide = MapScenario::new(dataset());
    wide.set_time_filter_secs(Some(0), Some(2));
    wide.run(query);
    insta::assert_snapshot!(wide.picture(), @"
    track.gtd#0  x..
    counts: shown 2, halos 0
    ");

    let mut narrow = MapScenario::new(dataset());
    narrow.set_time_filter_secs(Some(0), Some(1));
    narrow.run(query);
    insta::assert_snapshot!(narrow.picture(), @"
    track.gtd#0  xx-
    counts: shown 0, halos 0
    ");
}

/// A window with no points in it: the query sees an empty slice, so it hides
/// nothing - every point is already off the map by the filter alone.
#[test]
fn a_window_holding_no_points_leaves_keep_nothing_to_do() {
    let mut scenario = MapScenario::new(Dataset::single_track(TrackSpec::from_speeds_kmh(&[
        5.0, 40.0, 40.0,
    ])));
    scenario.set_time_filter_secs(Some(50), Some(60));
    scenario.run("points | where velocity > 30 km/h | keep");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  ooo
    counts: shown 0, halos 0
    ");
    assert!(
        scenario
            .matches()
            .is_some_and(|matches| matches.hidden_ranges(track(0, 0)).is_empty()),
        "an empty slice gives the query nothing to hide"
    );
}

/// Two tracks in one file and a third in another: each track's matches stay on
/// their own track.
#[test]
fn matches_never_bleed_across_tracks_or_files() {
    let mut scenario = MapScenario::new(Dataset::of_files(&[
        FileSpec::with_tracks(
            "a.gtd",
            vec![
                TrackSpec::from_speeds_kmh(&[40.0, 5.0, 5.0]),
                TrackSpec::from_speeds_kmh(&[5.0, 40.0, 5.0]),
            ],
        ),
        FileSpec::with_tracks("b.gtd", vec![TrackSpec::from_speeds_kmh(&[5.0, 5.0, 40.0])]),
    ]));
    scenario.run("points | where velocity > 30 km/h | draw");
    insta::assert_snapshot!(scenario.picture(), @"
    a.gtd#0  0..
    a.gtd#1  .0.
    b.gtd#0  ..0
    counts: shown 9, halos 3
    ");
}

/// Tracks of different lengths: the shorter track's indices must not be read
/// against the longer one's ranges.
#[test]
fn tracks_of_different_lengths_keep_their_own_ranges() {
    let mut scenario = MapScenario::new(Dataset::one_file(vec![
        TrackSpec::from_speeds_kmh(&[40.0, 40.0, 40.0, 40.0, 40.0]),
        TrackSpec::from_speeds_kmh(&[40.0]),
    ]));
    scenario.run("points | where velocity > 30 km/h | hide");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  xxxxx
    track.gtd#1  x
    counts: shown 0, halos 0
    ");
}

/// A track whose points sit outside the window entirely drops out at the track
/// level, while its neighbour still runs. The second track of a file starts a
/// recording's worth of time after the first, so a window over the first
/// excludes it.
#[test]
fn a_track_outside_the_window_drops_out_whole() {
    let mut scenario = MapScenario::new(Dataset::one_file(vec![
        TrackSpec::steady(3, 40.0),
        TrackSpec::steady(3, 40.0),
    ]));
    scenario.set_time_filter_secs(None, Some(10));
    scenario.run("points | where velocity > 30 km/h | draw");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  000
    track.gtd#1  ooo
    counts: shown 3, halos 1
    ");
}
