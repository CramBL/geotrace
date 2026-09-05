//! Write a `.gtd` file with ad-hoc sensor channels, then read them back.
//!
//! A [`Channel`] is a named time series sampled at its own rate, correlated
//! with the nav track by timestamp - not resampled onto the fixes. It can be
//! scalar (an inclinometer angle) or a vector whose components share one
//! sample clock (an accelerometer's x/y/z axes). This example also shows
//! recognized milli-g values and a custom display-only unit, then reads the
//! file back and prints its channel metadata.

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
    Angle, Channel, ChannelUnit, DateTime, Duration, NavFile, NavFileBuilder, NavFix, NavFixTime,
    Unit, Utc,
};

fn main() -> Result<(), Box<dyn Error>> {
    let t0 = "2024-06-01T08:00:00Z".parse::<DateTime<Utc>>()?;

    let mut recorder = NavFileBuilder::new().with_title("Channel tour").open();
    recorder.add(
        NavFix::builder()
            .time(NavFixTime::Receiver(t0))
            .lat(Angle::degrees(51.5074))
            .lon(Angle::degrees(-0.1278))
            .build(),
    );

    // Three samples, one second apart. A real recorder would sample faster
    // than the fixes. The channel keeps its own clock either way.
    let times: Vec<DateTime<Utc>> = (0..3).map(|i| t0 + Duration::seconds(i)).collect();

    // A scalar channel: one value per timestamp.
    recorder.add(
        Channel::builder()
            .name("incline")
            .unit(Unit::DEG)
            .description("boom inclinometer")
            .times(times.clone())
            .values(vec![1.0, 1.5, 2.0])
            .build()?,
    );

    // A vector channel: `values` is row-major, one row of x/y/z per
    // timestamp. Declaring the unit as `mg` lets GeoTrace's query
    // language compare it against acceleration literals.
    recorder.add(
        Channel::builder()
            .name("accel")
            .unit(Unit::MG)
            .description("IMU acceleration")
            .components(["x", "y", "z"])
            .times(times.clone())
            .values(vec![
                0.0, 200.0, 980.0, //
                100.0, 200.0, 980.0, //
                200.0, 200.0, 980.0,
            ])
            .build()?,
    );

    // A custom unit is displayed verbatim and remains dimensionless in queries.
    recorder.add(
        Channel::builder()
            .name("quality")
            .unit(ChannelUnit::custom("vendor score")?)
            .times(times)
            .values(vec![80.0, 81.0, 82.0])
            .build()?,
    );

    let nav_file = recorder.finish()?;

    let path = env::temp_dir().join("geotrace_channels.gtd");
    nav_file.write_to_file(&path)?;

    let loaded = NavFile::open(&path)?;
    println!("{} channels:", loaded.channels().len());
    for channel in loaded.channels() {
        print!("  {:<10} {} samples", channel.name(), channel.times().len());
        if let Some(unit) = channel.unit() {
            print!(" [{unit}]");
        }
        if !channel.components().is_empty() {
            print!(" components: {}", channel.components().join(" "));
        }
        println!();
    }

    fs::remove_file(&path)?;
    Ok(())
}
