//! Classification against what the host really returned.
//!
//! The unit tests script synthetic statuses. These replay the captured
//! bodies and statuses from `capture.json`, so the served day and the
//! refused day are classified from real responses.

use serde_json::Value;

use gt_fetch::test_util::{self as scripted_transport, ScriptedTransport};
use gt_jam::test_util;
use gt_jam::transport::{self, FetchOutcome};
use gt_jam::wire::{self, ParseWarningReporter};
use gt_jam::{DEFAULT_BASE_URL, dataset_url, parse_day};

/// The captured world day classifies as served, and its body parses.
#[test]
fn the_captured_day_is_served_and_parses() {
    let fixture = test_util::served_day().unwrap();
    let csv = test_util::captured_csv(fixture.day).unwrap();
    let day = parse_day(fixture.day).unwrap();

    let transport = ScriptedTransport::always(scripted_transport::response(
        fixture.http_status,
        csv.clone(),
    ));
    let outcome = transport::fetch_day(&transport, DEFAULT_BASE_URL, day);

    assert_eq!(outcome, FetchOutcome::Served(csv.clone()));
    assert_eq!(
        transport.requested_urls(),
        [dataset_url(DEFAULT_BASE_URL, day)]
    );

    let reporter = ParseWarningReporter::default();
    let observations = wire::parse_dataset(&csv, &reporter).unwrap();
    assert!(reporter.is_empty());
    assert!(!observations.is_empty());
}

/// The captured refusal from the host classifies as missing, from its own
/// status and body.
#[test]
fn the_captured_refusal_is_missing() {
    let fixture = test_util::refused_day().unwrap();
    let entry = test_util::manifest_entry(fixture.day).unwrap();
    let body = entry
        .get("body")
        .and_then(Value::as_str)
        .expect("a refused day records its body");
    let day = parse_day(fixture.day).unwrap();

    let transport =
        ScriptedTransport::always(scripted_transport::response(fixture.http_status, body));
    let outcome = transport::fetch_day(&transport, DEFAULT_BASE_URL, day);

    assert_eq!(outcome, FetchOutcome::Missing);
    assert_eq!(
        transport.requested_urls().len(),
        1,
        "a refusal is deterministic"
    );
}
