//! Data a query cannot say much about: no points, one point, fewer points than
//! the window, and a metric the track never carried.

use gt_query_map_harness::{Dataset, FileSpec, MapScenario, PointSpec, TrackSpec};

/// A recording with no fixes has no tracks, so a run over it has nothing to
/// evaluate and nothing to say.
#[test]
fn a_file_without_tracks_runs_and_matches_nothing() {
    let mut scenario = MapScenario::new(Dataset::of_files(&[FileSpec::new("silent.gtd")]));
    scenario.run("points | where velocity > 30 km/h | draw");
    insta::assert_snapshot!(scenario.picture(), @"counts: shown 0, halos 0");
    insta::assert_snapshot!(scenario.panel(), @"
    chunks: 1
      0..40 ok
    run: completed
      0 matches on 0 tracks
    ");
}

/// A trackless file beside a real one must not swallow the real one's result.
#[test]
fn a_trackless_file_beside_a_real_one_is_harmless() {
    let mut scenario = MapScenario::new(Dataset::of_files(&[
        FileSpec::new("silent.gtd"),
        FileSpec::with_tracks("ride.gtd", vec![TrackSpec::from_speeds_kmh(&[5.0, 40.0])]),
    ]));
    scenario.run("points | where velocity > 30 km/h | draw");
    insta::assert_snapshot!(scenario.picture(), @"
    ride.gtd#0  .0
    counts: shown 2, halos 1
    ");
}

#[test]
fn a_single_point_track_can_match() {
    let mut scenario = MapScenario::new(Dataset::single_track(TrackSpec::from_speeds_kmh(&[40.0])));
    scenario.run("points | where velocity > 30 km/h | draw");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  0
    counts: shown 1, halos 1
    ");
}

/// `accel` needs a previous point to difference against, so a one-point track
/// resolves no value and matches nothing.
#[test]
fn a_single_point_track_has_no_acceleration() {
    let mut scenario = MapScenario::new(Dataset::single_track(TrackSpec::from_speeds_kmh(&[40.0])));
    scenario.run("points | where accel > 0 m/s2 | draw");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .
    counts: shown 1, halos 0
    ");
    insta::assert_snapshot!(scenario.panel(), @"
    chunks: 1
      0..36 ok
    run: completed
      0 matches on 0 tracks — 1 skipped (missing accel)
    ");
}

/// A window longer than the track matches nothing, and the summary says the
/// track was too short rather than staying silent.
#[test]
fn a_track_shorter_than_the_window_reports_it() {
    let mut scenario = MapScenario::new(Dataset::single_track(TrackSpec::from_speeds_kmh(&[
        40.0, 40.0, 40.0,
    ])));
    scenario.run("points | window 5 | where avg(velocity) > 1 km/h | draw");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  ...
    counts: shown 3, halos 0
    ");
    insta::assert_snapshot!(scenario.panel(), @"
    chunks: 1
      0..55 ok
    run: completed
      0 matches on 0 tracks — 1 track shorter than window
    ");
}

/// A metric the track carries no values for: every point is skipped, the
/// summary names the metric, and the map stays untouched.
#[test]
fn a_metric_the_track_never_carried_matches_nothing() {
    let mut scenario = MapScenario::new(Dataset::single_track(TrackSpec::from_speeds_kmh(&[
        5.0, 40.0, 40.0,
    ])));
    scenario.run("points | where snap_error > 1 m | draw");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  ...
    counts: shown 3, halos 0
    ");
    insta::assert_snapshot!(scenario.panel(), @"
    chunks: 1
      0..38 ok
    run: completed
      0 matches on 0 tracks — 3 skipped (missing snap_error) — 1 track without snap_error values
    ");
}

/// `keep` on a metric with no values hides the whole track: nothing can be
/// kept, and the summary accounts for every hidden point.
#[test]
fn keep_on_an_absent_metric_hides_everything() {
    let mut scenario = MapScenario::new(Dataset::single_track(TrackSpec::from_speeds_kmh(&[
        5.0, 40.0, 40.0,
    ])));
    scenario.run("points | where snap_error > 1 m | keep");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  xxx
    counts: shown 0, halos 0
    ");
    insta::assert_snapshot!(scenario.panel(), @"
    chunks: 1
      0..38 ok
    run: completed
      0 matches on 0 tracks — 3 of 3 points hidden — 3 skipped (missing snap_error) — 1 track without snap_error values
    ");
}

/// Points without a velocity at all - a receiver that reported position only.
#[test]
fn points_without_velocity_are_skipped_not_matched() {
    let mut scenario = MapScenario::new(Dataset::single_track(TrackSpec::from_points(
        (0..3).map(PointSpec::at_secs).collect(),
    )));
    scenario.run("points | where velocity > 30 km/h | draw");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  ...
    counts: shown 3, halos 0
    ");
}
