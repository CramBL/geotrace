use crate::error::Error;

/// A point in time, wrapping [`chrono::DateTime<chrono::Utc>`].
///
/// Construct from a raw integer count with one of the explicit unit
/// constructors, each of which rejects a count outside the range a UTC
/// timestamp covers. A [`chrono::DateTime<chrono::Utc>`] converts into
/// `Timestamp` via [`From`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timestamp(chrono::DateTime<chrono::Utc>);

impl Timestamp {
    pub fn try_from_unix_seconds(seconds: i64) -> Result<Self, Error> {
        chrono::DateTime::from_timestamp(seconds, 0)
            .map(Self)
            .ok_or(Error::TimestampCountOutOfRange {
                count: seconds,
                unit: "seconds",
            })
    }

    pub fn try_from_unix_millis(millis: i64) -> Result<Self, Error> {
        chrono::DateTime::from_timestamp_millis(millis)
            .map(Self)
            .ok_or(Error::TimestampCountOutOfRange {
                count: millis,
                unit: "milliseconds",
            })
    }

    pub fn try_from_unix_micros(micros: i64) -> Result<Self, Error> {
        chrono::DateTime::from_timestamp_micros(micros)
            .map(Self)
            .ok_or(Error::TimestampCountOutOfRange {
                count: micros,
                unit: "microseconds",
            })
    }

    /// The count is truncated towards zero to whole microseconds, the
    /// resolution a `.gtd` file stores: 1999 nanoseconds is 1 microsecond and
    /// -1999 nanoseconds is -1 microsecond.
    pub fn try_from_unix_nanos(nanos: i64) -> Result<Self, Error> {
        chrono::DateTime::from_timestamp_micros(nanos / 1_000)
            .map(Self)
            .ok_or(Error::TimestampCountOutOfRange {
                count: nanos,
                unit: "nanoseconds",
            })
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
/// Construct with [`Angle::degrees`] or [`Angle::radians`].
/// Read back with [`Angle::as_degrees`] or [`Angle::as_radians`].
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

    /// The shortest signed arc from `self` to `other`, in [-180, 180)
    /// degrees, so a heading step from 359° to 1° reads as +2°, not -358°.
    pub fn signed_arc_to(self, other: Angle) -> Angle {
        Angle((other.0 - self.0 + 180.0).rem_euclid(360.0) - 180.0)
    }

    pub(crate) fn wrapped_to_plus_minus_180_degrees(self) -> Self {
        Self((self.0 + 180.0).rem_euclid(360.0) - 180.0)
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
