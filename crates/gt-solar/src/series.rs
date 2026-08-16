//! The parsed index series: one sample per published period.

use chrono::{DateTime, Utc};

use crate::activity::GeomagneticActivity;

/// One period of an index series.
pub trait IndexSample {
    /// Start of the period this sample covers. It runs for
    /// [`GeomagneticIndex::period_length`](crate::GeomagneticIndex::period_length).
    fn period_start(&self) -> DateTime<Utc>;

    /// The period's value, or [`None`] where the service published no value
    /// for it.
    fn activity(&self) -> Option<GeomagneticActivity>;
}

/// Whether a Kp value is the final one for its period.
///
/// The variant names are the endpoint's status values.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::EnumString, strum::VariantNames)]
pub enum KpStatus {
    /// Derived from the full station set and final.
    #[strum(serialize = "def")]
    Definitive,
    /// The nowcast value, which GFZ replaces with a definitive one once all
    /// stations have reported.
    #[strum(serialize = "pre")]
    Nowcast,
}

impl KpStatus {
    /// Canonical human-readable name shown in the UI.
    pub const fn display_name(self) -> &'static str {
        match self {
            Self::Definitive => "Definitive",
            Self::Nowcast => "Nowcast",
        }
    }
}

/// One three-hour Kp period.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct KpSample {
    pub period_start: DateTime<Utc>,
    pub activity: Option<GeomagneticActivity>,
    pub status: KpStatus,
}

impl IndexSample for KpSample {
    fn period_start(&self) -> DateTime<Utc> {
        self.period_start
    }

    fn activity(&self) -> Option<GeomagneticActivity> {
        self.activity
    }
}

/// One 30-minute Hp30 period. The service publishes no status for Hp30.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Hp30Sample {
    pub period_start: DateTime<Utc>,
    pub activity: Option<GeomagneticActivity>,
}

impl IndexSample for Hp30Sample {
    fn period_start(&self) -> DateTime<Utc> {
        self.period_start
    }

    fn activity(&self) -> Option<GeomagneticActivity> {
        self.activity
    }
}

/// One index over one requested window, in published order: oldest period
/// first, one sample per period the service answered with.
#[derive(Debug, Clone, PartialEq)]
pub struct IndexSeries<S> {
    pub samples: Vec<S>,
}

/// A Kp series, whose samples carry [`KpStatus`].
pub type KpSeries = IndexSeries<KpSample>;

/// An Hp30 series.
pub type Hp30Series = IndexSeries<Hp30Sample>;

impl<S: IndexSample> IndexSeries<S> {
    /// The highest value in the series, which is the one a storm summary
    /// leads with. [`None`] for a window the service published no value in.
    pub fn peak_activity(&self) -> Option<GeomagneticActivity> {
        self.samples
            .iter()
            .filter_map(IndexSample::activity)
            .max_by(|left, right| left.value().total_cmp(&right.value()))
    }

    /// The period start times, oldest first.
    pub fn period_starts(&self) -> impl Iterator<Item = DateTime<Utc>> {
        self.samples.iter().map(IndexSample::period_start)
    }

    pub fn is_empty(&self) -> bool {
        self.samples.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use crate::GeomagneticIndex;
    use crate::activity::GeomagneticStormClass;
    use crate::parse_timestamp;

    use super::*;

    fn hp30_sample(period_start: &str, value: f64) -> Hp30Sample {
        Hp30Sample {
            period_start: parse_timestamp(period_start).unwrap(),
            activity: GeomagneticActivity::from_published_value(GeomagneticIndex::Hp30, value),
        }
    }

    #[test]
    fn the_peak_is_the_highest_value_in_the_series() {
        let series = Hp30Series {
            samples: vec![
                hp30_sample("2024-05-10T00:00:00Z", 3.0),
                hp30_sample("2024-05-10T00:30:00Z", 11.333),
                hp30_sample("2024-05-10T01:00:00Z", 8.0),
            ],
        };
        assert_eq!(
            series
                .peak_activity()
                .and_then(GeomagneticActivity::storm_class),
            Some(GeomagneticStormClass::Extreme)
        );
        assert_eq!(series.period_starts().count(), 3);
        assert!(!series.is_empty());
    }

    #[test]
    fn a_series_of_only_gaps_has_no_peak() {
        let series = Hp30Series {
            samples: vec![Hp30Sample {
                period_start: parse_timestamp("2024-05-10T00:00:00Z").unwrap(),
                activity: None,
            }],
        };
        assert_eq!(series.peak_activity(), None);
        assert!(!series.is_empty());
    }

    #[test]
    fn an_empty_series_has_no_peak() {
        let series = KpSeries { samples: vec![] };
        assert_eq!(series.peak_activity(), None);
        assert!(series.is_empty());
    }
}
