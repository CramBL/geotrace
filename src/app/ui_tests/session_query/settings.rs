use chrono::Duration;
use egui::accesskit::Role;
use egui::emath::GuiRounding as _;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use gt_session_bridge::{
    AnalysisSettingKey, ProcessingSettingKey, Reply, Request, SettingApply, SettingKey,
    SettingSelection, SettingUnit, SettingValue,
};
use gt_test_utils::HarnessInteraction as _;
use rstest::rstest;
use strum::IntoEnumIterator as _;

use crate::app::settings_ui::{self, SettingsPage};
use crate::app::test_util::{harness, session};
use crate::app::{App, ui_tests};

mod edit;

#[test]
fn analysis_settings_report_live_values_and_share_the_control_bounds() {
    let mut harness = ui_tests::app_with_a_recording();
    let app = harness.state();
    let startup_value = 100.0;
    app.shared
        .borrow_mut()
        .plot_state
        .analysis
        .elevation_mask_deg = startup_value;
    let generation = app.shared.borrow().loaded_files.data_generation();
    let Reply::Settings { settings } = session::execute(&mut harness, Request::GetSettings) else {
        panic!("expected settings");
    };
    let keys = AnalysisSettingKey::iter()
        .map(SettingKey::Analysis)
        .chain(ProcessingSettingKey::iter().map(SettingKey::Processing));
    assert_eq!(settings.len(), keys.clone().count());
    for key in keys {
        assert_eq!(
            settings.iter().filter(|setting| setting.key == key).count(),
            1
        );
    }
    let key = SettingKey::Analysis(AnalysisSettingKey::ElevationMask);
    let setting = settings.iter().find(|setting| setting.key == key).unwrap();
    let number = SettingValue::Number {
        value: Some(f64::from(startup_value)),
        minimum: 0.0,
        maximum: 90.0,
        unit: SettingUnit::Degrees,
    };
    assert_eq!(setting.value, number);
    assert_eq!(setting.label, "Elevation mask");
    assert_eq!(setting.apply, SettingApply::LiveWithDeferredMarkers);
    let toggle = settings
        .iter()
        .find(|setting| {
            setting.key == SettingKey::Analysis(AnalysisSettingKey::MarkBackwardTimeSteps)
        })
        .unwrap();
    assert_eq!(toggle.value, SettingValue::Boolean { value: true });
    assert_eq!(toggle.apply, SettingApply::Live);
    assert!(!harness.state().settings_open);

    let select = SettingSelection { key };
    assert_eq!(
        session::execute(&mut harness, Request::OpenSetting(select)),
        Reply::SettingOpened { key: select.key }
    );
    harness.run_steps(3);
    assert_eq!(
        session::execute(&mut harness, Request::DescribeSetting(select)),
        Reply::Setting {
            setting: setting.clone()
        }
    );
    harness.get_by_label_contains("Elevation mask");
    harness
        .query_all_by_role(Role::SpinButton)
        .find(|node| node.accesskit_node().numeric_value() == Some(f64::from(startup_value)))
        .unwrap()
        .set_value_accesskit(180.0);
    harness.run_steps(2);
    let Reply::Setting { setting } =
        session::execute(&mut harness, Request::DescribeSetting(select))
    else {
        panic!("expected setting");
    };
    assert_eq!(
        setting.value,
        SettingValue::Number {
            value: Some(90.0),
            minimum: 0.0,
            maximum: 90.0,
            unit: SettingUnit::Degrees,
        }
    );
    assert_eq!(
        harness
            .state()
            .shared
            .borrow()
            .loaded_files
            .data_generation(),
        generation
    );
    assert_eq!(
        harness
            .state()
            .processing_config
            .generated_markers
            .slip_elevation_mask_deg,
        90.0
    );
}

#[test]
fn opening_a_setting_reveals_the_row_after_current_input() {
    let mut harness = ui_tests::app_with_a_recording();
    harness.set_size(egui::vec2(520.0, 155.0));
    let app = harness.state_mut();
    app.settings_open = true;
    app.settings_page = SettingsPage::Analysis;
    harness.run_steps(4);
    let label = "Mark backward time steps";
    let viewport = harness.ctx.content_rect();
    assert!(!viewport.contains_rect(harness.get_by_label_contains(label).rect()));

    let key = SettingKey::Analysis(AnalysisSettingKey::MarkBackwardTimeSteps);
    assert_eq!(
        session::exchange_queued_request(
            &mut harness,
            &Request::OpenSetting(SettingSelection { key }),
            |harness, _| {
                harness
                    .input_mut()
                    .events
                    .push(ui_tests::key_press(egui::Key::Escape));
                harness.step();
            },
        ),
        Reply::SettingOpened { key }
    );
    harness.run_steps(4);
    let row = harness.get_by_label_contains(label).rect();
    assert!(
        viewport.contains_rect(row),
        "{row:?} must fit within {viewport:?}"
    );
    assert!(harness.state().settings_open);
    assert_eq!(harness.state().settings_page, SettingsPage::Analysis);
    assert!(harness.state().setting_to_reveal.is_none());
}

#[test]
fn a_setting_reply_confirms_the_selected_page_without_painting() {
    let mut harness = ui_tests::app_with_a_recording();
    let key = SettingKey::Analysis(AnalysisSettingKey::ElevationMask);
    assert_eq!(
        session::exchange_queued_request(
            &mut harness,
            &Request::OpenSetting(SettingSelection { key }),
            |harness, _| {
                let _ = harness::step_minimized_logic(harness);
            },
        ),
        Reply::SettingOpened { key },
    );
    assert!(harness.state().settings_open);
    assert_eq!(harness.state().settings_page, SettingsPage::Analysis);
    assert!(harness.state().setting_to_reveal.is_some());

    harness.run_steps(3);
    harness.get_by_label_contains("Elevation mask");
    assert!(harness.state().setting_to_reveal.is_none());
}

#[test]
fn opening_a_setting_raises_the_window_above_overlapping_windows() {
    let mut harness = ui_tests::app_with_a_recording();
    harness.set_size(egui::vec2(640.0, 440.0));
    let app = harness.state_mut();
    app.settings_open = true;
    app.settings_page = SettingsPage::Analysis;
    harness.run_steps(3);
    let label = "Elevation mask";
    let target = harness.get_by_label_contains(label).rect().center();
    let settings = egui::LayerId::new(
        egui::Order::Middle,
        egui::Id::unique(settings_ui::WINDOW_ID),
    );
    assert_eq!(harness.ctx.layer_id_at(target), Some(settings));

    harness.state_mut().query_window.open = true;
    harness.run_steps(3);
    let query = egui::LayerId::new(egui::Order::Middle, egui::Id::unique(Some("Query")));
    harness.ctx.move_to_top(query);
    harness.step();
    assert_eq!(harness.ctx.layer_id_at(target), Some(query));

    let key = SettingKey::Analysis(AnalysisSettingKey::ElevationMask);
    assert_eq!(
        session::execute(&mut harness, Request::OpenSetting(SettingSelection { key })),
        Reply::SettingOpened { key }
    );
    harness.run_steps(3);
    let target = harness.get_by_label_contains(label).rect().center();
    assert_eq!(harness.ctx.layer_id_at(target), Some(settings));
    assert!(harness.state().query_window.open);
    assert_eq!(
        session::visible_action_notices(&harness),
        ["MCP revealed Elevation mask"]
    );

    harness.ctx.move_to_top(query);
    harness.step();
    harness.state_mut().query_window.open = false;
    harness.run_steps(2);
    assert_eq!(harness.ctx.layer_id_at(target), Some(settings));
    assert_eq!(
        session::execute(&mut harness, Request::OpenSetting(SettingSelection { key })),
        Reply::SettingOpened { key }
    );
    harness.run_steps(3);
    assert_eq!(
        session::visible_action_notices(&harness),
        ["MCP revealed Elevation mask"]
    );
}

#[test]
fn a_setting_reveal_reports_an_overlap_over_the_input() {
    let mut harness = ui_tests::app_with_a_recording();
    harness.set_size(egui::vec2(1200.0, 700.0));
    let app = harness.state_mut();
    app.settings_open = true;
    app.settings_page = SettingsPage::Analysis;
    harness.run_steps(3);
    let label = harness.get_by_label_contains("Elevation mask").rect();
    let input = harness
        .query_all_by_role(Role::SpinButton)
        .find(|input| label.y_range().contains(input.rect().center().y))
        .unwrap()
        .rect();
    let row_center = label.union(input).center();

    let overlap_left = f32::midpoint(row_center.x, input.min.x);
    self::move_query_window(&mut harness, egui::pos2(overlap_left, row_center.y));
    let settings_layer = egui::LayerId::new(
        egui::Order::Middle,
        egui::Id::unique(settings_ui::WINDOW_ID),
    );
    let query_layer = egui::LayerId::new(egui::Order::Middle, egui::Id::unique(Some("Query")));
    assert_eq!(harness.ctx.layer_id_at(row_center), Some(settings_layer));
    assert_eq!(harness.ctx.layer_id_at(input.center()), Some(query_layer));

    let key = SettingKey::Analysis(AnalysisSettingKey::ElevationMask);
    assert_eq!(
        session::execute(&mut harness, Request::OpenSetting(SettingSelection { key })),
        Reply::SettingOpened { key }
    );
    harness.run_steps(3);
    assert_eq!(
        harness.ctx.layer_id_at(input.center()),
        Some(settings_layer)
    );
    assert_eq!(
        session::visible_action_notices(&harness),
        ["MCP revealed Elevation mask"]
    );
}

#[rstest]
#[case::toggle(ProcessingSettingKey::DetectClockDiscontinuities, &[])]
#[case::sensitivity(ProcessingSettingKey::ClockDiscontinuitySigmas, &["MCP revealed Jump sensitivity"])]
fn an_overlap_over_sensitivity_applies_only_to_its_own_reveal(
    #[case] key: ProcessingSettingKey,
    #[case] notices: &[&str],
) {
    let mut harness = ui_tests::app_with_a_recording();
    harness.set_size(egui::vec2(1200.0, 800.0));
    let request = |key| {
        Request::OpenSetting(SettingSelection {
            key: SettingKey::Processing(key),
        })
    };
    assert_eq!(
        session::execute(
            &mut harness,
            request(ProcessingSettingKey::ClockDiscontinuitySigmas)
        ),
        Reply::SettingOpened {
            key: SettingKey::Processing(ProcessingSettingKey::ClockDiscontinuitySigmas)
        }
    );
    harness.run_steps(5);
    assert!(harness.step_until(|harness| session::visible_action_notices(harness).is_empty()));
    let sensitivity = harness.get_by_label("Jump sensitivity").rect();
    let toggle = harness
        .query_all_by_role(Role::CheckBox)
        .find(|node| sensitivity.y_range().contains(node.rect().center().y))
        .unwrap()
        .rect();
    let overlap_left = f32::midpoint(toggle.max.x, sensitivity.min.x);
    self::move_query_window(
        &mut harness,
        egui::pos2(overlap_left, sensitivity.center().y),
    );
    let settings_layer = egui::LayerId::new(
        egui::Order::Middle,
        egui::Id::unique(settings_ui::WINDOW_ID),
    );
    let query_layer = egui::LayerId::new(egui::Order::Middle, egui::Id::unique(Some("Query")));
    assert_eq!(
        harness.ctx.layer_id_at(toggle.center()),
        Some(settings_layer)
    );
    assert_eq!(
        harness.ctx.layer_id_at(sensitivity.center()),
        Some(query_layer)
    );

    assert_eq!(
        session::execute(&mut harness, request(key)),
        Reply::SettingOpened {
            key: SettingKey::Processing(key)
        }
    );
    harness.run_steps(3);
    assert_eq!(session::visible_action_notices(&harness), notices);
}

fn move_query_window(harness: &mut Harness<'_, App>, target: egui::Pos2) {
    harness.state_mut().query_window.open = true;
    harness.run_steps(3);
    let query = harness.get_by_role_and_label(Role::Window, "Query");
    let query_rect = query.rect();
    let title = egui::pos2(
        query_rect.center().x,
        query
            .get_by_role_and_label(Role::Button, "Hide")
            .rect()
            .center()
            .y,
    );
    let delta = egui::vec2(target.x - query_rect.min.x, target.y - title.y);
    harness.press_drag_release(title, delta, 3);
}

#[rstest]
#[case::idle(false)]
#[case::wheel(true)]
fn a_setting_notice_reports_only_the_requested_scroll(#[case] wheel: bool) {
    let mut harness = self::narrow_scrolled_analysis_page();
    let label = "Elevation mask";
    let before = harness.get_by_label_contains(label).rect();
    let key = SettingKey::Analysis(AnalysisSettingKey::ElevationMask);
    let request = Request::OpenSetting(SettingSelection { key });
    assert_eq!(
        session::execute(&mut harness, request.clone()),
        Reply::SettingOpened { key }
    );
    harness.run_steps(4);
    let after = harness.get_by_label_contains(label).rect();
    assert!(after.min.x > before.min.x, "{before:?} -> {after:?}");
    assert_eq!(
        session::visible_action_notices(&harness),
        ["MCP revealed Elevation mask"]
    );

    harness
        .input_mut()
        .events
        .push(self::horizontal_scroll(60.0));
    harness.run_steps(10);
    assert!(harness.step_until(|harness| session::visible_action_notices(harness).is_empty()));
    let settled = harness.get_by_label_contains(label).rect();
    assert!(settled.min.x > after.min.x);
    assert_eq!(
        session::execute(&mut harness, request),
        Reply::SettingOpened { key }
    );
    if wheel {
        harness
            .input_mut()
            .events
            .push(self::horizontal_scroll(16.0));
    }
    assert!(harness.step_until(|harness| harness.state().setting_to_reveal.is_none()));
    harness.run_steps(3);
    assert_eq!(
        harness.get_by_label_contains(label).rect().min.x > settled.min.x,
        wheel
    );
    assert!(session::visible_action_notices(&harness).is_empty());
}

#[rstest]
#[case::visible(
    AnalysisSettingKey::ElevationMask,
    Some("MCP revealed Elevation mask"),
    false
)]
#[case::native_only(AnalysisSettingKey::SnrDropThreshold, None, true)]
#[case::reveal(
    AnalysisSettingKey::MarkBackwardTimeSteps,
    Some("MCP revealed Mark backward time steps"),
    true
)]
fn a_setting_reveal_waits_for_a_keyboard_edit(
    #[case] key: AnalysisSettingKey,
    #[case] notice: Option<&str>,
    #[case] clipped_caret: bool,
) {
    let mut harness = self::narrow_scrolled_analysis_page();
    let label = "Elevation mask";
    let snr_label = harness.get_by_label_contains("SNR drop threshold").rect();
    let snr_input = harness
        .query_all_by_role(Role::SpinButton)
        .find(|node| snr_label.y_range().contains(node.rect().center().y))
        .unwrap()
        .rect();
    harness.click_at(snr_input.center());
    harness.run_steps(2);
    assert!(harness.ctx.egui_wants_keyboard_input());
    if clipped_caret {
        self::clip_the_focused_caret(&mut harness, snr_label);
    }
    let before = harness.get_by_label_contains(label).rect();
    assert_eq!(
        session::execute(
            &mut harness,
            Request::OpenSetting(SettingSelection {
                key: SettingKey::Analysis(key),
            })
        ),
        Reply::SettingOpened {
            key: SettingKey::Analysis(key)
        }
    );
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("12".to_owned()));
    harness.step();
    assert_eq!(
        harness
            .state()
            .shared
            .borrow()
            .plot_state
            .analysis
            .snr_drop_db,
        12.0
    );
    assert!(harness.state().setting_to_reveal.is_some());
    assert!(session::visible_action_notices(&harness).is_empty());

    harness.step();
    let native = harness.get_by_label_contains(label).rect();
    if clipped_caret {
        assert_ne!(native, before);
    }
    harness.run_steps(3);
    let revealed = harness.get_by_label_contains(label).rect();
    assert_eq!(revealed != native, notice.is_some());
    let now = harness.ctx.input(|input| input.time);
    harness.input_mut().time = Some(now + 0.4);
    harness.step();
    assert_eq!(harness.get_by_label_contains(label).rect(), revealed);
    assert_eq!(
        session::visible_action_notices(&harness),
        notice.into_iter().collect::<Vec<_>>()
    );
}

fn clip_the_focused_caret(harness: &mut Harness<'_, App>, label: egui::Rect) {
    let viewport_right = harness.ctx.content_rect().right();
    let visible_caret = harness.output().platform_output.ime.unwrap().cursor_rect;
    harness.hover_at(egui::pos2(viewport_right - 40.0, label.center().y));
    harness.step();
    harness.input_mut().events.push(self::horizontal_scroll(
        viewport_right - visible_caret.left() - 26.0,
    ));
    harness.run_steps(10);
    let focused = harness.ctx.memory(|memory| memory.focused()).unwrap();
    let ime = harness
        .output()
        .platform_output
        .ime
        .expect("partially visible editor");
    let response = harness.ctx.read_response(focused).unwrap();
    assert!(response.interact_rect.is_positive());
    assert!(ime.cursor_rect.left() > response.interact_rect.right());
}

fn narrow_scrolled_analysis_page() -> Harness<'static, App> {
    let mut harness = ui_tests::app_with_a_recording();
    harness.set_size(egui::vec2(304.0, 500.0));
    let app = harness.state_mut();
    app.settings_open = true;
    app.settings_page = SettingsPage::Analysis;
    harness.run_steps(3);
    let label = "Elevation mask";
    let start = harness.get_by_label_contains(label).rect();
    harness.hover_at(start.center());
    harness.step();
    harness
        .input_mut()
        .events
        .push(self::horizontal_scroll(-1000.0));
    harness.run_steps(10);
    assert!(harness.get_by_label_contains(label).rect().min.x < start.min.x);
    harness
}

fn horizontal_scroll(delta: f32) -> egui::Event {
    egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        source: egui::MouseWheelSource::Wheel,
        delta: egui::vec2(delta, 0.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
    }
}

#[test]
fn opening_a_disabled_processing_control_preserves_settings_and_loaded_data() {
    let mut harness = ui_tests::app_with_a_recording();
    harness.set_size(egui::vec2(520.0, 155.0));
    let app = harness.state_mut();
    app.processing_config.track_layout.track_split_gap =
        Duration::days(8) + Duration::milliseconds(750);
    app.processing_config
        .generated_markers
        .detect_clock_discontinuities = false;
    app.processing_config
        .generated_markers
        .clock_discontinuity_sigmas = 27.125_000_000_1;
    let config = app.processing_config;
    let generation = app.shared.borrow().loaded_files.data_generation();

    let gap = SettingSelection {
        key: SettingKey::Processing(ProcessingSettingKey::TrackSplitGap),
    };
    let Reply::Setting { setting } = session::execute(&mut harness, Request::DescribeSetting(gap))
    else {
        panic!("expected the split gap");
    };
    assert_eq!(
        setting.value,
        SettingValue::Number {
            value: Some(config.track_layout.track_split_gap.as_seconds_f64()),
            minimum: 30.0,
            maximum: 604_800.0,
            unit: SettingUnit::Seconds,
        }
    );
    assert_eq!(setting.apply, SettingApply::DeferredProcessing);

    let select = SettingSelection {
        key: SettingKey::Processing(ProcessingSettingKey::ClockDiscontinuitySigmas),
    };
    assert_eq!(
        session::execute(&mut harness, Request::OpenSetting(select)),
        Reply::SettingOpened { key: select.key }
    );
    assert!(harness.step_until(|harness| {
        harness
            .query_by_label("Jump sensitivity")
            .is_some_and(|node| harness.ctx.content_rect().contains_rect(node.rect()))
    }));
    let sensitivity = harness.get_by_label("Jump sensitivity");
    assert!(sensitivity.accesskit_node().is_disabled());
    assert_eq!(harness.state().settings_page, SettingsPage::Processing);
    assert!(harness.state().setting_to_reveal.is_none());
    assert_eq!(harness.state().processing_config, config);
    assert_eq!(
        harness
            .state()
            .shared
            .borrow()
            .loaded_files
            .data_generation(),
        generation
    );

    let Reply::Setting { setting } =
        session::execute(&mut harness, Request::DescribeSetting(select))
    else {
        panic!("expected jump sensitivity");
    };
    assert_eq!(
        setting.value,
        SettingValue::Number {
            value: Some(config.generated_markers.clock_discontinuity_sigmas),
            minimum: 1.0,
            maximum: 20.0,
            unit: SettingUnit::RobustStandardDeviations,
        }
    );
    assert_eq!(setting.page, "Processing");
    assert_eq!(setting.label, "Jump sensitivity");
    assert_eq!(setting.apply, SettingApply::DeferredProcessing);
}

#[test]
fn opening_the_track_split_gap_reveals_all_duration_fields() {
    let mut harness = ui_tests::app_with_a_recording();
    harness.set_size(egui::vec2(520.0, 440.0));
    let key = SettingKey::Processing(ProcessingSettingKey::TrackSplitGap);
    assert_eq!(
        session::execute(&mut harness, Request::OpenSetting(SettingSelection { key })),
        Reply::SettingOpened { key }
    );
    assert!(harness.step_until(|harness| {
        let Some(gap_label) = harness.query_by_label_contains("Track split gap") else {
            return false;
        };
        let row = gap_label.rect().y_range();
        let Some(clip_rect) = harness.output().shapes.iter().find_map(|clipped| {
            let egui::Shape::Text(text) = &clipped.shape else {
                return None;
            };
            (row.contains(text.pos.y) && text.galley.text() == "d").then_some(clipped.clip_rect)
        }) else {
            return false;
        };
        let pixels_per_point = harness.ctx.pixels_per_point();
        harness
            .query_all_by_role(Role::SpinButton)
            .filter(|node| row.contains(node.rect().center().y))
            .filter(|node| clip_rect.contains_rect(node.rect().round_to_pixels(pixels_per_point)))
            .count()
            == 4
    }));
}

#[test]
fn processing_settings_report_edits_with_the_control_bounds() {
    let mut harness = ui_tests::app_with_a_recording();
    let generation = harness
        .state()
        .shared
        .borrow()
        .loaded_files
        .data_generation();
    let select = SettingSelection {
        key: SettingKey::Processing(ProcessingSettingKey::ClockDiscontinuitySigmas),
    };
    assert_eq!(
        session::execute(&mut harness, Request::OpenSetting(select)),
        Reply::SettingOpened { key: select.key }
    );
    harness.run_steps(3);
    harness
        .get_by_label("Jump sensitivity")
        .set_value_accesskit(25.0);
    harness.run_steps(2);
    let Reply::Setting { setting } =
        session::execute(&mut harness, Request::DescribeSetting(select))
    else {
        panic!("expected jump sensitivity");
    };
    assert_eq!(
        setting.value,
        SettingValue::Number {
            value: Some(20.0),
            minimum: 1.0,
            maximum: 20.0,
            unit: SettingUnit::RobustStandardDeviations,
        }
    );

    let split_gap = harness.get_by_label_contains("Track split gap").rect();
    harness
        .query_all_by_label("m")
        .find(|node| node.rect().y_range().contains(split_gap.center().y))
        .unwrap()
        .set_value_accesskit(0.0);
    harness.run_steps(2);
    assert_eq!(
        harness
            .state()
            .processing_config
            .track_layout
            .track_split_gap,
        Duration::seconds(30)
    );
    assert_eq!(
        harness
            .state()
            .shared
            .borrow()
            .loaded_files
            .data_generation(),
        generation
    );
}
