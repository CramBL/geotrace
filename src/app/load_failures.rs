use std::fmt;

use egui::{Button, Label, Window};
use egui_phosphor::regular::COPY as ICON_COPY;

use crate::app::modals::{self, DialogActionRow, DialogBody};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) enum LoadSource {
    Filename(String),
    PastedText,
    UnnamedDrop,
}

impl LoadSource {
    pub(super) fn into_filename(self) -> Option<String> {
        match self {
            Self::Filename(filename) => Some(filename),
            Self::PastedText | Self::UnnamedDrop => None,
        }
    }
}

impl fmt::Display for LoadSource {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Filename(filename) => f.write_str(filename),
            Self::PastedText => f.write_str("pasted log text"),
            Self::UnnamedDrop => f.write_str("unnamed dropped log"),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(super) struct LoadFailure {
    pub source: LoadSource,
    pub details: String,
}

impl LoadFailure {
    pub(super) fn new(source: LoadSource, error: impl fmt::Display) -> Self {
        Self {
            source,
            details: format!("{error:#}"),
        }
    }

    fn title(&self) -> String {
        format!("Failed to load {}", self.source)
    }

    fn copy_text(&self) -> String {
        format!("{}\n{}", self.title(), self.details)
    }
}

/// The load failures the user has not dismissed yet, oldest first.
#[derive(Default)]
pub(super) struct LoadFailures {
    retained: Vec<RetainedLoadFailure>,
    next_failure_id: u64,
}

impl LoadFailures {
    pub(super) fn push(&mut self, failure: LoadFailure) {
        self.retained.push(RetainedLoadFailure {
            failure_id: self.next_failure_id,
            failure,
        });
        self.next_failure_id += 1;
    }

    /// Shows one error window per failure, and removes the failures the user
    /// dismissed on this frame.
    pub(super) fn show_windows(&mut self, ctx: &egui::Context) {
        let failure_count = self.retained.len();
        let mut dismissed: Vec<u64> = Vec::new();
        let mut dismiss_all = false;
        for retained in &self.retained {
            match retained.show_window(ctx, failure_count) {
                FailureWindowAction::Dismiss => dismissed.push(retained.failure_id),
                FailureWindowAction::DismissAll => dismiss_all = true,
                FailureWindowAction::Keep => {}
            }
        }
        if dismiss_all {
            self.retained.clear();
        } else {
            self.retained
                .retain(|retained| !dismissed.contains(&retained.failure_id));
        }
    }

    #[cfg(test)]
    pub(super) fn iter(&self) -> impl Iterator<Item = &LoadFailure> {
        self.retained.iter().map(|retained| &retained.failure)
    }
}

struct RetainedLoadFailure {
    /// The window id of the failure: egui derives a window id from the title
    /// otherwise, and two failures of one source have the same title.
    failure_id: u64,
    failure: LoadFailure,
}

impl RetainedLoadFailure {
    fn show_window(&self, ctx: &egui::Context, failure_count: usize) -> FailureWindowAction {
        let failure = &self.failure;
        let mut action = FailureWindowAction::Keep;
        Window::new(failure.title())
            .id(self.window_id())
            .collapsible(false)
            .resizable(true)
            .min_width(LOAD_FAILURE_WINDOW_MIN_WIDTH)
            .show(ctx, |ui| {
                modals::dialog_body_above_the_action_row(
                    ui,
                    DialogBody::new(|ui| {
                        ui.add(Label::new(&failure.details).selectable(true).wrap());
                    }),
                    DialogActionRow::buttons(|ui| {
                        if ui.button(DISMISS_BUTTON_LABEL).clicked() {
                            action = FailureWindowAction::Dismiss;
                        }
                        let dismiss_all = ui
                            .add_enabled(failure_count > 1, Button::new(DISMISS_ALL_BUTTON_LABEL))
                            .on_hover_text(format!("Dismiss all {failure_count} load failures"))
                            .on_disabled_hover_text("No other load failed");
                        if dismiss_all.clicked() {
                            action = FailureWindowAction::DismissAll;
                        }
                        if ui
                            .button(format!("{ICON_COPY} {COPY_DETAILS_BUTTON_LABEL}"))
                            .on_hover_text("Copy the source and the error text")
                            .clicked()
                        {
                            ui.ctx().copy_text(failure.copy_text());
                        }
                    }),
                );
            });
        action
    }

    fn window_id(&self) -> egui::Id {
        egui::Id::new(LOAD_FAILURE_WINDOW_ID_SALT).with(self.failure_id)
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum FailureWindowAction {
    Dismiss,
    DismissAll,
    Keep,
}

const DISMISS_BUTTON_LABEL: &str = "Dismiss";

const DISMISS_ALL_BUTTON_LABEL: &str = "Dismiss all";

const COPY_DETAILS_BUTTON_LABEL: &str = "Copy details";

const LOAD_FAILURE_WINDOW_ID_SALT: &str = "load_failure_window";

const LOAD_FAILURE_WINDOW_MIN_WIDTH: f32 = 420.0;

#[cfg(test)]
mod tests {
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    use gt_test_utils::window_fit::{CRAMPED_VIEWPORT, NARROW_VIEWPORT, SHORT_VIEWPORT};
    use gt_test_utils::{
        AuditedWindow, ControlLabel, HarnessInteraction as _, TestHarness, WindowFitAssertions as _,
    };
    use rstest::rstest;

    use super::*;

    const PARSE_ERROR: &str =
        "Not a recognised log: no line has a timestamp in a known format (first line: \"boot\")";

    fn failure_of(source: LoadSource) -> LoadFailure {
        LoadFailure::new(source, PARSE_ERROR)
    }

    fn harness_showing(failures: Vec<LoadFailure>) -> TestHarness<'static, LoadFailures> {
        harness_showing_at(failures, egui::vec2(900.0, 600.0))
    }

    fn harness_showing_at(
        failures: Vec<LoadFailure>,
        viewport: egui::Vec2,
    ) -> TestHarness<'static, LoadFailures> {
        let mut retained = LoadFailures::default();
        for failure in failures {
            retained.push(failure);
        }
        let mut harness = TestHarness::builder().size(viewport).ui_state(
            |ui: &mut egui::Ui, failures: &mut LoadFailures| failures.show_windows(ui.ctx()),
            retained,
        );
        harness.run();
        harness
    }

    #[rstest]
    #[case::file(LoadSource::Filename("navsyncd.log".to_owned()), "Failed to load navsyncd.log")]
    #[case::pasted_text(LoadSource::PastedText, "Failed to load pasted log text")]
    #[case::unnamed_drop(LoadSource::UnnamedDrop, "Failed to load unnamed dropped log")]
    fn the_window_title_states_the_source_of_the_failure(
        #[case] source: LoadSource,
        #[case] title: &str,
    ) {
        let harness = harness_showing(vec![failure_of(source)]);

        let window = harness.inner.get_by_role_and_label(Role::Window, title);
        assert!(window.query_by_label(PARSE_ERROR).is_some());
    }

    #[test]
    fn copying_the_details_writes_the_title_above_the_error_text() {
        let mut harness = harness_showing(vec![failure_of(LoadSource::Filename(
            "navsyncd.log".to_owned(),
        ))]);

        harness
            .inner
            .get_by_label_contains(COPY_DETAILS_BUTTON_LABEL)
            .click();
        harness.step();

        assert_eq!(
            harness.inner.copied_text().as_deref(),
            Some(format!("Failed to load navsyncd.log\n{PARSE_ERROR}").as_str())
        );
    }

    #[test]
    fn dragging_across_the_details_selects_them_for_copying() {
        let mut harness = harness_showing(vec![failure_of(LoadSource::Filename(
            "navsyncd.log".to_owned(),
        ))]);
        let details = harness.inner.get_by_label(PARSE_ERROR).rect();

        harness.inner.press_drag_release(
            details.left_top() + egui::vec2(1.0, 2.0),
            details.size() - egui::vec2(2.0, 2.0),
            4,
        );
        harness.inner.input_mut().events.push(egui::Event::Copy);
        harness.step();

        assert_eq!(harness.inner.copied_text().as_deref(), Some(PARSE_ERROR));
    }

    #[test]
    fn dismissing_a_failure_removes_only_that_failure_of_the_same_source() {
        let first = LoadFailure::new(LoadSource::PastedText, "first error");
        let second = LoadFailure::new(LoadSource::PastedText, "second error");
        let mut harness = harness_showing(vec![first, second.clone()]);

        harness
            .inner
            .query_all_by_role_and_label(Role::Window, "Failed to load pasted log text")
            .find(|window| window.query_by_label("first error").is_some())
            .expect("the window of the first failure")
            .get_by_label(DISMISS_BUTTON_LABEL)
            .click();
        harness.run();

        assert_eq!(harness.state().iter().collect::<Vec<_>>(), [&second]);
        assert_eq!(
            harness
                .inner
                .query_all_by_role_and_label(Role::Window, "Failed to load pasted log text")
                .count(),
            1
        );
    }

    #[rstest]
    #[case::one_failure(1, false)]
    #[case::two_failures(2, true)]
    fn dismiss_all_is_enabled_with_more_than_one_failure(
        #[case] failure_count: usize,
        #[case] enabled: bool,
    ) {
        let harness = harness_showing(vec![failure_of(LoadSource::PastedText); failure_count]);

        let buttons: Vec<bool> = harness
            .inner
            .query_all_by_label(DISMISS_ALL_BUTTON_LABEL)
            .map(|button| !button.accesskit_node().is_disabled())
            .collect();
        assert_eq!(buttons, vec![enabled; failure_count]);
    }

    #[test]
    fn dismissing_all_from_one_window_removes_every_failure() {
        let mut harness = harness_showing(vec![
            failure_of(LoadSource::PastedText),
            failure_of(LoadSource::Filename("navsyncd.log".to_owned())),
        ]);

        harness
            .inner
            .get_by_role_and_label(Role::Window, "Failed to load navsyncd.log")
            .get_by_label(DISMISS_ALL_BUTTON_LABEL)
            .click();
        harness.run();

        assert_eq!(harness.state().iter().count(), 0);
        assert!(
            harness
                .inner
                .query_by_label_contains("Failed to load")
                .is_none()
        );
    }

    #[rstest]
    fn the_window_keeps_its_actions_reachable_with_oversized_text(
        #[values(CRAMPED_VIEWPORT, NARROW_VIEWPORT, SHORT_VIEWPORT)] viewport: egui::Vec2,
    ) {
        let failure = LoadFailure::new(
            LoadSource::Filename(gt_test_utils::oversized_text('f')),
            gt_test_utils::oversized_text('d'),
        );
        let title = failure.title();
        let mut harness = harness_showing_at(vec![failure], viewport);
        harness.inner.run_steps(8);
        let window_id = harness
            .state()
            .retained
            .first()
            .map(RetainedLoadFailure::window_id)
            .expect("the failure is retained");

        let window = AuditedWindow::identified(&title, window_id);
        harness.inner.assert_window_fits_the_viewport(window);
        let copy_label = format!("{ICON_COPY} {COPY_DETAILS_BUTTON_LABEL}");
        for control in [
            DISMISS_BUTTON_LABEL,
            DISMISS_ALL_BUTTON_LABEL,
            copy_label.as_str(),
        ] {
            harness
                .inner
                .assert_control_is_reachable(window, ControlLabel(control));
        }
    }

    #[test]
    fn snapshot_load_failure_window() {
        let mut harness = harness_showing_at(
            vec![failure_of(LoadSource::Filename("navsyncd.log".to_owned()))],
            egui::vec2(480.0, 160.0),
        );
        harness.snapshot_with_color_tolerance("load_failure_window");
    }
}
