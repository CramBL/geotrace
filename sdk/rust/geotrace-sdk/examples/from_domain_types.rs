//! Feed custom domain types into the SDK using `From` trait implementations.
//!
//! **Scenario**: your application already owns its GPS data structs - produced by
//! a hardware driver, a third-party library, or your own domain model.
//! Implement `From` for each SDK type once.
//! After that, [`NavRecorder::add_nav_fix`] and [`NavRecorder::add_satellite_report`]
//! accept your types directly. [`Annotation`] takes `TryFrom` instead: a label
//! past the capacity of `markers/label` is rejected.

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

use std::{env, error::Error, fs};

use geotrace_sdk::{
    Angle, Annotation, Constellation, MarkerIcon, NavFileBuilder, NavFix, NavFixTime, Satellite,
    SatelliteReport, Timestamp, Velocity,
};

// Your existing domain types - not SDK types.
struct GpsFix {
    unix_ms: u64,
    lat_deg: f64,
    lon_deg: f64,
    heading_deg: f64,
    speed_mps: f64,
}

struct SatReport {
    unix_ms: u64,
    satellites: Vec<SatView>,
}

struct SatView {
    constellation: GnssConst,
    prn: u32,
    elevation_deg: f32,
    azimuth_deg: f32,
    snr_db_hz: f32,
    in_fix: bool,
}

#[derive(Clone, Copy)]
enum GnssConst {
    Gps,
    Glonass,
    Galileo,
    Beidou,
}

struct LogEntry {
    unix_ms: u64,
    text: String,
}

// One-time From implementations - written once, called nowhere explicitly.
impl From<GpsFix> for NavFix {
    fn from(f: GpsFix) -> Self {
        NavFix::builder()
            .time(NavFixTime::Receiver(
                Timestamp::from_unix_millis(f.unix_ms).into(),
            ))
            .lat(Angle::degrees(f.lat_deg))
            .lon(Angle::degrees(f.lon_deg))
            .heading(Angle::degrees(f.heading_deg))
            .speed(Velocity::meter_per_second(f.speed_mps))
            .build()
    }
}

impl From<GnssConst> for Constellation {
    fn from(c: GnssConst) -> Self {
        match c {
            GnssConst::Gps => Constellation::Gps,
            GnssConst::Glonass => Constellation::Glonass,
            GnssConst::Galileo => Constellation::Galileo,
            GnssConst::Beidou => Constellation::Beidou,
        }
    }
}

impl From<SatReport> for SatelliteReport {
    fn from(r: SatReport) -> Self {
        let tracked = r
            .satellites
            .into_iter()
            .map(|s| {
                Satellite::builder()
                    .constellation(s.constellation) // GnssConst: Into<Constellation>
                    .prn(s.prn)
                    .in_fix(s.in_fix)
                    .elevation(s.elevation_deg)
                    .azimuth(s.azimuth_deg)
                    .snr(s.snr_db_hz)
                    .build()
            })
            .collect();
        SatelliteReport::builder()
            .gps_time(Timestamp::from_unix_millis(r.unix_ms))
            .tracked(tracked)
            .build()
    }
}

impl TryFrom<LogEntry> for Annotation {
    type Error = geotrace_sdk::Error;

    fn try_from(e: LogEntry) -> Result<Self, Self::Error> {
        Annotation::builder()
            .time(Timestamp::from_unix_millis(e.unix_ms))
            .label(e.text)
            .icon(MarkerIcon::Pin)
            .build()
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    // 2024-06-15 08:00:00 UTC - 6 fixes at 30 s intervals, Copenhagen area.
    const BASE_MS: u64 = 1_718_438_400_000;

    let fixes = vec![
        GpsFix {
            unix_ms: BASE_MS,
            lat_deg: 55.6760,
            lon_deg: 12.5683,
            heading_deg: 45.0,
            speed_mps: 2.6,
        },
        GpsFix {
            unix_ms: BASE_MS + 30_000,
            lat_deg: 55.6772,
            lon_deg: 12.5695,
            heading_deg: 45.0,
            speed_mps: 2.7,
        },
        GpsFix {
            unix_ms: BASE_MS + 60_000,
            lat_deg: 55.6784,
            lon_deg: 12.5707,
            heading_deg: 45.0,
            speed_mps: 2.6,
        },
        GpsFix {
            unix_ms: BASE_MS + 90_000,
            lat_deg: 55.6796,
            lon_deg: 12.5719,
            heading_deg: 45.0,
            speed_mps: 2.5,
        },
        GpsFix {
            unix_ms: BASE_MS + 120_000,
            lat_deg: 55.6808,
            lon_deg: 12.5731,
            heading_deg: 45.0,
            speed_mps: 2.7,
        },
        GpsFix {
            unix_ms: BASE_MS + 150_000,
            lat_deg: 55.6820,
            lon_deg: 12.5743,
            heading_deg: 45.0,
            speed_mps: 2.6,
        },
    ];

    let sat_reports = vec![
        SatReport {
            unix_ms: BASE_MS,
            satellites: vec![
                SatView {
                    constellation: GnssConst::Gps,
                    prn: 1,
                    elevation_deg: 45.0,
                    azimuth_deg: 120.0,
                    snr_db_hz: 40.0,
                    in_fix: true,
                },
                SatView {
                    constellation: GnssConst::Gps,
                    prn: 7,
                    elevation_deg: 30.0,
                    azimuth_deg: 200.0,
                    snr_db_hz: 36.0,
                    in_fix: true,
                },
                SatView {
                    constellation: GnssConst::Galileo,
                    prn: 3,
                    elevation_deg: 55.0,
                    azimuth_deg: 80.0,
                    snr_db_hz: 38.0,
                    in_fix: true,
                },
                SatView {
                    constellation: GnssConst::Glonass,
                    prn: 12,
                    elevation_deg: 20.0,
                    azimuth_deg: 310.0,
                    snr_db_hz: 32.0,
                    in_fix: false,
                },
            ],
        },
        SatReport {
            unix_ms: BASE_MS + 60_000,
            satellites: vec![
                SatView {
                    constellation: GnssConst::Gps,
                    prn: 1,
                    elevation_deg: 46.0,
                    azimuth_deg: 122.0,
                    snr_db_hz: 41.0,
                    in_fix: true,
                },
                SatView {
                    constellation: GnssConst::Gps,
                    prn: 7,
                    elevation_deg: 31.0,
                    azimuth_deg: 202.0,
                    snr_db_hz: 37.0,
                    in_fix: true,
                },
                SatView {
                    constellation: GnssConst::Galileo,
                    prn: 3,
                    elevation_deg: 54.0,
                    azimuth_deg: 81.0,
                    snr_db_hz: 39.0,
                    in_fix: true,
                },
                SatView {
                    constellation: GnssConst::Beidou,
                    prn: 5,
                    elevation_deg: 35.0,
                    azimuth_deg: 150.0,
                    snr_db_hz: 35.0,
                    in_fix: false,
                },
            ],
        },
    ];

    let log_entries = vec![
        LogEntry {
            unix_ms: BASE_MS + 45_000,
            text: "Entered cycle lane".to_owned(),
        },
        LogEntry {
            unix_ms: BASE_MS + 105_000,
            text: "Speed bump".to_owned(),
        },
    ];

    let mut recorder = NavFileBuilder::new().open();

    // The typed add_* methods accept `impl Into<…>`, so the domain types feed in
    // directly via their From impls. An annotation converts through TryFrom,
    // which rejects a label past the capacity of `markers/label`.
    for fix in fixes {
        recorder.add_nav_fix(fix);
    }
    for report in sat_reports {
        recorder.add_satellite_report(report);
    }
    for entry in log_entries {
        recorder.add_annotation(Annotation::try_from(entry)?);
    }

    let nav_file = recorder.finish()?;

    let out = env::temp_dir().join("geotrace_from_domain_types.gtd");
    nav_file.write_to_file(&out)?;
    println!(
        "{} fixes, {} satellite reports, {} annotations - written to {out:?}",
        nav_file.nav_points().len(),
        nav_file
            .nav_points()
            .iter()
            .filter(|p| p.satellites.is_some())
            .count(),
        nav_file.markers().len(),
    );
    fs::remove_file(&out)?;

    Ok(())
}
