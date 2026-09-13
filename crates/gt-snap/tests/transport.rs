//! Validate plan sending and outcome classification with a scripted transport.
//!
//! No network: the scripted transport replays captured response bodies and
//! synthetic statuses, exercising the same classification path production
//! uses (static dispatch through the `Transport` trait).

use gt_fetch::TransportSource;
use gt_fetch::test_util::{self as fetch_test_util, ScriptedTransport, TransportResponse};
use gt_snap::merge::{ChunkOutcome, SnapWarningReporter};
use gt_snap::request_plan::{CHUNK_POINTS, SnapParams};
use gt_snap::wire::Costing;
use gt_snap::{DEFAULT_SERVER_URL, test_util, transport};

fn ok(body: String) -> TransportResponse<String> {
    fetch_test_util::response(200, body)
}

fn status(code: u16, body: &str) -> TransportResponse<String> {
    fetch_test_util::response(code, body)
}

fn connection_reset() -> TransportResponse<String> {
    fetch_test_util::transport_error("connection reset")
}

#[test]
fn captured_success_body_classifies_and_merges_end_to_end() {
    // The captured `partially_snappable` response has 20 matched points, so a
    // 20-point plan is one chunk.
    let plan = test_util::plan_of(&test_util::points(20));
    let transport = ScriptedTransport::in_order(vec![ok(test_util::read_capture(
        "partially_snappable.response.json",
    )
    .expect("capture"))]);

    let mut progress = Vec::new();
    let outcomes = transport::send_plan(
        &transport,
        DEFAULT_SERVER_URL,
        &plan,
        &SnapParams::new(Costing::Auto),
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
    gt_snap::merge::merge(plan, SnapParams::new(Costing::Auto), outcomes, reporter)
}

#[test]
fn off_network_error_becomes_off_network_outcome_without_retry() {
    let plan = test_util::plan_of(&test_util::points(10));
    let transport = ScriptedTransport::in_order(vec![status(
        400,
        &test_util::read_capture("unsnappable.response.json").expect("capture"),
    )]);

    let outcomes = transport::send_plan(
        &transport,
        DEFAULT_SERVER_URL,
        &plan,
        &SnapParams::new(Costing::Auto),
        |_, _| {},
    );

    assert_eq!(outcomes, vec![ChunkOutcome::OffNetwork]);
    assert_eq!(transport.sends(), 1, "4xx is never retried");
}

#[test]
fn deterministic_client_error_fails_without_retry() {
    let plan = test_util::plan_of(&test_util::points(10));
    let transport = ScriptedTransport::in_order(vec![status(
        400,
        &test_util::read_capture("bad_request.response.json").expect("capture"),
    )]);

    let outcomes = transport::send_plan(
        &transport,
        DEFAULT_SERVER_URL,
        &plan,
        &SnapParams::new(Costing::Auto),
        |_, _| {},
    );

    assert!(
        matches!(outcomes.first(), Some(ChunkOutcome::Failed(detail)) if detail.contains("114"))
    );
    assert_eq!(transport.sends(), 1);
}

#[test]
fn html_error_body_fails_without_retry() {
    let plan = test_util::plan_of(&test_util::points(10));
    let transport = ScriptedTransport::in_order(vec![status(
        413,
        &test_util::read_capture("too_large_body.response.json").expect("capture"),
    )]);

    let outcomes = transport::send_plan(
        &transport,
        DEFAULT_SERVER_URL,
        &plan,
        &SnapParams::new(Costing::Auto),
        |_, _| {},
    );

    assert!(
        matches!(outcomes.first(), Some(ChunkOutcome::Failed(detail)) if detail.contains("non-JSON"))
    );
    assert_eq!(transport.sends(), 1);
}

#[test]
fn transient_transport_failure_gets_one_retry_then_succeeds() {
    let plan = test_util::plan_of(&test_util::points(10));
    let transport = ScriptedTransport::in_order(vec![
        connection_reset(),
        ok(test_util::read_capture("clean_drive.response.json").expect("capture")),
    ]);

    let outcomes = transport::send_plan(
        &transport,
        DEFAULT_SERVER_URL,
        &plan,
        &SnapParams::new(Costing::Auto),
        |_, _| {},
    );

    assert!(matches!(outcomes.first(), Some(ChunkOutcome::Success(_))));
    assert_eq!(transport.sends(), 2);
}

#[test]
fn server_error_gets_one_retry_then_fails() {
    let plan = test_util::plan_of(&test_util::points(10));
    let transport = ScriptedTransport::in_order(vec![
        status(503, "upstream overloaded"),
        status(503, "upstream overloaded"),
    ]);

    let outcomes = transport::send_plan(
        &transport,
        DEFAULT_SERVER_URL,
        &plan,
        &SnapParams::new(Costing::Auto),
        |_, _| {},
    );

    assert!(
        matches!(outcomes.first(), Some(ChunkOutcome::Failed(detail)) if detail.contains("503"))
    );
    assert_eq!(transport.sends(), 2, "exactly one retry");
}

#[test]
fn failed_chunk_does_not_stop_later_chunks() {
    let plan = test_util::plan_of(&test_util::points(CHUNK_POINTS + 1));
    assert_eq!(plan.chunks.len(), 2, "precondition");
    let transport = ScriptedTransport::in_order(vec![
        connection_reset(),
        connection_reset(),
        ok(test_util::read_capture("clean_drive.response.json").expect("capture")),
    ]);

    let mut progress = Vec::new();
    let outcomes = transport::send_plan(
        &transport,
        DEFAULT_SERVER_URL,
        &plan,
        &SnapParams::new(Costing::Auto),
        |done, total| {
            progress.push((done, total));
        },
    );

    assert_eq!(progress, vec![(1, 2), (2, 2)]);
    assert!(matches!(outcomes.first(), Some(ChunkOutcome::Failed(_))));
    // The second chunk was still attempted (its scripted success consumed).
    assert_eq!(transport.sends(), 3);
}

#[test]
fn unparsable_success_body_is_a_failure() {
    let plan = test_util::plan_of(&test_util::points(10));
    let transport = ScriptedTransport::in_order(vec![status(200, "not json")]);

    let outcomes = transport::send_plan(
        &transport,
        DEFAULT_SERVER_URL,
        &plan,
        &SnapParams::new(Costing::Auto),
        |_, _| {},
    );

    assert!(
        matches!(outcomes.first(), Some(ChunkOutcome::Failed(detail)) if detail.contains("unparsable success body"))
    );
}

proptest::proptest! {
    /// The classifier consumes untrusted network responses: any status code
    /// crossed with any body must produce outcomes, never a panic. Curated
    /// capture bodies are exercised by the tests above, this covers everything
    /// else, mirroring the shape-decoder fuzz tests.
    #[test]
    fn arbitrary_responses_never_panic(code in proptest::prelude::any::<u16>(), body in ".{0,512}") {
        let plan = test_util::plan_of(&test_util::points(5));
        let transport = ScriptedTransport::in_order(vec![
            status(code, &body),
            status(code, &body), // a transient classification retries once
        ]);
        let outcomes = transport::send_plan(
            &transport,
            DEFAULT_SERVER_URL,
            &plan,
            &SnapParams::new(Costing::Auto),
            |_, _| {},
        );
        proptest::prop_assert_eq!(outcomes.len(), plan.chunks.len());
    }
}

/// Every chunk of an offline run fails. No request is sent, so nothing is
/// classified off-network.
#[test]
fn an_offline_plan_fails_every_chunk() {
    let transport = TransportSource::Offline
        .connect(None)
        .expect("the offline source connects");
    let plan = test_util::plan_of(&test_util::points(10));

    let outcomes = transport::send_plan(
        &transport,
        DEFAULT_SERVER_URL,
        &plan,
        &SnapParams::new(Costing::Auto),
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
