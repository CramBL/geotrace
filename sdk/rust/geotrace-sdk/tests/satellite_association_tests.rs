//! Tests for satellite-report association logic.
//!
//! See `docs/satellite-association.md` for the authoritative description of
//! what these tests are verifying.  Tests are grouped by the phase they target
//! (Phase 1 = nearest-fix assignment, Phase 2 = ghost-fix creation for orphans).
#![expect(
    clippy::panic_in_result_fn,
    reason = "test functions mix ? propagation with assert! - both are correct in test code"
)]
#![expect(
    clippy::unwrap_in_result,
    reason = "test code may use expect() for infallible test invariants"
)]

use geotrace_sdk::{Angle, DateTime, Duration, Utc};
use geotrace_sdk::{BuildError, Constellation, NavFileBuilder, NavFix, Satellite, SatelliteReport};

/// A fixed base epoch for all tests (2025-05-23 UTC, arbitrary but stable).
#[expect(clippy::expect_used, reason = "fixed timestamp is always valid")]
fn base() -> DateTime<Utc> {
    DateTime::from_timestamp(1_748_000_000, 0).expect("valid")
}

fn t(offset_ms: i64) -> DateTime<Utc> {
    base() + Duration::milliseconds(offset_ms)
}

fn t_us(offset_us: i64) -> DateTime<Utc> {
    base() + Duration::microseconds(offset_us)
}

/// NavFix at `(lat, lon)` with `gps_time = t(offset_ms)` and heading north.
fn fix_at(offset_ms: i64, lat: f64, lon: f64) -> NavFix {
    NavFix::builder()
        .gps_time(t(offset_ms))
        .lat(Angle::degrees(lat))
        .lon(Angle::degrees(lon))
        .heading(Angle::degrees(0.0))
        .build()
}

/// `SatelliteReport` with `gps_time = t(offset_ms)` and a single GPS satellite.
fn report_gps(offset_ms: i64) -> SatelliteReport {
    report_with(offset_ms, Constellation::Gps, 1)
}

/// `SatelliteReport` with `gps_time = t(offset_ms)` and a single satellite
/// of `constellation` with PRN `prn`.
fn report_with(offset_ms: i64, constellation: Constellation, prn: u32) -> SatelliteReport {
    SatelliteReport::builder()
        .gps_time(t(offset_ms))
        .tracked(vec![
            Satellite::builder()
                .constellation(constellation)
                .prn(prn)
                .in_fix(true)
                .build(),
        ])
        .build()
}

/// Extract the constellation of the first tracked satellite in `p`'s report.
/// Panics if there is no report or no satellite - intended for assertions.
#[expect(clippy::expect_used, reason = "test helper")]
#[expect(clippy::indexing_slicing, reason = "test helper")]
fn first_constellation(p: &geotrace_sdk::NavPoint) -> Constellation {
    p.satellites
        .as_ref()
        .expect("expected a satellite report")
        .tracked[0]
        .constellation
}

/// The association window comparison is `<=`, not `<`.
/// A report at exactly `window` distance from a fix must be assigned to it.
#[test]
fn window_boundary_inclusive() -> Result<(), BuildError> {
    let mut recorder = NavFileBuilder::new()
        .with_satellite_window(Duration::milliseconds(100))
        .open();
    recorder.add_nav_fix(fix_at(0, 55.0, 12.0));
    recorder.add_satellite_report(report_gps(100)); // exactly 100 ms away
    let nav_file = recorder.finish()?;

    assert_eq!(nav_file.nav_points().len(), 1, "no ghost fix expected");
    assert!(
        nav_file.nav_points()[0].satellites.is_some(),
        "report at exactly the boundary must be assigned"
    );
    Ok(())
}

/// One microsecond past the window boundary falls outside (`dist > window`).
/// The report must become a ghost fix, not be silently dropped.
#[test]
fn window_boundary_one_microsecond_past_is_excluded() -> Result<(), BuildError> {
    let mut recorder = NavFileBuilder::new()
        .with_satellite_window(Duration::milliseconds(100))
        .open();
    recorder.add_nav_fix(fix_at(0, 0.0, 0.0));
    // 100 ms + 1 μs → just outside the window.
    recorder.add_satellite_report(
        SatelliteReport::builder()
            .gps_time(t_us(100_001))
            .tracked(vec![
                Satellite::builder()
                    .constellation(Constellation::Gps)
                    .prn(1u32)
                    .build(),
            ])
            .build(),
    );

    let nav_file = recorder.finish()?;

    // Real fix + 1 dead-reckoned ghost.
    assert_eq!(nav_file.nav_points().len(), 2, "one ghost fix expected");
    assert!(
        nav_file.nav_points()[0].satellites.is_none(),
        "real fix must have no satellite data"
    );
    assert!(
        nav_file.nav_points()[1].satellites.is_some(),
        "ghost must carry the report"
    );
    Ok(())
}

/// When a report falls between two fixes, it is assigned to the nearer one.
/// Here the report is closer to fix B (t=2 000 ms) than to fix A (t=0).
#[test]
fn report_goes_to_nearer_of_two_candidate_fixes() -> Result<(), BuildError> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix_at(0, 10.0, 10.0)); // fix A - GPS constellation expected
    recorder.add_nav_fix(fix_at(2000, 20.0, 20.0)); // fix B - Glonass constellation expected
    // Report at t=1 800 ms: 1 800 ms from A (outside 500 ms window), 200 ms from B (inside).
    recorder.add_satellite_report(report_with(1800, Constellation::Gps, 1));

    let nav_file = recorder.finish()?;
    let points = nav_file.nav_points();

    assert_eq!(points.len(), 2, "no ghost fixes expected");
    assert!(
        points[0].satellites.is_none(),
        "fix A (too far) must have no satellite data"
    );
    assert!(
        points[1].satellites.is_some(),
        "fix B (nearer) must carry the report"
    );
    Ok(())
}

/// When a report is equidistant between two fixes (and both are within the
/// window), it must go to the earlier fix.
#[test]
fn report_equidistant_goes_to_earlier_fix() -> Result<(), BuildError> {
    // Use a 2 000 ms window so both fixes are candidates.
    let mut recorder = NavFileBuilder::new()
        .with_satellite_window(Duration::milliseconds(2000))
        .open();
    recorder.add_nav_fix(fix_at(0, 10.0, 10.0)); // fix A
    recorder.add_nav_fix(fix_at(2000, 20.0, 20.0)); // fix B
    // Report at t=1 000 ms: exactly 1 000 ms from both fixes.
    recorder.add_satellite_report(report_gps(1000));

    let nav_file = recorder.finish()?;
    let points = nav_file.nav_points();

    assert_eq!(points.len(), 2, "no ghost fixes expected");
    assert!(
        points[0].satellites.is_some(),
        "earlier fix (A) must win the tie"
    );
    assert!(
        points[1].satellites.is_none(),
        "later fix (B) must have no satellite data"
    );
    Ok(())
}

/// Four fixes spaced 2 s apart, each with one nearby report.
/// All reports must be matched. No ghost fixes created.
#[test]
fn four_reports_matched_to_four_fixes_no_ghosts() -> Result<(), BuildError> {
    let mut recorder = NavFileBuilder::new().open();
    for i in 0..4_i64 {
        recorder.add_nav_fix(fix_at(i * 2000, 10.0 + i as f64, 10.0));
        // Each report is 100 ms after its corresponding fix - well within the window.
        recorder.add_satellite_report(report_with(
            i * 2000 + 100,
            [
                Constellation::Gps,
                Constellation::Glonass,
                Constellation::Galileo,
                Constellation::Beidou,
            ][usize::try_from(i).expect("fits")],
            u32::try_from(i).expect("fits") + 1,
        ));
    }

    let nav_file = recorder.finish()?;
    let points = nav_file.nav_points();

    assert_eq!(points.len(), 4, "no ghost fixes expected");
    assert_eq!(first_constellation(&points[0]), Constellation::Gps);
    assert_eq!(first_constellation(&points[1]), Constellation::Glonass);
    assert_eq!(first_constellation(&points[2]), Constellation::Galileo);
    assert_eq!(first_constellation(&points[3]), Constellation::Beidou);
    Ok(())
}

/// When a report supplies only `sys_time` (no `gps_time`), `sys_time` is used
/// as the comparison timestamp.  If `sys_time` happens to place the report
/// within the window of a fix (e.g. when the clocks agree), the report is
/// assigned to that fix.
#[test]
fn sys_time_only_report_within_window_is_assigned() -> Result<(), BuildError> {
    let mut recorder = NavFileBuilder::new().open();
    // Fix with `gps_time` = t(0).  The fix `effective_time` is t(0).
    recorder.add_nav_fix(fix_at(0, 55.0, 12.0));
    // Report with only `sys_time` = t(200).  `rep_us` = `sys_time` = t(200).
    // Distance to fix: 200 ms - inside the 500 ms window.
    recorder.add_satellite_report(
        SatelliteReport::builder()
            .sys_time(t(200)) // no `gps_time`
            .tracked(vec![
                Satellite::builder()
                    .constellation(Constellation::Gps)
                    .prn(1u32)
                    .in_fix(true)
                    .build(),
            ])
            .build(),
    );

    let nav_file = recorder.finish()?;

    assert_eq!(nav_file.nav_points().len(), 1, "no ghost expected");
    assert!(
        nav_file.nav_points()[0].satellites.is_some(),
        "sys_time-only report within window must be assigned"
    );
    Ok(())
}

/// When a report has both `gps_time` and `sys_time`, `gps_time` is used for
/// the comparison, not `sys_time`.
///
/// Layout:
///   Fix A  at t=0
///   Fix B  at t=2 000 ms
///   Report: gps_time=t(200) [200 ms from A - inside window]
///           sys_time=t(1800) [200 ms from B - inside window if `sys_time` were used]
///
/// If `gps_time` is preferred, the report goes to Fix A.
/// If `sys_time` were used instead, it would go to Fix B.
#[test]
fn gps_time_is_preferred_over_sys_time_for_comparison() -> Result<(), BuildError> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix_at(0, 10.0, 10.0)); // fix A
    recorder.add_nav_fix(fix_at(2000, 20.0, 20.0)); // fix B

    recorder.add_satellite_report(
        SatelliteReport::builder()
            .gps_time(t(200)) // 200 ms from fix A → inside window
            .sys_time(t(1800)) // 200 ms from fix B → would go to B if `sys_time` were used
            .tracked(vec![
                Satellite::builder()
                    .constellation(Constellation::Gps)
                    .prn(1u32)
                    .build(),
            ])
            .build(),
    );

    let nav_file = recorder.finish()?;
    let points = nav_file.nav_points();

    assert_eq!(points.len(), 2, "no ghost fixes expected");
    assert!(
        points[0].satellites.is_some(),
        "gps_time was used: report must go to fix A (t=0)"
    );
    assert!(
        points[1].satellites.is_none(),
        "fix B must have no satellite data"
    );
    Ok(())
}

/// A report with neither `gps_time` nor `sys_time` must be discarded in
/// `finish()` before Phase 1 or Phase 2 runs.
/// No ghost fix must be created for it.
#[test]
fn report_with_no_timestamp_is_discarded() -> Result<(), BuildError> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix_at(0, 55.0, 12.0));
    recorder.add_satellite_report(
        SatelliteReport::builder()
            // neither `gps_time` nor `sys_time` - dropped in finish()
            .tracked(vec![
                Satellite::builder()
                    .constellation(Constellation::Gps)
                    .prn(1u32)
                    .build(),
            ])
            .build(),
    );

    let nav_file = recorder.finish()?;

    assert_eq!(
        nav_file.nav_points().len(),
        1,
        "discarded report must not create a ghost fix"
    );
    assert!(
        nav_file.nav_points()[0].satellites.is_none(),
        "fix must have no satellite data (report was discarded)"
    );
    Ok(())
}

/// With no satellite reports at all, every fix must have `satellites = None`
/// and no ghost fixes must be created.
#[test]
fn zero_reports_all_fixes_have_no_satellite_data() -> Result<(), BuildError> {
    let mut recorder = NavFileBuilder::new().open();
    for i in 0..3_i64 {
        recorder.add_nav_fix(fix_at(i * 1000, 55.0, 12.0));
    }

    let nav_file = recorder.finish()?;
    let points = nav_file.nav_points();

    assert_eq!(points.len(), 3, "no ghost fixes expected");
    assert!(
        points.iter().all(|p| p.satellites.is_none()),
        "all fixes must have satellites = None"
    );
    Ok(())
}

/// Three reports all within the window of the same fix.
/// The closest must win. The other two must become ghost fixes.
#[test]
fn three_reports_one_fix_only_closest_wins() -> Result<(), BuildError> {
    // Fix A at t=0, Fix B at t=5 000 ms (far enough that none of the reports reach it).
    // R1 at t=100 ms → 100 ms from A (winner).
    // R2 at t=200 ms → 200 ms from A (first loser).
    // R3 at t=300 ms → 300 ms from A (second loser).
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix_at(0, 55.0, 12.0));
    recorder.add_nav_fix(fix_at(5000, 55.1, 12.1));

    recorder.add_satellite_report(report_with(100, Constellation::Gps, 1)); // R1 - winner
    recorder.add_satellite_report(report_with(200, Constellation::Glonass, 2)); // R2 - loser
    recorder.add_satellite_report(report_with(300, Constellation::Galileo, 3)); // R3 - loser

    let nav_file = recorder.finish()?;
    let points = nav_file.nav_points();

    // 2 real fixes + 2 ghost fixes for the two losers = 4 total.
    assert_eq!(
        points.len(),
        4,
        "expected 2 real + 2 ghost fixes, got {}",
        points.len()
    );

    // Fix A (first point by time) carries R1 (GPS).
    assert_eq!(first_constellation(&points[0]), Constellation::Gps);

    // Both losers became ghost fixes between A and B.
    // Verify by checking that GPS is only at index 0 and the remaining two
    // ghosts carry Glonass and Galileo in time order.
    let ghost_constellations: Vec<Constellation> =
        points[1..=2].iter().map(first_constellation).collect();
    assert!(
        ghost_constellations.contains(&Constellation::Glonass),
        "R2 (Glonass) must be a ghost"
    );
    assert!(
        ghost_constellations.contains(&Constellation::Galileo),
        "R3 (Galileo) must be a ghost"
    );

    // Fix B (last point) carries no satellite data.
    assert!(
        points[3].satellites.is_none(),
        "fix B must have no satellite data"
    );
    Ok(())
}

/// When two reports are equidistant from the same fix, the one that arrived
/// earlier (lower insertion order) wins - even when added to the builder after
/// the later one.
#[test]
fn equidistant_reports_earlier_one_wins() -> Result<(), BuildError> {
    // Fix at t=500 ms.  Both reports are 250 ms away.
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix_at(500, 55.0, 12.0));
    // Add the LATER report first to confirm it doesn't win just by insertion order.
    recorder.add_satellite_report(report_with(750, Constellation::Glonass, 99)); // later
    recorder.add_satellite_report(report_with(250, Constellation::Gps, 1)); // earlier

    let nav_file = recorder.finish()?;

    // Earlier report wins. One ghost for the loser.
    assert_eq!(nav_file.nav_points().len(), 2);
    assert_eq!(
        first_constellation(&nav_file.nav_points()[0]),
        Constellation::Gps
    );
    Ok(())
}

/// Three orphan reports after the last real fix must each produce a ghost fix
/// with `heading = None` (rendered as a circle, not an arrow).
/// They must be in chronological order.
#[test]
fn multiple_ghosts_after_last_fix_all_have_heading_none() -> Result<(), BuildError> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix_at(0, 0.0, 0.0));
    // Three reports well past the last fix.
    recorder.add_satellite_report(report_with(2000, Constellation::Gps, 1));
    recorder.add_satellite_report(report_with(3000, Constellation::Glonass, 2));
    recorder.add_satellite_report(report_with(4000, Constellation::Galileo, 3));

    let nav_file = recorder.finish()?;
    let points = nav_file.nav_points();

    // 1 real fix + 3 ghost fixes = 4 total.
    assert_eq!(points.len(), 4, "expected 1 real + 3 ghost fixes");
    assert!(
        points[0].fix.heading.is_some(),
        "real fix must retain its heading"
    );
    for (i, ghost) in points[1..].iter().enumerate() {
        assert!(
            ghost.fix.heading.is_none(),
            "ghost fix {i} must have heading=None (circle rendering)"
        );
        assert!(
            ghost.satellites.is_some(),
            "ghost fix {i} must carry a satellite report"
        );
    }

    // Chronological order: GPS (t=2 000), Glonass (t=3 000), Galileo (t=4 000).
    assert_eq!(first_constellation(&points[1]), Constellation::Gps);
    assert_eq!(first_constellation(&points[2]), Constellation::Glonass);
    assert_eq!(first_constellation(&points[3]), Constellation::Galileo);
    Ok(())
}

/// The second ghost after the last real fix must be placed further from the
/// fix than the first ghost (1 m then 2 m stepping).
#[test]
fn second_ghost_after_last_fix_is_further_than_first() -> Result<(), BuildError> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t(0))
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(0.0))
            .heading(Angle::degrees(0.0)) // heading north
            .build(),
    );
    recorder.add_satellite_report(report_gps(2000));
    recorder.add_satellite_report(report_gps(3000));

    let nav_file = recorder.finish()?;
    let points = nav_file.nav_points();

    assert_eq!(points.len(), 3);
    let fix_lat = points[0].fix.lat.as_degrees();
    let g1_lat = points[1].fix.lat.as_degrees();
    let g2_lat = points[2].fix.lat.as_degrees();

    // Both ghosts must be north of the real fix (heading = 0°).
    assert!(g1_lat > fix_lat, "first ghost must be north of fix");
    assert!(
        g2_lat > g1_lat,
        "second ghost must be further north than first"
    );

    // First ghost ≈ 1 m ahead. Second ≈ 1 + 2 = 3 m ahead from fix.
    let deg_per_metre = 1.0_f64 / 111_320.0;
    let d1 = g1_lat - fix_lat;
    let d2 = g2_lat - fix_lat;

    assert!(
        (d1 - deg_per_metre).abs() < deg_per_metre * 0.2,
        "first ghost should be ~1 m from fix, got {:.2} m",
        d1 / deg_per_metre
    );
    assert!(
        (d2 - 3.0 * deg_per_metre).abs() < deg_per_metre * 0.3,
        "second ghost should be ~3 m from fix (1 + 2), got {:.2} m",
        d2 / deg_per_metre
    );
    Ok(())
}

/// Ghost fixes between two real fixes must be interpolated at the correct
/// fractional position along the segment.
///
/// Both fixes supply `gps_time` and `sys_time`. The GPS/system-clock delta is
/// known and constant, so `segment_corrected_gps_us` applies the correction.
///
/// Setup:
///   Fix B: gps_time=t(0),      sys_time=t(1 000) → delta = −1 000 ms
///   Fix A: gps_time=t(10 000), sys_time=t(11 000) → delta = −1 000 ms
///   Report: sys_time=t(5 000), no `gps_time`
///     corrected GPS time = `sys_time` + delta = 5 000 − 1 000 = 4 000 ms
///     fraction = 4 000 / 10 000 = 0.40
///
/// Fix B = (lat=0, lon=0), Fix A = (lat=10, lon=0).
/// Expected ghost position: lat = 0 + 0.40 × 10 = 4.0, lon = 0.
#[test]
fn between_fix_ghost_interpolated_at_correct_fraction() -> Result<(), BuildError> {
    let mut recorder = NavFileBuilder::new().open();

    // Fix B: `gps_time` ahead of `sys_time` by 1 000 ms.
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t(0))
            .sys_time(t(1000))
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(0.0))
            .heading(Angle::degrees(90.0))
            .build(),
    );
    // Fix A: same constant delta.
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t(10_000))
            .sys_time(t(11_000))
            .lat(Angle::degrees(10.0))
            .lon(Angle::degrees(0.0))
            .heading(Angle::degrees(90.0))
            .build(),
    );

    // Report: `sys_time` only.  Corrected GPS time = t(4 000) → `frac` = 0.40.
    recorder.add_satellite_report(
        SatelliteReport::builder()
            .sys_time(t(5000)) // no `gps_time`, and 5 000 ms from both fixes' `gps_time` → orphan
            .tracked(vec![
                Satellite::builder()
                    .constellation(Constellation::Gps)
                    .prn(1u32)
                    .in_fix(true)
                    .build(),
            ])
            .build(),
    );

    let nav_file = recorder.finish()?;
    let points = nav_file.nav_points();

    // Fix B (t=0) + ghost (t≈4 000 ms) + Fix A (t=10 000 ms) = 3 total.
    assert_eq!(
        points.len(),
        3,
        "expected fix B + 1 ghost + fix A = 3 points, got {}",
        points.len()
    );

    // The ghost is the middle point after time-sorting.
    let ghost = &points[1];
    assert!(ghost.satellites.is_some(), "ghost must carry the report");
    assert!(
        ghost.fix.heading.is_some(),
        "between-fix ghost has a bearing heading"
    );

    let ghost_lat = ghost.fix.lat.as_degrees();
    let ghost_lon = ghost.fix.lon.as_degrees();

    assert!(
        (ghost_lat - 4.0).abs() < 0.01,
        "ghost lat should be ~4.0° (40% of 0→10), got {ghost_lat:.4}"
    );
    assert!(
        ghost_lon.abs() < 0.001,
        "ghost lon should be ~0.0° (no lon variation), got {ghost_lon:.4}"
    );
    Ok(())
}

/// When no clock delta can be computed (fixes have no `sys_time`, and reports
/// have no `gps_time`), ghost fixes are evenly distributed along the segment.
/// This complements `ghost_points_between_fixes_are_evenly_distributed` by
/// explicitly verifying the "no information" fallback.
#[test]
fn between_fix_ghosts_evenly_distributed_when_no_delta_available() -> Result<(), BuildError> {
    let mut recorder = NavFileBuilder::new().open();

    // Fixes with `gps_time` only - no `sys_time`, so no delta anchors.
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t(0))
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(0.0))
            .heading(Angle::degrees(90.0))
            .build(),
    );
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t(10_000))
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(10.0))
            .heading(Angle::degrees(90.0))
            .build(),
    );

    // Two reports with `sys_time` only - no `gps_time`, no delta → even distribution.
    // `sys_time` values are clustered near the end, but the ghost positions must
    // ignore that and distribute evenly at fractions 1/3 and 2/3.
    for sys_offset_ms in [8000_i64, 9000] {
        recorder.add_satellite_report(
            SatelliteReport::builder()
                .sys_time(t(sys_offset_ms))
                .tracked(vec![
                    Satellite::builder()
                        .constellation(Constellation::Gps)
                        .prn(1u32)
                        .build(),
                ])
                .build(),
        );
    }

    let nav_file = recorder.finish()?;
    let points = nav_file.nav_points();

    // Fix (t=0) + ghost1 + ghost2 + Fix (t=10 000) = 4 total.
    assert_eq!(points.len(), 4, "expected 4 points");

    let lon1 = points[1].fix.lon.as_degrees();
    let lon2 = points[2].fix.lon.as_degrees();

    // Even distribution: fractions 1/3 and 2/3 of [0, 10].
    assert!(
        (lon1 - 10.0 / 3.0).abs() < 0.01,
        "first ghost lon should be ~3.33°, got {lon1:.4}"
    );
    assert!(
        (lon2 - 20.0 / 3.0).abs() < 0.01,
        "second ghost lon should be ~6.67°, got {lon2:.4}"
    );
    Ok(())
}

/// Reports before the first real fix are dropped (no reference position).
/// Reports after the last fix become dead-reckoned ghosts.
/// Both must be handled correctly when they appear together.
#[test]
fn pre_fix_dropped_post_fix_ghosted_in_same_batch() -> Result<(), BuildError> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(fix_at(5000, 0.0, 0.0));

    // Pre-first-fix report - must be dropped.
    recorder.add_satellite_report(report_with(0, Constellation::Gps, 1));
    // Post-last-fix report - must become a ghost.
    recorder.add_satellite_report(report_with(10_000, Constellation::Glonass, 2));

    let nav_file = recorder.finish()?;
    let points = nav_file.nav_points();

    // 1 real fix + 1 post-fix ghost = 2 total (pre-fix report is dropped).
    assert_eq!(
        points.len(),
        2,
        "pre-fix report must be dropped; post-fix must become ghost"
    );
    assert!(points[0].fix.heading.is_some(), "real fix");
    assert!(points[0].satellites.is_none(), "real fix has no report");
    assert!(
        points[1].fix.heading.is_none(),
        "ghost has no heading (circle)"
    );
    assert_eq!(first_constellation(&points[1]), Constellation::Glonass);
    Ok(())
}

/// A single fix with no heading cannot supply a travel direction for
/// dead-reckoned ghosts.  The builder must not panic. It must still produce
/// ghost fixes at some valid position.
#[test]
fn ghost_after_fix_with_no_heading_does_not_panic() -> Result<(), BuildError> {
    let mut recorder = NavFileBuilder::new().open();
    // Fix with no heading (e.g. a ghost fix used as a real fix by a caller).
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t(0))
            .lat(Angle::degrees(55.0))
            .lon(Angle::degrees(12.0))
            // no heading
            .build(),
    );
    recorder.add_satellite_report(report_gps(2000));

    let nav_file = recorder.finish()?;

    // Must not panic and must produce the ghost.
    assert_eq!(nav_file.nav_points().len(), 2);
    assert!(nav_file.nav_points()[1].satellites.is_some());
    Ok(())
}

/// In the `--no-filter` pipeline, SAT records carry only `sys_time`. TPV records
/// carry both `gps_time` and `sys_time`.
/// When the GPS/sys-clock offset exceeds the association window a naive comparison
/// of `SAT.sys_time` against `TPV.gps_time` places every report outside the window
/// - all become orphans - causing the alternating real-fix (no-sat) / ghost (with-sat) pattern observed in practice.
///
/// The improved algorithm applies the GPS/sys-clock delta (derived from fixes that
/// have both timestamps) to correct `sys_time`-only reports into the GPS time domain
/// before the window comparison, so association succeeds regardless of offset size.
///
/// Setup (D = −2 000 ms, i.e. GPS clock is 2 s behind the system clock):
///   Fix 0: gps_time=t(0),     sys_time=t(2 000)  → delta = −2 000 ms
///   Fix 1: gps_time=t(1 000), sys_time=t(3 000)  → delta = −2 000 ms
///   Fix 2: gps_time=t(2 000), sys_time=t(4 000)  → delta = −2 000 ms
///   SAT 0: sys_time=t(2 000)  → corrected = t(0)     → assigned to Fix 0
///   SAT 1: sys_time=t(3 000)  → corrected = t(1 000) → assigned to Fix 1
///   SAT 2: sys_time=t(4 000)  → corrected = t(2 000) → assigned to Fix 2
///
/// Expected: all 3 SAT reports assigned. Exactly 3 nav points (no ghost fixes).
#[test]
fn no_filter_sys_time_only_with_large_gps_offset_are_associated() -> Result<(), BuildError> {
    const GPS_SYS_OFFSET_MS: i64 = 2_000; // `sys_time` is 2 s ahead of `gps_time`

    let mut recorder = NavFileBuilder::new().open();
    for i in 0..3_i64 {
        recorder.add_nav_fix(
            NavFix::builder()
                .gps_time(t(i * 1_000))
                .sys_time(t(i * 1_000 + GPS_SYS_OFFSET_MS))
                .lat(Angle::degrees(55.0 + i as f64 * 0.1))
                .lon(Angle::degrees(12.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
        // SAT record: only `sys_time`, at the same moment as the fix's `sys_time`.
        recorder.add_satellite_report(
            SatelliteReport::builder()
                .sys_time(t(i * 1_000 + GPS_SYS_OFFSET_MS))
                .tracked(vec![
                    Satellite::builder()
                        .constellation(Constellation::Gps)
                        .prn(u32::try_from(i).expect("fits") + 1)
                        .in_fix(true)
                        .build(),
                ])
                .build(),
        );
    }

    let nav_file = recorder.finish()?;
    let points = nav_file.nav_points();

    assert_eq!(
        points.len(),
        3,
        "all 3 SAT reports should associate to their matching fix; \
         no ghost fixes expected, got {}",
        points.len()
    );
    for (i, p) in points.iter().enumerate() {
        assert!(
            p.satellites.is_some(),
            "fix {i} must have an associated satellite report"
        );
    }
    Ok(())
}

/// Extending the offset scenario: 6 fixes and 6 sys_time-only SAT reports at
/// 1 Hz each, with an offset large enough that a naïve comparison would make
/// all 6 reports fall outside the 500 ms window.
/// After delta correction every report must land exactly on its matching fix.
#[test]
fn no_filter_1hz_all_sat_associated_with_large_gps_offset() -> Result<(), BuildError> {
    const GPS_SYS_OFFSET_MS: i64 = 1_800; // comfortably beyond the 500 ms window

    let mut recorder = NavFileBuilder::new().open();
    for i in 0..6_i64 {
        recorder.add_nav_fix(
            NavFix::builder()
                .gps_time(t(i * 1_000))
                .sys_time(t(i * 1_000 + GPS_SYS_OFFSET_MS))
                .lat(Angle::degrees(55.0 + i as f64 * 0.01))
                .lon(Angle::degrees(12.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
        recorder.add_satellite_report(
            SatelliteReport::builder()
                .sys_time(t(i * 1_000 + GPS_SYS_OFFSET_MS))
                .tracked(vec![
                    Satellite::builder()
                        .constellation(Constellation::Gps)
                        .prn(u32::try_from(i).expect("fits") + 1)
                        .in_fix(true)
                        .build(),
                ])
                .build(),
        );
    }

    let nav_file = recorder.finish()?;
    let points = nav_file.nav_points();

    assert_eq!(
        points.len(),
        6,
        "expected 6 matched points (no ghost fixes), got {}",
        points.len()
    );
    assert!(
        points.iter().all(|p| p.satellites.is_some()),
        "every fix must have an associated satellite report"
    );
    Ok(())
}

/// Regression test: when GPS time is ahead of system time by ~600 ms (as
/// observed on the test device), each sys_time-only SAT report must associate
/// to the fix from its own GPS epoch, not the neighboring one.
///
/// ## Root-cause sketch
/// `best_guess_gps_us` corrects `SAT.sys_time` into the GPS domain by adding
/// the delta from the nearest anchor (nearest by anchor-sys_time distance,
/// where `anchor_sys_time = anchor_gps_time − delta`).
///
/// When GPS is ahead by D and the TPV + SAT messages for one epoch are logged
/// with a small real-time gap ε between them:
///
///   Fix i:  `gps_time` = t(i·1000),  `sys_time` = t(i·1000 − D)
///   SAT i:  `sys_time` = t(i·1000 − D + ε)
///
///   corrected(SAT i) = `sys_time` + D = t(i·1000 + ε)   → distance ε from Fix i ✓
///
/// But anchor selection is "nearest by sys-clock distance".  The anchor for Fix i
/// has `anchor_sys_time` = t(i·1000 − D).  If ε > D/2 = 300 ms, the SAT's
/// `sys_time` is closer (in sys-clock space) to Fix i+1's anchor than to Fix i's
/// anchor - but the delta values are identical so the corrected time is still
/// t(i·1000 + ε).  The distance to Fix i is ε, still within the window.
///
/// The failure occurs when ε > window (500 ms): the corrected time falls
/// outside the window for Fix i and inside the window for Fix i+1.
///
/// This test covers the common case (ε ≈ sys-time logging jitter, a few ms)
/// and verifies correct constellation-per-fix assignment.
///
/// Layout (D = 600 ms, GPS 600 ms ahead of `sys_time`, ε ≈ 0 ms):
///   Fix 0:  gps=t(0),    sys=t(−600)  - GPS
///   Fix 1:  gps=t(1000), sys=t(400)   - Galileo
///   Fix 2:  gps=t(2000), sys=t(1400)  - GLONASS
///   Fix 3:  gps=t(3000), sys=t(2400)  - BeiDou
///   SAT i:  `sys_time = Fix[i].sys_time`  (same host-clock moment as the fix)
///
/// Expected: Fix[i] carries SAT[i]'s constellation. 4 nav points, no ghosts.
#[test]
fn gps_ahead_600ms_sat_associates_to_own_fix_not_neighbor() -> Result<(), BuildError> {
    const GPS_SYS_OFFSET_MS: i64 = 600; // GPS clock ahead of system clock

    let constellations = [
        Constellation::Gps,
        Constellation::Galileo,
        Constellation::Glonass,
        Constellation::Beidou,
    ];

    let mut recorder = NavFileBuilder::new().open();
    for (i, &constellation) in constellations.iter().enumerate() {
        let i = i as i64;
        recorder.add_nav_fix(
            NavFix::builder()
                .gps_time(t(i * 1_000))
                .sys_time(t(i * 1_000 - GPS_SYS_OFFSET_MS))
                .lat(Angle::degrees(55.0 + i as f64 * 0.01))
                .lon(Angle::degrees(12.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
        // SAT logged at the same host-clock moment as the TPV (ε = 0).
        recorder.add_satellite_report(
            SatelliteReport::builder()
                .sys_time(t(i * 1_000 - GPS_SYS_OFFSET_MS))
                .tracked(vec![
                    Satellite::builder()
                        .constellation(constellation)
                        .prn(u32::try_from(i).expect("fits") + 1)
                        .in_fix(true)
                        .build(),
                ])
                .build(),
        );
    }

    let nav_file = recorder.finish()?;
    let points = nav_file.nav_points();

    assert_eq!(
        points.len(),
        4,
        "all 4 SAT reports must associate to their own fix; no ghost fixes, got {}",
        points.len()
    );
    for (i, &expected) in constellations.iter().enumerate() {
        assert_eq!(
            first_constellation(&points[i]),
            expected,
            "fix {i} must carry {expected:?} (its own SAT), not {:?}",
            first_constellation(&points[i])
        );
    }
    Ok(())
}

/// Same scenario as above but the SAT message arrives at the host slightly
/// later than the TPV (modelling serial-port message sequencing where GSV
/// follows GGA): SAT.sys_time = Fix.sys_time + 200 ms.
///
/// 200 ms is a realistic delay for ~20 GPGSV sentences at 9600 baud.
/// The corrected time = Fix.gps_time + 200 ms - 200 ms from the current fix,
/// 800 ms from the next - so the association must still be correct.
#[test]
fn gps_ahead_600ms_with_sat_logging_delay_no_off_by_one() -> Result<(), BuildError> {
    const GPS_SYS_OFFSET_MS: i64 = 600;
    const SAT_DELAY_MS: i64 = 200; // SAT message arrives 200 ms after TPV

    let constellations = [
        Constellation::Gps,
        Constellation::Galileo,
        Constellation::Glonass,
        Constellation::Beidou,
    ];

    let mut recorder = NavFileBuilder::new().open();
    for (i, &constellation) in constellations.iter().enumerate() {
        let i = i as i64;
        recorder.add_nav_fix(
            NavFix::builder()
                .gps_time(t(i * 1_000))
                .sys_time(t(i * 1_000 - GPS_SYS_OFFSET_MS))
                .lat(Angle::degrees(55.0 + i as f64 * 0.01))
                .lon(Angle::degrees(12.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
        recorder.add_satellite_report(
            SatelliteReport::builder()
                .sys_time(t(i * 1_000 - GPS_SYS_OFFSET_MS + SAT_DELAY_MS))
                .tracked(vec![
                    Satellite::builder()
                        .constellation(constellation)
                        .prn(u32::try_from(i).expect("fits") + 1)
                        .in_fix(true)
                        .build(),
                ])
                .build(),
        );
    }

    let nav_file = recorder.finish()?;
    let points = nav_file.nav_points();

    assert_eq!(
        points.len(),
        4,
        "200 ms SAT delay must not cause ghost fixes, got {}",
        points.len()
    );
    for (i, &expected) in constellations.iter().enumerate() {
        assert_eq!(
            first_constellation(&points[i]),
            expected,
            "fix {i} must carry {expected:?} even with a 200 ms SAT logging delay"
        );
    }
    Ok(())
}

/// Boundary test: SAT.sys_time = Fix.sys_time + 499 ms (just inside the window
/// after delta correction).
/// corrected = Fix.gps_time + 499 ms → 499 ms from current fix, 501 ms from
/// next → must still go to the correct fix.
#[test]
fn gps_ahead_600ms_sat_at_499ms_delay_still_correct() -> Result<(), BuildError> {
    const GPS_SYS_OFFSET_MS: i64 = 600;
    const SAT_DELAY_MS: i64 = 499;

    let mut recorder = NavFileBuilder::new().open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t(0))
            .sys_time(t(-GPS_SYS_OFFSET_MS))
            .lat(Angle::degrees(55.0))
            .lon(Angle::degrees(12.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t(1_000))
            .sys_time(t(1_000 - GPS_SYS_OFFSET_MS))
            .lat(Angle::degrees(55.01))
            .lon(Angle::degrees(12.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    // SAT for fix 0 arrives 499 ms late in sys-time.
    recorder.add_satellite_report(
        SatelliteReport::builder()
            .sys_time(t(-GPS_SYS_OFFSET_MS + SAT_DELAY_MS))
            .tracked(vec![
                Satellite::builder()
                    .constellation(Constellation::Gps)
                    .prn(1)
                    .in_fix(true)
                    .build(),
            ])
            .build(),
    );

    let nav_file = recorder.finish()?;
    let points = nav_file.nav_points();

    // No ghosts - the SAT must go to fix 0, not fix 1.
    assert_eq!(
        points.len(),
        2,
        "SAT at 499 ms delay must associate to fix 0; no ghosts, got {}",
        points.len()
    );
    assert!(
        points[0].satellites.is_some(),
        "fix 0 must carry the satellite report"
    );
    assert!(
        points[1].satellites.is_none(),
        "fix 1 must have no satellite data"
    );
    Ok(())
}

/// At exactly 500 ms delay the two anchors and the two fixes are all equidistant.
/// The tie-breaking rules (first anchor wins, earlier fix wins) conspire to give
/// the correct result.  This confirms the window boundary is ≤, not <.
#[test]
fn gps_ahead_600ms_sat_at_exactly_500ms_delay_boundary() -> Result<(), BuildError> {
    const GPS_SYS_OFFSET_MS: i64 = 600;
    const SAT_DELAY_MS: i64 = 500; // exactly at the window boundary

    let mut recorder = NavFileBuilder::new()
        .with_satellite_window(Duration::milliseconds(500))
        .open();
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t(0))
            .sys_time(t(-GPS_SYS_OFFSET_MS))
            .lat(Angle::degrees(55.0))
            .lon(Angle::degrees(12.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_nav_fix(
        NavFix::builder()
            .gps_time(t(1_000))
            .sys_time(t(1_000 - GPS_SYS_OFFSET_MS))
            .lat(Angle::degrees(55.01))
            .lon(Angle::degrees(12.0))
            .heading(Angle::degrees(0.0))
            .build(),
    );
    recorder.add_satellite_report(
        SatelliteReport::builder()
            .sys_time(t(-GPS_SYS_OFFSET_MS + SAT_DELAY_MS))
            .tracked(vec![
                Satellite::builder()
                    .constellation(Constellation::Gps)
                    .prn(1)
                    .in_fix(true)
                    .build(),
            ])
            .build(),
    );

    let nav_file = recorder.finish()?;
    let points = nav_file.nav_points();

    assert_eq!(
        points.len(),
        2,
        "SAT at 500 ms delay must still associate; no ghosts"
    );
    assert!(
        points[0].satellites.is_some(),
        "fix 0 must carry the report"
    );
    assert!(
        points[1].satellites.is_none(),
        "fix 1 must have no satellite data"
    );
    Ok(())
}

/// When both the report and the candidate fix have `sys_time`, the association
/// distance is computed as `|report.sys_time − fix.sys_time|` directly.
///
/// This matters when the GPS/sys-clock offset drifts across a track: a single
/// delta anchor would introduce approximation error in the distance to the
/// non-nearest candidate.  Direct `sys_time` comparison is always exact.
///
/// Setup - GPS/sys offset changes from fix to fix:
///   Fix 0: gps=t(0),    sys=t(100)   (GPS 100 ms behind `sys_time`)  - GPS
///   Fix 1: gps=t(1000), sys=t(1600)  (GPS 600 ms behind `sys_time`)  - Galileo
///   Fix 2: gps=t(2000), sys=t(2250)  (GPS 250 ms behind `sys_time`)  - GLONASS
///   Fix 3: gps=t(3000), sys=t(3450)  (GPS 450 ms behind `sys_time`)  - BeiDou
///
/// Each SAT report's `sys_time` matches its fix's `sys_time` exactly (ε = 0).
/// Expected: all 4 reports assigned to their own fix with no ghost fixes.
#[test]
fn sys_time_direct_comparison_with_drifting_gps_offset() -> Result<(), BuildError> {
    let constellations = [
        Constellation::Gps,
        Constellation::Galileo,
        Constellation::Glonass,
        Constellation::Beidou,
    ];

    // GPS/sys offset per fix in milliseconds (varies, simulating a drifting clock).
    let offsets_ms: [i64; 4] = [100, 600, 250, 450];

    let mut recorder = NavFileBuilder::new().open();
    for (i, (&constellation, &off)) in constellations.iter().zip(offsets_ms.iter()).enumerate() {
        let i = i as i64;
        recorder.add_nav_fix(
            NavFix::builder()
                .gps_time(t(i * 1_000))
                .sys_time(t(i * 1_000 + off)) // `sys_time` ahead of GPS by `off`
                .lat(Angle::degrees(55.0 + i as f64 * 0.01))
                .lon(Angle::degrees(12.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
        // SAT recorded at the same host-clock moment as the TPV.
        recorder.add_satellite_report(
            SatelliteReport::builder()
                .sys_time(t(i * 1_000 + off))
                .tracked(vec![
                    Satellite::builder()
                        .constellation(constellation)
                        .prn(u32::try_from(i).expect("fits") + 1)
                        .in_fix(true)
                        .build(),
                ])
                .build(),
        );
    }

    let nav_file = recorder.finish()?;
    let points = nav_file.nav_points();

    assert_eq!(
        points.len(),
        4,
        "all 4 SAT reports must associate to their own fix; no ghost fixes, got {}",
        points.len()
    );
    for (i, &expected) in constellations.iter().enumerate() {
        assert_eq!(
            first_constellation(&points[i]),
            expected,
            "fix {i} must carry {expected:?} even with a drifting GPS/sys offset"
        );
    }
    Ok(())
}

/// Like the previous test but with a non-zero SAT logging delay (200 ms after
/// the fix's `sys_time`), modelling the realistic case where GPGSV sentences
/// arrive after the GGA sentence on the same serial port.
///
/// With drifting offsets the GPS-domain corrected estimate would compute a
/// different `rep_us` for each epoch. The `sys_time` comparison is always exact.
#[test]
fn sys_time_direct_comparison_drifting_offset_with_sat_delay() -> Result<(), BuildError> {
    const SAT_DELAY_MS: i64 = 200; // realistic GPGSV logging delay

    let constellations = [
        Constellation::Gps,
        Constellation::Galileo,
        Constellation::Glonass,
        Constellation::Beidou,
    ];

    let offsets_ms: [i64; 4] = [50, 550, 300, 480];

    let mut recorder = NavFileBuilder::new().open();
    for (i, (&constellation, &off)) in constellations.iter().zip(offsets_ms.iter()).enumerate() {
        let i = i as i64;
        recorder.add_nav_fix(
            NavFix::builder()
                .gps_time(t(i * 1_000))
                .sys_time(t(i * 1_000 + off))
                .lat(Angle::degrees(55.0 + i as f64 * 0.01))
                .lon(Angle::degrees(12.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
        // SAT arrives `SAT_DELAY_MS` after the TPV's `sys_time` - still well inside the window.
        recorder.add_satellite_report(
            SatelliteReport::builder()
                .sys_time(t(i * 1_000 + off + SAT_DELAY_MS))
                .tracked(vec![
                    Satellite::builder()
                        .constellation(constellation)
                        .prn(u32::try_from(i).expect("fits") + 1)
                        .in_fix(true)
                        .build(),
                ])
                .build(),
        );
    }

    let nav_file = recorder.finish()?;
    let points = nav_file.nav_points();

    assert_eq!(
        points.len(),
        4,
        "200 ms SAT delay with drifting offset must not cause ghost fixes, got {}",
        points.len()
    );
    for (i, &expected) in constellations.iter().enumerate() {
        assert_eq!(
            first_constellation(&points[i]),
            expected,
            "fix {i} must carry {expected:?} with drifting offset + 200 ms delay"
        );
    }
    Ok(())
}

/// With no nav fixes at all, `finish()` succeeds (no annotations to fail on)
/// and returns an empty nav-point list.
/// Satellite reports are silently dropped since there is nothing to ghost from.
#[test]
fn no_nav_fixes_no_annotations_returns_empty() -> Result<(), BuildError> {
    let mut recorder = NavFileBuilder::new().open();
    recorder.add_satellite_report(report_gps(0));
    recorder.add_satellite_report(report_gps(1000));

    let nav_file = recorder.finish()?;

    assert_eq!(
        nav_file.nav_points().len(),
        0,
        "no fixes → no nav points; reports cannot be ghosted"
    );
    Ok(())
}
