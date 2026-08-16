//! The parsed index series: one sample per published period.

use chrono::{DateTime, Utc};

use crate::GeomagneticIndex;
use crate::activity::GeomagneticActivity;

/// One period of an index series.
pub trait IndexSample {
    /// The index this sample is published under.
    const INDEX: GeomagneticIndex;

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
    const INDEX: GeomagneticIndex = GeomagneticIndex::Kp;

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
    const INDEX: GeomagneticIndex = GeomagneticIndex::Hp30;

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

impl KpSeries {
    /// Whether any sample is a [`KpStatus::Nowcast`] value, which GFZ replaces
    /// with a definitive one once every station has reported. A caller holding
    /// such a series is holding one the service can still revise.
    pub fn contains_nowcast_samples(&self) -> bool {
        self.samples
            .iter()
            .any(|sample| sample.status == KpStatus::Nowcast)
    }
}

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

    /// The value of the period covering `time`, or [`None`] where no period
    /// covers it or the one that does has no published value.
    ///
    /// A period runs from its start for
    /// [`GeomagneticIndex::period_length`], and the value holds for its whole
    /// length: the series is a step function, not a curve to interpolate
    /// along. Found by binary search over the period starts, which the series
    /// holds in published order.
    pub fn activity_at(&self, time: DateTime<Utc>) -> Option<GeomagneticActivity> {
        let position = self
            .samples
            .partition_point(|sample| sample.period_start() <= time)
            .checked_sub(1)?;
        let sample = self.samples.get(position)?;
        let period_end = sample
            .period_start()
            .checked_add_signed(S::INDEX.period_length())?;
        (time < period_end).then(|| sample.activity()).flatten()
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

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

    /// One quiet Hp30 day's worth of periods, half an hour apart, with a gap
    /// where the service published no value.
    fn hp30_day() -> Hp30Series {
        Hp30Series {
            samples: vec![
                hp30_sample("2024-05-10T00:00:00Z", 3.0),
                hp30_sample("2024-05-10T00:30:00Z", 11.333),
                Hp30Sample {
                    period_start: parse_timestamp("2024-05-10T01:00:00Z").unwrap(),
                    activity: None,
                },
                hp30_sample("2024-05-10T01:30:00Z", 4.667),
            ],
        }
    }

    #[rstest]
    #[case::before_the_first_period("2024-05-09T23:59:59Z", None)]
    #[case::the_first_period_starts("2024-05-10T00:00:00Z", Some(3.0))]
    #[case::inside_the_first_period("2024-05-10T00:29:59Z", Some(3.0))]
    #[case::the_next_period_starts("2024-05-10T00:30:00Z", Some(11.333))]
    #[case::a_period_without_a_value("2024-05-10T01:15:00Z", None)]
    #[case::the_last_period("2024-05-10T01:59:59Z", Some(4.667))]
    #[case::past_the_last_period("2024-05-10T02:00:00Z", None)]
    fn a_value_holds_for_its_whole_period(#[case] time: &str, #[case] expected: Option<f64>) {
        assert_eq!(
            hp30_day()
                .activity_at(parse_timestamp(time).unwrap())
                .map(GeomagneticActivity::value),
            expected
        );
    }

    /// Kp periods are six times as long, so the same value covers a fix three
    /// hours after the period started.
    #[test]
    fn a_kp_value_holds_for_three_hours() {
        let series = KpSeries {
            samples: vec![kp_sample(KpStatus::Definitive)],
        };
        let at = |time: &str| {
            series
                .activity_at(parse_timestamp(time).unwrap())
                .map(GeomagneticActivity::value)
        };
        assert_eq!(at("2024-05-10T02:59:59Z"), Some(3.0));
        assert_eq!(at("2024-05-10T03:00:00Z"), None);
    }

    #[test]
    fn an_empty_series_has_no_value_at_any_time() {
        assert_eq!(
            Hp30Series { samples: vec![] }
                .activity_at(parse_timestamp("2024-05-10T00:00:00Z").unwrap()),
            None
        );
    }

    #[test]
    fn an_empty_series_has_no_peak() {
        let series = KpSeries { samples: vec![] };
        assert_eq!(series.peak_activity(), None);
        assert!(series.is_empty());
    }

    fn kp_sample(status: KpStatus) -> KpSample {
        KpSample {
            period_start: parse_timestamp("2024-05-10T00:00:00Z").unwrap(),
            activity: GeomagneticActivity::from_published_value(GeomagneticIndex::Kp, 3.0),
            status,
        }
    }

    #[rstest]
    #[case::all_definitive(vec![KpStatus::Definitive, KpStatus::Definitive], false)]
    #[case::one_nowcast(vec![KpStatus::Definitive, KpStatus::Nowcast], true)]
    #[case::no_samples(vec![], false)]
    fn a_nowcast_sample_marks_the_series_revisable(
        #[case] statuses: Vec<KpStatus>,
        #[case] expected: bool,
    ) {
        let series = KpSeries {
            samples: statuses.into_iter().map(kp_sample).collect(),
        };
        assert_eq!(series.contains_nowcast_samples(), expected);
    }
}
