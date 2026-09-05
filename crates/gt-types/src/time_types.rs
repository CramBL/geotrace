//! Newtype wrappers that prevent GPS time and system time from being silently
//! conflated.
//!
//! A GPS receiver maintains its own clock (`GpsTime`) that can drift from the
//! host OS clock (`SysTime`) by hundreds of milliseconds or more.  Mixing the
//! two in arithmetic produces silently-incorrect durations.  These newtypes
//! make the clock domain explicit and block cross-domain subtraction at compile
//! time.
//!
//! # Public surface
//!
//! These types are internal to the workspace (not re-exported by `geotrace-sdk`).
//! The `geotrace-sdk` public API continues to accept and return plain
//! `DateTime<Utc>` values. Conversions happen at the `gt-io` boundary.
//!
//! # Cross-domain operations
//!
//! When you genuinely need the GPS/sys-clock offset, use
//! [`GpsTime::offset_from_sys`] - a reader sees from the explicit name that the
//! measurement crosses the two clock domains.

use chrono::{DateTime, Duration, Utc};
use std::fmt;
use std::ops::Sub;

const NANOS_PER_SEC: f64 = 1e9;

/// Unix seconds with the sub-second fraction, the conversion both clock types
/// read their timestamps through.
fn secs_f64_with_subseconds(dt: DateTime<Utc>) -> f64 {
    dt.timestamp() as f64 + f64::from(dt.timestamp_subsec_nanos()) / NANOS_PER_SEC
}

/// A timestamp from the GPS receiver clock.
///
/// GPS time and system time are different clocks. Use [`SysTime`] for host
/// system-clock timestamps.  Subtracting a `GpsTime` from a `SysTime` (or
/// vice-versa) is a compile-time error - use [`GpsTime::offset_from_sys`] for
/// intentional cross-domain comparisons.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct GpsTime(DateTime<Utc>);

impl GpsTime {
    /// Wrap a `DateTime<Utc>` known to originate from the GPS receiver clock.
    #[inline]
    pub fn from_utc(dt: DateTime<Utc>) -> Self {
        Self(dt)
    }

    #[inline]
    pub fn utc(self) -> DateTime<Utc> {
        self.0
    }

    /// Signed duration `self − other` within the GPS clock domain.
    #[inline]
    pub fn signed_duration_since(self, other: GpsTime) -> Duration {
        self.0.signed_duration_since(other.0)
    }

    /// GPS/sys-clock offset: `GPS − sys`.
    ///
    /// A positive value means the GPS clock is ahead of the system clock.
    /// This is the only sanctioned way to compare across clock domains.
    #[inline]
    pub fn offset_from_sys(self, sys: SysTime) -> Duration {
        self.0 - sys.0
    }

    /// Unix timestamp as `f64` seconds, floored to a whole second, for use in
    /// float-based plot axes.
    ///
    /// Fixes less than a second apart share one value here: use
    /// [`GpsTime::as_secs_f64_with_subseconds`] wherever they must stay
    /// distinct.
    #[inline]
    pub fn as_secs_f64(self) -> f64 {
        self.0.timestamp() as f64
    }

    /// Unix timestamp as `f64` seconds, keeping the sub-second fraction.
    ///
    /// This is what a rate or a window computed over fixes faster than 1 Hz
    /// needs. [`GpsTime::as_secs_f64`] is the whole-second form.
    #[inline]
    pub fn as_secs_f64_with_subseconds(self) -> f64 {
        secs_f64_with_subseconds(self.0)
    }
}

/// A start/end span in the GPS-time domain, the GPS-clock counterpart of
/// [`TimeRange`](crate::TimeRange).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpsTimeRange {
    pub start: GpsTime,
    pub end: GpsTime,
}

impl GpsTimeRange {
    pub fn new(start: GpsTime, end: GpsTime) -> Self {
        Self { start, end }
    }
}

impl Sub<GpsTime> for GpsTime {
    type Output = Duration;

    #[inline]
    fn sub(self, rhs: GpsTime) -> Duration {
        self.0 - rhs.0
    }
}

impl fmt::Display for GpsTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// A timestamp from the host system (OS) clock.
///
/// System time and GPS time are different clocks. Use [`GpsTime`] for GPS
/// receiver timestamps.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct SysTime(DateTime<Utc>);

impl SysTime {
    /// Wrap a `DateTime<Utc>` known to originate from the host system clock.
    #[inline]
    pub fn from_utc(dt: DateTime<Utc>) -> Self {
        Self(dt)
    }

    #[inline]
    pub fn utc(self) -> DateTime<Utc> {
        self.0
    }

    /// Signed duration `self − other` within the system clock domain.
    #[inline]
    pub fn signed_duration_since(self, other: SysTime) -> Duration {
        self.0.signed_duration_since(other.0)
    }

    /// Unix timestamp as `f64` seconds, keeping the sub-second fraction.
    ///
    /// The same conversion as [`GpsTime::as_secs_f64_with_subseconds`]: a value
    /// from either clock compares against the other.
    #[inline]
    pub fn as_secs_f64_with_subseconds(self) -> f64 {
        secs_f64_with_subseconds(self.0)
    }
}

impl Sub<SysTime> for SysTime {
    type Output = Duration;

    #[inline]
    fn sub(self, rhs: SysTime) -> Duration {
        self.0 - rhs.0
    }
}

impl fmt::Display for SysTime {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// The clock that stamped a fix with its time.
///
/// The host clock stamps a fix when the receiver has no lock and no GPS time
/// to report. That fix's difference from the same host clock is a structural
/// zero. Keeping the two clocks apart here stops that zero from being reported
/// as a measured GPS/system offset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixTimestamp {
    /// The receiver had a lock and reported this time with the fix.
    FromGpsReceiver(GpsTime),
    /// The receiver had no lock: the host clock stamped the fix.
    FromHostClock(SysTime),
}

impl From<GpsTime> for FixTimestamp {
    fn from(gps: GpsTime) -> Self {
        Self::FromGpsReceiver(gps)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn gps(ms: i64) -> GpsTime {
        GpsTime::from_utc(DateTime::from_timestamp_millis(ms).expect("valid"))
    }

    fn sys(ms: i64) -> SysTime {
        SysTime::from_utc(DateTime::from_timestamp_millis(ms).expect("valid"))
    }

    #[test]
    fn gps_sub_gps_gives_duration() {
        let a = gps(2000);
        let b = gps(1000);
        assert_eq!((a - b).num_milliseconds(), 1000);
        assert_eq!((b - a).num_milliseconds(), -1000);
    }

    #[test]
    fn sys_sub_sys_gives_duration() {
        let a = sys(3000);
        let b = sys(1000);
        assert_eq!((a - b).num_milliseconds(), 2000);
    }

    #[test]
    fn offset_from_sys_positive_means_gps_ahead() {
        let g = gps(1000); // GPS at t=1s
        let s = sys(400); // `sys` at t=0.4s → GPS 600 ms ahead
        assert_eq!(g.offset_from_sys(s).num_milliseconds(), 600);
    }

    #[test]
    fn offset_from_sys_negative_means_sys_ahead() {
        let g = gps(400); // GPS at t=0.4s
        let s = sys(1000); // `sys` at t=1s → the system clock is 600 ms ahead
        assert_eq!(g.offset_from_sys(s).num_milliseconds(), -600);
    }

    #[test]
    fn signed_duration_since() {
        let later = gps(2000);
        let earlier = gps(500);
        assert_eq!(
            later.signed_duration_since(earlier).num_milliseconds(),
            1500
        );
    }

    #[rstest::rstest]
    #[case::whole_second(1_700_000_000_000, 1_700_000_000.0)]
    #[case::sub_second(1_700_000_000_250, 1_700_000_000.25)]
    #[case::before_the_epoch(-1500, -1.5)]
    fn as_secs_f64_with_subseconds(#[case] millis: i64, #[case] expected_secs: f64) {
        #[expect(clippy::float_cmp, reason = "every case is exact in binary")]
        {
            assert_eq!(gps(millis).as_secs_f64_with_subseconds(), expected_secs);
            assert_eq!(sys(millis).as_secs_f64_with_subseconds(), expected_secs);
        }
    }

    #[test]
    fn round_trip_utc() {
        use chrono::Utc;
        let dt = Utc::now();
        assert_eq!(GpsTime::from_utc(dt).utc(), dt);
        assert_eq!(SysTime::from_utc(dt).utc(), dt);
    }
}
