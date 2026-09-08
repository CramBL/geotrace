//! Fixture builders shared by the test modules of this crate: the epoch every
//! fixture is stamped from, the satellites a report lists, and the fixes that
//! carry them.

#![cfg(test)]

use chrono::{DateTime, Duration, Utc};

use gt_types::satellites::{Constellation, Satellite, Satellites};
use gt_types::{GpsTime, Latitude, Longitude, NavPoint, TimePositionVelocity};

/// Degrees clockwise from north. A satellite the receiver reported no azimuth
/// for is unplaceable on the plot, which the builders below take as `None`.
#[derive(Clone, Copy)]
pub struct Azimuth(pub f32);

/// Degrees above the horizon, `None` as for [`Azimuth`].
#[derive(Clone, Copy)]
pub struct Elevation(pub f32);

/// The signal quality [`sat`] gives every satellite it builds.
pub const FIXTURE_SNR_DB: f32 = 40.0;

/// The first instant of every fixture in this crate: 2025-05-23 12:53:20 UTC.
pub fn start() -> DateTime<Utc> {
    DateTime::<Utc>::from_timestamp(1_748_000_000, 0).unwrap_or_default()
}

/// `secs` seconds after [`start`].
pub fn at(secs: i64) -> GpsTime {
    GpsTime::from_utc(start() + Duration::seconds(secs))
}

/// One satellite of `constellation` at `azimuth` and `elevation`, reporting
/// `snr_db`.
pub fn satellite(
    constellation: Constellation,
    prn: u32,
    azimuth: Option<Azimuth>,
    elevation: Option<Elevation>,
    snr_db: Option<f32>,
    in_fix: bool,
) -> Satellite {
    Satellite::new(
        constellation,
        prn,
        elevation.map(|Elevation(degrees)| degrees),
        azimuth.map(|Azimuth(degrees)| degrees),
        snr_db,
        in_fix,
    )
}

/// [`satellite`] at [`FIXTURE_SNR_DB`], for a case whose assertions cover the
/// sky position and the fix state alone.
pub fn sat(
    constellation: Constellation,
    prn: u32,
    azimuth: Option<Azimuth>,
    elevation: Option<Elevation>,
    in_fix: bool,
) -> Satellite {
    satellite(
        constellation,
        prn,
        azimuth,
        elevation,
        Some(FIXTURE_SNR_DB),
        in_fix,
    )
}

/// A fix at [`at`] `secs`, at 55°N 12°E, reporting `satellites`. A fix
/// without a report passes `None`.
pub fn nav_point_reporting(secs: i64, satellites: Option<Vec<Satellite>>) -> NavPoint {
    let tpv = TimePositionVelocity::builder()
        .time(at(secs))
        .lat(Latitude::new(55.0))
        .lon(Longitude::new(12.0))
        .build();
    NavPoint::new(tpv, satellites.map(|s| Satellites::new(None, None, s)))
}
