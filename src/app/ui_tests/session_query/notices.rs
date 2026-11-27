use std::num::NonZeroUsize;

use egui::accesskit::Role;
use egui_kittest::kittest::Queryable as _;
use gt_session_bridge::{
    AnalysisSettingKey, ErrorCode, QueryJobRequest, QueryJobState, QueryText, Reply, Request,
    SetSetting, SettingInput, SettingKey, SettingSelection,
};
use rstest::rstest;

use crate::app::test_util::{harness, session};
use crate::app::ui_tests;

#[test]
fn a_queued_query_notice_remains_until_painting() {
    let mut harness = ui_tests::app_with_a_recording();
    let before = harness.state().toasts.len();
    let job = session::start_query(&mut harness, "# /private/receiver.raw\n\npoints | draw");
    for _ in 0..2 {
        assert_eq!(
            session::query_status(&mut harness, job),
            QueryJobState::Preparing
        );
    }
    let Reply::QueryJob { job: busy } = session::execute(
        &mut harness,
        Request::RunQuery(QueryText {
            text: "points".to_owned(),
        }),
    ) else {
        panic!("expected the active query job");
    };
    assert_eq!(busy.state, QueryJobState::Busy);
    let _logic = harness::step_minimized_logic(&mut harness);
    assert_eq!(harness.state().toasts.len(), before + 1);
    assert!(session::visible_action_notices(&harness).is_empty());
    harness.run_steps(2);
    assert_eq!(
        session::visible_action_notices(&harness),
        ["MCP queued a query"]
    );
}

#[test]
fn cancelling_a_queued_query_has_one_notice() {
    let mut harness = ui_tests::app_with_a_recording();
    let job = session::start_query(&mut harness, "points");
    let request = Request::CancelQuery(QueryJobRequest { job });
    let cancelled = session::execute(&mut harness, request.clone());
    let Reply::QueryJob { job: status } = &cancelled else {
        panic!("expected the cancelled query job");
    };
    assert_eq!(status.state, QueryJobState::Cancelled);
    assert_eq!(session::execute(&mut harness, request), cancelled);
    harness.run_steps(2);
    let notices = session::visible_action_notices(&harness);
    assert_eq!(notices.len(), 2);
    assert!(notices.contains(&"MCP cancelled a queued query"));
}

#[test]
fn an_editor_notice_requires_a_changed_draft_or_an_opened_window() {
    let mut harness = ui_tests::app_with_a_recording();
    let first = Request::OpenQuery(QueryText {
        text: "# /private/draft.gtd".to_owned(),
    });
    assert_eq!(
        session::execute(&mut harness, first.clone()),
        Reply::QueryDraftOpened
    );
    harness.run_steps(2);
    assert_eq!(
        session::visible_action_notices(&harness),
        ["MCP opened the query editor"]
    );
    let count = harness.state().toasts.len();
    assert_eq!(
        session::execute(&mut harness, first),
        Reply::QueryDraftOpened
    );
    harness.step();
    assert_eq!(harness.state().toasts.len(), count);
    assert_eq!(
        session::execute(
            &mut harness,
            Request::OpenQuery(QueryText {
                text: "points | draw".to_owned()
            })
        ),
        Reply::QueryDraftOpened,
    );
    harness.run_steps(2);
    assert!(session::visible_action_notices(&harness).contains(&"MCP updated the query editor"));
}

#[rstest]
#[case::draft(Request::OpenQuery, "MCP opened the query editor")]
#[case::run(Request::RunQuery, "MCP queued a query")]
fn an_editor_request_expands_the_query_window(
    #[case] request: fn(QueryText) -> Request,
    #[case] expected_notice: &str,
) {
    let mut harness = ui_tests::app_with_query_window_open();
    let text = "points";
    harness
        .state_mut()
        .editor_query
        .session_mut()
        .set_text(text.to_owned());
    harness
        .get_by_role_and_label(Role::Window, "Query")
        .get_by_role_and_label(Role::Button, "Hide")
        .click();
    harness.run_steps(10);
    assert!(harness.state().query_window.open);
    assert!(harness.query_by_role(Role::MultilineTextInput).is_none());

    assert!(matches!(
        session::execute(
            &mut harness,
            request(QueryText {
                text: text.to_owned()
            })
        ),
        Reply::QueryDraftOpened | Reply::QueryJob { .. }
    ));
    harness.run_steps(10);
    harness.get_by_role(Role::MultilineTextInput);
    assert_eq!(session::visible_action_notices(&harness), [expected_notice]);
}

#[test]
fn a_setting_open_notice_survives_a_repeat_during_window_sizing() {
    let mut harness = ui_tests::app_with_a_recording();
    let max_passes = harness.ctx.options(|options| options.max_passes);
    harness
        .ctx
        .options_mut(|options| options.max_passes = NonZeroUsize::MIN);
    let key = SettingKey::Analysis(AnalysisSettingKey::ElevationMask);
    let request = Request::OpenSetting(SettingSelection { key });
    assert_eq!(
        session::execute(&mut harness, request.clone()),
        Reply::SettingOpened { key }
    );
    harness.step();
    assert!(harness.state().setting_to_reveal.is_some());
    assert_eq!(
        session::execute(&mut harness, request),
        Reply::SettingOpened { key }
    );
    assert!(harness.state().setting_to_reveal.is_some());
    assert!(session::visible_action_notices(&harness).is_empty());

    harness
        .ctx
        .options_mut(|options| options.max_passes = max_passes);
    harness.run_steps(3);
    assert_eq!(
        session::visible_action_notices(&harness),
        ["MCP revealed Elevation mask"]
    );
}

#[test]
fn setting_notices_report_a_value_change_and_a_reveal() {
    let mut harness = ui_tests::app_with_a_recording();
    let key = SettingKey::Analysis(AnalysisSettingKey::ElevationMask);
    let changed = Request::SetSetting(SetSetting {
        key,
        value: SettingInput::Number(18.5),
    });
    let applied = session::execute(&mut harness, changed.clone());
    assert!(matches!(applied, Reply::Setting { .. }));
    let count = harness.state().toasts.len();
    assert_eq!(session::execute(&mut harness, changed), applied);
    assert_eq!(
        session::execute(
            &mut harness,
            Request::SetSetting(SetSetting {
                key,
                value: SettingInput::Number(-10.0)
            })
        ),
        ErrorCode::InvalidSettingValue.into(),
    );
    assert_eq!(harness.state().toasts.len(), count);
    let reveal = Request::OpenSetting(SettingSelection { key });
    assert_eq!(
        session::execute(&mut harness, reveal.clone()),
        Reply::SettingOpened { key }
    );
    assert_eq!(harness.state().toasts.len(), count);
    harness.run_steps(3);
    let notices = session::visible_action_notices(&harness);
    assert_eq!(notices.len(), 2);
    assert!(notices.contains(&"MCP changed Elevation mask"));
    assert!(notices.contains(&"MCP revealed Elevation mask"));
    assert_eq!(
        session::execute(&mut harness, reveal),
        Reply::SettingOpened { key }
    );
    harness.run_steps(2);
    let notices = session::visible_action_notices(&harness);
    assert_eq!(
        notices
            .iter()
            .filter(|&&notice| notice == "MCP revealed Elevation mask")
            .count(),
        1
    );
}
