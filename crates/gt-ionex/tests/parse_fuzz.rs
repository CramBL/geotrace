//! The parser and the interpolation against uncurated input.
//!
//! [`gt_ionex::parse`] is the crate's only consumer of files it did not
//! write, which the hand-written unit tests cannot cover on their own. Every
//! property asserts the same thing: no input panics, and what survives holds
//! only finite values within the ones the file writes. Mirrors how `gt-solar`
//! fuzzes its wire parser.

mod support;

use std::sync::OnceLock;

use chrono::TimeDelta;
use proptest::test_runner::TestCaseError;

use gt_ionex::maps::{GlobalIonosphereMaps, TecMap};
use gt_ionex::parse;
use gt_ionex::tec::TotalElectronContent;
use gt_types::{Latitude, Longitude};

/// How far into a capture the truncation property cuts: past the header and
/// the first maps, where the structural work is.
const MAX_TRUNCATION_BYTES: usize = 60_000;

/// How many lines into a capture the rewriting property reaches, which covers
/// its header and its first map.
const MAX_REWRITTEN_LINE: usize = 400;

/// The values a capture holds, which no interpolated value may leave.
#[derive(Debug, Clone, Copy)]
struct ValueRange {
    lowest_tecu: f64,
    highest_tecu: f64,
}

/// The capture the truncation and query properties read, kept for the whole
/// run.
fn captured_storm() -> Result<&'static str, String> {
    static TEXT: OnceLock<Result<String, String>> = OnceLock::new();
    TEXT.get_or_init(|| support::captured_text(support::declared_fixture(support::STORM_CAPTURE)?))
        .as_deref()
        .map_err(Clone::clone)
}

fn parsed_storm() -> Result<&'static GlobalIonosphereMaps, String> {
    static MAPS: OnceLock<Result<GlobalIonosphereMaps, String>> = OnceLock::new();
    MAPS.get_or_init(|| support::captured_maps(support::STORM_CAPTURE))
        .as_ref()
        .map_err(Clone::clone)
}

fn storm_value_range() -> Result<ValueRange, String> {
    static RANGE: OnceLock<Result<ValueRange, String>> = OnceLock::new();
    RANGE
        .get_or_init(|| {
            let values: Vec<f64> = parsed_storm()?
                .maps()
                .iter()
                .flat_map(TecMap::values)
                .flatten()
                .map(TotalElectronContent::tecu)
                .collect();
            Ok(ValueRange {
                lowest_tecu: values.iter().copied().fold(f64::INFINITY, f64::min),
                highest_tecu: values.iter().copied().fold(f64::NEG_INFINITY, f64::max),
            })
        })
        .clone()
}

/// What the parser promises: every value it keeps is a finite number.
fn check_values(maps: &GlobalIonosphereMaps) -> Result<(), TestCaseError> {
    for value in maps.maps().iter().flat_map(TecMap::values).flatten() {
        if !value.tecu().is_finite() {
            return Err(TestCaseError::fail(format!("{value:?} is not a number")));
        }
    }
    Ok(())
}

proptest::proptest! {
    /// Any input at all. Most cases are refused for having no header end.
    #[test]
    fn arbitrary_input_yields_only_finite_values(text in ".{0,2048}") {
        if let Ok(maps) = parse::global_ionosphere_maps(&text) {
            check_values(&maps)?;
        }
    }

    /// A dropped connection or a half-written cache entry.
    #[test]
    fn a_truncated_capture_yields_only_finite_values(cut in 0..MAX_TRUNCATION_BYTES) {
        let text = captured_storm().map_err(TestCaseError::fail)?;
        // The capture is ASCII, so every byte is a character boundary. `get`
        // checks it.
        if let Some(truncated) = text.get(..cut)
            && let Ok(maps) = parse::global_ionosphere_maps(truncated)
        {
            check_values(&maps)?;
        }
    }

}

// Every case of this property rebuilds and reparses a whole capture, so it
// runs fewer of them than the default.
proptest::proptest! {
    #![proptest_config(proptest::test_runner::Config { cases: 64, ..proptest::test_runner::Config::default() })]

    /// A capture with one line rewritten, which reaches the record readers
    /// far more often than arbitrary text does.
    #[test]
    fn a_capture_with_a_rewritten_line_is_read_or_refused(
        rewritten_line in 0_usize..MAX_REWRITTEN_LINE,
        replacement in ".{0,80}",
    ) {
        let text = captured_storm().map_err(TestCaseError::fail)?;
        let rewritten: String = text
            .lines()
            .enumerate()
            .map(|(number, line)| {
                if number == rewritten_line {
                    format!("{replacement}\n")
                } else {
                    format!("{line}\n")
                }
            })
            .collect();
        if let Ok(maps) = parse::global_ionosphere_maps(&rewritten) {
            check_values(&maps)?;
        }
    }
}

proptest::proptest! {
    /// Any position and time a caller can ask for, against a whole capture.
    #[test]
    fn any_position_and_time_resolves_within_the_captured_values(
        latitude in -90.0_f64..=90.0,
        longitude in -180.0_f64..=180.0,
        seconds_from_the_first_epoch in -86_400_i64..172_800,
    ) {
        let maps = parsed_storm().map_err(TestCaseError::fail)?;
        let range = storm_value_range().map_err(TestCaseError::fail)?;
        let first_epoch = maps
            .epoch_of_first_map()
            .ok_or_else(|| TestCaseError::fail("the capture holds no maps".to_owned()))?;
        let value = maps.total_electron_content_at(
            Latitude::new(latitude),
            Longitude::new(longitude),
            first_epoch + TimeDelta::seconds(seconds_from_the_first_epoch),
        );
        if let Some(value) = value {
            proptest::prop_assert!(
                (range.lowest_tecu..=range.highest_tecu).contains(&value.tecu()),
                "{value:?} is outside the values the capture holds"
            );
        }
    }

    /// Times outside the captured day never resolve, whatever the position.
    #[test]
    fn a_time_outside_the_capture_never_resolves(
        latitude in -90.0_f64..=90.0,
        longitude in -180.0_f64..=180.0,
        seconds_past_the_edge in 1_i64..86_400,
    ) {
        let maps = parsed_storm().map_err(TestCaseError::fail)?;
        let first_epoch = maps
            .epoch_of_first_map()
            .ok_or_else(|| TestCaseError::fail("the capture holds no maps".to_owned()))?;
        let last_epoch = maps
            .epoch_of_last_map()
            .ok_or_else(|| TestCaseError::fail("the capture holds no maps".to_owned()))?;
        let outside = TimeDelta::seconds(seconds_past_the_edge);
        for time in [first_epoch - outside, last_epoch + outside] {
            proptest::prop_assert_eq!(
                maps.total_electron_content_at(
                    Latitude::new(latitude),
                    Longitude::new(longitude),
                    time,
                ),
                None
            );
        }
    }
}
