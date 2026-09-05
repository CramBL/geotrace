//! Generates `tests/fixtures/snapshot.gtd` at the workspace root and verifies
//! the structure produced by `gt_loader::load_file`.
//!
//! The fixture is deterministic so it can be committed and referenced by GUI
//! snapshot tests.
//!
//! ## Layout
//!
//! Track 0 - 12 points at 30 s intervals, all with satellite reports.
//!   Points 0-4: GPS fix held (5 satellites in fix).
//!   Point 5:    fix lost (0 in fix)   -> generates GnssFixLost marker.
//!   Point 6:    still lost.
//!   Point 7:    fix regained (4 in fix) -> generates GnssFixRegained marker.
//!   Points 8-11: fix maintained.
//!   Custom markers at t+75 ("Bike lock spot") and t+225 ("Coffee stop").
//!
//! 7-minute gap between tracks.
//!
//! Track 1 - 8 points at 30 s intervals, no satellite reports.
//!   Custom marker at t+780 ("Checkpoint").

#![expect(
    clippy::expect_used,
    reason = "test fixture helpers use expect() for setup invariants"
)]
#![expect(
    clippy::indexing_slicing,
    reason = "test fixture uses known-length arrays indexed within bounds"
)]
#![expect(
    clippy::cognitive_complexity,
    reason = "test fixture setup is inherently complex"
)]

use std::{fs, path::PathBuf};

use geotrace_sdk::{
    Angle, Annotation, Constellation as SdkConst, DateTime, Duration, MarkerIcon as SdkIcon,
    NavFileBuilder, NavFix, NavFixTime, Satellite as SdkSat, SatelliteReport, Utc, Velocity,
};
use gt_test_utils::assert_matches_sequence;
use gt_types::GeneratedMarkerKind;
use uom::si::f64::Length;
use uom::si::length::kilometer;

fn fixture_path() -> PathBuf {
    // The workspace root is two levels up from crates/gt-loader.
    let manifest = gt_test_utils::cargo_manifest_dir();
    let workspace = manifest
        .parent()
        .expect("crates/ dir")
        .parent()
        .expect("workspace root");
    workspace.join("tests/fixtures/snapshot.gtd")
}

/// 2024-06-15 08:00:00 UTC
fn base() -> DateTime<Utc> {
    DateTime::from_timestamp(1_718_438_400, 0).expect("fixed timestamp is always valid")
}

fn offset(base: DateTime<Utc>, secs: i64) -> DateTime<Utc> {
    base + Duration::seconds(secs)
}

fn a(deg: f64) -> Angle {
    Angle::degrees(deg)
}

fn v(mps: f64) -> Velocity {
    Velocity::meter_per_second(mps)
}

fn satellite_report(
    time: DateTime<Utc>,
    tracked_prns: &[u32],
    fix_prns: &[u32],
) -> SatelliteReport {
    let tracked: Vec<SdkSat> = tracked_prns
        .iter()
        .map(|&prn| {
            SdkSat::builder()
                .constellation(SdkConst::Gps)
                .prn(prn)
                .elevation(35.0_f32)
                .azimuth(120.0_f32)
                .snr(38.0_f32)
                .in_fix(fix_prns.contains(&prn))
                .build()
        })
        .collect();
    SatelliteReport::builder()
        .gps_time(time)
        .tracked(tracked)
        .build()
}

fn build_snapshot_bytes() -> Vec<u8> {
    let base = base();
    let mut recorder = NavFileBuilder::new().with_scrubbed_provenance().open();

    // Track 0: 12 points, Copenhagen area moving NE, all with satellite data
    let trip0_lats = [
        55.6760_f64,
        55.6766,
        55.6772,
        55.6778,
        55.6784,
        55.6790,
        55.6796,
        55.6802,
        55.6808,
        55.6814,
        55.6820,
        55.6826,
    ];
    let trip0_lons = [
        12.5683_f64,
        12.5689,
        12.5695,
        12.5701,
        12.5707,
        12.5713,
        12.5719,
        12.5725,
        12.5731,
        12.5737,
        12.5743,
        12.5749,
    ];
    // Satellite fix counts per point: 5, 5, 5, 5, 5, 0 (lost), 0, 4 (regained), 4, 4, 4, 4
    let fix_prn_sets: [&[u32]; 12] = [
        &[1, 3, 7, 12, 17], // 5 in fix
        &[1, 3, 7, 12, 17],
        &[1, 3, 7, 12, 17],
        &[1, 3, 7, 12, 17],
        &[1, 3, 7, 12, 17],
        &[],            // 0 in fix → GPS fix lost
        &[],            // still lost
        &[1, 3, 7, 12], // 4 in fix → GPS fix regained
        &[1, 3, 7, 12],
        &[1, 3, 7, 12],
        &[1, 3, 7, 12],
        &[1, 3, 7, 12],
    ];
    let tracked_prns = [1u32, 3, 7, 12, 17];

    for i in 0..12_usize {
        let pt_time = offset(base, i as i64 * 30);
        recorder.add_nav_fix(
            NavFix::builder()
                .time(NavFixTime::Receiver(pt_time))
                .lat(a(trip0_lats[i]))
                .lon(a(trip0_lons[i]))
                .heading(a(45.0))
                .speed(v(2.6))
                .build(),
        );
        recorder.add_satellite_report(satellite_report(pt_time, &tracked_prns, fix_prn_sets[i]));
    }

    // Custom markers within Track 0 time range [t+0, t+330]
    recorder.add_annotation(
        Annotation::builder()
            .time(offset(base, 75))
            .icon(SdkIcon::Pin)
            .maybe_label(Some("Bike lock spot".to_owned()))
            .build()
            .expect("the marker label fits its field"),
    );
    recorder.add_annotation(
        Annotation::builder()
            .time(offset(base, 225))
            .icon(SdkIcon::Circle)
            .maybe_label(Some("Coffee stop".to_owned()))
            .build()
            .expect("the marker label fits its field"),
    );

    // Track 1: 8 points, starts 7 min after Track 0 ends (t+330 → t+750)
    // No satellite data, slightly different area
    let trip1_lats = [
        55.6750_f64,
        55.6757,
        55.6764,
        55.6771,
        55.6778,
        55.6785,
        55.6792,
        55.6799,
    ];
    let trip1_lons = [
        12.5600_f64,
        12.5607,
        12.5614,
        12.5621,
        12.5628,
        12.5635,
        12.5642,
        12.5649,
    ];

    for i in 0..8_usize {
        let pt_time = offset(base, 750 + i as i64 * 30);
        recorder.add_nav_fix(
            NavFix::builder()
                .time(NavFixTime::Receiver(pt_time))
                .lat(a(trip1_lats[i]))
                .lon(a(trip1_lons[i]))
                .heading(a(45.0))
                .speed(v(3.0))
                .build(),
        );
        // No satellite report for track 1 points
    }

    // Custom marker within track 1 time range [t+750, t+960]
    recorder.add_annotation(
        Annotation::builder()
            .time(offset(base, 780))
            .icon(SdkIcon::Check)
            .maybe_label(Some("Checkpoint".to_owned()))
            .build()
            .expect("the marker label fits its field"),
    );

    let nav_file = recorder
        .finish()
        .expect("all builder constraints satisfied");
    let mut bytes = Vec::new();
    nav_file
        .write(&mut bytes)
        .expect("in-memory serialization succeeds");
    bytes
}

#[test]
fn generate_and_verify_snapshot_fixture() {
    let path = fixture_path();
    fs::create_dir_all(path.parent().expect("fixture path has parent"))
        .expect("can create fixtures directory");

    let bytes = build_snapshot_bytes();
    fs::write(&path, &bytes).expect("can write fixture file");

    let loaded = gt_loader::load_file(&path).expect("fixture loads without error");

    // Two tracks separated by a >5-minute gap
    assert_eq!(loaded.tracks.len(), 2, "expected 2 tracks");

    // Track 0
    let t0 = loaded.tracks.first().expect("track 0 exists");

    assert_eq!(t0.points.len(), 12, "track 0: 12 TPV points");
    assert_eq!(
        t0.metadata.satellite_report_count, 12,
        "track 0: every point has a satellite report"
    );
    assert_eq!(t0.custom_markers.len(), 2, "track 0: 2 custom markers");
    assert_eq!(
        t0.generated_markers.len(),
        2,
        "track 0: GnssFixLost + GnssFixRegained"
    );
    assert_eq!(t0.metadata.duration.num_seconds(), 330, "track 0: 5m30s");
    let t0_distance_km = t0
        .geometry
        .measured()
        .expect("track 0 has a geometry")
        .distance_km;
    assert!(
        t0_distance_km > Length::new::<kilometer>(0.5)
            && t0_distance_km < Length::new::<kilometer>(1.5),
        "track 0 distance ~0.84 km, got {t0_distance_km:?}"
    );
    assert!(t0.metadata.has_custom_markers, "track 0 has custom markers");

    let gen_kinds: Vec<_> = t0
        .generated_markers
        .iter()
        .map(|m| m.kind.clone())
        .collect();
    assert_matches_sequence!(
        gen_kinds,
        [
            GeneratedMarkerKind::GnssFixLost,
            GeneratedMarkerKind::GnssFixRegained { .. }
        ]
    );

    let mut custom = t0.custom_markers.iter();
    let bike_lock = custom.next().expect("first custom marker");
    let coffee = custom.next().expect("second custom marker");
    assert_eq!(bike_lock.label, "Bike lock spot");
    assert_eq!(coffee.label, "Coffee stop");

    // Track 1
    let t1 = loaded.tracks.get(1).expect("track 1 exists");

    assert_eq!(t1.points.len(), 8, "track 1: 8 TPV points");
    assert_eq!(
        t1.metadata.satellite_report_count, 0,
        "track 1: no satellite reports"
    );
    assert_eq!(t1.custom_markers.len(), 1, "track 1: 1 custom marker");
    assert_eq!(
        t1.generated_markers.len(),
        0,
        "track 1: no generated markers"
    );
    assert_eq!(t1.metadata.duration.num_seconds(), 210, "track 1: 3m30s");
    let t1_distance_km = t1
        .geometry
        .measured()
        .expect("track 1 has a geometry")
        .distance_km;
    assert!(
        t1_distance_km > Length::new::<kilometer>(0.3)
            && t1_distance_km < Length::new::<kilometer>(1.0),
        "track 1 distance ~0.62 km, got {t1_distance_km:?}"
    );
    assert!(t1.metadata.has_custom_markers, "track 1 has custom marker");

    let checkpoint = t1.custom_markers.first().expect("track 1 custom marker");
    assert_eq!(checkpoint.label, "Checkpoint");

    // File-level metadata
    assert_eq!(loaded.metadata.filename, "snapshot.gtd");
    let total_distance = loaded
        .metadata
        .total_distance
        .measured()
        .expect("both tracks are measured");
    assert!(
        total_distance > Length::new::<kilometer>(0.8)
            && total_distance < Length::new::<kilometer>(2.5),
        "total distance sane"
    );
    assert_eq!(
        loaded.metadata.total_duration.num_seconds(),
        330 + 210,
        "total duration is sum of both tracks"
    );
}
