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
//! [`GpsTime::offset_from_sys`] - the explicit name signals that you are
//! performing a cross-domain measurement.

use chrono::{DateTime, Duration, Utc};
use std::fmt;
use std::ops::Sub;

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

    /// Return the inner `DateTime<Utc>`.
    ///
    /// Use this when you need a plain UTC value for display, serialisation, or
    /// comparison with non-domain-typed values (e.g. filter bounds).
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

    /// Unix timestamp as `f64` seconds, for use in float-based plot axes.
    #[inline]
    pub fn as_secs_f64(self) -> f64 {
        self.0.timestamp() as f64
    }
}

/// A start/end span in the GPS-time domain. Named so `start`/`end` are
/// self-documenting and a swapped pair is a compile error, the same reason
/// [`TimeRange`](crate::TimeRange) exists for wall-clock times.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GpsTimeRange {
    pub start: GpsTime,
    pub end: GpsTime,
}

impl GpsTimeRange {
    pub fn new(start: GpsTime, end: GpsTime) -> Self {
        Self { start, end }
    }

    /// `time`'s position across the span, in `[0, 1]`: 0 at `start`, 1 at
    /// `end`. Returns 0 for an empty span (`start == end`), and clamps values
    /// outside the span.
    pub fn normalize(self, time: GpsTime) -> f32 {
        let span_ms = self
            .end
            .signed_duration_since(self.start)
            .num_milliseconds();
        if span_ms <= 0 {
            return 0.0;
        }
        let offset_ms = time.signed_duration_since(self.start).num_milliseconds();
        (offset_ms as f32 / span_ms as f32).clamp(0.0, 1.0)
    }
}

/// `GpsTime − GpsTime → Duration` (same clock domain, always valid).
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

    /// Return the inner `DateTime<Utc>`.
    #[inline]
    pub fn utc(self) -> DateTime<Utc> {
        self.0
    }

    /// Signed duration `self − other` within the system clock domain.
    #[inline]
    pub fn signed_duration_since(self, other: SysTime) -> Duration {
        self.0.signed_duration_since(other.0)
    }
}

/// `SysTime − SysTime → Duration` (same clock domain, always valid).
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
    #[expect(
        clippy::float_cmp,
        reason = "0.0/0.5/1.0 are exactly representable, so the ratios are bit-exact"
    )]
    fn gps_time_range_normalize() {
        let range = GpsTimeRange::new(gps(1000), gps(3000));
        assert_eq!(range.normalize(gps(1000)), 0.0);
        assert_eq!(range.normalize(gps(3000)), 1.0);
        assert_eq!(range.normalize(gps(2000)), 0.5);
        // Outside the span clamps; an empty span is all zero.
        assert_eq!(range.normalize(gps(500)), 0.0);
        assert_eq!(range.normalize(gps(4000)), 1.0);
        assert_eq!(
            GpsTimeRange::new(gps(1000), gps(1000)).normalize(gps(1000)),
            0.0
        );
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
        let s = sys(400); // sys at t=0.4s → GPS 600 ms ahead
        assert_eq!(g.offset_from_sys(s).num_milliseconds(), 600);
    }

    #[test]
    fn offset_from_sys_negative_means_sys_ahead() {
        let g = gps(400); // GPS at t=0.4s
        let s = sys(1000); // sys at t=1s → sys 600 ms ahead
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

    #[test]
    fn round_trip_utc() {
        use chrono::Utc;
        let dt = Utc::now();
        assert_eq!(GpsTime::from_utc(dt).utc(), dt);
        assert_eq!(SysTime::from_utc(dt).utc(), dt);
    }
}
