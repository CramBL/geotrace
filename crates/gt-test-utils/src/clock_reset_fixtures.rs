//! A recording from a tracker whose real-time clock restarts at every boot,
//! built to the shape a field recording of one was found to have.
//!
//! [`recording_whose_clock_restarts_at_every_boot`] writes the two clock epochs
//! that shape produces:
//!
//! - one track of [`COLD_BOOT_FIX_COUNT`] fixes stamped in the RTC epoch, from
//!   before the receiver ever corrected the clock, with no host timestamp.
//! - [`BOOT_COUNT`] tracks of [`FIXES_PER_BOOT`] fixes stamped in the true
//!   epoch, the first [`FIXES_BEFORE_THE_CLOCK_IS_CORRECTED`] of each carrying
//!   a host timestamp still at the RTC default, the rest a host timestamp
//!   [`HOST_AHEAD_MS`] ahead of the receiver's.
//! - an `accel` channel sampled at [`CHANNEL_RATE_HZ`], every sample stamped in
//!   the RTC epoch, in [`CHANNEL_RUN_LENGTHS`] runs that each restart at
//!   [`CHANNEL_RUN_START_SECS`] after the RTC default.
//! - event markers in both epochs.
//!
//! The route, the names and the timestamps are invented.

use chrono::{DateTime, Duration, Utc};
use geotrace_sdk as sdk;

/// Where the tracker's clock stands at boot, before the receiver corrects it.
const RTC_DEFAULT: &str = "2025-01-01T00:00:00Z";

/// The first true-epoch timestamp: where the receiver puts the clock once it
/// has a lock, 530 days past the RTC default.
const CORRECTED_CLOCK_START: &str = "2026-06-15T09:00:00Z";

/// Fixes of the track recorded before the receiver ever corrected the clock,
/// at 1 Hz.
pub const COLD_BOOT_FIX_COUNT: i64 = 30;

/// Boots that follow, each its own track.
pub const BOOT_COUNT: i64 = 4;

/// Fixes each boot records, at 1 Hz.
pub const FIXES_PER_BOOT: i64 = 200;

/// Fixes at the head of each boot whose host timestamp is still at the RTC
/// default, the receiver not yet having corrected the clock.
pub const FIXES_BEFORE_THE_CLOCK_IS_CORRECTED: i64 = 40;

/// Seconds between the starts of two boots, in the true epoch.
const SECONDS_BETWEEN_BOOTS: i64 = 3_600;

/// How far the host clock runs ahead of the receiver's once corrected, in
/// milliseconds.
pub const HOST_AHEAD_MS: i64 = 200;

/// Sample rate of the `accel` channel, in hertz.
pub const CHANNEL_RATE_HZ: i64 = 20;

/// Samples in each run of the `accel` channel, in stored order.
pub const CHANNEL_RUN_LENGTHS: [i64; 4] = [250, 200, 300, 250];

/// Seconds after the RTC default at which every run of the `accel` channel
/// starts.
pub const CHANNEL_RUN_START_SECS: i64 = 1;

/// First latitude of the invented route, in degrees.
const ORIGIN_LAT_DEG: f64 = 10.0;

/// Longitude of the invented route, in degrees. It runs due north.
const ROUTE_LON_DEG: f64 = 20.0;

/// Degrees of latitude between two fixes of the invented route.
const LAT_STEP_DEG: f64 = 1e-5;

/// How far the true epoch lies past the RTC default: the offset a fix carries
/// while the receiver has not yet corrected the clock.
pub fn gap_between_the_clock_epochs() -> Duration {
    parse_time(CORRECTED_CLOCK_START) - parse_time(RTC_DEFAULT)
}

/// Bytes of a `.gtd` recording holding both clock epochs, a channel whose
/// sample timestamps restart with the RTC, and event markers in either epoch.
///
/// See the module documentation for the shape.
#[expect(
    clippy::expect_used,
    reason = "fixture generation should fail loudly when its own input is invalid"
)]
pub fn recording_whose_clock_restarts_at_every_boot() -> Vec<u8> {
    let rtc_default = parse_time(RTC_DEFAULT);
    let corrected_clock_start = parse_time(CORRECTED_CLOCK_START);

    let mut recorder = sdk::NavFileBuilder::new()
        .with_meta(
            sdk::Meta::builder()
                .title("Harbour patrol")
                .device("Fieldlogger")
                .notes("Tracker whose clock restarts at every boot")
                .identity("clock-reset-v1")
                .build(),
        )
        .open();

    let mut fix_index = 0_i64;
    for i in 0..COLD_BOOT_FIX_COUNT {
        recorder.add_nav_fix(fix(
            sdk::NavFixTime::Receiver(rtc_default + Duration::seconds(i)),
            fix_index,
        ));
        fix_index += 1;
    }
    for boot in 0..BOOT_COUNT {
        let boot_start = corrected_clock_start + Duration::seconds(boot * SECONDS_BETWEEN_BOOTS);
        for i in 0..FIXES_PER_BOOT {
            let receiver_time = boot_start + Duration::seconds(i);
            let host_time = if i < FIXES_BEFORE_THE_CLOCK_IS_CORRECTED {
                rtc_default + Duration::seconds(i)
            } else {
                receiver_time + Duration::milliseconds(HOST_AHEAD_MS)
            };
            recorder.add_nav_fix(fix(
                sdk::NavFixTime::Both {
                    gps: receiver_time,
                    sys: host_time,
                },
                fix_index,
            ));
            fix_index += 1;
        }
    }

    recorder.add_channel(accel_channel(rtc_default));

    for (variant_path, time) in [
        ("power/boot", rtc_default + Duration::seconds(2)),
        ("sensor/calibrated", rtc_default + Duration::seconds(12)),
        (
            "clock/corrected",
            corrected_clock_start + Duration::seconds(FIXES_BEFORE_THE_CLOCK_IS_CORRECTED),
        ),
        (
            "power/boot",
            corrected_clock_start + Duration::seconds(SECONDS_BETWEEN_BOOTS + 1),
        ),
    ] {
        recorder.add_event_marker(
            sdk::EventMarker::builder()
                .variant_path(variant_path)
                .sys_time(time)
                .build()
                .expect("the fixture's event markers are well formed"),
        );
    }

    let nav_file = recorder.finish().expect("the fixture must be valid");
    let mut bytes = Vec::new();
    nav_file
        .write(&mut bytes)
        .expect("writing the fixture must succeed");
    bytes
}

/// One fix of the invented route, `index` fixes along it.
fn fix(time: sdk::NavFixTime, index: i64) -> sdk::NavFix {
    sdk::NavFix::builder()
        .time(time)
        .lat(sdk::Angle::degrees(
            ORIGIN_LAT_DEG + index as f64 * LAT_STEP_DEG,
        ))
        .lon(sdk::Angle::degrees(ROUTE_LON_DEG))
        .heading(sdk::Angle::degrees(0.0))
        .speed(sdk::Velocity::kilometer_per_hour(4.0))
        .eph_m(3.5)
        .build()
}

/// The `accel` channel: [`CHANNEL_RUN_LENGTHS`] runs of samples in milli-g,
/// every one of them stamped in the RTC epoch and restarting at
/// [`CHANNEL_RUN_START_SECS`] past the RTC default.
#[expect(
    clippy::expect_used,
    reason = "fixture generation should fail loudly when its own input is invalid"
)]
fn accel_channel(rtc_default: DateTime<Utc>) -> sdk::Channel {
    let run_start = rtc_default + Duration::seconds(CHANNEL_RUN_START_SECS);
    let mut times = Vec::new();
    let mut values = Vec::new();
    for run_length in CHANNEL_RUN_LENGTHS {
        for i in 0..run_length {
            times.push(run_start + Duration::milliseconds(i * 1_000 / CHANNEL_RATE_HZ));
            let position = times.len() as f64;
            values.push((position % 40.0 - 20.0) * 5.0);
            values.push((position % 25.0 - 12.0) * 8.0);
            values.push(1_000.0 + (position % 17.0 - 8.0) * 6.0);
        }
    }
    sdk::Channel::builder()
        .name("accel")
        .unit(sdk::Unit::MG)
        .description("Device-frame acceleration, sampled on the tracker's own clock")
        .components(["x", "y", "z"])
        .times(times)
        .values(values)
        .build()
        .expect("the fixture's channel is well formed")
}

#[expect(
    clippy::expect_used,
    reason = "fixture generation should fail loudly when its own input is invalid"
)]
fn parse_time(iso8601: &str) -> DateTime<Utc> {
    iso8601
        .parse()
        .expect("the fixture's timestamps are well formed")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fixture_holds_both_clock_epochs_and_a_channel_that_restarts() {
        let bytes = recording_whose_clock_restarts_at_every_boot();
        let file = sdk::NavFile::read(&mut std::io::Cursor::new(bytes)).expect("a valid recording");

        assert_eq!(
            file.nav_points().len() as i64,
            COLD_BOOT_FIX_COUNT + BOOT_COUNT * FIXES_PER_BOOT
        );
        let host_times: Vec<DateTime<Utc>> = file
            .nav_points()
            .iter()
            .filter_map(|point| point.fix.sys_time())
            .collect();
        assert_eq!(
            host_times.windows(2).filter(|w| w[1] < w[0]).count() as i64,
            BOOT_COUNT - 1,
            "the host clock returns to the RTC default at every boot after the first"
        );

        let [channel] = file.channels() else {
            panic!("the fixture holds one channel");
        };
        assert_eq!(
            channel.times().len() as i64,
            CHANNEL_RUN_LENGTHS.iter().sum::<i64>()
        );
        assert_eq!(
            channel
                .times()
                .windows(2)
                .filter(|w| w[1] < w[0])
                .count()
                .saturating_add(1),
            CHANNEL_RUN_LENGTHS.len()
        );
        assert_eq!(file.event_markers().len(), 4);
    }
}
