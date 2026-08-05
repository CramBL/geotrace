//! The selected point and the hovered results-table match, against what a
//! query does to the map underneath them.

use gt_query_map_harness::{Dataset, FileSpec, MapScenario, TrackSpec, track};

fn scenario() -> MapScenario {
    MapScenario::new(Dataset::single_track(TrackSpec::from_speeds_kmh(&[
        5.0, 40.0, 40.0, 5.0,
    ])))
}

#[test]
fn clicking_a_point_pins_it_and_clicking_again_unpins_it() {
    let mut scenario = scenario();
    scenario.select_point(track(0, 0), 2);
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  ....
         select    ^
    counts: shown 4, halos 0
    ");

    scenario.select_point(track(0, 0), 2);
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  ....
    counts: shown 4, halos 0
    ");
}

#[test]
fn clicking_another_point_moves_the_pin() {
    let mut scenario = scenario();
    scenario.select_point(track(0, 0), 1);
    scenario.select_point(track(0, 0), 3);
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  ....
         select     ^
    counts: shown 4, halos 0
    ");
}

/// A point selected before a query hides it stays selected: the selection is
/// not query state, so the pin survives a run that removes its point from the
/// map.
#[test]
fn a_query_that_hides_the_selected_point_keeps_the_pin() {
    let mut scenario = scenario();
    scenario.select_point(track(0, 0), 1);
    scenario.run("points | where velocity > 30 km/h | hide");
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .xx.
         select   ^
    counts: shown 2, halos 0
    ");
}

/// The same for a point the time filter excludes.
#[test]
fn a_time_filter_that_excludes_the_selected_point_keeps_the_pin() {
    let mut scenario = scenario();
    scenario.select_point(track(0, 0), 0);
    scenario.set_time_filter_secs(Some(2), None);
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  --..
         select  ^
    counts: shown 2, halos 0
    ");
}

/// Hovering a results-table row bands exactly the points that row lists.
#[test]
fn hovering_a_match_bands_its_points() {
    let mut scenario = scenario();
    scenario.run("points | where velocity > 30 km/h | draw");
    scenario.hover_match(0, 0);
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .00.
          hover   ~~
    counts: shown 4, halos 1
    ");

    scenario.clear_hover_match();
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .00.
    counts: shown 4, halos 1
    ");
}

/// A hovered match bands its own track only.
#[test]
fn hovering_a_match_leaves_the_other_tracks_alone() {
    let mut scenario = MapScenario::new(Dataset::of_files(&[
        FileSpec::with_tracks("a.gtd", vec![TrackSpec::from_speeds_kmh(&[40.0, 40.0])]),
        FileSpec::with_tracks("b.gtd", vec![TrackSpec::from_speeds_kmh(&[40.0, 40.0])]),
    ]));
    scenario.run("points | where velocity > 30 km/h | draw");
    scenario.hover_match(0, 1);
    insta::assert_snapshot!(scenario.picture(), @"
    a.gtd#0  00
    b.gtd#0  00
      hover  ~~
    counts: shown 4, halos 2
    ");
}

/// Hovering a match of a `hide` query bands points that are no longer drawn:
/// the table lists what the query matched, the map has removed it.
#[test]
fn hovering_a_hide_query_match_bands_hidden_points() {
    let mut scenario = scenario();
    scenario.run("points | where velocity > 30 km/h | hide");
    scenario.hover_match(0, 0);
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .xx.
          hover   ~~
    counts: shown 2, halos 0
    ");
}

/// Selection and hover coexist on the same point.
#[test]
fn a_point_can_be_selected_and_hovered_at_once() {
    let mut scenario = scenario();
    scenario.run("points | where velocity > 30 km/h | draw");
    scenario.select_point(track(0, 0), 1);
    scenario.hover_match(0, 0);
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  .00.
          hover   ~~
         select   ^
    counts: shown 4, halos 1
    ");
}
