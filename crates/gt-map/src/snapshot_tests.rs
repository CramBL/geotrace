//! Every map here draws a base layer from local files: the labelled
//! synthetic tiles, or the captured Mapbox tiles where the imagery under the
//! track is part of what the baseline shows.

use std::path::PathBuf;

use egui_kittest::kittest::Queryable as _;
use egui_phosphor::regular::CLOUD_LIGHTNING as ICON_CLOUD_LIGHTNING;

use super::*;
use crate::test_util::{self, MapScene};
use gt_types::mercator::MercPoint;
use gt_types::{DataCategory, DisplayMode, FileIdx, PointIdx, TrackIdx};
use gt_ui_types::{DisplayCategory, DisplayMask};
use rustc_hash::FxHashMap;

/// Where the map draws each fix of a fixture whose every fix records its
/// position.
fn recorded_positions(points: &[gt_types::NavPoint]) -> Vec<gt_types::ResolvedPosition> {
    points
        .iter()
        .filter_map(|point| point.tpv.position())
        .map(|(latitude, longitude)| gt_types::ResolvedPosition::measured(latitude, longitude))
        .collect()
}

/// The recordings the compound-label cases hover over, one per name, each a
/// copy of the marker fixture.
fn recordings_named(filenames: &[&str]) -> gt_loaded_files::LoadedFiles {
    let mut loaded = gt_loaded_files::LoadedFiles::new();
    for filename in filenames {
        let mut file = test_util::a_recording_with_every_marker_kind();
        file.metadata.filename = (*filename).to_owned();
        loaded.push(file, gt_loaded_files::FileHistory::None);
    }
    loaded
}

/// Two loaded files with distinct filenames, so the labels stating a
/// recording have something to distinguish.
fn two_recordings_loaded() -> gt_loaded_files::LoadedFiles {
    recordings_named(&["morning.gtd", "evening.gtd"])
}

/// Snapshot: the compound label of the elements one pointer reaches, a section
/// per element. The fix section shows the whole hover table, a marker section
/// its own kind and text, and the generated marker its fix-lost duration. With
/// two recordings loaded the fix section also states the recording the fix
/// came from.
#[rstest::rstest]
#[case::a_fix_and_two_markers(
    "multi_hover_stacked_label",
    &["walk.gtd"],
    HoverCandidates {
        tpv_or_satellite_report: Some(test_util::point_ref(DataCategory::Tpv, 0)),
        event_marker: Some(test_util::point_ref(DataCategory::EventMarker, 0)),
        custom_marker: Some(test_util::point_ref(DataCategory::CustomMarker, 0)),
        generated_marker: None,
    }
)]
#[case::a_fix_and_a_regained_fix_marker(
    "multi_hover_tpv_and_generated_marker",
    &["walk.gtd"],
    HoverCandidates {
        tpv_or_satellite_report: Some(test_util::point_ref(DataCategory::Tpv, 0)),
        generated_marker: Some(test_util::point_ref(DataCategory::GeneratedMarker, 0)),
        ..HoverCandidates::default()
    }
)]
#[case::two_recordings_loaded(
    "multi_hover_stacked_label_two_files",
    &["morning.gtd", "evening.gtd"],
    HoverCandidates {
        tpv_or_satellite_report: Some(test_util::point_ref_in(
            FileIdx::new(1),
            DataCategory::Tpv,
            0,
        )),
        event_marker: Some(test_util::point_ref(DataCategory::EventMarker, 0)),
        custom_marker: Some(test_util::point_ref(DataCategory::CustomMarker, 0)),
        generated_marker: None,
    }
)]
fn snap_multi_hover_stacked_label(
    #[case] name: &str,
    #[case] filenames: &[&str],
    #[case] candidates: HoverCandidates,
) {
    let loaded = recordings_named(filenames);

    let mut harness = test_util::harness_builder()
        .size(egui::vec2(400.0, 800.0))
        .ui(move |ui| {
            let names = RecordingNames::resolve(loaded.view(), "{filename}");
            let labels = RecordingLabels::new(loaded.files(), &names);
            hover_labels::draw_multi_hover_label_contents(ui, candidates, loaded.files(), labels);
        });

    harness.fit_contents();
    harness.snapshot(name);
}

/// The compound label states the recording of the fix it shows, which need
/// not be the file the markers stacked with it came from.
#[test]
fn multi_hover_names_the_hovered_fixs_recording() {
    let loaded = two_recordings_loaded();
    let candidates = HoverCandidates {
        tpv_or_satellite_report: Some(test_util::point_ref_in(
            FileIdx::new(1),
            DataCategory::Tpv,
            0,
        )),
        event_marker: Some(test_util::point_ref(DataCategory::EventMarker, 0)),
        ..HoverCandidates::default()
    };

    let mut harness = test_util::harness_builder()
        .size(egui::vec2(400.0, 800.0))
        .ui(move |ui| {
            let names = RecordingNames::resolve(loaded.view(), "{filename}");
            let labels = RecordingLabels::new(loaded.files(), &names);
            hover_labels::draw_multi_hover_label_contents(ui, candidates, loaded.files(), labels);
        });
    harness.run();

    assert!(harness.inner.query_by_label("evening.gtd").is_some());
    assert!(harness.inner.query_by_label("morning.gtd").is_none());
}

/// Snapshot: the disambiguation popup with large icons via LayoutJob. Calls
/// the real `draw_disambig_row` so the test stays in sync with the production
/// code. Verifies that the icon renders at a visually larger size than the
/// label text.
#[test]
fn snap_disambig_popup_big_icons() {
    let files = vec![test_util::a_recording_with_every_marker_kind()];
    let candidates = [
        Some(test_util::point_ref(DataCategory::Tpv, 0)),
        Some(test_util::point_ref(DataCategory::EventMarker, 0)),
        None,
        None,
    ];
    let sticky = Some(test_util::point_ref(DataCategory::Tpv, 0));

    let mut harness = test_util::harness_builder()
        .size(egui::vec2(300.0, 90.0))
        .ui(move |ui| {
            Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(200.0);
                for candidate in candidates.iter().flatten().copied() {
                    draw_disambig_row(ui, candidate, &files, sticky == Some(candidate));
                }
            });
        });

    harness.run();
    harness.snapshot("disambig_popup_big_icons");
}

/// The gaps between `ranges` within `0..len` - the points a `keep` query
/// hides.
fn complement(ranges: &[std::ops::Range<usize>], len: usize) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut cursor = 0;
    for r in ranges {
        if cursor < r.start {
            out.push(cursor..r.start);
        }
        cursor = r.end;
    }
    if cursor < len {
        out.push(cursor..len);
    }
    out
}

/// When a matches snapshot captures the map.
#[derive(Clone, Copy, PartialEq, Eq)]
enum MatchCapture {
    /// Once the halos and every load animation have settled. The matches carry
    /// no run number, so the reveal never fires.
    Settled,
    /// On the single frame a completed run's reveal fires, with the halos at
    /// their most inflated.
    RevealStart,
}

/// The fixture track under a completed run, drawn through the whole
/// `NavMap::draw` path. `draw` mode halos the matched stretches, `hide` drops
/// them and `keep` drops everything else. The map dims a stale run and still
/// draws it. The reveal row captures the frame the halos inflate on.
#[rstest::rstest]
#[case::halos(
    "query_match_halos",
    DisplayMode::Draw,
    false,
    MatchCapture::Settled,
    TileAccess::Captured(gt_test_utils::map_tile_capture_dir())
)]
#[case::stale_halos(
    "query_match_halos_stale",
    DisplayMode::Draw,
    true,
    MatchCapture::Settled,
    TileAccess::Synthetic
)]
#[case::keep_mode(
    "query_keep_mode",
    DisplayMode::Keep,
    false,
    MatchCapture::Settled,
    TileAccess::Synthetic
)]
#[case::hide_mode(
    "query_hide_mode",
    DisplayMode::Hide,
    false,
    MatchCapture::Settled,
    TileAccess::Synthetic
)]
#[case::reveal(
    "query_match_reveal",
    DisplayMode::Draw,
    false,
    MatchCapture::RevealStart,
    TileAccess::Synthetic
)]
fn snap_query_matches(
    #[case] name: &str,
    #[case] mode: DisplayMode,
    #[case] stale: bool,
    #[case] capture: MatchCapture,
    #[case] tile_access: TileAccess,
) {
    use gt_ui_types::{DrawLayer, QueryMatches, TrackRanges};

    let files = vec![test_util::a_recording_with_every_marker_kind()];
    let track = test_util::track0();
    let len = files
        .first()
        .and_then(|f| f.tracks.first())
        .map_or(0, |t| t.points.len());
    // Two multi-point stretches on different legs of the fixture loop,
    // plus a single-point match that must render as a ring.
    let ranges = vec![150..300, 700..701, 900..1000];
    let per_track = |rs: Vec<std::ops::Range<usize>>| TrackRanges::from_iter([(track, rs)]);
    let run = match capture {
        MatchCapture::Settled => 0,
        MatchCapture::RevealStart => 1,
    };
    let matches = match mode {
        DisplayMode::Draw => QueryMatches {
            draws: vec![DrawLayer {
                color: 0,
                ranges: per_track(ranges),
            }],
            stale,
            run,
            ..QueryMatches::default()
        },
        DisplayMode::Hide => QueryMatches {
            hidden: per_track(ranges),
            stale,
            run,
            ..QueryMatches::default()
        },
        DisplayMode::Keep => QueryMatches {
            hidden: per_track(complement(&ranges, len)),
            stale,
            run,
            ..QueryMatches::default()
        },
    };

    let on_captured_tiles = matches!(tile_access, TileAccess::Captured(_));
    let scene = MapScene::of(files)
        .tiles(tile_access)
        .overlays(|overlays| overlays.query_matches = Some(matches));
    let mut map = match capture {
        // The first frame zooms to fit the newly seen file. The rest let the
        // blink and fade animations settle before the snapshot.
        MatchCapture::Settled => scene.render(),
        // One frame exactly, so the reveal is captured on the frame it starts.
        MatchCapture::RevealStart => scene.render_one_frame(),
    };
    if on_captured_tiles {
        // One more frame off a fresh record, so the check covers only the
        // frame the snapshot captures.
        if let Some(map) = map.map_mut() {
            map.forget_missing_captured_tiles();
        }
        map.render_one_more_frame();
        let missing = map
            .map()
            .and_then(NavMap::missing_captured_tiles)
            .expect("the map draws the captured tiles");
        gt_test_utils::assert_map_tile_capture_is_complete(name, missing);
    }
    map.snapshot(name);
}

/// The interference overlay under the fixture track. At the zoom that
/// frames a 1 km track a single 22 km cell covers the viewport, so this
/// pins the fill and the draw order - track ink over cells.
#[rstest::rstest]
#[case::dark("jamming_overlay_dark", true, None)]
#[case::light("jamming_overlay_light", false, None)]
#[case::hover("jamming_overlay_hover", true, Some(egui::pos2(400.0, 300.0)))]
fn snapshot_jamming_overlay(
    #[case] name: &str,
    #[case] dark_mode: bool,
    #[case] hover: Option<egui::Pos2>,
) {
    let files = vec![test_util::a_recording_with_every_marker_kind()];

    let mut map = MapScene::of(files)
        .tiles(TileAccess::Synthetic)
        .theme(dark_mode)
        .overlays(|overlays| {
            overlays.jamming_dataset = Some(test_util::an_interference_ring_around(
                test_util::MARKER_POSITION_DEGREES,
            ));
        })
        .render();
    if let Some(pos) = hover {
        map.hover_at_and_settle(pos);
    }
    map.snapshot(name);
}

/// The levels the application lists, as the popup receives them.
fn snapshot_warning_levels() -> Vec<gt_ui_types::WarningLevelExplanation> {
    vec![
        gt_ui_types::WarningLevelExplanation {
            trigger: "Aircraft interference: ≥2% of aircraft in a crossed cell reported low \
                      navigation accuracy (gpsjam.org's yellow level)."
                .to_owned(),
            reference: gt_jam::reference::AIRCRAFT_INTERFERENCE,
        },
        gt_ui_types::WarningLevelExplanation {
            trigger: gt_ionex::text::DEVIATION_WARNING_TRIGGER.clone(),
            reference: gt_ionex::reference::IONOSPHERIC_TEC,
        },
    ]
}

/// One disturbed track, as the application hands it over: every metric that
/// reached its level over it, with the value it reached.
fn snapshot_track_warnings() -> Vec<gt_ui_types::TrackSpaceWeatherWarning> {
    vec![gt_ui_types::TrackSpaceWeatherWarning {
        track_label: "morning.gtd (track 2)".to_owned(),
        lines: vec![
            "Geomagnetic storm (≥5): Hp30 7.667, G3".to_owned(),
            "Aircraft interference (≥2%): up to 34.2% of aircraft in a crossed cell".to_owned(),
            "Solar flare (≥M1, sunlit): X5.8 at 2024-05-11 02:01 UTC, R3".to_owned(),
            "ΔTEC (< -30%): -73% from the 27-day median, intense ionospheric storm (W = -4), 22h, \
             after a G5 storm 9h before"
                .to_owned(),
            "TEC over track: 12–175 TECU".to_owned(),
        ],
        states_tec_deviation: true,
    }]
}

/// More disturbed tracks than the hover names, so the snapshot pins both the
/// tracks it lists and the count of those it leaves to the popup.
fn snapshot_many_track_warnings() -> Vec<gt_ui_types::TrackSpaceWeatherWarning> {
    (0..8)
        .map(|index| gt_ui_types::TrackSpaceWeatherWarning {
            track_label: format!("ride-{index}.gtd"),
            lines: vec![format!(
                "Geomagnetic storm (≥5): Kp {}.667, G3",
                5 + index % 3
            )],
            states_tec_deviation: false,
        })
        .collect()
}

/// How the glyph is interacted with before the snapshot is taken.
enum IndicatorInteraction {
    /// Hold the pointer on it, which opens its hover.
    Hover,
    /// Click it, which opens the levels popup under it.
    Click,
}

/// The warning indicator in the map's top-right corner, with the pointer on
/// it so the snapshot pins its place, its strength, and whatever its hover
/// holds. The idle case pins the faint glyph the map shows until a metric
/// warns, and the levels case the popup a click opens, which lists every
/// affected track over the same rows either way.
#[rstest::rstest]
#[case::warned(
    "space_weather_warning",
    snapshot_track_warnings(),
    IndicatorInteraction::Hover
)]
#[case::idle("space_weather_warning_idle", Vec::new(), IndicatorInteraction::Hover)]
#[case::many_tracks(
    "space_weather_warning_many_tracks",
    snapshot_many_track_warnings(),
    IndicatorInteraction::Hover
)]
#[case::levels(
    "space_weather_warning_levels",
    snapshot_many_track_warnings(),
    IndicatorInteraction::Click
)]
fn snapshot_space_weather_warning(
    #[case] name: &str,
    #[case] warning: Vec<gt_ui_types::TrackSpaceWeatherWarning>,
    #[case] interaction: IndicatorInteraction,
) {
    let files = vec![test_util::a_recording_with_every_marker_kind()];

    let mut map = MapScene::of(files)
        .tiles(TileAccess::Synthetic)
        .draw_state(|state| {
            state.space_weather_warnings = warning;
            state.space_weather_levels = snapshot_warning_levels();
        })
        .render();
    match interaction {
        IndicatorInteraction::Hover => {
            let glyph = map.harness.inner.get_by_label(ICON_CLOUD_LIGHTNING).rect();
            map.hover_at_and_settle(glyph.center());
        }
        IndicatorInteraction::Click => {
            map.harness.inner.get_by_label(ICON_CLOUD_LIGHTNING).click();
            map.harness.inner.run_steps(2);
        }
    }
    map.snapshot(name);
}

/// Zoom at which the whole world fits the snapshot canvas: the world spans
/// `256 * 2^zoom` pixels.
const WORLD_ZOOM: f64 = 1.5;

/// The TEC heatmap under the fixture track, at the 10 May 2024 storm's peak
/// hours. The whole world is in view, so one snapshot covers the ramp from the
/// quiet night side to the equatorial crests past 150 TECU.
#[rstest::rstest]
#[case::dark("tec_heatmap_dark", true, None)]
#[case::light("tec_heatmap_light", false, None)]
#[case::hover("tec_heatmap_hover", true, Some(egui::pos2(300.0, 380.0)))]
fn snapshot_tec_heatmap(
    #[case] name: &str,
    #[case] dark_mode: bool,
    #[case] hover: Option<egui::Pos2>,
) {
    let files = vec![test_util::a_recording_with_every_marker_kind()];
    let maps = gt_ionex::captured_maps(gt_ionex::STORM_CAPTURE).expect("the storm capture");
    let instant = chrono::NaiveDate::from_ymd_opt(2024, 5, 10)
        .and_then(|day| day.and_hms_opt(20, 0, 0))
        .map(|naive| naive.and_utc())
        .expect("an epoch of the captured day");

    let mut map = MapScene::of(files)
        .tiles(TileAccess::Synthetic)
        .theme(dark_mode)
        .zoomed_to(WORLD_ZOOM)
        .centred_on((0.0, 0.0))
        .draw_state(|state| {
            state
                .display_mask
                .set_visible(DisplayCategory::TecHeatmap, true);
            state.tec_instant =
                gt_ionex::TecInstantSelection::new(Some(instant), instant.date_naive());
            state.tec_snapshot = Some((maps, instant));
        })
        .render();
    if let Some(pos) = hover {
        map.hover_at_and_settle(pos);
    }
    map.snapshot(name);
}

/// Nudge north (smaller Mercator y) by roughly ten pixels at the
/// snapped-track snapshot tests' zoom, so the snapped line is drawn beside
/// the recorded one.
const SNAPPED_OFFSET_MERC_Y: f64 = -1.5e-6;

/// A snapped segment following the recorded points in `range`, nudged
/// north by [`SNAPPED_OFFSET_MERC_Y`].
fn snapped_segment(drawn: &[MercPoint], range: std::ops::Range<usize>) -> Vec<MercPoint> {
    drawn
        .get(range)
        .unwrap_or_default()
        .iter()
        .map(|p| MercPoint {
            x: p.x,
            y: p.y + SNAPPED_OFFSET_MERC_Y,
        })
        .collect()
}

/// `segments` as a geometry with no edge data, which is what the polyline
/// cases draw.
fn bare_polylines(segments: Vec<Vec<MercPoint>>) -> gt_ui_types::SnappedTrackGeometry {
    gt_ui_types::SnappedTrackGeometry {
        segments: segments
            .into_iter()
            .map(|points| gt_ui_types::SnappedSegment {
                points,
                recorded_points: Vec::new(),
                edge_spans: Vec::new(),
            })
            .collect(),
        edges: Vec::new(),
        whiskers: Vec::new(),
    }
}

/// Two snapped stretches with an unsnapped gap between them, which the map
/// draws as a route discontinuity.
fn two_snapped_stretches(drawn: &[MercPoint]) -> gt_ui_types::SnappedTrackGeometry {
    bare_polylines(vec![
        snapped_segment(drawn, 100..400),
        snapped_segment(drawn, 600..950),
    ])
}

/// One snapped stretch whose tail runs about five viewport widths east, so
/// most of it is provably off screen.
fn a_snapped_stretch_with_a_tail_past_the_viewport(
    drawn: &[MercPoint],
) -> gt_ui_types::SnappedTrackGeometry {
    /// Mercator step between the synthetic tail points.
    const TAIL_STEP_MERC_X: f64 = 2e-5;

    let mut segment = snapped_segment(drawn, 100..400);
    if let Some(&end) = segment.last() {
        segment.extend((1..=60).map(|i| MercPoint {
            x: end.x + f64::from(i) * TAIL_STEP_MERC_X,
            y: end.y,
        }));
    }
    bare_polylines(vec![segment])
}

/// Four snapped points packed about a tenth of a pixel apart at the fitted
/// zoom, north of the middle of the recorded track.
fn a_snapped_cluster_below_one_pixel(drawn: &[MercPoint]) -> gt_ui_types::SnappedTrackGeometry {
    /// Mercator spacing of the cluster's points, far below the sub-pixel
    /// merge threshold.
    const CLUSTER_STEP_MERC_X: f64 = 2e-8;

    /// Extra northward offset, so the dot is clearly separate from the
    /// recorded trackline.
    const CLUSTER_OFFSET_MERC_Y: f64 = -6e-6;

    let Some(base) = drawn.get(drawn.len() / 2) else {
        return bare_polylines(Vec::new());
    };
    bare_polylines(vec![
        (0..4)
            .map(|i| MercPoint {
                x: base.x + f64::from(i) * CLUSTER_STEP_MERC_X,
                y: base.y + CLUSTER_OFFSET_MERC_Y,
            })
            .collect(),
    ])
}

/// The named edge of [`test_util::a_snapped_edge_at`], running through the
/// middle of the recorded track's bounds, which is where the fit centres the
/// viewport.
fn a_named_snapped_edge_across_the_viewport(
    drawn: &[MercPoint],
) -> gt_ui_types::SnappedTrackGeometry {
    let (min, max) = drawn.iter().fold(
        ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN)),
        |(min, max), p| {
            (
                (min.0.min(p.x), min.1.min(p.y)),
                (max.0.max(p.x), max.1.max(p.y)),
            )
        },
    );
    test_util::a_snapped_edge_at(MercPoint {
        x: f64::midpoint(min.0, max.0),
        y: f64::midpoint(min.1, max.1),
    })
}

/// Eastward Mercator offset of the synthetic whisker tests' snapped
/// positions: ~9 m at the fixture latitude, so whiskers are clearly
/// longer than the strokes they connect.
const WHISKER_OFFSET_MERC_X: f64 = 4.0e-7;

/// Whisker anchors and the matching snapped polyline for a run over
/// the fixes drawn at `drawn`: every one snaps [`WHISKER_OFFSET_MERC_X`] east.
fn whisker_geometry(drawn: &[MercPoint]) -> gt_ui_types::SnappedTrackGeometry {
    let snapped: Vec<MercPoint> = drawn
        .iter()
        .map(|p| MercPoint {
            x: p.x + WHISKER_OFFSET_MERC_X,
            y: p.y,
        })
        .collect();
    gt_ui_types::SnappedTrackGeometry {
        segments: vec![gt_ui_types::SnappedSegment {
            points: snapped.clone(),
            recorded_points: (0..drawn.len()).map(PointIdx::new).collect(),
            edge_spans: Vec::new(),
        }],
        edges: Vec::new(),
        whiskers: drawn
            .iter()
            .zip(snapped)
            .enumerate()
            .map(|(i, (_, snapped))| gt_ui_types::WhiskerAnchor {
                point: PointIdx::new(i),
                snapped,
            })
            .collect(),
    }
}

/// A file whose single track spans only ~55 m, so zoom-to-fit lands
/// far above the whisker scale gate.
fn a_recording_of_a_short_walk() -> gt_types::LoadedFile {
    use gt_types::time_types::GpsTime;
    use gt_types::{
        FileMetadata, GeoBounds, Latitude, LoadedFile, LoadedTrack, Longitude, MercBounds,
        TimeRange, TrackMetadata,
    };

    let t0 = chrono::DateTime::from_timestamp(1_767_268_800, 0).unwrap_or_default();
    let points: Vec<gt_types::NavPoint> = (0..6)
        .map(|i| {
            let tpv = gt_types::TimePositionVelocity::builder()
                .time(GpsTime::from_utc(t0 + chrono::Duration::seconds(i)))
                .lat(Latitude::new(55.68 + i as f64 * 1.0e-4))
                .lon(Longitude::new(12.56))
                .build();
            gt_types::NavPoint::new(tpv, None)
        })
        .collect();
    let bb = GeoBounds::from_positions([
        (Latitude::new(55.68), Longitude::new(12.56)),
        (Latitude::new(55.6805), Longitude::new(12.56)),
    ])
    .expect("two positions");
    let n = points.len();
    let track = LoadedTrack {
        metadata: TrackMetadata {
            time_range: TimeRange::new(t0, t0 + chrono::Duration::seconds(n as i64)),
            tpv_count: n,
            invalid_position_count: 0,
            ..gt_test_utils::empty_track_metadata()
        },
        geometry: gt_types::TrackGeometry::Measured(gt_types::MeasuredTrackGeometry {
            resolved_positions: recorded_positions(&points),
            bounding_box: bb,
            merc_bounds: MercBounds::from(bb),
            distance_km: uom::si::f64::Length::new::<uom::si::length::kilometer>(0.0),
            point_set_diameter_m: uom::si::f64::Length::new::<uom::si::length::meter>(0.0),
            segment_length_range: None,
        }),
        ..gt_test_utils::loaded_track_with_points(points)
    };
    LoadedFile {
        metadata: FileMetadata {
            filename: "short_walk.gtd".to_string(),
            time_range: Some(TimeRange::new(t0, t0 + chrono::Duration::seconds(n as i64))),
            ..gt_test_utils::empty_file_metadata()
        },
        tracks: vec![track],
        event_marker_styles: FxHashMap::default(),
        orphaned_event_markers: vec![],
        source: gt_types::FileSource::GtdPath(PathBuf::from("short_walk.gtd")),
        load_warnings: vec![],
    }
}

/// The snapped track beside the recorded one: dashed translucent polylines
/// that never paint over the recorded ink, a gap where the route breaks, the
/// exact viewport edge where a stretch runs off screen, a dot where one packs
/// below a pixel, the matched edge's attributes under the pointer, and no
/// dashes at all while the category is hidden. The whisker rows sit either
/// side of the scale gate: over a 55 m track every snapped point draws its
/// error whisker, and over the kilometre-scale fixture none does.
#[rstest::rstest]
#[case::polylines(
    "snapped_track_polylines",
    test_util::a_recording_with_every_marker_kind(),
    None,
    None,
    two_snapped_stretches
)]
#[case::culled_tail(
    "snapped_track_culled_tail",
    test_util::a_recording_with_every_marker_kind(),
    None,
    None,
    a_snapped_stretch_with_a_tail_past_the_viewport
)]
#[case::collapsed_dot(
    "snapped_track_collapsed_dot",
    test_util::a_recording_with_every_marker_kind(),
    None,
    None,
    a_snapped_cluster_below_one_pixel
)]
#[case::hidden_by_the_display_mask(
    "snapped_track_hidden_by_display_mask",
    test_util::a_recording_with_every_marker_kind(),
    Some(DisplayCategory::SnappedTracks),
    None,
    two_snapped_stretches
)]
#[case::edge_hover(
    "snapped_track_edge_hover",
    test_util::a_recording_with_every_marker_kind(),
    None,
    Some(egui::pos2(400.0, 300.0)),
    a_named_snapped_edge_across_the_viewport
)]
#[case::whiskers_above_the_scale_gate(
    "snapped_track_whiskers",
    a_recording_of_a_short_walk(),
    None,
    None,
    whisker_geometry
)]
#[case::whiskers_below_the_scale_gate(
    "snapped_track_whiskers_below_gate",
    test_util::a_recording_with_every_marker_kind(),
    None,
    None,
    whisker_geometry
)]
fn snap_snapped_tracks(
    #[case] name: &str,
    #[case] file: gt_types::LoadedFile,
    #[case] hidden: Option<DisplayCategory>,
    #[case] hover: Option<egui::Pos2>,
    #[case] geometry_for: fn(&[MercPoint]) -> gt_ui_types::SnappedTrackGeometry,
) {
    let files = vec![file];
    let mut snapped = gt_ui_types::SnappedTracks::default();
    snapped.insert(
        test_util::track0(),
        std::sync::Arc::new(geometry_for(&test_util::drawn_positions(&files))),
    );
    let mut mask = DisplayMask::default();
    if let Some(category) = hidden {
        mask.set_visible(category, false);
    }

    let mut map = MapScene::of(files)
        .tiles(TileAccess::Synthetic)
        .draw_state(|state| state.display_mask = mask)
        .overlays(|overlays| overlays.snapped_tracks = Some(snapped))
        .render();
    if let Some(pos) = hover {
        map.hover_at_and_settle(pos);
    }
    map.snapshot(name);
}

/// Snapshot: the halo band for the match hovered in the query results
/// table - the highlight blue over the matched stretch, without any
/// `draw` layers underneath.
#[test]
fn snap_query_match_hover_halo() {
    let files = vec![test_util::a_recording_with_every_marker_kind()];

    let mut map = MapScene::of(files)
        .tiles(TileAccess::Synthetic)
        .draw_state(|state| {
            state.highlight.hover_match = Some(gt_ui_types::MatchHighlight::new(
                test_util::track0(),
                &(150..300),
            ));
        })
        .render();
    map.snapshot("query_match_hover_halo");
}

/// The log the hexagon snapshots draw the matches of: one line per entry, in
/// the shape a journald export writes them.
fn snapshot_log_source(entry_count: usize) -> gt_ui_types::LogMatchSource {
    let start = gt_test_utils::synthetic_log_start();
    let text: String = (0..entry_count)
        .map(|index| {
            let time = start + chrono::Duration::seconds(index as i64);
            format!(
                "{} navsyncd[770]: gnss fix acquired, {} satellites in view\n",
                time.format("%Y-%m-%d %H:%M:%S"),
                4 + index % 8
            )
        })
        .collect();
    gt_ui_types::LogMatchSource {
        id: gt_ui_types::LoadedLogId::new(0),
        parsed: std::sync::Arc::new(
            gt_logfile::parse_log(text.into(), start).expect("the fixture log parses"),
        ),
        display_name: None,
    }
}

/// One filter's layer: its matches take the entries of `source` in order, so
/// every hexagon stands for a line the tooltip can read back.
///
/// Every match is addressed to the first fix of the only track: these cases
/// place their hexagons by position, and no filter is active to read the
/// address.
fn log_layer(
    color: gt_ui_types::LogMatchColor,
    source: &gt_ui_types::LogMatchSource,
    positions: Vec<MercPoint>,
) -> gt_ui_types::LogMatchLayer {
    let fix = gt_types::FixRef::new(test_util::track0(), gt_types::PointIdx::new(0));
    gt_ui_types::LogMatchLayer {
        color,
        log: source.clone(),
        matches: positions
            .into_iter()
            .enumerate()
            .map(|(entry_index, merc)| gt_ui_types::LogMatch {
                merc,
                entry_index,
                fix,
            })
            .collect(),
    }
}

/// A filter's plain glyphs, a cluster large enough to state its count, a
/// second filter's colour beside the first, and the doubled outline of a
/// shared colour, all over one track.
fn every_hexagon_state(drawn: &[MercPoint]) -> gt_ui_types::LogMatches {
    let merc_at = |index: usize| {
        drawn
            .get(index)
            .copied()
            .unwrap_or(MercPoint { x: 0.5, y: 0.5 })
    };
    let spread = |every: usize, count: usize| {
        (0..count)
            .map(|step| merc_at(step.saturating_mul(every)))
            .collect::<Vec<_>>()
    };
    // Eight lines logged where the recording stood still: one cluster stating
    // what it collapsed.
    let clustered = vec![merc_at(70); 8];
    let source = snapshot_log_source(20);
    gt_ui_types::LogMatches::from_layers(vec![
        log_layer(
            gt_ui_types::LogMatchColor::LayerSlot {
                index: 0,
                shared: false,
            },
            &source,
            spread(40, 6),
        ),
        log_layer(
            gt_ui_types::LogMatchColor::LayerSlot {
                index: 1,
                shared: true,
            },
            &source,
            spread(53, 4),
        ),
        log_layer(gt_ui_types::LogMatchColor::LiveFilter, &source, clustered),
    ])
}

/// One filter that matched every line of the log, one line per fix.
fn one_layer_over_every_fix(drawn: &[MercPoint]) -> gt_ui_types::LogMatches {
    let source = snapshot_log_source(drawn.len());
    gt_ui_types::LogMatches::from_layers(vec![log_layer(
        gt_ui_types::LogMatchColor::LayerSlot {
            index: 0,
            shared: false,
        },
        &source,
        drawn.to_vec(),
    )])
}

/// Two filters over the same run of lines. The layer below counts three times
/// as many lines as the one on top, so a count escaping from under a covering
/// hexagon would be wider than the one that belongs there. The covering layer
/// matched only the first half, which leaves the layer below its own hexagons
/// and counts along the rest of the track.
fn two_overlapping_layers(drawn: &[MercPoint]) -> gt_ui_types::LogMatches {
    let source = snapshot_log_source(drawn.len() * 3);
    let covered = log_layer(
        gt_ui_types::LogMatchColor::LayerSlot {
            index: 0,
            shared: false,
        },
        &source,
        drawn.iter().flat_map(|&merc| [merc; 3]).collect(),
    );
    let covering = log_layer(
        gt_ui_types::LogMatchColor::LayerSlot {
            index: 1,
            shared: false,
        },
        &source,
        drawn
            .iter()
            .skip(2)
            .take(drawn.len() / 2)
            .copied()
            .collect(),
    );
    gt_ui_types::LogMatches::from_layers(vec![covered, covering])
}

/// The hexagons a log filter puts on the map, and the layer switching off
/// with its display category like every other kind of map ink.
#[rstest::rstest]
#[case::every_state("log_match_hexagons", every_hexagon_state, None)]
#[case::a_dense_track("log_matches_along_a_dense_track", one_layer_over_every_fix, None)]
#[case::overlapping_layers("log_matches_of_overlapping_layers", two_overlapping_layers, None)]
#[case::hidden_by_the_display_mask(
    "log_matches_hidden_by_display_mask",
    one_layer_over_every_fix,
    Some(DisplayCategory::LogMatches)
)]
fn snap_log_matches(
    #[case] name: &str,
    #[case] layers_for: fn(&[MercPoint]) -> gt_ui_types::LogMatches,
    #[case] hidden: Option<DisplayCategory>,
) {
    let files = vec![test_util::a_recording_with_every_marker_kind()];
    let log_matches = layers_for(&test_util::drawn_positions(&files));

    let mut map = MapScene::of(files)
        .tiles(TileAccess::Synthetic)
        .draw_state(|state| {
            state.log_matches = log_matches;
            if let Some(category) = hidden {
                state.display_mask.set_visible(category, false);
            }
        })
        .render();
    map.snapshot(name);
}

/// Entries the live-filter layer's cluster at the centre of the fixture stands
/// for: more than the tooltip writes out, leaving it a tail to state.
const HOVERED_CLUSTER_ENTRIES: usize = 8;

/// The map framed on the fixture recording, drawing the matches the caller
/// puts at its centre. That centre is where the map frames the recording, so
/// it lands at the centre of the viewport.
fn log_map_harness(
    matches_at_center: impl FnOnce(MercPoint) -> gt_ui_types::LogMatches,
) -> (crate::test_util::RenderedMap, MercPoint) {
    use gt_types::mercator;
    use gt_ui_types::TrackDataVisibility;

    let files = vec![test_util::a_recording_with_every_marker_kind()];
    let bounds = crate::viewport::compute_visible_bounding_box(
        &files,
        &TrackDataVisibility::from_loaded(&files),
        &gt_filter::GlobalFilter::default(),
        DisplayMask::default(),
    )
    .expect("the fixture recording has points");
    let (center_lat, center_lon) = bounds.center();
    let center = mercator::normalize(center_lat, center_lon);

    let map = MapScene::of(files)
        .tiles(TileAccess::Synthetic)
        .draw_state(|state| state.log_matches = matches_at_center(center))
        .render();
    (map, center)
}

/// The fixture the hover tests drive: a layer chip's three matches and the
/// live filter's eight, all at the point the map centres on. The cursor at the
/// centre of the canvas is then on the live filter's hexagon, with the chip's
/// underneath it.
fn log_hover_harness() -> crate::test_util::RenderedMap {
    use gt_ui_types::{LogMatchColor, LogMatches};

    let (mut map, _) = log_map_harness(|center| {
        let source = snapshot_log_source(HOVERED_CLUSTER_ENTRIES + 3);
        LogMatches::from_layers(vec![
            log_layer(
                LogMatchColor::LayerSlot {
                    index: 0,
                    shared: false,
                },
                &source,
                vec![center; 3],
            ),
            log_layer(
                LogMatchColor::LiveFilter,
                &source,
                vec![center; HOVERED_CLUSTER_ENTRIES],
            ),
        ])
    });
    map.hover_at_and_settle(test_util::viewport_center());
    map
}

/// The map rings the viewer's hovered row even where the filters selected
/// nothing: the row has a position wherever its line was recorded.
#[test]
fn a_hovered_viewer_row_is_ringed_on_the_map() {
    let (mut map, center) = log_map_harness(|_| gt_ui_types::LogMatches::default());
    let before = map
        .harness
        .inner
        .render()
        .expect("the harness renders a frame");

    map.draw_state().log_hover.row_position = Some(center);
    map.harness.run();

    let after = map
        .harness
        .inner
        .render()
        .expect("the harness renders a frame");
    let around_the_centre =
        egui::Rect::from_center_size(test_util::viewport_center(), egui::Vec2::splat(40.0));
    assert!(
        gt_test_utils::snapshot_harness::pixels_differ(
            &before,
            &after,
            around_the_centre,
            map.harness.inner.ctx.pixels_per_point()
        ),
        "the ring draws where the hovered row's line was recorded"
    );
}

/// The cursor picks the hexagon of the topmost layer it is on, and that
/// hexagon identifies the lines it stands for - what the viewer marks the rows
/// of.
#[test]
fn hovering_a_hexagon_names_the_lines_of_the_topmost_layer_it_is_on() {
    let map = log_hover_harness();

    let glyph = map
        .hovered_log_glyph()
        .expect("the cursor is on the centre hexagon");
    assert_eq!(glyph.color, gt_ui_types::LogMatchColor::LiveFilter);
    assert_eq!(
        glyph.entry_indices,
        (0..HOVERED_CLUSTER_ENTRIES).collect::<Vec<usize>>(),
        "the hexagon stands for every line its cluster collapsed"
    );
}

/// The name the layers of the tooltip test were built with, as a session of
/// two logs of one name gives it.
const TOOLTIP_LOG_NAME: &str = "navsyncd.log · walk.gtd";

/// The tooltip writes the name over the hexagon's lines while the layers were
/// built with one, and the lines alone while they were not.
#[rstest::rstest]
#[case::several_logs_loaded(Some(TOOLTIP_LOG_NAME.to_owned()), true)]
#[case::one_log_loaded(None, false)]
fn a_hexagon_tooltip_shows_the_name_its_layer_was_built_with(
    #[case] display_name: Option<String>,
    #[case] shown: bool,
) {
    let (mut map, _) = log_map_harness(|center| {
        let mut source = snapshot_log_source(HOVERED_CLUSTER_ENTRIES);
        source.display_name = display_name;
        gt_ui_types::LogMatches::from_layers(vec![log_layer(
            gt_ui_types::LogMatchColor::LiveFilter,
            &source,
            vec![center; HOVERED_CLUSTER_ENTRIES],
        )])
    });
    map.hover_at_and_settle(test_util::viewport_center());

    assert_eq!(
        map.harness.inner.query_by_label(TOOLTIP_LOG_NAME).is_some(),
        shown
    );
}

/// The cursor resting on a hexagon leaves the clicked glyph unset: a hover
/// lists the lines in a tooltip and marks their rows, and leaves the log the
/// viewer shows alone.
#[test]
fn hovering_a_hexagon_leaves_the_clicked_glyph_unset() {
    let map = log_hover_harness();

    assert_eq!(map.clicked_log_glyph(), None);
}

/// Clicking the hexagon under the cursor hands the viewer that hexagon's log
/// and its lines, which the viewer opens on.
#[test]
fn clicking_a_hexagon_hands_its_log_and_lines_to_the_viewer() {
    let mut map = log_hover_harness();

    map.click_at(test_util::viewport_center());

    let clicked = map
        .clicked_log_glyph()
        .expect("the click landed on the centre hexagon");
    assert_eq!(clicked.log, gt_ui_types::LoadedLogId::new(0));
    assert_eq!(
        clicked.entry_indices,
        (0..HOVERED_CLUSTER_ENTRIES).collect::<Vec<usize>>(),
        "every line the clicked hexagon collapsed"
    );
}

/// Snapshot: the hovered cluster ringed, over the lines it collapsed and the
/// count of the ones the tooltip left out.
#[test]
fn snap_log_match_hover() {
    let mut map = log_hover_harness();

    map.snapshot("log_match_hover");
}

/// Snapshot: the display mask removes the marker ink (custom, generated,
/// event) while the track, its icons, and the satellite labels stay.
/// Compare against the marker-bearing fixture in the other snapshots.
#[test]
fn snap_display_mask_hides_markers() {
    let files = vec![test_util::a_recording_with_every_marker_kind()];

    let mut map = MapScene::of(files)
        .tiles(TileAccess::Synthetic)
        .draw_state(|state| {
            for category in [
                DisplayCategory::CustomMarkers,
                DisplayCategory::GeneratedMarkers,
                DisplayCategory::EventMarkers,
            ] {
                state.display_mask.set_visible(category, false);
            }
        })
        .render();
    map.snapshot("display_mask_hides_markers");
}

/// Fix stride along the close-up snapshot's road. Eleven fixes at this stride
/// draw the line across two thirds of the 800 px canvas at the map's maximum
/// zoom, where a metre is 2.97 px.
const CLOSE_UP_STRIDE_M: f64 = 18.0;

/// How far the road bends north at its middle.
const CLOSE_UP_BEND_M: f64 = 12.0;

/// The horizontal accuracy each fix of the close-up snapshot reports, in
/// metres: the receiver loses accuracy through the middle of the road and
/// recovers by its end. The map draws them as circles 42 px to 95 px across,
/// and the widest reach over their neighbours' arrows.
const CLOSE_UP_ACCURACIES_M: [f32; 11] =
    [7.0, 8.0, 10.0, 13.0, 15.0, 16.0, 14.0, 11.0, 9.0, 8.0, 7.0];

/// Metres east and north of the first fix that the close-up snapshot's fix
/// `index` sits at.
fn close_up_offset_m(index: usize) -> (f64, f64) {
    let last = CLOSE_UP_ACCURACIES_M.len().saturating_sub(1) as f64;
    let bend_phase = std::f64::consts::PI * index as f64 / last;
    (
        index as f64 * CLOSE_UP_STRIDE_M,
        CLOSE_UP_BEND_M * bend_phase.sin(),
    )
}

/// A file whose single track walks [`CLOSE_UP_ACCURACIES_M`] along the bend of
/// [`close_up_offset_m`]. Every fix heads at the next one and reports eight
/// satellites: the map draws an arrow, an accuracy circle and a sky disc for
/// each of them.
fn make_accuracy_circle_walk_file() -> gt_types::LoadedFile {
    use gt_types::satellites::{Constellation, Satellite, Satellites};
    use gt_types::time_types::GpsTime;
    use gt_types::{
        FileMetadata, GeoBounds, Latitude, LoadedFile, LoadedTrack, Longitude, MercBounds,
        TimeRange, TrackMetadata,
    };
    use uom::si::angle::degree;
    use uom::si::f64::Angle;

    const FIRST_LAT_DEGREES: f64 = 55.6867;
    const FIRST_LON_DEGREES: f64 = 12.5638;
    const METERS_PER_LATITUDE_DEGREE: f64 = 111_320.0;

    let meters_per_longitude_degree =
        METERS_PER_LATITUDE_DEGREE * FIRST_LAT_DEGREES.to_radians().cos();
    let offsets: Vec<(f64, f64)> = (0..CLOSE_UP_ACCURACIES_M.len())
        .map(close_up_offset_m)
        .collect();
    let bearings: Vec<Angle> = offsets
        .iter()
        .zip(offsets.iter().skip(1))
        .map(|(&(east_a, north_a), &(east_b, north_b))| {
            Angle::new::<degree>((east_b - east_a).atan2(north_b - north_a).to_degrees())
        })
        .collect();

    let t0 = chrono::DateTime::from_timestamp(1_767_268_800, 0).unwrap_or_default();
    let satellites = Satellites::new(
        Some(GpsTime::from_utc(t0)),
        None,
        (1..=8)
            .map(|prn| {
                Satellite::new(
                    Constellation::Gps,
                    prn,
                    Some(15.0 + prn as f32 * 8.0),
                    Some(prn as f32 * 43.0),
                    Some(28.0 + prn as f32),
                    prn % 3 != 0,
                )
            })
            .collect(),
    );
    let points: Vec<gt_types::NavPoint> = CLOSE_UP_ACCURACIES_M
        .iter()
        .zip(&offsets)
        .enumerate()
        .map(|(i, (&eph_m, &(east_m, north_m)))| {
            // The last fix keeps the bearing it arrived on.
            let heading = bearings
                .get(i)
                .or_else(|| bearings.last())
                .copied()
                .unwrap_or_else(|| Angle::new::<degree>(0.0));
            let tpv = gt_types::TimePositionVelocity::builder()
                .time(GpsTime::from_utc(t0 + chrono::Duration::seconds(i as i64)))
                .lat(Latitude::new(
                    FIRST_LAT_DEGREES + north_m / METERS_PER_LATITUDE_DEGREE,
                ))
                .lon(Longitude::new(
                    FIRST_LON_DEGREES + east_m / meters_per_longitude_degree,
                ))
                .heading(heading)
                .eph_m(eph_m)
                .build();
            gt_types::NavPoint::new(tpv, Some(satellites.clone()))
        })
        .collect();
    let n = points.len();
    let positions = recorded_positions(&points);
    let bb = GeoBounds::from_positions(points.iter().filter_map(|point| point.tpv.position()))
        .expect("every fixture fix records its position");
    let geometry = gt_types::TrackGeometry::Measured(gt_types::MeasuredTrackGeometry {
        resolved_positions: positions,
        bounding_box: bb,
        merc_bounds: MercBounds::from(bb),
        distance_km: uom::si::f64::Length::new::<uom::si::length::kilometer>(0.18),
        point_set_diameter_m: uom::si::f64::Length::new::<uom::si::length::meter>(180.0),
        segment_length_range: None,
    });
    let sat_label_anchors = geometry
        .measured()
        .and_then(|measured| gt_types::PlacedPoints::new(&points, &measured.resolved_positions))
        .map_or_else(Vec::new, gt_track_builder::build_sat_label_anchors);
    let track = LoadedTrack {
        metadata: TrackMetadata {
            time_range: TimeRange::new(t0, t0 + chrono::Duration::seconds(n as i64)),
            tpv_count: n,
            invalid_position_count: 0,
            satellite_report_count: n,
            ..gt_test_utils::empty_track_metadata()
        },
        geometry,
        sat_label_anchors,
        ..gt_test_utils::loaded_track_with_points(points)
    };
    LoadedFile {
        metadata: FileMetadata {
            filename: "accuracy_circles.gtd".to_string(),
            time_range: Some(TimeRange::new(t0, t0 + chrono::Duration::seconds(n as i64))),
            ..gt_test_utils::empty_file_metadata()
        },
        tracks: vec![track],
        event_marker_styles: FxHashMap::default(),
        orphaned_event_markers: vec![],
        source: gt_types::FileSource::GtdPath(PathBuf::from("accuracy_circles.gtd")),
        load_warnings: vec![],
    }
}

/// Snapshot: a road of eleven fixes at the map's maximum zoom, each arrow
/// pointing along the road and sitting on its own accuracy circle. The widest
/// circles reach over their neighbours' arrows, which draw on top of them.
/// Every circle draws above the sky discs, with its fill and stroke alphas
/// letting the tiles through.
#[test]
fn snap_accuracy_circles_close_up() {
    let files = vec![make_accuracy_circle_walk_file()];

    let mut map = MapScene::of(files)
        .tiles(TileAccess::Synthetic)
        .draw_state(|state| state.sky_glyph_variant = gt_ui_types::SkyGlyphVariant::Disc)
        .render();
    map.snapshot("accuracy_circles_close_up");
}

/// Snapshot: with every category except sky glyphs hidden, the glyphs are
/// the only ink left - so their own category keeps drawing them even when
/// the trackline, points, and labels are all off. Run for each variant so
/// both the ring and the disc are exercised through the full map path.
#[rstest::rstest]
#[case::ring("sky_glyphs_only_ring", gt_ui_types::SkyGlyphVariant::Ring)]
#[case::disc("sky_glyphs_only_disc", gt_ui_types::SkyGlyphVariant::Disc)]
fn snap_sky_glyphs_only(#[case] name: &str, #[case] variant: gt_ui_types::SkyGlyphVariant) {
    let files = vec![test_util::a_recording_with_every_marker_kind()];

    let mut map = MapScene::of(files)
        .tiles(TileAccess::Synthetic)
        .draw_state(|state| {
            for category in [
                DisplayCategory::Tracks,
                DisplayCategory::TrackPoints,
                DisplayCategory::SatelliteLabels,
                DisplayCategory::CustomMarkers,
                DisplayCategory::GeneratedMarkers,
                DisplayCategory::EventMarkers,
            ] {
                state.display_mask.set_visible(category, false);
            }
            state.sky_glyph_variant = variant;
        })
        .render();
    map.snapshot(name);
}

/// Snapshot: hovering the time-series plot draws the detailed sky disc at
/// the hovered sample's map point - even with the sky glyphs overlay hidden,
/// since the plot-hover disc is a focus indicator, not part of the
/// overlay. The ring around the point is the existing cross-highlight.
#[test]
fn snap_plot_hover_sky_disc() {
    let files = vec![test_util::a_recording_with_every_marker_kind()];
    // A mid-track point that carries a satellite report in the fixture.
    let hovered = (FileIdx::new(0), TrackIdx::new(0), PointIdx::new(50));

    let mut map = MapScene::of(files)
        .tiles(TileAccess::Synthetic)
        .draw_state(|state| {
            // Overlay off, so the only disc on the map is the plot-hover one.
            state
                .display_mask
                .set_visible(DisplayCategory::SkyGlyphs, false);
            state.highlight.plot_hover_point = Some(hovered);
        })
        .render();
    map.snapshot("plot_hover_sky_disc");
}

/// Snapshot: the clicked-point window itself - the resizable frame, the
/// sky plot pinned beside the satellite tables, and the deselect hint on
/// the window floor. Guards the whole composition, not just the body.
#[test]
fn snap_sticky_point_window() {
    let files = vec![test_util::a_recording_with_every_marker_kind()];
    // A mid-track point carrying a multi-constellation satellite report.
    let clicked = test_util::point_ref(DataCategory::Tpv, 50);

    let mut map = MapScene::of(files)
        .tiles(TileAccess::Synthetic)
        .size(egui::vec2(900.0, 700.0))
        .draw_state(|state| state.highlight.sticky = Some(clicked))
        .render();
    map.snapshot("sticky_point_window");
}

/// Wheel points the scroll test sends over the sky column: several rows of the
/// fix metrics.
const POINT_WINDOW_WHEEL_POINTS: f32 = 200.0;

/// The point window's body has no scroll of its own: the wheel over the sky
/// column moves that column inside its own scroll area, and the title bar
/// renders the same before and after. Both layouts of the body are covered,
/// the plot beside the satellite tables and the plot stacked above them.
#[rstest::rstest]
#[case::side_by_side(egui::vec2(700.0, 420.0))]
#[case::stacked(egui::vec2(360.0, 420.0))]
fn scrolling_the_point_window_leaves_the_title_bar_untouched(#[case] viewport: egui::Vec2) {
    use gt_test_utils::HarnessInteraction as _;

    let files = vec![test_util::a_recording_with_every_marker_kind()];
    let clicked = test_util::point_ref(DataCategory::Tpv, 50);

    let mut map = MapScene::of(files)
        .tiles(TileAccess::Synthetic)
        .size(viewport)
        .draw_state(|state| state.highlight.sticky = Some(clicked))
        .render();
    let harness = &mut map.harness;

    let window = harness
        .inner
        .ctx
        .memory(|memory| memory.area_rect(egui::Id::new(("sticky_popup", clicked))))
        .expect("the point window is shown");
    let sky_before = harness.inner.get_by_label("Sky").rect();
    // From the window's top edge to halfway to the body's first row, clear of
    // the fade egui paints at the scrolled edge of a scroll area.
    let title_bar = egui::Rect::from_min_max(
        window.min,
        egui::pos2(window.max.x, (window.top() + sky_before.top()) / 2.0),
    );
    let before = harness.inner.render().expect("the harness renders a frame");

    harness.inner.scroll_wheel_at(
        egui::pos2(sky_before.center().x, window.center().y),
        -POINT_WINDOW_WHEEL_POINTS,
        4,
    );

    let sky_after = harness.inner.get_by_label("Sky").rect();
    assert!(
        sky_after.top() < sky_before.top(),
        "the wheel must scroll the sky column, or this proves nothing"
    );
    let after = harness.inner.render().expect("the harness renders a frame");
    assert!(
        !gt_test_utils::snapshot_harness::pixels_differ(
            &before,
            &after,
            title_bar,
            harness.inner.ctx.pixels_per_point()
        ),
        "the scrolled content must stay below the title bar"
    );
}

/// The point window's open-trails button has to travel the whole way out
/// of `draw`: through the window body, into a [`SkyTrailsRequest`] carrying
/// the clicked point's instant, and out as a [`MapAction`]. The
/// widget-level test one layer down cannot see this wiring, so a dropped
/// return value here would leave the button a silent no-op.
#[test]
fn the_point_window_button_returns_a_timed_sky_trails_action() {
    use egui_kittest::kittest::Queryable as _;

    let files = vec![test_util::a_recording_with_every_marker_kind()];
    let clicked = test_util::point_ref(DataCategory::Tpv, 50);
    let point_time = files
        .first()
        .and_then(|f| f.tracks.first())
        .and_then(|t| t.points.get(50))
        .map(|p| p.tpv.time())
        .expect("the fixture has a point 50");

    let mut map = MapScene::of(files)
        .tiles(TileAccess::Synthetic)
        .size(egui::vec2(900.0, 700.0))
        .draw_state(|state| state.highlight.sticky = Some(clicked))
        .render();
    assert!(
        map.returned_action().is_none(),
        "nothing requested before the click"
    );

    map.harness
        .inner
        .get_by_label(egui_phosphor::regular::ARROW_SQUARE_OUT)
        .click();
    map.harness.inner.run_steps(2);

    assert_eq!(
        map.returned_action(),
        Some(MapAction::ShowSkyTrails(
            gt_ui_types::SkyTrailsRequest::at_instant(clicked.track, point_time)
        ))
    );
}

/// The point layout - and with it the resizable frame - covers both
/// categories that render the sky plot beside the satellite tables. A
/// satellite-report popup carries the same 40-satellite content as a fix,
/// so it must not fall back to the cramped auto-sized frame.
#[rstest::rstest]
#[case::tpv(gt_types::DataCategory::Tpv, true)]
#[case::satellite_report(gt_types::DataCategory::SatelliteReport, true)]
#[case::custom_marker(gt_types::DataCategory::CustomMarker, false)]
#[case::generated_marker(gt_types::DataCategory::GeneratedMarker, false)]
#[case::event_marker(gt_types::DataCategory::EventMarker, false)]
#[case::track(gt_types::DataCategory::Track, false)]
fn point_layout_covers_the_satellite_bearing_categories(
    #[case] category: gt_types::DataCategory,
    #[case] expected: bool,
) {
    assert_eq!(super::sticky_uses_point_layout(category), expected);
}

/// A copy of the snapshot fixture whose custom marker carries a label far
/// longer than any of the audit viewports fits.
fn file_with_an_overlong_marker_label() -> gt_types::LoadedFile {
    let mut file = test_util::a_recording_with_every_marker_kind();
    for track in &mut file.tracks {
        for marker in &mut track.custom_markers {
            marker.label = gt_test_utils::oversized_text('m');
        }
    }
    file
}

/// The sticky popup stays inside the screen whichever map item it pins and
/// however long that item's label reads: the point layout's resizable frame
/// and the auto-sized frame the markers use both scroll their content.
#[rstest::rstest]
#[case::point(gt_types::DataCategory::Tpv, PointIdx::new(50))]
#[case::custom_marker(gt_types::DataCategory::CustomMarker, PointIdx::new(0))]
fn the_sticky_popup_fits_every_viewport(
    #[case] category: gt_types::DataCategory,
    #[case] point_index: PointIdx,
    #[values(
        gt_test_utils::window_fit::CRAMPED_VIEWPORT,
        gt_test_utils::window_fit::NARROW_VIEWPORT,
        gt_test_utils::window_fit::SHORT_VIEWPORT
    )]
    viewport: egui::Vec2,
) {
    use gt_test_utils::WindowFitAssertions as _;

    let files = vec![file_with_an_overlong_marker_label()];
    let clicked = test_util::point_ref(category, point_index.as_usize());

    let map = MapScene::of(files)
        .tiles(TileAccess::Synthetic)
        .size(viewport)
        .draw_state(|state| state.highlight.sticky = Some(clicked))
        .render();

    map.harness
        .inner
        .assert_window_fits_the_viewport(gt_test_utils::AuditedWindow::identified(
            "sticky popup",
            egui::Id::new(("sticky_popup", clicked)),
        ));
}
