//! Several queries composing as one pipeline.

use gt_query_map_harness::{Dataset, MapScenario, PointSpec, TrackSpec};

/// Slow at the ends, fast in the middle - the shape that makes composition
/// visible.
fn scenario() -> MapScenario {
    MapScenario::new(Dataset::single_track(TrackSpec::from_speeds_kmh(&[
        5.0, 5.0, 40.0, 40.0, 40.0, 40.0, 5.0, 5.0,
    ])))
}

/// Two queries, blank-line separated: the first hides, the second draws over
/// what survives.
#[test]
fn hide_then_draw_halos_only_the_survivors() {
    let mut scenario = scenario();
    scenario.run(
        "points | where velocity < 10 km/h | hide\n\n\
         points | where velocity > 10 km/h | draw",
    );
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  xx0000xx
    counts: shown 4, halos 1
    ");
}

/// A later `hide` retracts the halo of an earlier `draw`: the pipeline only
/// halos points that survive to the end.
#[test]
fn a_later_hide_removes_the_earlier_halo() {
    let mut scenario = scenario();
    scenario.run(
        "points | where velocity > 1 km/h | draw\n\n\
         points | where velocity < 10 km/h | hide",
    );
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  xx0000xx
    counts: shown 4, halos 1
    ");
}

/// Two `keep` queries intersect: only points both keep survive.
#[test]
fn keep_then_keep_intersects() {
    let mut scenario = MapScenario::new(Dataset::single_track(TrackSpec::from_speeds_kmh(&[
        5.0, 20.0, 40.0, 60.0,
    ])));
    scenario.run(
        "points | where velocity > 10 km/h | keep\n\n\
         points | where velocity < 50 km/h | keep",
    );
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  x..x
    counts: shown 2, halos 0
    ");
}

/// Three stages: hide the slow ends, draw the fast middle, then draw a narrower
/// band inside it - the second halo layer stacks on the first.
#[test]
fn three_stages_stack_two_halo_layers() {
    let mut scenario = MapScenario::new(Dataset::single_track(TrackSpec::from_speeds_kmh(&[
        5.0, 20.0, 40.0, 60.0, 40.0, 20.0, 5.0,
    ])));
    scenario.run(
        "points | where velocity < 10 km/h | hide\n\n\
         points | where velocity > 15 km/h | draw\n\n\
         points | where velocity > 50 km/h | draw",
    );
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  x00*00x
    counts: shown 5, halos 2
    ");
}

/// A stage whose input is already fully hidden matches nothing: the pipeline
/// evaluates it over the survivors, and there are none.
#[test]
fn a_stage_after_everything_is_hidden_matches_nothing() {
    let mut scenario = scenario();
    scenario.run(
        "points | where velocity > 0 km/h | hide\n\n\
         points | where velocity > 10 km/h | draw",
    );
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  xxxxxxxx
    counts: shown 0, halos 0
    ");
    insta::assert_snapshot!(scenario.panel(), @"
    chunks: 2
      0..39 ok
      41..81 ok
    run: completed
      1 match on 1 track — 8 of 8 points hidden
      0 matches on 0 tracks
    ");
}

/// Two `hide` stages do not commute when one judges a point by its neighbours.
///
/// `accel` differences against the previous point *of the current run*, so
/// hiding first can leave a survivor at a run boundary where `accel` is missing.
/// A point whose `accel` is missing does not match, so it stays. The
/// counterexample a property test shrank to, kept as the documented semantics.
#[test]
fn swapping_two_hides_can_change_the_map() {
    let dataset = || {
        Dataset::single_track(TrackSpec::from_points(vec![
            PointSpec::at_secs(18).speed_kmh(30.0),
            PointSpec::at_secs(31).speed_kmh(22.0),
            PointSpec::at_secs(42).speed_kmh(31.0),
            PointSpec::at_secs(46).speed_kmh(9.0),
        ]))
    };
    let slow = "points | where velocity < 23 km/h | hide";
    let slowing_or_fast = "points | where (accel < 0 m/s2) or (velocity >= 31 km/h) | hide";

    // Hiding the slow points first strands point 2 at the start of its own run,
    // where it has no acceleration to judge, so the second stage skips it.
    let mut slow_first = MapScenario::new(dataset());
    slow_first.set_time_filter_secs(Some(25), Some(48));
    slow_first.run(&format!("{slow}\n\n{slowing_or_fast}"));
    insta::assert_snapshot!(slow_first.picture(), @"
    track.gtd#0  -x.x
    counts: shown 1, halos 0
    ");

    // The other way round, point 2 still has its predecessor and is hidden for
    // being fast.
    let mut fast_first = MapScenario::new(dataset());
    fast_first.set_time_filter_secs(Some(25), Some(48));
    fast_first.run(&format!("{slowing_or_fast}\n\n{slow}"));
    insta::assert_snapshot!(fast_first.picture(), @"
    track.gtd#0  -xxx
    counts: shown 0, halos 0
    ");
}

/// Hiding then keeping cannot resurrect a hidden point.
#[test]
fn a_keep_after_a_hide_never_brings_points_back() {
    let mut scenario = scenario();
    scenario.run(
        "points | where velocity > 30 km/h | hide\n\n\
         points | where velocity > 30 km/h | keep",
    );
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  xxxxxxxx
    counts: shown 0, halos 0
    ");
}
