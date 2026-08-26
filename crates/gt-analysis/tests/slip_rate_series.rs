//! The trailing-window slip rate over tracks whose fixes arrive faster than
//! once a second.

use chrono::DateTime;
use gt_analysis::loss_of_lock;
use gt_types::coordinates::{Latitude, Longitude};
use gt_types::nav_point::NavPoint;
use gt_types::satellites::{Constellation, Satellite, Satellites};
use gt_types::time_types::GpsTime;
use gt_types::tpv::TimePositionVelocity;

/// Elevation mask, in degrees.
const MASK_DEG: f32 = 15.0;

/// SNR fall that counts as a slip, in dB-Hz.
const SNR_DROP_DB: f32 = 10.0;

/// Trailing window of the rate, in minutes.
const WINDOW_MIN: f32 = 1.0;

/// A recent epoch to hang the synthetic fixes off, in Unix milliseconds.
const BASE_MILLIS: i64 = 1_700_000_000_000;

fn fix_at(millis: i64, satellites: Vec<Satellite>) -> NavPoint {
    let time = GpsTime::from_utc(
        DateTime::from_timestamp_millis(millis).expect("timestamp within chrono's range"),
    );
    let tpv = TimePositionVelocity::builder()
        .time(time)
        .lat(Latitude::new(55.0))
        .lon(Longitude::new(12.0))
        .build();
    NavPoint::new(tpv, Some(Satellites::new(Some(time), None, satellites)))
}

fn two_satellites_in_view() -> Vec<Satellite> {
    vec![
        Satellite::new(Constellation::Gps, 1, Some(40.0), None, Some(45.0), true),
        Satellite::new(Constellation::Gps, 2, Some(30.0), None, Some(40.0), true),
    ]
}

/// The same report after PRN 2 dropped out: one lost-lock slip.
fn one_satellite_in_view() -> Vec<Satellite> {
    vec![Satellite::new(
        Constellation::Gps,
        1,
        Some(40.0),
        None,
        Some(45.0),
        true,
    )]
}

/// No slip can be attributed to a track's first epoch: there is no earlier
/// report to have slipped from, so its trailing window counts nothing. The
/// slip belongs to the second fix, 500 ms later.
#[test]
fn slip_rate_at_the_first_epoch_of_a_sub_second_track_is_zero() {
    let points = vec![
        fix_at(BASE_MILLIS, two_satellites_in_view()),
        fix_at(BASE_MILLIS + 500, one_satellite_in_view()),
    ];

    let series = loss_of_lock::slip_rate_series(&points, MASK_DEG, SNR_DROP_DB, WINDOW_MIN);

    assert_eq!(series.all.first().map(|point| point[1]), Some(0.0));
    assert_eq!(series.all.get(1).map(|point| point[1]), Some(1.0));
}
