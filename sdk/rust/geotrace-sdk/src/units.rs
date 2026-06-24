use crate::error::Error;

/// A point in time, wrapping [`chrono::DateTime<chrono::Utc>`].
///
/// Construct from a raw integer timestamp with one of the explicit unit
/// constructors. Each constructor panics if the value is out of the representable
/// range (which is impossible for any realistic GPS or system timestamp).
/// A [`chrono::DateTime<chrono::Utc>`] converts into `Timestamp` via [`From`],
/// so existing code that already has a `DateTime<Utc>` requires no change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp(chrono::DateTime<chrono::Utc>);

impl Timestamp {
    #[expect(
        clippy::expect_used,
        reason = "caller contract: panics on out-of-range timestamp"
    )]
    pub fn from_unix_seconds(secs: u64) -> Self {
        let s = i64::try_from(secs).expect("unix seconds timestamp out of valid range");
        Self(
            chrono::DateTime::from_timestamp(s, 0)
                .expect("unix seconds timestamp out of valid range"),
        )
    }

    #[expect(
        clippy::expect_used,
        reason = "caller contract: panics on out-of-range timestamp"
    )]
    pub fn from_unix_millis(millis: u64) -> Self {
        let ms = i64::try_from(millis).expect("unix milliseconds timestamp out of valid range");
        Self(
            chrono::DateTime::from_timestamp_millis(ms)
                .expect("unix milliseconds timestamp out of valid range"),
        )
    }

    #[expect(
        clippy::expect_used,
        reason = "caller contract: panics on out-of-range timestamp"
    )]
    pub fn from_unix_micros(micros: u64) -> Self {
        let us = i64::try_from(micros).expect("unix microseconds timestamp out of valid range");
        Self(
            chrono::DateTime::from_timestamp_micros(us)
                .expect("unix microseconds timestamp out of valid range"),
        )
    }

    #[expect(
        clippy::expect_used,
        reason = "caller contract: panics on out-of-range timestamp"
    )]
    pub fn from_unix_nanos(nanos: u64) -> Self {
        let ns = i64::try_from(nanos).expect("unix nanoseconds timestamp out of valid range");
        Self(chrono::DateTime::from_timestamp_nanos(ns))
    }

    /// Parse an ISO 8601 / RFC 3339 timestamp from a string.
    pub fn try_from_iso8601(s: impl AsRef<str>) -> Result<Self, Error> {
        let s = s.as_ref();
        s.parse::<chrono::DateTime<chrono::Utc>>()
            .map(Self)
            .map_err(|e| Error::ParseError {
                unit: "Timestamp (ISO 8601)",
                input: s.to_owned(),
                reason: e.to_string(),
            })
    }
}

impl From<chrono::DateTime<chrono::Utc>> for Timestamp {
    fn from(dt: chrono::DateTime<chrono::Utc>) -> Self {
        Self(dt)
    }
}

impl From<Timestamp> for chrono::DateTime<chrono::Utc> {
    fn from(ts: Timestamp) -> Self {
        ts.0
    }
}

/// An angle, stored internally as degrees.
///
/// Construct with [`Angle::degrees`] or [`Angle::radians`];
/// read back with [`Angle::as_degrees`] or [`Angle::as_radians`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Angle(f64);

impl Angle {
    pub fn degrees(v: f64) -> Self {
        Self(v)
    }

    pub fn radians(v: f64) -> Self {
        Self(v.to_degrees())
    }

    pub fn as_degrees(self) -> f64 {
        self.0
    }

    pub fn as_radians(self) -> f64 {
        self.0.to_radians()
    }

    /// Parse a decimal degree value from a string.
    pub fn try_from_degrees_str(s: impl AsRef<str>) -> Result<Self, Error> {
        let s = s.as_ref();
        s.parse::<f64>()
            .map(Self::degrees)
            .map_err(|e| Error::ParseError {
                unit: "Angle (degrees)",
                input: s.to_owned(),
                reason: e.to_string(),
            })
    }
}

impl From<uom::si::f64::Angle> for Angle {
    fn from(a: uom::si::f64::Angle) -> Self {
        Self(a.get::<uom::si::angle::degree>())
    }
}

/// A speed, stored internally as metres per second.
///
/// Construct with [`Velocity::meter_per_second`], [`Velocity::kilometer_per_hour`],
/// or [`Velocity::knot`]. Read back with the matching `as_*` getter.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Velocity(f64);

const MPS_PER_KMH: f64 = 1.0 / 3.6;
const MPS_PER_KNOT: f64 = 1852.0 / 3600.0;

impl Velocity {
    pub fn meter_per_second(v: f64) -> Self {
        Self(v)
    }

    pub fn kilometer_per_hour(v: f64) -> Self {
        Self(v * MPS_PER_KMH)
    }

    pub fn knot(v: f64) -> Self {
        Self(v * MPS_PER_KNOT)
    }

    pub fn as_meters_per_second(self) -> f64 {
        self.0
    }

    pub fn as_kilometers_per_hour(self) -> f64 {
        self.0 / MPS_PER_KMH
    }

    pub fn as_knots(self) -> f64 {
        self.0 / MPS_PER_KNOT
    }

    /// Parse a km/h value from a string.
    pub fn try_from_kmh_str(s: impl AsRef<str>) -> Result<Self, Error> {
        let s = s.as_ref();
        s.parse::<f64>()
            .map(Self::kilometer_per_hour)
            .map_err(|e| Error::ParseError {
                unit: "Velocity (km/h)",
                input: s.to_owned(),
                reason: e.to_string(),
            })
    }
}

impl From<uom::si::f64::Velocity> for Velocity {
    fn from(v: uom::si::f64::Velocity) -> Self {
        Self(v.get::<uom::si::velocity::meter_per_second>())
    }
}
