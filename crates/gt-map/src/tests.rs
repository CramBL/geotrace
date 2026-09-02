use std::path::PathBuf;

use super::*;
use crate::hover_labels::candidate_label;
use crate::viewport::match_bounding_box;
use gt_test_utils::nav_test_data;
use gt_types::{
    DataCategory, FileIdx, FileMetadata, GeoBounds, Latitude, LoadedFile, LoadedTrack, Longitude,
    MercBounds, MercPoint, PointIdx, SpatialPoint, TimeRange, TotalDistance, TrackIdx,
    TrackMetadata,
};
use gt_ui_types::{DrawLayer, FileVisibility, TrackRanges, TrackVisibility};
use rustc_hash::FxHashMap;
use uom::si::f64::Length;
use uom::si::length::{kilometer, meter};

fn make_file_from_points(points: Vec<gt_types::NavPoint>) -> LoadedFile {
    let now = chrono::Utc::now();
    let n = points.len();
    let track = LoadedTrack {
        metadata: TrackMetadata {
            index: 0,
            duration: chrono::Duration::seconds(n as i64),
            time_range: TimeRange::new(now, now + chrono::Duration::seconds(n as i64)),
            has_custom_markers: false,
            tpv_count: n,
            invalid_position_count: 0,
            satellite_report_count: 0,
            custom_marker_count: 0,
            generated_marker_count: 0,
            event_marker_count: 0,
            ..gt_test_utils::empty_track_metadata()
        },
        ..gt_test_utils::loaded_track_with_points(points)
    };
    LoadedFile {
        metadata: FileMetadata {
            filename: format!("test_{n}.gtd"),
            total_distance: TotalDistance::Measured(Length::new::<kilometer>(1.0)),
            total_duration: chrono::Duration::seconds(n as i64),
            time_range: Some(TimeRange::new(
                now,
                now + chrono::Duration::seconds(n as i64),
            )),
            ..gt_test_utils::empty_file_metadata()
        },
        tracks: vec![track],
        event_marker_styles: FxHashMap::default(),
        orphaned_event_markers: vec![],
        source: gt_types::FileSource::GtdPath(PathBuf::from(format!("test_{n}.gtd"))),
        load_warnings: vec![],
    }
}

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

/// Everything visible: no display category masked, no query run. Cases that
/// need either override the field they are about - `MapScope { query_matches:
/// Some(&matches), ..scope(&files, &vis, &filter) }`.
fn scope<'a>(
    files: &'a [LoadedFile],
    visibility: &'a TrackDataVisibility,
    filter: &'a GlobalFilter,
) -> MapScope<'a> {
    MapScope {
        files,
        visibility,
        filter,
        display_mask: DisplayMask::default(),
        query_matches: None,
    }
}

/// Regression test: a point in a visible track must be hoverable.
#[test]
fn visible_tpv_point_is_hoverable() {
    let sp = tpv_spatial_point(0, 0, 0);
    let files = vec![file_with_tracks(vec![track_at(55.0, 12.0)])];
    let vis = vis_all_visible();
    assert!(is_spatial_point_visible(
        &sp,
        scope(&files, &vis, &GlobalFilter::default())
    ));
}

/// Regression test: hiding the file must prevent hover on all its points.
#[test]
fn hidden_file_blocks_hover() {
    let sp = tpv_spatial_point(0, 0, 0);
    let files = vec![file_with_tracks(vec![track_at(55.0, 12.0)])];
    let mut vis = vis_all_visible();
    vis.files[0].enabled = false;
    assert!(!is_spatial_point_visible(
        &sp,
        scope(&files, &vis, &GlobalFilter::default())
    ));
}

/// Regression test: hiding the track must prevent hover even when the file is visible.
#[test]
fn hidden_track_blocks_hover() {
    let sp = tpv_spatial_point(0, 0, 0);
    let files = vec![file_with_tracks(vec![track_at(55.0, 12.0)])];
    let mut vis = vis_all_visible();
    vis.files[0].tracks[0].enabled = false;
    assert!(!is_spatial_point_visible(
        &sp,
        scope(&files, &vis, &GlobalFilter::default())
    ));
}

fn track_at(lat: f64, lon: f64) -> LoadedTrack {
    track_over(vec![nav_at(chrono::Utc::now(), lat, lon)])
}

pub(crate) fn track_over(points: Vec<gt_types::NavPoint>) -> LoadedTrack {
    gt_test_utils::loaded_track_with_points(points)
}

pub(crate) fn file_with_tracks(tracks: Vec<LoadedTrack>) -> LoadedFile {
    LoadedFile {
        metadata: gt_test_utils::empty_file_metadata(),
        tracks,
        event_marker_styles: FxHashMap::default(),
        orphaned_event_markers: vec![],
        source: gt_types::FileSource::GtdPath(PathBuf::from("test.gtd")),
        load_warnings: vec![],
    }
}

/// Regression test: "zoom to fit" frames only the visible tracks. Hiding a
/// track must drop its corner from `compute_visible_bounding_box`.
#[test]
fn visible_bounding_box_excludes_hidden_tracks() {
    // Track 0 sits south-west, track 1 sits far north-east.
    let files = vec![file_with_tracks(vec![
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
    let all_visible = compute_visible_bounding_box(&files, &vis, &filter, DisplayMask::default())
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
    let only_first = compute_visible_bounding_box(&files, &vis, &filter, DisplayMask::default())
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
    let files = vec![file_with_tracks(vec![track_at(55.0, 12.0)])];
    let mut vis = vis_all_visible();
    vis.files[0].tracks[0].set_category_visible(DataCategory::Tpv, false);
    assert!(!is_spatial_point_visible(
        &sp,
        scope(&files, &vis, &GlobalFilter::default())
    ));
}

/// A masked display category must block hover exactly like the tree
/// toggle: hidden ink cannot be hit-tested.
#[test]
fn masked_track_points_block_hover() {
    let sp = tpv_spatial_point(0, 0, 0);
    let files = vec![file_with_tracks(vec![track_at(55.0, 12.0)])];
    let vis = vis_all_visible();
    let mut mask = DisplayMask::default();
    mask.set_visible(DisplayCategory::TrackPoints, false);
    assert!(!is_spatial_point_visible(
        &sp,
        MapScope {
            display_mask: mask,
            ..scope(&files, &vis, &GlobalFilter::default())
        }
    ));
}

/// The display mask gates each plan decision independently of the tree
/// toggles: tracks, track points, and satellite labels have their own
/// categories.
#[test]
fn track_plan_respects_the_display_mask() {
    let files = vec![file_with_tracks(vec![track_at(55.0, 12.0)])];
    let vis = vis_all_visible();
    let filter = GlobalFilter::default();
    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));

    let all_on = viewport::TrackPlan::compute(&files, &vis, &filter, DisplayMask::default(), 15.0)
        .entry(track)
        .expect("track is in the plan");
    assert!(all_on.trackline);
    assert!(all_on.fade.is_some());

    assert!(all_on.sky_glyphs);

    let mut mask = DisplayMask::default();
    mask.set_visible(DisplayCategory::Tracks, false);
    mask.set_visible(DisplayCategory::TrackPoints, false);
    mask.set_visible(DisplayCategory::SatelliteLabels, false);
    mask.set_visible(DisplayCategory::SkyGlyphs, false);
    let all_off = viewport::TrackPlan::compute(&files, &vis, &filter, mask, 15.0)
        .entry(track)
        .expect("track is in the plan");
    assert!(!all_off.trackline);
    assert!(all_off.fade.is_none());
    assert!(all_off.draws_nothing());

    // Track points masked alone: the line, the labels, and the sky
    // glyphs stay - they have their own categories.
    let mut mask = DisplayMask::default();
    mask.set_visible(DisplayCategory::TrackPoints, false);
    let points_off = viewport::TrackPlan::compute(&files, &vis, &filter, mask, 15.0)
        .entry(track)
        .expect("track is in the plan");
    assert!(points_off.trackline);
    assert!(points_off.fade.is_none());
    assert!(points_off.sat_labels);
    assert!(points_off.sky_glyphs);

    // Sky glyphs masked alone: everything else stays.
    let mut mask = DisplayMask::default();
    mask.set_visible(DisplayCategory::SkyGlyphs, false);
    let glyphs_off = viewport::TrackPlan::compute(&files, &vis, &filter, mask, 15.0)
        .entry(track)
        .expect("track is in the plan");
    assert!(!glyphs_off.sky_glyphs);
    assert!(glyphs_off.trackline);
    assert!(glyphs_off.fade.is_some());
}

/// With every position-carrying category masked there is no visible
/// ink to frame, so zoom-to-fit must do nothing.
#[test]
fn fully_masked_map_has_no_bounding_box() {
    let files = vec![file_with_tracks(vec![track_at(55.0, 12.0)])];
    let vis = vis_all_visible();
    let filter = GlobalFilter::default();
    let mut mask = DisplayMask::default();
    for category in [
        DisplayCategory::Tracks,
        DisplayCategory::TrackPoints,
        DisplayCategory::SatelliteLabels,
        DisplayCategory::CustomMarkers,
    ] {
        mask.set_visible(category, false);
    }
    assert_eq!(
        compute_visible_bounding_box(&files, &vis, &filter, mask),
        None
    );
    mask.set_visible(DisplayCategory::Tracks, true);
    assert!(compute_visible_bounding_box(&files, &vis, &filter, mask).is_some());
}

/// Builds a single-point [`NavPoint`] stamped at `time`.
pub(crate) fn nav_at(
    time: chrono::DateTime<chrono::Utc>,
    lat: f64,
    lon: f64,
) -> gt_types::NavPoint {
    let tpv = gt_types::TimePositionVelocity::builder()
        .time(gt_types::GpsTime::from_utc(time))
        .lat(gt_types::Latitude::new(lat))
        .lon(gt_types::Longitude::new(lon))
        .build();
    gt_types::NavPoint::new(tpv, None)
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
        ..track_over(vec![nav_at(early, 55.0, 12.0), nav_at(late, 55.0, 12.0)])
    };
    let files = vec![file_with_tracks(vec![track])];
    let vis = vis_all_visible();
    // Start the window between the two points: the track still overlaps it,
    // but the early point falls outside.
    let filter = GlobalFilter {
        time_start: Some(early + chrono::Duration::seconds(50)),
        ..GlobalFilter::default()
    };
    assert!(
        !is_spatial_point_visible(&tpv_spatial_point(0, 0, 0), scope(&files, &vis, &filter)),
        "the pre-window point must not be hoverable"
    );
    assert!(
        is_spatial_point_visible(&tpv_spatial_point(0, 0, 1), scope(&files, &vis, &filter)),
        "the in-window point must stay hoverable"
    );
}

/// Regression test: points a `keep`/`hide` query removed are not drawn, so
/// they must not be hoverable or clickable either.
#[test]
fn query_hidden_point_is_not_hoverable() {
    let now = chrono::DateTime::from_timestamp(0, 0).expect("valid");
    let track = track_over(vec![nav_at(now, 55.0, 12.0), nav_at(now, 55.0001, 12.0001)]);
    let files = vec![file_with_tracks(vec![track])];
    let vis = vis_all_visible();
    let filter = GlobalFilter::default();
    // A range built from arguments, so the single-element vec does not trip
    // clippy's `single_range_in_vec_init`.
    let rng = |start: usize, end: usize| start..end;
    let matches = QueryMatches {
        hidden: TrackRanges::from_iter([(
            TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
            vec![rng(0, 1)],
        )]),
        ..QueryMatches::default()
    };
    assert!(
        !is_spatial_point_visible(
            &tpv_spatial_point(0, 0, 0),
            MapScope {
                query_matches: Some(&matches),
                ..scope(&files, &vis, &filter)
            }
        ),
        "the query-hidden point must not be hoverable"
    );
    assert!(
        is_spatial_point_visible(
            &tpv_spatial_point(0, 0, 1),
            MapScope {
                query_matches: Some(&matches),
                ..scope(&files, &vis, &filter)
            }
        ),
        "the point the query kept must stay hoverable"
    );
    assert!(
        is_spatial_point_visible(&tpv_spatial_point(0, 0, 0), scope(&files, &vis, &filter)),
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
    let files = vec![file_with_tracks(vec![
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
        .find(|sp| is_spatial_point_visible(sp, scope(&files, &vis, &filter)));
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
    let all_points = nav_test_data(); // 1 200 points, all with headings
    let points_a: Vec<_> = all_points.iter().take(700).cloned().collect();
    let points_b: Vec<_> = all_points.iter().take(340).cloned().collect();

    let file_a = make_file_from_points(points_a);
    let file_b = make_file_from_points(points_b.clone());

    // Confirm the bug scenario: the stale tree (built before deletion) has
    // entries with point_index ≥ 340, which would be OOB for file_b alone.
    let files_initial = vec![file_a, make_file_from_points(points_b)];
    let stale_tree = gt_track_builder::build_global_tree(&files_initial);
    let files_after = vec![file_b];
    let stale_has_oob = stale_tree.iter().any(|sp| {
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

    // After calling rebuild_spatial_index, all entries must be in-bounds.
    let mut map = NavMap::new(egui::Context::default(), TileAccess::Offline);
    map.rebuild_spatial_index(&files_initial);
    map.rebuild_spatial_index(&files_after);

    assert!(
        map.all_tree_indices_valid(&files_after),
        "spatial index has stale entries after file deletion"
    );
}

#[test]
fn compound_label_guard_truth_table() {
    for (multi, disambig, suppress, expected) in [
        (true, false, false, false), // first frame, suppress not yet set
        (true, false, true, true),   // settled multi-hover
        (false, false, true, false), // single hover
        (true, true, true, false),   // disambiguation popup open
    ] {
        assert_eq!(
            should_show_compound_label(multi, disambig, suppress),
            expected,
            "multi={multi} disambig={disambig} suppress={suppress}"
        );
    }
}

fn hover_ref(category: DataCategory) -> DataPointRef {
    DataPointRef {
        track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        category,
        point_index: PointIdx::new(0),
    }
}

/// The element a hover or a click acts on is the fix whenever one is among the
/// candidates.
#[test]
fn the_primary_candidate_is_the_fix_when_one_is_present() {
    let tpv = hover_ref(DataCategory::Tpv);
    let marker = hover_ref(DataCategory::EventMarker);

    let marker_only = HoverCandidates {
        event_marker: Some(marker),
        ..HoverCandidates::default()
    };
    assert_eq!(marker_only.primary(), Some(marker));
    assert!(!marker_only.is_ambiguous());

    let both = HoverCandidates {
        tpv_or_satellite_report: Some(tpv),
        event_marker: Some(marker),
        ..HoverCandidates::default()
    };
    assert_eq!(both.primary(), Some(tpv));
    assert!(both.is_ambiguous());
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

/// What the compound hover label leans on: a renderer keeps its own label
/// unless the disambiguation popup owns the cursor area, the previous frame
/// already had several candidates under the pointer, or a log hexagon over the
/// fix took the pointer and listed that line itself.
#[rstest::rstest]
#[case::one_candidate(false, 1, false, false)]
#[case::popup_open(true, 1, false, true)]
#[case::settled_multi_hover(false, 2, false, true)]
#[case::a_log_hexagon_over_the_fix(false, 1, true, true)]
fn hover_labels_yield_to_the_popup_a_previous_multi_hover_or_a_log_hexagon(
    #[case] disambig_open: bool,
    #[case] previous_candidates: usize,
    #[case] log_glyph_hovered: bool,
    #[case] expected: bool,
) {
    let visibility = TrackDataVisibility::from_loaded(&[]);
    let mut state = DrawState::default();
    for category in [DataCategory::Tpv, DataCategory::EventMarker]
        .into_iter()
        .take(previous_candidates)
    {
        state
            .highlight
            .hover_candidates
            .keep_nearest(hover_ref(category));
    }
    if log_glyph_hovered {
        state.log_hover.glyph = Some(gt_ui_types::LogMatchGlyph {
            log: gt_ui_types::LoadedLogId::new(0),
            color: gt_ui_types::LogMatchColor::LiveFilter,
            entry_indices: vec![0],
        });
    }

    let mut ctx = state.context(&[], &visibility);
    ctx.suppress_overlapping_hover_labels(disambig_open);

    assert_eq!(ctx.highlight.suppress_hover_labels, expected);
}

/// candidate_label for a GnssFixRegained marker with a known duration must
/// produce the same string as generated_marker_header, both surfaces share
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

    let candidate = gt_ui_types::DataPointRef {
        track: gt_types::TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        category: DataCategory::GeneratedMarker,
        point_index: PointIdx::new(0),
    };
    let expected = crate::generated_marker_renderer::generated_marker_header(
        &GeneratedMarkerKind::GnssFixRegained {
            fix_lost_duration: dur,
        },
    );
    assert_eq!(
        candidate_label(candidate, &[file]),
        expected,
        "candidate_label must delegate to generated_marker_header"
    );
}

/// A draw layer covering the first point of `track`, as one completed run.
fn matches_of_run(run: u64, track: TrackRef) -> QueryMatches {
    // A range built from arguments, so the single-element vec does not trip
    // clippy's `single_range_in_vec_init`.
    let rng = |start: usize, end: usize| start..end;
    QueryMatches {
        draws: vec![DrawLayer {
            color: 0,
            ranges: TrackRanges::from_iter([(track, vec![rng(0, 1)])]),
        }],
        run,
        ..QueryMatches::default()
    }
}

/// The run-wide map button frames the matched points alone, not every
/// recording the query ran over.
#[test]
fn matched_bounding_box_covers_only_the_drawn_matches() {
    let files = vec![file_with_tracks(vec![
        track_at(55.0, 12.0),
        track_at(56.0, 13.0),
    ])];
    let matches = matches_of_run(1, TrackRef::new(FileIdx::new(0), TrackIdx::new(1)));
    assert_eq!(
        matched_bounding_box(&files, &matches, &GlobalFilter::default()),
        Some(GeoBounds::single_position(
            Latitude::new(56.0),
            Longitude::new(13.0)
        ))
    );
    assert_eq!(
        matched_bounding_box(&files, &QueryMatches::default(), &GlobalFilter::default()),
        None
    );
}

/// Matches spread either side of the antimeridian are framed on the tight
/// arc they cover, 175.0° E to 180.5° E, whichever track the fold reads
/// first.
#[test]
fn matched_bounding_box_across_the_antimeridian_frames_the_arc_the_matches_cover() {
    let files = vec![file_with_tracks(vec![
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

    let bounds = matched_bounding_box(&files, &matches, &GlobalFilter::default())
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
    let files = vec![file_with_tracks(vec![track_at(55.0, 12.0)])];
    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    assert_eq!(
        match_bounding_box(&files, track, &(0..1), &GlobalFilter::default()),
        Some(GeoBounds::single_position(
            Latitude::new(55.0),
            Longitude::new(12.0)
        ))
    );
    // A range reaching past the track frames nothing: its points are gone.
    assert_eq!(
        match_bounding_box(&files, track, &(0..10_000), &GlobalFilter::default()),
        None
    );
    let missing_file = TrackRef::new(FileIdx::new(9), TrackIdx::new(0));
    assert_eq!(
        match_bounding_box(&files, missing_file, &(0..1), &GlobalFilter::default()),
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
    let files = vec![file_with_tracks(vec![track])];
    let track_ref = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    let expected = Some(GeoBounds::single_position(drawn.0, drawn.1));

    assert_eq!(
        compute_visible_bounding_box(
            &files,
            &vis_all_visible(),
            &GlobalFilter::default(),
            DisplayMask::default()
        ),
        expected,
        "zoom to fit"
    );
    assert_eq!(
        matched_bounding_box(
            &files,
            &matches_of_run(1, track_ref),
            &GlobalFilter::default()
        ),
        expected,
        "the run's map button"
    );
    assert_eq!(
        match_bounding_box(&files, track_ref, &(0..1), &GlobalFilter::default()),
        expected,
        "a match row's map button"
    );
}

/// A file whose only track has no geometry puts no ink on the map, so zoom to
/// fit has nothing to frame.
#[test]
fn a_file_whose_only_track_has_no_geometry_has_nothing_to_frame() {
    let files = vec![file_with_tracks(vec![
        gt_test_utils::loaded_track_with_points(
            gt_test_utils::nav_points_without_a_valid_position(3),
        ),
    ])];

    assert_eq!(
        compute_visible_bounding_box(
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
    let files = vec![file_with_tracks(vec![
        track_at(55.0, 12.0),
        gt_test_utils::loaded_track_with_points(
            gt_test_utils::nav_points_without_a_valid_position(3),
        ),
    ])];
    let visibility = TrackDataVisibility::from_loaded(&files);
    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(400.0, 400.0))
        .ui_state(
            |ui, map: &mut Option<NavMap>| {
                let map =
                    map.get_or_insert_with(|| NavMap::new(ui.ctx().clone(), TileAccess::Offline));
                let mut state = DrawState::default();
                map.draw(ui, state.context(&files, &visibility));
            },
            None,
        );

    harness.step();

    let framed = harness
        .state()
        .as_ref()
        .and_then(NavMap::viewport_geo_bounds)
        .expect("the map framed the track that has a geometry");
    assert!(
        framed.lat_min < 55.0
            && framed.lat_max > 55.0
            && framed.lon_min < 12.0
            && framed.lon_max > 12.0,
        "the framing covers the drawn track alone, got {framed:?}"
    );
}

/// The map answers the query window's map button by framing the matches,
/// wherever the camera stood before.
#[test]
fn revealing_matches_frames_the_map_on_them() {
    let files = vec![file_with_tracks(vec![
        track_at(55.0, 12.0),
        track_at(56.0, 13.0),
    ])];
    let visibility = TrackDataVisibility::from_loaded(&files);
    let matches = matches_of_run(1, TrackRef::new(FileIdx::new(0), TrackIdx::new(1)));
    let reveal_requested = std::cell::RefCell::new(None);
    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(400.0, 400.0))
        .ui_state(
            |ui, map: &mut Option<NavMap>| {
                let map =
                    map.get_or_insert_with(|| NavMap::new(ui.ctx().clone(), TileAccess::Offline));
                let mut state = DrawState::default();
                map.draw(
                    ui,
                    MapDrawContext {
                        query_matches: Some(&matches),
                        reveal_query_matches: reveal_requested.borrow().clone(),
                        ..state.context(&files, &visibility)
                    },
                );
            },
            None,
        );

    harness.step();
    let framed_all = harness
        .state()
        .as_ref()
        .and_then(NavMap::viewport_geo_bounds)
        .expect("the map framed the newly loaded file");
    assert!(
        framed_all.lon_min < 12.0 && framed_all.lon_max > 13.0,
        "loading frames both tracks, got {framed_all:?}"
    );

    *reveal_requested.borrow_mut() = Some(MatchRevealTarget::WholeRun);
    harness.step();
    let framed_matches = harness
        .state()
        .as_ref()
        .and_then(NavMap::viewport_geo_bounds)
        .expect("the map framed the matches");
    assert!(
        framed_matches.lon_min > 12.9
            && framed_matches.lon_max < 13.1
            && framed_matches.lat_min > 55.9
            && framed_matches.lat_max < 56.1,
        "the reveal frames the matched track alone, got {framed_matches:?}"
    );
}
