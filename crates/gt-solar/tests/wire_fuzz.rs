//! The parsers against uncurated input.
//!
//! [`gt_solar::wire`] is the crate's only consumer of untrusted network
//! bytes, which the unit tests' hand-written responses cannot cover.
//!
//! Every property asserts the same thing: no input panics a parser, and what
//! survives holds only values inside the index's published range. Mirrors how
//! `gt-jam` fuzzes its dataset parser.

mod support;

use std::sync::OnceLock;

use proptest::test_runner::TestCaseError;

use gt_solar::activity::{GeomagneticActivity, KP_MAX_VALUE, MIN_VALUE};
use gt_solar::series::{Hp30Series, KpSeries};
use gt_solar::wire;

/// How far into the captured storm the truncation property cuts: past the end
/// of the largest capture, so a whole response is reachable too.
const MAX_TRUNCATION_BYTES: usize = 4096;

/// The capture the truncation property cuts, read once for the whole run.
fn captured_storm() -> Result<&'static str, String> {
    static JSON: OnceLock<Result<String, String>> = OnceLock::new();
    JSON.get_or_init(|| support::captured_response(support::declared_window("hp30-storm")?))
        .as_deref()
        .map_err(Clone::clone)
}

/// What both parsers promise: a sample's value is one its index publishes.
fn check_values(
    values: impl Iterator<Item = GeomagneticActivity>,
    ceiling: f64,
) -> Result<(), TestCaseError> {
    for activity in values {
        if !(MIN_VALUE..=ceiling).contains(&activity.value()) {
            return Err(TestCaseError::fail(format!(
                "{activity} is outside the published range"
            )));
        }
    }
    Ok(())
}

fn check_kp(series: &KpSeries) -> Result<(), TestCaseError> {
    check_values(
        series.samples.iter().filter_map(|sample| sample.activity),
        KP_MAX_VALUE,
    )
}

fn check_hp30(series: &Hp30Series) -> Result<(), TestCaseError> {
    check_values(
        series.samples.iter().filter_map(|sample| sample.activity),
        f64::MAX,
    )
}

/// Timestamps that reach the parser's deeper checks as often as they fail at
/// the first one.
fn timestamp_strategy() -> impl proptest::strategy::Strategy<Value = String> {
    proptest::prop_oneof![
        proptest::strategy::Just("2024-05-10T00:00:00Z".to_owned()),
        "[0-9TZ:+-]{0,25}",
    ]
}

proptest::proptest! {
    /// Any input at all. Most cases are rejected as not being JSON.
    #[test]
    fn arbitrary_input_yields_only_published_values(json in ".{0,2048}") {
        if let Ok(series) = wire::parse_kp_series(&json) {
            check_kp(&series)?;
        }
        if let Ok(series) = wire::parse_hp30_series(&json) {
            check_hp30(&series)?;
        }
    }

    /// The published shape with arbitrary arrays in it, so cases land past
    /// the JSON check.
    #[test]
    fn arbitrary_arrays_in_the_published_shape_yield_only_published_values(
        values in proptest::collection::vec(proptest::option::of(-20.0f64..20.0), 0..16),
        timestamps in proptest::collection::vec(timestamp_strategy(), 0..16),
        statuses in proptest::collection::vec("[a-z]{0,4}", 0..16),
    ) {
        let json = serde_json::json!({
            "Kp": &values,
            "Hp30": &values,
            "datetime": &timestamps,
            "status": &statuses,
            "meta": {"license": "CC BY 4.0", "source": "GFZ Potsdam"},
        })
        .to_string();

        if let Ok(series) = wire::parse_kp_series(&json) {
            proptest::prop_assert_eq!(series.samples.len(), values.len());
            check_kp(&series)?;
        }
        if let Ok(series) = wire::parse_hp30_series(&json) {
            proptest::prop_assert_eq!(series.samples.len(), values.len());
            check_hp30(&series)?;
        }
    }

    /// A dropped connection or a truncated cache entry.
    #[test]
    fn a_truncated_capture_yields_only_published_values(cut in 0..MAX_TRUNCATION_BYTES) {
        let json = captured_storm().map_err(TestCaseError::fail)?;
        // The capture is ASCII, so every byte is a character boundary. `get`
        // checks it.
        if let Some(truncated) = json.get(..cut)
            && let Ok(series) = wire::parse_hp30_series(truncated)
        {
            check_hp30(&series)?;
        }
    }
}
