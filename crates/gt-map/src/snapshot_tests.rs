use std::path::PathBuf;

use egui_kittest::kittest::Queryable as _;
use egui_phosphor::regular::CLOUD_LIGHTNING as ICON_CLOUD_LIGHTNING;

use super::*;
use gt_types::mercator::MercPoint;
use gt_types::{DataCategory, DisplayMode, FileIdx, NavPoint, PointIdx, TrackIdx, TrackRef};
use gt_ui_types::{DataPointRef, DisplayCategory, DisplayMask};
use rustc_hash::FxHashMap;

fn tpv_ref_in(file: FileIdx) -> DataPointRef {
    DataPointRef {
        track: TrackRef::new(file, TrackIdx::new(0)),
        category: DataCategory::Tpv,
        point_index: PointIdx::new(0),
    }
}

fn event_ref() -> DataPointRef {
    DataPointRef {
        track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        category: DataCategory::EventMarker,
        point_index: PointIdx::new(0),
    }
}

fn custom_ref() -> DataPointRef {
    DataPointRef {
        track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        category: DataCategory::CustomMarker,
        point_index: PointIdx::new(0),
    }
}

fn gen_ref() -> DataPointRef {
    DataPointRef {
        track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        category: DataCategory::GeneratedMarker,
        point_index: PointIdx::new(0),
    }
}

/// Builds a single `LoadedFile` with one TPV point, one event marker, one custom
/// marker, and one `GnssFixRegained` generated marker, all at index 0.  Used by
/// snapshot tests so each candidate type produces real human-readable text.
fn make_snapshot_file() -> gt_types::LoadedFile {
    use gt_types::{
        CustomMarker, EventMarker, FileMetadata, GeneratedMarker, GeneratedMarkerKind, GeoBounds,
        Latitude, LoadedFile, LoadedTrack, Longitude, MarkerIcon, MercBounds, TimeRange,
        TrackMetadata, mercator,
    };
    use uom::si::f64::Length;
    use uom::si::length::kilometer;

    let points = gt_test_utils::nav_test_data();
    let t0 = points[0].tpv.time().utc();
    let lat = Latitude::new(55.686_7);
    let lon = Longitude::new(12.563_8);

    let event_marker = EventMarker::new(
        t0,
        "Lap/Start".to_string(),
        Some("Lap start point".to_string()),
        lat,
        lon,
    );
    let custom_marker = CustomMarker::new(t0, "Coffee stop".to_string(), MarkerIcon::Pin, lat, lon);
    let generated_marker = GeneratedMarker {
        time: t0,
        kind: GeneratedMarkerKind::GnssFixRegained {
            fix_lost_duration: chrono::Duration::milliseconds(12_300),
        },
        lat,
        lon,
        merc: mercator::normalize(lat, lon),
    };

    let bb = GeoBounds::from_positions([
        (Latitude::new(55.67), Longitude::new(12.55)),
        (Latitude::new(55.69), Longitude::new(12.59)),
    ])
    .expect("two positions");
    let n = points.len();
    // Counted from the points rather than hard-coded: `SkySection::resolve`
    // short-circuits on a zero count, so claiming zero here would hide the
    // sky plot even though these points carry satellite reports.
    let satellite_report_count = points.iter().filter(|p| p.satellites.is_some()).count();
    let track = LoadedTrack {
        metadata: TrackMetadata {
            index: 0,
            distance_km: Length::new::<kilometer>(5.0),
            duration: chrono::Duration::seconds(n as i64),
            time_range: TimeRange::new(t0, t0 + chrono::Duration::seconds(n as i64)),
            bounding_box: bb,
            merc_bounds: MercBounds::from(bb),
            point_set_diameter_m: Length::new::<uom::si::length::meter>(500.0),
            has_custom_markers: true,
            tpv_count: n,
            satellite_report_count,
            custom_marker_count: 1,
            generated_marker_count: 1,
            event_marker_count: 1,
            ..gt_test_utils::empty_track_metadata()
        },
        sat_label_anchors: gt_track_builder::build_sat_label_anchors(&points),
        points,
        lod: gt_types::TrackLod::default(),
        custom_markers: vec![custom_marker],
        generated_markers: vec![generated_marker],
        event_markers: vec![event_marker],
        channels: vec![],
    };

    LoadedFile {
        metadata: FileMetadata {
            filename: "snapshot_test.gtd".to_string(),
            total_distance_km: Length::new::<kilometer>(5.0),
            total_duration: chrono::Duration::seconds(n as i64),
            time_range: TimeRange::new(t0, t0 + chrono::Duration::seconds(n as i64)),
            ..gt_test_utils::empty_file_metadata()
        },
        tracks: vec![track],
        event_marker_styles: FxHashMap::default(),
        orphaned_event_markers: vec![],
        source: gt_types::FileSource::GtdPath(PathBuf::from("snapshot_test.gtd")),
        load_warnings: vec![],
    }
}

/// Snapshot: the stacked multi-hover label popup for TPV + event marker +
/// custom marker simultaneously within cursor radius.  Calls the real
/// production function so the test stays in sync with the code.
#[test]
fn snap_multi_hover_stacked_label() {
    let files = vec![make_snapshot_file()];
    let candidates = HoverCandidates {
        tpv_or_satellite_report: Some(tpv_ref_in(FileIdx::new(0))),
        event_marker: Some(event_ref()),
        custom_marker: Some(custom_ref()),
        generated_marker: None,
    };

    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(400.0, 800.0))
        .ui(move |ui| {
            let names = RecordingNames::default();
            let labels = RecordingLabels::new(&files, &names);
            draw_multi_hover_label_contents(ui, candidates, &files, labels);
        });

    harness.fit_contents();
    harness.snapshot("multi_hover_stacked_label");
}

/// Snapshot: the stacked multi-hover label for the common case where a TPV
/// fix point and a GNSS-fix-regained generated marker share the same map
/// position.  The TPV section shows the full hover table. The generated-marker
/// section shows the kind and the fix-lost duration.
#[test]
fn snap_multi_hover_tpv_and_generated_marker() {
    let files = vec![make_snapshot_file()];
    let candidates = HoverCandidates {
        tpv_or_satellite_report: Some(tpv_ref_in(FileIdx::new(0))),
        generated_marker: Some(gen_ref()),
        ..HoverCandidates::default()
    };

    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(400.0, 800.0))
        .ui(move |ui| {
            let names = RecordingNames::default();
            let labels = RecordingLabels::new(&files, &names);
            draw_multi_hover_label_contents(ui, candidates, &files, labels);
        });

    harness.fit_contents();
    harness.snapshot("multi_hover_tpv_and_generated_marker");
}

/// Two loaded files with distinct filenames, so the labels that name a
/// recording have something to distinguish.
fn two_recordings_loaded() -> gt_loaded_files::LoadedFiles {
    let mut loaded = gt_loaded_files::LoadedFiles::new();
    for filename in ["morning.gtd", "evening.gtd"] {
        let mut file = make_snapshot_file();
        file.metadata.filename = filename.to_owned();
        loaded.push(file, gt_loaded_files::FileHistory::None);
    }
    loaded
}

/// Snapshot: the same stacked label with two files loaded, where the fix
/// section carries the recording row.
#[test]
fn snap_multi_hover_stacked_label_two_files() {
    let loaded = two_recordings_loaded();
    let candidates = HoverCandidates {
        tpv_or_satellite_report: Some(tpv_ref_in(FileIdx::new(1))),
        event_marker: Some(event_ref()),
        custom_marker: Some(custom_ref()),
        generated_marker: None,
    };

    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(400.0, 800.0))
        .ui(move |ui| {
            let names = RecordingNames::resolve(loaded.view(), "{filename}");
            let labels = RecordingLabels::new(loaded.files(), &names);
            draw_multi_hover_label_contents(ui, candidates, loaded.files(), labels);
        });

    harness.fit_contents();
    harness.snapshot("multi_hover_stacked_label_two_files");
}

/// The compound label names the recording of the fix it shows, which need
/// not be the file the markers stacked with it came from.
#[test]
fn multi_hover_names_the_hovered_fixs_recording() {
    let loaded = two_recordings_loaded();
    let candidates = HoverCandidates {
        tpv_or_satellite_report: Some(tpv_ref_in(FileIdx::new(1))),
        event_marker: Some(event_ref()),
        ..HoverCandidates::default()
    };

    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(400.0, 800.0))
        .ui(move |ui| {
            let names = RecordingNames::resolve(loaded.view(), "{filename}");
            let labels = RecordingLabels::new(loaded.files(), &names);
            draw_multi_hover_label_contents(ui, candidates, loaded.files(), labels);
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
    let files = vec![make_snapshot_file()];
    let candidates = [
        Some(tpv_ref_in(FileIdx::new(0))),
        Some(event_ref()),
        None,
        None,
    ];
    let sticky = Some(tpv_ref_in(FileIdx::new(0)));

    let mut harness = crate::test_harness::builder()
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

/// Drive the full `NavMap::draw` path over the fixture track with a
/// hardcoded set of query matches. No map tiles render beneath the halos:
/// the map is built with [`TileAccess::Offline`].
fn snapshot_nav_map_with_matches(
    name: &'static str,
    mode: DisplayMode,
    stale: bool,
    capture: MatchCapture,
) {
    use gt_ui_types::{DrawLayer, QueryMatches, TrackDataVisibility, TrackRanges};

    let files = vec![make_snapshot_file()];
    let visibility = TrackDataVisibility::from_loaded(&files);
    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
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

    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(800.0, 600.0))
        .ui_state(
            move |ui, map: &mut Option<NavMap>| {
                let map =
                    map.get_or_insert_with(|| NavMap::new(ui.ctx().clone(), TileAccess::Offline));
                let mut state = DrawState::default();
                map.draw(
                    ui,
                    MapDrawContext {
                        query_matches: Some(&matches),
                        ..state.context(&files, &visibility)
                    },
                );
            },
            None,
        );

    match capture {
        // The first frame zooms to fit the newly seen file; the rest let the
        // blink and fade animations settle before the snapshot.
        MatchCapture::Settled => {
            for _ in 0..5 {
                harness.run();
            }
        }
        // One frame exactly, so the reveal is captured on the frame it starts.
        MatchCapture::RevealStart => harness.step(),
    }
    harness.snapshot_loose(name);
}

/// Interference cells around the snapshot fixture's track, tallied
/// across the ramp so one snapshot shows clear, elevated, heavy, and
/// low-sample fills together.
fn snapshot_jamming_dataset() -> JamDataset {
    use gt_jam::wire::HexObservation;

    let center = h3o::LatLng::new(55.686_7, 12.563_8)
        .expect("fixture position")
        .to_cell(gt_jam::H3_RESOLUTION);
    let tallies = [
        (400, 0),
        (98, 2),
        (94, 6),
        (90, 10),
        (60, 40),
        (2, 2),
        (1, 1),
    ];
    let observations = center
        .grid_disk::<Vec<_>>(1)
        .into_iter()
        .zip(tallies)
        .map(|(cell, (good, bad))| HexObservation { cell, good, bad })
        .collect();
    JamDataset::new(
        chrono::NaiveDate::from_ymd_opt(2026, 7, 20).expect("date"),
        observations,
    )
}

/// The interference overlay under the fixture track. At the zoom that
/// frames a 1 km track a single 22 km cell covers the viewport, so this
/// pins the fill and the draw order - track ink over cells - rather than
/// the ramp, which `jamming_renderer`'s own tests cover. No map tiles
/// render: the map is built with [`TileAccess::Offline`].
#[rstest::rstest]
#[case::dark("jamming_overlay_dark", true, None)]
#[case::light("jamming_overlay_light", false, None)]
#[case::hover("jamming_overlay_hover", true, Some(egui::pos2(400.0, 300.0)))]
fn snapshot_jamming_overlay(
    #[case] name: &str,
    #[case] dark_mode: bool,
    #[case] hover: Option<egui::Pos2>,
) {
    let files = vec![make_snapshot_file()];
    let visibility = gt_ui_types::TrackDataVisibility::from_loaded(&files);
    let dataset = snapshot_jamming_dataset();

    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(800.0, 600.0))
        .theme(dark_mode)
        .ui_state(
            move |ui, map: &mut Option<NavMap>| {
                let map =
                    map.get_or_insert_with(|| NavMap::new(ui.ctx().clone(), TileAccess::Offline));
                let mut state = DrawState::default();
                map.draw(
                    ui,
                    MapDrawContext {
                        jamming_dataset: Some(&dataset),
                        ..state.context(&files, &visibility)
                    },
                );
            },
            None,
        );

    // The first frame zooms to fit the file; the rest settle animations.
    for _ in 0..5 {
        harness.run();
    }
    if let Some(pos) = hover {
        harness.inner.hover_at(pos);
        // Tooltips appear after egui's hover delay.
        for _ in 0..60 {
            harness.run();
        }
    }
    harness.snapshot_loose(name);
}

/// The levels the application lists, as the popup receives them.
fn snapshot_warning_levels() -> Vec<gt_ui_types::WarningLevelExplanation> {
    vec![
        gt_ui_types::WarningLevelExplanation {
            trigger: "Aircraft interference: 2 % or more of aircraft in a crossed cell reported \
                      low navigation accuracy (gpsjam.org's own yellow level)."
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
            "Geomagnetic storm: Hp30 reached 7.667 (G3)".to_owned(),
            "Aircraft interference: up to 34.2 % of aircraft in a crossed cell (warns from 2 %)"
                .to_owned(),
            "Solar flare: X5.8 at 2024-05-11 02:01 UTC (R3), receiver on the sunlit side"
                .to_owned(),
            "TEC deviation: -73 % from the 27-day median (warns from -30 %), intense ionospheric \
             storm (W = -4), for 22 h, after a G5 storm 9 h before"
                .to_owned(),
            "TEC over the track: 12 to 175 TECU".to_owned(),
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
                "Geomagnetic storm: Kp reached {}.667 (G3)",
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
/// affected track over the same rows either way. No map tiles render: the map
/// is built with [`TileAccess::Offline`].
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
    let files = vec![make_snapshot_file()];
    let visibility = gt_ui_types::TrackDataVisibility::from_loaded(&files);
    let levels = snapshot_warning_levels();

    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(800.0, 600.0))
        .ui_state(
            move |ui, map: &mut Option<NavMap>| {
                let map =
                    map.get_or_insert_with(|| NavMap::new(ui.ctx().clone(), TileAccess::Offline));
                let mut state = DrawState::default();
                map.draw(
                    ui,
                    MapDrawContext {
                        space_weather: crate::SpaceWeatherIndicator {
                            track_warnings: &warning,
                            levels: &levels,
                            tec_deviation_caveat: &gt_ionex::text::DEVIATION_REFERENCE_CAVEAT,
                        },
                        ..state.context(&files, &visibility)
                    },
                );
            },
            None,
        );

    // The first frame zooms to fit the file. The rest settle animations.
    for _ in 0..5 {
        harness.run();
    }
    match interaction {
        IndicatorInteraction::Hover => {
            let glyph = harness.inner.get_by_label(ICON_CLOUD_LIGHTNING).rect();
            harness.inner.hover_at(glyph.center());
            // Tooltips appear after egui's hover delay.
            for _ in 0..60 {
                harness.run();
            }
        }
        IndicatorInteraction::Click => {
            harness.inner.get_by_label(ICON_CLOUD_LIGHTNING).click();
            harness.inner.run_steps(2);
        }
    }
    harness.snapshot_loose(name);
}

/// Zoom at which the whole world fits the snapshot canvas: the world spans
/// `256 * 2^zoom` pixels.
const WORLD_ZOOM: f64 = 1.5;

/// The TEC heatmap under the fixture track, at the 10 May 2024 storm's peak
/// hours. The whole world is in view, so one snapshot covers the ramp from the
/// quiet night side to the equatorial crests past 150 TECU. No map tiles
/// render: the map is built with [`TileAccess::Offline`].
#[rstest::rstest]
#[case::dark("tec_heatmap_dark", true, None)]
#[case::light("tec_heatmap_light", false, None)]
#[case::hover("tec_heatmap_hover", true, Some(egui::pos2(300.0, 380.0)))]
fn snapshot_tec_heatmap(
    #[case] name: &str,
    #[case] dark_mode: bool,
    #[case] hover: Option<egui::Pos2>,
) {
    let files = vec![make_snapshot_file()];
    let visibility = gt_ui_types::TrackDataVisibility::from_loaded(&files);
    let maps = gt_ionex::captured_maps(gt_ionex::STORM_CAPTURE).expect("the storm capture");
    let instant = chrono::NaiveDate::from_ymd_opt(2024, 5, 10)
        .and_then(|day| day.and_hms_opt(20, 0, 0))
        .map(|naive| naive.and_utc())
        .expect("an epoch of the captured day");

    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(800.0, 600.0))
        .theme(dark_mode)
        .ui_state(
            move |ui, map: &mut Option<NavMap>| {
                let map =
                    map.get_or_insert_with(|| NavMap::new(ui.ctx().clone(), TileAccess::Offline));
                map.map_memory.set_zoom(WORLD_ZOOM).expect("a valid zoom");
                let mut state = DrawState::default();
                state
                    .display_mask
                    .set_visible(DisplayCategory::TecHeatmap, true);
                let mut shown =
                    gt_ionex::TecInstantSelection::new(Some(instant), instant.date_naive());
                map.draw(
                    ui,
                    MapDrawContext {
                        tec: crate::TecLayer {
                            snapshot: Some(crate::TecHeatmapSnapshot {
                                maps: &maps,
                                instant,
                            }),
                            instant: &mut shown,
                            empty_reason: None,
                        },
                        center_request: Some((0.0, 0.0)),
                        ..state.context(&files, &visibility)
                    },
                );
            },
            None,
        );

    // The first frame zooms to fit the file. The rest settle animations.
    for _ in 0..5 {
        harness.run();
    }
    if let Some(pos) = hover {
        harness.inner.hover_at(pos);
        // Tooltips appear after egui's hover delay.
        for _ in 0..60 {
            harness.run();
        }
    }
    harness.snapshot_loose(name);
}

/// Nudge north (smaller Mercator y) by roughly ten pixels at the
/// snapped-track snapshot tests' zoom, so the snapped line reads beside
/// the recorded one instead of on top of it.
const SNAPPED_OFFSET_MERC_Y: f64 = -1.5e-6;

/// Snapshot the map with `make_snapshot_file`'s track plus the snapped
/// geometry `geometry_for` derives from its points, drawn under `mask`.
/// With `hover`, the pointer is parked there before the snapshot (frames
/// are stepped past egui's tooltip delay). No map tiles render: the map is
/// built with [`TileAccess::Offline`].
fn snapshot_snapped_tracks_with(
    name: &str,
    mask: DisplayMask,
    hover: Option<egui::Pos2>,
    geometry_for: impl Fn(&[NavPoint]) -> gt_ui_types::SnappedTrackGeometry,
) {
    use std::sync::Arc;

    use gt_ui_types::{SnappedTracks, TrackDataVisibility};

    let files = vec![make_snapshot_file()];
    let visibility = TrackDataVisibility::from_loaded(&files);
    let track_ref = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    let points = files
        .first()
        .and_then(|f| f.tracks.first())
        .map(|t| t.points.clone())
        .unwrap_or_default();
    let mut snapped = SnappedTracks::default();
    snapped.insert(track_ref, Arc::new(geometry_for(&points)));

    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(800.0, 600.0))
        .ui_state(
            move |ui, map: &mut Option<NavMap>| {
                let map =
                    map.get_or_insert_with(|| NavMap::new(ui.ctx().clone(), TileAccess::Offline));
                let mut state = DrawState {
                    display_mask: mask,
                    ..DrawState::default()
                };
                map.draw(
                    ui,
                    MapDrawContext {
                        snapped_tracks: Some(&snapped),
                        ..state.context(&files, &visibility)
                    },
                );
            },
            None,
        );

    for _ in 0..5 {
        harness.run();
    }
    if let Some(pos) = hover {
        harness.inner.hover_at(pos);
        // Tooltips appear after egui's hover delay; keep stepping until
        // it elapsed and the tooltip laid itself out.
        for _ in 0..60 {
            harness.run();
        }
    }
    harness.snapshot_loose(name);
}

/// [`snapshot_snapped_tracks_with`] for bare polylines (no edge data,
/// no hover).
fn snapshot_snapped_tracks(
    name: &str,
    mask: DisplayMask,
    segments_for: impl Fn(&[NavPoint]) -> Vec<Vec<MercPoint>>,
) {
    snapshot_snapped_tracks_with(name, mask, None, |points| {
        gt_ui_types::SnappedTrackGeometry {
            segments: segments_for(points)
                .into_iter()
                .map(|points| gt_ui_types::SnappedSegment {
                    points,
                    edge_spans: Vec::new(),
                })
                .collect(),
            edges: Vec::new(),
            whiskers: Vec::new(),
        }
    });
}

/// A snapped segment following the recorded points in `range`, nudged
/// north by [`SNAPPED_OFFSET_MERC_Y`].
fn snapped_segment(points: &[NavPoint], range: std::ops::Range<usize>) -> Vec<MercPoint> {
    points
        .get(range)
        .unwrap_or_default()
        .iter()
        .map(|p| MercPoint {
            x: p.merc.x,
            y: p.merc.y + SNAPPED_OFFSET_MERC_Y,
        })
        .collect()
}

/// Snapshot: dashed translucent snapped-track polylines beside the
/// recorded track, with the empty stretch between the two segments
/// rendering as a gap (a route discontinuity) - the recorded track
/// beneath is never painted over or hidden.
#[test]
fn snap_snapped_track_polylines() {
    snapshot_snapped_tracks(
        "snapped_track_polylines",
        DisplayMask::default(),
        |points| {
            vec![
                snapped_segment(points, 100..400),
                snapped_segment(points, 600..950),
            ]
        },
    );
}

/// Snapshot: a snapped segment whose tail runs far past the viewport.
/// The culling in `SnappedTrackRenderer` must not clip visible geometry:
/// the dashed line has to reach the viewport edge exactly, while the
/// off-screen stretch generates no dashes at all (partially visible
/// segments keep exact endpoints; only provably invisible ones are
/// dropped).
#[test]
fn snap_snapped_track_culled_tail() {
    /// Mercator step between synthetic tail points, ≈ 5 viewport widths
    /// beyond the fitted view over 60 points, so most of the tail is
    /// provably off-screen.
    const TAIL_STEP_MERC_X: f64 = 2e-5;

    snapshot_snapped_tracks(
        "snapped_track_culled_tail",
        DisplayMask::default(),
        |points| {
            let mut segment = snapped_segment(points, 100..400);
            if let Some(&end) = segment.last() {
                segment.extend((1..=60).map(|i| MercPoint {
                    x: end.x + f64::from(i) * TAIL_STEP_MERC_X,
                    y: end.y,
                }));
            }
            vec![segment]
        },
    );
}

/// Snapshot: a snapped segment whose on-screen extent packs below one
/// pixel draws as a dot instead of vanishing - the `VisiblePath::Dot`
/// case, reached when snapped geometry collapses at low zoom. The dot
/// sits north of the recorded track's midpoint.
#[test]
fn snap_snapped_track_collapsed_dot() {
    /// Mercator spacing of the collapsed cluster's points, ≈ 0.1 px at
    /// the fitted zoom - far below the sub-pixel merge threshold.
    const CLUSTER_STEP_MERC_X: f64 = 2e-8;

    /// Extra northward offset so the dot is clearly separate from the
    /// recorded trackline.
    const CLUSTER_OFFSET_MERC_Y: f64 = -6e-6;

    snapshot_snapped_tracks(
        "snapped_track_collapsed_dot",
        DisplayMask::default(),
        |points| {
            let mid = points.len() / 2;
            let Some(base) = points.get(mid) else {
                return vec![];
            };
            vec![
                (0..4)
                    .map(|i| MercPoint {
                        x: base.merc.x + f64::from(i) * CLUSTER_STEP_MERC_X,
                        y: base.merc.y + CLUSTER_OFFSET_MERC_Y,
                    })
                    .collect(),
            ]
        },
    );
}

/// Snapshot: hiding the snapped-tracks display category removes the
/// dashed ink entirely - only the recorded track remains - without
/// touching the underlying snapped geometry.
#[test]
fn snap_snapped_track_hidden_by_display_mask() {
    let mut mask = DisplayMask::default();
    mask.set_visible(DisplayCategory::SnappedTracks, false);
    snapshot_snapped_tracks("snapped_track_hidden_by_display_mask", mask, |points| {
        vec![
            snapped_segment(points, 100..400),
            snapped_segment(points, 600..950),
        ]
    });
}

/// Snapshot: hovering the snapped line shows the matched edge's
/// attributes. The synthetic segment runs horizontally through the
/// viewport center (zoom-to-fit centers the recorded track's bounds),
/// so parking the pointer at the center hits it deterministically.
#[test]
fn snap_snapped_track_edge_hover() {
    /// Half-width of the synthetic segment, Mercator units - wide
    /// enough to cross the whole fitted viewport.
    const SEGMENT_HALF_WIDTH_MERC: f64 = 1.0e-4;

    snapshot_snapped_tracks_with(
        "snapped_track_edge_hover",
        DisplayMask::default(),
        Some(egui::pos2(400.0, 300.0)),
        |points| {
            let (min, max) = points.iter().fold(
                ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN)),
                |(min, max), p| {
                    (
                        (min.0.min(p.merc.x), min.1.min(p.merc.y)),
                        (max.0.max(p.merc.x), max.1.max(p.merc.y)),
                    )
                },
            );
            let center = MercPoint {
                x: f64::midpoint(min.0, max.0),
                y: f64::midpoint(min.1, max.1),
            };
            gt_ui_types::SnappedTrackGeometry {
                segments: vec![gt_ui_types::SnappedSegment {
                    points: vec![
                        MercPoint {
                            x: center.x - SEGMENT_HALF_WIDTH_MERC,
                            y: center.y,
                        },
                        MercPoint {
                            x: center.x + SEGMENT_HALF_WIDTH_MERC,
                            y: center.y,
                        },
                    ],
                    edge_spans: vec![gt_ui_types::SnappedEdgeSpan {
                        start: 0,
                        end: 2,
                        edge: 0,
                    }],
                }],
                edges: vec![gt_ui_types::SnappedEdgeInfo {
                    name: Some("H.C. Andersens Boulevard".to_owned()),
                    road_class: Some("Tertiary".to_owned()),
                    speed_limit: Some("50 km/h".to_owned()),
                    surface: Some("Paved smooth".to_owned()),
                }],
                whiskers: Vec::new(),
            }
        },
    );
}

/// Eastward Mercator offset of the synthetic whisker tests' snapped
/// positions: ~9 m at the fixture latitude, so whiskers are clearly
/// longer than the strokes they connect.
const WHISKER_OFFSET_MERC_X: f64 = 4.0e-7;

/// Whisker anchors and the matching snapped polyline for a run over
/// `points`: every recorded point snaps [`WHISKER_OFFSET_MERC_X`] east.
fn whisker_geometry(points: &[NavPoint]) -> gt_ui_types::SnappedTrackGeometry {
    let snapped: Vec<MercPoint> = points
        .iter()
        .map(|p| MercPoint {
            x: p.merc.x + WHISKER_OFFSET_MERC_X,
            y: p.merc.y,
        })
        .collect();
    gt_ui_types::SnappedTrackGeometry {
        segments: vec![gt_ui_types::SnappedSegment {
            points: snapped.clone(),
            edge_spans: Vec::new(),
        }],
        edges: Vec::new(),
        whiskers: points
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
fn make_short_walk_file() -> gt_types::LoadedFile {
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
            bounding_box: bb,
            merc_bounds: MercBounds::from(bb),
            tpv_count: n,
            ..gt_test_utils::empty_track_metadata()
        },
        sat_label_anchors: Vec::new(),
        points,
        lod: gt_types::TrackLod::default(),
        custom_markers: Vec::new(),
        generated_markers: Vec::new(),
        event_markers: Vec::new(),
        channels: Vec::new(),
    };
    LoadedFile {
        metadata: FileMetadata {
            filename: "short_walk.gtd".to_string(),
            time_range: TimeRange::new(t0, t0 + chrono::Duration::seconds(n as i64)),
            ..gt_test_utils::empty_file_metadata()
        },
        tracks: vec![track],
        event_marker_styles: FxHashMap::default(),
        orphaned_event_markers: vec![],
        source: gt_types::FileSource::GtdPath(PathBuf::from("short_walk.gtd")),
        load_warnings: vec![],
    }
}

/// Snapshot: above the scale gate (a ~55 m track fitted into the
/// viewport) every snapped point gets its error whisker - a thin line
/// from the recorded point to the snapped position.
#[test]
fn snap_snapped_track_whiskers_at_high_zoom() {
    use std::sync::Arc;

    use gt_ui_types::{SnappedTracks, TrackDataVisibility};

    let files = vec![make_short_walk_file()];
    let visibility = TrackDataVisibility::from_loaded(&files);
    let track_ref = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    let points = files
        .first()
        .and_then(|f| f.tracks.first())
        .map(|t| t.points.clone())
        .unwrap_or_default();
    let mut snapped = SnappedTracks::default();
    snapped.insert(track_ref, Arc::new(whisker_geometry(&points)));

    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(800.0, 600.0))
        .ui_state(
            move |ui, map: &mut Option<NavMap>| {
                let map =
                    map.get_or_insert_with(|| NavMap::new(ui.ctx().clone(), TileAccess::Offline));
                let mut state = DrawState::default();
                map.draw(
                    ui,
                    MapDrawContext {
                        snapped_tracks: Some(&snapped),
                        ..state.context(&files, &visibility)
                    },
                );
            },
            None,
        );
    for _ in 0..5 {
        harness.run();
    }
    harness.snapshot_loose("snapped_track_whiskers");
}

/// Snapshot: below the scale gate (the standard km-scale fixture) the
/// same whisker anchors draw nothing - only the dashed snapped line.
#[test]
fn snap_snapped_track_whiskers_hidden_below_gate() {
    snapshot_snapped_tracks_with(
        "snapped_track_whiskers_below_gate",
        DisplayMask::default(),
        None,
        whisker_geometry,
    );
}

/// Snapshot: match halos along the track, including the single-point
/// ring, over the live map canvas.
#[test]
fn snap_query_match_halos() {
    snapshot_nav_map_with_matches(
        "query_match_halos",
        DisplayMode::Draw,
        false,
        MatchCapture::Settled,
    );
}

/// Snapshot: the same matches grayed out after the visible data changed
/// (stale results are dimmed, never hidden).
#[test]
fn snap_query_match_halos_stale() {
    snapshot_nav_map_with_matches(
        "query_match_halos_stale",
        DisplayMode::Draw,
        true,
        MatchCapture::Settled,
    );
}

/// Snapshot: `keep` mode shows only the matching stretches; the rest of
/// the track is hidden and the polyline breaks at the gaps.
#[test]
fn snap_query_keep_mode() {
    snapshot_nav_map_with_matches(
        "query_keep_mode",
        DisplayMode::Keep,
        false,
        MatchCapture::Settled,
    );
}

/// Snapshot: `hide` mode drops the matching stretches, leaving the rest
/// of the track with breaks where the matches were.
#[test]
fn snap_query_hide_mode() {
    snapshot_nav_map_with_matches(
        "query_hide_mode",
        DisplayMode::Hide,
        false,
        MatchCapture::Settled,
    );
}

/// Snapshot: the halos of a run that just completed, caught on the frame its
/// reveal fires - every band and ring inflated and brightened before it
/// settles back to the state `snap_query_match_halos` shows.
#[test]
fn snap_query_match_reveal() {
    snapshot_nav_map_with_matches(
        "query_match_reveal",
        DisplayMode::Draw,
        false,
        MatchCapture::RevealStart,
    );
}

/// Snapshot: the halo band for the match hovered in the query results
/// table - the highlight blue over the matched stretch, without any
/// `draw` layers underneath.
#[test]
fn snap_query_match_hover_halo() {
    let files = vec![make_snapshot_file()];
    let visibility = gt_ui_types::TrackDataVisibility::from_loaded(&files);
    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));

    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(800.0, 600.0))
        .ui_state(
            move |ui, map: &mut Option<NavMap>| {
                let map =
                    map.get_or_insert_with(|| NavMap::new(ui.ctx().clone(), TileAccess::Offline));
                let mut state = DrawState {
                    highlight: MapHighlight {
                        hover_match: Some(gt_ui_types::MatchHighlight::new(track, &(150..300))),
                        ..MapHighlight::default()
                    },
                    ..DrawState::default()
                };
                map.draw(ui, state.context(&files, &visibility));
            },
            None,
        );

    for _ in 0..5 {
        harness.run();
    }
    harness.snapshot_loose("query_match_hover_halo");
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
    }
}

/// One filter's layer: its matches take the entries of `source` in order, so
/// every hexagon stands for a line the tooltip can read back.
fn log_layer(
    color: gt_ui_types::LogMatchColor,
    source: &gt_ui_types::LogMatchSource,
    positions: Vec<MercPoint>,
) -> gt_ui_types::LogMatchLayer {
    gt_ui_types::LogMatchLayer {
        color,
        log: source.clone(),
        matches: positions
            .into_iter()
            .enumerate()
            .map(|(entry_index, merc)| gt_ui_types::LogMatch { merc, entry_index })
            .collect(),
    }
}

/// Every state a log hexagon draws in, over one track: a filter's plain
/// glyphs, a cluster large enough to state its count, a second filter's
/// colour beside the first, and the doubled outline of a shared colour.
#[test]
fn snap_log_match_hexagons() {
    use gt_ui_types::{LogMatchColor, LogMatches};

    let files = vec![make_snapshot_file()];
    let visibility = gt_ui_types::TrackDataVisibility::from_loaded(&files);
    let merc_at = |index: usize| {
        files
            .first()
            .and_then(|file| file.tracks.first())
            .and_then(|track| track.points.get(index))
            .map_or(MercPoint { x: 0.5, y: 0.5 }, |point| point.merc)
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
    let log_matches = LogMatches::from_layers(vec![
        log_layer(
            LogMatchColor::LayerSlot {
                index: 0,
                shared: false,
            },
            &source,
            spread(40, 6),
        ),
        log_layer(
            LogMatchColor::LayerSlot {
                index: 1,
                shared: true,
            },
            &source,
            spread(53, 4),
        ),
        log_layer(LogMatchColor::LiveFilter, &source, clustered),
    ]);

    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(800.0, 600.0))
        .ui_state(
            move |ui, map: &mut Option<NavMap>| {
                let map =
                    map.get_or_insert_with(|| NavMap::new(ui.ctx().clone(), TileAccess::Offline));
                let mut state = DrawState {
                    log_matches: log_matches.clone(),
                    ..DrawState::default()
                };
                map.draw(ui, state.context(&files, &visibility));
            },
            None,
        );

    for _ in 0..5 {
        harness.run();
    }
    harness.snapshot_loose("log_match_hexagons");
}

/// A filter that matched every point of the track: its clusters draw evenly
/// spaced along the line, each stating its own count.
#[test]
fn snap_log_matches_along_a_dense_track() {
    use gt_ui_types::{LogMatchColor, LogMatches};

    let files = vec![make_snapshot_file()];
    let visibility = gt_ui_types::TrackDataVisibility::from_loaded(&files);
    let positions: Vec<MercPoint> = files
        .first()
        .and_then(|file| file.tracks.first())
        .map(|track| track.points.iter().map(|point| point.merc).collect())
        .unwrap_or_default();
    let source = snapshot_log_source(positions.len());
    let log_matches = LogMatches::from_layers(vec![log_layer(
        LogMatchColor::LayerSlot {
            index: 0,
            shared: false,
        },
        &source,
        positions,
    )]);

    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(800.0, 600.0))
        .ui_state(
            move |ui, map: &mut Option<NavMap>| {
                let map =
                    map.get_or_insert_with(|| NavMap::new(ui.ctx().clone(), TileAccess::Offline));
                let mut state = DrawState {
                    log_matches: log_matches.clone(),
                    ..DrawState::default()
                };
                map.draw(ui, state.context(&files, &visibility));
            },
            None,
        );

    for _ in 0..5 {
        harness.run();
    }
    harness.snapshot_loose("log_matches_along_a_dense_track");
}

/// Two filters that matched the same run of lines: the layer on top covers the
/// one below glyph by glyph, count and all. The layer below counts three times
/// as many lines, so a count escaping from under a covering hexagon would be
/// wider than the one that belongs there.
#[test]
fn snap_log_matches_of_overlapping_layers() {
    use gt_ui_types::{LogMatchColor, LogMatches};

    let files = vec![make_snapshot_file()];
    let visibility = gt_ui_types::TrackDataVisibility::from_loaded(&files);
    let track_positions: Vec<MercPoint> = files
        .first()
        .and_then(|file| file.tracks.first())
        .map(|track| track.points.iter().map(|point| point.merc).collect())
        .unwrap_or_default();
    let source = snapshot_log_source(track_positions.len() * 3);
    let covered = log_layer(
        LogMatchColor::LayerSlot {
            index: 0,
            shared: false,
        },
        &source,
        track_positions.iter().flat_map(|&merc| [merc; 3]).collect(),
    );
    // This layer matched only the first half of the run, so the layer below
    // draws its own hexagons and counts along the rest of the track.
    let covering = log_layer(
        LogMatchColor::LayerSlot {
            index: 1,
            shared: false,
        },
        &source,
        track_positions
            .iter()
            .skip(2)
            .take(track_positions.len() / 2)
            .copied()
            .collect(),
    );
    let log_matches = LogMatches::from_layers(vec![covered, covering]);

    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(800.0, 600.0))
        .ui_state(
            move |ui, map: &mut Option<NavMap>| {
                let map =
                    map.get_or_insert_with(|| NavMap::new(ui.ctx().clone(), TileAccess::Offline));
                let mut state = DrawState {
                    log_matches: log_matches.clone(),
                    ..DrawState::default()
                };
                map.draw(ui, state.context(&files, &visibility));
            },
            None,
        );

    for _ in 0..5 {
        harness.run();
    }
    harness.snapshot_loose("log_matches_of_overlapping_layers");
}

/// The map state a hover test drives across frames: the map itself, and the
/// per-frame inputs it draws from, kept so the hexagon it found under the
/// cursor can be read back after the frame.
struct LogHoverState {
    map: Option<NavMap>,
    draw: DrawState,
}

/// Entries the live-filter layer's cluster at the centre of the fixture stands
/// for: more than the tooltip writes out, leaving it a tail to state.
const HOVERED_CLUSTER_ENTRIES: usize = 8;

/// The canvas the log interaction tests draw the map on.
const LOG_HOVER_CANVAS: egui::Vec2 = egui::vec2(800.0, 600.0);

/// The map framed on the fixture recording, drawing the matches the caller
/// puts at its centre. That centre is where the map frames the recording, so
/// it lands at the centre of the canvas.
fn log_map_harness(
    matches_at_center: impl FnOnce(MercPoint) -> gt_ui_types::LogMatches,
) -> (
    gt_test_utils::TestHarness<'static, LogHoverState>,
    MercPoint,
) {
    use gt_types::mercator;
    use gt_ui_types::TrackDataVisibility;

    let files = vec![make_snapshot_file()];
    let visibility = TrackDataVisibility::from_loaded(&files);
    let bounds = crate::viewport::compute_visible_bounding_box(
        &files,
        &visibility,
        &gt_filter::GlobalFilter::default(),
        DisplayMask::default(),
    )
    .expect("the fixture recording has points");
    let (center_lat, center_lon) = bounds.center();
    let center = mercator::normalize(center_lat, center_lon);

    let mut harness = crate::test_harness::builder()
        .size(LOG_HOVER_CANVAS)
        .ui_state(
            move |ui, state: &mut LogHoverState| {
                let map = state
                    .map
                    .get_or_insert_with(|| NavMap::new(ui.ctx().clone(), TileAccess::Offline));
                map.draw(ui, state.draw.context(&files, &visibility));
            },
            LogHoverState {
                map: None,
                draw: DrawState {
                    log_matches: matches_at_center(center),
                    ..DrawState::default()
                },
            },
        );
    // The first frame frames the recording, the rest settle the animations.
    for _ in 0..5 {
        harness.run();
    }
    (harness, center)
}

/// The centre of the canvas, where the map draws the centre of the recording.
fn log_hover_canvas_center() -> egui::Pos2 {
    egui::Rect::from_min_size(egui::Pos2::ZERO, LOG_HOVER_CANVAS).center()
}

/// The fixture the hover tests drive: a layer chip's three matches and the
/// live filter's eight, all at the point the map centres on. The cursor at the
/// centre of the canvas is then on the live filter's hexagon, with the chip's
/// underneath it.
fn log_hover_harness() -> gt_test_utils::TestHarness<'static, LogHoverState> {
    use gt_ui_types::{LogMatchColor, LogMatches};

    let (mut harness, _) = log_map_harness(|center| {
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
    harness.inner.hover_at(log_hover_canvas_center());
    // Tooltips appear after egui's hover delay.
    for _ in 0..60 {
        harness.run();
    }
    harness
}

/// The map rings the viewer's hovered row even where the filters selected
/// nothing: the row has a position wherever its line was recorded.
#[test]
fn a_hovered_viewer_row_is_ringed_on_the_map() {
    let (mut harness, center) = log_map_harness(|_| gt_ui_types::LogMatches::default());
    let before = harness.inner.render().expect("the harness renders a frame");

    harness.state_mut().draw.log_hover.row_position = Some(center);
    harness.run();

    let after = harness.inner.render().expect("the harness renders a frame");
    let around_the_centre =
        egui::Rect::from_center_size(log_hover_canvas_center(), egui::Vec2::splat(40.0));
    assert!(
        gt_test_utils::snapshot_harness::pixels_differ(
            &before,
            &after,
            around_the_centre,
            harness.inner.ctx.pixels_per_point()
        ),
        "the ring draws where the hovered row's line was recorded"
    );
}

/// The cursor picks the hexagon of the topmost layer it is on, and that
/// hexagon names the lines it stands for - what the viewer marks the rows of.
#[test]
fn hovering_a_hexagon_names_the_lines_of_the_topmost_layer_it_is_on() {
    let harness = log_hover_harness();

    let glyph = harness
        .state()
        .draw
        .log_hover
        .glyph
        .as_ref()
        .expect("the cursor is on the centre hexagon");
    assert_eq!(glyph.color, gt_ui_types::LogMatchColor::LiveFilter);
    assert_eq!(
        glyph.entry_indices,
        (0..HOVERED_CLUSTER_ENTRIES).collect::<Vec<usize>>(),
        "the hexagon stands for every line its cluster collapsed"
    );
}

/// Snapshot: the hovered cluster ringed, over the lines it collapsed and the
/// count of the ones the tooltip left out.
#[test]
fn snap_log_match_hover() {
    let mut harness = log_hover_harness();

    harness.snapshot_loose("log_match_hover");
}

/// The layer switches off with its display category, like every other kind of
/// map ink.
#[test]
fn snap_log_matches_hidden_by_display_mask() {
    use gt_ui_types::{LogMatchColor, LogMatches};

    let files = vec![make_snapshot_file()];
    let visibility = gt_ui_types::TrackDataVisibility::from_loaded(&files);
    let positions: Vec<gt_types::MercPoint> = files
        .first()
        .and_then(|file| file.tracks.first())
        .map(|track| track.points.iter().map(|point| point.merc).collect())
        .unwrap_or_default();
    let source = snapshot_log_source(positions.len());
    let log_matches = LogMatches::from_layers(vec![log_layer(
        LogMatchColor::LayerSlot {
            index: 0,
            shared: false,
        },
        &source,
        positions,
    )]);
    let mut mask = DisplayMask::default();
    mask.set_visible(DisplayCategory::LogMatches, false);

    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(800.0, 600.0))
        .ui_state(
            move |ui, map: &mut Option<NavMap>| {
                let map =
                    map.get_or_insert_with(|| NavMap::new(ui.ctx().clone(), TileAccess::Offline));
                let mut state = DrawState {
                    log_matches: log_matches.clone(),
                    display_mask: mask,
                    ..DrawState::default()
                };
                map.draw(ui, state.context(&files, &visibility));
            },
            None,
        );

    for _ in 0..5 {
        harness.run();
    }
    harness.snapshot_loose("log_matches_hidden_by_display_mask");
}

/// Snapshot: the display mask removes the marker ink (custom, generated,
/// event) while the track, its icons, and the satellite labels stay.
/// Compare against the marker-bearing fixture in the other snapshots.
#[test]
fn snap_display_mask_hides_markers() {
    use gt_ui_types::{DisplayCategory, DisplayMask, TrackDataVisibility};

    let files = vec![make_snapshot_file()];
    let visibility = TrackDataVisibility::from_loaded(&files);
    let mut mask = DisplayMask::default();
    for category in [
        DisplayCategory::CustomMarkers,
        DisplayCategory::GeneratedMarkers,
        DisplayCategory::EventMarkers,
    ] {
        mask.set_visible(category, false);
    }

    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(800.0, 600.0))
        .ui_state(
            move |ui, map: &mut Option<NavMap>| {
                let map =
                    map.get_or_insert_with(|| NavMap::new(ui.ctx().clone(), TileAccess::Offline));
                let mut state = DrawState {
                    display_mask: mask,
                    ..DrawState::default()
                };
                map.draw(ui, state.context(&files, &visibility));
            },
            None,
        );

    for _ in 0..5 {
        harness.run();
    }
    harness.snapshot_loose("display_mask_hides_markers");
}

/// Snapshot: with every category except sky glyphs hidden, the glyphs are
/// the only ink left - so their own category keeps drawing them even when
/// the trackline, points, and labels are all off. Run for each variant so
/// both the ring and the disc are exercised through the full map path.
#[rstest::rstest]
#[case::ring("sky_glyphs_only_ring", gt_ui_types::SkyGlyphVariant::Ring)]
#[case::disc("sky_glyphs_only_disc", gt_ui_types::SkyGlyphVariant::Disc)]
fn snap_sky_glyphs_only(#[case] name: &str, #[case] variant: gt_ui_types::SkyGlyphVariant) {
    use gt_ui_types::{DisplayCategory, DisplayMask, TrackDataVisibility};

    let files = vec![make_snapshot_file()];
    let visibility = TrackDataVisibility::from_loaded(&files);
    let mut mask = DisplayMask::default();
    for category in [
        DisplayCategory::Tracks,
        DisplayCategory::TrackPoints,
        DisplayCategory::SatelliteLabels,
        DisplayCategory::CustomMarkers,
        DisplayCategory::GeneratedMarkers,
        DisplayCategory::EventMarkers,
    ] {
        mask.set_visible(category, false);
    }

    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(800.0, 600.0))
        .ui_state(
            move |ui, map: &mut Option<NavMap>| {
                let map =
                    map.get_or_insert_with(|| NavMap::new(ui.ctx().clone(), TileAccess::Offline));
                let mut state = DrawState {
                    display_mask: mask,
                    sky_glyph_variant: variant,
                    ..DrawState::default()
                };
                map.draw(ui, state.context(&files, &visibility));
            },
            None,
        );

    for _ in 0..5 {
        harness.run();
    }
    harness.snapshot_loose(name);
}

/// Snapshot: hovering the time-series plot draws the detailed sky disc at
/// the corresponding map point - even with the sky glyphs overlay hidden,
/// since the plot-hover disc is a focus indicator, not part of the
/// overlay. The ring around the point is the existing cross-highlight.
#[test]
fn snap_plot_hover_sky_disc() {
    use gt_ui_types::{DisplayCategory, DisplayMask, TrackDataVisibility};

    let files = vec![make_snapshot_file()];
    let visibility = TrackDataVisibility::from_loaded(&files);
    // Overlay off, so the only disc on the map is the plot-hover one.
    let mut mask = DisplayMask::default();
    mask.set_visible(DisplayCategory::SkyGlyphs, false);
    // A mid-track point that carries a satellite report in the fixture.
    let hovered = (FileIdx::new(0), TrackIdx::new(0), PointIdx::new(50));

    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(800.0, 600.0))
        .ui_state(
            move |ui, map: &mut Option<NavMap>| {
                let map =
                    map.get_or_insert_with(|| NavMap::new(ui.ctx().clone(), TileAccess::Offline));
                let mut state = DrawState {
                    display_mask: mask,
                    highlight: MapHighlight {
                        plot_hover_point: Some(hovered),
                        ..MapHighlight::default()
                    },
                    ..DrawState::default()
                };
                map.draw(ui, state.context(&files, &visibility));
            },
            None,
        );

    for _ in 0..5 {
        harness.run();
    }
    harness.snapshot_loose("plot_hover_sky_disc");
}

/// Snapshot: the clicked-point window itself - the resizable frame, the
/// sky plot pinned beside the satellite tables, and the deselect hint on
/// the window floor. Guards the whole composition, not just the body.
#[test]
fn snap_sticky_point_window() {
    use gt_ui_types::TrackDataVisibility;

    let files = vec![make_snapshot_file()];
    let visibility = TrackDataVisibility::from_loaded(&files);
    // A mid-track point carrying a multi-constellation satellite report.
    let clicked = gt_ui_types::DataPointRef {
        track: gt_types::TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        category: gt_types::DataCategory::Tpv,
        point_index: PointIdx::new(50),
    };

    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(900.0, 700.0))
        .ui_state(
            move |ui, map: &mut Option<NavMap>| {
                let map =
                    map.get_or_insert_with(|| NavMap::new(ui.ctx().clone(), TileAccess::Offline));
                let mut state = DrawState {
                    highlight: MapHighlight {
                        sticky: Some(clicked),
                        ..MapHighlight::default()
                    },
                    ..DrawState::default()
                };
                map.draw(ui, state.context(&files, &visibility));
            },
            None,
        );

    for _ in 0..5 {
        harness.run();
    }
    harness.snapshot_loose("sticky_point_window");
}

/// The point window's open-trails button has to travel the whole way out
/// of `draw`: through the window body, into a [`SkyTrailsRequest`] carrying
/// the clicked point's instant, and out as a [`MapAction`]. The
/// widget-level test one layer down cannot see this wiring, so a dropped
/// return value here would leave the button a silent no-op.
#[test]
fn the_point_window_button_returns_a_timed_sky_trails_action() {
    use egui_kittest::kittest::Queryable as _;
    use gt_ui_types::TrackDataVisibility;

    let files = vec![make_snapshot_file()];
    let visibility = TrackDataVisibility::from_loaded(&files);
    let clicked = gt_ui_types::DataPointRef {
        track: gt_types::TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        category: gt_types::DataCategory::Tpv,
        point_index: PointIdx::new(50),
    };
    let point_time = files
        .first()
        .and_then(|f| f.tracks.first())
        .and_then(|t| t.points.get(50))
        .map(|p| p.tpv.time())
        .expect("the fixture has a point 50");
    let action = std::rc::Rc::new(Cell::new(None));
    let seen = action.clone();

    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(900.0, 700.0))
        .ui_state(
            move |ui, map: &mut Option<NavMap>| {
                let map =
                    map.get_or_insert_with(|| NavMap::new(ui.ctx().clone(), TileAccess::Offline));
                let mut state = DrawState {
                    highlight: MapHighlight {
                        sticky: Some(clicked),
                        ..MapHighlight::default()
                    },
                    ..DrawState::default()
                };
                let returned = map.draw(ui, state.context(&files, &visibility));
                if returned.is_some() {
                    seen.set(returned);
                }
            },
            None,
        );

    for _ in 0..5 {
        harness.run();
    }
    assert!(action.get().is_none(), "nothing requested before the click");

    harness
        .inner
        .get_by_label(egui_phosphor::regular::ARROW_SQUARE_OUT)
        .click();
    harness.inner.run_steps(2);

    assert_eq!(
        action.get(),
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
    let mut file = make_snapshot_file();
    for track in &mut file.tracks {
        for marker in &mut track.custom_markers {
            marker.label = gt_test_utils::oversized_text('m');
        }
    }
    file
}

/// The sticky popup stays inside the screen whichever map item it pins and
/// however long that item's label reads: the point layout's resizable frame
/// and the auto-sized frame the markers use both scroll their content
/// instead of growing past the screen edge.
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
    use gt_ui_types::TrackDataVisibility;

    let files = vec![file_with_an_overlong_marker_label()];
    let visibility = TrackDataVisibility::from_loaded(&files);
    let clicked = DataPointRef {
        track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        category,
        point_index,
    };

    let mut harness = crate::test_harness::builder().size(viewport).ui_state(
        move |ui, map: &mut Option<NavMap>| {
            let map = map.get_or_insert_with(|| NavMap::new(ui.ctx().clone(), TileAccess::Offline));
            let mut state = DrawState {
                highlight: MapHighlight {
                    sticky: Some(clicked),
                    ..MapHighlight::default()
                },
                ..DrawState::default()
            };
            map.draw(ui, state.context(&files, &visibility));
        },
        None,
    );
    harness.inner.run_steps(8);

    harness
        .inner
        .assert_window_fits_the_viewport(gt_test_utils::AuditedWindow::identified(
            "sticky popup",
            egui::Id::new(("sticky_popup", clicked)),
        ));
}
