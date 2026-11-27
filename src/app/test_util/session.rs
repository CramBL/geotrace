use std::num::NonZeroUsize;
use std::sync::Arc;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use egui_kittest::Harness;
use gt_session_bridge::test_util::{TestEndpoint, TestSessionReply};
use gt_session_bridge::{
    GetQueryResults, GetRecording, GetTrack, ListRecordings, QueryCounts, QueryJobId,
    QueryJobRequest, QueryJobState, QueryMatch, QueryMatchPosition, QueryPageRequest,
    QueryResultPage, QueryResultRow, QueryResults, QueryText, RecordingDetails, Reply, Request,
    SessionRequest, TrackDetails, TrackReference,
};
use gt_test_utils::HarnessInteraction as _;
use uuid::Uuid;

use crate::app::App;
use crate::app::session_bridge::SessionBridge;

use super::harness;

pub fn app_with_bridge() -> (Harness<'static, App>, Arc<TestEndpoint>) {
    let mut harness = Harness::builder()
        .with_context_setup(gt_ui_theme::install_app_style)
        .with_wait_for_pending_images(false)
        .build_eframe(harness::transient_app);
    let endpoint = self::install_bridge(&mut harness);
    (harness, endpoint)
}

pub fn install_bridge(harness: &mut Harness<'_, App>) -> Arc<TestEndpoint> {
    let endpoint = Arc::new(TestEndpoint::new().unwrap());
    let app = harness.state_mut();
    app.session_bridge_endpoint = Some(Arc::clone(&endpoint));
    app.start_session_bridge();
    assert!(app.session_bridge.server().is_some());
    endpoint
}

pub fn exchange_payload(
    harness: &mut Harness<'_, App>,
    endpoint: &TestEndpoint,
    payload: &[u8],
) -> Vec<u8> {
    thread::scope(|scope| {
        let client = scope.spawn(|| endpoint.exchange_payload(payload).unwrap());
        assert!(harness.step_until(|_| client.is_finished()));
        client.join().unwrap()
    })
}

pub fn exchange_request(
    harness: &mut Harness<'_, App>,
    endpoint: &TestEndpoint,
    request: &Request,
) -> Reply {
    let payload = serde_json::to_vec(request).unwrap();
    serde_json::from_slice(&self::exchange_payload(harness, endpoint, &payload)).unwrap()
}

pub fn exchange_queued_request(
    harness: &mut Harness<'_, App>,
    request: &Request,
    advance_desktop: impl FnOnce(&mut Harness<'_, App>),
) -> Reply {
    assert!(harness.state().session_bridge.server().is_none());
    let endpoint = TestEndpoint::new().unwrap();
    let (ready, queued) = mpsc::sync_channel(1);
    let ctx = harness.ctx.clone();
    harness.state_mut().session_bridge = SessionBridge::Running(
        endpoint
            .start_server(move || {
                ready.try_send(()).unwrap();
                ctx.request_repaint();
            })
            .unwrap(),
    );
    let payload = serde_json::to_vec(request).unwrap();
    let reply = thread::scope(|scope| {
        let client = scope.spawn(|| endpoint.exchange_payload(&payload).unwrap());
        queued.recv_timeout(Duration::from_secs(2)).unwrap();
        advance_desktop(harness);
        client.join().unwrap()
    });
    harness.state_mut().stop_session_bridge();
    serde_json::from_slice(&reply).unwrap()
}

pub fn match_page(page: QueryResultPage) -> (Vec<QueryMatch>, Option<QueryMatchPosition>) {
    let QueryResultPage::Matches {
        matches,
        next_start,
    } = page
    else {
        panic!("expected match page");
    };
    (matches, next_start)
}

pub fn row_page(page: QueryResultPage) -> (Vec<QueryResultRow>, Option<usize>) {
    let QueryResultPage::Rows {
        rows, next_start, ..
    } = page
    else {
        panic!("expected row page");
    };
    (rows, next_start)
}

pub fn first_query_match_request(job: QueryJobId) -> GetQueryResults {
    GetQueryResults {
        job,
        query: 0,
        page: QueryPageRequest::Matches { start: None },
        limit: Some(NonZeroUsize::MIN),
    }
}

pub fn results(harness: &mut Harness<'_, App>, args: GetQueryResults) -> QueryResults {
    let Reply::QueryResults { results } = self::execute(harness, Request::GetQueryResults(args))
    else {
        panic!("expected query results");
    };
    results
}

pub fn start_query(harness: &mut Harness<'_, App>, text: &str) -> QueryJobId {
    let reply = self::execute(
        harness,
        Request::RunQuery(QueryText {
            text: text.to_owned(),
        }),
    );
    let Reply::QueryJob { job } = reply else {
        panic!("expected a running query job, got {reply:?}");
    };
    assert_eq!(job.state, QueryJobState::Preparing);
    job.job
}

pub fn query_status(harness: &mut Harness<'_, App>, job: QueryJobId) -> QueryJobState {
    let Reply::QueryJob { job: result } =
        self::execute(harness, Request::GetQueryStatus(QueryJobRequest { job }))
    else {
        panic!("expected query status");
    };
    assert_eq!(result.job, job);
    result.state
}

pub fn wait_for_query_to_stop(harness: &mut Harness<'_, App>, job: QueryJobId) -> QueryJobState {
    assert!(harness.step_until(|harness| { !harness.state().editor_query.is_active() }));
    self::query_status(harness, job)
}

pub fn execute(harness: &mut Harness<'_, App>, request: Request) -> Reply {
    let mut reply = self::begin(harness, request);
    if let Some(reply) = reply.try_recv().unwrap() {
        return reply;
    }
    harness
        .step_until_some(|_| reply.try_recv().unwrap())
        .unwrap()
}

pub fn begin(harness: &mut Harness<'_, App>, request: Request) -> TestSessionReply {
    let (exchange, reply) = TestSessionReply::channel(SessionRequest {
        request,
        session: SESSION_ID,
    });
    harness.state_mut().respond_to_session_exchange(exchange);
    reply
}

pub fn visible_action_notices<'a>(harness: &'a Harness<'_, App>) -> Vec<&'a str> {
    harness
        .output()
        .shapes
        .iter()
        .filter_map(|clipped| match &clipped.shape {
            egui::Shape::Text(text) => Some(text.galley.text()),
            _ => None,
        })
        .filter(|text| text.starts_with("MCP "))
        .collect()
}

pub fn first_track_reference(harness: &mut Harness<'_, App>) -> TrackReference {
    let Reply::Recordings { page } = self::execute(
        harness,
        Request::ListRecordings(ListRecordings {
            after: None,
            limit: None,
        }),
    ) else {
        panic!("expected recordings");
    };
    self::recording_details(
        harness,
        GetRecording {
            recording: page.recordings[0].id,
            after: None,
            stored_track_start: None,
            limit: None,
        },
    )
    .tracks[0]
}

pub fn recording_details(harness: &mut Harness<'_, App>, args: GetRecording) -> RecordingDetails {
    let Reply::Recording { details } = self::execute(harness, Request::GetRecording(args)) else {
        panic!("expected recording details");
    };
    details
}

pub fn track_details(harness: &mut Harness<'_, App>, track: TrackReference) -> TrackDetails {
    let Reply::Track { details } = self::execute(harness, Request::GetTrack(GetTrack { track }))
    else {
        panic!("expected track details");
    };
    details
}

pub fn completed_single_track_query(matches: usize) -> QueryJobState {
    QueryJobState::Completed {
        queries: vec![QueryCounts {
            matches,
            tracks_with_matches: usize::from(matches > 0),
            skipped: 0,
        }],
    }
}

pub const SESSION_ID: Uuid = Uuid::from_u128(1);
