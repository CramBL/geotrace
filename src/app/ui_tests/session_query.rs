use chrono::TimeDelta;
use egui_kittest::Harness;
use gt_loaded_files::FileHistory;
use gt_session_bridge::{
    CheckQuery, ErrorCode, QueryCounts, QueryJobId, QueryJobRequest, QueryJobState, QueryJobStatus,
    QueryValidation, Reply, Request, SessionRequest,
};
use gt_test_utils::HarnessInteraction as _;
use gt_test_utils::fixtures;
use uuid::Uuid;

use super::{self as ui_tests, App};

#[test]
fn session_query_uses_current_filter_and_visibility_without_changing_the_editor() {
    let mut harness = ui_tests::app_with_query_window_open();
    let editor_text = "points | where eph < 20 m";
    ui_tests::run_query(&mut harness, editor_text);
    let editor_matches = harness.state().query_window.matches().unwrap().clone();
    let history_revision = harness.state().query_window.history_revision();

    let first = self::start(&mut harness, "points | where velocity > 1 km/h | hide");
    assert_eq!(self::complete(&mut harness, first), self::completed(1));
    let window = &harness.state().query_window;
    assert_eq!(window.text(), editor_text);
    assert_eq!(window.matches(), Some(&editor_matches));
    assert_eq!(window.history_revision(), history_revision);

    harness
        .state_mut()
        .shared
        .borrow_mut()
        .filter
        .set_time_bounds(Some(ui_tests::base_time() + TimeDelta::minutes(10)), None);
    assert_eq!(self::status(&mut harness, first), QueryJobState::Stale);
    let filtered = self::start(&mut harness, "points | where velocity > 1 km/h");
    assert_eq!(self::complete(&mut harness, filtered), self::completed(0));

    {
        let mut shared = harness.state_mut().shared.borrow_mut();
        shared.filter.set_time_bounds(None, None);
        shared.tree.set_all_enabled(false);
    }
    let hidden = self::start(&mut harness, "points | where velocity > 1 km/h");
    assert_eq!(self::complete(&mut harness, hidden), self::completed(0));
    let old = Request::GetQueryStatus(QueryJobRequest { job: filtered });
    assert_eq!(
        self::execute(&mut harness, old),
        ErrorCode::QueryJobNotFound.into()
    );
}

#[test]
fn session_query_cancellation_drains_the_worker_and_retains_its_job() {
    let mut harness = ui_tests::app_with_a_recording();
    let job = self::start(&mut harness, "points | where velocity > 1 km/h");
    let ctx = harness.ctx.clone();
    harness.state_mut().prepare_session_query(&ctx);
    assert_eq!(
        self::execute(
            &mut harness,
            Request::RunQuery(CheckQuery {
                text: "points".to_owned()
            })
        ),
        Reply::QueryJob {
            job: QueryJobStatus {
                job,
                state: QueryJobState::Busy,
            }
        }
    );
    let previous_session = QueryJobId {
        session: Uuid::from_u128(2),
        ..job
    };
    assert_eq!(
        self::execute(
            &mut harness,
            Request::CancelQuery(QueryJobRequest {
                job: previous_session
            })
        ),
        ErrorCode::StaleSession.into()
    );
    let Reply::QueryJob { job: status } =
        self::execute(&mut harness, Request::CancelQuery(QueryJobRequest { job }))
    else {
        panic!("expected a cancellation status");
    };
    assert_eq!(status.state, QueryJobState::Cancelling);
    assert_eq!(self::complete(&mut harness, job), QueryJobState::Cancelled);
    assert_eq!(
        self::execute(
            &mut harness,
            Request::RunQuery(CheckQuery {
                text: "# empty".to_owned()
            })
        ),
        Reply::QueryCheck {
            validation: QueryValidation::Empty
        }
    );
    assert_eq!(self::status(&mut harness, job), QueryJobState::Cancelled);
}

#[test]
fn session_query_status_becomes_stale_after_replacing_the_recording() {
    let mut harness = ui_tests::app_with_a_recording();
    let job = self::start(&mut harness, "points | where velocity > 1 km/h");
    let completed = self::completed(1);
    assert_eq!(self::complete(&mut harness, job), completed);
    assert_eq!(self::status(&mut harness, job), completed);

    let points = fixtures::nav_points_from(ui_tests::base_time(), 1, 1);
    let replacement =
        gt_test_utils::loaded_file_with_tracks(vec![gt_test_utils::loaded_track_with_points(
            points,
        )]);
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .loaded_files
        .replace_file(0, replacement, FileHistory::None);

    assert_eq!(self::status(&mut harness, job), QueryJobState::Stale);
}

fn start(harness: &mut Harness<'_, App>, text: &str) -> QueryJobId {
    let Reply::QueryJob { job } = self::execute(
        harness,
        Request::RunQuery(CheckQuery {
            text: text.to_owned(),
        }),
    ) else {
        panic!("expected a running query job");
    };
    assert_eq!(job.state, QueryJobState::Preparing);
    job.job
}

fn status(harness: &mut Harness<'_, App>, job: QueryJobId) -> QueryJobState {
    let Reply::QueryJob { job: result } =
        self::execute(harness, Request::GetQueryStatus(QueryJobRequest { job }))
    else {
        panic!("expected query status");
    };
    assert_eq!(result.job, job);
    result.state
}

fn complete(harness: &mut Harness<'_, App>, job: QueryJobId) -> QueryJobState {
    assert!(harness.step_until(|harness| { !harness.state().session_query.is_active() }));
    self::status(harness, job)
}

fn execute(harness: &mut Harness<'_, App>, request: Request) -> Reply {
    harness
        .state_mut()
        .execute_session_request_in_test(SessionRequest {
            request,
            session: Uuid::from_u128(1),
        })
}

fn completed(matches: usize) -> QueryJobState {
    QueryJobState::Completed {
        queries: vec![QueryCounts {
            matches,
            tracks_with_matches: usize::from(matches > 0),
            skipped: 0,
        }],
    }
}
