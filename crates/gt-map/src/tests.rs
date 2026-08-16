use std::path::PathBuf;

use super::*;
use crate::hover_labels::candidate_label;
use gt_test_utils::nav_test_data;
use gt_types::{
    Coord, DataCategory, FileIdx, FileMetadata, LoadedFile, LoadedTrack, MercPoint, PointIdx, Rect,
    SpatialPoint, TimeRange, TrackIdx, TrackMetadata, merc_bounds_for_rect,
};
use gt_ui_types::{FileVisibility, TrackVisibility};
use uom::si::f64::Length;
use uom::si::length::{kilometer, meter};

fn make_file_from_points(points: Vec<gt_types::NavPoint>) -> LoadedFile {
    let now = chrono::Utc::now();
    let bb = Rect::new(
        Coord {
            x: 12.55f64,
            y: 55.67,
        },
        Coord {
            x: 12.59f64,
            y: 55.69,
        },
    );
    let n = points.len();
    let track = LoadedTrack {
        metadata: TrackMetadata {
            index: 0,
            distance_km: Length::new::<kilometer>(1.0),
            duration: chrono::Duration::seconds(n as i64),
            time_range: TimeRange::new(now, now + chrono::Duration::seconds(n as i64)),
            bounding_box: bb,
            merc_bounds: merc_bounds_for_rect(bb),
            point_set_diameter_m: Length::new::<meter>(100.0),
            has_custom_markers: false,
            tpv_count: n,
            satellite_report_count: 0,
            custom_marker_count: 0,
            generated_marker_count: 0,
            event_marker_count: 0,
            ..gt_test_utils::empty_track_metadata()
        },
        points,
        lod: gt_types::TrackLod::default(),
        sat_label_anchors: Vec::new(),
        custom_markers: vec![],
        generated_markers: vec![],
        event_markers: vec![],
        channels: vec![],
    };
    LoadedFile {
        metadata: FileMetadata {
            filename: format!("test_{n}.gtd"),
            total_distance_km: Length::new::<kilometer>(1.0),
            total_duration: chrono::Duration::seconds(n as i64),
            time_range: TimeRange::new(now, now + chrono::Duration::seconds(n as i64)),
            ..gt_test_utils::empty_file_metadata()
        },
        tracks: vec![track],
        event_marker_styles: std::collections::HashMap::new(),
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

fn vis_all_visible() -> TrackDataVisibility {
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
    let tpv = gt_types::TimePositionVelocity::builder()
        .time(gt_types::GpsTime::from_utc(chrono::Utc::now()))
        .lat(gt_types::Latitude::new(lat))
        .lon(gt_types::Longitude::new(lon))
        .build();
    LoadedTrack {
        metadata: gt_test_utils::empty_track_metadata(),
        points: vec![gt_types::NavPoint::new(tpv, None)],
        lod: gt_types::TrackLod::default(),
        sat_label_anchors: Vec::new(),
        custom_markers: vec![],
        generated_markers: vec![],
        event_markers: vec![],
        channels: vec![],
    }
}

fn file_with_tracks(tracks: Vec<LoadedTrack>) -> LoadedFile {
    LoadedFile {
        metadata: gt_test_utils::empty_file_metadata(),
        tracks,
        event_marker_styles: std::collections::HashMap::new(),
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

    // Everything visible: the box spans both tracks (min_lat, max_lat, min_lon, max_lon).
    let filter = GlobalFilter::default();
    let all_visible = compute_visible_bounding_box(&files, &vis, &filter, DisplayMask::default())
        .expect("visible data has a bbox");
    assert_eq!(all_visible, (55.0, 56.0, 12.0, 13.0));

    // Hide the north-east track: its corner drops out of the box.
    vis.files[0].tracks[1].enabled = false;
    let only_first = compute_visible_bounding_box(&files, &vis, &filter, DisplayMask::default())
        .expect("track 0 still visible");
    assert_eq!(only_first, (55.0, 55.0, 12.0, 12.0));
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
fn nav_at(time: chrono::DateTime<chrono::Utc>, lat: f64, lon: f64) -> gt_types::NavPoint {
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
        points: vec![nav_at(early, 55.0, 12.0), nav_at(late, 55.0, 12.0)],
        lod: gt_types::TrackLod::default(),
        sat_label_anchors: Vec::new(),
        custom_markers: vec![],
        generated_markers: vec![],
        event_markers: vec![],
        channels: vec![],
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
    let track = LoadedTrack {
        metadata: gt_test_utils::empty_track_metadata(),
        points: vec![nav_at(now, 55.0, 12.0), nav_at(now, 55.0001, 12.0001)],
        lod: gt_types::TrackLod::default(),
        sat_label_anchors: Vec::new(),
        custom_markers: vec![],
        generated_markers: vec![],
        event_markers: vec![],
        channels: vec![],
    };
    let files = vec![file_with_tracks(vec![track])];
    let vis = vis_all_visible();
    let filter = GlobalFilter::default();
    // A range built from arguments, so the single-element vec does not trip
    // clippy's `single_range_in_vec_init`.
    let rng = |start: usize, end: usize| start..end;
    let matches = QueryMatches {
        hidden: std::collections::HashMap::from([(
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
/// unless the disambiguation popup owns the cursor area, or the previous
/// frame already had several candidates under the pointer.
#[rstest::rstest]
#[case::one_candidate(false, 1, false)]
#[case::popup_open(true, 1, true)]
#[case::settled_multi_hover(false, 2, true)]
fn hover_labels_yield_to_the_popup_or_a_previous_multi_hover(
    #[case] disambig_open: bool,
    #[case] previous_candidates: usize,
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
    let bb = Rect::new(Coord { x: 12.55, y: 55.67 }, Coord { x: 12.59, y: 55.69 });
    let track = LoadedTrack {
        metadata: TrackMetadata {
            index: 0,
            distance_km: uom::si::f64::Length::new::<uom::si::length::kilometer>(1.0),
            duration: chrono::Duration::seconds(1),
            time_range: TimeRange::new(now, now + chrono::Duration::seconds(1)),
            bounding_box: bb,
            merc_bounds: merc_bounds_for_rect(bb),
            point_set_diameter_m: uom::si::f64::Length::new::<uom::si::length::meter>(10.0),
            has_custom_markers: false,
            tpv_count: 0,
            satellite_report_count: 0,
            custom_marker_count: 0,
            generated_marker_count: 1,
            event_marker_count: 0,
            ..gt_test_utils::empty_track_metadata()
        },
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
            total_distance_km: uom::si::f64::Length::new::<uom::si::length::kilometer>(1.0),
            total_duration: chrono::Duration::seconds(1),
            time_range: TimeRange::new(now, now + chrono::Duration::seconds(1)),
            ..gt_test_utils::empty_file_metadata()
        },
        tracks: vec![track],
        event_marker_styles: std::collections::HashMap::new(),
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
