//! Validate plan sending and outcome classification with a canned transport.
//!
//! No network: the canned transport replays captured fixture bodies and
//! synthetic statuses, exercising the same classification path production
//! uses (static dispatch through the `Transport` trait).

mod support;

use std::cell::RefCell;
use std::fs;

use support::points;

use gt_fetch::{HttpRequest, HttpResponse, Transport, TransportError, TransportSource};
use gt_snap::merge::{ChunkOutcome, SnapWarningReporter};
use gt_snap::request_plan::{self, CHUNK_POINTS, SnapParams};
use gt_snap::wire::Costing;
use gt_snap::{DEFAULT_SERVER_URL, fixtures_dir, transport};

/// The params every scenario in this file runs with: default advanced
/// options, auto costing.
fn auto_params() -> SnapParams {
    SnapParams::new(Costing::Auto)
}

fn fixture_body(name: &str) -> Result<String, String> {
    let path = fixtures_dir().join(name);
    fs::read_to_string(&path).map_err(|err| format!("reading {}: {err}", path.display()))
}

/// A canned transport replaying a scripted sequence of results. Panics if
/// the script runs dry (a test sent more requests than it declared).
struct CannedTransport {
    script: RefCell<Vec<Result<HttpResponse, TransportError>>>,
    requests_seen: RefCell<usize>,
}

impl CannedTransport {
    fn new(script: Vec<Result<HttpResponse, TransportError>>) -> Self {
        Self {
            script: RefCell::new(script),
            requests_seen: RefCell::new(0),
        }
    }

    fn requests_seen(&self) -> usize {
        *self.requests_seen.borrow_mut()
    }
}

impl Transport for CannedTransport {
    fn send(&self, _request: &HttpRequest) -> Result<HttpResponse, TransportError> {
        *self.requests_seen.borrow_mut() += 1;
        let mut script = self.script.borrow_mut();
        if script.is_empty() {
            // A dry script means the test under-declared its requests.
            // Surface it as a transport error the assertions will trip on.
            return Err(TransportError {
                detail: "canned transport script ran dry".to_owned(),
            });
        }
        script.remove(0)
    }
}

fn ok(body: String) -> Result<HttpResponse, TransportError> {
    Ok(HttpResponse { status: 200, body })
}

fn status(code: u16, body: &str) -> Result<HttpResponse, TransportError> {
    Ok(HttpResponse {
        status: code,
        body: body.to_owned(),
    })
}

fn connection_reset() -> Result<HttpResponse, TransportError> {
    Err(TransportError {
        detail: "connection reset".to_owned(),
    })
}

#[test]
fn fixture_success_body_classifies_and_merges_end_to_end() {
    // The captured partially_snappable response has 20 matched points, so a
    // 20-point plan is one chunk.
    let plan = request_plan::plan(&points(20));
    let transport = CannedTransport::new(vec![ok(fixture_body(
        "partially_snappable.response.json",
    )
    .expect("fixture"))]);

    let mut progress = Vec::new();
    let outcomes = transport::send_plan(
        &transport,
        DEFAULT_SERVER_URL,
        &plan,
        &auto_params(),
        |done, total| {
            progress.push((done, total));
        },
    );

    assert_eq!(progress, vec![(1, 1)]);
    assert!(matches!(outcomes.first(), Some(ChunkOutcome::Success(_))));

    // The full offline pipeline: plan -> outcomes -> result.
    let reporter = SnapWarningReporter::default();
    let result = merge_all(&plan, &outcomes, &reporter);
    assert_eq!(result.kind_counts.total(), 20);
    assert!(!result.partial);
}

fn merge_all(
    plan: &gt_snap::request_plan::RequestPlan,
    outcomes: &[ChunkOutcome],
    reporter: &SnapWarningReporter,
) -> gt_snap::merge::SnapResult {
    gt_snap::merge::merge(plan, auto_params(), outcomes, reporter)
}

#[test]
fn off_network_error_becomes_off_network_outcome_without_retry() {
    let plan = request_plan::plan(&points(10));
    let transport = CannedTransport::new(vec![status(
        400,
        &fixture_body("unsnappable.response.json").expect("fixture"),
    )]);

    let outcomes = transport::send_plan(
        &transport,
        DEFAULT_SERVER_URL,
        &plan,
        &auto_params(),
        |_, _| {},
    );

    assert_eq!(outcomes, vec![ChunkOutcome::OffNetwork]);
    assert_eq!(transport.requests_seen(), 1, "4xx is never retried");
}

#[test]
fn deterministic_client_error_fails_without_retry() {
    let plan = request_plan::plan(&points(10));
    let transport = CannedTransport::new(vec![status(
        400,
        &fixture_body("bad_request.response.json").expect("fixture"),
    )]);

    let outcomes = transport::send_plan(
        &transport,
        DEFAULT_SERVER_URL,
        &plan,
        &auto_params(),
        |_, _| {},
    );

    assert!(
        matches!(outcomes.first(), Some(ChunkOutcome::Failed(detail)) if detail.contains("114"))
    );
    assert_eq!(transport.requests_seen(), 1);
}

#[test]
fn html_error_body_fails_without_retry() {
    let plan = request_plan::plan(&points(10));
    let transport = CannedTransport::new(vec![status(
        413,
        &fixture_body("too_large_body.response.json").expect("fixture"),
    )]);

    let outcomes = transport::send_plan(
        &transport,
        DEFAULT_SERVER_URL,
        &plan,
        &auto_params(),
        |_, _| {},
    );

    assert!(
        matches!(outcomes.first(), Some(ChunkOutcome::Failed(detail)) if detail.contains("non-JSON"))
    );
    assert_eq!(transport.requests_seen(), 1);
}

#[test]
fn transient_transport_failure_gets_one_retry_then_succeeds() {
    let plan = request_plan::plan(&points(10));
    let transport = CannedTransport::new(vec![
        connection_reset(),
        ok(fixture_body("clean_drive.response.json").expect("fixture")),
    ]);

    let outcomes = transport::send_plan(
        &transport,
        DEFAULT_SERVER_URL,
        &plan,
        &auto_params(),
        |_, _| {},
    );

    assert!(matches!(outcomes.first(), Some(ChunkOutcome::Success(_))));
    assert_eq!(transport.requests_seen(), 2);
}

#[test]
fn server_error_gets_one_retry_then_fails() {
    let plan = request_plan::plan(&points(10));
    let transport = CannedTransport::new(vec![
        status(503, "upstream overloaded"),
        status(503, "upstream overloaded"),
    ]);

    let outcomes = transport::send_plan(
        &transport,
        DEFAULT_SERVER_URL,
        &plan,
        &auto_params(),
        |_, _| {},
    );

    assert!(
        matches!(outcomes.first(), Some(ChunkOutcome::Failed(detail)) if detail.contains("503"))
    );
    assert_eq!(transport.requests_seen(), 2, "exactly one retry");
}

#[test]
fn failed_chunk_does_not_stop_later_chunks() {
    let plan = request_plan::plan(&points(CHUNK_POINTS + 1));
    assert_eq!(plan.chunks.len(), 2, "precondition");
    let transport = CannedTransport::new(vec![
        connection_reset(),
        connection_reset(),
        ok(fixture_body("clean_drive.response.json").expect("fixture")),
    ]);

    let mut progress = Vec::new();
    let outcomes = transport::send_plan(
        &transport,
        DEFAULT_SERVER_URL,
        &plan,
        &auto_params(),
        |done, total| {
            progress.push((done, total));
        },
    );

    assert_eq!(progress, vec![(1, 2), (2, 2)]);
    assert!(matches!(outcomes.first(), Some(ChunkOutcome::Failed(_))));
    // The second chunk was still attempted (its canned success consumed).
    assert_eq!(transport.requests_seen(), 3);
}

#[test]
fn unparsable_success_body_is_a_failure() {
    let plan = request_plan::plan(&points(10));
    let transport = CannedTransport::new(vec![status(200, "not json")]);

    let outcomes = transport::send_plan(
        &transport,
        DEFAULT_SERVER_URL,
        &plan,
        &auto_params(),
        |_, _| {},
    );

    assert!(
        matches!(outcomes.first(), Some(ChunkOutcome::Failed(detail)) if detail.contains("unparsable success body"))
    );
}

proptest::proptest! {
    /// The classifier consumes untrusted network responses: any status code
    /// crossed with any body must produce outcomes, never a panic. Curated
    /// fixture bodies are exercised by the tests above, this covers everything
    /// else, mirroring the shape-decoder fuzz tests.
    #[test]
    fn arbitrary_responses_never_panic(code in proptest::prelude::any::<u16>(), body in ".{0,512}") {
        let plan = request_plan::plan(&points(5));
        let transport = CannedTransport::new(vec![
            status(code, &body),
            status(code, &body), // a transient classification retries once
        ]);
        let outcomes = transport::send_plan(&transport, DEFAULT_SERVER_URL, &plan, &auto_params(), |_, _| {});
        proptest::prop_assert_eq!(outcomes.len(), plan.chunks.len());
    }
}

/// The offline source connects. Every send fails.
#[test]
fn the_offline_source_refuses_every_request() {
    let transport = TransportSource::Offline
        .connect(None)
        .expect("the offline source connects");
    let request = HttpRequest::post_json(DEFAULT_SERVER_URL, "{}");

    let err =
        Transport::<String>::send(&transport, &request).expect_err("offline transport refuses");
    assert!(err.detail.contains(gt_fetch::OFFLINE_DETAIL));
}

/// Every chunk of an offline run fails. No request is sent, so nothing is
/// classified off-network.
#[test]
fn an_offline_plan_fails_every_chunk() {
    let transport = TransportSource::Offline
        .connect(None)
        .expect("the offline source connects");
    let plan = request_plan::plan(&points(10));

    let outcomes = transport::send_plan(
        &transport,
        DEFAULT_SERVER_URL,
        &plan,
        &auto_params(),
        |_, _| {},
    );
    assert_eq!(outcomes.len(), plan.chunks.len());
    assert!(
        outcomes
            .iter()
            .all(|outcome| matches!(outcome, ChunkOutcome::Failed { .. })),
        "every chunk fails offline"
    );
}
