//! The unit the footer's association-window value is entered and shown in.

use chrono::Duration;
use gt_types::{AssociationWindow, InvalidAssociationWindow};
use gt_ui_types::LoadedLogId;
use strum::EnumIter;

pub(super) struct AssociationWindowEdit {
    pub log: LoadedLogId,
    pub window: AssociationWindow,
}

/// The unit an association window is written in. A log from a device whose
/// clock drifted against its recording needs a window in minutes or hours. One
/// checked against the fix rate needs milliseconds.
#[derive(Clone, Copy, Debug, EnumIter, Eq, PartialEq)]
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

    pub(super) fn measure(self, window: AssociationWindow) -> f64 {
        Duration::from(window).as_seconds_f64() * NANOS_PER_SECOND / self.nanoseconds_each()
    }

    pub(super) fn window_of(
        self,
        value: f64,
    ) -> Result<AssociationWindow, InvalidAssociationWindow> {
        if !value.is_finite() || !(0.0..=self.largest_value()).contains(&value) {
            return Err(InvalidAssociationWindow);
        }
        let nanos = (value * self.nanoseconds_each()).round();
        AssociationWindow::try_from(Duration::nanoseconds(nanos as i64))
    }

    pub(super) fn largest_value(self) -> f64 {
        self.measure(AssociationWindow::MAX)
    }

    /// The window written out for a hover text, e.g. "60s". A window this unit
    /// does not divide evenly is rounded to [`DESCRIBED_DECIMALS`] decimals.
    pub(super) fn describe(self, window: AssociationWindow) -> String {
        let value = format!("{:.*}", DESCRIBED_DECIMALS, self.measure(window));
        let value = value.trim_end_matches('0').trim_end_matches('.');
        format!("{value}{}", self.label())
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
}

/// Decimals a window is written to in a hover text. Two are enough to
/// distinguish one association window from another without spelling out a
/// repeating fraction.
const DESCRIBED_DECIMALS: usize = 2;

const NANOS_PER_MICROSECOND: f64 = 1e3;
const NANOS_PER_MILLISECOND: f64 = 1e6;
const NANOS_PER_SECOND: f64 = 1e9;
const NANOS_PER_MINUTE: f64 = 60.0 * NANOS_PER_SECOND;
const NANOS_PER_HOUR: f64 = 60.0 * NANOS_PER_MINUTE;
const NANOS_PER_DAY: f64 = 24.0 * NANOS_PER_HOUR;

#[cfg(test)]
mod tests {
    use rstest::rstest;

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
    #[case::round_down(
        AssociationWindowUnit::Seconds,
        0.000_000_001_4,
        Duration::nanoseconds(1)
    )]
    #[case::round_up(
        AssociationWindowUnit::Seconds,
        0.000_000_001_6,
        Duration::nanoseconds(2)
    )]
    #[case::minutes(AssociationWindowUnit::Minutes, 1.5, Duration::seconds(90))]
    #[case::hours(AssociationWindowUnit::Hours, 2.0, Duration::hours(2))]
    #[case::days(AssociationWindowUnit::Days, 0.5, Duration::hours(12))]
    fn a_value_in_a_unit_is_the_window_that_unit_counts(
        #[case] unit: AssociationWindowUnit,
        #[case] value: f64,
        #[case] expected: Duration,
    ) {
        assert_eq!(unit.window_of(value).map(Duration::from), Ok(expected));
    }

    #[rstest]
    #[case::negative(-1.0)]
    #[case::above_maximum(366.0)]
    #[case::not_a_number(f64::NAN)]
    #[case::infinite(f64::INFINITY)]
    fn invalid_window_values_return_an_error(#[case] value: f64) {
        assert_eq!(
            AssociationWindowUnit::Days.window_of(value),
            Err(InvalidAssociationWindow)
        );
    }

    #[test]
    fn a_window_the_unit_does_not_divide_evenly_is_described_rounded() {
        assert_eq!(
            AssociationWindowUnit::Minutes
                .describe(AssociationWindow::try_from_seconds(61).unwrap()),
            "1.02min"
        );
    }
}
