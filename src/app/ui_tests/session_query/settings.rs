use egui::accesskit::Role;
use egui_kittest::kittest::NodeT as _;
use egui_kittest::kittest::Queryable as _;
use gt_session_bridge::{
    Reply, Request, SettingApply, SettingKey, SettingSelection, SettingUnit, SettingValue,
};
use strum::IntoEnumIterator as _;

use super::ui_tests;
use crate::app::settings_ui::{self, SettingsPage};

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
    let Reply::Settings { settings } = super::execute(&mut harness, Request::GetSettings) else {
        panic!("expected settings");
    };
    let keys = SettingKey::iter();
    assert_eq!(settings.len(), keys.len());
    for key in keys {
        assert_eq!(
            settings.iter().filter(|setting| setting.key == key).count(),
            1
        );
    }
    let key = SettingKey::AnalysisElevationMask;
    let setting = settings.iter().find(|setting| setting.key == key).unwrap();
    let number = SettingValue::Number {
        value: Some(startup_value),
        minimum: 0.0,
        maximum: 90.0,
        unit: SettingUnit::Degrees,
    };
    assert_eq!(setting.value, number);
    assert_eq!(setting.label, "Elevation mask");
    assert_eq!(setting.apply, SettingApply::LiveWithDeferredMarkers);
    let toggle = settings
        .iter()
        .find(|setting| setting.key == SettingKey::AnalysisMarkBackwardTimeSteps)
        .unwrap();
    assert_eq!(toggle.value, SettingValue::Boolean { value: true });
    assert_eq!(toggle.apply, SettingApply::Live);
    assert!(!harness.state().settings_open);

    let select = SettingSelection { key };
    assert_eq!(
        super::execute(&mut harness, Request::OpenSetting(select)),
        Reply::SettingOpenQueued
    );
    harness.run_steps(3);
    assert_eq!(
        super::execute(&mut harness, Request::DescribeSetting(select)),
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
    let Reply::Setting { setting } = super::execute(&mut harness, Request::DescribeSetting(select))
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

    let key = SettingKey::AnalysisMarkBackwardTimeSteps;
    assert_eq!(
        super::execute(&mut harness, Request::OpenSetting(SettingSelection { key })),
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

    let key = SettingKey::AnalysisElevationMask;
    assert_eq!(
        super::execute(&mut harness, Request::OpenSetting(SettingSelection { key })),
        Reply::SettingOpenQueued
    );
    harness.run_steps(3);
    let target = harness.get_by_label_contains(label).rect().center();
    assert_eq!(harness.ctx.layer_id_at(target), Some(settings));
    assert!(harness.state().query_window.open);
}
