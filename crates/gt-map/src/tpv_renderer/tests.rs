use egui_kittest::kittest::Queryable as _;
use rstest::rstest;

use super::*;
use egui::Color32;
use gt_types::MercPoint;
use gt_types::NavPoint;
use gt_types::coordinates::{Latitude, Longitude};
use gt_types::satellites::{Constellation, Satellite, Satellites};
use gt_types::time_types::GpsTime;
use gt_types::tpv::TimePositionVelocity;
use uom::si::angle::degree;
use uom::si::f64::{Angle, Length};

fn make_point(satellites: Option<Satellites>) -> NavPoint {
    let tpv = TimePositionVelocity::builder()
        .time(GpsTime::from_utc(chrono::Utc::now()))
        .lat(Latitude::new(51.5))
        .lon(Longitude::new(-0.1))
        .heading(Angle::new::<degree>(90.0))
        .build();
    NavPoint::new(tpv, satellites)
}

fn sats_with_fix(fix_count: u32) -> Satellites {
    let satellites: Vec<_> = (1u32..=12)
        .map(|prn| Satellite::new(Constellation::Gps, prn, None, None, None, prn <= fix_count))
        .collect();
    Satellites::new(None, None, satellites)
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
        // sky instead of stacking on top of each other, and vary SNR and
        // fix state so the table shows a realistic mix.
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
/// satellite badge exercises every count tier, the full SNR gradient,
/// both the in-fix and idle PRN colours, and the sky plot's placed and
/// unplaceable satellites.
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
fn balanced_split_cuts_where_the_columns_even_out(
    #[case] weights: &[usize],
    #[case] expected: usize,
) {
    assert_eq!(super::balanced_split(weights), expected);
}

/// Fewer than two panels cannot be split, so everything stays in the
/// first column.
#[test]
fn balanced_split_keeps_a_lone_panel_in_one_column() {
    assert_eq!(super::balanced_split(&[7]), 1);
    assert_eq!(super::balanced_split(&[]), 0);
}

/// Snapshot: a 40-satellite, 4-constellation fix. The two columns are cut
/// where they even out, so the uneven constellations pack tight, and the plot
/// stays beside them.
#[test]
fn dense_multi_constellation_packs_into_two_columns() {
    let point = make_point(Some(sats_dense_multi_constellation()));
    let mut folds = gt_ui_types::PointWindowFolds::default();
    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(620.0, 560.0))
        .theme(true)
        .ui(move |ui| {
            let _opened = show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None);
        });
    harness.snapshot("sticky_dense_two_columns");
}

/// The same fix in a narrow window falls back to a single column rather
/// than squeezing two.
#[test]
fn dense_multi_constellation_reflows_to_one_column_when_narrow() {
    let point = make_point(Some(sats_dense_multi_constellation()));
    let mut folds = gt_ui_types::PointWindowFolds::default();
    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(330.0, 560.0))
        .theme(true)
        .ui(move |ui| {
            let _opened = show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None);
        });
    harness.snapshot_loose("sticky_dense_one_column");
}

/// A folded panel costs only its header when the columns are balanced, so
/// folding re-packs rather than leaving a column sized for rows that are
/// no longer drawn.
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
    let point = make_point(Some(sats_dense_multi_constellation()));
    let mut folds = gt_ui_types::PointWindowFolds {
        plot_folded: true,
        ..Default::default()
    };
    folds.toggle(Constellation::Gps);
    folds.toggle(Constellation::Beidou);
    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(620.0, 380.0))
        .theme(true)
        .ui(move |ui| {
            let _opened = show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None);
        });
    harness.snapshot("sticky_folded_sections");
}

/// Folding a constellation drops its satellite rows while its header
/// stays, so the window shrinks without hiding what is there.
#[rstest]
#[case::unfolded(false, true)]
#[case::folded(true, false)]
fn folding_a_constellation_hides_only_its_rows(#[case] fold_gps: bool, #[case] expect_rows: bool) {
    let point = make_point(Some(sats_dense_multi_constellation()));
    let mut folds = gt_ui_types::PointWindowFolds::default();
    if fold_gps {
        folds.toggle(Constellation::Gps);
    }
    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(620.0, 560.0))
        .theme(true)
        .ui(move |ui| {
            let _opened = show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None);
        });
    harness.run();

    // The header survives either way; only the PRN rows come and go.
    assert!(
        harness.inner.query_by_label("GPS").is_some(),
        "the constellation header must stay visible when folded"
    );
    assert_eq!(harness.inner.query_by_label("G01").is_some(), expect_rows);
}

#[test]
fn clicking_anywhere_on_the_header_folds() {
    let point = make_point(Some(sats_multi_constellation()));
    let folded = std::rc::Rc::new(std::cell::Cell::new(false));
    let seen = folded.clone();
    let mut folds = gt_ui_types::PointWindowFolds::default();
    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(600.0, 440.0))
        .theme(true)
        .ui(move |ui| {
            let _opened = show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None);
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
    let point = make_point(Some(sats_multi_constellation()));
    let state = std::rc::Rc::new(std::cell::Cell::new((false, false)));
    let seen = state.clone();
    let mut folds = gt_ui_types::PointWindowFolds::default();
    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(600.0, 440.0))
        .theme(true)
        .ui(move |ui| {
            let opened = show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None);
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
/// and a click lands on the wrong panel; this pins the second panel
/// folding itself and leaving the first alone.
#[test]
fn each_header_folds_its_own_constellation() {
    let point = make_point(Some(sats_multi_constellation()));
    let state = std::rc::Rc::new(std::cell::Cell::new((false, false)));
    let seen = state.clone();
    let mut folds = gt_ui_types::PointWindowFolds::default();
    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(600.0, 440.0))
        .theme(true)
        .ui(move |ui| {
            let _opened = show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None);
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
    let point = make_point(Some(sats_multi_constellation()));
    let id_cell = std::rc::Rc::new(std::cell::Cell::new(None));
    let cell = id_cell.clone();
    let mut folds = gt_ui_types::PointWindowFolds::default();
    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(600.0, 440.0))
        .theme(true)
        .ui(move |ui| {
            cell.set(Some(sky_table_highlight_id(ui)));
            let _opened = show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None);
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
/// dark visuals; the light baseline is what catches colours that only read
/// on a dark surface.
#[rstest]
#[case::dark("satellite_badge_dark", true)]
#[case::light("satellite_badge_light", false)]
fn satellite_badge(#[case] name: &str, #[case] dark_mode: bool) {
    let point = make_point(Some(sats_multi_constellation()));
    // Sized like the real point window: the plot sits beside the satellite
    // tables, so this is wide and short rather than narrow and tall.
    let mut folds = gt_ui_types::PointWindowFolds::default();
    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(600.0, 440.0))
        .theme(dark_mode)
        .ui(move |ui| {
            let _opened = show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None);
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
    let point = make_point(Some(sats_multi_constellation()));
    let id_cell = std::rc::Rc::new(std::cell::Cell::new(None));
    let cell = id_cell.clone();
    let mut folds = gt_ui_types::PointWindowFolds::default();
    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(320.0, 920.0))
        .theme(true)
        .ui(move |ui| {
            cell.set(Some(sky_table_highlight_id(ui)));
            let _opened = show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None);
        });
    harness.run();
    harness.inner.get_by_label(label).hover();
    harness.inner.run_steps(2);

    let id = id_cell.get().expect("sticky content rendered");
    let highlight: Option<SkyHighlight> = harness.inner.ctx.data(|d| d.get_temp(id)).flatten();
    assert_eq!(highlight, Some(expected));
}

/// Hovering a highlight target paints a band over it - the affordance
/// that it does something, rather than reading as plain text.
#[test]
fn hovering_a_prn_row_shows_the_affordance_band() {
    let point = make_point(Some(sats_multi_constellation()));
    let mut folds = gt_ui_types::PointWindowFolds::default();
    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(600.0, 440.0))
        .theme(true)
        .ui(move |ui| {
            let _opened = show_sticky_tpv_content(ui, &point, &sky_for(&point), &mut folds, None);
        });
    harness.run();
    harness.inner.get_by_label("G01").hover();
    harness.inner.run_steps(2);
    harness.snapshot("sticky_prn_row_hovered");
}

fn track_with_points(points: Vec<NavPoint>) -> LoadedTrack {
    let satellite_report_count = points.iter().filter(|p| p.satellites.is_some()).count();
    LoadedTrack {
        metadata: gt_types::TrackMetadata {
            satellite_report_count,
            ..gt_test_utils::empty_track_metadata()
        },
        points,
        lod: gt_types::TrackLod::default(),
        sat_label_anchors: Vec::new(),
        custom_markers: Vec::new(),
        generated_markers: Vec::new(),
        event_markers: Vec::new(),
        channels: Vec::new(),
    }
}

/// A nav point at a fixed time plus `secs`, so hover-badge snapshots
/// (which render the time row) stay deterministic.
fn point_at(secs: i64, satellites: Option<Satellites>) -> NavPoint {
    let start = chrono::DateTime::from_timestamp(1_748_000_000, 0).expect("valid");
    let tpv = TimePositionVelocity::builder()
        .time(GpsTime::from_utc(start + chrono::Duration::seconds(secs)))
        .lat(Latitude::new(51.5))
        .lon(Longitude::new(-0.1))
        .heading(Angle::new::<degree>(90.0))
        .build();
    NavPoint::new(tpv, satellites)
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

#[rstest]
#[case::dark("hover_badge_own_report_dark", true)]
#[case::light("hover_badge_own_report_light", false)]
fn hover_badge_own_report(#[case] name: &str, #[case] dark_mode: bool) {
    let track = track_with_points(vec![point_at(0, Some(sats_with_sky()))]);
    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(430.0, 260.0))
        .theme(dark_mode)
        .ui(move |ui| {
            let sky = SkySection::resolve(&track, PointIdx::new(0));
            if let Some(point) = track.points.first() {
                show_hover_table(ui, point, &sky, None);
            }
        });
    harness.snapshot(name);
}

#[rstest]
#[case::borrowed_report("hover_badge_borrowed_report", &[(0, true), (3, false)], 1)]
#[case::no_report_nearby("hover_badge_no_report_nearby", &[(0, true), (60, false)], 1)]
#[case::track_without_reports("hover_badge_track_without_reports", &[(0, false)], 0)]
fn hover_badge_report_states(
    #[case] name: &str,
    #[case] spec: &[(i64, bool)],
    #[case] query: usize,
) {
    let points = spec
        .iter()
        .map(|&(secs, has_report)| point_at(secs, has_report.then(sats_with_sky)))
        .collect();
    let track = track_with_points(points);
    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(430.0, 260.0))
        .theme(true)
        .ui(move |ui| {
            let sky = SkySection::resolve(&track, PointIdx::new(query));
            if let Some(point) = track.points.get(query) {
                show_hover_table(ui, point, &sky, None);
            }
        });
    harness.snapshot(name);
}

/// The recording row names the file a fix came from, and is absent while
/// a single file is loaded.
#[rstest]
#[case::several_files(Some("Morning ride"), true)]
#[case::single_file(None, false)]
fn hover_badge_recording_row(
    #[case] recording_name: Option<&'static str>,
    #[case] expect_row: bool,
) {
    let track = track_with_points(vec![point_at(0, Some(sats_with_sky()))]);
    let mut harness = crate::test_harness::builder()
        .size(egui::vec2(430.0, 260.0))
        .theme(true)
        .ui(move |ui| {
            let sky = SkySection::resolve(&track, PointIdx::new(0));
            if let Some(point) = track.points.first() {
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
#[case::earlier(2100, "Report 2.1 s earlier")]
#[case::later(-2100, "Report 2.1 s later")]
fn report_age_label_names_the_side(#[case] ms: i64, #[case] expected: &str) {
    assert_eq!(
        report_age_label(chrono::Duration::milliseconds(ms)),
        expected
    );
}

fn make_tpv(lat: f64, lon: f64, heading: Option<f64>) -> TimePositionVelocity {
    if let Some(h) = heading {
        TimePositionVelocity::builder()
            .time(GpsTime::from_utc(chrono::Utc::now()))
            .lat(Latitude::new(lat))
            .lon(Longitude::new(lon))
            .heading(Angle::new::<degree>(h))
            .build()
    } else {
        TimePositionVelocity::builder()
            .time(GpsTime::from_utc(chrono::Utc::now()))
            .lat(Latitude::new(lat))
            .lon(Longitude::new(lon))
            .build()
    }
}

/// No satellite report → blue (unknown quality, assume fine).
#[test]
fn color_no_satellite_report_is_blue() {
    let point = make_point(None);
    assert_eq!(tpv_point_color(&point), Color32::from_rgb(66, 133, 244));
}

/// 10+ satellites in fix → blue (strong fix).
#[test]
fn color_strong_fix_is_blue() {
    let point = make_point(Some(sats_with_fix(10)));
    assert_eq!(tpv_point_color(&point), Color32::from_rgb(66, 133, 244));
}

/// 1–9 satellites in fix → yellow (marginal fix).
#[test]
fn color_marginal_fix_is_yellow() {
    let point = make_point(Some(sats_with_fix(5)));
    assert_eq!(tpv_point_color(&point), Color32::from_rgb(244, 180, 0));
}

/// 1 satellite in fix → yellow (lowest marginal threshold).
#[test]
fn color_single_sat_fix_is_yellow() {
    let point = make_point(Some(sats_with_fix(1)));
    assert_eq!(tpv_point_color(&point), Color32::from_rgb(244, 180, 0));
}

/// Satellite report present but 0 in fix → red (fix lost).
#[test]
fn color_fix_lost_is_red() {
    let point = make_point(Some(sats_with_fix(0)));
    assert_eq!(tpv_point_color(&point), Color32::from_rgb(219, 68, 55));
}

/// A point with no heading → classified as ghost (hollow chevron).
#[test]
fn no_heading_is_ghost() {
    let tpv = make_tpv(51.5, -0.1, None);
    let point = NavPoint::new(tpv, None);
    assert!(point.is_ghost_fix());
}

/// A point with heading and no satellite report → classified as Real (blue arrow).
#[test]
fn heading_no_satellite_report_is_real() {
    let tpv = make_tpv(51.5, -0.1, Some(90.0));
    let point = NavPoint::new(tpv, None);
    assert!(!point.is_ghost_fix());
}

/// Fix count > 0 with heading → classified as Real (filled arrow, good fix).
///
/// Dead reckoning or any device that supplies heading during a genuine fix
/// is rendered as a filled arrow.
#[test]
fn heading_with_good_fix_is_real() {
    let tpv = make_tpv(51.5, -0.1, Some(225.0));
    let point = NavPoint::new(tpv, Some(sats_with_fix(5)));
    assert!(!point.is_ghost_fix());
}

/// Fix count == 0 → ghost even when heading is present.
///
/// This is the common case for devices that continue outputting heading
/// estimates after fix loss. Without any satellite in the fix, the heading
/// is an internal guess and the icon should clearly signal uncertainty.
#[test]
fn heading_with_fix_lost_is_ghost() {
    let tpv = make_tpv(51.5, -0.1, Some(180.0));
    let point = NavPoint::new(tpv, Some(sats_with_fix(0)));
    assert!(point.is_ghost_fix());
}

/// Ghost chevron points east when the surrounding fixes move eastward.
#[test]
fn ghost_direction_points_east_for_eastward_movement() {
    let prev = MercPoint { x: 0.50, y: 0.50 };
    let next = MercPoint { x: 0.60, y: 0.50 };
    let dir = ghost_direction(prev, next);
    assert!(
        dir.x > 0.99,
        "eastward movement → large positive x; got {dir:?}"
    );
    assert!(
        dir.y.abs() < 0.01,
        "eastward movement → near-zero y; got {dir:?}"
    );
}

/// Ghost chevron points south when the surrounding fixes move southward.
/// Mercator y increases southward, so this also tests that no Y-flip is applied.
#[test]
fn ghost_direction_points_south_for_southward_movement() {
    let prev = MercPoint { x: 0.50, y: 0.40 };
    let next = MercPoint { x: 0.50, y: 0.60 };
    let dir = ghost_direction(prev, next);
    assert!(
        dir.y > 0.99,
        "southward movement → large positive y; got {dir:?}"
    );
    assert!(
        dir.x.abs() < 0.01,
        "southward movement → near-zero x; got {dir:?}"
    );
}

/// When prev and next coincide (isolated point) the direction falls back to DOWN.
#[test]
fn ghost_direction_falls_back_when_neighbours_coincide() {
    let pt = MercPoint { x: 0.5, y: 0.5 };
    let dir = ghost_direction(pt, pt);
    assert_eq!(
        dir,
        Vec2::DOWN,
        "coincident neighbours → fallback direction DOWN"
    );
}

// With a 12 px icon, the fade band spans local spacings of 2.4 px
// (LO, 0.2 icon sizes - arrows share almost all pixels) to 6 px
// (HI, 0.5 icon sizes - arrows overlap but stay readable).
const TEST_ICON_PX: f32 = 12.0;

#[test]
fn icon_fade_is_opaque_while_arrows_merely_overlap() {
    assert!(icon_fade_alpha(100.0, TEST_ICON_PX) >= 1.0);
    assert!(icon_fade_alpha(12.0, TEST_ICON_PX) >= 1.0); // fully side by side
    assert!(icon_fade_alpha(8.0, TEST_ICON_PX) >= 1.0); // overlapping a bit
    assert!(icon_fade_alpha(6.0, TEST_ICON_PX) >= 1.0); // exactly at the HI bound
}

#[test]
fn icon_fade_is_transparent_when_arrows_blend_together() {
    assert!(icon_fade_alpha(2.4, TEST_ICON_PX) <= 0.0); // exactly at the LO bound
    assert!(icon_fade_alpha(0.0, TEST_ICON_PX) <= 0.0); // stacked on one point
}

#[test]
fn icon_fade_is_linear_between_the_bounds() {
    let alpha = icon_fade_alpha(4.2, TEST_ICON_PX); // midway between 2.4 and 6
    assert!((alpha - 0.5).abs() < 1e-6);
}

#[test]
fn icon_fade_stays_opaque_for_degenerate_icon_size() {
    assert!(icon_fade_alpha(10.0, 0.0) >= 1.0);
    assert!(icon_fade_alpha(10.0, -1.0) >= 1.0);
}

// At low zoom icons shrink to 3 px and the proportional band would be
// 0.6-1.5 px. The absolute floors widen it to 2-5 px so dot-sized
// arrows stacked a couple of pixels apart fade into the quality line.
const SMALL_ICON_PX: f32 = 3.0;

#[test]
fn icon_fade_band_is_floored_for_small_icons() {
    assert!(icon_fade_alpha(1.2, SMALL_ICON_PX) <= 0.0); // below the 2 px floor
    assert!(icon_fade_alpha(2.0, SMALL_ICON_PX) <= 0.0); // exactly at the LO floor
    assert!(icon_fade_alpha(5.0, SMALL_ICON_PX) >= 1.0); // exactly at the HI floor
    let alpha = icon_fade_alpha(3.5, SMALL_ICON_PX); // midway between 2 and 5
    assert!((alpha - 0.5).abs() < 1e-6);
}

#[test]
fn classify_uses_the_floored_band_for_small_icons() {
    // 1.9 m segments at 1 px/m: below the 2 px floor, fully hidden even
    // though 1.9 px is well above 0.2 x 3 px.
    let track = track_with_segment_range(0.0, 1.9);
    assert_eq!(
        classify_icon_fade(&track, unit_transform().scale(), SMALL_ICON_PX),
        TrackIconFade::AllHidden
    );
    let track = track_with_segment_range(5.0, 50.0);
    assert_eq!(
        classify_icon_fade(&track, unit_transform().scale(), SMALL_ICON_PX),
        TrackIconFade::AllVisible
    );
    let track = track_with_segment_range(3.0, 4.0);
    assert_eq!(
        classify_icon_fade(&track, unit_transform().scale(), SMALL_ICON_PX),
        TrackIconFade::PerFix
    );
}

#[test]
fn icon_fade_stays_opaque_for_infinite_spacing() {
    // Spacing can overflow to infinity when a long track meets an
    // extreme zoom. The result must clamp to opaque, not turn NaN.
    assert!(icon_fade_alpha(f32::INFINITY, TEST_ICON_PX) >= 1.0);
}

/// Same value as `MercTransform::pixels_per_meter`'s internal constant;
/// with `for_test(EARTH_CIRCUMFERENCE_M)` the map scale is 1 px/m at the
/// equator, so test geometry can be written directly in metres.
const EARTH_CIRCUMFERENCE_M: f64 = 40_030_173.0;

fn unit_transform() -> crate::transform::MercTransform {
    crate::transform::MercTransform::for_test(EARTH_CIRCUMFERENCE_M)
}

/// A real fix on the equator, `x_m` metres east of the origin.
fn nav_point_at_meters(x_m: f64) -> NavPoint {
    let lon_deg = x_m * 360.0 / EARTH_CIRCUMFERENCE_M;
    NavPoint::new(make_tpv(0.0, lon_deg, Some(90.0)), None)
}

fn track_with_segment_range(min_m: f64, max_m: f64) -> LoadedTrack {
    LoadedTrack {
        metadata: gt_types::TrackMetadata {
            segment_length_range: Some(gt_types::SegmentLengthRange {
                min: Length::new::<meter>(min_m),
                max: Length::new::<meter>(max_m),
            }),
            ..gt_test_utils::empty_track_metadata()
        },
        points: Vec::new(),
        lod: gt_types::TrackLod::default(),
        sat_label_anchors: Vec::new(),
        custom_markers: Vec::new(),
        generated_markers: Vec::new(),
        event_markers: Vec::new(),
        channels: Vec::new(),
    }
}

#[test]
fn classify_keeps_lone_fix_visible_at_every_zoom() {
    // No segments means nothing can overlap. A spacing of zero would
    // hide the lone fix forever.
    let track = track_with_points(Vec::new());
    assert_eq!(
        classify_icon_fade(&track, unit_transform().scale(), TEST_ICON_PX),
        TrackIconFade::AllVisible
    );
}

#[test]
fn classify_hides_all_icons_when_even_the_longest_segment_blends() {
    // Longest segment 2 m = 2 px, below the 2.4 px fade-out bound.
    let track = track_with_segment_range(0.0, 2.0);
    assert_eq!(
        classify_icon_fade(&track, unit_transform().scale(), TEST_ICON_PX),
        TrackIconFade::AllHidden
    );
}

#[test]
fn classify_shows_all_icons_when_even_the_shortest_segment_is_spaced() {
    // Shortest segment 6 m = 6 px, exactly the fade-in bound.
    let track = track_with_segment_range(6.0, 100.0);
    assert_eq!(
        classify_icon_fade(&track, unit_transform().scale(), TEST_ICON_PX),
        TrackIconFade::AllVisible
    );
}

#[test]
fn classify_mixed_spacing_decides_per_fix() {
    // Parked-then-highway: zero-length segments next to 100 m hops.
    let track = track_with_segment_range(0.0, 100.0);
    assert_eq!(
        classify_icon_fade(&track, unit_transform().scale(), TEST_ICON_PX),
        TrackIconFade::PerFix
    );
    // A range entirely inside the fade band is also per-fix.
    let track = track_with_segment_range(3.0, 5.0);
    assert_eq!(
        classify_icon_fade(&track, unit_transform().scale(), TEST_ICON_PX),
        TrackIconFade::PerFix
    );
}

fn spacing_at(track: &LoadedTrack, pi: usize) -> Option<f32> {
    let transform = unit_transform();
    let screen_pos = transform.to_screen(track.points[pi].merc);
    local_fix_spacing_px(track, pi, screen_pos, &transform)
}

#[test]
fn local_spacing_is_none_for_a_lone_fix() {
    let track = track_with_points(vec![nav_point_at_meters(0.0)]);
    assert_eq!(spacing_at(&track, 0), None);
}

#[test]
fn local_spacing_of_endpoints_uses_their_single_neighbour() {
    let track = track_with_points(vec![nav_point_at_meters(0.0), nav_point_at_meters(100.0)]);
    let first = spacing_at(&track, 0).expect("has a neighbour");
    let last = spacing_at(&track, 1).expect("has a neighbour");
    assert!((first - 100.0).abs() < 1.0, "got {first} px");
    assert!((last - 100.0).abs() < 1.0, "got {last} px");
}

#[test]
fn local_spacing_keeps_cluster_boundary_visible() {
    // Three stacked fixes (parked), then a 100 m hop: the interior
    // parked fixes have zero spacing, but the departure fix sees its
    // far next-neighbour and must stay visible.
    let track = track_with_points(vec![
        nav_point_at_meters(0.0),
        nav_point_at_meters(0.0),
        nav_point_at_meters(0.0),
        nav_point_at_meters(100.0),
    ]);
    let interior = spacing_at(&track, 1).expect("has neighbours");
    let departure = spacing_at(&track, 2).expect("has neighbours");
    assert!(interior < f32::EPSILON, "got {interior} px");
    assert!((departure - 100.0).abs() < 1.0, "got {departure} px");
}

#[test]
fn fix_icon_alpha_short_circuits_uniform_tracks() {
    let track = track_with_points(vec![nav_point_at_meters(0.0), nav_point_at_meters(0.0)]);
    let transform = unit_transform();
    let pos = transform.to_screen(track.points[0].merc);
    // AllHidden / AllVisible ignore local spacing entirely.
    let hidden = fix_icon_alpha(
        TrackIconFade::AllHidden,
        &track,
        0,
        pos,
        TEST_ICON_PX,
        &transform,
    );
    let visible = fix_icon_alpha(
        TrackIconFade::AllVisible,
        &track,
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
    let track = track_with_points(vec![
        nav_point_at_meters(0.0),
        nav_point_at_meters(0.0),
        nav_point_at_meters(0.0), // departure: next neighbour is far
        nav_point_at_meters(100.0),
        nav_point_at_meters(200.0),
        nav_point_at_meters(300.0), // arrival: prev neighbour is far
        nav_point_at_meters(300.0),
        nav_point_at_meters(300.0),
    ]);
    let transform = unit_transform();
    let alpha_at = |pi: usize| {
        let pos = transform.to_screen(track.points[pi].merc);
        fix_icon_alpha(
            TrackIconFade::PerFix,
            &track,
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
    // Out-of-range inputs clamp instead of wrapping.
    assert_eq!(line_alpha_bucket(-1.0), 0);
    assert_eq!(line_alpha_bucket(2.0), QUALITY_LINE_ALPHA_STEPS);
    assert!((bucket_alpha(QUALITY_LINE_ALPHA_STEPS) - 1.0).abs() < f32::EPSILON);
    assert!(bucket_alpha(0) < f32::EPSILON);
}

#[test]
fn quality_line_color_marks_ghost_fixes_red() {
    // No heading and no satellite report: tpv_point_color alone would say
    // blue, but the point is a ghost fix and must show as red.
    let tpv = make_tpv(51.5, -0.1, None);
    let point = NavPoint::new(tpv, None);
    assert_eq!(quality_line_color(&point), FIX_LOST_RED);
}

#[test]
fn quality_line_color_follows_fix_quality_for_real_fixes() {
    let marginal = make_point(Some(sats_with_fix(4)));
    assert_eq!(quality_line_color(&marginal), FIX_MARGINAL_YELLOW);
    let strong = make_point(Some(sats_with_fix(12)));
    assert_eq!(quality_line_color(&strong), FIX_STRONG_BLUE);
}

#[test]
fn split_spans_by_single_key_is_one_sub_span() {
    use egui::pos2;
    let span = [
        (Color32::BLUE, pos2(0.0, 0.0)),
        (Color32::BLUE, pos2(10.0, 0.0)),
        (Color32::BLUE, pos2(20.0, 0.0)),
    ];
    let subs = split_spans_by(&span, |k| k);
    assert_eq!(
        subs,
        vec![(
            Color32::BLUE,
            vec![pos2(0.0, 0.0), pos2(10.0, 0.0), pos2(20.0, 0.0)]
        )]
    );
}

#[test]
fn split_spans_by_edge_takes_key_of_its_starting_point() {
    use egui::pos2;
    let span = [
        (Color32::BLUE, pos2(0.0, 0.0)),
        (Color32::YELLOW, pos2(10.0, 0.0)),
        (Color32::YELLOW, pos2(20.0, 0.0)),
    ];
    let subs = split_spans_by(&span, |k| k);
    // The blue->yellow edge is blue (starting point's quality). The
    // boundary point is shared so the line stays continuous.
    assert_eq!(
        subs,
        vec![
            (Color32::BLUE, vec![pos2(0.0, 0.0), pos2(10.0, 0.0)]),
            (Color32::YELLOW, vec![pos2(10.0, 0.0), pos2(20.0, 0.0)]),
        ]
    );
}

#[test]
fn split_spans_by_splits_on_alpha_bucket_within_one_color() {
    use egui::pos2;
    // Same quality color but different crossfade buckets: the line must
    // split so an opaque stretch (a parked cluster, bucket 3) and an
    // invisible stretch (well-spaced fixes, bucket 0) get separate
    // strokes - this is what localizes the quality line to the cluster.
    // Each edge takes its starting point's bucket, so the transition
    // edge still belongs to the cluster.
    let span = [
        ((Color32::BLUE, 3_u8), pos2(0.0, 0.0)),
        ((Color32::BLUE, 3_u8), pos2(10.0, 0.0)),
        ((Color32::BLUE, 0_u8), pos2(20.0, 0.0)),
        ((Color32::BLUE, 0_u8), pos2(30.0, 0.0)),
    ];
    let subs = split_spans_by(&span, |k| k);
    assert_eq!(
        subs,
        vec![
            (
                (Color32::BLUE, 3_u8),
                vec![pos2(0.0, 0.0), pos2(10.0, 0.0), pos2(20.0, 0.0)]
            ),
            (
                (Color32::BLUE, 0_u8),
                vec![pos2(20.0, 0.0), pos2(30.0, 0.0)]
            ),
        ]
    );
}

#[test]
fn split_spans_by_too_short_span_is_empty() {
    use egui::pos2;
    assert!(split_spans_by::<Color32, Color32>(&[], |k| k).is_empty());
    assert!(split_spans_by(&[(Color32::BLUE, pos2(0.0, 0.0))], |k| k).is_empty());
}
