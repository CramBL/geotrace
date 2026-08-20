//! One flare event, as the catalog lists it.

use chrono::{DateTime, NaiveDate, Utc};
use gt_types::SunlitSide;

use crate::class::FlareClassification;

/// One solar flare of the catalog.
///
/// The three times are minute resolution, which is what the catalog
/// publishes. [`end`](Self::end) is absent for a flare whose decay the
/// catalog never closed off.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolarFlare {
    /// The catalog's own identifier, as `2024-05-09T00:58:00-FLR-001`.
    pub id: String,
    pub begin: DateTime<Utc>,
    pub peak: DateTime<Utc>,
    pub end: Option<DateTime<Utc>>,
    pub classification: FlareClassification,
    /// Heliographic coordinates of the flaring region, as `S20W19`.
    pub source_location: Option<String>,
    /// NOAA number of the active region the flare came from.
    pub active_region: Option<u32>,
}

impl SolarFlare {
    /// The UTC day the flare began in, which is the day the catalog lists it
    /// under.
    pub fn begin_day(&self) -> NaiveDate {
        self.begin.date_naive()
    }
}

/// One archived flare as a surface marks it, with the side of Earth the
/// receiver was on when the flare peaked.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MarkedFlare {
    pub flare: SolarFlare,
    /// [`None`] with no recording loaded, which leaves the receiver without a
    /// position to read a side at.
    pub receiver_side: Option<SunlitSide>,
}
