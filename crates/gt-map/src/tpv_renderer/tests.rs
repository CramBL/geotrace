use egui::Color32;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use egui_phosphor::regular::ARROW_SQUARE_OUT as ICON_ARROW_SQUARE_OUT;
use gt_fmt::UTC_SECOND_FORMAT;
use gt_loaded_files::{FileHistory, LoadedFiles, RecordingNames};
use gt_sky::{SkyHighlight, SkyPlotSize};
use gt_test_utils::{By, HarnessInteraction as _};
use gt_types::coordinates::{Latitude, Longitude, RecordedLatitude};
use gt_types::satellites::{
    Constellation, ConstellationSet, NO_DATA_SENTINEL_DB_HZ, Prn, Satellite, Satellites, Snr,
};
use gt_types::time_types::{GpsTime, SysTime};
use gt_types::tpv::TimePositionVelocity;
use gt_types::{
    MercPoint, NavPoint, NavigationMetadata, NearbySkyEvidence, PositionProvenance,
    RecordedLongitude, ResolvedPosition, SatelliteCoverage, SatelliteIdentity, SatelliteMembership,
    SatelliteReportMetadata, SatelliteScope, SatelliteUsage, SatelliteUsageReport,
    SolutionDimension, SolutionMethod,
};
use gt_ui_types::{DRAWN_AT_CAPTION, DisplayCategory, PointWindowFolds};
use rstest::rstest;
use uom::si::angle::degree;
use uom::si::f64::{Angle, Length};

use super::pinned_point::{self, MIN_SIDE_BY_SIDE_WIDTH_PX, StickyRegion};
use super::satellite_tables::{MIN_SATELLITE_COLUMN_WIDTH_PX, STICKY_COLUMN_GAP_PX};
use super::sky_context;
use super::*;
use crate::recording_labels::RecordingLabels;
use crate::test_util::{self, MapScene};

/// The instant fix 0 of every fixture track is stamped at. It is a constant,
/// so the snapshots that draw the time row stay deterministic.
fn fixture_epoch() -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp(1_748_000_000, 0).unwrap_or_default()
}

/// A fix `secs` after [`fixture_epoch`] on a heading of 90°, with
/// `satellites` as its report.
fn point_at(secs: i64, satellites: Option<Satellites>) -> NavPoint {
    let time = fixture_epoch() + chrono::Duration::seconds(secs);
    let lat = Latitude::new(FIXTURE_LAT);
    let lon = Longitude::new(FIXTURE_LON);
    match satellites {
        Some(report) => gt_test_utils::fixtures::nav_point_with_report(time, lat, lon, report),
        None => gt_test_utils::fixtures::nav_point_heading(
            time,
            lat,
            lon,
            Some(Angle::new::<degree>(90.0)),
            gt_types::fixtures::FixKind::GhostWithoutHeading,
        ),
    }
}

/// A report of twelve GPS satellites, `in_fix` of them used in the fix.
fn twelve_satellites(in_fix: u32) -> Satellites {
    gt_test_utils::fixtures::satellite_report(
        None,
        gt_types::fixtures::SatelliteCounts {
            in_fix,
            in_view_only: 12 - in_fix,
        },
    )
}

/// A dense, uneven multi-constellation fix - the case the point window was
/// rebuilt for: 40 satellites across four constellations with very
/// different counts (GPS 11, GLONASS 8, Galileo 6, BeiDou 15), so the
/// column packing has something real to balance.
fn sats_dense_multi_constellation() -> Satellites {
    let spec = [
        (Constellation::Gps, 11u32),
        (Constellation::Glonass, 8),
        (Constellation::Galileo, 6),
        (Constellation::Beidou, 15),
    ];
    let mut satellites = Vec::new();
    for (c, (constellation, count)) in spec.into_iter().enumerate() {
        // Offset each constellation's arc so the marks spread across the
        // sky, and vary SNR and fix state so the table shows a realistic mix.
        let offset = f32::from(u16::try_from(c).unwrap_or(0));
        for i in 0..count {
            let n = f32::from(u16::try_from(i).unwrap_or(0));
            let azimuth = (offset * 83.0 + n * 29.0) % 360.0;
            let elevation = 8.0 + (offset * 17.0 + n * 11.0) % 76.0;
            satellites.push(Satellite::new(
                constellation,
                i + 1,
                Some(elevation),
                Some(azimuth),
                Some(28.0 + (offset * 3.0 + n) % 20.0),
                i % 4 != 0,
            ));
        }
    }
    Satellites::new(None, None, satellites)
}

/// A report spanning several constellations with a spread of SNR values,
/// fix membership, and sky positions (two satellites without), so the
/// satellite badge exercises every count tier, the full SNR gradient, a
/// satellite whose receiver reported the no-data SNR value, both the in-fix
/// and idle PRN colours, and the sky plot's placed and unplaceable
/// satellites.
fn sats_multi_constellation() -> Satellites {
    let satellites = vec![
        Satellite::new(
            Constellation::Gps,
            1,
            Some(62.0),
            Some(45.0),
            Some(48.0),
            true,
        ),
        Satellite::new(
            Constellation::Gps,
            2,
            Some(35.0),
            Some(110.0),
            Some(41.0),
            true,
        ),
        Satellite::new(
            Constellation::Gps,
            3,
            Some(18.0),
            Some(305.0),
            Some(33.0),
            true,
        ),
        Satellite::new(Constellation::Gps, 4, Some(12.0), None, Some(22.0), false),
        Satellite::new(
            Constellation::Galileo,
            5,
            Some(55.0),
            Some(80.0),
            Some(37.0),
            true,
        ),
        Satellite::new(
            Constellation::Galileo,
            6,
            Some(25.0),
            Some(220.0),
            Some(14.0),
            false,
        ),
        Satellite::new(Constellation::Glonass, 7, None, None, None, false),
        Satellite::new(
            Constellation::Glonass,
            8,
            Some(45.0),
            Some(240.0),
            Some(NO_DATA_SENTINEL_DB_HZ),
            true,
        ),
        Satellite::new(
            Constellation::Beidou,
            8,
            Some(65.0),
            Some(275.0),
            Some(45.0),
            true,
        ),
    ];
    Satellites::new(None, None, satellites)
}

/// Where the map draws a fixture point, resolved over a one-fix track the way
/// the point window reaches it.
fn placement_of(point: &NavPoint) -> FixPlacement {
    FixPlacement::resolve(
        &gt_test_utils::loaded_track_with_points(vec![point.clone()]),
        PointIdx::new(0),
    )
}

/// The sticky content's sky section for a fixture point: its own report
/// when it has one.
fn sky_for(point: &NavPoint) -> SkySection<'_> {
    point.sky_evidence(SkyGeometry::Identities).map_or(
        SkySection::TrackWithoutReports,
        |evidence| {
            SkySection::Report(NearbySkyEvidence {
                evidence,
                point_index: PointIdx::new(0),
                age: chrono::Duration::zero(),
            })
        },
    )
}

/// Snapshot: a 40-satellite, 4-constellation fix. The two columns are cut
/// where they even out, so the uneven constellations pack tight, and the plot
/// stays beside them.
#[test]
fn dense_multi_constellation_packs_into_two_columns() {
    let point = point_at(0, Some(sats_dense_multi_constellation()));
    let mut folds = gt_ui_types::PointWindowFolds::default();
    let placement = placement_of(&point);
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(620.0, 560.0))
        .theme(true)
        .ui(move |ui| {
            let _opened =
                show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None, placement);
        });
    harness.snapshot("sticky_dense_two_columns");
}

#[test]
fn dense_multi_constellation_reflows_to_one_column_when_narrow() {
    let point = point_at(0, Some(sats_dense_multi_constellation()));
    let mut folds = gt_ui_types::PointWindowFolds::default();
    let placement = placement_of(&point);
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(330.0, 560.0))
        .theme(true)
        .ui(move |ui| {
            let _opened =
                show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None, placement);
        });
    harness.snapshot_with_color_tolerance("sticky_dense_one_column");
}

/// Snapshot: a folded plot and two folded constellations. Each folded
/// header keeps its colour, name and fix/seen count, so the overview
/// survives folding - only the rows go away.
#[test]
fn folded_sections_keep_their_headers() {
    let point = point_at(0, Some(sats_dense_multi_constellation()));
    let mut folds = gt_ui_types::PointWindowFolds {
        plot_folded: true,
        ..Default::default()
    };
    let placement = placement_of(&point);
    folds.toggle(Constellation::Gps);
    folds.toggle(Constellation::Beidou);
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(620.0, 380.0))
        .theme(true)
        .ui(move |ui| {
            let _opened =
                show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None, placement);
        });
    harness.snapshot("sticky_folded_sections");
}

/// Folding a constellation drops its satellite rows while its header
/// stays, so the window shrinks without hiding what is there.
#[rstest]
#[case::unfolded(false, true)]
#[case::folded(true, false)]
fn folding_a_constellation_hides_only_its_rows(#[case] fold_gps: bool, #[case] expect_rows: bool) {
    let point = point_at(0, Some(sats_dense_multi_constellation()));
    let mut folds = gt_ui_types::PointWindowFolds::default();
    let placement = placement_of(&point);
    if fold_gps {
        folds.toggle(Constellation::Gps);
    }
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(620.0, 560.0))
        .theme(true)
        .ui(move |ui| {
            let _opened =
                show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None, placement);
        });
    harness.run();

    // The header survives either way. Only the PRN rows come and go.
    assert!(
        harness.inner.query_by_label("GPS").is_some(),
        "the constellation header must stay visible when folded"
    );
    assert_eq!(harness.inner.query_by_label("G01").is_some(), expect_rows);
}

#[test]
fn clicking_anywhere_on_the_header_folds() {
    let point = point_at(0, Some(sats_multi_constellation()));
    let folded = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = folded.clone();
    let mut folds = gt_ui_types::PointWindowFolds::default();
    let placement = placement_of(&point);
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(600.0, 440.0))
        .theme(true)
        .ui(move |ui| {
            let _opened =
                show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None, placement);
            seen.set(folds.is_folded(Constellation::Gps));
        });
    harness.run();
    assert!(!folded.get(), "starts unfolded");
    harness.inner.get_by_label("GPS").click();
    harness.inner.run_steps(2);
    assert!(folded.get(), "clicking the header should fold GPS");
}

/// The open-trails button sits inside the sky header's fold click target,
/// so pressing it must open the trails window without folding the plot out
/// from under the pointer.
#[test]
fn the_open_trails_button_does_not_fold_the_sky_plot() {
    let point = point_at(0, Some(sats_multi_constellation()));
    let state = std::rc::Rc::new(std::cell::Cell::new((false, false)));
    let seen = state.clone();
    let mut folds = gt_ui_types::PointWindowFolds::default();
    let placement = placement_of(&point);
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(600.0, 440.0))
        .theme(true)
        .ui(move |ui| {
            let opened =
                show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None, placement);
            let (ever_opened, _) = seen.get();
            seen.set((ever_opened || opened, folds.plot_folded));
        });
    harness.run();
    assert_eq!(
        state.get(),
        (false, false),
        "nothing opened before the click"
    );

    harness.inner.get_by_label(ICON_ARROW_SQUARE_OUT).click();
    harness.inner.run_steps(2);

    let (opened, folded) = state.get();
    assert!(opened, "the button must request the sky trails window");
    assert!(!folded, "the button must not fold the sky plot");
}

/// Each header folds its own constellation. Sibling panels lay out
/// identically, so an auto-generated interaction id collides across them
/// and a click lands on the wrong panel. This pins the second panel
/// folding itself and leaving the first alone.
#[test]
fn each_header_folds_its_own_constellation() {
    let point = point_at(0, Some(sats_multi_constellation()));
    let state = std::rc::Rc::new(std::cell::Cell::new((false, false)));
    let seen = state.clone();
    let mut folds = gt_ui_types::PointWindowFolds::default();
    let placement = placement_of(&point);
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(600.0, 440.0))
        .theme(true)
        .ui(move |ui| {
            let _opened =
                show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None, placement);
            seen.set((
                folds.is_folded(Constellation::Gps),
                folds.is_folded(Constellation::Glonass),
            ));
        });
    harness.run();

    harness.inner.get_by_label("GLONASS").click();
    harness.inner.run_steps(2);

    let (gps, glonass) = state.get();
    assert!(glonass, "clicking GLONASS must fold GLONASS");
    assert!(!gps, "clicking GLONASS must not fold GPS");
}

/// Sliding down the satellite table must not drop the sky highlight in the
/// spacing between rows. It used to: the gap hovered nothing, so the plot
/// flashed back to full strength between one satellite and the next.
#[test]
fn the_gap_between_satellite_rows_keeps_the_highlight() {
    let point = point_at(0, Some(sats_multi_constellation()));
    let id_cell = std::rc::Rc::new(std::cell::Cell::new(None));
    let cell = id_cell.clone();
    let mut folds = gt_ui_types::PointWindowFolds::default();
    let placement = placement_of(&point);
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(600.0, 440.0))
        .theme(true)
        .ui(move |ui| {
            cell.set(Some(pinned_point::sky_table_highlight_id(ui)));
            let _opened =
                show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None, placement);
        });
    harness.run();

    let first = harness.inner.get_by_label("G01").rect();
    let second = harness.inner.get_by_label("G02").rect();
    assert!(
        second.top() > first.bottom(),
        "rows must actually be spaced apart, or this proves nothing"
    );

    // Dead centre of the strip between the two rows.
    harness.inner.hover_at(egui::pos2(
        first.center().x,
        (first.bottom() + second.top()) / 2.0,
    ));
    harness.inner.run_steps(2);

    let id = id_cell.get().expect("sticky content rendered");
    let highlight: Option<SkyHighlight> = harness.inner.ctx.data(|d| d.get_temp(id)).flatten();
    assert!(
        highlight.is_some(),
        "the gap between rows must hand the highlight from one row to the next"
    );
}

/// The satellite badge (counts, SNR gradient, PRN colours) must stay
/// legible on both themes. These render the same content under light and
/// dark visuals. The light baseline is what catches colours that only read
/// on a dark surface.
#[rstest]
#[case::dark("satellite_badge_dark", true)]
#[case::light("satellite_badge_light", false)]
fn satellite_badge(#[case] name: &str, #[case] dark_mode: bool) {
    let point = point_at(0, Some(sats_multi_constellation()));
    let mut folds = gt_ui_types::PointWindowFolds::default();
    let placement = placement_of(&point);
    let mut harness = test_util::harness_builder()
        .size(STICKY_CONTENT_CANVAS)
        .theme(dark_mode)
        .ui(move |ui| {
            let _opened =
                show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None, placement);
        });
    harness.snapshot(name);
}

/// Hovering an element of the satellite tables stores the matching sky
/// highlight, which the plot reads back the next frame. Drives the real
/// hover path end to end: the label lookup, the response hit-test, and
/// the `ctx.data` round trip keyed by [`pinned_point::sky_table_highlight_id`].
#[rstest]
#[case::prn_row(
    "G01",
    SkyHighlight::satellite(Constellation::Gps, gt_types::satellites::Prn::new(1))
)]
#[case::constellation_header("GPS", SkyHighlight::constellation(Constellation::Gps))]
fn hovering_a_table_sets_the_sky_highlight(#[case] label: &str, #[case] expected: SkyHighlight) {
    let point = point_at(0, Some(sats_multi_constellation()));
    let id_cell = std::rc::Rc::new(std::cell::Cell::new(None));
    let cell = id_cell.clone();
    let mut folds = gt_ui_types::PointWindowFolds::default();
    let placement = placement_of(&point);
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(320.0, 920.0))
        .theme(true)
        .ui(move |ui| {
            cell.set(Some(pinned_point::sky_table_highlight_id(ui)));
            let _opened =
                show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None, placement);
        });
    harness.run();
    harness.inner.hover_and_settle(By::new().label(label), 2);

    let id = id_cell.get().expect("sticky content rendered");
    let highlight: Option<SkyHighlight> = harness.inner.ctx.data(|d| d.get_temp(id)).flatten();
    assert_eq!(highlight, Some(expected));
}

/// Hovering a highlight target paints a band over it - the affordance
/// that it does something.
#[test]
fn hovering_a_prn_row_shows_the_affordance_band() {
    let point = point_at(0, Some(sats_multi_constellation()));
    let mut folds = gt_ui_types::PointWindowFolds::default();
    let placement = placement_of(&point);
    let mut harness = test_util::harness_builder()
        .size(STICKY_CONTENT_CANVAS)
        .theme(true)
        .ui(move |ui| {
            let _opened =
                show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None, placement);
        });
    harness.run();
    harness.inner.hover_and_settle(By::new().label("G01"), 2);
    harness.snapshot("sticky_prn_row_hovered");
}

/// The fix at `index` of `track` with the position the track builder placed
/// it at, the way the map's hover reaches it.
fn placed_point(track: &LoadedTrack, index: usize) -> Option<gt_types::PlacedPoint<'_>> {
    track.placed_points()?.get(index)
}

/// A report whose satellites carry sky positions, so the badge's compact
/// sky plot has marks to place, plus one unplaceable satellite.
fn sats_with_sky() -> Satellites {
    let satellites = vec![
        Satellite::new(
            Constellation::Gps,
            5,
            Some(62.0),
            Some(45.0),
            Some(44.0),
            true,
        ),
        Satellite::new(
            Constellation::Gps,
            12,
            Some(35.0),
            Some(110.0),
            Some(38.0),
            true,
        ),
        Satellite::new(
            Constellation::Gps,
            29,
            Some(12.0),
            Some(155.0),
            Some(24.0),
            false,
        ),
        Satellite::new(
            Constellation::Galileo,
            3,
            Some(55.0),
            Some(80.0),
            Some(42.0),
            true,
        ),
        Satellite::new(
            Constellation::Beidou,
            14,
            Some(65.0),
            Some(275.0),
            Some(41.0),
            true,
        ),
        Satellite::new(Constellation::Qzss, 1, Some(50.0), None, Some(36.0), false),
    ];
    Satellites::new(None, None, satellites)
}

/// The hover table over a fix: its own report in both themes, a report
/// borrowed from a fix nearby, no report near enough to borrow, a track that
/// records none at all, and a fix whose recorded latitude is out of range,
/// which the table marks above the position the map draws it at.
#[rstest]
#[case::own_report_dark(
    "hover_badge_own_report_dark",
    true,
    vec![point_at(0, Some(sats_with_sky()))],
    0
)]
#[case::own_report_light(
    "hover_badge_own_report_light",
    false,
    vec![point_at(0, Some(sats_with_sky()))],
    0
)]
#[case::borrowed_report(
    "hover_badge_borrowed_report",
    true,
    vec![point_at(0, Some(sats_with_sky())), point_at(3, None)],
    1
)]
#[case::no_report_nearby(
    "hover_badge_no_report_nearby",
    true,
    vec![point_at(0, Some(sats_with_sky())), point_at(60, None)],
    1
)]
#[case::track_without_reports(
    "hover_badge_track_without_reports",
    true,
    vec![point_at(0, None)],
    0
)]
#[case::coordinate_out_of_range(
    "hover_badge_coordinate_out_of_range",
    true,
    gt_test_utils::fixtures::nav_points_with_a_latitude_out_of_range(3, PointIdx::new(1)),
    1
)]
fn snap_hover_badge(
    #[case] name: &str,
    #[case] dark_mode: bool,
    #[case] points: Vec<NavPoint>,
    #[case] hovered: usize,
) {
    let track = gt_test_utils::loaded_track_with_points(points);
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(430.0, 260.0))
        .theme(dark_mode)
        .ui(move |ui| {
            let sky = SkySection::resolve(&track, PointIdx::new(hovered));
            if let Some(point) = placed_point(&track, hovered) {
                show_hover_table(ui, point, &sky, None);
            }
        });
    harness.snapshot(name);
}

/// A fix the receiver wrote a latitude of 91° for, with the heading it
/// reported for it if any.
fn fix_with_a_latitude_out_of_range(heading_degrees: Option<f64>) -> NavPoint {
    let tpv = TimePositionVelocity::builder()
        .time(GpsTime::from_utc(fixture_epoch()))
        .lat(RecordedLatitude::from_degrees(91.0))
        .lon(Longitude::new(FIXTURE_LON))
        .maybe_heading(heading_degrees.map(Angle::new::<degree>))
        .build();
    NavPoint::new(tpv, None)
}

/// What the point window says under the two recorded coordinates.
#[derive(Debug, Clone, Copy, PartialEq)]
enum PlacementRow {
    /// The receiver's own coordinates place the fix, so neither row is drawn.
    Absent,
    DrawnAt,
    NotDrawn,
}

/// The point window writes both coordinates the receiver recorded, marking one
/// outside its range, and names where the map draws the fix.
#[rstest]
#[case::measured(
    gt_test_utils::fixtures::nav_points_with_a_latitude_out_of_range(3, PointIdx::new(1)),
    PointIdx::new(0),
    "55.000000° N",
    PlacementRow::Absent
)]
#[case::latitude_out_of_range(
    gt_test_utils::fixtures::nav_points_with_a_latitude_out_of_range(3, PointIdx::new(1)),
    PointIdx::new(1),
    "91° (invalid)",
    PlacementRow::DrawnAt
)]
#[case::track_without_a_position(
    gt_test_utils::fixtures::nav_points_without_a_valid_position(3),
    PointIdx::new(1),
    "91° (invalid)",
    PlacementRow::NotDrawn
)]
fn the_point_window_names_the_recorded_coordinates(
    #[case] points: Vec<NavPoint>,
    #[case] point_index: PointIdx,
    #[case] expected_latitude: &str,
    #[case] expected_placement: PlacementRow,
) {
    let track = gt_test_utils::loaded_track_with_points(points);
    let mut folds = gt_ui_types::PointWindowFolds::default();
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(430.0, 300.0))
        .theme(true)
        .ui(move |ui| {
            let Some(point) = point_index.get(&track.points) else {
                return;
            };
            let _opened = show_sticky_tpv_content(
                ui,
                point,
                &SkySection::resolve(&track, point_index),
                &mut folds,
                None,
                FixPlacement::resolve(&track, point_index),
            );
        });
    harness.run();

    assert!(harness.inner.query_by_label(expected_latitude).is_some());
    assert!(harness.inner.query_by_label("Lon").is_some());
    assert_eq!(
        harness.inner.query_by_label("Drawn at").is_some(),
        expected_placement == PlacementRow::DrawnAt
    );
    assert_eq!(
        harness.inner.query_by_label("Not drawn").is_some(),
        expected_placement == PlacementRow::NotDrawn
    );
}

/// The recording row states the file a fix came from, and is absent while
/// a single file is loaded.
#[rstest]
#[case::several_files(Some("Morning ride"), true)]
#[case::single_file(None, false)]
fn hover_badge_recording_row(
    #[case] recording_name: Option<&'static str>,
    #[case] expect_row: bool,
) {
    let track = gt_test_utils::loaded_track_with_points(vec![point_at(0, Some(sats_with_sky()))]);
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(430.0, 260.0))
        .theme(true)
        .ui(move |ui| {
            let sky = SkySection::resolve(&track, PointIdx::new(0));
            if let Some(point) = placed_point(&track, 0) {
                show_hover_table(ui, point, &sky, recording_name);
            }
        });
    harness.run();

    assert_eq!(
        harness.inner.query_by_label("Recording").is_some(),
        expect_row
    );
    assert_eq!(
        harness.inner.query_by_label("Morning ride").is_some(),
        expect_row
    );
}

#[rstest]
#[case::earlier(2100, "Sky -2.1s")]
#[case::later(-2100, "Sky +2.1s")]
fn report_age_label_names_the_side(#[case] ms: i64, #[case] expected: &str) {
    assert_eq!(
        sky_context::report_age_label(chrono::Duration::milliseconds(ms)),
        expected
    );
}

fn a_fix_without_a_heading() -> NavPoint {
    gt_test_utils::fixtures::nav_point(
        fixture_epoch(),
        Latitude::new(FIXTURE_LAT),
        Longitude::new(FIXTURE_LON),
        gt_types::fixtures::FixKind::GhostWithoutHeading,
    )
}

#[rstest]
#[case::without_a_report(None, FIX_UNKNOWN_GRAY)]
#[case::ten_in_fix(Some(twelve_satellites(10)), Color32::from_rgb(66, 133, 244))]
#[case::one_in_fix(Some(twelve_satellites(1)), FIX_BLUE)]
#[case::nothing_in_fix(Some(twelve_satellites(0)), Color32::from_rgb(219, 68, 55))]
fn tpv_point_color_states_the_resolved_navigation_state(
    #[case] satellites: Option<Satellites>,
    #[case] expected: Color32,
) {
    assert_eq!(tpv_point_color(&point_at(0, satellites)), expected);
}

#[rstest]
#[case::measured(point_at(0, None), None)]
#[case::without_a_heading(a_fix_without_a_heading(), None)]
#[case::with_nothing_in_fix(
    point_at(0, Some(twelve_satellites(0))),
    Some(ChevronFix::NavigationLoss)
)]
#[case::latitude_out_of_range(
    fix_with_a_latitude_out_of_range(Some(90.0)),
    Some(ChevronFix::CoordinateOutOfRange)
)]
#[case::out_of_range_without_a_heading(
    fix_with_a_latitude_out_of_range(None),
    Some(ChevronFix::CoordinateOutOfRange)
)]
fn a_fix_the_receiver_did_not_measure_is_drawn_as_a_chevron(
    #[case] mut fix: NavPoint,
    #[case] expected: Option<ChevronFix>,
) {
    if fix.tpv.position().is_none() {
        fix.navigation.state = Some(SolutionState::EstimatedOnly);
        fix.navigation.position_provenance = Some(PositionProvenance::SdkSynthesized);
    }
    assert_eq!(ChevronFix::for_fix(&fix), expected);
}

/// The chevron points along the line between the fixes either side of it.
/// Mercator y grows southward, and the southward case pins that no y flip is
/// applied. Two coincident neighbours give a fallback of down.
#[rstest]
#[case::eastward(
    MercPoint { x: 0.50, y: 0.50 },
    MercPoint { x: 0.60, y: 0.50 },
    Vec2::new(1.0, 0.0)
)]
#[case::southward(
    MercPoint { x: 0.50, y: 0.40 },
    MercPoint { x: 0.50, y: 0.60 },
    Vec2::new(0.0, 1.0)
)]
#[case::coincident_neighbours(
    MercPoint { x: 0.5, y: 0.5 },
    MercPoint { x: 0.5, y: 0.5 },
    Vec2::DOWN
)]
fn chevron_direction_follows_the_neighbouring_fixes(
    #[case] previous: MercPoint,
    #[case] next: MercPoint,
    #[case] expected: Vec2,
) {
    let direction = chevron_direction(previous, next);
    assert!(
        (direction - expected).length() < 0.01,
        "got {direction:?}, expected {expected:?}"
    );
}

/// Arrows a spacing apart fade linearly from opaque at the top of the band to
/// invisible at its foot. A degenerate icon size and a spacing that overflowed
/// to infinity both clamp the alpha to opaque.
#[rstest]
#[case::far_apart(100.0, TEST_ICON_PX, 1.0)]
#[case::side_by_side(12.0, TEST_ICON_PX, 1.0)]
#[case::overlapping_a_little(8.0, TEST_ICON_PX, 1.0)]
#[case::at_the_upper_bound(6.0, TEST_ICON_PX, 1.0)]
#[case::midway_through_the_band(4.2, TEST_ICON_PX, 0.5)]
#[case::at_the_lower_bound(2.4, TEST_ICON_PX, 0.0)]
#[case::stacked_on_one_point(0.0, TEST_ICON_PX, 0.0)]
#[case::spacing_overflowed_to_infinity(f32::INFINITY, TEST_ICON_PX, 1.0)]
#[case::icon_of_no_size(10.0, 0.0, 1.0)]
#[case::icon_of_negative_size(10.0, -1.0, 1.0)]
#[case::below_the_small_icon_floor(1.2, SMALL_ICON_PX, 0.0)]
#[case::at_the_small_icon_lower_floor(2.0, SMALL_ICON_PX, 0.0)]
#[case::at_the_small_icon_upper_floor(5.0, SMALL_ICON_PX, 1.0)]
#[case::midway_through_the_floored_band(3.5, SMALL_ICON_PX, 0.5)]
fn icon_fade_alpha_ramps_across_the_fade_band(
    #[case] spacing_px: f32,
    #[case] icon_px: f32,
    #[case] expected: f32,
) {
    let alpha = icon_fade_alpha(spacing_px, icon_px);
    assert!((alpha - expected).abs() < 1e-6, "got {alpha}");
}

/// Which fade pass a track takes is read off its segment length range against
/// the fade band of the icon size the zoom draws at.
#[rstest]
// No segment at all means nothing can overlap: a spacing of zero would hide
// a lone fix forever.
#[case::a_lone_fix(None, TEST_ICON_PX, TrackIconFade::AllVisible)]
// Longest segment 2 m = 2 px, below the 2.4 px fade-out bound.
#[case::every_segment_blends(Some((0.0, 2.0)), TEST_ICON_PX, TrackIconFade::AllHidden)]
// Shortest segment 6 m = 6 px, exactly the fade-in bound.
#[case::every_segment_is_spaced(Some((6.0, 100.0)), TEST_ICON_PX, TrackIconFade::AllVisible)]
// Parked then highway: zero-length segments next to 100 m hops.
#[case::mixed_spacing(Some((0.0, 100.0)), TEST_ICON_PX, TrackIconFade::PerFix)]
// A range entirely inside the fade band is per-fix as well.
#[case::inside_the_band(Some((3.0, 5.0)), TEST_ICON_PX, TrackIconFade::PerFix)]
// 1.9 m segments at 1 px/m: below the 2 px floor, fully hidden even though
// 1.9 px is well above 0.2 x 3 px.
#[case::below_the_small_icon_floor(Some((0.0, 1.9)), SMALL_ICON_PX, TrackIconFade::AllHidden)]
#[case::above_the_small_icon_floor(Some((5.0, 50.0)), SMALL_ICON_PX, TrackIconFade::AllVisible)]
#[case::inside_the_floored_band(Some((3.0, 4.0)), SMALL_ICON_PX, TrackIconFade::PerFix)]
fn classify_icon_fade_reads_the_segment_length_range(
    #[case] segment_range_m: Option<(f64, f64)>,
    #[case] icon_px: f32,
    #[case] expected: TrackIconFade,
) {
    let track = match segment_range_m {
        Some((min_m, max_m)) => track_with_segment_range(min_m, max_m),
        None => gt_test_utils::loaded_track_with_points(Vec::new()),
    };
    assert_eq!(
        classify_icon_fade(&track, unit_transform().scale(), icon_px),
        expected
    );
}

fn unit_transform() -> crate::transform::MercTransform {
    crate::transform::MercTransform::for_test(EARTH_CIRCUMFERENCE_M)
}

/// A real fix on the equator, `x_m` metres east of the origin, reporting a
/// horizontal accuracy of `eph_m` metres where it has one.
fn nav_point_at_meters(x_m: f64, eph_m: Option<f32>) -> NavPoint {
    let tpv = TimePositionVelocity::builder()
        .time(GpsTime::from_utc(chrono::Utc::now()))
        .lat(Latitude::new(0.0))
        .lon(Longitude::new(x_m * 360.0 / EARTH_CIRCUMFERENCE_M))
        .heading(Angle::new::<degree>(90.0))
        .maybe_eph_m(eph_m)
        .build();
    NavPoint::new(tpv, None)
}

fn track_with_segment_range(min_m: f64, max_m: f64) -> LoadedTrack {
    let bounding_box =
        gt_types::GeoBounds::single_position(Latitude::new(0.0), gt_types::Longitude::new(0.0));
    LoadedTrack {
        metadata: gt_test_utils::empty_track_metadata(),
        geometry: gt_types::TrackGeometry::Measured(gt_types::MeasuredTrackGeometry {
            resolved_positions: Vec::new(),
            bounding_box,
            merc_bounds: gt_types::MercBounds::from(bounding_box),
            distance_km: Length::new::<uom::si::length::kilometer>(0.0),
            point_set_diameter_m: Length::new::<meter>(0.0),
            segment_length_range: Some(gt_types::SegmentLengthRange {
                min: Length::new::<meter>(min_m),
                max: Length::new::<meter>(max_m),
            }),
        }),
        points: Vec::new(),
        lod: gt_types::TrackLod::default(),
        sat_label_anchors: Vec::new(),
        custom_markers: Vec::new(),
        generated_markers: Vec::new(),
        event_markers: Vec::new(),
        channels: Vec::new(),
    }
}

fn spacing_at(track: &LoadedTrack, pi: usize) -> Option<f32> {
    let transform = unit_transform();
    let placed = track.placed_points()?;
    let screen_pos = transform.to_screen(placed.get(pi)?.merc());
    local_fix_spacing_px(placed, pi, screen_pos, &transform)
}

/// A fix's local spacing is the shorter distance to a neighbour, and is
/// absent for a fix with no neighbour at all. Three stacked fixes followed by
/// a 100 m hop leave the interior of the cluster at zero, while the fix the
/// hop departs from sees its far neighbour and stays visible.
#[rstest]
#[case::a_lone_fix(&[0.0], 0, None)]
#[case::the_first_of_two(&[0.0, 100.0], 0, Some(100.0))]
#[case::the_last_of_two(&[0.0, 100.0], 1, Some(100.0))]
#[case::inside_a_cluster(&[0.0, 0.0, 0.0, 100.0], 1, Some(0.0))]
#[case::the_fix_a_hop_departs_from(&[0.0, 0.0, 0.0, 100.0], 2, Some(100.0))]
fn local_fix_spacing_px_reads_the_nearer_neighbour(
    #[case] positions_m: &[f64],
    #[case] fix_index: usize,
    #[case] expected_px: Option<f32>,
) {
    let points = positions_m
        .iter()
        .map(|&x_m| nav_point_at_meters(x_m, None))
        .collect();
    let track = gt_test_utils::loaded_track_with_points(points);
    let spacing = spacing_at(&track, fix_index);
    let within_a_pixel = match (spacing, expected_px) {
        (None, None) => true,
        (Some(actual), Some(expected)) => (actual - expected).abs() < 1.0,
        (None, Some(_)) | (Some(_), None) => false,
    };
    assert!(
        within_a_pixel,
        "got {spacing:?} px, expected {expected_px:?} px"
    );
}

#[test]
fn fix_icon_alpha_short_circuits_uniform_tracks() {
    let track = gt_test_utils::loaded_track_with_points(vec![
        nav_point_at_meters(0.0, None),
        nav_point_at_meters(0.0, None),
    ]);
    let placed = track.placed_points().expect("the fixture track is placed");
    let transform = unit_transform();
    let pos = transform.to_screen(placed.get(0).expect("the first fix").merc());
    // AllHidden / AllVisible ignore local spacing entirely.
    let hidden = fix_icon_alpha(
        TrackIconFade::AllHidden,
        placed,
        0,
        pos,
        TEST_ICON_PX,
        &transform,
    );
    let visible = fix_icon_alpha(
        TrackIconFade::AllVisible,
        placed,
        0,
        pos,
        TEST_ICON_PX,
        &transform,
    );
    assert!(hidden <= 0.0);
    assert!(visible >= 1.0);
}

#[test]
fn per_fix_alpha_handles_parked_highway_parked() {
    // The shape from the bug report: parked (stacked fixes), then
    // highway (100 m hops), then parked again. Parked interiors fade,
    // every highway fix and both cluster boundary fixes stay opaque.
    let track = gt_test_utils::loaded_track_with_points(vec![
        nav_point_at_meters(0.0, None),
        nav_point_at_meters(0.0, None),
        nav_point_at_meters(0.0, None), // departure: next neighbour is far
        nav_point_at_meters(100.0, None),
        nav_point_at_meters(200.0, None),
        nav_point_at_meters(300.0, None), // arrival: prev neighbour is far
        nav_point_at_meters(300.0, None),
        nav_point_at_meters(300.0, None),
    ]);
    let placed = track.placed_points().expect("the fixture track is placed");
    let transform = unit_transform();
    let alpha_at = |pi: usize| {
        let pos = transform.to_screen(placed.get(pi).expect("a fix at pi").merc());
        fix_icon_alpha(
            TrackIconFade::PerFix,
            placed,
            pi,
            pos,
            TEST_ICON_PX,
            &transform,
        )
    };
    // Parked interiors (including the track ends) are fully faded.
    assert!(alpha_at(0) <= 0.0);
    assert!(alpha_at(1) <= 0.0);
    assert!(alpha_at(6) <= 0.0);
    assert!(alpha_at(7) <= 0.0);
    // Departure, highway, and arrival fixes are fully opaque.
    for pi in 2..=5 {
        assert!(alpha_at(pi) >= 1.0, "fix {pi} should be opaque");
    }
}

#[test]
fn line_alpha_buckets_quantize_the_crossfade() {
    assert_eq!(line_alpha_bucket(0.0), 0);
    assert_eq!(line_alpha_bucket(0.16), 0); // rounds down: still invisible
    assert_eq!(line_alpha_bucket(0.34), 1);
    assert_eq!(line_alpha_bucket(0.5), 2); // rounds half away from zero
    assert_eq!(line_alpha_bucket(1.0), QUALITY_LINE_ALPHA_STEPS);
    // Out-of-range inputs clamp.
    assert_eq!(line_alpha_bucket(-1.0), 0);
    assert_eq!(line_alpha_bucket(2.0), QUALITY_LINE_ALPHA_STEPS);
    assert!((bucket_alpha(QUALITY_LINE_ALPHA_STEPS) - 1.0).abs() < f32::EPSILON);
    assert!(bucket_alpha(0) < f32::EPSILON);
}

#[test]
fn sub_span_ranges_of_one_projected_key_is_one_range() {
    // The points form one run: the alpha bucket changes along the span, but
    // the projection depends on the color alone.
    let span = [
        ((Color32::BLUE, 3_u8), egui::pos2(0.0, 0.0)),
        ((Color32::BLUE, 0_u8), egui::pos2(10.0, 0.0)),
        ((Color32::BLUE, 3_u8), egui::pos2(20.0, 0.0)),
    ];
    let ranges: Vec<_> = sub_span_ranges(&span, |(color, _)| color).collect();
    assert_eq!(ranges, vec![(Color32::BLUE, 0..=2)]);
}

#[test]
fn sub_span_ranges_split_at_a_key_change_share_their_boundary_index() {
    // Same quality color, different crossfade buckets: an opaque stretch (a
    // parked cluster, bucket 3) and an invisible one (well-spaced fixes,
    // bucket 0) get separate strokes, which localizes the quality line to
    // the cluster. The bucket-3 range ends at index 2: the edge into the
    // first bucket-0 point takes bucket 3, the key of its starting point.
    let span = [
        ((Color32::BLUE, 3_u8), egui::pos2(0.0, 0.0)),
        ((Color32::BLUE, 3_u8), egui::pos2(10.0, 0.0)),
        ((Color32::BLUE, 0_u8), egui::pos2(20.0, 0.0)),
        ((Color32::BLUE, 0_u8), egui::pos2(30.0, 0.0)),
    ];
    let ranges: Vec<_> = sub_span_ranges(&span, |k| k).collect();
    assert_eq!(
        ranges,
        vec![
            ((Color32::BLUE, 3_u8), 0..=2),
            ((Color32::BLUE, 0_u8), 2..=3),
        ]
    );
}

#[test]
fn sub_span_ranges_of_a_new_key_at_every_point_is_one_range_per_edge() {
    // The three ranges over these four points are keyed blue, yellow and
    // red: each edge takes the key of its starting point.
    let span = [
        (Color32::BLUE, egui::pos2(0.0, 0.0)),
        (Color32::YELLOW, egui::pos2(10.0, 0.0)),
        (Color32::RED, egui::pos2(20.0, 0.0)),
        (Color32::GREEN, egui::pos2(30.0, 0.0)),
    ];
    let ranges: Vec<_> = sub_span_ranges(&span, |k| k).collect();
    assert_eq!(
        ranges,
        vec![
            (Color32::BLUE, 0..=1),
            (Color32::YELLOW, 1..=2),
            (Color32::RED, 2..=3),
        ]
    );
}

#[test]
fn sub_span_ranges_of_a_span_without_an_edge_is_empty() {
    assert_eq!(sub_span_ranges::<Color32, Color32>(&[], |k| k).count(), 0);
    assert_eq!(
        sub_span_ranges(&[(Color32::BLUE, egui::pos2(0.0, 0.0))], |k| k).count(),
        0
    );
}

#[rstest]
#[case::arrows_with_accuracy(true, Some(10.0), 8)]
#[case::dots_with_accuracy(false, Some(10.0), 8)]
#[case::dots_with_only_hdop(false, None, 0)]
fn ordinary_icons_share_one_batch_after_accuracy_circles(
    #[case] with_heading: bool,
    #[case] eph_m: Option<f32>,
    #[case] expected_circles: usize,
) {
    const FIX_COUNT: usize = 8;
    const SPACING_M: f64 = 20.0;

    let points = (0..FIX_COUNT)
        .map(|i| {
            let tpv = TimePositionVelocity::builder()
                .time(GpsTime::from_utc(fixture_epoch()))
                .lat(Latitude::new(0.0))
                .lon(Longitude::new(
                    i as f64 * SPACING_M * 360.0 / EARTH_CIRCUMFERENCE_M,
                ))
                .maybe_heading(with_heading.then_some(Angle::new::<degree>(90.0)))
                .maybe_eph_m(eph_m)
                .build();
            let mut point = NavPoint::new(tpv, None);
            point.navigation.hdop = Some(0.9);
            point
        })
        .collect();
    let track = gt_test_utils::loaded_track_with_points(points);
    let indices: Vec<usize> = (0..FIX_COUNT).collect();
    let library = crate::icon_mesh::IconMeshLibrary::embedded().ok();
    let style = TpvDrawStyle {
        outline_alpha: 1.0,
        base_arrow_size: 8.0,
        icon_alpha: 1.0,
    };
    let transform = crate::transform::MercTransform::for_test_view(
        EARTH_CIRCUMFERENCE_M,
        Latitude::new(0.0),
        Longitude::new(0.0),
        egui::pos2(100.0, 100.0),
    );

    let mut harness = gt_test_utils::TestHarness::builder()
        .size(egui::vec2(400.0, 200.0))
        .ui(move |ui| {
            draw_track_icons(
                ui,
                ui.max_rect(),
                FileIdx::new(0),
                TrackIdx::new(0),
                &track,
                Some(&indices),
                &[],
                &style,
                TrackIconFade::AllVisible,
                &transform,
                &MapHighlight::default(),
                &|_| true,
                library.as_ref(),
            );
        });
    harness.run();

    let shapes = &harness.inner.output().shapes;
    let circles = shapes
        .iter()
        .filter(|s| matches!(s.shape, egui::Shape::Circle(_)))
        .count();
    let meshes = shapes
        .iter()
        .filter(|s| matches!(s.shape, egui::Shape::Mesh(_)))
        .count();
    assert_eq!((circles, meshes), (expected_circles, 1));
}

/// Canvas the sticky-content snapshots render into: as wide as the real point
/// window, where the plot sits beside the satellite tables, and tall enough to
/// hold the whole plot column. A column clipped at its scroll edge leaves a
/// part-drawn row that each GPU backend antialiases differently.
const STICKY_CONTENT_CANVAS: egui::Vec2 = egui::vec2(600.0, 500.0);

/// The position every fix built here sits at.
const FIXTURE_LAT: f64 = 51.5;
const FIXTURE_LON: f64 = -0.1;

// With a 12 px icon, the fade band spans local spacings of 2.4 px
// (LO, 0.2 icon sizes - arrows share almost all pixels) to 6 px
// (HI, 0.5 icon sizes - arrows overlap but stay readable).
const TEST_ICON_PX: f32 = 12.0;

// At low zoom icons shrink to 3 px and the proportional band would be
// 0.6-1.5 px. The absolute floors widen it to 2-5 px so dot-sized
// arrows stacked a couple of pixels apart fade into the quality line.
const SMALL_ICON_PX: f32 = 3.0;

/// Same value as `MercTransform::pixels_per_meter`'s internal constant.
/// With `for_test(EARTH_CIRCUMFERENCE_M)` the map scale is 1 px/m at the
/// equator, so test geometry can be written directly in metres.
const EARTH_CIRCUMFERENCE_M: f64 = 40_030_173.0;

#[rstest]
#[case::summary_fix(
    Some(SolutionState::Fix),
    Some(8),
    None,
    MapFixKind::Recorded,
    FIX_BLUE
)]
#[case::fix_with_zero(
    Some(SolutionState::Fix),
    Some(0),
    None,
    MapFixKind::Recorded,
    FIX_BLUE
)]
#[case::fix_without_count(Some(SolutionState::Fix), None, None, MapFixKind::Recorded, FIX_BLUE)]
#[case::unknown_with_zero(
    Some(SolutionState::Unknown),
    Some(0),
    None,
    MapFixKind::Recorded,
    FIX_UNKNOWN_GRAY
)]
#[case::no_state(None, None, None, MapFixKind::Recorded, FIX_UNKNOWN_GRAY)]
#[case::loss_with_positive_count(
    Some(SolutionState::NoFix),
    Some(12),
    None,
    MapFixKind::NavigationLoss,
    FIX_LOST_RED
)]
#[case::estimated(
    Some(SolutionState::EstimatedOnly),
    Some(12),
    None,
    MapFixKind::Estimated,
    FIX_ESTIMATED_PURPLE
)]
#[case::synthesized_fix(
    Some(SolutionState::Fix),
    Some(12),
    Some(PositionProvenance::SdkSynthesized),
    MapFixKind::Synthesized,
    FIX_BLUE
)]
#[case::synthesized_unknown(
    None,
    None,
    Some(PositionProvenance::SdkSynthesized),
    MapFixKind::Synthesized,
    FIX_UNKNOWN_GRAY
)]
fn navigation_presentation_depends_on_state_and_provenance(
    #[case] state: Option<SolutionState>,
    #[case] used_count: Option<u32>,
    #[case] position_provenance: Option<PositionProvenance>,
    #[case] expected_kind: MapFixKind,
    #[case] expected_color: Color32,
) {
    for mut point in [point_at(0, None), a_fix_without_a_heading()] {
        point.navigation = NavigationMetadata {
            state,
            used_count,
            position_provenance,
            ..NavigationMetadata::default()
        };
        assert_eq!(point.map_fix_kind(), expected_kind);
        assert_eq!(tpv_point_color(&point), expected_color);
        let chevron = ChevronFix::for_fix(&point);
        assert_eq!(chevron.is_some(), expected_kind != MapFixKind::Recorded);
        if let Some(chevron) = chevron {
            assert_eq!(chevron.tint(false), expected_color);
            assert_eq!(chevron.tint(true), expected_color);
        }
        assert_eq!(point.navigation.state, state);
        assert_eq!(point.navigation.used_count, used_count);
    }
}

#[rstest]
#[case::summary(
    Some(SolutionState::Fix),
    Some(8),
    "Fix",
    "Reported state",
    "Satellites used / seen 8 / —"
)]
#[case::contradiction(
    Some(SolutionState::NoFix),
    Some(12),
    "No fix",
    "Reported state",
    "Satellites used / seen 12 / —"
)]
#[case::fix_zero(
    Some(SolutionState::Fix),
    Some(0),
    "Fix",
    "Reported state",
    "Satellites used / seen 0 / —"
)]
#[case::explicit_unknown(
    Some(SolutionState::Unknown),
    Some(0),
    "Unknown",
    "Reported state",
    "Satellites used / seen 0 / —"
)]
#[case::legacy(None, None, "Unknown", "Unreported", "Satellites used / seen — / —")]
#[case::count_fallback(
    None,
    Some(8),
    "Fix",
    "Navigation used count",
    "Satellites used / seen 8 / —"
)]
#[case::estimate(
    Some(SolutionState::EstimatedOnly),
    None,
    "Estimated only",
    "Reported state",
    "Satellites used / seen — / —"
)]
fn point_details_preserve_state_and_count_independently(
    #[case] state: Option<SolutionState>,
    #[case] used_count: Option<u32>,
    #[case] state_label: &str,
    #[case] evidence_label: &str,
    #[case] count_label: &str,
) {
    let mut point = point_at(0, None);
    point.navigation.state = state;
    point.navigation.used_count = used_count;
    let mut folds = PointWindowFolds::default();
    let placement = placement_of(&point);
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(380.0, 500.0))
        .ui(move |ui| {
            let _opened = show_sticky_tpv_content(
                ui,
                &point,
                &SkySection::TrackWithoutReports,
                &mut folds,
                None,
                placement,
            );
        });
    harness.run();
    assert!(harness.inner.query_by_label("State evidence").is_none());
    assert!(harness.inner.query_by_label("Position source").is_none());
    harness.inner.get_by_label("Details").click();
    harness.inner.run_steps(3);
    for label in [state_label, evidence_label, count_label] {
        assert!(
            harness.inner.query_all_by_label(label).next().is_some(),
            "missing {label}"
        );
    }
    let row_y = harness
        .inner
        .query_all_by_label("Satellites")
        .next()
        .expect("summary label")
        .rect()
        .center()
        .y;
    assert!(
        harness
            .inner
            .query_all_by_label(count_label)
            .any(|node| node.rect().y_range().contains(row_y))
    );
    assert!(harness.inner.query_by_label("Position source").is_some());
}

#[rstest]
#[case::source(
    "navigation_details_source_dark",
    true,
    PositionProvenance::SourceProvided
)]
#[case::synthesized(
    "navigation_details_synthesized_light",
    false,
    PositionProvenance::SdkSynthesized
)]
fn navigation_details_show_recorded_metadata(
    #[case] name: &str,
    #[case] dark_mode: bool,
    #[case] provenance: PositionProvenance,
) {
    let mut point = point_at(0, None);
    point.tpv = TimePositionVelocity::builder()
        .time(point.tpv.time())
        .lat(point.tpv.lat())
        .lon(point.tpv.lon())
        .maybe_heading(
            (provenance == PositionProvenance::SdkSynthesized)
                .then_some(Angle::new::<degree>(90.0)),
        )
        .build();
    point.navigation = NavigationMetadata {
        state: Some(SolutionState::Fix),
        method: Some(SolutionMethod::Other),
        dimension: Some(SolutionDimension::ThreeD),
        used_count: Some(8),
        hdop: Some(0.9),
        position_provenance: Some(provenance),
        ..NavigationMetadata::default()
    };
    let placement = placement_of(&point);
    let mut folds = PointWindowFolds::default();
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(380.0, 360.0))
        .theme(dark_mode)
        .ui(move |ui| {
            let _opened = show_sticky_tpv_content(
                ui,
                &point,
                &SkySection::TrackWithoutReports,
                &mut folds,
                None,
                placement,
            );
        });
    harness.run();
    for label in [
        "Fix",
        "Satellites used / seen 8 / —",
        "Other",
        "3D",
        "DOP",
        "H 0.90",
    ] {
        assert!(
            harness.inner.query_all_by_label(label).next().is_some(),
            "missing {label}"
        );
    }
    assert!(harness.inner.query_by_label("Accuracy").is_none());
    assert!(harness.inner.query_by_label("Position source").is_none());
    harness.snapshot(name);
    harness.inner.get_by_label("Details").click();
    harness.inner.run_steps(3);
    assert!(
        harness
            .inner
            .query_by_label(&provenance.to_string())
            .is_some()
    );
    harness.snapshot(&format!("{name}_expanded"));
}

#[test]
fn recorded_reports_show_conflicting_aggregates_scope_and_original_times() {
    let mut point = point_at(0, None);
    point.navigation.used_count = Some(8);
    let track_time =
        GpsTime::from_utc(point.tpv.time().utc() + chrono::Duration::milliseconds(400));
    let metadata = SatelliteReportMetadata {
        gps_time: Some(track_time),
        sys_time: Some(SysTime::from_utc(
            fixture_epoch() + chrono::Duration::milliseconds(1400),
        )),
        normalized_time: track_time,
        input_order: 1,
        scope: SatelliteScope::Constellations(ConstellationSet::single(Constellation::Gps)),
        coverage: SatelliteCoverage::Partial,
        reported_count: Some(9),
    };
    point.satellite_evidence.usage.push(SatelliteUsageReport {
        metadata,
        satellites: [
            (1, SatelliteMembership::Unknown),
            (2, SatelliteMembership::Unused),
            (3, SatelliteMembership::Used),
        ]
        .into_iter()
        .map(|(prn, membership)| SatelliteUsage {
            identity: SatelliteIdentity {
                constellation: Constellation::Gps,
                prn: Prn::new(prn),
            },
            membership,
        })
        .collect(),
    });
    point.satellite_evidence.usage.push(SatelliteUsageReport {
        metadata: SatelliteReportMetadata {
            scope: SatelliteScope::Global,
            coverage: SatelliteCoverage::Complete,
            reported_count: Some(0),
            ..metadata
        },
        satellites: Vec::new(),
    });
    let placement = placement_of(&point);
    let mut folds = PointWindowFolds::default();
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(430.0, 550.0))
        .ui(move |ui| {
            let _opened = show_sticky_tpv_content(
                ui,
                &point,
                &SkySection::TrackWithoutReports,
                &mut folds,
                None,
                placement,
            );
        });
    harness.run();
    assert!(
        harness
            .inner
            .query_by_label("Satellites used / seen 8 / —")
            .is_some()
    );
    harness.inner.get_by_label("Details").click();
    harness.inner.run_steps(3);
    for label in [
        "Usage 1 · 11:33:20.400 · GPS",
        "Usage 2 · 11:33:20.400 · Global",
        "Partial",
        "Complete",
        "Unknown",
        "Unused",
        "Used",
        "9",
        "0",
        "2025-05-23 11:33:20.400 UTC",
        "2025-05-23 11:33:21.400 UTC",
    ] {
        assert!(
            harness.inner.query_all_by_label(label).next().is_some(),
            "missing {label}"
        );
    }
}

#[rstest]
#[case::dark(true)]
#[case::light(false)]
fn many_identities_have_a_bounded_hover_preview_and_a_complete_pinned_table(
    #[case] dark_mode: bool,
) {
    let mut point = point_at(
        0,
        Some(gt_test_utils::fixtures::satellite_report(
            None,
            gt_types::fixtures::SatelliteCounts {
                in_fix: 8,
                in_view_only: 40,
            },
        )),
    );
    for satellite in &mut point.satellite_evidence.observations[0].satellites {
        satellite.measurements.azimuth = None;
        satellite.measurements.elevation = None;
    }
    let track = test_util::track_built_from(vec![point]);
    let file = gt_test_utils::loaded_file_with_tracks(vec![track]);
    let size = egui::vec2(600.0, 500.0);
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, size);
    let mut map = MapScene::of(vec![file])
        .theme(dark_mode)
        .size(size)
        .centred_on_fix(0)
        .zoomed_to(15.0)
        .draw_state(|state| state.highlight.fading_enabled = false)
        .render();
    let center = viewport.center();
    map.hover_at_and_settle(center);
    let first = map.harness.inner.get_by_label("G01 GPS").rect();
    let last = map.harness.inner.get_by_label("G03 GPS").rect();
    let remaining = map
        .harness
        .inner
        .get_by_label("45 more · Pin to inspect")
        .rect();
    let preview = first.union(last).union(remaining);
    assert!(viewport.contains_rect(preview));
    assert!(preview.height() < 100.0);
    assert!(map.harness.inner.query_by_label("G04 GPS").is_none());
    assert!(map.harness.inner.query_by_label("G48 GPS").is_none());
    map.snapshot(if dark_mode {
        "many_identities_hover_dark"
    } else {
        "many_identities_hover_light"
    });
    assert_eq!(
        map.point_pinned_by_a_click_at(center),
        Some(test_util::point_ref(DataCategory::Tpv, 0))
    );
    map.harness.run();
    let tables = StickyRegion::Tables.rect(&map.harness.inner.ctx);
    let scroll_position = egui::pos2(
        map.harness.inner.get_by_label("G01").rect().center().x,
        tables.center().y,
    );
    map.harness
        .inner
        .scroll_wheel_at(scroll_position, -REPORT_SCROLL_TO_END_POINTS, 8);
    let final_identity = map.harness.inner.get_by_label("G48").rect();
    assert!(
        tables.contains_rect(final_identity),
        "tables {tables:?}, final identity {final_identity:?}"
    );
    assert!(viewport.contains_rect(map.harness.inner.get_by_label("Details").rect()));
    map.snapshot(if dark_mode {
        "many_identities_pinned_dark"
    } else {
        "many_identities_pinned_light"
    });
}

#[test]
fn pinned_identity_only_evidence_reuses_the_constellation_table() {
    let mut point = point_at(0, Some(twelve_satellites(8)));
    for satellite in &mut point.satellite_evidence.observations[0].satellites {
        satellite.measurements.azimuth = None;
        satellite.measurements.elevation = None;
    }
    let placement = placement_of(&point);
    let mut folds = PointWindowFolds::default();
    let mut harness = test_util::harness_builder()
        .size(STICKY_CONTENT_CANVAS)
        .ui(move |ui| {
            let _opened =
                show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None, placement);
        });
    harness.run();
    assert!(harness.inner.query_by_label("No sky positions").is_some());
    assert!(harness.inner.query_by_label("G01 GPS").is_none());
    assert_eq!(harness.inner.query_all_by_label("G01").count(), 1);
    assert_eq!(
        harness
            .inner
            .query_all_by_label("Satellites used / seen 8 / 12")
            .count(),
        2
    );
    assert!(StickyRegion::Sky.rect(&harness.inner.ctx).height() < SkyPlotSize::Full.diameter());
    harness.snapshot("pinned_identities_without_chart");
}

#[test]
fn borrowed_sky_observations_preserve_point_count_absence() {
    let mut point = point_at(0, None);
    point.navigation.state = Some(SolutionState::Fix);
    point.navigation.used_count = Some(8);
    let borrowed = point_at(2, Some(twelve_satellites(10)));
    let track = gt_test_utils::loaded_track_with_points(vec![point, borrowed]);
    let mut folds = PointWindowFolds::default();
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(380.0, 850.0))
        .ui(move |ui| {
            let point_index = PointIdx::new(0);
            let point = point_index.get(&track.points).expect("source point");
            let _opened = show_sticky_tpv_content(
                ui,
                point,
                &SkySection::resolve(&track, point_index),
                &mut folds,
                None,
                FixPlacement::resolve(&track, point_index),
            );
        });
    harness.run();
    assert!(
        harness
            .inner
            .query_by_label("Satellites used / seen 8 / —")
            .is_some()
    );
    assert!(
        harness
            .inner
            .query_by_label("Satellites used / seen 8 / 12")
            .is_none()
    );
    assert!(
        harness
            .inner
            .query_by_label("Satellites used / seen 10 / 12")
            .is_some()
    );
    assert!(harness.inner.query_by_label("Sky +2.0s").is_some());
    harness.inner.get_by_label("Nearby sky details").click();
    harness.inner.run_steps(3);
    assert!(harness.inner.query_by_label("Report 2.0s later").is_some());
    assert!(
        harness
            .inner
            .query_by_label("Sky observation 1 · 2025-05-23 11:33:22 UTC")
            .is_some()
    );
}

#[rstest]
#[case::observations_with_summary(
    SatelliteCoverage::Complete,
    SatelliteScope::Global,
    true,
    Some(8),
    "Satellites used / seen 8 / 12"
)]
#[case::observations_only(
    SatelliteCoverage::Complete,
    SatelliteScope::Global,
    true,
    None,
    "Satellites used / seen — / 12"
)]
#[case::complete_empty(
    SatelliteCoverage::Complete,
    SatelliteScope::Global,
    false,
    None,
    "Satellites used / seen — / 0"
)]
#[case::partial_empty(
    SatelliteCoverage::Partial,
    SatelliteScope::Global,
    false,
    None,
    "Satellites used / seen — / —"
)]
#[case::scoped(
    SatelliteCoverage::Complete,
    SatelliteScope::Constellations(ConstellationSet::single(Constellation::Gps)),
    true,
    None,
    "Satellites used / seen — / —"
)]
fn observation_details_preserve_global_count_availability(
    #[case] coverage: SatelliteCoverage,
    #[case] scope: SatelliteScope,
    #[case] identity_rows: bool,
    #[case] used_count: Option<u32>,
    #[case] expected: &str,
) {
    let mut point = point_at(0, Some(twelve_satellites(8)));
    point.navigation.used_count = used_count;
    point.satellite_evidence.usage.clear();
    point.satellites = None;
    let report = point
        .satellite_evidence
        .observations
        .first_mut()
        .expect("observation");
    report.metadata.coverage = coverage;
    report.metadata.scope = scope;
    if !identity_rows {
        report.satellites.clear();
    }
    let placement = placement_of(&point);
    let mut folds = PointWindowFolds::default();
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(420.0, 800.0))
        .ui(move |ui| {
            let _opened =
                show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None, placement);
        });
    harness.run();
    assert!(
        harness.inner.query_all_by_label(expected).any(|node| node
            .rect()
            .y_range()
            .contains(harness.inner.get_by_label("Satellites").rect().center().y)),
        "missing {expected}"
    );
    assert!(harness.inner.query_by_label("State evidence").is_none());
}

#[test]
fn legacy_empty_report_details_show_known_zero_usage_and_observations() {
    let point = point_at(0, Some(Satellites::new(None, None, Vec::new())));
    let placement = placement_of(&point);
    let mut folds = PointWindowFolds::default();
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(380.0, 500.0))
        .ui(move |ui| {
            let _opened =
                show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None, placement);
        });
    harness.run();
    assert!(
        harness
            .inner
            .query_by_label("Satellites used / seen 0 / 0")
            .is_some()
    );
    assert!(harness.inner.query_by_label("No fix").is_some());
    harness.inner.get_by_label("Details").click();
    harness.inner.run_steps(3);
    assert!(
        harness
            .inner
            .query_all_by_label("Usage 1 membership")
            .next()
            .is_some()
    );
}

#[rstest]
#[case::used("Satellites", "8", Some(SkyHighlight::in_fix()))]
#[case::seen("Satellites", "12", None)]
#[case::constellation_used(
    "GPS",
    "8",
    Some(SkyHighlight::constellation_in_fix(Constellation::Gps))
)]
#[case::constellation_seen("GPS", "12", Some(SkyHighlight::constellation(Constellation::Gps)))]
fn satellite_summary_numbers_have_independent_highlight_regions(
    #[case] caption: &str,
    #[case] number: &str,
    #[case] expected: Option<SkyHighlight>,
) {
    let point = point_at(0, Some(twelve_satellites(8)));
    let id_cell = std::rc::Rc::new(std::cell::Cell::new(None));
    let cell = id_cell.clone();
    let mut folds = PointWindowFolds {
        plot_folded: true,
        ..PointWindowFolds::default()
    };
    let placement = placement_of(&point);
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(380.0, 500.0))
        .ui(move |ui| {
            cell.set(Some(pinned_point::sky_table_highlight_id(ui)));
            let _opened =
                show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None, placement);
        });
    harness.run();
    let summary_y = harness.inner.get_by_label(caption).rect().center().y;
    let number_rect = harness
        .inner
        .query_all_by_label(number)
        .map(|node| node.rect())
        .find(|rect| rect.y_range().contains(summary_y))
        .expect("summary number");
    assert!(number_rect.bottom() < 500.0);
    harness.inner.hover_at(number_rect.center());
    harness.inner.run_steps(2);
    let highlight = harness
        .inner
        .ctx
        .data(|data| data.get_temp::<Option<SkyHighlight>>(id_cell.get().expect("highlight id")))
        .flatten();
    assert_eq!(highlight, expected);
}

#[rstest]
#[case::dark(true)]
#[case::light(false)]
fn narrow_pinned_summary_precedes_sky_and_keeps_details_visible(#[case] dark_mode: bool) {
    let point = point_at(0, Some(sats_dense_multi_constellation()));
    let mut folds = PointWindowFolds::default();
    let placement = placement_of(&point);
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(300.0, 320.0))
        .theme(dark_mode)
        .ui(move |ui| {
            let _opened =
                show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None, placement);
        });
    harness.run();
    let details = harness.inner.get_by_label("Details").rect();
    let counts = harness
        .inner
        .get_by_label("Satellites used / seen 29 / 40")
        .rect();
    let sky = harness.inner.get_by_label("Sky").rect();
    assert!(counts.bottom() < details.top());
    assert!(details.bottom() < sky.top());
    assert!(details.right() < 300.0 && details.bottom() < 320.0);
    assert!(harness.inner.query_by_label("Position source").is_none());
    harness.snapshot(if dark_mode {
        "narrow_pinned_summary_dark"
    } else {
        "narrow_pinned_summary_light"
    });
}

#[rstest]
#[case::summary("sparse_summary_hover_dark", true, Some(8))]
#[case::legacy("sparse_legacy_hover_light", false, None)]
fn sparse_hover_uses_compact_summary(
    #[case] name: &str,
    #[case] dark_mode: bool,
    #[case] used_count: Option<u32>,
) {
    let mut point = point_at(0, None);
    point.tpv = TimePositionVelocity::builder()
        .time(point.tpv.time())
        .lat(point.tpv.lat())
        .lon(point.tpv.lon())
        .build();
    point.navigation.used_count = used_count;
    point.navigation.state = used_count.map(|_| SolutionState::Fix);
    point.navigation.hdop = used_count.map(|_| 0.9);
    let track = gt_test_utils::loaded_track_with_points(vec![point]);
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(320.0, 260.0))
        .theme(dark_mode)
        .ui(move |ui| {
            let placed = track
                .placed_points()
                .expect("geometry")
                .get(0)
                .expect("point");
            show_hover_table(ui, placed, &SkySection::TrackWithoutReports, None);
        });
    harness.run();
    assert!(harness.inner.query_by_label("State evidence").is_none());
    assert!(harness.inner.query_by_label("Position source").is_none());
    assert!(harness.inner.query_by_label("Details").is_none());
    harness.snapshot(name);
}

#[rstest]
#[case::narrow(400.0)]
#[case::wide(600.0)]
fn a_report_from_an_equal_time_point_remains_inspectable(#[case] width: f32) {
    let mut source = point_at(0, Some(twelve_satellites(10)));
    let observation = source
        .satellite_evidence
        .observations
        .first_mut()
        .expect("observation");
    observation.metadata.scope =
        SatelliteScope::Constellations(ConstellationSet::single(Constellation::Gps));
    observation.metadata.reported_count = Some(12);
    observation.metadata.gps_time = Some(GpsTime::from_utc(fixture_epoch()));
    observation.metadata.sys_time = Some(SysTime::from_utc(
        fixture_epoch() + chrono::Duration::seconds(1),
    ));
    let mut point = point_at(0, None);
    point.navigation.state = Some(SolutionState::Fix);
    point.navigation.used_count = Some(8);
    let track = gt_test_utils::loaded_track_with_points(vec![source, point]);
    let mut folds = PointWindowFolds::default();
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(width, 650.0))
        .ui(move |ui| {
            let index = PointIdx::new(1);
            let point = index.get(&track.points).expect("selected point");
            let sky = SkySection::resolve(&track, index);
            assert!(matches!(sky, SkySection::Report(report) if report.age.is_zero() && report.point_index == PointIdx::new(0)));
            let _opened = show_sticky_tpv_content(ui, point, &sky, &mut folds, None, FixPlacement::resolve(&track, index));
        });
    harness.run();
    assert!(
        harness
            .inner
            .query_by_label("Satellites used / seen 8 / —")
            .is_some()
    );
    let sky = StickyRegion::Sky.rect(&harness.inner.ctx);
    harness.inner.get_by_label("Nearby sky details").click();
    harness.inner.run_steps(3);
    if width >= MIN_SIDE_BY_SIDE_WIDTH_PX {
        assert_eq!(StickyRegion::Sky.rect(&harness.inner.ctx), sky);
        assert!(StickyRegion::Summary.rect(&harness.inner.ctx).left() > sky.right());
    }
    for label in [
        "Report at the same time",
        "Observation 1 · 11:33:20.000 · GPS",
        "2025-05-23 11:33:20 UTC",
        "2025-05-23 11:33:21 UTC",
        "Reported count",
        "12",
    ] {
        assert!(
            harness.inner.query_all_by_label(label).next().is_some(),
            "missing {label}"
        );
    }
    assert!(
        harness
            .inner
            .query_by_label("Satellites used / seen 8 / —")
            .is_some()
    );
    assert!(
        harness
            .inner
            .query_by_label("Satellites used / seen 8 / 12")
            .is_none()
    );
}

#[rstest]
#[case::dark(true)]
#[case::light(false)]
fn the_minimum_point_window_keeps_details_reachable_with_all_readouts(#[case] dark_mode: bool) {
    let mut point = point_at(10, Some(twelve_satellites(10)));
    point.tpv = TimePositionVelocity::builder()
        .time(point.tpv.time())
        .lat(Latitude::new(51.6))
        .lon(Longitude::new(0.2))
        .heading(Angle::new::<degree>(90.0))
        .eph_m(5.0)
        .build();
    point.navigation = NavigationMetadata {
        state: Some(SolutionState::Fix),
        used_count: Some(8),
        hdop: Some(0.9),
        vdop: Some(1.2),
        pdop: Some(1.5),
        position_provenance: Some(PositionProvenance::SdkSynthesized),
        ..NavigationMetadata::default()
    };
    let mut last = point
        .satellite_evidence
        .observations
        .first()
        .expect("observation")
        .clone();
    last.metadata.input_order = 9999;
    last.metadata.scope =
        SatelliteScope::Constellations(ConstellationSet::single(Constellation::Gps));
    last.metadata.coverage = SatelliteCoverage::Partial;
    last.metadata.reported_count = Some(123);
    last.satellites.clear();
    point.satellite_evidence.observations.push(last);
    let track =
        gt_test_utils::loaded_track_with_points(vec![point_at(0, None), point, point_at(20, None)]);
    assert!(matches!(
        FixPlacement::resolve(&track, PointIdx::new(1)),
        FixPlacement::Placed(ResolvedPosition::Interpolated(_))
    ));
    let mut file = gt_test_utils::loaded_file_with_tracks(vec![track]);
    file.metadata.filename = "morning.gtd".to_owned();
    let mut second = gt_test_utils::loaded_file_with_tracks(Vec::new());
    second.metadata.filename = "evening.gtd".to_owned();
    let mut files = LoadedFiles::new();
    files.push(file, FileHistory::None);
    files.push(second, FileHistory::None);
    let names = RecordingNames::resolve(files.view(), "{filename}");
    let clicked = test_util::point_ref(DataCategory::Tpv, 1);
    let window_id = egui::Id::new(("sticky_popup", clicked));
    let mut folds = PointWindowFolds::default();
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(
            crate::POINT_WINDOW_MIN_WIDTH_PX,
            crate::POINT_WINDOW_MIN_HEIGHT_PX,
        ))
        .theme(dark_mode)
        .ui(move |ui| {
            let _request = crate::show_sticky_popup(
                ui.ctx(),
                files.files(),
                RecordingLabels::new(files.files(), &names),
                clicked,
                egui::Pos2::ZERO,
                &mut folds,
            );
        });
    harness.run();
    let window = harness
        .inner
        .ctx
        .memory(|memory| memory.area_rect(window_id))
        .expect("window");
    assert!(
        window.height() <= crate::POINT_WINDOW_MIN_HEIGHT_PX + 1.0,
        "window {window:?}"
    );
    let footer = harness.inner.get_by_label("Click to deselect").rect();
    let visible_body =
        egui::Rect::from_min_max(window.min, egui::pos2(window.right(), footer.top())).intersect(
            egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(
                    crate::POINT_WINDOW_MIN_WIDTH_PX,
                    crate::POINT_WINDOW_MIN_HEIGHT_PX,
                ),
            ),
        );
    let details = harness.inner.get_by_label("Details").rect();
    assert!(
        visible_body.contains_rect(details),
        "Details {details:?}, body {visible_body:?}"
    );
    assert!(
        visible_body.contains_rect(
            harness
                .inner
                .get_by_label("Satellites used / seen 8 / 12")
                .rect()
        )
    );
    assert!(harness.inner.query_by_label("Recording").is_some());
    assert!(harness.inner.query_by_label(DRAWN_AT_CAPTION).is_some());
    assert!(harness.inner.query_by_label("Accuracy").is_some());
    assert!(harness.inner.query_by_label("DOP").is_some());
    harness.inner.get_by_label("Details").click();
    harness.inner.run_steps(3);
    assert!(harness.inner.query_by_label("State evidence").is_some());
    let details = harness.inner.get_by_label("Details").rect();
    harness.inner.scroll_wheel_at(
        egui::pos2(
            details.center().x,
            details.bottom() + REPORT_SCROLL_TARGET_OFFSET_PX,
        ),
        -REPORT_SCROLL_TO_END_POINTS,
        8,
    );
    let details_after_scroll = harness.inner.get_by_label("Details").rect();
    assert_eq!(details_after_scroll, details);
    assert!(visible_body.contains_rect(details_after_scroll));
    let last = harness.inner.get_by_label("9999").rect();
    assert!(
        visible_body.contains_rect(last),
        "final report {last:?}, body {visible_body:?}"
    );
    harness.snapshot_with_color_tolerance(if dark_mode {
        "minimum_pinned_report_inspection_dark"
    } else {
        "minimum_pinned_report_inspection_light"
    });
    harness.inner.get_by_label("Details").click();
    harness.run();
    assert!(harness.inner.query_by_label("State evidence").is_none());
    assert!(visible_body.contains_rect(harness.inner.get_by_label("Details").rect()));
}

const REPORT_SCROLL_TARGET_OFFSET_PX: f32 = 25.0;
const REPORT_SCROLL_TO_END_POINTS: f32 = 10_000.0;

impl StickyRegion {
    fn rect(self, ctx: &egui::Context) -> egui::Rect {
        ctx.data(|data| data.get_temp(egui::Id::new(("sticky_region", self))))
            .expect("rendered region")
    }
}

#[rstest]
#[case::wide_dark(egui::vec2(600.0, 500.0), true, true)]
#[case::wide_light(egui::vec2(600.0, 500.0), false, true)]
#[case::dense(egui::vec2(620.0, 560.0), true, true)]
#[case::narrow(egui::vec2(330.0, 560.0), true, false)]
#[case::short_narrow(egui::vec2(300.0, 320.0), false, false)]
#[case::below_breakpoint(egui::vec2(MIN_SIDE_BY_SIDE_WIDTH_PX + 15.0, 560.0), true, false)]
#[case::above_breakpoint(egui::vec2(MIN_SIDE_BY_SIDE_WIDTH_PX + 17.0, 560.0), true, true)]
fn pinned_regions_follow_the_available_width(
    #[case] size: egui::Vec2,
    #[case] dark_mode: bool,
    #[case] wide: bool,
) {
    let mut point = point_at(0, Some(sats_dense_multi_constellation()));
    point.navigation.hdop = Some(0.9);
    point.navigation.vdop = Some(1.2);
    point.navigation.pdop = Some(1.5);
    point.navigation.method = Some(SolutionMethod::DeadReckoning);
    point.navigation.dimension = Some(SolutionDimension::ThreeD);
    let placement = placement_of(&point);
    let mut folds = PointWindowFolds::default();
    let mut harness = test_util::harness_builder()
        .size(size)
        .theme(dark_mode)
        .ui(move |ui| {
            let _opened = show_sticky_tpv_content(
                ui,
                &point,
                &sky_for(&point),
                &mut folds,
                Some("A recording with a long descriptive filename and repeated coordinates.gtd"),
                placement,
            );
        });
    harness.run();
    let ctx = &harness.inner.ctx;
    let body = StickyRegion::Body.rect(ctx);
    let summary = StickyRegion::Summary.rect(ctx);
    let sky_header = StickyRegion::SkyHeader.rect(ctx);
    let sky = StickyRegion::Sky.rect(ctx);
    let metrics = StickyRegion::Metrics.rect(ctx);
    assert!(
        body.contains_rect(summary),
        "summary {summary:?}, body {body:?}"
    );
    assert!(body.x_range().contains(sky.left()));
    assert!(body.x_range().contains(sky.right()));
    assert!(
        metrics.top() >= sky.bottom(),
        "metrics {metrics:?}, sky {sky:?}"
    );
    assert!(
        metrics.right()
            <= if wide {
                sky.right() + 1.0
            } else {
                body.right() + 1.0
            },
        "metrics {metrics:?}, sky {sky:?}"
    );
    if wide {
        assert!((sky_header.top() - body.top()).abs() < f32::EPSILON);
        assert!((summary.top() - body.top()).abs() < f32::EPSILON);
        assert!(summary.left() >= sky_header.right() + STICKY_COLUMN_GAP_PX - 1.0);
        assert!(StickyRegion::Tables.rect(ctx).width() >= MIN_SATELLITE_COLUMN_WIDTH_PX - 12.0);
        let gps = harness.inner.get_by_label("GPS").rect();
        let glonass = harness.inner.get_by_label("GLONASS").rect();
        if size.x >= 600.0 {
            let beidou = harness.inner.get_by_label("BeiDou").rect();
            assert!(
                beidou.left() > gps.right(),
                "two columns: {gps:?}, {beidou:?}"
            );
        } else {
            assert!((gps.left() - glonass.left()).abs() < f32::EPSILON);
        }
    } else {
        assert!((summary.top() - body.top()).abs() < f32::EPSILON);
        assert!(sky_header.top() >= summary.bottom());
        assert!((summary.left() - sky_header.left()).abs() < f32::EPSILON);
    }
}

#[test]
fn opening_right_inspection_preserves_the_sky_and_scrolls_reports_and_tables() {
    let mut point = point_at(0, Some(sats_dense_multi_constellation()));
    let mut final_report = point
        .satellite_evidence
        .observations
        .first()
        .expect("report")
        .clone();
    final_report.metadata.input_order = 9999;
    final_report.satellites.clear();
    point.satellite_evidence.observations.push(final_report);
    let placement = placement_of(&point);
    let mut folds = PointWindowFolds::default();
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(620.0, 560.0))
        .ui(move |ui| {
            let _opened =
                show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None, placement);
        });
    harness.run();
    let sky = StickyRegion::Sky.rect(&harness.inner.ctx);
    let details = harness.inner.get_by_label("Details").rect();
    harness.inner.get_by_label("Details").click();
    harness.run();
    assert_eq!(StickyRegion::Sky.rect(&harness.inner.ctx), sky);
    assert_eq!(harness.inner.get_by_label("Details").rect(), details);
    let inspection = StickyRegion::Inspection.rect(&harness.inner.ctx);
    let tables = StickyRegion::Tables.rect(&harness.inner.ctx);
    assert!(inspection.left() > sky.right());
    assert!(tables.top() > inspection.bottom());
    harness
        .inner
        .scroll_wheel_at(inspection.center(), -REPORT_SCROLL_TO_END_POINTS, 8);
    assert!(inspection.contains_rect(harness.inner.get_by_label("9999").rect()));
    harness
        .inner
        .scroll_wheel_at(tables.center(), -REPORT_SCROLL_TO_END_POINTS, 8);
    assert!(
        harness
            .inner
            .query_all_by_label("C15")
            .any(|node| tables.contains_rect(node.rect()))
    );
    harness.snapshot("sticky_wide_evidence_scrolled");
    assert_eq!(StickyRegion::Sky.rect(&harness.inner.ctx), sky);
    harness.inner.get_by_label("Details").click();
    harness.run();
    assert!(harness.inner.query_by_label("State evidence").is_none());
}

#[test]
fn resizing_preserves_inspection_and_constellation_folds() {
    let point = point_at(0, Some(sats_multi_constellation()));
    let placement = placement_of(&point);
    let mut folds = PointWindowFolds::default();
    folds.toggle(Constellation::Gps);
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(600.0, 500.0))
        .ui(move |ui| {
            let _opened =
                show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None, placement);
        });
    harness.run();
    harness.inner.get_by_label("Details").click();
    harness.run();
    for size in [egui::vec2(300.0, 320.0), egui::vec2(620.0, 560.0)] {
        harness.inner.set_size(size);
        harness.run();
        assert!(harness.inner.query_by_label("State evidence").is_some());
        let summary = StickyRegion::Summary.rect(&harness.inner.ctx);
        assert!(
            StickyRegion::Body
                .rect(&harness.inner.ctx)
                .contains_rect(summary)
        );
        harness.inner.get_by_label("Details").click();
        harness.run();
        assert!(harness.inner.query_by_label("G01").is_none());
        harness.inner.get_by_label("Details").click();
        harness.run();
    }
}

#[rstest]
#[case::track_without_reports(SkySection::TrackWithoutReports)]
#[case::no_report_nearby(SkySection::NoReportNearby)]
fn summary_only_inspection_uses_the_full_content_width(#[case] sky: SkySection<'static>) {
    let mut point = point_at(0, None);
    point.navigation.state = Some(SolutionState::Fix);
    point.navigation.used_count = Some(8);
    let placement = placement_of(&point);
    let mut folds = PointWindowFolds::default();
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(600.0, 500.0))
        .ui(move |ui| {
            let _opened = show_sticky_tpv_content(ui, &point, &sky, &mut folds, None, placement);
        });
    harness.run();
    harness.inner.get_by_label("Details").click();
    harness.run();
    let body = StickyRegion::Body.rect(&harness.inner.ctx);
    let inspection = StickyRegion::Inspection.rect(&harness.inner.ctx);
    assert!((inspection.left() - body.left()).abs() < f32::EPSILON);
    assert!(inspection.width() > SkyPlotSize::Full.diameter());
    assert!(harness.inner.query_by_label("Lat").is_some());
    assert!(
        (StickyRegion::Summary.rect(&harness.inner.ctx).left() - body.left()).abs() < f32::EPSILON
    );
}

#[rstest]
#[case::wide_to_narrow(600.0, 330.0)]
#[case::narrow_to_wide(330.0, 600.0)]
fn point_and_nearby_reports_have_independent_controls_after_resizing(
    #[case] initial_width: f32,
    #[case] resized_width: f32,
) {
    let mut point = point_at(0, Some(sats_with_sky()));
    point.satellite_evidence.usage.clear();
    point.navigation.state = Some(SolutionState::Fix);
    point.navigation.used_count = Some(8);
    let mut nearby = point.clone();
    for (report_point, host_second, snr) in [(&mut point, 1, 17.25), (&mut nearby, 2, 47.75)] {
        let report = report_point
            .satellite_evidence
            .observations
            .first_mut()
            .expect("observation");
        report.metadata.sys_time = Some(SysTime::from_utc(
            fixture_epoch() + chrono::Duration::seconds(host_second),
        ));
        report.satellites.truncate(1);
        report
            .satellites
            .first_mut()
            .expect("satellite")
            .measurements
            .snr = Some(Snr::new(snr));
    }
    let report = point
        .satellite_evidence
        .observations
        .first_mut()
        .expect("observation");
    let measurements = &mut report
        .satellites
        .first_mut()
        .expect("satellite")
        .measurements;
    measurements.azimuth = None;
    measurements.elevation = None;
    let track = gt_test_utils::loaded_track_with_points(vec![point, nearby]);
    let mut folds = PointWindowFolds::default();
    let mut harness = test_util::harness_builder().size(egui::vec2(initial_width, 560.0)).ui(move |ui| {
        let index = PointIdx::new(0);
        let point = index.get(&track.points).expect("point");
        let sky = SkySection::resolve(&track, index);
        assert!(matches!(sky, SkySection::Report(report) if report.point_index == PointIdx::new(1) && report.age.is_zero()));
        let _opened = show_sticky_tpv_content(ui, point, &sky, &mut folds, None, FixPlacement::resolve(&track, index));
    });
    harness.run();
    harness.inner.get_by_label("Details").click();
    harness.run();
    harness.inner.get_by_label("Nearby sky details").click();
    harness.run();
    let header = "Observation 1 · 11:33:20.000 · Global";
    let ids: Vec<_> = harness
        .inner
        .query_all_by_label(header)
        .map(|node| node.accesskit_node().id())
        .collect();
    assert_eq!(ids.len(), 2);
    assert_ne!(ids.first(), ids.last());
    let source_values = [
        ["2025-05-23 11:33:21 UTC", "17.25"],
        ["2025-05-23 11:33:22 UTC", "47.75"],
    ];
    for width in [initial_width, resized_width] {
        harness.inner.set_size(egui::vec2(width, 560.0));
        harness.run();
        assert_eq!(
            harness
                .inner
                .query_all_by_label(header)
                .map(|node| node.accesskit_node().id())
                .collect::<Vec<_>>(),
            ids
        );
        for (index, own_values) in source_values.iter().enumerate() {
            for values in source_values {
                for value in values {
                    assert!(
                        harness.inner.query_by_label(value).is_some(),
                        "missing {value}"
                    );
                }
            }
            harness
                .inner
                .query_all_by_label(header)
                .nth(index)
                .expect("report header")
                .scroll_to_me();
            harness.run();
            harness
                .inner
                .query_all_by_label(header)
                .nth(index)
                .expect("report header")
                .click();
            harness.run();
            for value in own_values {
                assert!(
                    harness.inner.query_by_label(value).is_none(),
                    "collapsed report contains {value}"
                );
            }
            let other_values = source_values.get(1 - index).expect("other source");
            for value in other_values {
                assert!(
                    harness.inner.query_by_label(value).is_some(),
                    "other report lost {value}"
                );
            }
            harness
                .inner
                .query_all_by_label(header)
                .nth(index)
                .expect("report header")
                .click();
            harness.run();
        }
    }
}

fn positionless_status_recording(state: SolutionState) -> LoadedFile {
    let mut status = point_at(2, None);
    status.tpv = TimePositionVelocity::builder()
        .time(status.tpv.time())
        .lat(RecordedLatitude::from_degrees(f64::NAN))
        .lon(RecordedLongitude::from_degrees(f64::NAN))
        .build();
    status.navigation.state = Some(state);
    let mut before = point_at(0, None);
    before.navigation.state = Some(SolutionState::Fix);
    let mut after = point_at(5, None);
    after.tpv = TimePositionVelocity::builder()
        .time(after.tpv.time())
        .lat(Latitude::new(FIXTURE_LAT))
        .lon(Longitude::new(FIXTURE_LON + 0.002))
        .build();
    after.navigation.state = Some(SolutionState::Fix);
    gt_test_utils::build_file(
        "status.gtd",
        &[before, status, after],
        gt_test_utils::FileParts::default(),
    )
}

#[rstest]
#[case::fix(SolutionState::Fix, MapFixKind::UnreportedPosition, FIX_BLUE)]
#[case::loss(SolutionState::NoFix, MapFixKind::NavigationLoss, FIX_LOST_RED)]
#[case::estimated(
    SolutionState::EstimatedOnly,
    MapFixKind::Estimated,
    FIX_ESTIMATED_PURPLE
)]
#[case::unknown(
    SolutionState::Unknown,
    MapFixKind::UnreportedPosition,
    FIX_UNKNOWN_GRAY
)]
fn a_positionless_epoch_uses_an_inferred_state_glyph(
    #[case] state: SolutionState,
    #[case] expected_kind: MapFixKind,
    #[case] expected_color: Color32,
) {
    let file = positionless_status_recording(state);
    let track = file.tracks.first().unwrap();
    let status = track.points.get(1).expect("status or recovery epoch");
    assert_eq!(status.map_fix_kind(), expected_kind);
    assert!(status.is_navigation_estimate());
    let glyph = ChevronFix::for_fix(status).expect("inferred status glyph");
    assert_eq!(glyph.tint(false), expected_color);
    assert_eq!(glyph.tint(true), expected_color);
    assert!(matches!(
        FixPlacement::resolve(track, PointIdx::new(1)),
        FixPlacement::Placed(ResolvedPosition::Interpolated(_))
    ));

    let reference = test_util::point_ref(DataCategory::Tpv, 1);
    let mut map = MapScene::of(vec![file])
        .centred_on_fix(1)
        .zoomed_to(18.0)
        .render();
    assert_eq!(
        map.primary_hover_candidate_at(test_util::viewport_center()),
        Some(reference)
    );
    map.draw_state()
        .display_mask
        .set_visible(DisplayCategory::GhostFixes, false);
    assert_eq!(
        map.primary_hover_candidate_at(test_util::viewport_center()),
        None
    );
    assert!(!map.pin(reference));
    map.draw_state()
        .display_mask
        .set_visible(DisplayCategory::GhostFixes, true);
    assert_eq!(
        map.primary_hover_candidate_at(test_util::viewport_center()),
        Some(reference)
    );
    assert!(map.pin(reference));
}

#[rstest]
#[case::dark(true)]
#[case::light(false)]
fn snapshot_the_pinned_positionless_epoch_shows_absent_and_derived_positions(
    #[case] dark_mode: bool,
) {
    let file = positionless_status_recording(SolutionState::NoFix);
    let reference = test_util::point_ref(DataCategory::Tpv, 1);
    let mut map = MapScene::of(vec![file])
        .theme(dark_mode)
        .pinned(reference)
        .render();
    assert!(map.harness.inner.query_by_label("Position").is_some());
    assert!(map.harness.inner.query_by_label("No fix").is_some());
    assert!(map.harness.inner.query_by_label("Drawn at").is_some());
    assert!(
        map.harness
            .inner
            .query_by_label("Derived from recorded positions")
            .is_some()
    );
    assert!(map.harness.inner.query_by_label("Lat").is_none());
    assert!(map.harness.inner.query_by_label("Lon").is_none());
    assert!(map.harness.inner.query_by_label("Sky").is_none());
    assert!(map.harness.inner.query_by_label("Details").is_some());
    map.snapshot(&format!(
        "positionless_status_window_{}",
        if dark_mode { "dark" } else { "light" }
    ));
}

#[test]
fn a_status_only_loss_hover_distinguishes_the_last_fix_from_the_observed_loss() {
    let file = positionless_status_recording(SolutionState::NoFix);
    let track = file.tracks.first().unwrap();
    let loss = track.generated_markers.first().unwrap();
    let recovery = track
        .generated_markers
        .get(1)
        .expect("status or recovery epoch");
    assert_eq!(
        crate::generated_marker_renderer::generated_marker_header(&recovery.kind),
        "GNSS fix regained (5s since last Fix)"
    );
    let mut harness = test_util::harness_builder()
        .size(egui::vec2(420.0, 340.0))
        .ui(|ui| {
            crate::generated_marker_renderer::show_hover_label(ui, track, loss, None);
        });
    harness.run();
    assert!(
        harness
            .inner
            .query_by_label("Last Fix epoch before observed loss")
            .is_some()
    );
    assert!(
        harness
            .inner
            .query_by_label(&fixture_epoch().format(UTC_SECOND_FORMAT).to_string())
            .is_some()
    );
    let loss_observed = fixture_epoch() + chrono::Duration::seconds(2);
    assert!(
        harness
            .inner
            .query_by_label(&loss_observed.format(UTC_SECOND_FORMAT).to_string())
            .is_none()
    );
}

fn loaded_navigation_only_recording() -> LoadedFile {
    let recording =
        gt_test_utils::navigation_evidence_fixtures::sparse_navigation_recording().unwrap();
    let mut bytes = Vec::new();
    recording.write(&mut bytes).unwrap();
    gt_loader::load_bytes(&bytes, "navigation_only.gtd".to_owned()).unwrap()
}

#[rstest]
#[case::headingless(0, MapFixKind::Recorded, FIX_BLUE)]
#[case::rtk(1, MapFixKind::Recorded, FIX_BLUE)]
#[case::status_only(2, MapFixKind::NavigationLoss, FIX_LOST_RED)]
#[case::fix_with_zero(4, MapFixKind::Recorded, FIX_BLUE)]
#[case::explicit_unknown(5, MapFixKind::Recorded, FIX_UNKNOWN_GRAY)]
#[case::inferred_fix(6, MapFixKind::Recorded, FIX_BLUE)]
#[case::unreported_state(7, MapFixKind::Recorded, FIX_UNKNOWN_GRAY)]
#[case::loss_with_count(8, MapFixKind::NavigationLoss, FIX_LOST_RED)]
fn navigation_only_epochs_use_state_colours_and_remain_selectable(
    #[case] index: usize,
    #[case] kind: MapFixKind,
    #[case] color: Color32,
) {
    let file = loaded_navigation_only_recording();
    let point = file
        .tracks
        .first()
        .unwrap()
        .points
        .get(index)
        .expect("navigation epoch");
    assert_eq!(point.map_fix_kind(), kind);
    assert_eq!(tpv_point_color(point), color);
    assert!(point.tpv.eph_m().is_none());
    let reference = test_util::point_ref(DataCategory::Tpv, index);
    let mut map = MapScene::of(vec![file])
        .centred_on_fix(index)
        .zoomed_to(20.0)
        .render();
    assert_eq!(
        map.primary_hover_candidate_at(test_util::viewport_center()),
        Some(reference)
    );
    assert!(map.pin(reference));
}

#[rstest]
fn snapshot_navigation_only_glyphs(#[values(true, false)] dark_mode: bool) {
    let mut map = MapScene::of(vec![loaded_navigation_only_recording()])
        .centred_on_fix(3)
        .zoomed_to(18.0)
        .theme(dark_mode)
        .render();
    map.snapshot(&format!(
        "navigation_only_glyphs_{}",
        if dark_mode { "dark" } else { "light" }
    ));
}

#[rstest]
#[case::headingless(0, "Fix", None)]
#[case::rtk(1, "Fix", Some("Satellites used / seen 8 / —"))]
#[case::status_only(2, "No fix", None)]
#[case::zero(4, "Fix", Some("Satellites used / seen 0 / —"))]
#[case::unknown(5, "Unknown", Some("Satellites used / seen 12 / —"))]
fn snapshot_navigation_only_point_details(
    #[case] index: usize,
    #[case] state: &str,
    #[case] counts: Option<&str>,
    #[values(true, false)] dark_mode: bool,
) {
    let file = loaded_navigation_only_recording();
    let track = file.tracks.first().unwrap();
    let point = track
        .placed_points()
        .unwrap()
        .get(index)
        .expect("navigation epoch");
    let sky = SkySection::resolve(track, PointIdx::new(index));
    assert!(matches!(sky, SkySection::TrackWithoutReports));
    let mut hover = test_util::harness_builder()
        .size(egui::vec2(340.0, 260.0))
        .theme(dark_mode)
        .ui(|ui| {
            show_hover_table(ui, point, &sky, None);
        });
    hover.run();
    assert!(hover.inner.query_by_label(state).is_some());
    assert!(hover.inner.query_by_label("Sky").is_none());
    assert!(hover.inner.query_by_label("Accuracy").is_none());
    if let Some(counts) = counts {
        assert!(hover.inner.query_by_label(counts).is_some());
    }
    if index == 1 {
        assert!(hover.inner.query_by_label("RTK fixed").is_some());
        assert!(hover.inner.query_by_label("H 0.90").is_some());
    }
    if index == 2 {
        assert!(hover.inner.query_by_label("Position").is_some());
        assert!(hover.inner.query_by_label("NaN").is_none());
    }
    let theme = if dark_mode { "dark" } else { "light" };
    hover.snapshot(&format!("navigation_only_hover_{index}_{theme}"));
    let mut snapshots = hover.inner.take_snapshot_results();
    drop(hover);
    let reference = test_util::point_ref(DataCategory::Tpv, index);
    let mut pinned = MapScene::of(vec![file])
        .theme(dark_mode)
        .pinned(reference)
        .render();
    assert!(pinned.harness.inner.query_by_label(state).is_some());
    assert!(pinned.harness.inner.query_by_label("Details").is_some());
    assert!(pinned.harness.inner.query_by_label("Sky").is_none());
    assert!(pinned.harness.inner.query_by_label("Accuracy").is_none());
    if index == 2 {
        assert!(
            pinned
                .harness
                .inner
                .query_by_label("Derived from recorded positions")
                .is_some()
        );
    }
    pinned.snapshot(&format!("navigation_only_window_{index}_{theme}"));
    snapshots.extend(pinned.harness.inner.take_snapshot_results());
}

#[rstest]
fn snapshot_navigation_only_narrow_window(#[values(true, false)] dark_mode: bool) {
    let mut map = MapScene::of(vec![loaded_navigation_only_recording()])
        .size(egui::vec2(330.0, 340.0))
        .theme(dark_mode)
        .pinned(test_util::point_ref(DataCategory::Tpv, 1))
        .render();
    let viewport = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(330.0, 340.0));
    for label in [
        "Details",
        "Fix",
        "RTK fixed",
        "Satellites used / seen 8 / —",
        "H 0.90",
    ] {
        let rect = map.harness.inner.get_by_label(label).rect();
        assert!(viewport.contains_rect(rect), "{label}: {rect:?}");
    }
    assert!(map.harness.inner.query_by_label("Sky").is_none());
    map.snapshot(&format!(
        "navigation_only_narrow_window_{}",
        if dark_mode { "dark" } else { "light" }
    ));
}
