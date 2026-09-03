//! The unit the footer's association-window value is entered and shown in.

use chrono::Duration;
use strum::EnumIter;

/// Longest association window the footer offers, in nanoseconds: one year, past
/// any clock offset a log and a recording could plausibly have.
const MAX_ASSOCIATION_WINDOW_NANOS: f64 = 365.0 * 24.0 * 60.0 * 60.0 * 1e9;

/// Decimals a window is written to in a hover text. Two are enough to tell one
/// association window from another without spelling out a repeating fraction.
const DESCRIBED_DECIMALS: usize = 2;

const NANOS_PER_MICROSECOND: f64 = 1e3;
const NANOS_PER_MILLISECOND: f64 = 1e6;
const NANOS_PER_SECOND: f64 = 1e9;
const NANOS_PER_MINUTE: f64 = 60.0 * NANOS_PER_SECOND;
const NANOS_PER_HOUR: f64 = 60.0 * NANOS_PER_MINUTE;
const NANOS_PER_DAY: f64 = 24.0 * NANOS_PER_HOUR;

/// The unit an association window is written in. A log from a device whose
/// clock drifted against its recording needs a window in minutes or hours. One
/// checked against the fix rate needs milliseconds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, EnumIter)]
pub(super) enum AssociationWindowUnit {
    Nanoseconds,
    Microseconds,
    Milliseconds,
    Seconds,
    Minutes,
    Hours,
    Days,
}

impl AssociationWindowUnit {
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Nanoseconds => "ns",
            Self::Microseconds => "µs",
            Self::Milliseconds => "ms",
            Self::Seconds => "s",
            Self::Minutes => "min",
            Self::Hours => "h",
            Self::Days => "d",
        }
    }

    fn nanoseconds_each(self) -> f64 {
        match self {
            Self::Nanoseconds => 1.0,
            Self::Microseconds => NANOS_PER_MICROSECOND,
            Self::Milliseconds => NANOS_PER_MILLISECOND,
            Self::Seconds => NANOS_PER_SECOND,
            Self::Minutes => NANOS_PER_MINUTE,
            Self::Hours => NANOS_PER_HOUR,
            Self::Days => NANOS_PER_DAY,
        }
    }

    /// `window` counted in this unit, as the drag value shows it.
    pub(super) fn measure(self, window: Duration) -> f64 {
        let nanos = window.num_nanoseconds().unwrap_or(i64::MAX);
        nanos as f64 / self.nanoseconds_each()
    }

    /// The window `value` of this unit stands for, clamped to the range the
    /// drag value accepts.
    pub(super) fn window_of(self, value: f64) -> Duration {
        let nanos = (value * self.nanoseconds_each()).clamp(0.0, MAX_ASSOCIATION_WINDOW_NANOS);
        Duration::nanoseconds(nanos as i64)
    }

    pub(super) fn largest_value(self) -> f64 {
        MAX_ASSOCIATION_WINDOW_NANOS / self.nanoseconds_each()
    }

    /// The window written out for a hover text, e.g. "60s". A window this unit
    /// does not divide evenly is rounded to [`DESCRIBED_DECIMALS`] decimals.
    pub(super) fn describe(self, window: Duration) -> String {
        let value = format!("{:.*}", DESCRIBED_DECIMALS, self.measure(window));
        let value = value.trim_end_matches('0').trim_end_matches('.');
        format!("{value}{}", self.label())
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use strum::IntoEnumIterator as _;

    use super::*;

    #[rstest]
    #[case::nanoseconds(
        AssociationWindowUnit::Nanoseconds,
        1_500.0,
        Duration::nanoseconds(1_500)
    )]
    #[case::microseconds(
        AssociationWindowUnit::Microseconds,
        1_500.0,
        Duration::microseconds(1_500)
    )]
    #[case::milliseconds(
        AssociationWindowUnit::Milliseconds,
        250.0,
        Duration::milliseconds(250)
    )]
    #[case::seconds(AssociationWindowUnit::Seconds, 60.0, Duration::seconds(60))]
    #[case::minutes(AssociationWindowUnit::Minutes, 1.5, Duration::seconds(90))]
    #[case::hours(AssociationWindowUnit::Hours, 2.0, Duration::hours(2))]
    #[case::days(AssociationWindowUnit::Days, 0.5, Duration::hours(12))]
    fn a_value_in_a_unit_is_the_window_that_unit_counts(
        #[case] unit: AssociationWindowUnit,
        #[case] value: f64,
        #[case] expected: Duration,
    ) {
        assert_eq!(unit.window_of(value), expected);
        assert_eq!(unit.describe(expected), format!("{value}{}", unit.label()));
    }

    /// Every unit measures the same window: the dropdown only restates it.
    #[test]
    fn switching_the_unit_leaves_the_window_it_describes_unchanged() {
        let window = Duration::hours(24);
        for unit in AssociationWindowUnit::iter() {
            assert_eq!(unit.window_of(unit.measure(window)), window, "{unit:?}");
        }
    }

    #[test]
    fn a_value_past_the_longest_window_clamps_to_it() {
        assert_eq!(
            AssociationWindowUnit::Days.window_of(10_000.0),
            AssociationWindowUnit::Days.window_of(365.0)
        );
    }

    #[test]
    fn a_window_the_unit_does_not_divide_evenly_is_described_rounded() {
        assert_eq!(
            AssociationWindowUnit::Minutes.describe(Duration::seconds(61)),
            "1.02min"
        );
    }
}
