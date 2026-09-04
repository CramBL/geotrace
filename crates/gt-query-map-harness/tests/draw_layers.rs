//! More `draw` queries than the halo mask can hold.

use gt_query_map_harness::{Dataset, MapScenario, TrackSpec, track};
use gt_ui_types::DrawLayerMask;

/// One draw query per layer index, each with a higher threshold than the last,
/// so every layer covers a shorter stretch of the track.
fn stacked_draws(count: usize) -> String {
    (0..count)
        .map(|i| format!("points | where velocity > {i} km/h | draw"))
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// `count` points, the i-th at `i + 1` km/h, so the fastest clears every
/// threshold [`stacked_draws`] writes.
fn rising_speeds(count: usize) -> Vec<f64> {
    (0..count).map(|i| (i + 1) as f64).collect()
}

/// Exactly as many draw queries as the mask has bits: every one renders, and the
/// points they all cover read as multiply-haloed.
#[test]
fn every_layer_up_to_the_cap_renders() {
    let count = DrawLayerMask::MAX_LAYERS;
    let mut scenario = MapScenario::new(Dataset::single_track(TrackSpec::from_speeds_kmh(
        &rising_speeds(count),
    )));
    scenario.run(&stacked_draws(count));

    let matches = scenario.matches().expect("the run completed");
    assert_eq!(
        matches.draws.len(),
        count,
        "every draw query has its own layer"
    );
    // The fastest point clears every threshold, so its mask is full.
    let last = count - 1;
    assert_eq!(
        scenario.classify(track(0, 0), last).draw_layers.count() as usize,
        count,
        "the fastest point carries every layer"
    );
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  0***************
    counts: shown 16, halos 16
    ");
}

/// One draw query past the cap: the extra layer is dropped from the map, and
/// the query still reports its own matches in the panel.
#[test]
fn a_draw_query_past_the_cap_is_dropped_from_the_map() {
    let count = DrawLayerMask::MAX_LAYERS + 1;
    let mut scenario = MapScenario::new(Dataset::single_track(TrackSpec::from_speeds_kmh(
        &rising_speeds(count),
    )));
    scenario.run(&stacked_draws(count));

    let matches = scenario.matches().expect("the run completed");
    assert_eq!(
        matches.draws.len(),
        DrawLayerMask::MAX_LAYERS,
        "the map holds only as many layers as the mask has bits"
    );
    assert_eq!(
        scenario
            .classify(track(0, 0), count - 1)
            .draw_layers
            .count() as usize,
        DrawLayerMask::MAX_LAYERS,
        "the dropped layer does not alias onto a rendered one"
    );
    // The panel still lists every query, the last one without a color swatch.
    insta::assert_snapshot!(scenario.panel(), @"
    chunks: 17
      0..39 ok
      41..80 ok
      82..121 ok
      123..162 ok
      164..203 ok
      205..244 ok
      246..285 ok
      287..326 ok
      328..367 ok
      369..408 ok
      410..450 ok
      452..492 ok
      494..534 ok
      536..576 ok
      578..618 ok
      620..660 ok
      662..702 ok
    run: completed
      1 match on 1 track
      1 match on 1 track
      1 match on 1 track
      1 match on 1 track
      1 match on 1 track
      1 match on 1 track
      1 match on 1 track
      1 match on 1 track
      1 match on 1 track
      1 match on 1 track
      1 match on 1 track
      1 match on 1 track
      1 match on 1 track
      1 match on 1 track
      1 match on 1 track
      1 match on 1 track
      1 match on 1 track
    ");
}

/// Two draw layers over the same points read as one multiply-haloed stretch,
/// not as either layer alone.
#[test]
fn overlapping_layers_read_as_a_multi_layer_stretch() {
    let mut scenario = MapScenario::new(Dataset::single_track(TrackSpec::from_speeds_kmh(&[
        5.0, 20.0, 40.0, 60.0,
    ])));
    scenario.run(&stacked_draws(2));
    insta::assert_snapshot!(scenario.picture(), @"
    track.gtd#0  ****
    counts: shown 4, halos 2
    ");
}
