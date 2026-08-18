//! Validate the committed response captures.
//!
//! Guards [`gt_flare::FIXTURE_WINDOWS`], the capture harness
//! (`examples/fetch_flare_fixtures.rs`), and the files under `tests/fixtures/`
//! against each other, and checks the captures are still the shape the parser
//! is written for.

mod support;

use std::collections::BTreeSet;

use serde_json::Value;

use gt_flare::class::{FlareClass, RadioBlackoutClass};
use gt_flare::wire;
use gt_flare::{FIXTURE_WINDOWS, FixtureWindow, SolarFlare};

/// The May 2024 storm.
const STORM_CAPTURE: &str = "storm-may-2024";

/// Solar minimum, where the catalog closed off no end time.
const QUIET_CAPTURE: &str = "quiet-january-2019";

/// The year before the catalog begins.
const BEFORE_COVERAGE_CAPTURE: &str = "before-coverage";

const HTTP_OK: u64 = 200;

fn parse_capture(fixture: &FixtureWindow) -> Result<Vec<SolarFlare>, String> {
    let json = support::captured_response(fixture)?;
    wire::parse_flares(&json).map_err(|err| format!("{}: {err}", fixture.name))
}

fn captured_flares(name: &str) -> Result<Vec<SolarFlare>, String> {
    parse_capture(support::declared_window(name)?)
}

/// The manifest agrees with what each window declares.
#[test]
fn every_declared_window_has_a_matching_manifest_entry() {
    for fixture in FIXTURE_WINDOWS {
        let entry = support::manifest_entry(fixture.name).unwrap();
        assert_eq!(
            entry.get("start").and_then(Value::as_str),
            Some(fixture.start),
            "{}: the capture requested another window",
            fixture.name
        );
        assert_eq!(
            entry.get("end").and_then(Value::as_str),
            Some(fixture.end),
            "{}: the capture requested another window",
            fixture.name
        );
        assert!(
            entry
                .get("captured_at")
                .and_then(Value::as_str)
                .is_some_and(|captured_at| !captured_at.is_empty()),
            "{} has no capture date",
            fixture.name
        );
    }
}

/// No entry survives a dropped window, and no window is captured undeclared.
#[test]
fn the_manifest_lists_exactly_the_declared_windows() {
    let declared: BTreeSet<&str> = FIXTURE_WINDOWS.iter().map(|fixture| fixture.name).collect();
    let recorded: Vec<String> = support::manifest_entries()
        .unwrap()
        .iter()
        .filter_map(|entry| Some(entry.get("name")?.as_str()?.to_owned()))
        .collect();
    let recorded: BTreeSet<&str> = recorded.iter().map(String::as_str).collect();
    assert_eq!(declared, recorded);
}

/// The key is a secret, so nothing the capture writes down may hold the URL
/// it was sent in.
#[test]
fn no_capture_records_a_url() {
    let manifest = support::manifest().unwrap().to_string();
    assert!(!manifest.contains("api_key"), "{manifest}");
    assert!(!manifest.contains("DONKI/FLR"), "{manifest}");
    for fixture in FIXTURE_WINDOWS {
        let json = support::captured_response(&fixture).unwrap();
        assert!(!json.contains("api_key"), "{}", fixture.name);
    }
}

#[test]
fn every_capture_parses_into_the_recorded_number_of_flares() {
    for fixture in FIXTURE_WINDOWS {
        let recorded = support::manifest_entry(fixture.name)
            .unwrap()
            .get("flares")
            .and_then(Value::as_u64)
            .and_then(|flares| usize::try_from(flares).ok())
            .expect("a capture records its flare count");
        assert_eq!(
            parse_capture(&fixture).unwrap().len(),
            recorded,
            "{}: the file on disk is not the one the manifest describes",
            fixture.name
        );
    }
}

/// Every flare begins inside the requested window and peaks after it began,
/// and the events come back in peak order.
#[test]
fn every_capture_falls_inside_the_requested_window_in_peak_order() {
    for fixture in FIXTURE_WINDOWS {
        let window = fixture.window().unwrap();
        let mut previous_peak = None;
        for flare in parse_capture(&fixture).unwrap() {
            assert!(
                (window.start..=window.end).contains(&flare.begin_day()),
                "{}: {} begins outside the requested window",
                fixture.name,
                flare.id
            );
            assert!(
                flare.peak >= flare.begin,
                "{}: {} peaks before it begins",
                fixture.name,
                flare.id
            );
            assert!(
                previous_peak.is_none_or(|previous| previous <= flare.peak),
                "{}: {} is out of peak order",
                fixture.name,
                flare.id
            );
            previous_peak = Some(flare.peak);
        }
    }
}

/// The strongest flare of the storm window, which is what the marker colour
/// and the blackout wording are read off.
#[test]
fn the_captured_storm_holds_x_class_flares() {
    let flares = captured_flares(STORM_CAPTURE).unwrap();
    let strongest = flares
        .iter()
        .map(|flare| flare.classification)
        .max()
        .expect("the storm window lists flares");
    assert_eq!(strongest.to_string(), "X5.8");
    assert_eq!(
        strongest.radio_blackout_class(),
        Some(RadioBlackoutClass::Strong)
    );
    assert_eq!(
        flares
            .iter()
            .filter(|flare| flare.classification.class() == FlareClass::X)
            .count(),
        5
    );
}

/// The catalog lists a flare with no active region, which is the field the
/// archive keeps a presence column for.
#[test]
fn the_captured_storm_holds_a_flare_without_an_active_region() {
    let flares = captured_flares(STORM_CAPTURE).unwrap();
    assert!(
        flares.iter().any(|flare| flare.active_region.is_none()),
        "no flare in the storm window is missing its active region"
    );
    assert!(
        flares.iter().all(|flare| flare.source_location.is_some()),
        "the storm window's flares all have a source location"
    );
}

/// Solar minimum: weak flares, and neither of them closed off.
#[test]
fn the_captured_quiet_month_holds_c_class_flares_without_end_times() {
    let flares = captured_flares(QUIET_CAPTURE).unwrap();
    assert!(!flares.is_empty());
    for flare in &flares {
        assert_eq!(flare.classification.class(), FlareClass::C, "{}", flare.id);
        assert_eq!(flare.end, None, "{}", flare.id);
        assert_eq!(flare.classification.radio_blackout_class(), None);
    }
}

#[test]
fn a_window_before_the_catalog_begins_is_captured_as_an_empty_array() {
    let fixture = support::declared_window(BEFORE_COVERAGE_CAPTURE).unwrap();
    assert!(parse_capture(fixture).unwrap().is_empty());
    assert_eq!(
        support::manifest_entry(fixture.name)
            .unwrap()
            .get("http_status")
            .and_then(Value::as_u64),
        Some(HTTP_OK),
        "a window before coverage is served, not refused"
    );
}
