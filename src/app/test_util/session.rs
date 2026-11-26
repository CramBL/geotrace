use std::num::NonZeroUsize;
use std::thread;

use egui_kittest::Harness;
use gt_session_bridge::test_util::TestEndpoint;
use gt_session_bridge::{
    GetQueryResults, GetRecording, ListRecordings, QueryCounts, QueryJobId, QueryJobRequest,
    QueryJobState, QueryMatch, QueryMatchPosition, QueryPageRequest, QueryResultPage,
    QueryResultRow, QueryResults, QueryText, Reply, Request, SessionRequest, TrackReference,
};
use gt_test_utils::HarnessInteraction as _;
use uuid::Uuid;

use crate::app::App;

use super::harness;

pub fn app_with_bridge() -> (Harness<'static, App>, TestEndpoint) {
    let mut harness = Harness::builder()
        .with_context_setup(gt_ui_theme::install_app_style)
        .with_wait_for_pending_images(false)
        .build_eframe(harness::transient_app);
    let endpoint = self::install_bridge(&mut harness);
    (harness, endpoint)
}

pub fn install_bridge(harness: &mut Harness<'_, App>) -> TestEndpoint {
    let endpoint = TestEndpoint::new().unwrap();
    let ctx = harness.ctx.clone();
    harness.state_mut().session_bridge = Some(
        endpoint
            .start_server(move || ctx.request_repaint())
            .unwrap(),
    );
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
    harness
        .state_mut()
        .execute_session_request(SessionRequest {
            request,
            session: SESSION_ID,
        })
        .unwrap_or_else(Reply::from)
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
    let Reply::Recording { details } = self::execute(
        harness,
        Request::GetRecording(GetRecording {
            recording: page.recordings[0].id,
            after: None,
            stored_track_start: None,
            limit: None,
        }),
    ) else {
        panic!("expected recording details");
    };
    details.tracks[0]
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
