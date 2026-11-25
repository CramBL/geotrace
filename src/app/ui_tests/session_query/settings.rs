use chrono::Duration;
use egui::accesskit::Role;
use egui::emath::GuiRounding as _;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use gt_session_bridge::{
    AnalysisSettingKey, ProcessingSettingKey, Reply, Request, SettingApply, SettingKey,
    SettingSelection, SettingUnit, SettingValue,
};
use gt_test_utils::HarnessInteraction as _;
use strum::IntoEnumIterator as _;

use crate::app::settings_ui::{self, SettingsPage};
use crate::app::test_util::session;
use crate::app::ui_tests;

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
        Reply::SettingOpenQueued
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
        session::execute(&mut harness, Request::OpenSetting(SettingSelection { key })),
        Reply::SettingOpenQueued
    );
    harness
        .input_mut()
        .events
        .push(ui_tests::key_press(egui::Key::Escape));
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
        Reply::SettingOpenQueued
    );
    harness.run_steps(3);
    let target = harness.get_by_label_contains(label).rect().center();
    assert_eq!(harness.ctx.layer_id_at(target), Some(settings));
    assert!(harness.state().query_window.open);
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
        Reply::SettingOpenQueued
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
        Reply::SettingOpenQueued
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
        Reply::SettingOpenQueued
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
