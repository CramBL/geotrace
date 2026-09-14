use egui_kittest::kittest::Queryable as _;
use gt_test_utils::TestHarness;

use crate::app::App;
use crate::app::test_util;

/// The update prompt as a user installed via the shell/PowerShell installer
/// sees it: a prominent "Update and restart" plus lower-key Later / Skip.
#[test]
fn snapshot_update_prompt_self_update() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 400.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state_mut().update_checker =
        crate::app::update::UpdateChecker::available_for_test("0.2.0", true);
    harness.run();
    harness.snapshot_with_color_tolerance("update_prompt_self_update");
}

/// What a failed install reports in the cases below.
const UPDATE_INSTALL_FAILURE: &str = "the release asset could not be downloaded";

/// The prompt as it opens, with an update offered and no install started yet.
fn app_showing_the_update_prompt() -> TestHarness<'static, App> {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 400.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness.inner.state_mut().update_checker =
        crate::app::update::UpdateChecker::available_for_test("0.2.0", true);
    harness.inner.run_steps(4);
    harness
}

/// What the prompt shows once the install it started has failed: the reason
/// in the body, and a manual download beside the dismissal.
#[test]
fn snapshot_update_prompt_install_failed() {
    let mut harness = app_showing_the_update_prompt();
    harness
        .inner
        .state()
        .update_checker
        .report_a_failed_install_for_test(UPDATE_INSTALL_FAILURE);
    harness.inner.run_steps(4);

    harness.snapshot_with_color_tolerance("update_prompt_install_failed");
}

/// The install reports its outcome after the prompt has already opened.
#[test]
fn the_update_prompt_shows_the_reason_a_failed_install_gives_without_growing() {
    let mut harness = app_showing_the_update_prompt();

    harness
        .inner
        .state()
        .update_checker
        .report_a_failed_install_for_test(UPDATE_INSTALL_FAILURE);
    harness.inner.run_steps(4);

    let reason = harness
        .inner
        .get_by_label_contains(UPDATE_INSTALL_FAILURE)
        .rect();
    let dismissal = harness.inner.get_by_label("Later").rect();
    assert!(
        reason.bottom() <= dismissal.top(),
        "the reason the failed install gave is at {reason:?}, over the action row at \
         {dismissal:?}"
    );
}

/// A non-self-updatable build (Homebrew / MSI / manual download) exposes the
/// available version for the subtle menu-bar badge. A self-updatable build
/// has no badge version and gets the dialog.
#[test]
fn non_self_update_uses_badge_not_dialog() {
    let badge = crate::app::update::UpdateChecker::available_for_test("0.2.0", false);
    assert_eq!(badge.badge_version().as_deref(), Some("0.2.0"));

    let self_updatable = crate::app::update::UpdateChecker::available_for_test("0.2.0", true);
    assert_eq!(self_updatable.badge_version(), None);
}
