//! Snap-to-road runs pushed into the scheduler cache, with readers for the
//! cached run and the session costing override of a track.

use egui_kittest::Harness;
use gt_snap::merge::{SnapKindCounts, SnapPoint, SnapResult};
use gt_snap::request_plan::SnapParams;
use gt_snap::snapped_track::{Position, SnappedTrackSegment};
use gt_snap::wire::{Costing, SnapPointKind};
use gt_types::{PointIdx, TrackRef};

use crate::app::App;
use crate::app::snap::{SnapCacheKey, SnapRun, TrackContentKey};

/// Whether the scheduler still holds a cached auto-costing run for `track`.
pub fn has_cached_auto_run(harness: &Harness<'_, App>, track: TrackRef) -> bool {
    let state = harness.state();
    let shared = state.shared.borrow();
    let loaded = track.resolve(shared.loaded_files.files()).expect("track");
    state
        .snap
        .has_cached_run(loaded, state.snap_settings.params(Costing::Auto))
}

/// Add `track` to the panel's selection, as clicking its row would.
pub fn select_track(harness: &mut Harness<'_, App>, track: TrackRef) {
    let state = harness.state_mut();
    let mut shared = state.shared.borrow_mut();
    shared
        .tree
        .selection
        .insert(gt_side_panel::NodeKey::Track(track));
}

/// The session costing override stored for `track`, if any.
pub fn costing_override(harness: &Harness<'_, App>, track: TrackRef) -> Option<Costing> {
    let state = harness.state();
    let shared = state.shared.borrow();
    let loaded = track.resolve(shared.loaded_files.files())?;
    state
        .snap_costing_overrides
        .get(&TrackContentKey::new(loaded))
        .copied()
}

/// Inject a completed run for `track` straight into the scheduler cache,
/// keyed the way the app's view builders look it up: one snapped segment for
/// the map, and per-point results for the plot - errors for the first sixty
/// points with an unsnapped stretch at indices 20..25, so the snap error
/// series has a line break and markers to show.
pub fn inject_completed_run(harness: &mut Harness<'_, App>, track: TrackRef) {
    let points: Vec<SnapPoint> = (0..60)
        .map(|i| {
            let kind = if (20..25).contains(&i) {
                SnapPointKind::Unsnapped
            } else if i % 2 == 0 {
                SnapPointKind::Snapped
            } else {
                SnapPointKind::Interpolated
            };
            SnapPoint {
                point: PointIdx::new(i),
                kind,
                error_m: (kind != SnapPointKind::Unsnapped)
                    .then(|| 2.0 + f64::from(u8::try_from(i % 7).unwrap_or(0))),
                snapped: None,
                edge: None,
                follows_gap: i == 0,
            }
        })
        .collect();
    let result = SnapResult {
        points,
        segments: vec![SnappedTrackSegment {
            positions: vec![
                Position {
                    lat: 55.68,
                    lon: 12.56,
                },
                Position {
                    lat: 55.69,
                    lon: 12.57,
                },
            ],
            edge_spans: Vec::new(),
            recorded_points: Vec::new(),
        }],
        edges: Vec::new(),
        kind_counts: SnapKindCounts::default(),
        confidence_score: None,
        osm_changeset: None,
        params: SnapParams::new(Costing::Auto),
        gps_accuracy_sent_m: None,
        partial: false,
    };
    let key = {
        let state = harness.state();
        let shared = state.shared.borrow();
        let loaded_track = track
            .resolve(shared.loaded_files.files())
            .expect("track just pushed");
        SnapCacheKey::new(
            loaded_track,
            SnapParams::new(Costing::Auto),
            gt_snap::server_host(gt_snap::DEFAULT_SERVER_URL),
        )
    };
    harness.state_mut().snap.insert_run(
        key,
        SnapRun::new(
            result,
            Vec::new(),
            gt_snap::server_host(gt_snap::DEFAULT_SERVER_URL),
        ),
    );
}
