use chrono::{DateTime, Utc};

/// GPS-receiver clock timestamp - wraps [`DateTime<Utc>`] to prevent accidental
/// confusion with system-clock time inside SDK processing code.
///
/// The public SDK API still accepts and returns plain `DateTime<Utc>` so external
/// callers are not exposed to this type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct GpsTime(DateTime<Utc>);

impl GpsTime {
    pub(crate) fn from_utc(dt: DateTime<Utc>) -> Self {
        Self(dt)
    }

    pub(crate) fn utc(self) -> DateTime<Utc> {
        self.0
    }

    pub(crate) fn timestamp_micros(self) -> i64 {
        self.0.timestamp_micros()
    }
}

/// Host system-clock timestamp - wraps [`DateTime<Utc>`] to prevent accidental
/// confusion with GPS-receiver time inside SDK processing code.
///
/// The public SDK API still accepts and returns plain `DateTime<Utc>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct SysTime(DateTime<Utc>);

impl SysTime {
    pub(crate) fn from_utc(dt: DateTime<Utc>) -> Self {
        Self(dt)
    }

    pub(crate) fn timestamp_micros(self) -> i64 {
        self.0.timestamp_micros()
    }
}
