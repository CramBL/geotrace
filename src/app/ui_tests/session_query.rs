use std::num::NonZeroUsize;

use chrono::{DateTime, TimeDelta, Utc};
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use gt_loaded_files::FileHistory;
use gt_query::Unit;
use gt_session_bridge::{
    AnalysisSettingKey, ChannelEntry, ErrorCode, GetQueryResults, NumericObservation, QueryJobId,
    QueryJobRequest, QueryJobState, QueryJobStatus, QueryMatch, QueryMatchPosition,
    QueryPageRequest, QueryResultPage, QueryResultSource, QueryResultTime, QueryText,
    QueryValidation, RangeTarget, Reply, Request, SessionRequest, SettingKey, SettingSelection,
    ShowOnMap, ShowOnPlot,
};
use gt_test_utils::fixtures;
use gt_types::{DataCategory, FileIdx, PointIdx, TrackIdx, TrackRef};
use rstest::rstest;
use uuid::Uuid;

use crate::app::test_util;
use crate::app::test_util::harness::TestDroppedFile;
use crate::app::test_util::session;
use crate::app::{App, ui_tests};

mod background;
mod history;
mod inspection;
mod map;
mod notices;
mod panels;
mod plot;
mod processing;
mod recordings;
mod settings;
mod state;
mod view;
mod visibility;

#[derive(Clone, Copy)]
enum PresentationPane {
    Map,
    Plot,
}

#[rstest]
#[case::map(PresentationPane::Map)]
#[case::plot(PresentationPane::Plot)]
fn releasing_dismiss_keeps_a_replacement_with_the_same_caption(#[case] pane: PresentationPane) {
    let mut harness = ui_tests::harness_with_three_files_loaded();
    let mut track = session::first_track_reference(&mut harness);
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .recording_name_template = "Recording".to_owned();
    let dismiss = match pane {
        PresentationPane::Map => "Dismiss map presentation",
        PresentationPane::Plot => "Dismiss plot presentation",
    };
    let show = |track| {
        let target = RangeTarget::Track {
            track,
            start: Some(20),
            end: Some(24),
        };
        match pane {
            PresentationPane::Map => Request::ShowOnMap(ShowOnMap { target }),
            PresentationPane::Plot => Request::ShowOnPlot(ShowOnPlot {
                target,
                metrics: Vec::new(),
            }),
        }
    };
    let opened = match pane {
        PresentationPane::Map => Reply::MapRangeFramed {
            track,
            start: 20,
            end: 24,
        },
        PresentationPane::Plot => Reply::PlotPresentationQueued,
    };
    assert_eq!(session::execute(&mut harness, show(track)), opened);
    harness.run_steps(3);
    harness.get_by_label("Recording / Track 1 / Points 20 to 23");
    let button = harness
        .get_by_role_and_label(Role::Button, dismiss)
        .rect()
        .center();
    harness.hover_at(button);
    harness.step();
    let step = |harness: &mut Harness<'_, App>| {
        let time = harness.ctx.input(|input| input.time);
        harness.input_mut().time = Some(time + 0.05);
        harness.step();
    };
    let pointer_button = |pressed| egui::Event::PointerButton {
        pos: button,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    harness.input_mut().events.push(pointer_button(true));
    step(&mut harness);

    let second_file = FileIdx::new(1);
    track.recording.number = harness
        .state()
        .shared
        .borrow()
        .loaded_files
        .view()
        .entry_for(second_file)
        .unwrap()
        .id()
        .into();
    let replacement = match opened {
        Reply::MapRangeFramed { start, end, .. } => Reply::MapRangeFramed { track, start, end },
        reply => reply,
    };
    assert_eq!(session::execute(&mut harness, show(track)), replacement);
    for _ in 0..3 {
        step(&mut harness);
    }
    harness.input_mut().events.push(pointer_button(false));
    step(&mut harness);

    let shared = harness.state().shared.borrow();
    let presented = match pane {
        PresentationPane::Map => shared
            .presented_session_map
            .map(|active| active.target().track),
        PresentationPane::Plot => shared
            .presented_session_plot
            .as_ref()
            .map(|active| active.target.track),
    };
    assert_eq!(
        presented,
        Some(TrackRef::new(second_file, TrackIdx::new(0)))
    );
    drop(shared);
    harness.get_by_role_and_label(Role::Button, dismiss);
}

#[test]
fn session_inspection_uses_the_loaded_recording() {
    let mut harness = ui_tests::app_with_recording_bytes(ui_tests::accel_channel_gtd_bytes(28.0));
    let check =
        serde_json::from_str(r#"{"method":"check_query","text":"@accel | where @accel.x > 1 g"}"#)
            .unwrap();
    assert_eq!(
        session::execute(&mut harness, check),
        Reply::QueryCheck {
            validation: QueryValidation::Valid,
        }
    );

    let metrics = serde_json::from_str(r#"{"method":"list_metrics"}"#).unwrap();
    let Reply::Metrics { page } = session::execute(&mut harness, metrics) else {
        panic!("expected metrics");
    };
    let [ChannelEntry::Defined(channel)] = page.channels.as_slice() else {
        panic!("expected one channel");
    };
    assert_eq!(channel.name, "accel");

    let request = serde_json::from_value(serde_json::json!({
        "method": "get_range_details",
        "track": session::first_track_reference(&mut harness),
        "limit": 1,
    }))
    .unwrap();
    let Reply::Range { details } = session::execute(&mut harness, request) else {
        panic!("expected a point page");
    };
    let [point] = details.points.as_slice() else {
        panic!("expected one point");
    };
    assert_eq!(point.index, PointIdx::new(0));
    assert_eq!(point.time, ui_tests::base_time());
    assert_eq!(details.next_start, Some(PointIdx::new(1)));
}

#[test]
fn pending_run_input_leaves_a_session_draft_unrun() {
    let mut harness = ui_tests::app_with_a_recording();
    assert!(!harness.state().query_window.open);
    let draft = "points | where eph < 20 m | draw";
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
    assert_eq!(
        session::exchange_queued_request(
            &mut harness,
            &Request::OpenQuery(QueryText {
                text: draft.to_owned()
            }),
            press_run,
        ),
        Reply::QueryDraftOpened,
    );
    harness.run_steps(2);
    let app = harness.state();
    assert!(app.query_window.open);
    assert_eq!(app.editor_query.session().text(), draft);
    assert!(app.query_window.history().is_empty());
    assert!(app.editor_query.session().matches().is_none());

    press_run(&mut harness);
    test_util::harness::step_until_query_result(&mut harness);
    assert_eq!(harness.state().query_window.history()[0].text, draft);
}

#[rstest]
#[case::same_pass(false)]
#[case::held_press(true)]
fn a_queued_run_click_uses_the_previous_draft(#[case] held_press: bool) {
    let mut harness = ui_tests::app_with_query_window_open();
    let previous = "points | where eph < 20 m | draw";
    let draft = "points | where velocity > 999 km/h | hide";
    harness
        .state_mut()
        .editor_query
        .session_mut()
        .set_text(previous.to_owned());
    harness.run_steps(2);
    let run = harness.get_by_role_and_label(Role::Button, "Run");
    let pos = run.rect().center();
    run.hover();
    harness.step();
    let button_event = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    if held_press {
        harness.input_mut().events.push(button_event(true));
        harness.step();
    }

    assert_eq!(
        session::exchange_queued_request(
            &mut harness,
            &Request::OpenQuery(QueryText {
                text: draft.to_owned()
            }),
            |harness| {
                if held_press {
                    harness.step();
                    assert_eq!(harness.state().editor_query.session().text(), previous);
                } else {
                    harness.input_mut().events.push(button_event(true));
                }
                harness.input_mut().events.push(button_event(false));
                harness.step();
            },
        ),
        Reply::QueryDraftOpened,
    );
    let app = harness.state();
    assert_eq!(app.editor_query.session().text(), draft);
    assert_eq!(app.query_window.history()[0].text, previous);
    test_util::harness::step_until_query_result(&mut harness);
    let matches = harness.state().editor_query.session().matches().unwrap();
    assert_eq!(matches.draws.len(), 1);
    assert!(matches.hidden.is_empty());
}

#[test]
fn an_empty_query_draft_is_installed_without_painting() {
    let mut harness = ui_tests::app_with_a_recording();
    harness
        .state_mut()
        .editor_query
        .session_mut()
        .set_text("points | draw".to_owned());
    assert_eq!(
        session::exchange_queued_request(
            &mut harness,
            &Request::OpenQuery(QueryText {
                text: String::new()
            }),
            |harness| {
                let _ = test_util::harness::step_minimized_logic(harness);
            },
        ),
        Reply::QueryDraftOpened,
    );
    let app = harness.state();
    assert!(app.query_window.open);
    assert!(app.editor_query.session().text().is_empty());
    assert!(app.query_window.history().is_empty());
}

#[test]
fn shutdown_rejects_draft_and_setting_navigation() {
    let mut harness = ui_tests::app_with_a_recording();
    harness.state_mut().begin_shutdown();
    let draft = Request::OpenQuery(QueryText {
        text: "points".to_owned(),
    });
    let setting = Request::OpenSetting(SettingSelection {
        key: SettingKey::Analysis(AnalysisSettingKey::ElevationMask),
    });
    for request in [draft, setting] {
        assert_eq!(
            session::execute(&mut harness, request),
            ErrorCode::ShuttingDown.into(),
        );
    }
    let app = harness.state();
    assert!(!app.query_window.open);
    assert!(!app.settings_open);
    assert!(app.editor_query.session().text().is_empty());
    assert!(app.setting_to_reveal.is_none());
}

#[test]
fn a_session_draft_opens_the_query_editor_with_diagnostics_and_previous_results() {
    let mut harness = ui_tests::app_with_query_window_open();
    ui_tests::run_query(&mut harness, "points | where velocity > 1 km/h | hide");
    let app = harness.state_mut();
    let previous = app.editor_query.session().matches().unwrap().clone();
    let history_revision = app.query_window.history_revision();
    app.query_window.open = false;
    harness.step();

    let draft = "# Første\r\n\r\npoints | where speed > 10 km/h";
    assert_eq!(
        session::execute(
            &mut harness,
            Request::OpenQuery(QueryText {
                text: draft.to_owned()
            })
        ),
        Reply::QueryDraftOpened
    );
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::Window, "Query");
    harness.get_by_label_contains("unknown metric");
    let app = harness.state();
    let query = app.editor_query.session();
    assert_eq!(query.text(), draft);
    assert_eq!(query.matches(), Some(&previous));
    assert_eq!(app.query_window.history_revision(), history_revision);
}

#[test]
fn an_editor_run_finishes_after_a_session_draft_replaces_the_text() {
    let mut harness = ui_tests::app_with_query_window_open();
    let running = "points | where eph < 20 m | draw";
    harness
        .state_mut()
        .editor_query
        .session_mut()
        .set_text(running.to_owned());
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::Button, "Run").click();
    harness.step();
    let app = harness.state();
    assert!(app.editor_query.session().matches().is_none());
    assert_eq!(app.query_window.history()[0].text, running);
    let history_revision = app.query_window.history_revision();

    let draft = "points | where velocity > 999 km/h | hide";
    assert_eq!(
        session::execute(
            &mut harness,
            Request::OpenQuery(QueryText {
                text: draft.to_owned()
            })
        ),
        Reply::QueryDraftOpened
    );
    test_util::harness::step_until_query_result(&mut harness);
    let app = harness.state();
    let query = app.editor_query.session();
    assert_eq!(query.text(), draft);
    assert_eq!(app.query_window.history_revision(), history_revision);
    let matches = query.matches().unwrap();
    assert_eq!(matches.draws.len(), 1);
    assert!(matches.hidden.is_empty());
}

#[test]
fn editor_and_session_results_refresh_when_enabled_tracks_change() {
    let mut harness = ui_tests::harness_with_three_files_loaded();
    let text = "points | where velocity > 1 km/h | draw";
    let job = session::start_query(&mut harness, text);
    let completed = session::wait_for_query_to_stop(&mut harness, job);
    assert!(matches!(completed, QueryJobState::Completed { .. }));
    assert!(
        !harness
            .state()
            .editor_query
            .session()
            .matches()
            .unwrap()
            .stale
    );
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
    assert!(
        !harness
            .state()
            .editor_query
            .session()
            .matches()
            .unwrap()
            .stale
    );
    assert_eq!(session::query_status(&mut harness, job), completed);

    for expected_stale in [true, false] {
        harness
            .state_mut()
            .shared
            .borrow_mut()
            .tree
            .toggle_file_check(other_file);
        harness.step();
        assert_eq!(
            harness
                .state()
                .editor_query
                .session()
                .matches()
                .unwrap()
                .stale,
            expected_stale
        );
        let state = if expected_stale {
            QueryJobState::Stale
        } else {
            completed.clone()
        };
        assert_eq!(session::query_status(&mut harness, job), state);
    }
}

#[test]
fn session_query_replaces_an_older_draft_and_shows_its_results() {
    let mut harness = ui_tests::app_with_query_window_open();
    let editor_text = "points | where eph < 20 m";
    ui_tests::run_query(&mut harness, editor_text);
    let editor_run = harness
        .state()
        .editor_query
        .session()
        .matches()
        .unwrap()
        .run;
    let history_revision = harness.state().query_window.history_revision();

    harness.state_mut().query_window.open = false;
    assert_eq!(
        session::execute(
            &mut harness,
            Request::OpenQuery(QueryText {
                text: "points | where eph > 40 m".to_owned(),
            }),
        ),
        Reply::QueryDraftOpened,
    );
    let text = "points | where velocity > 1 km/h | draw";
    let first = session::start_query(&mut harness, text);
    test_util::harness::step_minimized_logic(&mut harness);
    assert_eq!(harness.state().editor_query.session().text(), text);
    assert_eq!(
        session::wait_for_query_to_stop(&mut harness, first),
        session::completed_single_track_query(1)
    );
    let app = harness.state();
    let query = app.editor_query.session();
    assert_eq!(query.text(), text);
    assert_eq!(query.matches().unwrap().run, editor_run + 1);
    assert_eq!(app.query_window.history_revision(), history_revision + 1);
    assert_eq!(app.query_window.history()[0].text, text);
    harness.get_by_role_and_label(Role::Window, "Query");
    harness.get_by_label_contains("Match 1");

    let page = session::first_query_match_request(first);
    let before_edit = session::results(&mut harness, page);
    assert!(matches!(
        session::execute(
            &mut harness,
            Request::RunQuery(QueryText {
                text: "points | where speed > 1".to_owned()
            }),
        ),
        Reply::QueryCheck {
            validation: QueryValidation::Invalid { .. }
        }
    ));
    assert_eq!(harness.state().editor_query.session().text(), text);
    assert_eq!(session::results(&mut harness, page).page, before_edit.page);
    let draft = "points | where velocity > 999 km/h";
    assert_eq!(
        session::execute(
            &mut harness,
            Request::OpenQuery(QueryText {
                text: draft.to_owned()
            })
        ),
        Reply::QueryDraftOpened,
    );
    harness.run_steps(3);
    assert_eq!(harness.state().editor_query.session().text(), draft);
    assert_eq!(session::results(&mut harness, page).page, before_edit.page);
}

#[test]
fn session_query_uses_current_filter_and_visibility() {
    let mut harness = ui_tests::app_with_a_recording();
    let text = "points | where velocity > 1 km/h";
    let first = session::start_query(&mut harness, text);
    assert_eq!(
        session::wait_for_query_to_stop(&mut harness, first),
        session::completed_single_track_query(1)
    );
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
    let filtered = session::start_query(&mut harness, text);
    assert_eq!(
        session::wait_for_query_to_stop(&mut harness, filtered),
        session::completed_single_track_query(0)
    );

    {
        let mut shared = harness.state_mut().shared.borrow_mut();
        shared.filter.set_time_bounds(None, None);
        shared.tree.set_all_enabled(false);
    }
    let hidden = session::start_query(&mut harness, text);
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
fn mcp_reads_a_desktop_query_until_the_editor_runs_again() {
    let mut harness = ui_tests::app_with_query_window_open();
    let text = "points | where velocity > 1 km/h";
    harness
        .state_mut()
        .editor_query
        .session_mut()
        .set_text(text.to_owned());
    harness.run_steps(3);
    let max_passes = harness.ctx.options(|options| options.max_passes);
    harness
        .ctx
        .options_mut(|options| options.max_passes = NonZeroUsize::MIN);
    harness.get_by_role_and_label(Role::Button, "Run").click();
    harness.step();

    let request = Request::RunQuery(QueryText {
        text: "points | draw".to_owned(),
    });
    let Reply::QueryJob { job: active } = session::execute(&mut harness, request.clone()) else {
        panic!("expected the active desktop job");
    };
    assert_eq!(active.state, QueryJobState::Busy);
    assert_eq!(harness.state().editor_query.session().text(), text);
    assert!(matches!(
        harness.state_mut().execute_session_request(SessionRequest {
            request,
            session: Uuid::from_u128(2),
        }),
        Err(ErrorCode::StaleSession)
    ));
    harness
        .ctx
        .options_mut(|options| options.max_passes = max_passes);
    let completed = session::completed_single_track_query(1);
    assert_eq!(
        session::wait_for_query_to_stop(&mut harness, active.job),
        completed
    );
    let result_page = Request::GetQueryResults(session::first_query_match_request(active.job));
    assert!(matches!(
        session::execute(&mut harness, result_page.clone()),
        Reply::QueryResults { .. }
    ));
    assert!(session::visible_action_notices(&harness).is_empty());

    harness.get_by_role_and_label(Role::Button, "Run").click();
    harness.step();
    assert_eq!(
        session::execute(&mut harness, result_page),
        ErrorCode::QueryJobNotFound.into()
    );
}

#[test]
fn session_query_cancellation_drains_the_worker_and_retains_its_job() {
    let mut harness = ui_tests::app_with_a_recording();
    let text = "points | where velocity > 1 km/h";
    let preparing = session::start_query(&mut harness, text);
    assert_eq!(
        session::query_status(&mut harness, preparing),
        QueryJobState::Preparing
    );
    assert_eq!(
        session::execute(
            &mut harness,
            Request::CancelQuery(QueryJobRequest { job: preparing }),
        ),
        Reply::QueryJob {
            job: QueryJobStatus {
                job: preparing,
                state: QueryJobState::Cancelled
            },
        },
    );
    assert_eq!(
        session::query_status(&mut harness, preparing),
        QueryJobState::Cancelled
    );

    let job = session::start_query(&mut harness, text);
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
    harness.step();
    let notices = session::visible_action_notices(&harness);
    assert_eq!(notices.len(), 4);
    assert!(notices.contains(&"MCP requested query cancellation"));
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
    assert_eq!(rows[0].values[1], NumericObservation::Missing);
    assert_eq!(rows[1].values[1], NumericObservation::Finite { value: 0.0 });
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
        [NumericObservation::Finite {
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
    assert_eq!(rows[0].values[0], NumericObservation::Finite { value: 1.5 });
}

#[test]
fn session_query_status_becomes_stale_after_replacing_the_recording() {
    let mut harness = ui_tests::app_with_a_recording();
    let job = session::start_query(&mut harness, "points | where velocity > 1 km/h | table kp");
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

    let cache_generation = harness
        .state()
        .geomagnetic_indices
        .track_values_generation();
    assert_eq!(
        session::execute(
            &mut harness,
            Request::GetQueryResults(session::first_query_match_request(job)),
        ),
        ErrorCode::StaleQueryResults.into(),
    );
    assert_eq!(
        session::query_status(&mut harness, job),
        QueryJobState::Stale
    );
    assert_eq!(
        harness
            .state()
            .geomagnetic_indices
            .track_values_generation(),
        cache_generation
    );
}
