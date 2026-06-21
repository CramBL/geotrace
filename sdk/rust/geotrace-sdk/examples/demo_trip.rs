//! Generate the demo-trip fixture: a short scripted ride along the Paris
//! quays with a 59 s tunnel fix-loss and a gradual reacquisition.
//!
//! Reads the CSV inputs in `tests/fixtures/demo_trip/` (same schemas as the
//! gold dataset - the loaders deliberately mirror `gold_dataset.rs`, which
//! stays untouched as the cross-SDK reference) and writes `demo_trip.gtd`
//! next to them. The 59 missing seconds carry satellite reports without
//! fixes; the builder turns those into ghost nav points, which is what the
//! app renders as a dashed fix-lost stretch.
// Examples favour brevity: the core's robustness restriction lints (no
// unwrap/expect/panic/indexing, no std::env::temp_dir) are not enforced on
// demonstration code, mirroring how clippy.toml relaxes them inside tests.
#![allow(
    clippy::restriction,
    clippy::cognitive_complexity,
    clippy::disallowed_methods,
    clippy::allow_attributes,
    reason = "SDK example: demonstration code"
)]

use geotrace_sdk::{
    Angle, Annotation, Constellation, EventMarker, EventMarkerStyle, MarkerIcon, Meta,
    NavFileBuilder, NavFix, NavRecorder, Satellite, SatelliteReport, Velocity,
};
use std::collections::HashMap;
use std::fs::File;
use std::io::{BufRead, BufReader};
use std::path::Path;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let base_dir = Path::new("tests/fixtures/demo_trip");

    if !base_dir.exists() {
        return Err(format!(
            "Base directory not found: {:?}. Are you running from the repository root?",
            base_dir
        )
        .into());
    }

    let meta = load_meta(base_dir)?;
    let mut recorder = NavFileBuilder::new()
        .with_lenient_errors()
        .with_meta(meta)
        .open();

    load_event_styles(&mut recorder, base_dir)?;
    load_satellites_and_fixes(&mut recorder, base_dir)?;
    load_markers(&mut recorder, base_dir)?;
    load_events(&mut recorder, base_dir)?;

    let nav_file = recorder.finish()?;
    nav_file.write_to_file(base_dir.join("demo_trip.gtd"))?;

    println!("Demo trip generated: tests/fixtures/demo_trip/demo_trip.gtd");
    println!("  Nav Points:    {}", nav_file.nav_points().len());
    println!("  Markers:       {}", nav_file.markers().len());
    println!("  Event Markers: {}", nav_file.event_markers().len());

    println!("Verifying the demo-trip storyline...");
    verify_demo_file(base_dir.join("demo_trip.gtd"))?;
    println!("Verified!");

    Ok(())
}

fn load_meta(base_dir: &Path) -> Result<Meta, Box<dyn std::error::Error>> {
    let meta_file = File::open(base_dir.join("meta.csv"))?;
    let reader = BufReader::new(meta_file);
    let mut lines = reader.lines().skip(1);
    if let Some(Ok(line)) = lines.next() {
        let cols: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if cols.len() >= 4 {
            return Ok(Meta::builder()
                .maybe_title(Some(cols[0]))
                .maybe_device(Some(cols[1]))
                .maybe_notes(Some(cols[2]))
                .maybe_identity(Some(cols[3]))
                .build());
        }
    }
    Ok(Meta::builder().build())
}

fn load_event_styles(
    recorder: &mut NavRecorder,
    base_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let style_file = File::open(base_dir.join("event_styles.csv"))?;
    let reader = BufReader::new(style_file);
    for line in reader.lines().skip(1) {
        let line = line?;
        let cols: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if cols.len() < 3 {
            continue;
        }
        recorder.add_event_marker_style(
            EventMarkerStyle::builder()
                .variant_path(cols[0])
                .maybe_icon(
                    MarkerIcon::try_from_lower_case(cols[1])
                        .ok()
                        .map(Into::into),
                )
                .maybe_color(Some(cols[2]))
                .build()?,
        );
    }
    Ok(())
}

fn load_satellites_and_fixes(
    recorder: &mut NavRecorder,
    base_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let mut satellite_reports: HashMap<(String, String), Vec<Satellite>> = HashMap::new();
    let sat_file = File::open(base_dir.join("satellites.csv"))?;
    let reader = BufReader::new(sat_file);
    for line in reader.lines().skip(1) {
        let line = line?;
        let cols: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if cols.len() < 8 {
            continue;
        }
        let sat = Satellite::builder()
            .constellation(Constellation::try_from_lower_case(cols[2])?)
            .prn(cols[3].parse()?)
            .in_fix(cols[4].parse()?)
            .maybe_elevation(cols[5].parse().ok())
            .maybe_azimuth(cols[6].parse().ok())
            .maybe_snr(cols[7].parse().ok())
            .build();

        satellite_reports
            .entry((cols[0].to_string(), cols[1].to_string()))
            .or_default()
            .push(sat);
    }

    let fix_file = File::open(base_dir.join("fixes.csv"))?;
    let reader = BufReader::new(fix_file);
    for line in reader.lines().skip(1) {
        let line = line?;
        let cols: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if cols.len() < 8 {
            continue;
        }
        let gps_time = parse_time(cols[1]);
        let sys_time = parse_time(cols[2]);

        recorder.add(
            NavFix::builder()
                .maybe_gps_time(gps_time)
                .maybe_sys_time(sys_time)
                .lat(Angle::try_from_degrees_str(cols[3])?)
                .lon(Angle::try_from_degrees_str(cols[4])?)
                .maybe_heading(Angle::try_from_degrees_str(cols[5]).ok())
                .maybe_speed(Velocity::try_from_kmh_str(cols[6]).ok())
                .maybe_eph_m(cols[7].parse().ok())
                .build(),
        );

        let key = (cols[1].to_string(), cols[2].to_string());
        if let Some(tracked) = satellite_reports.remove(&key) {
            recorder.add(
                SatelliteReport::builder()
                    .maybe_gps_time(gps_time)
                    .maybe_sys_time(sys_time)
                    .tracked(tracked)
                    .build(),
            );
        }
    }

    // The remaining reports are the tunnel seconds with no position fix:
    // the builder turns them into ghost nav points.
    for ((gt_str, st_str), tracked) in satellite_reports {
        recorder.add(
            SatelliteReport::builder()
                .maybe_gps_time(parse_time(&gt_str))
                .maybe_sys_time(parse_time(&st_str))
                .tracked(tracked)
                .build(),
        );
    }

    Ok(())
}

fn load_markers(
    recorder: &mut NavRecorder,
    base_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let marker_file = File::open(base_dir.join("markers.csv"))?;
    let reader = BufReader::new(marker_file);
    for line in reader.lines().skip(1) {
        let line = line?;
        let cols: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if cols.len() < 3 {
            continue;
        }
        recorder.add(
            Annotation::builder()
                .time(parse_time(cols[0]).expect("Marker time required"))
                .maybe_label(Some(cols[1]))
                .maybe_icon(MarkerIcon::try_from_lower_case(cols[2]).ok())
                .build(),
        );
    }
    Ok(())
}

fn load_events(
    recorder: &mut NavRecorder,
    base_dir: &Path,
) -> Result<(), Box<dyn std::error::Error>> {
    let event_file = File::open(base_dir.join("events.csv"))?;
    let reader = BufReader::new(event_file);
    for line in reader.lines().skip(1) {
        let line = line?;
        let cols: Vec<&str> = line.split(',').map(|s| s.trim()).collect();
        if cols.len() < 3 {
            continue;
        }
        recorder.add(
            EventMarker::builder()
                .variant_path(cols[1])
                .sys_time(parse_time(cols[0]).expect("Event sys_time required"))
                .maybe_annotation(if cols[2].is_empty() {
                    None
                } else {
                    Some(cols[2].to_string())
                })
                .build()?,
        );
    }
    Ok(())
}

/// Assert the storyline facts the demo trip exists to provide: the 60 s
/// fix gap covered second-by-second with satellite reports, the collapse
/// before the tunnel, and the gradual reacquisition after it.
fn verify_demo_file(path: impl AsRef<Path>) -> Result<(), Box<dyn std::error::Error>> {
    let file = geotrace_sdk::NavFile::open(path)?;

    assert_eq!(file.meta().identity.as_deref(), Some("demo-trip-v1"));
    assert_eq!(file.markers().len(), 3);
    assert_eq!(file.event_markers().len(), 4);
    assert_eq!(file.event_marker_styles().len(), 4);

    // 92 real fixes plus 59 ghost points (one per missing tunnel second).
    // Ghost points are the fixes the builder synthesizes for orphaned
    // satellite reports: interpolated position and heading, but no
    // measured accuracy - the missing eph is what tells them apart.
    let points = file.nav_points();
    assert_eq!(points.len(), 151);
    let is_ghost = |p: &&geotrace_sdk::NavPoint| p.fix.eph_m.is_none();
    let ghost_points = points.iter().filter(is_ghost).count();
    assert_eq!(ghost_points, 59, "one ghost nav point per tunnel second");

    // Every point of the trip carries a satellite report, gap included.
    assert!(points.iter().all(|p| p.satellites.is_some()));

    // The tunnel seconds never contain a satellite contributing to a fix.
    for p in points.iter().filter(is_ghost) {
        let sats = p.satellites.as_ref().unwrap();
        assert!(sats.tracked.iter().all(|s| !s.in_fix));
    }

    // Recovery: the first restored fix has the minimal 4-satellite
    // geometry and a large eph that converges by the end of the trip.
    let recovery: Vec<_> = points.iter().filter(|p| !is_ghost(p)).skip(80).collect();
    assert_eq!(recovery.len(), 12);
    let first = recovery.first().unwrap();
    let last = recovery.last().unwrap();
    let in_fix = |p: &&geotrace_sdk::NavPoint| {
        p.satellites
            .as_ref()
            .map_or(0, |s| s.tracked.iter().filter(|t| t.in_fix).count())
    };
    assert_eq!(in_fix(first), 4);
    assert!(in_fix(last) >= 10);
    assert!(first.fix.eph_m.unwrap() > 20.0);
    assert!(last.fix.eph_m.unwrap() < 5.0);

    Ok(())
}

fn parse_time(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    geotrace_sdk::Timestamp::try_from_iso8601(s)
        .ok()
        .map(Into::into)
}
