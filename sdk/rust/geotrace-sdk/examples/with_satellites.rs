//! Write a `.gtd` file that pairs each GPS fix with a satellite report.
//!
//! A [`SatelliteReport`] is a snapshot of every tracked satellite at one
//! instant: its constellation, PRN, whether it contributed to the fix, and
//! signal quality. Reports are matched to the nearest fix, so giving each
//! report the same timestamp as its fix keeps them aligned.
//!
//! The example writes the file, reads it back, and prints the per-fix satellite
//! counts - the data GeoTrace shows in its sky view.

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
    Angle, Constellation, DateTime, Duration, NavFile, NavFileBuilder, NavFix, NavFixTime,
    Satellite, SatelliteReport, Utc, Velocity,
};

struct TrackPoint {
    offset_secs: i64,
    lat_deg: f64,
    lon_deg: f64,
    heading_deg: f64,
    speed_mps: f64,
    eph_m: f64,
}

struct SkySatellite {
    constellation: Constellation,
    prn: u32,
    in_fix: bool,
    elevation_deg: Option<f32>,
    azimuth_deg: Option<f32>,
    snr_dbhz: f32,
}

/// A short urban loop through Southwark, London, one fix every 10 s.
const TRACK: &[TrackPoint] = &[
    TrackPoint {
        offset_secs: 0,
        lat_deg: 51.5030,
        lon_deg: -0.0978,
        heading_deg: 5.0,
        speed_mps: 0.0,
        eph_m: 4.2,
    },
    TrackPoint {
        offset_secs: 10,
        lat_deg: 51.5038,
        lon_deg: -0.0975,
        heading_deg: 8.0,
        speed_mps: 3.1,
        eph_m: 3.8,
    },
    TrackPoint {
        offset_secs: 20,
        lat_deg: 51.5045,
        lon_deg: -0.0971,
        heading_deg: 12.0,
        speed_mps: 4.4,
        eph_m: 3.5,
    },
    TrackPoint {
        offset_secs: 30,
        lat_deg: 51.5053,
        lon_deg: -0.0966,
        heading_deg: 10.0,
        speed_mps: 4.6,
        eph_m: 3.1,
    },
    TrackPoint {
        offset_secs: 40,
        lat_deg: 51.5060,
        lon_deg: -0.0961,
        heading_deg: 7.0,
        speed_mps: 4.4,
        eph_m: 2.9,
    },
    TrackPoint {
        offset_secs: 50,
        lat_deg: 51.5067,
        lon_deg: -0.0957,
        heading_deg: 5.0,
        speed_mps: 3.8,
        eph_m: 3.0,
    },
];

/// A mixed GPS, Galileo and GLONASS sky: eight satellites, five in the fix.
/// GLONASS 5 has an SNR and no elevation or azimuth. A receiver reports that
/// for a satellite whose position it has not computed.
const SKY: &[SkySatellite] = &[
    SkySatellite {
        constellation: Constellation::Gps,
        prn: 3,
        in_fix: true,
        elevation_deg: Some(72.0),
        azimuth_deg: Some(145.0),
        snr_dbhz: 44.0,
    },
    SkySatellite {
        constellation: Constellation::Gps,
        prn: 8,
        in_fix: true,
        elevation_deg: Some(58.0),
        azimuth_deg: Some(230.0),
        snr_dbhz: 41.0,
    },
    SkySatellite {
        constellation: Constellation::Gps,
        prn: 14,
        in_fix: true,
        elevation_deg: Some(41.0),
        azimuth_deg: Some(60.0),
        snr_dbhz: 37.0,
    },
    SkySatellite {
        constellation: Constellation::Gps,
        prn: 22,
        in_fix: false,
        elevation_deg: Some(18.0),
        azimuth_deg: Some(310.0),
        snr_dbhz: 28.0,
    },
    SkySatellite {
        constellation: Constellation::Galileo,
        prn: 7,
        in_fix: true,
        elevation_deg: Some(65.0),
        azimuth_deg: Some(195.0),
        snr_dbhz: 42.0,
    },
    SkySatellite {
        constellation: Constellation::Galileo,
        prn: 12,
        in_fix: true,
        elevation_deg: Some(33.0),
        azimuth_deg: Some(90.0),
        snr_dbhz: 35.0,
    },
    SkySatellite {
        constellation: Constellation::Galileo,
        prn: 19,
        in_fix: false,
        elevation_deg: Some(12.0),
        azimuth_deg: Some(15.0),
        snr_dbhz: 22.0,
    },
    SkySatellite {
        constellation: Constellation::Glonass,
        prn: 5,
        in_fix: false,
        elevation_deg: None,
        azimuth_deg: None,
        snr_dbhz: 31.0,
    },
];

fn main() -> Result<(), Box<dyn Error>> {
    let start = "2024-06-01T08:00:00Z".parse::<DateTime<Utc>>()?;

    let mut recorder = NavFileBuilder::new()
        .with_title("Satellite quality tour")
        .with_device("Example GNSS v1.0")
        .open();

    for (i, point) in TRACK.iter().enumerate() {
        let time = start + Duration::seconds(point.offset_secs);
        recorder.add(
            NavFix::builder()
                .time(NavFixTime::Receiver(time))
                .lat(Angle::degrees(point.lat_deg))
                .lon(Angle::degrees(point.lon_deg))
                .heading(Angle::degrees(point.heading_deg))
                .speed(Velocity::meter_per_second(point.speed_mps))
                .eph_m(point.eph_m)
                .build(),
        );

        // SNR climbs slightly along the track as the receiver settles.
        let snr_gain = 0.5 * i as f32;
        let tracked = SKY
            .iter()
            .map(|satellite| {
                Satellite::builder()
                    .constellation(satellite.constellation)
                    .prn(satellite.prn)
                    .in_fix(satellite.in_fix)
                    .maybe_elevation(satellite.elevation_deg)
                    .maybe_azimuth(satellite.azimuth_deg)
                    .snr(satellite.snr_dbhz + snr_gain)
                    .build()
            })
            .collect();
        recorder.add(
            SatelliteReport::builder()
                .time(NavFixTime::Receiver(time))
                .tracked(tracked)
                .build(),
        );
    }

    let nav_file = recorder.finish()?;

    let path = env::temp_dir().join("geotrace_with_satellites.gtd");
    nav_file.write_to_file(&path)?;

    let loaded = NavFile::open(&path)?;
    println!("Nav points: {}", loaded.nav_points().len());
    for (i, point) in loaded.nav_points().iter().enumerate() {
        let (tracked, in_fix) = match &point.satellites {
            Some(report) => (
                report.tracked.len(),
                report.tracked.iter().filter(|s| s.in_fix).count(),
            ),
            None => (0, 0),
        };
        println!("  [{i}] {tracked} tracked, {in_fix} in fix");
    }

    fs::remove_file(&path)?;
    Ok(())
}
