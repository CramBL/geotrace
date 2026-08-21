//! How far one archived TEC value stands from the quiet-time level of the
//! same place and time of day, graded on the planetary ionospheric storm
//! index.
//!
//! The index compares a value against the median of the 27 days before the
//! day observed, so a quiet reference exists for a recording made yesterday.
//! That median is only ever formed from what the archive already holds: a
//! window too sparsely archived yields no deviation at all.
//!
//! The grade boundaries below are the W index thresholds of Table 3 in
//! Gulyaeva, Stanislawska and Tomasik, Annales Geophysicae 26, 2008.

use std::fmt;

use chrono::{NaiveDate, TimeDelta};

use crate::tec::TotalElectronContent;

/// Days before the day assessed the quiet-time median is taken over.
///
/// The index takes its quiet reference over one solar rotation as seen from
/// Earth, which is 27 days.
pub const BACKGROUND_WINDOW_DAYS: usize = 27;

/// Archived days of the window a median is formed from at all.
///
/// An ionospheric storm and its recovery run over several days, so a median
/// drawn from a minority of the window can be a storm's own level rather than
/// the quiet one. 14 is the smallest majority of the 27 days.
pub const MINIMUM_BACKGROUND_DAYS: usize = 14;

/// The deviation, in log10 units, at which the index leaves the quiet grade.
pub const MODERATE_DISTURBANCE_LOG_RATIO: f64 = 0.046;

/// The deviation, in log10 units, at which the index reaches the moderate
/// storm grade, W = 3 above the median and W = -3 below it.
pub const MODERATE_STORM_LOG_RATIO: f64 = 0.155;

/// The deviation, in log10 units, past which the index reaches the intense
/// storm grade, W = 4 above the median and W = -4 below it.
pub const INTENSE_STORM_LOG_RATIO: f64 = 0.301;

/// The 27 UTC days one day's quiet-time median is taken over, oldest first.
///
/// Days that fall past the start of the calendar are left out.
pub fn background_days(day: NaiveDate) -> Vec<NaiveDate> {
    (1..=BACKGROUND_WINDOW_DAYS as i64)
        .rev()
        .filter_map(|days_before| day.checked_sub_signed(TimeDelta::days(days_before)))
        .collect()
}

/// How the planetary ionospheric storm index grades one deviation from the
/// quiet-time median, by the magnitude of that deviation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IonosphericStormGrade {
    Quiet,
    ModerateDisturbance,
    ModerateStorm,
    IntenseStorm,
}

impl IonosphericStormGrade {
    /// Whether the ionosphere was storming, which is what GeoTrace warns
    /// about.
    pub fn is_a_storm(self) -> bool {
        matches!(self, Self::ModerateStorm | Self::IntenseStorm)
    }

    /// The index value of this grade, 1 to 4, which the sign of the deviation
    /// then makes positive or negative.
    fn storm_index_magnitude(self) -> i8 {
        match self {
            Self::Quiet => 1,
            Self::ModerateDisturbance => 2,
            Self::ModerateStorm => 3,
            Self::IntenseStorm => 4,
        }
    }

    fn of_log_ratio(log_ratio: f64) -> Self {
        let magnitude = log_ratio.abs();
        if magnitude > INTENSE_STORM_LOG_RATIO {
            Self::IntenseStorm
        } else if magnitude >= MODERATE_STORM_LOG_RATIO {
            Self::ModerateStorm
        } else if magnitude >= MODERATE_DISTURBANCE_LOG_RATIO {
            Self::ModerateDisturbance
        } else {
            Self::Quiet
        }
    }
}

impl fmt::Display for IonosphericStormGrade {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self {
            Self::Quiet => "quiet ionosphere",
            Self::ModerateDisturbance => "moderate ionospheric disturbance",
            Self::ModerateStorm => "moderate ionospheric storm",
            Self::IntenseStorm => "intense ionospheric storm",
        };
        f.write_str(name)
    }
}

/// How far one value stands from the quiet-time median of the same place and
/// time of day.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QuietTimeDeviation {
    log_ratio: f64,
}

impl QuietTimeDeviation {
    /// The deviation the index writes as DTEC: the base-10 logarithm of the
    /// value over its quiet-time median.
    pub fn from_log_ratio(log_ratio: f64) -> Self {
        Self { log_ratio }
    }

    /// DTEC, which the index grades and which orders two deviations by
    /// strength.
    pub fn log_ratio(self) -> f64 {
        self.log_ratio
    }

    /// The change from the median in percent: 62.0 for 62 % above it, -35.0
    /// for 35 % below it.
    pub fn percent_from_median(self) -> f64 {
        (10_f64.powf(self.log_ratio) - 1.0) * 100.0
    }

    pub fn grade(self) -> IonosphericStormGrade {
        IonosphericStormGrade::of_log_ratio(self.log_ratio)
    }

    /// The signed index value, W = 3 for a moderate storm above the median
    /// and W = -3 for one below it.
    pub fn storm_index_value(self) -> i8 {
        let magnitude = self.grade().storm_index_magnitude();
        if self.log_ratio < 0.0 {
            -magnitude
        } else {
            magnitude
        }
    }
}

/// The deviation of `value` from the quiet-time median of `window_values`.
///
/// `window_values` are what the archived days of one [`background_days`]
/// published for the same place and time of day.
///
/// [`None`] where fewer than [`MINIMUM_BACKGROUND_DAYS`] of the window are
/// archived, or where either the value or the median is not positive, which
/// leaves their logarithmic ratio undefined.
pub fn deviation_from_quiet_time(
    value: TotalElectronContent,
    window_values: &[TotalElectronContent],
) -> Option<QuietTimeDeviation> {
    if window_values.len() < MINIMUM_BACKGROUND_DAYS {
        return None;
    }
    let median = median_tecu(window_values)?;
    let ratio = value.tecu() / median;
    (median > 0.0 && ratio > 0.0).then(|| QuietTimeDeviation::from_log_ratio(ratio.log10()))
}

/// The middle value in TEC units, the mean of the middle two over an even
/// count.
fn median_tecu(values: &[TotalElectronContent]) -> Option<f64> {
    let mut sorted: Vec<f64> = values
        .iter()
        .copied()
        .map(TotalElectronContent::tecu)
        .collect();
    sorted.sort_by(f64::total_cmp);
    let above = sorted.len() / 2;
    let upper = *sorted.get(above)?;
    if sorted.len() % 2 == 1 {
        return Some(upper);
    }
    let lower = *sorted.get(above.checked_sub(1)?)?;
    Some((lower + upper) / 2.0)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap_or_default()
    }

    fn tecu(value: f64) -> TotalElectronContent {
        TotalElectronContent::from_tecu(value)
    }

    /// A window of `count` days all at `background`, which medians to it.
    fn quiet_window(count: usize, background: f64) -> Vec<TotalElectronContent> {
        vec![tecu(background); count]
    }

    /// The window runs up to the day before the one assessed, so a recording
    /// made yesterday has a whole quiet reference behind it.
    #[test]
    fn the_window_is_the_twenty_seven_days_before_the_day_assessed() {
        let assessed = day(2024, 5, 20);

        let window = background_days(assessed);

        assert_eq!(window.len(), BACKGROUND_WINDOW_DAYS);
        assert_eq!(window.first().copied(), Some(day(2024, 4, 23)));
        assert_eq!(window.last().copied(), Some(day(2024, 5, 19)));
        assert!(!window.contains(&assessed));
    }

    /// A day so close to the calendar's start that the window runs off it
    /// keeps the days that do exist.
    #[test]
    fn a_window_past_the_calendar_holds_the_days_that_exist() {
        assert!(background_days(NaiveDate::MIN).is_empty());
        assert_eq!(
            background_days(NaiveDate::MAX).len(),
            BACKGROUND_WINDOW_DAYS
        );
    }

    /// The index grades DTEC, the base-10 logarithm of the ratio, and grades
    /// a rise and a fall of the same magnitude alike. The boundary between
    /// the two storm grades belongs to the moderate one, and the warning
    /// fires from that grade up.
    #[rstest]
    #[case::the_median_itself(0.0, IonosphericStormGrade::Quiet, false)]
    #[case::below_the_first_boundary(0.045, IonosphericStormGrade::Quiet, false)]
    #[case::at_the_first_boundary(0.046, IonosphericStormGrade::ModerateDisturbance, false)]
    #[case::below_the_storm_boundary(0.154, IonosphericStormGrade::ModerateDisturbance, false)]
    #[case::at_the_storm_boundary(0.155, IonosphericStormGrade::ModerateStorm, true)]
    #[case::at_the_intense_boundary(0.301, IonosphericStormGrade::ModerateStorm, true)]
    #[case::past_the_intense_boundary(0.302, IonosphericStormGrade::IntenseStorm, true)]
    #[case::a_negative_moderate_storm(-0.155, IonosphericStormGrade::ModerateStorm, true)]
    #[case::a_negative_intense_storm(-0.302, IonosphericStormGrade::IntenseStorm, true)]
    fn a_deviation_is_graded_by_the_magnitude_of_its_logarithm(
        #[case] log_ratio: f64,
        #[case] expected: IonosphericStormGrade,
        #[case] warns: bool,
    ) {
        let grade = QuietTimeDeviation::from_log_ratio(log_ratio).grade();

        assert_eq!(grade, expected);
        assert_eq!(grade.is_a_storm(), warns);
    }

    /// The index value is signed by the side of the median the value falls
    /// on, and runs 1 to 4 with the grade.
    #[rstest]
    #[case::quiet(0.02, 1)]
    #[case::moderate_disturbance(0.1, 2)]
    #[case::moderate_storm(0.2, 3)]
    #[case::intense_storm(0.4, 4)]
    #[case::a_negative_moderate_storm(-0.2, -3)]
    fn the_index_value_states_the_grade_and_the_side_of_the_median(
        #[case] log_ratio: f64,
        #[case] expected: i8,
    ) {
        assert_eq!(
            QuietTimeDeviation::from_log_ratio(log_ratio).storm_index_value(),
            expected
        );
    }

    /// A value read against a whole window is graded on its own logarithmic
    /// distance from the median, either side of it.
    #[rstest]
    #[case::a_quiet_rise(21.0, 5.0, IonosphericStormGrade::Quiet)]
    #[case::a_moderate_disturbance(25.0, 25.0, IonosphericStormGrade::ModerateDisturbance)]
    #[case::just_short_of_a_storm(28.5, 42.5, IonosphericStormGrade::ModerateDisturbance)]
    #[case::a_positive_storm(32.4, 62.0, IonosphericStormGrade::ModerateStorm)]
    #[case::a_negative_storm(13.0, -35.0, IonosphericStormGrade::ModerateStorm)]
    #[case::an_intense_storm(60.0, 200.0, IonosphericStormGrade::IntenseStorm)]
    fn a_value_is_graded_against_the_median_of_its_window(
        #[case] value: f64,
        #[case] percent: f64,
        #[case] expected: IonosphericStormGrade,
    ) {
        let deviation = deviation_from_quiet_time(tecu(value), &quiet_window(27, 20.0))
            .expect("a fully archived window");

        assert!(
            (deviation.percent_from_median() - percent).abs() < 0.05,
            "{deviation:?} is not {percent} %"
        );
        assert_eq!(deviation.grade(), expected);
    }

    /// A window the archive holds too little of yields nothing, so a storm is
    /// never read off a handful of days.
    #[rstest]
    #[case::one_short_of_the_minimum(13, None)]
    #[case::the_minimum(14, Some(IonosphericStormGrade::IntenseStorm))]
    #[case::the_whole_window(27, Some(IonosphericStormGrade::IntenseStorm))]
    fn a_median_needs_a_majority_of_the_window(
        #[case] archived_days: usize,
        #[case] expected: Option<IonosphericStormGrade>,
    ) {
        let deviation = deviation_from_quiet_time(tecu(40.0), &quiet_window(archived_days, 20.0));

        assert_eq!(deviation.map(QuietTimeDeviation::grade), expected);
    }

    /// The median is the middle of the window, not its mean, so a few stormy
    /// days in it do not raise the quiet level they are measured against.
    #[test]
    fn a_few_stormy_days_leave_the_median_at_the_quiet_level() {
        let mut window = quiet_window(24, 20.0);
        window.extend([tecu(200.0), tecu(180.0), tecu(160.0)]);

        let deviation =
            deviation_from_quiet_time(tecu(30.0), &window).expect("a fully archived window");

        assert!(
            (deviation.percent_from_median() - 50.0).abs() < 1e-9,
            "{deviation:?}"
        );
    }

    /// A value or median of zero leaves the logarithmic ratio undefined, so
    /// no deviation is reported.
    #[rstest]
    #[case::a_median_of_zero(40.0, 0.0)]
    #[case::a_value_of_zero(0.0, 20.0)]
    fn a_ratio_that_is_not_positive_yields_no_deviation(
        #[case] value: f64,
        #[case] background: f64,
    ) {
        assert_eq!(
            deviation_from_quiet_time(tecu(value), &quiet_window(27, background)),
            None
        );
    }

    /// An even count takes the mean of the two middle values.
    #[test]
    fn an_even_window_medians_between_its_middle_two() {
        let window = [
            quiet_window(7, 10.0).as_slice(),
            quiet_window(7, 30.0).as_slice(),
        ]
        .concat();

        let deviation =
            deviation_from_quiet_time(tecu(30.0), &window).expect("a fully archived window");

        assert!(
            (deviation.percent_from_median() - 50.0).abs() < 1e-9,
            "{deviation:?}"
        );
    }
}
