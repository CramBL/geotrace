use std::num::NonZeroUsize;

use chrono::{DateTime, TimeDelta, Utc};
use egui_kittest::Harness;
use gt_loaded_files::FileHistory;
use gt_query::Unit;
use gt_session_bridge::{
    CheckQuery, ErrorCode, GetQueryResults, GetRecording, ListRecordings, QueryCounts, QueryJobId,
    QueryJobRequest, QueryJobState, QueryJobStatus, QueryMatch, QueryMatchPosition,
    QueryPageRequest, QueryResultPage, QueryResultRow, QueryResultSource, QueryResultTime,
    QueryResultValue, QueryResults, QueryValidation, RangeTarget, Reply, Request, SessionRequest,
    ShowOnMap, ShowOnPlot, TrackReference,
};
use gt_test_utils::HarnessInteraction as _;
use gt_test_utils::fixtures;
use gt_types::{DataCategory, FileIdx, TrackIdx, TrackRef};
use uuid::Uuid;

use super::{self as ui_tests, App};
use crate::app::test_util;
use crate::app::test_util::harness::TestDroppedFile;

mod map;
mod plot;

#[test]
fn editor_and_session_results_refresh_when_enabled_tracks_change() {
    let mut harness = ui_tests::harness_with_three_files_loaded();
    harness.state_mut().query_window.open = true;
    let text = "points | where velocity > 1 km/h | draw";
    ui_tests::run_query(&mut harness, text);
    let job = self::start(&mut harness, text);
    let completed = self::complete(&mut harness, job);
    assert!(matches!(completed, QueryJobState::Completed { .. }));
    assert!(!harness.state().query_window.matches().unwrap().stale);
    let other_file = FileIdx::new(1);
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .tree
        .set_category_visible(
            TrackRef::new(other_file, TrackIdx::new(0)),
            DataCategory::Tpv,
            false,
        );
    harness.step();
    assert!(!harness.state().query_window.matches().unwrap().stale);
    assert_eq!(
        harness.state().session_query.retained_reply(),
        Some(Reply::QueryJob {
            job: QueryJobStatus {
                job,
                state: completed.clone(),
            },
        }),
    );

    for expected_stale in [true, false] {
        harness
            .state_mut()
            .shared
            .borrow_mut()
            .tree
            .toggle_file_check(other_file);
        harness.step();
        assert_eq!(
            harness.state().query_window.matches().unwrap().stale,
            expected_stale
        );
        let state = if expected_stale {
            QueryJobState::Stale
        } else {
            completed.clone()
        };
        assert_eq!(
            harness.state().session_query.retained_reply(),
            Some(Reply::QueryJob {
                job: QueryJobStatus { job, state },
            }),
        );
    }
}

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
    assert_eq!(
        self::execute(
            &mut harness,
            Request::GetQueryResults(self::first_page(job))
        ),
        ErrorCode::QueryResultsUnavailable.into()
    );
}

#[test]
fn session_query_pages_point_matches_and_absolute_table_rows() {
    let mut harness = ui_tests::harness_with_three_files_loaded();
    let time = ui_tests::base_time() + TimeDelta::seconds(20);
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .filter
        .set_time_bounds(Some(time), None);
    let job = self::start(
        &mut harness,
        "points | window 2 | where max(velocity) > 1 km/h | table time, accel, max(velocity), velocity\n\npoints | where velocity > 300 km/h",
    );
    let request = self::first_page(job);
    assert_eq!(
        self::execute(&mut harness, Request::GetQueryResults(request)),
        ErrorCode::QueryNotComplete.into()
    );
    assert!(matches!(
        self::complete(&mut harness, job),
        QueryJobState::Completed { .. }
    ));
    let results = self::results(&mut harness, request);
    assert_eq!(results.source, QueryResultSource::Points);
    assert_eq!(results.columns[0].ordinal, 2);
    assert_eq!(results.columns[0].unit.as_deref(), Some("m/s"));
    let (matches, next) = self::match_page(results.page);
    assert_eq!(matches.len(), 1);
    let matched = &matches[0];
    assert_eq!(matched.range, 20..240);
    assert_eq!(matched.first_time, Some(QueryResultTime::Utc(time)));
    assert_eq!(
        matched.last_time,
        Some(QueryResultTime::Utc(
            ui_tests::base_time() + TimeDelta::seconds(239)
        ))
    );
    assert_eq!(
        next,
        Some(QueryMatchPosition {
            track_ordinal: 1,
            match_ordinal: 0
        })
    );
    let (second, _) = self::match_page(
        self::results(
            &mut harness,
            GetQueryResults {
                page: QueryPageRequest::Matches { start: next },
                ..request
            },
        )
        .page,
    );
    assert_ne!(matched.track.recording, second[0].track.recording);

    self::assert_point_rows(&mut harness, request, matched);
    assert_eq!(
        self::results(
            &mut harness,
            GetQueryResults {
                query: 1,
                ..request
            }
        )
        .page,
        QueryResultPage::Matches {
            matches: Vec::new(),
            next_start: None
        }
    );
}

fn assert_point_rows(
    harness: &mut Harness<'_, App>,
    request: GetQueryResults,
    matched: &QueryMatch,
) {
    let rows_request = GetQueryResults {
        page: QueryPageRequest::Rows {
            matched: matched.position,
            start: None,
        },
        limit: Some(NonZeroUsize::MAX),
        ..request
    };
    let details = self::results(harness, rows_request);
    assert_eq!(
        details
            .columns
            .iter()
            .map(|column| column.ordinal)
            .collect::<Vec<_>>(),
        [0, 1, 3]
    );
    let (rows, next_start) = self::row_page(details.page);
    assert_eq!(rows.len(), 100);
    assert_eq!(rows[0].index, 20);
    assert_eq!(rows[0].time, matched.first_time);
    assert_eq!(rows[0].values[1], QueryResultValue::Missing);
    assert_eq!(rows[1].values[1], QueryResultValue::Finite { value: 0.0 });
    assert_eq!(rows[0].values[2], matched.aggregates[0]);
    assert_eq!(next_start, Some(120));

    let invalid = GetQueryResults {
        page: QueryPageRequest::Rows {
            matched: matched.position,
            start: Some(19),
        },
        ..rows_request
    };
    assert_eq!(
        self::execute(harness, Request::GetQueryResults(invalid)),
        ErrorCode::InvalidQueryCursor.into()
    );
}

#[test]
fn session_query_pages_channel_rows_on_the_filtered_timeline() {
    let mut harness = Harness::builder()
        .with_context_setup(gt_ui_theme::install_app_style)
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    test_util::harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(ui_tests::accel_channel_gtd_bytes(28.0), "accel.gtd"),
    );
    let time = ui_tests::base_time() + TimeDelta::seconds(100);
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .filter
        .set_time_bounds(Some(time), None);
    let job = self::start(
        &mut harness,
        "@accel | window 2 | where min(@accel.x) > 1 g | table max(@accel.x)",
    );
    assert!(matches!(
        self::complete(&mut harness, job),
        QueryJobState::Completed { .. }
    ));
    let request = self::first_page(job);
    let results = self::results(&mut harness, request);
    assert_eq!(
        results.source,
        QueryResultSource::Channel {
            channel: "accel".to_owned()
        }
    );
    assert_eq!(results.columns[0].ordinal, 3);
    assert_eq!(
        results.columns[0].unit.as_deref(),
        Some(Unit::M_PER_S2.label())
    );
    let (matches, next) = self::match_page(results.page);
    assert_eq!(matches[0].range, 0..20);
    assert_eq!(
        matches[0].first_time,
        Some(QueryResultTime::UnixSeconds(time.timestamp() as f64))
    );
    assert_eq!(
        matches[0].aggregates,
        [QueryResultValue::Finite {
            value: 1.5 * Unit::G.to_base()
        }]
    );
    assert_eq!(
        next,
        Some(QueryMatchPosition {
            track_ordinal: 0,
            match_ordinal: 1
        })
    );
    assert_eq!(
        matches[0].last_time,
        Some(QueryResultTime::UnixSeconds(
            (time + TimeDelta::seconds(19)).timestamp() as f64
        ))
    );
    self::assert_channel_rows(&mut harness, request, &matches[0]);
    let target = RangeTarget::QueryMatch {
        job,
        query: 0,
        matched: matches[0].position,
    };
    assert_eq!(
        self::execute(&mut harness, Request::ShowOnMap(ShowOnMap { target })),
        ErrorCode::QueryMatchIsChannel.into()
    );
    let frame = ShowOnPlot {
        target,
        metrics: Vec::new(),
    };
    self::assert_channel_plot(&mut harness, &frame, time);

    harness
        .state_mut()
        .shared
        .borrow_mut()
        .filter
        .set_time_bounds(None, None);
    assert_eq!(
        self::execute(&mut harness, Request::GetQueryResults(request)),
        ErrorCode::StaleQueryResults.into()
    );
    assert_eq!(
        self::execute(&mut harness, Request::ShowOnPlot(frame)),
        ErrorCode::StaleQueryResults.into()
    );
}

fn assert_channel_plot(harness: &mut Harness<'_, App>, frame: &ShowOnPlot, start: DateTime<Utc>) {
    {
        let mut shared = harness.state_mut().shared.borrow_mut();
        shared.plot_state.sync_to_map = false;
        shared.plot_state.channel_vis.set("accel", false);
    }
    assert_eq!(
        self::execute(harness, Request::ShowOnPlot(frame.clone())),
        Reply::PlotPresentationQueued
    );
    harness.run_steps(2);
    let shared = harness.state().shared.borrow();
    let start = start.timestamp() as f64;
    assert!(shared.plot_state.show_channels);
    assert!(shared.plot_state.channel_vis.is_visible("accel"));
    assert_eq!(
        shared.plot_state.visible_x_range(),
        Some(start - 1.0..=start + 20.0)
    );
}

fn assert_channel_rows(
    harness: &mut Harness<'_, App>,
    request: GetQueryResults,
    matched: &QueryMatch,
) {
    let details = self::results(
        harness,
        GetQueryResults {
            page: QueryPageRequest::Rows {
                matched: matched.position,
                start: Some(19),
            },
            ..request
        },
    );
    assert_eq!(
        details
            .columns
            .iter()
            .map(|column| column.label.as_str())
            .collect::<Vec<_>>(),
        ["x", "y", "z"]
    );
    assert_eq!(details.columns[0].unit.as_deref(), Some("g"));
    let (rows, next_start) = self::row_page(details.page);
    assert_eq!(next_start, None);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].index, 19);
    assert_eq!(rows[0].time, matched.last_time);
    assert_eq!(rows[0].values[0], QueryResultValue::Finite { value: 1.5 });
}

fn match_page(page: QueryResultPage) -> (Vec<QueryMatch>, Option<QueryMatchPosition>) {
    let QueryResultPage::Matches {
        matches,
        next_start,
    } = page
    else {
        panic!("expected match page");
    };
    (matches, next_start)
}

fn row_page(page: QueryResultPage) -> (Vec<QueryResultRow>, Option<usize>) {
    let QueryResultPage::Rows {
        rows, next_start, ..
    } = page
    else {
        panic!("expected row page");
    };
    (rows, next_start)
}

fn first_page(job: QueryJobId) -> GetQueryResults {
    GetQueryResults {
        job,
        query: 0,
        page: QueryPageRequest::Matches { start: None },
        limit: Some(NonZeroUsize::MIN),
    }
}

fn results(harness: &mut Harness<'_, App>, args: GetQueryResults) -> QueryResults {
    let Reply::QueryResults { results } = self::execute(harness, Request::GetQueryResults(args))
    else {
        panic!("expected query results");
    };
    results
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
    let reply = self::execute(
        harness,
        Request::RunQuery(CheckQuery {
            text: text.to_owned(),
        }),
    );
    let Reply::QueryJob { job } = reply else {
        panic!("expected a running query job, got {reply:?}");
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

fn first_track_reference(harness: &mut Harness<'_, App>) -> TrackReference {
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
            limit: None,
        }),
    ) else {
        panic!("expected recording details");
    };
    details.tracks[0]
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
