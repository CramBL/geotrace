use std::ops::Range;
use std::path::PathBuf;

use chrono::TimeDelta;
use egui::epaint::ColorMode;
use gt_types::fixtures::{self, FixKind};
use gt_types::{
    DataCategory, EventMarker, FileIdx, FileMetadata, GeoBounds, Latitude, LoadedFile, LoadedTrack,
    Longitude, MercBounds, MercPoint, PointIdx, SolutionState, SpatialPoint, TimeRange,
    TotalDistance, TrackIdx, TrackMetadata,
};
use gt_ui_types::{
    DisplayCategory, DrawLayer, FileVisibility, MatchHighlight, TrackRanges, TrackVisibility,
};
use rustc_hash::FxHashMap;
use uom::si::f64::Length;
use uom::si::length::{kilometer, meter};

use super::*;
use crate::hover_labels;
use crate::match_reveal::HaloStyle;
use crate::test_util::{self, DrawState, MapScene, RenderedMap};
use crate::viewport;

fn tpv_spatial_point(fi: usize, ti: usize, pi: usize) -> SpatialPoint {
    SpatialPoint {
        merc: MercPoint { x: 0.5, y: 0.5 },
        file_index: FileIdx::new(fi),
        track_index: TrackIdx::new(ti),
        point_index: PointIdx::new(pi),
        category: DataCategory::Tpv,
    }
}

pub(crate) fn vis_all_visible() -> TrackDataVisibility {
    TrackDataVisibility {
        files: vec![FileVisibility {
            enabled: true,
            tracks: vec![TrackVisibility::all_visible()],
        }],
    }
}

#[rstest::rstest]
#[case::one_omitted_fix(25..26, Vec::new(), false)]
#[case::coincident_omitted_fixes(1..63, (2..63).step_by(2).map(|i| i..i + 1).collect(), false)]
#[case::presented_fix(25..26, Vec::new(), true)]
fn map_highlight_paints_one_ring_for_omitted_fixes_at_the_same_position(
    #[case] points: Range<usize>,
    #[case] hidden: Vec<Range<usize>>,
    #[case] presented: bool,
) {
    let mut files = test_util::a_recording_of(64, 0.0);
    let stationary = &mut files[0].tracks[0];
    stationary.lod = gt_track_builder::build_track_lod(stationary.placed_points().unwrap());
    let track = test_util::track0();
    let zoom = 16.0_f64;
    let level = track
        .resolve(&files)
        .unwrap()
        .lod
        .select(256.0 * zoom.exp2(), crate::polyline::MAX_LOD_ERROR_PX)
        .unwrap();
    assert_eq!(level.indices(), &[0, 63]);

    let map = MapScene::of(files)
        .zoomed_to(zoom)
        .centred_on_fix(25)
        .draw_state(|state| {
            let highlight = Some(MatchHighlight::new(track, &points));
            if presented {
                state.highlight.presented_match = highlight;
            } else {
                state.highlight.hover_match = highlight;
            }
        })
        .overlays(|overlays| {
            overlays.query_matches = Some(QueryMatches {
                hidden: TrackRanges::from_iter([(track, hidden)]),
                ..Default::default()
            });
        })
        .render();
    let shapes = &map.harness.inner.output().shapes;
    assert_eq!(
        shapes
            .iter()
            .filter(|shape| matches!(&shape.shape, egui::Shape::Circle(circle)
                if circle.stroke.color == gt_ui_theme::QUERY_MATCH_HOVER_HALO))
            .count(),
        1,
    );
}

#[derive(Clone, Copy)]
enum HaloGap {
    QueryExclusion,
    TimeReversal,
}

#[rstest::rstest]
#[case::simplified(12.0, true, HaloGap::QueryExclusion)]
#[case::west_of_the_antimeridian(179.999_98, false, HaloGap::QueryExclusion)]
#[case::reversed_time(12.0, true, HaloGap::TimeReversal)]
fn query_hover_halo_stops_at_hidden_and_time_filtered_fixes(
    #[case] longitude_origin: f64,
    #[case] simplified: bool,
    #[case] gap: HaloGap,
) {
    let count = 128;
    let step_degrees = 0.000_004;
    let start = 15;
    let first_run_last = 74;
    let isolated = 96;
    let excluded = first_run_last + 1..isolated;
    let points = (0..count)
        .map(|index| {
            let lon =
                (longitude_origin + index as f64 * step_degrees + 180.0).rem_euclid(360.0) - 180.0;
            let minute = if matches!(gap, HaloGap::TimeReversal) && excluded.contains(&index) {
                index + count
            } else {
                index
            };
            fixtures::nav_point(
                test_util::epoch() + TimeDelta::minutes(minute as i64),
                Latitude::new(0.0),
                Longitude::new(lon),
                FixKind::Measured,
            )
        })
        .collect();
    let built = test_util::track_built_from(points);
    let zoom = 16.0_f64;
    let world_px = 256.0 * zoom.exp2();
    let level = built
        .lod
        .select(world_px, crate::polyline::MAX_LOD_ERROR_PX);
    assert_eq!(
        level.is_some_and(|level| level.indices().len() < count),
        simplified
    );
    let time = |index: usize| built.points[index].tpv.time().utc();
    let filter = GlobalFilter::default().with_time_bounds(Some(time(start)), Some(time(isolated)));
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![built])];
    let track = test_util::track0();
    let matches = matches!(gap, HaloGap::QueryExclusion).then(|| QueryMatches {
        hidden: TrackRanges::from_iter([(track, vec![excluded])]),
        ..Default::default()
    });
    let map = MapScene::of(files)
        .zoomed_to(zoom)
        .centred_on_fix(start)
        .draw_state(|state| {
            state.filter = filter;
            state.highlight.hover_match = Some(MatchHighlight::new(track, &(9..120)));
        })
        .overlays(|overlays| overlays.query_matches = matches)
        .render();
    let color = gt_ui_theme::QUERY_MATCH_HOVER_HALO;
    let shapes = &map.harness.inner.output().shapes;
    let bands: Vec<_> = shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Path(path) if path.stroke.color == ColorMode::Solid(color) => {
                Some(&path.points)
            }
            _ => None,
        })
        .collect();
    let rings: Vec<_> = shapes
        .iter()
        .filter_map(|shape| match &shape.shape {
            egui::Shape::Circle(circle) if circle.stroke.color == color => Some(circle.center),
            _ => None,
        })
        .collect();
    assert_eq!(bands.len(), 1);
    assert_eq!(rings.len(), 1);
    let expected_x = |index| {
        test_util::viewport_center().x
            + ((index - start) as f64 * step_degrees * world_px / 360.0) as f32
    };
    let band = bands[0];
    assert!(band.len() > 2);
    assert!((band.first().unwrap().x - expected_x(start)).abs() < 0.1);
    assert!((band.last().unwrap().x - expected_x(first_run_last)).abs() < 1.0);
    assert!((rings[0].x - expected_x(isolated)).abs() < 0.1);
}

#[test]
fn query_hover_paints_one_short_band_for_overlapping_omitted_runs() {
    let offsets_px = [
        0.0, 0.36, 0.53, -0.36, -0.53, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0,
    ];
    let repeats = 64;
    let count = repeats * offsets_px.len();
    let world_px = 0.185 / gt_types::track::LOD_BASE_TOLERANCE_MERC;
    let zoom = (world_px / 256.0).log2();
    let points = offsets_px
        .into_iter()
        .cycle()
        .take(count)
        .enumerate()
        .map(|(index, offset)| {
            fixtures::nav_point(
                test_util::epoch() + TimeDelta::minutes(index as i64),
                Latitude::new(0.0),
                Longitude::new(12.0 + offset * 360.0 / world_px),
                FixKind::Measured,
            )
        })
        .collect();
    let built = test_util::track_built_from(points);
    let level = built
        .lod
        .select(world_px, crate::polyline::MAX_LOD_ERROR_PX)
        .unwrap();
    assert_eq!(level.indices(), &[0, (count - 1) as u32]);

    let hidden = std::iter::once(0..2)
        .chain((0..repeats).map(|index| {
            let wave = index * offsets_px.len();
            wave + 5..(wave + offsets_px.len() + 2).min(count)
        }))
        .collect();
    let track = test_util::track0();
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![built])];
    let rendered = MapScene::of(files)
        .zoomed_to(zoom)
        .centred_on_fix(0)
        .draw_state(|state| {
            state.highlight.hover_match = Some(MatchHighlight::new(track, &(0..count)));
        })
        .overlays(|overlays| {
            overlays.query_matches = Some(QueryMatches {
                hidden: TrackRanges::from_iter([(track, hidden)]),
                ..Default::default()
            });
        })
        .render();
    let color = ColorMode::Solid(gt_ui_theme::QUERY_MATCH_HOVER_HALO);
    let bands = rendered
        .harness
        .inner
        .output()
        .shapes
        .iter()
        .filter(
            |shape| matches!(&shape.shape, egui::Shape::Path(path) if path.stroke.color == color),
        )
        .count();
    assert_eq!(bands, 1);
}

#[derive(Clone, Copy)]
enum ClipHalo {
    InteriorBand,
    InteriorRing,
    MarginBand,
    MarginRing,
}

#[rstest::rstest]
#[case::band(&[(-4.0, 0.0), (-2.0, 0.0), (0.0, 0.0), (2.0, 0.0), (4.0, 0.0)], 2..3, ClipHalo::InteriorBand)]
#[case::ring(&[(-3.0, 0.0), (0.0, 0.0), (3.0, 0.0)], 1..2, ClipHalo::InteriorRing)]
#[case::margin_band(&[(-4.0, 0.0), (-2.0, 0.0), (0.0, 0.0), (-2.0, -2.0), (-2.0, 2.0)], 2..3, ClipHalo::MarginBand)]
#[case::subpixel_cap(&[(-3.75, 0.0), (-0.75, 0.0), (0.0, 0.0), (-2.0, -2.0), (-2.0, 2.0)], 2..3, ClipHalo::MarginBand)]
#[case::margin_ring(&[(-5.0, 0.0), (0.0, 0.0), (2.0, 0.0)], 1..2, ClipHalo::MarginRing)]
fn query_hover_preserves_the_halo_inside_a_displaced_map_clip(
    #[case] offsets: &[(f64, f64)],
    #[case] hidden: Range<usize>,
    #[case] expected: ClipHalo,
) {
    let zoom = 16.0_f64;
    let world_px = 256.0 * zoom.exp2();
    let cell_size = HaloStyle::new(&tpv_renderer::frame_style(zoom), 0.0).ring_radius;
    let screen_center = test_util::viewport_center();
    let clip_x = (((screen_center.x / cell_size).floor() + 0.5) * cell_size).round();
    let origin_x = f64::from(clip_x - screen_center.x);
    let origin_x = if matches!(expected, ClipHalo::MarginRing) {
        origin_x - f64::from(cell_size)
    } else {
        origin_x
    };
    let clip = egui::Rect::from_min_max(
        egui::pos2(clip_x, 0.0),
        egui::Pos2::ZERO + test_util::VIEWPORT,
    );
    let points = offsets
        .iter()
        .enumerate()
        .map(|(index, &(x, y))| {
            fixtures::nav_point(
                test_util::epoch() + TimeDelta::minutes(index as i64),
                Latitude::new(-y * 360.0 / world_px),
                Longitude::new(12.0 + (origin_x + x) * 360.0 / world_px),
                FixKind::Measured,
            )
        })
        .collect();
    let track = test_util::track0();
    let count = offsets.len();
    let built = test_util::track_built_from(points);
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![built])];
    let mut rendered = MapScene::of(files)
        .zoomed_to(zoom)
        .centred_on((0.0, 12.0))
        .clipped_to(clip)
        .draw_state(|state| {
            state.highlight.hover_match = Some(MatchHighlight::new(track, &(0..count)));
        })
        .overlays(|overlays| {
            overlays.query_matches = Some(QueryMatches {
                hidden: TrackRanges::from_iter([(track, vec![hidden])]),
                ..Default::default()
            });
        })
        .render();
    let color = gt_ui_theme::QUERY_MATCH_HOVER_HALO;
    let margin = matches!(expected, ClipHalo::MarginBand | ClipHalo::MarginRing);
    let visible = rendered
        .harness
        .inner
        .output()
        .shapes
        .iter()
        .filter(|shape| match (expected, &shape.shape) {
            (ClipHalo::InteriorRing | ClipHalo::MarginRing, egui::Shape::Circle(circle))
                if circle.stroke.color == color =>
            {
                assert_eq!(shape.clip_rect.min.x, clip.min.x);
                clip.contains(circle.center) != margin
            }
            (ClipHalo::InteriorBand | ClipHalo::MarginBand, egui::Shape::Path(path))
                if path.stroke.color == ColorMode::Solid(color) =>
            {
                assert_eq!(shape.clip_rect.min.x, clip.min.x);
                path.points
                    .iter()
                    .all(|&point| clip.contains(point) != margin)
            }
            _ => false,
        })
        .count();
    assert_eq!(visible, 1);
    if margin {
        let with_halo = rendered.harness.inner.render().unwrap();
        rendered.draw_state().highlight.hover_match = None;
        rendered.render_one_more_frame();
        let without_halo = rendered.harness.inner.render().unwrap();
        let side = egui::Rect::from_min_size(
            egui::pos2(clip_x + 2.0, screen_center.y - 1.0),
            egui::vec2(2.0, 2.0),
        );
        assert!(gt_test_utils::snapshot_harness::pixels_differ(
            &with_halo,
            &without_halo,
            side,
            rendered.harness.inner.ctx.pixels_per_point(),
        ));
    }
}

/// Everything visible: no marker type hidden, no display category masked, no
/// query run. A case that needs one of those derives a complete policy from
/// this value with the corresponding `with_*` method.
fn scope<'a>(
    files: &'a [LoadedFile],
    visibility: &'a TrackDataVisibility,
    filter: &'a GlobalFilter,
) -> MapPresence<'a> {
    MapEligibility::new(
        files,
        visibility,
        filter,
        None,
        &NO_GENERATED_MARKER_KIND_HIDDEN,
        &NO_EVENT_MARKER_PATH_HIDDEN,
    )
    .with_display_mask(DisplayMask::default())
}

/// Regression test: a point in a visible track must be hoverable.
#[test]
fn visible_tpv_point_is_hoverable() {
    let sp = tpv_spatial_point(0, 0, 0);
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![track_at(
        55.0, 12.0,
    )])];
    let vis = vis_all_visible();
    assert!(viewport::is_spatial_point_visible(
        &sp,
        scope(&files, &vis, &GlobalFilter::default())
    ));
}

/// Regression test: hiding the file must prevent hover on all its points.
#[test]
fn hidden_file_blocks_hover() {
    let sp = tpv_spatial_point(0, 0, 0);
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![track_at(
        55.0, 12.0,
    )])];
    let mut vis = vis_all_visible();
    vis.files[0].enabled = false;
    assert!(!viewport::is_spatial_point_visible(
        &sp,
        scope(&files, &vis, &GlobalFilter::default())
    ));
}

/// Regression test: hiding the track must prevent hover even when the file is visible.
#[test]
fn hidden_track_blocks_hover() {
    let sp = tpv_spatial_point(0, 0, 0);
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![track_at(
        55.0, 12.0,
    )])];
    let mut vis = vis_all_visible();
    vis.files[0].tracks[0].enabled = false;
    assert!(!viewport::is_spatial_point_visible(
        &sp,
        scope(&files, &vis, &GlobalFilter::default())
    ));
}

fn track_at(lat: f64, lon: f64) -> LoadedTrack {
    gt_test_utils::loaded_track_with_points(vec![gt_test_utils::fixtures::nav_point(
        chrono::Utc::now(),
        Latitude::new(lat),
        Longitude::new(lon),
        gt_types::fixtures::FixKind::Measured,
    )])
}

#[rstest::rstest]
#[case::gui_reveal(true)]
#[case::session_range(false)]
fn file_load_fit_precedence_depends_on_camera_source(#[case] second_file_is_framed: bool) {
    let first_latitude = 55.0;
    let second_latitude = 56.0;
    let first = gt_test_utils::loaded_file_with_tracks(vec![track_at(first_latitude, 12.0)]);
    let second = gt_test_utils::loaded_file_with_tracks(vec![track_at(second_latitude, 13.0)]);
    let mut map = MapScene::of(vec![first]).render_one_frame();
    let initial = map.framed().unwrap();
    assert!(initial.lat_max < second_latitude);
    map.add_file(second);
    let track = test_util::track0();
    if second_file_is_framed {
        map.overlays().reveal = Some(MatchRevealTarget::OneMatch {
            track,
            points: 0..1,
        });
    } else {
        map.overlays().presentation_camera = Some(PresentationCamera::TrackRange(
            MatchHighlight::new(track, &(0..1)),
        ));
    }
    map.render_one_more_frame();
    let bounds = map.framed().unwrap();
    let latitudes = bounds.lat_min..=bounds.lat_max;
    assert!(latitudes.contains(&first_latitude));
    assert_eq!(latitudes.contains(&second_latitude), second_file_is_framed);
}

/// Regression test: "zoom to fit" frames only the visible tracks. Hiding a
/// track must drop its corner from `compute_visible_bounding_box`.
#[test]
fn visible_bounding_box_excludes_hidden_tracks() {
    // Track 0 sits south-west, track 1 sits far north-east.
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![
        track_at(55.0, 12.0),
        track_at(56.0, 13.0),
    ])];
    let mut vis = TrackDataVisibility {
        files: vec![FileVisibility {
            enabled: true,
            tracks: vec![
                TrackVisibility::all_visible(),
                TrackVisibility::all_visible(),
            ],
        }],
    };

    // Everything visible: the box spans both tracks.
    let filter = GlobalFilter::default();
    let all_visible =
        viewport::compute_visible_bounding_box(&files, &vis, &filter, DisplayMask::default())
            .expect("visible data has a bbox");
    assert_eq!(
        all_visible,
        GeoBounds::from_positions([
            (Latitude::new(55.0), Longitude::new(12.0)),
            (Latitude::new(56.0), Longitude::new(13.0)),
        ])
        .expect("two positions")
    );

    // Hide the north-east track: its corner drops out of the box.
    vis.files[0].tracks[1].enabled = false;
    let only_first =
        viewport::compute_visible_bounding_box(&files, &vis, &filter, DisplayMask::default())
            .expect("track 0 still visible");
    assert_eq!(
        only_first,
        GeoBounds::single_position(Latitude::new(55.0), Longitude::new(12.0))
    );
}

/// Regression test: turning off the TPV layer must prevent hover on TPV points.
#[test]
fn hidden_tpv_layer_blocks_hover() {
    let sp = tpv_spatial_point(0, 0, 0);
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![track_at(
        55.0, 12.0,
    )])];
    let mut vis = vis_all_visible();
    vis.files[0].tracks[0].set_category_visible(DataCategory::Tpv, false);
    assert!(!viewport::is_spatial_point_visible(
        &sp,
        scope(&files, &vis, &GlobalFilter::default())
    ));
}

/// A masked display category must block hover exactly like the tree
/// toggle: hidden ink cannot be hit-tested.
#[test]
fn masked_track_points_block_hover() {
    let sp = tpv_spatial_point(0, 0, 0);
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![track_at(
        55.0, 12.0,
    )])];
    let vis = vis_all_visible();
    let mut mask = DisplayMask::default();
    mask.set_visible(DisplayCategory::TrackPoints, false);
    assert!(!viewport::is_spatial_point_visible(
        &sp,
        scope(&files, &vis, &GlobalFilter::default()).with_display_mask(mask)
    ));
}

/// The display mask gates each plan decision independently of the tree
/// toggles: tracks, track points, and satellite labels have their own
/// categories.
#[test]
fn map_frame_plan_respects_the_display_mask() {
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![track_at(
        55.0, 12.0,
    )])];
    let vis = vis_all_visible();
    let filter = GlobalFilter::default();
    let track = test_util::track0();

    let all_on = viewport::MapFramePlan::compute_for_test(
        &files,
        &vis,
        &filter,
        DisplayMask::default(),
        15.0,
    )
    .entry(track)
    .expect("track is in the plan");
    assert!(all_on.trackline());
    assert!(all_on.real_fixes());
    assert!(all_on.fade().is_some());
    assert!(all_on.sky_glyphs());
    assert!(all_on.ghost_fixes());

    let mut mask = DisplayMask::default();
    mask.set_visible(DisplayCategory::Tracks, false);
    mask.set_visible(DisplayCategory::TrackPoints, false);
    mask.set_visible(DisplayCategory::GhostFixes, false);
    mask.set_visible(DisplayCategory::SatelliteLabels, false);
    mask.set_visible(DisplayCategory::SkyGlyphs, false);
    mask.set_visible(DisplayCategory::QueryHighlights, false);
    let all_off = viewport::MapFramePlan::compute_for_test(&files, &vis, &filter, mask, 15.0)
        .entry(track)
        .expect("track is in the plan");
    assert!(!all_off.trackline());
    assert!(!all_off.real_fixes());
    assert!(all_off.fade().is_none());
    assert!(!all_off.ghost_fixes());
    assert!(all_off.draws_nothing());
}

#[rstest::rstest]
#[case::track_points(DisplayCategory::TrackPoints, true, false, true, true)]
#[case::ghost_fixes(DisplayCategory::GhostFixes, true, true, false, true)]
#[case::sky_glyphs(DisplayCategory::SkyGlyphs, true, true, true, false)]
fn masking_a_single_category_leaves_independent_categories_in_the_plan(
    #[case] masked: DisplayCategory,
    #[case] expected_trackline: bool,
    #[case] expected_fade: bool,
    #[case] expected_ghost: bool,
    #[case] expected_glyphs: bool,
) {
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![track_at(
        55.0, 12.0,
    )])];
    let vis = vis_all_visible();
    let filter = GlobalFilter::default();
    let track = test_util::track0();

    let mut mask = DisplayMask::default();
    mask.set_visible(masked, false);
    let entry = viewport::MapFramePlan::compute_for_test(&files, &vis, &filter, mask, 15.0)
        .entry(track)
        .expect("track is in the plan");
    assert_eq!(entry.trackline(), expected_trackline);
    assert_eq!(entry.fade().is_some(), expected_fade);
    assert_eq!(entry.ghost_fixes(), expected_ghost);
    assert_eq!(entry.sky_glyphs(), expected_glyphs);
}

#[test]
fn soloing_ghost_fixes_plans_ghost_fixes_without_standard_lines_or_icons() {
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![track_at(
        55.0, 12.0,
    )])];
    let vis = vis_all_visible();
    let filter = GlobalFilter::default();
    let track = test_util::track0();

    let mut mask = DisplayMask::default();
    mask.solo(DisplayCategory::GhostFixes);
    let entry = viewport::MapFramePlan::compute_for_test(&files, &vis, &filter, mask, 15.0)
        .entry(track)
        .expect("track is in the plan");
    assert!(!entry.draws_nothing());
    assert!(!entry.trackline());
    assert!(entry.fade().is_none());
    assert!(entry.ghost_fixes());
    assert!(entry.ghost_fade().is_some());
    assert!(!entry.sky_glyphs());
}

/// With every position-carrying category masked there is no visible
/// ink to frame, so zoom-to-fit must do nothing.
#[test]
fn fully_masked_map_has_no_bounding_box() {
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![track_at(
        55.0, 12.0,
    )])];
    let vis = vis_all_visible();
    let filter = GlobalFilter::default();
    let mut mask = DisplayMask::default();
    for category in [
        DisplayCategory::Tracks,
        DisplayCategory::TrackPoints,
        DisplayCategory::SatelliteLabels,
        DisplayCategory::CustomMarkers,
        DisplayCategory::GhostFixes,
        DisplayCategory::SkyGlyphs,
        DisplayCategory::QueryHighlights,
    ] {
        mask.set_visible(category, false);
    }
    assert_eq!(
        viewport::compute_visible_bounding_box(&files, &vis, &filter, mask),
        None
    );
    mask.set_visible(DisplayCategory::Tracks, true);
    assert!(viewport::compute_visible_bounding_box(&files, &vis, &filter, mask).is_some());
}

/// A fix at `time` with no heading and no satellite report.
pub(crate) fn nav_at(
    time: chrono::DateTime<chrono::Utc>,
    lat: f64,
    lon: f64,
) -> gt_types::NavPoint {
    gt_test_utils::fixtures::nav_point(
        time,
        Latitude::new(lat),
        Longitude::new(lon),
        gt_types::fixtures::FixKind::GhostWithoutHeading,
    )
}

/// Regression test: with a partially-overlapping track, points outside the
/// filter's time window must not be hoverable - they are hidden on the map,
/// so the hit-test must agree (otherwise filtered points stay clickable).
#[test]
fn time_filtered_point_is_not_hoverable() {
    let early = chrono::DateTime::from_timestamp(0, 0).expect("valid");
    let late = early + chrono::Duration::seconds(100);
    let track = LoadedTrack {
        metadata: TrackMetadata {
            time_range: TimeRange::new(early, late),
            ..gt_test_utils::empty_track_metadata()
        },
        ..gt_test_utils::loaded_track_with_points(vec![
            nav_at(early, 55.0, 12.0),
            nav_at(late, 55.0, 12.0),
        ])
    };
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![track])];
    let vis = vis_all_visible();
    // Start the window between the two points: the track still overlaps it,
    // but the early point falls outside.
    let filter =
        GlobalFilter::default().with_time_bounds(Some(early + chrono::Duration::seconds(50)), None);
    assert!(
        !viewport::is_spatial_point_visible(
            &tpv_spatial_point(0, 0, 0),
            scope(&files, &vis, &filter)
        ),
        "the pre-window point must not be hoverable"
    );
    assert!(
        viewport::is_spatial_point_visible(
            &tpv_spatial_point(0, 0, 1),
            scope(&files, &vis, &filter)
        ),
        "the in-window point must stay hoverable"
    );
}

/// Regression test: points a `keep`/`hide` query removed are not drawn, so
/// they must not be hoverable or clickable either.
#[test]
fn query_hidden_point_is_not_hoverable() {
    let now = chrono::DateTime::from_timestamp(0, 0).expect("valid");
    let track = gt_test_utils::loaded_track_with_points(vec![
        nav_at(now, 55.0, 12.0),
        nav_at(now, 55.0001, 12.0001),
    ]);
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![track])];
    let vis = vis_all_visible();
    let filter = GlobalFilter::default();
    // A range built from arguments, so the single-element `vec!` does not trip
    // clippy's `single_range_in_vec_init`.
    let rng = |start: usize, end: usize| start..end;
    let matches = QueryMatches {
        hidden: TrackRanges::from_iter([(test_util::track0(), vec![rng(0, 1)])]),
        ..QueryMatches::default()
    };
    assert!(
        !viewport::is_spatial_point_visible(
            &tpv_spatial_point(0, 0, 0),
            scope(&files, &vis, &filter).with_query_matches(Some(&matches))
        ),
        "the query-hidden point must not be hoverable"
    );
    assert!(
        viewport::is_spatial_point_visible(
            &tpv_spatial_point(0, 0, 1),
            scope(&files, &vis, &filter).with_query_matches(Some(&matches))
        ),
        "the point the query kept must stay hoverable"
    );
    assert!(
        viewport::is_spatial_point_visible(
            &tpv_spatial_point(0, 0, 0),
            scope(&files, &vis, &filter)
        ),
        "without a query run the point is hoverable"
    );
}

/// The hover must skip the hidden nearest point and return a visible one instead.
#[test]
fn hover_skips_hidden_nearest_and_finds_visible() {
    // Two overlapping SpatialPoints in the same Mercator position.
    // Track 0 is hidden, track 1 is visible.
    let hidden = SpatialPoint {
        merc: MercPoint { x: 0.5, y: 0.5 },
        file_index: FileIdx::new(0),
        track_index: TrackIdx::new(0),
        point_index: PointIdx::new(0),
        category: DataCategory::Tpv,
    };
    let visible = SpatialPoint {
        merc: MercPoint { x: 0.5, y: 0.5 },
        file_index: FileIdx::new(0),
        track_index: TrackIdx::new(1),
        point_index: PointIdx::new(0),
        category: DataCategory::Tpv,
    };
    let tree = rstar::RTree::bulk_load(vec![hidden, visible]);
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![
        track_at(55.0, 12.0),
        track_at(56.0, 13.0),
    ])];
    let mut first_disabled = TrackVisibility::all_visible();
    first_disabled.enabled = false;
    let vis = TrackDataVisibility {
        files: vec![FileVisibility {
            enabled: true,
            tracks: vec![first_disabled, TrackVisibility::all_visible()],
        }],
    };
    let filter = GlobalFilter::default();
    let found = tree
        .nearest_neighbor_iter([0.5_f64, 0.5_f64])
        .take_while(|sp| sp.distance_2(&[0.5, 0.5]) <= f64::MAX)
        .find(|sp| viewport::is_spatial_point_visible(sp, scope(&files, &vis, &filter)));
    assert!(found.is_some(), "should find the visible track");
    assert_eq!(
        found.unwrap().track_index,
        TrackIdx::new(1),
        "should return track 1, not the hidden track 0"
    );
}

/// After deleting a file, the global spatial index must be rebuilt so that
/// point indices from the old (deleted) file don't survive into the next frame
/// and cause out-of-bounds panics in the renderers.
#[test]
fn spatial_index_valid_after_file_deletion() {
    let all_points = gt_test_utils::nav_test_data(); // 1 200 points, all with headings
    let points_a: Vec<_> = all_points.iter().take(700).cloned().collect();
    let points_b: Vec<_> = all_points.iter().take(340).cloned().collect();

    let file_a =
        gt_test_utils::loaded_file_with_tracks(vec![test_util::track_built_from(points_a)]);
    let file_b =
        gt_test_utils::loaded_file_with_tracks(vec![test_util::track_built_from(points_b.clone())]);

    // Confirm the bug scenario: the stale tree (built before deletion) has
    // entries with `point_index` ≥ 340, which would be OOB for `file_b` alone.
    let files_initial = vec![
        file_a,
        gt_test_utils::loaded_file_with_tracks(vec![test_util::track_built_from(points_b)]),
    ];
    let stale_index = gt_track_builder::SpatialIndex::build(&files_initial);
    let files_after = vec![file_b];
    let stale_has_oob = stale_index.points().any(|sp| {
        let Some(file) = sp.file_index.get(&files_after) else {
            return true; // file index out of bounds → OOB
        };
        let Some(track) = sp.track_index.get(&file.tracks) else {
            return true; // track index out of bounds → OOB
        };
        let len = match sp.category {
            DataCategory::Tpv => track.points.len(),
            DataCategory::CustomMarker => track.custom_markers.len(),
            DataCategory::GeneratedMarker => track.generated_markers.len(),
            DataCategory::EventMarker => track.event_markers.len(),
            DataCategory::Track | DataCategory::SatelliteReport => return false,
        };
        sp.point_index.as_usize() >= len
    });
    assert!(
        stale_has_oob,
        "test setup: stale tree must have OOB entries"
    );

    // After calling `rebuild_spatial_index`, all entries must be in-bounds.
    let mut map = NavMap::new(egui::Context::default(), TileAccess::Offline);
    map.rebuild_spatial_index(&files_initial);
    map.rebuild_spatial_index(&files_after);

    assert!(
        map.all_tree_indices_valid(&files_after),
        "spatial index has stale entries after file deletion"
    );
}

/// A marker sitting closer to the cursor than any fix still leaves the nearest
/// fix in the fix slot: the hit test reads the fix tree for the fix slot and
/// the marker tree for the marker slots.
#[test]
fn hover_finds_the_nearest_fix_and_the_nearest_event_marker() {
    let start = chrono::DateTime::from_timestamp(0, 0).expect("valid timestamp");
    let marker_at = |seconds: i64, lon: f64| {
        EventMarker::new(
            start + chrono::Duration::seconds(seconds),
            "power/boot".to_owned(),
            None,
            Latitude::new(55.0),
            Longitude::new(lon),
        )
    };
    let mut track = gt_test_utils::loaded_track_with_points(vec![
        nav_at(start, 55.0, 12.0001),
        nav_at(start + chrono::Duration::seconds(1), 55.0, 12.001),
    ]);
    track.event_markers = vec![marker_at(0, 12.0), marker_at(1, 12.002)];
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![track])];
    let mut map = NavMap::new(egui::Context::default(), TileAccess::Offline);
    map.rebuild_spatial_index(&files);
    let vis = vis_all_visible();
    let filter = GlobalFilter::default();
    // The cursor sits on the first marker, 11 m from the first fix. The radius
    // holds every fixture point: 1e-4 of the world is 4 km at this latitude.
    let cursor = gt_types::mercator::normalize(Latitude::new(55.0), Longitude::new(12.0));
    let radius_merc_sq = 1e-8;

    let plan = viewport::MapFramePlan::compute_for_test(
        &files,
        &vis,
        &filter,
        DisplayMask::default(),
        15.0,
    );
    let hover = map.nearest_hover_candidates([cursor.x, cursor.y], radius_merc_sq, &plan);

    assert_eq!(
        hover
            .tpv_or_satellite_report()
            .map(|point| point.fix().expect("fix hover").point),
        Some(PointIdx::new(0))
    );
    assert_eq!(
        hover.event_marker().and_then(|point| match point {
            MapElementRef::EventMarker(reference) => {
                Some(PointIdx::new(reference.index.as_usize()))
            }
            MapElementRef::Fix(_)
            | MapElementRef::SatelliteReport(_)
            | MapElementRef::CustomMarker(_)
            | MapElementRef::GeneratedMarker(_) => None,
        }),
        Some(PointIdx::new(0))
    );
}

#[test]
fn hover_skips_a_hidden_nearest_fix_and_keeps_searching() {
    let start = chrono::DateTime::from_timestamp(0, 0).expect("valid timestamp");
    let track = gt_test_utils::loaded_track_with_points(vec![
        nav_at(start, 55.0, 12.0),
        nav_at(start + chrono::Duration::seconds(1), 55.0, 12.0001),
    ]);
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![track])];
    let mut map = NavMap::new(egui::Context::default(), TileAccess::Offline);
    map.rebuild_spatial_index(&files);
    let vis = vis_all_visible();
    let filter = GlobalFilter::default();
    let hidden_range = |start: usize, end: usize| start..end;
    let matches = QueryMatches {
        hidden: TrackRanges::from_iter([(test_util::track0(), vec![hidden_range(0, 1)])]),
        ..QueryMatches::default()
    };
    let plan = viewport::MapFramePlan::compute(
        viewport::MapFrameInputs::new(
            &files,
            &vis,
            &filter,
            Some(&matches),
            &NO_GENERATED_MARKER_KIND_HIDDEN,
            &NO_EVENT_MARKER_PATH_HIDDEN,
            DisplayMask::default(),
        ),
        15.0,
    );
    let cursor = gt_types::mercator::normalize(Latitude::new(55.0), Longitude::new(12.0));

    let hover = map.nearest_hover_candidates([cursor.x, cursor.y], 1e-8, &plan);

    assert_eq!(
        hover
            .tpv_or_satellite_report()
            .map(|point| point.fix().expect("fix hover").point),
        Some(PointIdx::new(1)),
        "a hidden nearest fix must not stop nearest-first candidate search",
    );
}

#[test]
fn hiding_ghost_fixes_prevents_hovering_ghost_points_while_keeping_real_points() {
    let t0 = chrono::DateTime::from_timestamp(0, 0).expect("valid timestamp");
    let real_fix_0 = gt_test_utils::fixtures::nav_point(
        t0,
        Latitude::new(55.0),
        Longitude::new(12.0),
        gt_types::fixtures::FixKind::Measured,
    );
    let mut ghost_fix = nav_at(t0 + chrono::Duration::seconds(1), 55.0, 12.05);
    ghost_fix.navigation.state = Some(SolutionState::NoFix);
    let real_fix_1 = gt_test_utils::fixtures::nav_point(
        t0 + chrono::Duration::seconds(2),
        Latitude::new(55.0),
        Longitude::new(12.1),
        gt_types::fixtures::FixKind::Measured,
    );
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![
        gt_test_utils::loaded_track_with_points(vec![real_fix_0, ghost_fix, real_fix_1]),
    ])];
    let mut map = NavMap::new(egui::Context::default(), TileAccess::Offline);
    map.rebuild_spatial_index(&files);
    let vis = vis_all_visible();
    let filter = GlobalFilter::default();

    let ghost_cursor = gt_types::mercator::normalize(Latitude::new(55.0), Longitude::new(12.05));
    let real_cursor = gt_types::mercator::normalize(Latitude::new(55.0), Longitude::new(12.0));
    let radius_merc_sq = 1e-10;

    let default_plan = viewport::MapFramePlan::compute_for_test(
        &files,
        &vis,
        &filter,
        DisplayMask::default(),
        15.0,
    );
    let hover_ghost = map.nearest_hover_candidates(
        [ghost_cursor.x, ghost_cursor.y],
        radius_merc_sq,
        &default_plan,
    );
    assert_eq!(
        hover_ghost
            .tpv_or_satellite_report()
            .map(|point| point.fix().expect("fix hover").point),
        Some(PointIdx::new(1))
    );

    let mut mask = DisplayMask::default();
    mask.set_visible(DisplayCategory::GhostFixes, false);
    let hidden_ghost_plan =
        viewport::MapFramePlan::compute_for_test(&files, &vis, &filter, mask, 15.0);
    let hover_ghost_hidden = map.nearest_hover_candidates(
        [ghost_cursor.x, ghost_cursor.y],
        radius_merc_sq,
        &hidden_ghost_plan,
    );
    assert_eq!(hover_ghost_hidden.tpv_or_satellite_report(), None);

    let hover_real = map.nearest_hover_candidates(
        [real_cursor.x, real_cursor.y],
        radius_merc_sq,
        &hidden_ghost_plan,
    );
    assert_eq!(
        hover_real
            .tpv_or_satellite_report()
            .map(|point| point.fix().expect("fix hover").point),
        Some(PointIdx::new(0))
    );
}

#[test]
fn start_flag_remains_anchored_on_first_fix_when_ghost_fixes_are_hidden() {
    let mut ghost_fix = nav_at(chrono::Utc::now(), 55.0, 12.0);
    ghost_fix.navigation.state = Some(SolutionState::NoFix);
    let real_fix = gt_test_utils::fixtures::nav_point(
        chrono::Utc::now() + chrono::Duration::seconds(1),
        Latitude::new(55.0),
        Longitude::new(12.001),
        gt_types::fixtures::FixKind::Measured,
    );
    let track = gt_test_utils::loaded_track_with_points(vec![ghost_fix, real_fix]);
    let placed = track.placed_points().expect("placed points");
    let ends = track_endpoint_renderer::DrawnTrackEnds::of(placed, &GlobalFilter::default())
        .expect("drawn track ends");
    assert_eq!(ends.first, 1);
    assert_eq!(ends.last, 1);
}

fn hover_ref(category: DataCategory) -> MapElementRef {
    test_util::point_ref(category, 0)
}

/// The click that opens the disambiguation popup also fires `clicked_elsewhere`
/// on the popup area, so only a later frame's click or Escape closes it.
#[rstest::rstest]
#[case::opening_click(true, true, false, false)]
#[case::later_click_outside(false, true, false, true)]
#[case::escape(false, false, true, true)]
#[case::still_hovering_it(false, false, false, false)]
fn the_disambiguation_popup_survives_the_frame_it_opened_on(
    #[case] just_opened: bool,
    #[case] clicked_elsewhere: bool,
    #[case] escape_pressed: bool,
    #[case] expected: bool,
) {
    let dismissal = DisambiguationDismissal {
        just_opened,
        clicked_elsewhere,
        escape_pressed,
    };
    assert_eq!(dismissal.closes_popup(), expected);
}

/// What the compound hover label leans on: the map keeps a recorded element's
/// own label unless the disambiguation popup owns the cursor area or several
/// elements were already under the pointer on the previous frame.
#[rstest::rstest]
#[case::one_candidate(false, 1, false)]
#[case::popup_open(true, 1, true)]
#[case::settled_multi_hover(false, 2, true)]
fn the_individual_hover_labels_yield_to_the_popup_and_to_a_previous_multi_hover(
    #[case] disambig_open: bool,
    #[case] previous_candidates: usize,
    #[case] expected: bool,
) {
    let visibility = TrackDataVisibility::from_loaded(&[]);
    let mut state = DrawState::default();
    let previous_hover = HoverCandidates::from_refs_for_test(
        [DataCategory::Tpv, DataCategory::EventMarker]
            .into_iter()
            .take(previous_candidates)
            .map(hover_ref),
    );

    let mut ctx = state.context(&[], &visibility);
    ctx.suppress_overlapping_hover_labels(disambig_open, previous_hover);

    assert_eq!(ctx.highlight.suppress_hover_labels, expected);
}

/// `candidate_label` for a GnssFixRegained marker with a known duration must
/// produce the same string as `generated_marker_header`, both surfaces share
/// the same text so the disambiguation popup and the compound hover label agree.
#[test]
fn candidate_label_generated_marker_matches_header() {
    use gt_types::{
        GeneratedMarker, GeneratedMarkerKind, Latitude, LoadedTrack, Longitude, mercator,
    };

    let now = chrono::Utc::now();
    let dur = chrono::Duration::milliseconds(12_300);
    let lat = Latitude::new(55.686_7);
    let lon = Longitude::new(12.563_8);
    let bb = GeoBounds::from_positions([
        (Latitude::new(55.67), Longitude::new(12.55)),
        (Latitude::new(55.69), Longitude::new(12.59)),
    ])
    .expect("two positions");
    let track = LoadedTrack {
        metadata: TrackMetadata {
            index: 0,
            duration: chrono::Duration::seconds(1),
            time_range: TimeRange::new(now, now + chrono::Duration::seconds(1)),
            has_custom_markers: false,
            tpv_count: 0,
            invalid_position_count: 0,
            satellite_report_count: 0,
            custom_marker_count: 0,
            generated_marker_count: 1,
            event_marker_count: 0,
            ..gt_test_utils::empty_track_metadata()
        },
        geometry: gt_types::TrackGeometry::Measured(gt_types::MeasuredTrackGeometry {
            resolved_positions: Vec::new(),
            bounding_box: bb,
            merc_bounds: MercBounds::from(bb),
            distance_km: uom::si::f64::Length::new::<uom::si::length::kilometer>(1.0),
            point_set_diameter_m: uom::si::f64::Length::new::<uom::si::length::meter>(10.0),
            segment_length_range: None,
        }),
        points: vec![],
        lod: gt_types::TrackLod::default(),
        sat_label_anchors: Vec::new(),
        custom_markers: vec![],
        generated_markers: vec![GeneratedMarker {
            time: now,
            kind: GeneratedMarkerKind::GnssFixRegained {
                fix_lost_duration: dur,
            },
            lat,
            lon,
            merc: mercator::normalize(lat, lon),
        }],
        event_markers: vec![],
        channels: vec![],
    };
    let file = LoadedFile {
        metadata: FileMetadata {
            filename: "test.gtd".to_string(),
            total_distance: TotalDistance::Measured(Length::new::<kilometer>(1.0)),
            total_duration: chrono::Duration::seconds(1),
            time_range: Some(TimeRange::new(now, now + chrono::Duration::seconds(1))),
            ..gt_test_utils::empty_file_metadata()
        },
        tracks: vec![track],
        event_marker_styles: FxHashMap::default(),
        orphaned_event_markers: vec![],
        source: gt_types::FileSource::GtdPath(PathBuf::from("test.gtd")),
        load_warnings: vec![],
    };

    let candidate = test_util::point_ref(DataCategory::GeneratedMarker, 0);
    let expected = crate::generated_marker_renderer::generated_marker_header(
        &GeneratedMarkerKind::GnssFixRegained {
            fix_lost_duration: dur,
        },
    );
    assert_eq!(
        hover_labels::candidate_label(candidate, &[file]),
        expected,
        "candidate_label must delegate to generated_marker_header"
    );
}

/// The run-wide map button frames the matched points alone, not every
/// recording the query ran over.
#[test]
fn matched_bounding_box_covers_only_the_drawn_matches() {
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![
        track_at(55.0, 12.0),
        track_at(56.0, 13.0),
    ])];
    let matches = test_util::a_run_drawing(TrackRef::new(FileIdx::new(0), TrackIdx::new(1)), 0..1);
    assert_eq!(
        viewport::matched_bounding_box(&files, &matches, &GlobalFilter::default()),
        Some(GeoBounds::single_position(
            Latitude::new(56.0),
            Longitude::new(13.0)
        ))
    );
    assert_eq!(
        viewport::matched_bounding_box(&files, &QueryMatches::default(), &GlobalFilter::default()),
        None
    );
}

/// Matches spread either side of the antimeridian are framed on the tight
/// arc they cover, 175.0° E to 180.5° E, whichever track the fold reads
/// first.
#[test]
fn matched_bounding_box_across_the_antimeridian_frames_the_arc_the_matches_cover() {
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![
        track_at(0.0, 175.0),
        track_at(0.0, 179.0),
        track_at(0.0, -179.5),
    ])];
    let rng = |start: usize, end: usize| start..end;
    let matches = QueryMatches {
        draws: vec![DrawLayer {
            color: 0,
            ranges: (0..files[0].tracks.len())
                .map(|ti| {
                    (
                        TrackRef::new(FileIdx::new(0), TrackIdx::new(ti)),
                        vec![rng(0, 1)],
                    )
                })
                .collect(),
        }],
        run: 1,
        ..QueryMatches::default()
    };

    let bounds = viewport::matched_bounding_box(&files, &matches, &GlobalFilter::default())
        .expect("all three tracks are loaded");
    let (_, center_lon) = bounds.center();
    assert!(
        (bounds.lon.span_degrees() - 5.5).abs() < 1e-9,
        "spans {}°",
        bounds.lon.span_degrees()
    );
    assert!(
        (center_lon.as_degrees() - 177.75).abs() < 1e-9,
        "centered on {}°",
        center_lon.as_degrees()
    );
}

/// A match row's own map button frames that match's points, not every match
/// the run drew.
#[test]
fn match_bounding_box_covers_one_match() {
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![track_at(
        55.0, 12.0,
    )])];
    let track = test_util::track0();
    assert_eq!(
        viewport::match_bounding_box(&files, track, &(0..1), &GlobalFilter::default()),
        Some(GeoBounds::single_position(
            Latitude::new(55.0),
            Longitude::new(12.0)
        ))
    );
    // A range reaching past the track frames nothing: its points are gone.
    assert_eq!(
        viewport::match_bounding_box(&files, track, &(0..10_000), &GlobalFilter::default()),
        None
    );
    let missing_file = TrackRef::new(FileIdx::new(9), TrackIdx::new(0));
    assert_eq!(
        viewport::match_bounding_box(&files, missing_file, &(0..1), &GlobalFilter::default()),
        None
    );
}

/// Every framing fold covers where the points are drawn. A dead-reckoned fix
/// the builder resolved between its neighbours is framed there, and not at the
/// coordinates the receiver wrote for it: here the null island, half a world
/// from the track.
#[test]
fn map_framing_covers_where_the_points_are_drawn() {
    let drawn = (Latitude::new(55.0), Longitude::new(12.0));
    let mut track = track_at(0.0, 0.0);
    // The receiver wrote the null island, the builder placed the fix at
    // `drawn`.
    track.geometry = gt_types::TrackGeometry::Measured(gt_types::MeasuredTrackGeometry {
        resolved_positions: track
            .points
            .iter()
            .map(|_| gt_types::ResolvedPosition::interpolated(drawn.0, drawn.1))
            .collect(),
        bounding_box: GeoBounds::single_position(drawn.0, drawn.1),
        merc_bounds: MercBounds::from(GeoBounds::single_position(drawn.0, drawn.1)),
        distance_km: Length::new::<kilometer>(0.0),
        point_set_diameter_m: Length::new::<meter>(0.0),
        segment_length_range: None,
    });
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![track])];
    let track_ref = test_util::track0();
    let expected = Some(GeoBounds::single_position(drawn.0, drawn.1));

    assert_eq!(
        viewport::compute_visible_bounding_box(
            &files,
            &vis_all_visible(),
            &GlobalFilter::default(),
            DisplayMask::default()
        ),
        expected,
        "zoom to fit"
    );
    assert_eq!(
        viewport::matched_bounding_box(
            &files,
            &test_util::a_run_drawing(track_ref, 0..1),
            &GlobalFilter::default()
        ),
        expected,
        "the run's map button"
    );
    assert_eq!(
        viewport::match_bounding_box(&files, track_ref, &(0..1), &GlobalFilter::default()),
        expected,
        "a match row's map button"
    );
}

/// A file whose only track has no geometry puts no ink on the map, so zoom to
/// fit has nothing to frame.
#[test]
fn a_file_whose_only_track_has_no_geometry_has_nothing_to_frame() {
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![
        gt_test_utils::loaded_track_with_points(
            gt_test_utils::fixtures::nav_points_without_a_valid_position(3),
        ),
    ])];

    assert_eq!(
        viewport::compute_visible_bounding_box(
            &files,
            &vis_all_visible(),
            &GlobalFilter::default(),
            DisplayMask::default()
        ),
        None
    );
}

/// The map culls tracks by their Mercator bounds. A track without geometry has
/// none: every drawing pass leaves it out, and the track beside it draws and
/// frames as usual.
#[test]
fn a_track_without_geometry_is_left_out_of_every_drawing_pass() {
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![
        track_at(55.0, 12.0),
        gt_test_utils::loaded_track_with_points(
            gt_test_utils::fixtures::nav_points_without_a_valid_position(3),
        ),
    ])];
    let map = MapScene::of(files)
        .size(egui::vec2(400.0, 400.0))
        .render_one_frame();

    let framed = map
        .framed()
        .expect("the map framed the track that has a geometry");
    assert!(
        framed.lat_min < 55.0
            && framed.lat_max > 55.0
            && framed.lon_min < 12.0
            && framed.lon_max > 12.0,
        "the framing covers the drawn track alone, got {framed:?}"
    );
}

/// The query window's map button frames the map on the matches, wherever the
/// camera stood before.
#[test]
fn revealing_matches_frames_the_map_on_them() {
    let files = vec![gt_test_utils::loaded_file_with_tracks(vec![
        track_at(55.0, 12.0),
        track_at(56.0, 13.0),
    ])];
    let matches = test_util::a_run_drawing(TrackRef::new(FileIdx::new(0), TrackIdx::new(1)), 0..1);
    let mut map = MapScene::of(files)
        .size(egui::vec2(400.0, 400.0))
        .overlays(|overlays| overlays.query_matches = Some(matches))
        .render_one_frame();

    let framed_all = map.framed().expect("the map framed the newly loaded file");
    assert!(
        framed_all.lon_min < 12.0 && framed_all.lon_max > 13.0,
        "loading frames both tracks, got {framed_all:?}"
    );

    map.overlays().reveal = Some(MatchRevealTarget::WholeRun);
    map.render_one_more_frame();
    let framed_matches = map.framed().expect("the map framed the matches");
    assert!(
        framed_matches.lon_min > 12.9
            && framed_matches.lon_max < 13.1
            && framed_matches.lat_min > 55.9
            && framed_matches.lat_max < 56.1,
        "the reveal frames the matched track alone, got {framed_matches:?}"
    );
}

#[test]
fn fitting_visible_data_includes_the_presented_range() {
    let files = test_util::a_recording_of(5, 0.02);
    let start_longitude = test_util::fix_position(&files, 0).1;
    let first_longitude = test_util::fix_position(&files, 2).1;
    let last_longitude = test_util::fix_position(&files, 3).1;
    let finish_longitude = test_util::fix_position(&files, 4).1;
    let map = MapScene::of(files)
        .draw_state(|draw| {
            draw.display_mask.solo(DisplayCategory::QueryHighlights);
            draw.highlight.presented_match =
                Some(MatchHighlight::new(test_util::track0(), &(2..4)));
        })
        .render_one_frame();
    let framed = map.framed().unwrap();
    let longitudes = framed.lon_min..=framed.lon_max;
    assert!(longitudes.contains(&first_longitude));
    assert!(longitudes.contains(&last_longitude));
    assert!(framed.lon_min > start_longitude);
    assert!(framed.lon_max < finish_longitude);
}

#[test]
fn solo_map_presentation_draws_only_the_selected_range() {
    let files = test_util::a_recording_of(5, 0.001);
    let range = MatchHighlight::new(test_util::track0(), &(2..4));
    let mut map = MapScene::of(files)
        .centred_on_fix(2)
        .zoomed_to(14.0)
        .draw_state(|draw| {
            draw.display_mask.solo(DisplayCategory::QueryHighlights);
            draw.highlight.presented_match = Some(range);
        })
        .render();
    let start_flag_drawn = |map: &RenderedMap| {
        let dark_mode = map.harness.inner.ctx.theme() == egui::Theme::Dark;
        let flag_color = gt_ui_theme::TRACK_START_FLAG.resolve(dark_mode);
        map.harness.inner.output().shapes.iter().any(|shape| {
            matches!(&shape.shape, egui::Shape::Mesh(mesh)
                if mesh.vertices.iter().any(|vertex| vertex.color == flag_color))
        })
    };
    assert!(!start_flag_drawn(&map));
    map.draw_state().highlight.hover_match = Some(range);
    map.render_one_more_frame();
    assert!(start_flag_drawn(&map));
}

static NO_EVENT_MARKER_PATH_HIDDEN: EventMarkerVisibility = EventMarkerVisibility::new();

static NO_GENERATED_MARKER_KIND_HIDDEN: GeneratedMarkerVisibility =
    GeneratedMarkerVisibility::new();
