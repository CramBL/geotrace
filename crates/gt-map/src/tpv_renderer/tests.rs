use egui_kittest::kittest::Queryable as _;
use gt_test_utils::{By, HarnessInteraction as _};
use rstest::rstest;

use super::*;
use crate::test_util;
use egui::Color32;
use gt_types::MercPoint;
use gt_types::NavPoint;
use gt_types::coordinates::{Latitude, Longitude, RecordedLatitude};
use gt_types::satellites::{Constellation, NO_DATA_SENTINEL_DB_HZ, Satellite, Satellites};
use gt_types::time_types::GpsTime;
use gt_types::tpv::TimePositionVelocity;
use uom::si::angle::degree;
use uom::si::f64::{Angle, Length};

/// Canvas the sticky-content snapshots render into: as wide as the real point
/// window, where the plot sits beside the satellite tables, and tall enough to
/// hold the whole plot column. A column clipped at its scroll edge leaves a
/// part-drawn row that each GPU backend antialiases differently.
const STICKY_CONTENT_CANVAS: egui::Vec2 = egui::vec2(600.0, 500.0);

/// The position every fix built here sits at.
const FIXTURE_LAT: f64 = 51.5;
const FIXTURE_LON: f64 = -0.1;

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
    point
        .satellites
        .as_ref()
        .map_or(SkySection::TrackWithoutReports, |satellites| {
            SkySection::Report(gt_types::NearestSatelliteReport {
                satellites,
                age: chrono::Duration::zero(),
            })
        })
}

/// The two satellite columns are cut where they come out closest in height,
/// without reordering the constellations.
#[rstest]
// A 40-satellite, 4-constellation fix: GPS 11, GLONASS 8, Galileo 6,
// BeiDou 15 (plus 2 header rows each). Cutting after GLONASS gives
// 13+10=23 against 8+17=25 - the closest of the three possible cuts.
#[case::four_constellations(&[13, 10, 8, 17], 2)]
// Two constellations always split one and one.
#[case::two(&[13, 10], 1)]
// A single dominant constellation still keeps at least one on each side.
#[case::lopsided(&[30, 3, 3], 1)]
// Equal weights cut down the middle.
#[case::even(&[10, 10, 10, 10], 2)]
// Fewer than two panels cannot be split, so everything stays in the first
// column.
#[case::one_panel(&[7], 1)]
#[case::no_panel(&[], 0)]
fn balanced_split_cuts_where_the_columns_even_out(
    #[case] weights: &[usize],
    #[case] expected: usize,
) {
    assert_eq!(super::balanced_split(weights), expected);
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

/// A folded panel costs only its header when the columns are balanced, so
/// folding re-packs the columns.
#[test]
fn folded_panels_weigh_only_their_header() {
    let group = ConstellationGroup {
        grid_id: 0,
        constellation: Constellation::Gps,
        prn_prefix: "G",
        satellites: vec![
            Satellite::new(
                Constellation::Gps,
                1,
                Some(45.0),
                Some(40.0),
                Some(40.0),
                true
            );
            11
        ],
    };
    let unfolded = gt_ui_types::PointWindowFolds::default();
    assert_eq!(group.weight(unfolded), 11 + super::PANEL_HEADER_ROWS);

    let mut folded = unfolded;
    folded.toggle(Constellation::Gps);
    assert_eq!(group.weight(folded), super::FOLDED_PANEL_ROWS);
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
            cell.set(Some(sky_table_highlight_id(ui)));
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
/// the `ctx.data` round trip keyed by [`sky_table_highlight_id`].
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
            cell.set(Some(sky_table_highlight_id(ui)));
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
#[case::earlier(2100, "Report 2.1s earlier")]
#[case::later(-2100, "Report 2.1s later")]
fn report_age_label_names_the_side(#[case] ms: i64, #[case] expected: &str) {
    assert_eq!(
        report_age_label(chrono::Duration::milliseconds(ms)),
        expected
    );
}

/// A fix at [`fixture_epoch`] with neither a heading nor a satellite report,
/// which is what the map draws hollow.
fn a_fix_without_a_heading() -> NavPoint {
    gt_test_utils::fixtures::nav_point(
        fixture_epoch(),
        Latitude::new(FIXTURE_LAT),
        Longitude::new(FIXTURE_LON),
        gt_types::fixtures::FixKind::GhostWithoutHeading,
    )
}

/// The icon colour states the fix quality: blue while the receiver held a
/// strong solution or wrote no report at all, yellow while it had a report but
/// too few satellites in the fix, red once it had none.
#[rstest]
#[case::without_a_report(None, Color32::from_rgb(66, 133, 244))]
#[case::ten_in_fix(Some(twelve_satellites(10)), Color32::from_rgb(66, 133, 244))]
#[case::one_in_fix(Some(twelve_satellites(1)), Color32::from_rgb(244, 180, 0))]
#[case::nothing_in_fix(Some(twelve_satellites(0)), Color32::from_rgb(219, 68, 55))]
fn tpv_point_color_states_the_fix_quality(
    #[case] satellites: Option<Satellites>,
    #[case] expected: Color32,
) {
    assert_eq!(tpv_point_color(&point_at(0, satellites)), expected);
}

/// A fix the receiver did not measure is drawn as a chevron: one shape for a
/// fix it dead reckoned, and one for a fix whose recorded coordinates lie
/// outside their range. Every other fix is drawn as an arrow.
#[rstest]
#[case::measured(point_at(0, None), None)]
#[case::without_a_heading(a_fix_without_a_heading(), Some(ChevronFix::DeadReckoned))]
#[case::with_nothing_in_fix(
    point_at(0, Some(twelve_satellites(0))),
    Some(ChevronFix::DeadReckoned)
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
    #[case] fix: NavPoint,
    #[case] expected: Option<ChevronFix>,
) {
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

// With a 12 px icon, the fade band spans local spacings of 2.4 px
// (LO, 0.2 icon sizes - arrows share almost all pixels) to 6 px
// (HI, 0.5 icon sizes - arrows overlap but stay readable).
const TEST_ICON_PX: f32 = 12.0;

// At low zoom icons shrink to 3 px and the proportional band would be
// 0.6-1.5 px. The absolute floors widen it to 2-5 px so dot-sized
// arrows stacked a couple of pixels apart fade into the quality line.
const SMALL_ICON_PX: f32 = 3.0;

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

/// Same value as `MercTransform::pixels_per_meter`'s internal constant.
/// With `for_test(EARTH_CIRCUMFERENCE_M)` the map scale is 1 px/m at the
/// equator, so test geometry can be written directly in metres.
const EARTH_CIRCUMFERENCE_M: f64 = 40_030_173.0;

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
fn quality_line_color_marks_ghost_fixes_red() {
    // No heading and no satellite report: `tpv_point_color` alone would say
    // blue, but the point is a ghost fix and must show as red.
    assert_eq!(quality_line_color(&a_fix_without_a_heading()), FIX_LOST_RED);
}

#[test]
fn quality_line_color_follows_fix_quality_for_real_fixes() {
    let marginal = point_at(0, Some(twelve_satellites(4)));
    assert_eq!(quality_line_color(&marginal), FIX_MARGINAL_YELLOW);
    let strong = point_at(0, Some(twelve_satellites(12)));
    assert_eq!(quality_line_color(&strong), FIX_STRONG_BLUE);
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

/// The batch flushes at every painter primitive between its icons. The flush
/// takes the form of a mesh because the harness installs no GPU icon pipeline.
#[test]
fn a_tracks_arrows_are_one_mesh_whatever_the_accuracy_circle_count() {
    const FIX_COUNT: usize = 8;
    const SPACING_M: f64 = 20.0;
    const ACCURACY_M: f32 = 10.0;

    let points = (0..FIX_COUNT)
        .map(|i| nav_point_at_meters(i as f64 * SPACING_M, Some(ACCURACY_M)))
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
                &GlobalFilter::default(),
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
    assert_eq!((circles, meshes), (FIX_COUNT, 1));
}
