use std::num::NonZeroUsize;

use egui::accesskit::{Role, Toggled};
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use egui_phosphor::regular::GEAR as ICON_GEAR;
use gt_session_bridge::{ErrorCode, OpenPanel, PanelOpenState, Reply, Request, SessionPanel};
use gt_test_utils::{HarnessInteraction as _, recording_fixtures};
use rstest::rstest;

use crate::app::query::QueryWindow;
use crate::app::settings_ui::{self, SettingsPage};
use crate::app::test_util::{harness, session};
use crate::app::{App, ui_tests};

#[test]
fn panel_opening_updates_session_state_before_painting() {
    let mut harness = ui_tests::app_with_a_recording();
    let app = harness.state_mut();
    app.settings_page = SettingsPage::Analysis;
    app.tiles_tree.set_visible(app.plot_tile_id, false);
    let track = session::first_track_reference(&mut harness);
    let panels = [
        OpenPanel::History {},
        OpenPanel::LogViewer {},
        OpenPanel::Plot {},
        OpenPanel::Query {},
        OpenPanel::Settings {},
        OpenPanel::SkyTrails { track },
    ];
    for args in panels {
        let mut reply = session::begin(&mut harness, Request::OpenPanel(args));
        let panel = args.panel();
        let Reply::SessionState { state } =
            session::execute(&mut harness, Request::GetSessionState)
        else {
            panic!("expected desktop session state");
        };
        assert!(state.open_panels.contains(&panel));
        let immediate = reply.try_recv().unwrap();
        if panel == SessionPanel::Plot {
            assert_eq!(
                immediate,
                Some(Reply::PanelOpen {
                    panel,
                    state: PanelOpenState::Docked
                })
            );
        } else {
            assert_eq!(immediate, None);
            assert_eq!(
                harness.step_until_some(|_| reply.try_recv().unwrap()),
                Some(Reply::PanelOpen {
                    panel,
                    state: PanelOpenState::Frontmost
                })
            );
        }
    }
    let Reply::SessionState { state } = session::execute(&mut harness, Request::GetSessionState)
    else {
        panic!("expected desktop session state");
    };
    assert_eq!(state.open_panels, panels.map(OpenPanel::panel));
    assert_eq!(harness.state().settings_page, SettingsPage::Analysis);
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::Window, "Sky trails");
}

#[test]
fn a_bridge_request_opens_the_log_viewer() {
    let (mut harness, endpoint) = session::app_with_bridge();
    let reply = session::exchange_payload(
        &mut harness,
        &endpoint,
        br#"{"method":"open_panel","panel":"log_viewer"}"#,
    );
    assert_eq!(
        serde_json::from_slice::<Reply>(&reply).unwrap(),
        Reply::PanelOpen {
            panel: SessionPanel::LogViewer,
            state: PanelOpenState::Frontmost,
        }
    );
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::Window, "Log viewer");
}

#[test]
fn opening_query_expands_the_window_and_keeps_its_draft_and_result() {
    let mut harness = ui_tests::app_with_query_window_open();
    let text = "points | draw";
    ui_tests::run_query(&mut harness, text);
    harness
        .get_by_role_and_label(Role::Window, "Query")
        .get_by_role_and_label(Role::Button, "Hide")
        .click();
    harness.run_steps(8);
    assert!(harness.query_by_role(Role::MultilineTextInput).is_none());

    assert_eq!(
        session::execute(&mut harness, Request::OpenPanel(OpenPanel::Query {})),
        Reply::PanelOpen {
            panel: SessionPanel::Query,
            state: PanelOpenState::Frontmost,
        }
    );
    harness.run_steps(8);
    harness.get_by_role(Role::MultilineTextInput);
    let query = harness.state().editor_query.session();
    assert_eq!(query.text(), text);
    assert!(query.results().is_some());
    assert_eq!(
        session::visible_action_notices(&harness),
        ["MCP showed the Query window"]
    );
}

enum OverlappingWindowAction {
    None,
    OpenPlot,
    OpenSettings,
    PressPointer,
}

#[rstest]
#[case::ordinary(OverlappingWindowAction::None)]
#[case::plot(OverlappingWindowAction::OpenPlot)]
#[case::same_target(OverlappingWindowAction::OpenSettings)]
#[case::pointer(OverlappingWindowAction::PressPointer)]
fn opening_settings_raises_an_obscured_window_once(#[case] concurrent: OverlappingWindowAction) {
    let (
        mut harness,
        QueryOverSettings {
            target,
            settings,
            query,
        },
    ) = self::app_with_query_over_settings();

    let open_settings = Request::OpenPanel(OpenPanel::Settings {});
    let opened = Reply::PanelOpen {
        panel: SessionPanel::Settings,
        state: PanelOpenState::Frontmost,
    };
    let mut original = session::begin(&mut harness, open_settings.clone());
    assert_eq!(original.try_recv().unwrap(), None);
    let mut repeated = None;
    match concurrent {
        OverlappingWindowAction::None => {}
        OverlappingWindowAction::OpenPlot => {
            assert!(harness.state().plot_is_visible());
            assert_eq!(
                session::execute(&mut harness, Request::OpenPanel(OpenPanel::Plot {})),
                Reply::PanelOpen {
                    panel: SessionPanel::Plot,
                    state: PanelOpenState::Docked,
                }
            );
        }
        OverlappingWindowAction::OpenSettings => {
            repeated = Some(session::begin(&mut harness, open_settings.clone()));
        }
        OverlappingWindowAction::PressPointer => {
            let pointer_button = |pressed| egui::Event::PointerButton {
                pos: target,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            };
            harness
                .input_mut()
                .events
                .extend([egui::Event::PointerMoved(target), pointer_button(true)]);
            harness.step();
            assert_eq!(original.try_recv().unwrap(), None);
            assert_eq!(harness.ctx.layer_id_at(target), Some(query));
            assert!(session::visible_action_notices(&harness).is_empty());
            harness.input_mut().events.push(pointer_button(false));
        }
    }
    harness.run_steps(5);
    assert_eq!(harness.ctx.layer_id_at(target), Some(settings));
    assert_eq!(original.try_recv().unwrap(), Some(opened.clone()));
    if let Some(mut repeated) = repeated {
        assert_eq!(repeated.try_recv().unwrap(), Some(opened.clone()));
    }
    assert_eq!(session::execute(&mut harness, open_settings), opened);
    harness.run_steps(5);
    assert_eq!(
        session::visible_action_notices(&harness),
        ["MCP showed Settings"]
    );
}

enum AfterSettingsRaise {
    CloseFromToolbar,
    RaiseQuery,
}

#[rstest]
#[case::another_panel(AfterSettingsRaise::RaiseQuery)]
#[case::toolbar_close(AfterSettingsRaise::CloseFromToolbar)]
fn a_completed_raise_keeps_its_notice_after_the_next_panel_action(
    #[case] next_action: AfterSettingsRaise,
) {
    let (
        mut harness,
        QueryOverSettings {
            target,
            settings,
            query,
        },
    ) = self::app_with_query_over_settings();
    harness
        .ctx
        .options_mut(|options| options.max_passes = NonZeroUsize::MIN);
    let mut earlier = session::begin(&mut harness, Request::OpenPanel(OpenPanel::Settings {}));
    harness.step();
    assert_eq!(harness.ctx.layer_id_at(target), Some(settings));

    let mut expected_notices = vec!["MCP showed Settings"];
    match next_action {
        AfterSettingsRaise::CloseFromToolbar => {
            harness
                .query_all_by_label(ICON_GEAR)
                .find(|button| button.accesskit_node().toggled() == Some(Toggled::True))
                .unwrap()
                .click_accesskit();
        }
        AfterSettingsRaise::RaiseQuery => {
            assert_eq!(
                session::execute(&mut harness, Request::OpenPanel(OpenPanel::Query {})),
                Reply::PanelOpen {
                    panel: SessionPanel::Query,
                    state: PanelOpenState::Frontmost,
                }
            );
            expected_notices.push("MCP showed the Query window");
        }
    }
    harness.run_steps(5);
    assert_eq!(
        earlier.try_recv().unwrap(),
        Some(Reply::PanelOpen {
            panel: SessionPanel::Settings,
            state: PanelOpenState::Frontmost,
        })
    );
    if matches!(next_action, AfterSettingsRaise::RaiseQuery) {
        assert_eq!(harness.ctx.layer_id_at(target), Some(query));
    } else {
        assert!(!harness.state().settings_open);
    }
    assert_eq!(session::visible_action_notices(&harness), expected_notices);
}

#[test]
fn the_last_panel_request_raises_its_window_after_a_batch_without_painting() {
    let mut harness = ui_tests::app_with_a_recording();
    harness.set_size(egui::vec2(640.0, 440.0));
    harness.state_mut().settings_page = SettingsPage::Analysis;
    let mut settings = session::begin(&mut harness, Request::OpenPanel(OpenPanel::Settings {}));
    let mut query = session::begin(&mut harness, Request::OpenPanel(OpenPanel::Query {}));
    assert_eq!(
        settings.try_recv().unwrap(),
        Some(Reply::PanelOpen {
            panel: SessionPanel::Settings,
            state: PanelOpenState::Superseded,
        })
    );
    harness.run_steps(6);
    assert_eq!(
        query.try_recv().unwrap(),
        Some(Reply::PanelOpen {
            panel: SessionPanel::Query,
            state: PanelOpenState::Frontmost,
        })
    );
    let target = harness
        .get_by_label_contains("Elevation mask")
        .rect()
        .center();
    let query_id = QueryWindow::window_id();
    assert!(
        harness
            .ctx
            .memory(|memory| memory.area_rect(query_id).unwrap())
            .contains(target)
    );
    assert_eq!(
        harness.ctx.layer_id_at(target),
        Some(egui::LayerId::new(egui::Order::Middle, query_id))
    );
    assert_eq!(
        session::visible_action_notices(&harness),
        ["MCP showed Settings", "MCP showed the Query window"]
    );
}

#[rstest]
#[case::expired(false)]
#[case::cancelled(true)]
fn a_minimized_panel_raise_leaves_the_window_open_without_a_later_raise(#[case] cancel: bool) {
    let (mut harness, QueryOverSettings { target, query, .. }) =
        self::app_with_query_over_settings();
    harness.state_mut().settings_open = false;
    let mut reply = session::begin(&mut harness, Request::OpenPanel(OpenPanel::Settings {}));
    let _ = harness::step_minimized_logic(&mut harness);
    assert_eq!(reply.try_recv().unwrap(), None);
    let Reply::SessionState { state } = session::execute(&mut harness, Request::GetSessionState)
    else {
        panic!("expected desktop session state");
    };
    assert!(state.open_panels.contains(&SessionPanel::Settings));
    if cancel {
        drop(reply);
        let _ = harness::step_minimized_logic(&mut harness);
    } else {
        harness
            .state_mut()
            .pending_session_panel_reveals
            .expire_oldest();
        let _ = harness::step_minimized_logic(&mut harness);
        assert_eq!(
            reply.try_recv().unwrap(),
            Some(Reply::PanelOpen {
                panel: SessionPanel::Settings,
                state: PanelOpenState::TimedOut,
            })
        );
    }
    harness.run_steps(5);
    assert!(harness.state().settings_open);
    assert_eq!(harness.ctx.layer_id_at(target), Some(query));
    assert_eq!(
        session::visible_action_notices(&harness),
        ["MCP showed Settings"]
    );
}

#[test]
fn callers_waiting_for_the_same_panel_have_independent_deadlines() {
    let (
        mut harness,
        QueryOverSettings {
            target, settings, ..
        },
    ) = self::app_with_query_over_settings();
    let request = Request::OpenPanel(OpenPanel::Settings {});
    let mut older = session::begin(&mut harness, request.clone());
    let mut newer = session::begin(&mut harness, request);
    harness
        .state_mut()
        .pending_session_panel_reveals
        .expire_oldest();
    let _ = harness::step_minimized_logic(&mut harness);
    assert_eq!(
        older.try_recv().unwrap(),
        Some(Reply::PanelOpen {
            panel: SessionPanel::Settings,
            state: PanelOpenState::TimedOut,
        })
    );
    assert!(harness.state().settings_open);
    assert_eq!(newer.try_recv().unwrap(), None);
    assert_eq!(
        harness.step_until_some(|_| newer.try_recv().unwrap()),
        Some(Reply::PanelOpen {
            panel: SessionPanel::Settings,
            state: PanelOpenState::Frontmost,
        })
    );
    assert_eq!(harness.ctx.layer_id_at(target), Some(settings));
}

#[test]
fn the_desktop_can_close_a_panel_before_mcp_raises_it() {
    let (mut harness, _) = self::app_with_query_over_settings();
    let mut reply = session::begin(&mut harness, Request::OpenPanel(OpenPanel::Settings {}));
    harness
        .query_all_by_label(ICON_GEAR)
        .find(|button| button.accesskit_node().toggled() == Some(Toggled::True))
        .unwrap()
        .click_accesskit();
    harness.step();
    assert!(!harness.state().settings_open);
    assert_eq!(
        reply.try_recv().unwrap(),
        Some(Reply::PanelOpen {
            panel: SessionPanel::Settings,
            state: PanelOpenState::Closed
        })
    );
    assert!(session::visible_action_notices(&harness).is_empty());
}

#[test]
fn a_panel_raise_preserves_the_query_editors_keyboard_focus() {
    let (mut harness, _) = self::app_with_query_over_settings();
    ui_tests::focus_query_editor_at_end(&harness, "");
    let mut reply = session::begin(&mut harness, Request::OpenPanel(OpenPanel::Settings {}));
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("points".to_owned()));
    harness.run_steps(3);
    assert_eq!(
        reply.try_recv().unwrap(),
        Some(Reply::PanelOpen {
            panel: SessionPanel::Settings,
            state: PanelOpenState::Frontmost
        })
    );
    assert_eq!(harness.state().editor_query.session().text(), "points");
}

#[rstest]
#[case::no_reports(false, ErrorCode::NoSkyTrailReports)]
#[case::stale_track(true, ErrorCode::StaleTrack)]
fn sky_trails_checks_the_track_before_opening(#[case] stale: bool, #[case] error: ErrorCode) {
    let mut harness = ui_tests::app_with_recording_bytes(recording_fixtures::two_track_gtd_bytes());
    let mut track = session::first_track_reference(&mut harness);
    if stale {
        track.revision += 1;
    }
    assert_eq!(
        session::execute(
            &mut harness,
            Request::OpenPanel(OpenPanel::SkyTrails { track })
        ),
        Reply::from(error)
    );
    harness.run_steps(3);
    assert!(!harness.state().sky_trails_window.is_open());
    assert!(session::visible_action_notices(&harness).is_empty());
}

struct QueryOverSettings {
    target: egui::Pos2,
    settings: egui::LayerId,
    query: egui::LayerId,
}

fn app_with_query_over_settings() -> (Harness<'static, App>, QueryOverSettings) {
    let mut harness = ui_tests::app_with_a_recording();
    harness.set_size(egui::vec2(640.0, 440.0));
    let app = harness.state_mut();
    app.settings_open = true;
    app.settings_page = SettingsPage::Analysis;
    harness.run_steps(3);
    let target = harness
        .get_by_label_contains("Elevation mask")
        .rect()
        .center();
    let settings = egui::LayerId::new(
        egui::Order::Middle,
        egui::Id::unique(settings_ui::WINDOW_ID),
    );
    assert_eq!(harness.ctx.layer_id_at(target), Some(settings));
    harness.state_mut().query_window.open = true;
    harness.run_steps(3);
    let query = egui::LayerId::new(egui::Order::Middle, QueryWindow::window_id());
    harness.ctx.move_to_top(query);
    harness.step();
    assert_eq!(harness.ctx.layer_id_at(target), Some(query));
    (
        harness,
        QueryOverSettings {
            target,
            settings,
            query,
        },
    )
}
