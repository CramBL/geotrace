use std::iter;

use gt_map::test_util::{self, MapScene};
use gt_types::{FileIdx, TrackIdx, TrackRef};
use gt_ui_types::{DisplayCategory, DrawLayer, MatchHighlight, QueryMatches, TrackRanges};

#[rstest::rstest]
#[case::keep_without_draw(false)]
#[case::another_track_has_draw_ranges(true)]
fn hovered_kept_match_draws_with_other_track_layers_hidden(#[case] draw_other_track: bool) {
    let mut files = test_util::a_walking_recording();
    let track = files[0].tracks[0].clone();
    files[0].tracks.push(track);
    let mut query = QueryMatches {
        hidden: TrackRanges::from_iter([(test_util::track0(), vec![0..10, 25..30])]),
        ..QueryMatches::default()
    };
    if draw_other_track {
        query.draws.push(DrawLayer {
            color: 0,
            ranges: TrackRanges::from_iter([(
                TrackRef::new(FileIdx::new(0), TrackIdx::new(1)),
                iter::once(0..5).collect(),
            )]),
        });
    }
    let mut map = MapScene::of(files)
        .draw_state(|state| state.highlight.fading_enabled = false)
        .render();
    map.draw_state()
        .display_mask
        .solo(DisplayCategory::QueryHighlights);
    map.overlays().query_matches = Some(query);
    map.render_one_more_frame();
    let without_hover = map.shapes_painted();
    map.draw_state().highlight.hover_match =
        Some(MatchHighlight::new(test_util::track0(), &(10..25)));
    map.render_one_more_frame();
    assert!(map.shapes_painted() > without_hover);
}

#[test]
fn query_hidden_hovered_match_adds_no_halo_alongside_a_visible_draw_range() {
    let files = test_util::a_walking_recording();
    let mut query = test_util::a_run_drawing(test_util::track0(), 0..10);
    query
        .hidden
        .insert(test_util::track0(), iter::once(14..20).collect());
    let mut map = MapScene::of(files)
        .draw_state(|state| state.highlight.fading_enabled = false)
        .render();
    map.draw_state()
        .display_mask
        .solo(DisplayCategory::QueryHighlights);
    map.overlays().query_matches = Some(query);
    map.render_one_more_frame();
    let without_hover = map.shapes_painted();
    map.draw_state().highlight.hover_match =
        Some(MatchHighlight::new(test_util::track0(), &(14..20)));
    map.render_one_more_frame();
    assert_eq!(map.shapes_painted(), without_hover);
}
