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

    /// The last instant the catalog accounts for: the published
    /// [`end`](Self::end), or the peak where it never closed off the decay.
    pub fn end_or_peak(&self) -> DateTime<Utc> {
        self.end.unwrap_or(self.peak)
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

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::wire::parse_flare_time;

    fn time(text: &str) -> DateTime<Utc> {
        parse_flare_time(text).expect("a catalog time")
    }

    /// The May 2024 X2.2, whose decay the catalog closed off, and the same
    /// flare left open.
    #[rstest]
    #[case::published_end(Some("2024-05-09T09:36Z"), "2024-05-09T09:36Z")]
    #[case::left_open(None, "2024-05-09T09:13Z")]
    fn a_flare_left_open_is_accounted_for_up_to_its_peak(
        #[case] published_end: Option<&str>,
        #[case] expected: &str,
    ) {
        let flare = SolarFlare {
            id: "2024-05-09T08:45:00-FLR-001".to_owned(),
            begin: time("2024-05-09T08:45Z"),
            peak: time("2024-05-09T09:13Z"),
            end: published_end.map(time),
            classification: "X2.2".parse().expect("a published class"),
            source_location: Some("S20W25".to_owned()),
            active_region: Some(13664),
        };

        assert_eq!(flare.end_or_peak(), time(expected));
    }
}
