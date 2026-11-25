use std::num::NonZeroUsize;

use chrono::{DateTime, TimeDelta, Utc};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use gt_loaded_files::FileHistory;
use gt_query::Unit;
use gt_session_bridge::{
    ErrorCode, GetQueryResults, QueryJobId, QueryJobRequest, QueryJobState, QueryJobStatus,
    QueryMatch, QueryMatchPosition, QueryPageRequest, QueryResultPage, QueryResultSource,
    QueryResultTime, QueryResultValue, QueryText, QueryValidation, RangeTarget, Reply, Request,
    ShowOnMap, ShowOnPlot,
};
use gt_test_utils::fixtures;
use gt_types::{DataCategory, FileIdx, TrackIdx, TrackRef};
use uuid::Uuid;

use crate::app::test_util;
use crate::app::test_util::harness::TestDroppedFile;
use crate::app::test_util::session;
use crate::app::{App, ui_tests};

mod background;
mod history;
mod map;
mod plot;
mod processing;
mod settings;

#[test]
fn pending_run_input_leaves_a_session_draft_unrun() {
    let mut harness = ui_tests::app_with_a_recording();
    assert!(!harness.state().query_window.open);
    let draft = "points | where eph < 20 m | draw";
    assert_eq!(
        session::execute(
            &mut harness,
            Request::OpenQuery(QueryText {
                text: draft.to_owned(),
            })
        ),
        Reply::QueryDraftQueued
    );
    let press_run = |harness: &mut Harness<'_, App>| {
        harness
            .input_mut()
            .events
            .extend([true, false].map(|pressed| egui::Event::Key {
                key: egui::Key::Enter,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            }));
        harness.step();
    };
    press_run(&mut harness);
    harness.run_steps(2);
    let window = &harness.state().query_window;
    assert!(window.open);
    assert_eq!(window.text(), draft);
    assert!(window.history().is_empty());
    assert!(window.matches().is_none());

    press_run(&mut harness);
    test_util::harness::step_until_query_result(&mut harness);
    assert_eq!(harness.state().query_window.history()[0].text, draft);
}

#[test]
fn a_session_draft_opens_the_query_editor_with_diagnostics_and_previous_results() {
    let mut harness = ui_tests::app_with_query_window_open();
    ui_tests::run_query(&mut harness, "points | where velocity > 1 km/h | hide");
    let window = &mut harness.state_mut().query_window;
    let previous = window.matches().unwrap().clone();
    let history_revision = window.history_revision();
    window.open = false;
    harness.step();

    let draft = "# Første\r\n\r\npoints | where speed > 10 km/h";
    assert_eq!(
        session::execute(
            &mut harness,
            Request::OpenQuery(QueryText {
                text: draft.to_owned()
            })
        ),
        Reply::QueryDraftQueued
    );
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::Window, "Query");
    harness.get_by_label_contains("unknown metric");
    let window = &harness.state().query_window;
    assert_eq!(window.text(), draft);
    assert_eq!(window.matches(), Some(&previous));
    assert_eq!(window.history_revision(), history_revision);
    assert!(harness.state().session_query.retained_reply().is_none());
}

#[test]
fn an_editor_run_finishes_after_a_session_draft_replaces_the_text() {
    let mut harness = ui_tests::app_with_query_window_open();
    let running = "points | where eph < 20 m | draw";
    harness
        .state_mut()
        .query_window
        .set_text(running.to_owned());
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::Button, "Run").click();
    harness.step();
    let window = &harness.state().query_window;
    assert!(window.matches().is_none());
    assert_eq!(window.history()[0].text, running);
    let history_revision = window.history_revision();

    let draft = "points | where velocity > 999 km/h | hide";
    assert_eq!(
        session::execute(
            &mut harness,
            Request::OpenQuery(QueryText {
                text: draft.to_owned()
            })
        ),
        Reply::QueryDraftQueued
    );
    test_util::harness::step_until_query_result(&mut harness);
    let window = &harness.state().query_window;
    assert_eq!(window.text(), draft);
    assert_eq!(window.history_revision(), history_revision);
    assert_eq!(window.matches().unwrap().draws.len(), 1);
    assert!(window.matches().unwrap().hidden.is_empty());
}

#[test]
fn editor_and_session_results_refresh_when_enabled_tracks_change() {
    let mut harness = ui_tests::harness_with_three_files_loaded();
    harness.state_mut().query_window.open = true;
    let text = "points | where velocity > 1 km/h | draw";
    ui_tests::run_query(&mut harness, text);
    let job = session::start_query(&mut harness, text);
    let completed = session::wait_for_query_to_stop(&mut harness, job);
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

    let first = session::start_query(&mut harness, "points | where velocity > 1 km/h | hide");
    assert_eq!(
        session::wait_for_query_to_stop(&mut harness, first),
        session::completed_single_track_query(1)
    );
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
    assert_eq!(
        session::query_status(&mut harness, first),
        QueryJobState::Stale
    );
    let filtered = session::start_query(&mut harness, "points | where velocity > 1 km/h");
    assert_eq!(
        session::wait_for_query_to_stop(&mut harness, filtered),
        session::completed_single_track_query(0)
    );

    {
        let mut shared = harness.state_mut().shared.borrow_mut();
        shared.filter.set_time_bounds(None, None);
        shared.tree.set_all_enabled(false);
    }
    let hidden = session::start_query(&mut harness, "points | where velocity > 1 km/h");
    assert_eq!(
        session::wait_for_query_to_stop(&mut harness, hidden),
        session::completed_single_track_query(0)
    );
    let old = Request::GetQueryStatus(QueryJobRequest { job: filtered });
    assert_eq!(
        session::execute(&mut harness, old),
        ErrorCode::QueryJobNotFound.into()
    );
}

#[test]
fn session_query_cancellation_drains_the_worker_and_retains_its_job() {
    let mut harness = ui_tests::app_with_a_recording();
    let job = session::start_query(&mut harness, "points | where velocity > 1 km/h");
    let ctx = harness.ctx.clone();
    harness.state_mut().prepare_session_query(&ctx);
    assert_eq!(
        session::execute(
            &mut harness,
            Request::RunQuery(QueryText {
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
        session::execute(
            &mut harness,
            Request::CancelQuery(QueryJobRequest {
                job: previous_session
            })
        ),
        ErrorCode::StaleSession.into()
    );
    let Reply::QueryJob { job: status } =
        session::execute(&mut harness, Request::CancelQuery(QueryJobRequest { job }))
    else {
        panic!("expected a cancellation status");
    };
    assert_eq!(status.state, QueryJobState::Cancelling);
    assert_eq!(
        session::wait_for_query_to_stop(&mut harness, job),
        QueryJobState::Cancelled
    );
    assert_eq!(
        session::execute(
            &mut harness,
            Request::RunQuery(QueryText {
                text: "# empty".to_owned()
            })
        ),
        Reply::QueryCheck {
            validation: QueryValidation::Empty
        }
    );
    assert_eq!(
        session::query_status(&mut harness, job),
        QueryJobState::Cancelled
    );
    assert_eq!(
        session::execute(
            &mut harness,
            Request::GetQueryResults(session::first_query_match_request(job))
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
    let job = session::start_query(
        &mut harness,
        "points | window 2 | where max(velocity) > 1 km/h | table time, accel, max(velocity), velocity\n\npoints | where velocity > 300 km/h",
    );
    let request = session::first_query_match_request(job);
    assert_eq!(
        session::execute(&mut harness, Request::GetQueryResults(request)),
        ErrorCode::QueryNotComplete.into()
    );
    assert!(matches!(
        session::wait_for_query_to_stop(&mut harness, job),
        QueryJobState::Completed { .. }
    ));
    let results = session::results(&mut harness, request);
    assert_eq!(results.source, QueryResultSource::Points);
    assert_eq!(results.columns[0].ordinal, 2);
    assert_eq!(results.columns[0].unit.as_deref(), Some("m/s"));
    let (matches, next) = session::match_page(results.page);
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
    let (second, _) = session::match_page(
        session::results(
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
        session::results(
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
    let details = session::results(harness, rows_request);
    assert_eq!(
        details
            .columns
            .iter()
            .map(|column| column.ordinal)
            .collect::<Vec<_>>(),
        [0, 1, 3]
    );
    let (rows, next_start) = session::row_page(details.page);
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
        session::execute(harness, Request::GetQueryResults(invalid)),
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
    let job = session::start_query(
        &mut harness,
        "@accel | window 2 | where min(@accel.x) > 1 g | table max(@accel.x)",
    );
    assert!(matches!(
        session::wait_for_query_to_stop(&mut harness, job),
        QueryJobState::Completed { .. }
    ));
    let request = session::first_query_match_request(job);
    let results = session::results(&mut harness, request);
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
    let (matches, next) = session::match_page(results.page);
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
        session::execute(&mut harness, Request::ShowOnMap(ShowOnMap { target })),
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
        session::execute(&mut harness, Request::GetQueryResults(request)),
        ErrorCode::StaleQueryResults.into()
    );
    assert_eq!(
        session::execute(&mut harness, Request::ShowOnPlot(frame)),
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
        session::execute(harness, Request::ShowOnPlot(frame.clone())),
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
    let details = session::results(
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
    let (rows, next_start) = session::row_page(details.page);
    assert_eq!(next_start, None);
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].index, 19);
    assert_eq!(rows[0].time, matched.last_time);
    assert_eq!(rows[0].values[0], QueryResultValue::Finite { value: 1.5 });
}

#[test]
fn session_query_status_becomes_stale_after_replacing_the_recording() {
    let mut harness = ui_tests::app_with_a_recording();
    let job = session::start_query(&mut harness, "points | where velocity > 1 km/h");
    let completed = session::completed_single_track_query(1);
    assert_eq!(
        session::wait_for_query_to_stop(&mut harness, job),
        completed
    );
    assert_eq!(session::query_status(&mut harness, job), completed);

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

    assert_eq!(
        session::query_status(&mut harness, job),
        QueryJobState::Stale
    );
}
