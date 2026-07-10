//! Validate the committed live-API fixtures.
//!
//! Guards the contract between [`gt_snap::FIXTURE_SCENARIOS`], the capture
//! harness (`examples/fetch_fixtures.rs`), and the files under
//! `tests/fixtures/`: every scenario has its request/response pair, nothing
//! stray lingers after a scenario rename, and the captured statuses stay
//! pinned so a re-capture that changes server behavior fails loudly instead
//! of slipping through review.

use std::collections::BTreeSet;
use std::fs;

use serde_json::Value;

use gt_snap::{DEFAULT_SERVER_URL, FIXTURE_SCENARIOS, fixtures_dir};

/// The one scenario whose response is deliberately not JSON: the reverse
/// proxy's HTML 413 page.
const HTML_RESPONSE_SCENARIO: &str = "too_large_body";

fn read_fixture(name: &str) -> Result<String, String> {
    let path = fixtures_dir().join(name);
    fs::read_to_string(&path).map_err(|err| format!("reading {}: {err}", path.display()))
}

fn parse_json(name: &str) -> Result<Value, String> {
    serde_json::from_str(&read_fixture(name)?).map_err(|err| format!("{name}: {err}"))
}

#[test]
fn every_scenario_has_a_valid_pair() {
    for &scenario in FIXTURE_SCENARIOS {
        let request = parse_json(&format!("{scenario}.request.json")).expect("request fixture");
        assert!(
            request.is_object(),
            "{scenario} request must be a JSON object"
        );

        let response =
            read_fixture(&format!("{scenario}.response.json")).expect("response fixture");
        if scenario == HTML_RESPONSE_SCENARIO {
            assert!(
                serde_json::from_str::<Value>(&response).is_err(),
                "{scenario} exists to pin a non-JSON error body, but it parsed as JSON - \
                 re-point the scenario at whatever non-JSON failure the server now produces"
            );
            assert!(
                response.contains("413"),
                "{scenario} response no longer looks like the proxy's 413 page"
            );
        } else {
            parse_json(&format!("{scenario}.response.json")).expect("response fixture JSON");
        }
    }
}

#[test]
fn fixture_dir_matches_scenario_list_exactly() {
    let mut expected: BTreeSet<String> = FIXTURE_SCENARIOS
        .iter()
        .flat_map(|s| [format!("{s}.request.json"), format!("{s}.response.json")])
        .collect();
    expected.insert("capture.json".to_owned());

    let actual: BTreeSet<String> = fs::read_dir(fixtures_dir())
        .expect("fixtures dir must exist - run `just snap-fixtures` once")
        .map(|entry| entry.expect("readable dir entry").file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect();

    assert_eq!(
        expected, actual,
        "fixture files and FIXTURE_SCENARIOS drifted apart - \
         after renaming or removing a scenario, delete its stale pair"
    );
}

#[test]
fn capture_metadata_pins_server_and_statuses() {
    let capture = parse_json("capture.json").expect("capture metadata");
    assert_eq!(
        capture["server"], DEFAULT_SERVER_URL,
        "fixtures must be captured from the default server, not a local override"
    );

    let scenarios = capture["scenarios"]
        .as_array()
        .expect("capture.json scenarios array");
    let statuses: Vec<(String, u64)> = scenarios
        .iter()
        .map(|s| {
            (
                s["name"].as_str().unwrap_or_default().to_owned(),
                s["http_status"].as_u64().unwrap_or_default(),
            )
        })
        .collect();

    // Pins scenario order and the HTTP status each behavior produces. A
    // re-capture that changes either is a server behavior change and must be
    // reviewed deliberately.
    insta::assert_debug_snapshot!(statuses);
}
