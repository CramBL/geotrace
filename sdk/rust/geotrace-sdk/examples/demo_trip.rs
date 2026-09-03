//! Generate the demo-trip fixture: a short scripted ride along the Paris
//! quays with a 59 s tunnel fix-loss and a gradual reacquisition.
//!
//! Reads the CSV inputs in `tests/fixtures/demo_trip/` (same schemas as the
//! gold dataset - the loaders deliberately mirror `gold_dataset.rs`, which
//! stays untouched as the cross-SDK reference) and writes `demo_trip.gtd`
//! next to them. The 59 missing seconds carry satellite reports without
//! fixes. The builder turns those into ghost nav points, which is what the
//! app renders as a dashed fix-lost stretch.
//!
//! The trip also carries a 25 Hz `accel` channel derived from its own speed
//! and heading dynamics (see [`synthesize_accel`]) - and unlike the fixes,
//! the IMU samples run right through the tunnel.
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
    Angle, Annotation, Channel, Constellation, EventMarker, EventMarkerStyle, MarkerIcon, Meta,
    NavFileBuilder, NavFix, NavRecorder, Satellite, SatelliteReport, Unit, Velocity,
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
        .with_scrubbed_provenance()
        .open();

    load_event_styles(&mut recorder, base_dir)?;
    let fixes = load_satellites_and_fixes(&mut recorder, base_dir)?;
    recorder.add_channel(synthesize_accel(&fixes)?);
    load_markers(&mut recorder, base_dir)?;
    load_events(&mut recorder, base_dir)?;

    let nav_file = recorder.finish()?;
    nav_file.write_to_file(base_dir.join("demo_trip.gtd"))?;

    println!("Demo trip generated: tests/fixtures/demo_trip/demo_trip.gtd");
    println!("  Nav Points:    {}", nav_file.nav_points().len());
    println!("  Markers:       {}", nav_file.markers().len());
    println!("  Event Markers: {}", nav_file.event_markers().len());
    println!("  Channels:      {}", nav_file.channels().len());

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

/// One fix's dynamics, kept for the accel synthesis.
struct FixDynamics {
    time: chrono::DateTime<chrono::Utc>,
    speed: Velocity,
    heading: Angle,
}

/// Loads the satellite reports and fixes into the recorder, returning each
/// fix's dynamics in trip order for [`synthesize_accel`].
fn load_satellites_and_fixes(
    recorder: &mut NavRecorder,
    base_dir: &Path,
) -> Result<Vec<FixDynamics>, Box<dyn std::error::Error>> {
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

    let mut dynamics = Vec::new();
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
        if let (Some(time), Ok(heading), Ok(speed)) = (
            gps_time,
            Angle::try_from_degrees_str(cols[5]),
            Velocity::try_from_kmh_str(cols[6]),
        ) {
            dynamics.push(FixDynamics {
                time,
                speed,
                heading,
            });
        }

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

    Ok(dynamics)
}

/// Samples per second of the synthesized IMU channel.
const ACCEL_RATE_HZ: i64 = 25;
/// Wall-clock length of the scripted trip, first fix to last.
const TRIP_DURATION_S: i64 = 150;
/// Standard gravity, m/s².
const STANDARD_GRAVITY: f64 = 9.806_65;
/// Vibration amplitude in g at [`VIBRATION_REFERENCE_SPEED_MPS`]. Scales
/// linearly with speed - a parked IMU is quiet.
const VIBRATION_AT_REFERENCE_SPEED_G: f64 = 0.03;
/// Speed the vibration amplitude is calibrated at, m/s.
const VIBRATION_REFERENCE_SPEED_MPS: f64 = 8.0;
/// The vertical axis shakes less than the ride plane.
const VERTICAL_VIBRATION_SCALE: f64 = 0.5;

/// Derive a device-frame `accel` channel from the trip's own dynamics, at
/// [`ACCEL_RATE_HZ`] across the whole trip - tunnel included, an IMU does
/// not lose the sky. That stretch is one long quiet segment between its
/// bracketing fixes. The fixes are smoothed first: the scripted
/// reacquisition carries jumpy GPS speeds (that noise is its storyline),
/// and an IMU measures the true motion, not the fix error.
fn synthesize_accel(fixes: &[FixDynamics]) -> Result<Channel, Box<dyn std::error::Error>> {
    let fixes = &smooth_dynamics(fixes);
    let (first, last) = match (fixes.first(), fixes.last()) {
        (Some(first), Some(last)) if fixes.len() >= 2 => (first, last),
        _ => return Err("accel synthesis needs at least two fixes".into()),
    };
    let step_ms = 1_000 / ACCEL_RATE_HZ;
    // Fixes arrive in trip order, so the span is non-negative.
    let count = usize::try_from((last.time - first.time).num_milliseconds() / step_ms + 1)?;

    let mut noise = NoiseGen::new();
    let mut times = Vec::with_capacity(count);
    let mut values = Vec::with_capacity(count * 3);
    let mut seg = 0;
    for k in 0..count {
        let time = first.time + geotrace_sdk::Duration::milliseconds(k as i64 * step_ms);
        while seg + 2 < fixes.len() && fixes[seg + 1].time <= time {
            seg += 1;
        }
        let (a, b) = (&fixes[seg], &fixes[seg + 1]);
        let (v_a, v_b) = (
            a.speed.as_meters_per_second(),
            b.speed.as_meters_per_second(),
        );
        let dt = (b.time - a.time).num_milliseconds() as f64 / 1_000.0;
        let into = (time - a.time).num_milliseconds() as f64 / 1_000.0;
        let frac = (into / dt).clamp(0.0, 1.0);
        let speed = v_a + (v_b - v_a) * frac;
        let longitudinal = (v_b - v_a) / dt;
        let yaw_rate = a.heading.signed_arc_to(b.heading).as_radians() / dt;
        let vibration = VIBRATION_AT_REFERENCE_SPEED_G * (speed / VIBRATION_REFERENCE_SPEED_MPS);
        times.push(time);
        values.push(longitudinal / STANDARD_GRAVITY + vibration * noise.next());
        values.push(speed * yaw_rate / STANDARD_GRAVITY + vibration * noise.next());
        values.push(1.0 + VERTICAL_VIBRATION_SCALE * vibration * noise.next());
    }
    Ok(Channel::builder()
        .name("accel")
        .unit(Unit::G)
        .description("Device-frame IMU acceleration derived from the trip's speed and heading")
        .components(["x", "y", "z"])
        .times(times)
        .values(values)
        .build()?)
}

/// Seconds of fix dynamics averaged on each side when smoothing.
const SMOOTH_HALF_WINDOW_S: i64 = 2;

/// Average each fix's speed and heading with its neighbours within
/// [`SMOOTH_HALF_WINDOW_S`]. Headings are unwrapped into a continuous series
/// first so averaging across the 360° wrap boundary cannot fold a bend into a
/// spin.
fn smooth_dynamics(fixes: &[FixDynamics]) -> Vec<FixDynamics> {
    let mut unwrapped: Vec<f64> = Vec::with_capacity(fixes.len());
    for fix in fixes {
        let prev = unwrapped
            .last()
            .copied()
            .unwrap_or(fix.heading.as_degrees());
        unwrapped.push(prev + Angle::degrees(prev).signed_arc_to(fix.heading).as_degrees());
    }
    fixes
        .iter()
        .map(|fix| {
            let near =
                |j: &usize| (fixes[*j].time - fix.time).num_seconds().abs() <= SMOOTH_HALF_WINDOW_S;
            let window: Vec<usize> = (0..fixes.len()).filter(near).collect();
            let n = window.len() as f64;
            let speed_mps = window
                .iter()
                .map(|&j| fixes[j].speed.as_meters_per_second())
                .sum::<f64>()
                / n;
            FixDynamics {
                time: fix.time,
                speed: Velocity::meter_per_second(speed_mps),
                heading: Angle::degrees(window.iter().map(|&j| unwrapped[j]).sum::<f64>() / n),
            }
        })
        .collect()
}

/// Fixed LCG seed, multiplier, and increment (Knuth's MMIX parameters), so
/// every regeneration produces the same noise.
const LCG_SEED: u64 = 0x9E37_79B9_7F4A_7C15;
const LCG_MULTIPLIER: u64 = 6_364_136_223_846_793_005;
const LCG_INCREMENT: u64 = 1_442_695_040_888_963_407;
/// One-pole low-pass coefficient turning the LCG's white noise into
/// vibration, not hiss.
const NOISE_SMOOTHING: f64 = 0.35;

/// Smoothed noise in roughly [-1, 1] from a fixed-seed LCG: integer and
/// basic float arithmetic only, so the generated bytes reproduce on every
/// platform (libm's transcendentals may differ in the last ulp).
struct NoiseGen {
    state: u64,
    smoothed: f64,
}

impl NoiseGen {
    fn new() -> Self {
        Self {
            state: LCG_SEED,
            smoothed: 0.0,
        }
    }

    fn next(&mut self) -> f64 {
        self.state = self
            .state
            .wrapping_mul(LCG_MULTIPLIER)
            .wrapping_add(LCG_INCREMENT);
        // The top 31 bits, mapped onto [-1, 1).
        let raw = (self.state >> 33) as f64 / (1_u64 << 30) as f64 - 1.0;
        self.smoothed += NOISE_SMOOTHING * (raw - self.smoothed);
        self.smoothed
    }
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
                .build()?,
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
    // measured accuracy - the missing eph is what distinguishes them.
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

    // The derived IMU channel: [`ACCEL_RATE_HZ`] across the whole trip with
    // no gap - the samples run right through the tunnel - and gravity on z.
    let channels = file.channels();
    assert_eq!(channels.len(), 1);
    let accel = &channels[0];
    assert_eq!(accel.name(), "accel");
    assert_eq!(accel.unit().map(ToString::to_string).as_deref(), Some("g"));
    assert_eq!(accel.components(), ["x", "y", "z"]);
    let times = accel.times();
    assert_eq!(times.len() as i64, TRIP_DURATION_S * ACCEL_RATE_HZ + 1);
    assert!(
        times
            .windows(2)
            .all(|w| (w[1] - w[0]).num_milliseconds() == 1_000 / ACCEL_RATE_HZ)
    );
    let z_mean = accel.values().chunks(3).map(|row| row[2]).sum::<f64>() / times.len() as f64;
    assert!((z_mean - 1.0).abs() < 0.05, "z mean {z_mean} sits near 1 g");
    // The ride turns: some samples carry a distinctly nonzero lateral g.
    let y_peak = accel
        .values()
        .chunks(3)
        .map(|row| row[1].abs())
        .fold(0.0_f64, f64::max);
    assert!(y_peak > 0.05, "lateral peak {y_peak} shows the bends");

    Ok(())
}

fn parse_time(s: &str) -> Option<chrono::DateTime<chrono::Utc>> {
    geotrace_sdk::Timestamp::try_from_iso8601(s)
        .ok()
        .map(Into::into)
}
