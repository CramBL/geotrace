//! Write a basic `.gtd` file from a hardcoded GPS track.
//!
//! The minimal write workflow: open a [`NavRecorder`] with some metadata, add a
//! few [`NavFix`] points (plus an optional satellite report and a map
//! annotation), call [`NavRecorder::finish`], and write the result to disk.

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
    Angle, Annotation, Constellation, DateTime, MarkerIcon, Meta, NavFileBuilder, NavFix,
    NavFixTime, Satellite, SatelliteReport, Utc, Velocity,
};

fn main() -> Result<(), Box<dyn Error>> {
    let t = |s: &str| s.parse::<DateTime<Utc>>().expect("valid timestamp");

    let meta = Meta::builder()
        .title("Quick tour")
        .device("Example GPS v1.0")
        .build();
    let mut recorder = NavFileBuilder::new().with_meta(meta).open();

    recorder.add(
        NavFix::builder()
            .time(NavFixTime::Receiver(t("2024-06-01T08:00:00Z")))
            .lat(Angle::degrees(51.5074))
            .lon(Angle::degrees(-0.1278))
            .heading(Angle::degrees(90.0))
            .speed(Velocity::meter_per_second(5.5))
            .eph_m(3.2)
            .build(),
    );

    recorder.add(
        SatelliteReport::builder()
            .time(NavFixTime::Receiver(t("2024-06-01T08:00:00Z")))
            .tracked(vec![
                Satellite::builder()
                    .constellation(Constellation::Gps)
                    .prn(1)
                    .in_fix(true)
                    .elevation(45.0)
                    .azimuth(90.0)
                    .snr(38.0)
                    .build(),
                Satellite::builder()
                    .constellation(Constellation::Galileo)
                    .prn(3)
                    .snr(22.0)
                    .build(),
            ])
            .build(),
    );

    recorder.add(
        NavFix::builder()
            .time(NavFixTime::Receiver(t("2024-06-01T08:00:10Z")))
            .lat(Angle::degrees(51.5080))
            .lon(Angle::degrees(-0.1265))
            .heading(Angle::degrees(85.0))
            .speed(Velocity::meter_per_second(5.8))
            .build(),
    );

    recorder.add(
        Annotation::builder()
            .time(t("2024-06-01T08:00:00Z"))
            .label("Start point")
            .icon(MarkerIcon::Pin)
            .build()?,
    );

    let nav_file = recorder.finish()?;

    let path = env::temp_dir().join("geotrace_write_basic.gtd");
    nav_file.write_to_file(&path)?;
    println!(
        "Wrote {} nav points to {}",
        nav_file.nav_points().len(),
        path.display()
    );

    fs::remove_file(&path)?;
    Ok(())
}
