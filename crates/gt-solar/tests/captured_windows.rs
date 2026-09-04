//! Validate the committed response captures.
//!
//! Guards [`gt_solar::FIXTURE_WINDOWS`], the capture harness
//! (`examples/fetch_solar_fixtures.rs`), and the files under `tests/fixtures/`
//! against each other, and checks the captures are still the shape the parser
//! is written for.

mod support;

use std::collections::BTreeSet;

use chrono::{DateTime, Utc};
use serde_json::Value;

use gt_solar::activity::{GeomagneticActivity, GeomagneticActivityClass, GeomagneticStormClass};
use gt_solar::series::KpStatus;
use gt_solar::text;
use gt_solar::wire;
use gt_solar::{FIXTURE_WINDOWS, FixtureWindow, GeomagneticIndex};

/// The day no storm reached.
const QUIET_CAPTURE: &str = "kp-quiet";

/// The May 2024 storm, at both cadences.
const KP_STORM_CAPTURE: &str = "kp-storm";
const HP30_STORM_CAPTURE: &str = "hp30-storm";

/// The window before Hp30 begins.
const BEFORE_COVERAGE_CAPTURE: &str = "hp30-before-coverage";

const HTTP_OK: u64 = 200;

/// A capture, reduced to what these tests check.
struct CapturedSeries {
    period_starts: Vec<DateTime<Utc>>,
    peak: Option<GeomagneticActivity>,
    kp_statuses: Vec<KpStatus>,
}

fn parse_capture(fixture: &FixtureWindow) -> Result<CapturedSeries, String> {
    let json = support::captured_response(fixture)?;
    let capture = match fixture.index {
        GeomagneticIndex::Kp => {
            let series =
                wire::parse_kp_series(&json).map_err(|err| format!("{}: {err}", fixture.name))?;
            CapturedSeries {
                period_starts: series.period_starts().collect(),
                peak: series.peak_activity(),
                kp_statuses: series.samples.iter().map(|sample| sample.status).collect(),
            }
        }
        GeomagneticIndex::Hp30 => {
            let series =
                wire::parse_hp30_series(&json).map_err(|err| format!("{}: {err}", fixture.name))?;
            CapturedSeries {
                period_starts: series.period_starts().collect(),
                peak: series.peak_activity(),
                kp_statuses: Vec::new(),
            }
        }
    };
    Ok(capture)
}

fn peak_activity(name: &str) -> Result<Option<GeomagneticActivity>, String> {
    Ok(parse_capture(support::declared_window(name)?)?.peak)
}

/// The manifest agrees with what each window declares.
#[test]
fn every_declared_window_has_a_matching_manifest_entry() {
    for fixture in FIXTURE_WINDOWS {
        let entry = support::manifest_entry(fixture.name).unwrap();
        assert_eq!(
            entry.get("index").and_then(Value::as_str),
            Some(fixture.index.wire_name()),
            "{}: the capture requested another index than FIXTURE_WINDOWS declares",
            fixture.name
        );
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

#[test]
fn every_capture_parses_into_the_recorded_number_of_samples() {
    for fixture in FIXTURE_WINDOWS {
        let recorded = support::manifest_entry(fixture.name)
            .unwrap()
            .get("samples")
            .and_then(Value::as_u64)
            .and_then(|samples| usize::try_from(samples).ok())
            .expect("a capture records its sample count");
        assert_eq!(
            parse_capture(&fixture).unwrap().period_starts.len(),
            recorded,
            "{}: the file on disk is not the one the manifest describes",
            fixture.name
        );
    }
}

/// Every period starts inside the capture window, one index cadence after
/// the one before it.
#[test]
fn every_capture_runs_at_its_index_cadence_inside_the_requested_window() {
    for fixture in FIXTURE_WINDOWS {
        let window = fixture.window().unwrap();
        let mut previous: Option<DateTime<Utc>> = None;
        for period_start in parse_capture(&fixture).unwrap().period_starts {
            assert!(
                (window.start..=window.end).contains(&period_start),
                "{}: {period_start} is outside the requested window",
                fixture.name
            );
            if let Some(previous) = previous {
                assert_eq!(
                    period_start - previous,
                    fixture.index.period_length(),
                    "{}: {previous} and {period_start} are not one period apart",
                    fixture.name
                );
            }
            previous = Some(period_start);
        }
    }
}

/// The service publishes a status per Kp value and none for Hp30.
#[test]
fn only_kp_captures_carry_a_status_array() {
    for fixture in FIXTURE_WINDOWS {
        let json = support::captured_response(&fixture).unwrap();
        let body: Value = serde_json::from_str(&json).unwrap();
        assert_eq!(
            body.get("status").is_some(),
            fixture.index.publishes_status(),
            "{}: {}",
            fixture.name,
            fixture.purpose
        );
    }
}

/// Published years ago, so every value in these captures is final.
#[test]
fn every_captured_kp_value_is_definitive() {
    for fixture in FIXTURE_WINDOWS {
        let capture = parse_capture(&fixture).unwrap();
        assert!(
            capture
                .kp_statuses
                .iter()
                .all(|status| *status == KpStatus::Definitive),
            "{}: {:?}",
            fixture.name,
            capture.kp_statuses
        );
    }
}

/// Kp tops out at 9 and Hp30 climbs past it, and both land in G5.
#[test]
fn the_captured_storm_reaches_the_top_of_the_scale_at_both_cadences() {
    let kp_peak = peak_activity(KP_STORM_CAPTURE).unwrap();
    let hp30_peak = peak_activity(HP30_STORM_CAPTURE).unwrap();
    assert_eq!(kp_peak.map(GeomagneticActivity::value), Some(9.0));
    assert_eq!(
        hp30_peak.map(GeomagneticActivity::value),
        Some(11.333),
        "Hp30 is not capped at 9"
    );
    for peak in [kp_peak, hp30_peak] {
        assert_eq!(
            peak.and_then(GeomagneticActivity::storm_class),
            Some(GeomagneticStormClass::Extreme)
        );
    }
}

#[test]
fn the_captured_quiet_day_reaches_no_storm_class() {
    assert_eq!(
        peak_activity(QUIET_CAPTURE)
            .unwrap()
            .map(GeomagneticActivity::class),
        Some(GeomagneticActivityClass::Unsettled)
    );
}

#[test]
fn a_window_before_the_index_begins_is_captured_as_an_empty_series() {
    let fixture = support::declared_window(BEFORE_COVERAGE_CAPTURE).unwrap();
    assert!(parse_capture(fixture).unwrap().period_starts.is_empty());
    assert_eq!(
        support::manifest_entry(fixture.name)
            .unwrap()
            .get("http_status")
            .and_then(Value::as_u64),
        Some(HTTP_OK),
        "a window before coverage is served, not refused"
    );
}

/// The attribution the app shows is the one the service publishes with every
/// response.
#[test]
fn every_capture_records_the_services_license_and_source() {
    for fixture in FIXTURE_WINDOWS {
        let entry = support::manifest_entry(fixture.name).unwrap();
        assert_eq!(
            entry.get("license").and_then(Value::as_str),
            Some(text::LICENSE_NAME),
            "{}: the service published another license than the attribution names",
            fixture.name
        );
        assert_eq!(
            entry.get("source").and_then(Value::as_str),
            Some(text::SOURCE_NAME),
            "{}: the service published another source than the attribution names",
            fixture.name
        );
    }
}
