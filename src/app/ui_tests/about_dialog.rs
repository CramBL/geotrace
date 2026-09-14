use egui_kittest::{Harness, kittest::Queryable as _};
use gt_test_utils::{HarnessInteraction as _, TestHarness};

use crate::app::test_util;
use crate::app::test_util::harness::TEST_APP_VERSION;

#[test]
fn snapshot_about_dialog() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(1024.0, 768.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state_mut().about_open = true;
    harness.run();
    // The dialog must render the injected placeholder, never the live crate
    // version - otherwise every release bump would diff the snapshot. If this
    // regresses to `env!`, the snapshot below diffs and this asserts too.
    assert!(
        harness
            .inner
            .query_by_label_contains(TEST_APP_VERSION)
            .is_some(),
        "the dialog must render the injected placeholder version"
    );
    assert!(
        harness
            .inner
            .query_by_label_contains(&format!("GeoTrace {}", env!("CARGO_PKG_VERSION")))
            .is_none(),
        "the live crate version must never reach the rendered dialog"
    );
    harness.snapshot_with_color_tolerance("about_dialog");
}

#[test]
fn snapshot_file_menu_open() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(400.0, 300.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.get_by_label("File").click();
    harness.run();
    harness.snapshot_with_color_tolerance("file_menu_open");
}

/// The File menu is the sole route to the About dialog: opening the menu and
/// clicking the entry must raise it.
#[test]
fn file_menu_opens_about_dialog() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();

    harness.get_by_label("File").click();
    harness.run_steps(2);
    harness.get_by_label("About GeoTrace").click();
    harness.run_steps(2);

    assert!(harness.state().about_open, "the menu entry must open About");
}

/// A line that only states something shows the ordinary cursor: labels do not
/// select anywhere in the app. Text a reader copies out opts back in, and
/// shows the I-beam over it.
#[rstest::rstest]
#[case::tagline("GPS/GNSS navigation data visualizer", egui::CursorIcon::Default)]
#[case::version(TEST_APP_VERSION, egui::CursorIcon::Text)]
fn the_about_dialog_shows_the_text_cursor_only_over_the_version(
    #[case] label: &str,
    #[case] expected: egui::CursorIcon,
) {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    harness.state_mut().about_open = true;
    harness.run_steps(2);

    let line = harness.get_by_label_contains(label).rect().center();
    harness.hover_at_and_settle(line, 5);

    assert_eq!(
        harness.output().platform_output.cursor_icon,
        expected,
        "hovering {label:?} should request {expected:?}"
    );
}

#[test]
fn about_dialog_closes_on_escape() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    harness.state_mut().about_open = true;
    harness.step();

    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();

    assert!(!harness.state().about_open, "Escape must close the dialog");
}
