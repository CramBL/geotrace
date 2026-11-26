use std::fs;

use chrono::Duration;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use gt_pending_writes::WriteAccess;
use gt_session_bridge::{
    AnalysisSettingKey, ErrorCode, ProcessingSettingKey, Reply, Request, SetSetting, SettingApply,
    SettingInput, SettingKey, SettingSelection, SettingUnit, SettingValue,
};
use gt_test_utils::{HarnessInteraction as _, TestHarness};
use rstest::rstest;

use crate::app::settings_ui::search::QUERY_FIELD_ID;
use crate::app::test_util::harness;
use crate::app::test_util::session;
use crate::app::{App, ui_tests};
use crate::settings::Settings;

#[test]
fn a_session_analysis_edit_updates_live_values_and_keeps_the_query_draft() {
    let mut harness = ui_tests::app_with_a_recording();
    let generation = harness
        .state()
        .shared
        .borrow()
        .loaded_files
        .data_generation();
    let draft = "points | where eph < 20 m";
    harness.state_mut().query_window.set_text(draft.to_owned());
    let key = SettingKey::Analysis(AnalysisSettingKey::ElevationMask);
    let request = Request::SetSetting(SetSetting {
        key,
        value: SettingInput::Number(18.5),
    });
    let reply = session::execute(&mut harness, request);
    let Reply::Setting { setting } = &reply else {
        panic!("expected the effective elevation mask");
    };
    assert_eq!(
        setting.value,
        SettingValue::Number {
            value: Some(18.5),
            minimum: 0.0,
            maximum: 90.0,
            unit: SettingUnit::Degrees,
        }
    );
    assert_eq!(setting.apply, SettingApply::LiveWithDeferredMarkers);
    assert_eq!(setting, &harness.state().describe_setting(key));
    let app = harness.state();
    assert_eq!(
        app.shared.borrow().plot_state.analysis.elevation_mask_deg,
        18.5
    );
    assert_eq!(app.loader.analysis_config.elevation_mask_deg, 18.5);
    assert_eq!(
        app.processing_config
            .generated_markers
            .slip_elevation_mask_deg,
        18.5
    );
    assert_eq!(app.query_window.text(), draft);
    assert!(!app.query_window.open);
    assert!(!app.settings_open);

    let selection = SettingSelection { key };
    assert_eq!(
        session::execute(&mut harness, Request::OpenSetting(selection)),
        Reply::SettingOpenQueued
    );
    harness.run_steps(3);
    harness
        .query_all_by_role(Role::SpinButton)
        .find(|node| node.accesskit_node().numeric_value() == Some(18.5))
        .unwrap()
        .set_value_accesskit(35.0);
    assert!(matches!(
        session::execute(
            &mut harness,
            Request::SetSetting(SetSetting {
                key,
                value: SettingInput::Number(20.0),
            }),
        ),
        Reply::Setting { .. }
    ));
    harness.run_steps(2);
    let Reply::Setting { setting } =
        session::execute(&mut harness, Request::DescribeSetting(selection))
    else {
        panic!("expected the value from the visible editor");
    };
    assert_eq!(
        setting.value,
        SettingValue::Number {
            value: Some(35.0),
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
}

#[test]
fn setting_an_equivalent_zero_reports_the_existing_value() {
    let mut harness = ui_tests::app_with_a_recording();
    let key = SettingKey::Analysis(AnalysisSettingKey::ElevationMask);
    for input in [0.0, -0.0] {
        let reply = session::execute(
            &mut harness,
            Request::SetSetting(SetSetting {
                key,
                value: SettingInput::Number(input),
            }),
        );
        let Reply::Setting { setting } = reply else {
            panic!("expected the effective elevation mask");
        };
        let SettingValue::Number {
            value: Some(value), ..
        } = setting.value
        else {
            panic!("expected a number");
        };
        assert_eq!(value.to_bits(), 0.0_f64.to_bits());
    }
}

#[test]
fn processing_edits_keep_full_precision_and_defer_reprocessing() {
    let mut harness = ui_tests::app_with_a_recording();
    let generation = harness
        .state()
        .shared
        .borrow()
        .loaded_files
        .data_generation();
    let sigmas = 9.234_567_890_123;
    let edits = [
        (
            SettingKey::Processing(ProcessingSettingKey::ClockDiscontinuitySigmas),
            SettingInput::Number(sigmas),
            SettingValue::Number {
                value: Some(sigmas),
                minimum: 1.0,
                maximum: 20.0,
                unit: SettingUnit::RobustStandardDeviations,
            },
        ),
        (
            SettingKey::Processing(ProcessingSettingKey::TrackSplitGap),
            SettingInput::Number(432.0),
            SettingValue::Number {
                value: Some(432.0),
                minimum: 30.0,
                maximum: 604_800.0,
                unit: SettingUnit::Seconds,
            },
        ),
        (
            SettingKey::Processing(ProcessingSettingKey::DetectSlips),
            SettingInput::Boolean(false),
            SettingValue::Boolean { value: false },
        ),
    ];
    for (key, input, expected) in edits {
        let Reply::Setting { setting } = session::execute(
            &mut harness,
            Request::SetSetting(SetSetting { key, value: input }),
        ) else {
            panic!("expected the effective Processing setting");
        };
        assert_eq!(setting.value, expected);
        assert_eq!(setting.apply, SettingApply::DeferredProcessing);
    }
    let Reply::Settings { settings } = session::execute(&mut harness, Request::GetSettings) else {
        panic!("expected settings");
    };
    for (key, _, expected) in edits {
        assert_eq!(
            settings
                .iter()
                .find(|setting| setting.key == key)
                .unwrap()
                .value,
            expected
        );
    }
    let app = harness.state();
    assert_eq!(
        app.processing_config.track_layout.track_split_gap,
        Duration::seconds(432)
    );
    assert_eq!(
        app.processing_config
            .generated_markers
            .clock_discontinuity_sigmas,
        sigmas
    );
    assert!(!app.processing_config.generated_markers.detect_slips);
    assert_eq!(
        app.shared.borrow().loaded_files.data_generation(),
        generation
    );
}

#[rstest]
#[case::analysis(
    SettingKey::Analysis(AnalysisSettingKey::ElevationMask),
    18.5, "20.625", |saved: &Settings| f64::from(saved.analysis.elevation_mask_deg),
)]
#[case::processing(
    SettingKey::Processing(ProcessingSettingKey::ClockDiscontinuitySigmas),
    9.234_567_890_123, "8.765432109876", |saved: &Settings| saved.processing.clock_discontinuity_sigmas,
)]
fn the_numeric_editor_preserves_precision_on_focus_and_save(
    #[case] key: SettingKey,
    #[case] initial: f64,
    #[case] replacement: &str,
    #[case] saved_number: fn(&Settings) -> f64,
) {
    let (mut harness, config_path) = TestHarness::builder().eframe(harness::build_app);
    let app = &mut harness.inner;
    app.step();
    assert!(matches!(
        session::execute(
            app,
            Request::SetSetting(SetSetting {
                key,
                value: SettingInput::Number(initial),
            })
        ),
        Reply::Setting { .. }
    ));
    assert_eq!(
        session::execute(app, Request::OpenSetting(SettingSelection { key })),
        Reply::SettingOpenQueued
    );
    app.run_steps(3);

    let drag_id = self::click_numeric_setting(app, initial);
    app.step();
    let edit_text = app.get_by(|node| node.is_focused()).value();
    self::blur_numeric_setting(app);
    self::assert_numeric_setting(app.state(), key, initial);
    assert_eq!(edit_text.as_deref(), Some(initial.to_string().as_str()));

    assert_eq!(self::click_numeric_setting(app, initial), drag_id);
    app.input_mut()
        .events
        .push(egui::Event::Text(replacement.to_owned()));
    app.step();
    self::blur_numeric_setting(app);
    let expected = replacement.parse::<f64>().unwrap();
    self::assert_numeric_setting(app.state(), key, expected);
    app.state().flush_settings();
    let saved: Settings = toml::from_str(&fs::read_to_string(config_path).unwrap()).unwrap();
    assert_eq!(saved_number(&saved), expected);
}

fn click_numeric_setting(harness: &mut Harness<'_, App>, number: f64) -> egui::Id {
    let position = harness
        .query_all_by_role(Role::SpinButton)
        .find(|node| node.accesskit_node().numeric_value() == Some(number))
        .unwrap()
        .rect()
        .center();
    harness.click_at(position);
    let focused = harness.ctx.memory(|memory| memory.focused()).unwrap();
    assert!(
        harness
            .ctx
            .read_response(focused)
            .unwrap()
            .rect
            .contains(position)
    );
    focused
}

fn blur_numeric_setting(harness: &mut Harness<'_, App>) {
    let search = egui::Id::unique(QUERY_FIELD_ID);
    let position = harness.ctx.read_response(search).unwrap().rect.center();
    harness.click_at(position);
    harness.step();
    assert!(harness.ctx.memory(|memory| memory.has_focus(search)));
}

fn assert_numeric_setting(app: &App, key: SettingKey, expected: f64) {
    let SettingValue::Number { value, .. } = app.describe_setting(key).value else {
        panic!("expected a numeric setting");
    };
    assert_eq!(value, Some(expected));
}

#[test]
fn invalid_and_inactive_setting_edits_preserve_the_app_snapshot() {
    let mut harness = ui_tests::app_with_a_recording();
    let detection = SettingKey::Processing(ProcessingSettingKey::DetectClockDiscontinuities);
    assert!(matches!(
        session::execute(
            &mut harness,
            Request::SetSetting(SetSetting {
                key: detection,
                value: SettingInput::Boolean(false),
            }),
        ),
        Reply::Setting { .. }
    ));
    let before = harness.state().collect_snapshot();
    let mask = SettingKey::Analysis(AnalysisSettingKey::ElevationMask);
    let gap = SettingKey::Processing(ProcessingSettingKey::TrackSplitGap);
    let sigmas = SettingKey::Processing(ProcessingSettingKey::ClockDiscontinuitySigmas);
    for (key, value, error) in [
        (
            mask,
            SettingInput::Boolean(true),
            ErrorCode::InvalidSettingType,
        ),
        (
            detection,
            SettingInput::Number(0.0),
            ErrorCode::InvalidSettingType,
        ),
        (
            mask,
            SettingInput::Number(90.000_001),
            ErrorCode::InvalidSettingValue,
        ),
        (
            mask,
            SettingInput::Number(f64::NAN),
            ErrorCode::InvalidSettingValue,
        ),
        (
            gap,
            SettingInput::Number(f64::INFINITY),
            ErrorCode::InvalidSettingValue,
        ),
        (
            gap,
            SettingInput::Number(30.5),
            ErrorCode::InvalidSettingValue,
        ),
        (
            sigmas,
            SettingInput::Number(6.0),
            ErrorCode::InactiveSetting,
        ),
    ] {
        assert_eq!(
            session::execute(&mut harness, Request::SetSetting(SetSetting { key, value })),
            Reply::Error { code: error }
        );
        assert!(harness.state().collect_snapshot() == before);
    }
}

#[rstest]
#[case::owner(WriteAccess::Owner)]
#[case::read_only(WriteAccess::ReadOnly)]
fn a_session_edit_uses_the_settings_write_access(#[case] access: WriteAccess) {
    let (mut harness, config_path) = TestHarness::builder()
        .eframe(|cc, path, fading| harness::build_app_with_write_access(cc, path, fading, access));
    harness.inner.step();
    let key = SettingKey::Processing(ProcessingSettingKey::TrackSplitGap);
    assert!(matches!(
        session::execute(
            &mut harness.inner,
            Request::SetSetting(SetSetting {
                key,
                value: SettingInput::Number(765.0),
            }),
        ),
        Reply::Setting { .. }
    ));
    assert_eq!(
        harness
            .inner
            .state()
            .processing_config
            .track_layout
            .track_split_gap,
        Duration::seconds(765)
    );
    assert!(harness.inner.state().config.time_until_flush().is_some());
    assert!(
        harness
            .inner
            .step_until(|harness| harness.state().config.time_until_flush().is_none())
    );
    if access == WriteAccess::Owner {
        let saved: Settings = toml::from_str(&fs::read_to_string(config_path).unwrap()).unwrap();
        assert_eq!(saved.processing.track_split_gap_seconds, 765);
    } else {
        assert!(!config_path.exists());
    }
}
