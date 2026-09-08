//! Fixture builders shared by the test modules of this crate: a satellite
//! report, and a fix with one on it.

#![cfg(test)]

use chrono::{DateTime, Duration, Utc};

use gt_types::coordinates::{Latitude, Longitude};
use gt_types::fixtures;
use gt_types::nav_point::NavPoint;
use gt_types::satellites::{Satellite, Satellites};
use gt_types::time_types::GpsTime;

/// A report listing `satellites`, without a timestamp of its own: every rule
/// that reads one reads the satellites alone.
pub fn report(satellites: Vec<Satellite>) -> Satellites {
    Satellites::new(None, None, satellites)
}

/// A fix `millis` past the Unix epoch at 55°N 12°E, reporting `satellites`
/// under a report stamped at the same instant.
pub fn point_at(millis: i64, satellites: Vec<Satellite>) -> NavPoint {
    let time = DateTime::<Utc>::UNIX_EPOCH + Duration::milliseconds(millis);
    fixtures::nav_point_with_report(
        time,
        Latitude::new(55.0),
        Longitude::new(12.0),
        Satellites::new(Some(GpsTime::from_utc(time)), None, satellites),
    )
}
