//! Classification against what the host really answered.
//!
//! The unit tests script synthetic statuses. These replay the captured
//! bodies and statuses from `capture.json`, so the served day and the
//! refused day are classified from real responses.

mod support;

use std::cell::RefCell;

use serde_json::Value;

use gt_fetch::{HttpRequest, HttpResponse, Transport, TransportError};
use gt_jam::transport::{self, FetchOutcome};
use gt_jam::wire::{self, ParseWarningReporter};
use gt_jam::{DEFAULT_BASE_URL, dataset_url, parse_day};

/// Answers every request with one captured response, recording the URLs.
struct FixtureTransport {
    response: HttpResponse,
    urls: RefCell<Vec<String>>,
}

impl FixtureTransport {
    fn new(status: u16, body: String) -> Self {
        Self {
            response: HttpResponse { status, body },
            urls: RefCell::new(Vec::new()),
        }
    }

    fn urls(&self) -> Vec<String> {
        self.urls.borrow().clone()
    }
}

impl Transport for FixtureTransport {
    fn send(&self, request: &HttpRequest) -> Result<HttpResponse, TransportError> {
        self.urls.borrow_mut().push(request.url().to_owned());
        Ok(self.response.clone())
    }
}

/// The captured world day classifies as served, and its body parses.
#[test]
fn the_captured_day_is_served_and_parses() {
    let fixture = support::served_day().unwrap();
    let csv = support::captured_csv(fixture.day).unwrap();
    let day = parse_day(fixture.day).unwrap();

    let transport = FixtureTransport::new(fixture.http_status, csv.clone());
    let outcome = transport::fetch_day(&transport, DEFAULT_BASE_URL, day);

    assert_eq!(outcome, FetchOutcome::Served(csv.clone()));
    assert_eq!(transport.urls(), [dataset_url(DEFAULT_BASE_URL, day)]);

    let reporter = ParseWarningReporter::default();
    let observations = wire::parse_dataset(&csv, &reporter).unwrap();
    assert!(reporter.is_empty());
    assert!(!observations.is_empty());
}

/// The captured refusal classifies as missing, from the host's own status
/// and body.
#[test]
fn the_captured_refusal_is_missing() {
    let fixture = support::refused_day().unwrap();
    let entry = support::manifest_entry(fixture.day).unwrap();
    let body = entry
        .get("body")
        .and_then(Value::as_str)
        .expect("a refused day records its body");
    let day = parse_day(fixture.day).unwrap();

    let transport = FixtureTransport::new(fixture.http_status, body.to_owned());
    let outcome = transport::fetch_day(&transport, DEFAULT_BASE_URL, day);

    assert_eq!(outcome, FetchOutcome::Missing);
    assert_eq!(transport.urls().len(), 1, "a refusal is deterministic");
}
