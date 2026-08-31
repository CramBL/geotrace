//! The parser against uncurated input.
//!
//! [`gt_flare::wire`] is the crate's only consumer of untrusted network
//! bytes, which the unit tests' hand-written responses cannot cover.
//!
//! Every property asserts the same thing: no input panics the parser, and
//! what survives holds only events the catalog could publish. Mirrors how
//! `gt-solar` fuzzes its series parser.

mod support;

use std::sync::OnceLock;

use proptest::test_runner::TestCaseError;

use gt_flare::SolarFlare;
use gt_flare::wire;

/// How far into the captured storm the truncation property cuts: past the end
/// of the largest capture, so a whole response is reachable too.
const MAX_TRUNCATION_BYTES: usize = 32_768;

/// The capture the truncation property cuts, read once for the whole run.
fn captured_storm() -> Result<&'static str, String> {
    static JSON: OnceLock<Result<String, String>> = OnceLock::new();
    JSON.get_or_init(|| support::captured_response(support::declared_window("storm-may-2024")?))
        .as_deref()
        .map_err(Clone::clone)
}

/// What the parser promises: every event carries a classification the scale
/// defines, a peak at or after its beginning, and the events run in peak
/// order.
fn check_flares(flares: &[SolarFlare]) -> Result<(), TestCaseError> {
    let mut previous_peak = None;
    for flare in flares {
        let magnitude = flare.classification.magnitude();
        if !magnitude.is_finite() || magnitude < 1.0 {
            return Err(TestCaseError::fail(format!(
                "{} is classified {magnitude}",
                flare.id
            )));
        }
        if previous_peak.is_some_and(|previous| previous > flare.peak) {
            return Err(TestCaseError::fail(format!("{} is out of order", flare.id)));
        }
        previous_peak = Some(flare.peak);
    }
    Ok(())
}

/// Times that reach the parser's deeper checks as often as they fail at the
/// first one.
fn time_strategy() -> impl proptest::strategy::Strategy<Value = String> {
    proptest::prop_oneof![
        proptest::strategy::Just("2024-05-09T00:58Z".to_owned()),
        "[0-9TZ:+-]{0,25}",
    ]
}

/// Class strings that land on a published class as often as they are rejected.
fn class_strategy() -> impl proptest::strategy::Strategy<Value = String> {
    proptest::prop_oneof![
        proptest::strategy::Just("X2.2".to_owned()),
        proptest::strategy::Just("M1.8".to_owned()),
        "[A-Za-z0-9. -]{0,8}",
    ]
}

proptest::proptest! {
    /// Any input at all. Most cases are rejected as not being JSON.
    #[test]
    fn arbitrary_input_yields_only_publishable_events(json in ".{0,2048}") {
        if let Ok(flares) = wire::parse_flares(&json) {
            check_flares(&flares)?;
        }
    }

    /// The published shape with arbitrary fields in it, so cases land past
    /// the JSON check.
    #[test]
    fn arbitrary_fields_in_the_published_shape_yield_only_publishable_events(
        ids in proptest::collection::vec("[A-Za-z0-9:.-]{0,32}", 0..8),
        begins in proptest::collection::vec(time_strategy(), 0..8),
        peaks in proptest::collection::vec(time_strategy(), 0..8),
        ends in proptest::collection::vec(proptest::option::of(time_strategy()), 0..8),
        classes in proptest::collection::vec(class_strategy(), 0..8),
        regions in proptest::collection::vec(proptest::option::of(-10i64..100_000), 0..8),
    ) {
        let events: Vec<serde_json::Value> = (0..ids.len())
            .map(|index| {
                serde_json::json!({
                    "flrID": ids.get(index),
                    "beginTime": begins.get(index),
                    "peakTime": peaks.get(index),
                    "endTime": ends.get(index).cloned().flatten(),
                    "classType": classes.get(index),
                    "sourceLocation": "S20W19",
                    "activeRegionNum": regions.get(index).copied().flatten(),
                })
            })
            .collect();

        if let Ok(flares) = wire::parse_flares(&serde_json::Value::Array(events).to_string()) {
            proptest::prop_assert_eq!(flares.len(), ids.len());
            check_flares(&flares)?;
        }
    }

    /// A dropped connection or a truncated cache entry.
    #[test]
    fn a_truncated_capture_yields_only_publishable_events(cut in 0..MAX_TRUNCATION_BYTES) {
        let json = captured_storm().map_err(TestCaseError::fail)?;
        // The capture is ASCII, so every byte is a character boundary. `get`
        // checks it.
        if let Some(truncated) = json.get(..cut)
            && let Ok(flares) = wire::parse_flares(truncated)
        {
            check_flares(&flares)?;
        }
    }
}
