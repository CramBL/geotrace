//! What every [`AnchoredDialogKind`] holds to while its content arrives: its
//! footprint, the position of its controls, the weight of the control at the
//! left end of its action row, the press a user aims at it, the viewport it
//! stays inside, and the size the user gave it.
//!
//! A dialog joining [`AnchoredDialogKind`] is held to all six from the frame
//! it is named there: the suite drives the machinery over each kind, with a
//! body region the result lands in. The `disallowed-methods` entry for
//! `egui::Window::anchor` in `clippy.toml` covers the other half, that a dialog
//! is drawn by this machinery at all.

use egui_kittest::Harness;
use egui_kittest::kittest::{By, Queryable as _};
use gt_test_utils::{AuditedWindow, HarnessInteraction as _, WindowFitAssertions as _};
use strum::IntoEnumIterator as _;

use super::{AnchoredDialog, AnchoredDialogKind};
use crate::app::modals::{DialogActions, DialogBody, DialogRowLeadingControl};

/// A control moves only because its own window moved it, never because the
/// screen clipped it: this is wider and taller than every dialog.
const VIEWPORT: egui::Vec2 = egui::vec2(1000.0, 800.0);

const CANCEL_LABEL: &str = "Cancel";

/// The tickbox a dialog puts at the left end of its action row.
const SUPPRESS_LABEL: &str = "Don't show this again";

/// The body's one line before the result arrives, drawn above the room the
/// result lands in.
const PROMPT: &str = "Which recording should this log take its positions from?";

/// The region of the body the result lands in.
const RESULT_REGION: &str = "result";

/// Lines the result's region holds from the frame the dialog opens.
const RESERVED_RESULT_LINES: u8 = 2;

/// Lines the result adds, more than its region holds: they scroll inside it.
const ARRIVING_LINE_COUNT: usize = 40;

/// The width a dialog opens at here is the viewport-relative cap: this is
/// narrower than every dialog.
const VIEWPORT_NARROWER_THAN_EVERY_DIALOG: egui::Vec2 = egui::vec2(400.0, 300.0);

/// Rounding slack when comparing the room on one side of the window against
/// the room on the other, in points.
const CENTRING_SLACK: f32 = 1.0;

const USER_CORNER_DRAG: egui::Vec2 = egui::vec2(60.0, 40.0);

struct DialogUnderTest {
    kind: AnchoredDialogKind,
    shown: bool,
    result_arrived: bool,
    cancelled: bool,
    suppressed: bool,
    background_pressed: bool,

    /// The color the action row handed its leading control on the last pass.
    leading_control_text_color: Option<egui::Color32>,
}

/// Which opening of the dialog a guarantee is read on. The second opening
/// starts from the layout of the first: egui keeps a window's layout for the
/// life of the context.
#[derive(Clone, Copy, Debug)]
enum DialogOpening {
    First,
    Second,
}

/// Passes the dialog stays closed between two openings. The machinery
/// discards the held layout after a gap of more than one pass.
const PASSES_CLOSED: usize = 3;

fn dialog_ui(ui: &mut egui::Ui, state: &mut DialogUnderTest) {
    // The app under the dialog: what a press that misses the dialog reaches.
    if ui
        .allocate_response(ui.available_size(), egui::Sense::click())
        .clicked()
    {
        state.background_pressed = true;
    }
    if !state.shown {
        return;
    }
    let result_arrived = state.result_arrived;
    let dialog = AnchoredDialog::new(state.kind, title_of(state.kind));
    let regions = dialog.regions();
    let cancelled = dialog.show_with_action_row(
        ui.ctx(),
        DialogBody::new(|ui| {
            ui.label(PROMPT);
            regions.frozen_at_open_holding_lines(ui, RESULT_REGION, RESERVED_RESULT_LINES, |ui| {
                if result_arrived {
                    for line in 0..ARRIVING_LINE_COUNT {
                        ui.label(format!("the result, line {line}"));
                    }
                }
            });
        }),
        DialogRowLeadingControl::new(|ui| {
            state.leading_control_text_color = Some(ui.visuals().text_color());
            ui.checkbox(&mut state.suppressed, SUPPRESS_LABEL);
        }),
        DialogActions::new(|ui| ui.button(CANCEL_LABEL).clicked()),
    );
    if cancelled == Some(true) {
        state.cancelled = true;
    }
}

/// Each kind gets a title of its own, which is what egui derives the window's
/// area id from.
fn title_of(kind: AnchoredDialogKind) -> String {
    format!("{kind:?}")
}

fn dialog_over(
    kind: AnchoredDialogKind,
    viewport: egui::Vec2,
    opening: DialogOpening,
) -> Harness<'static, DialogUnderTest> {
    let mut harness = Harness::builder().with_size(viewport).build_ui_state(
        dialog_ui,
        DialogUnderTest {
            kind,
            shown: true,
            result_arrived: false,
            cancelled: false,
            suppressed: false,
            background_pressed: false,
            leading_control_text_color: None,
        },
    );
    harness.run_steps(4);
    if matches!(opening, DialogOpening::Second) {
        harness.state_mut().shown = false;
        harness.run_steps(PASSES_CLOSED);
        harness.state_mut().shown = true;
        harness.run_steps(4);
    }
    harness
}

/// Every kind, on each of its first two openings.
fn every_dialog_opening() -> impl Iterator<Item = (AnchoredDialogKind, DialogOpening)> {
    AnchoredDialogKind::iter()
        .flat_map(|kind| [DialogOpening::First, DialogOpening::Second].map(|open| (kind, open)))
}

fn deliver_the_result(harness: &mut Harness<'_, DialogUnderTest>) {
    harness.state_mut().result_arrived = true;
    harness.run_steps(4);
}

fn window_rect(harness: &Harness<'_, DialogUnderTest>, kind: AnchoredDialogKind) -> egui::Rect {
    let title = title_of(kind);
    harness
        .window_rect(&title)
        .unwrap_or_else(|| panic!("the {title} dialog is shown"))
}

#[test]
fn the_footprint_is_the_same_once_the_result_arrives() {
    for (kind, opening) in every_dialog_opening() {
        let mut harness = dialog_over(kind, VIEWPORT, opening);
        let before = window_rect(&harness, kind);

        deliver_the_result(&mut harness);

        assert_eq!(
            window_rect(&harness, kind),
            before,
            "the {kind:?} dialog on its {opening:?} opening"
        );
    }
}

#[test]
fn no_control_moves_when_the_result_arrives() {
    for (kind, opening) in every_dialog_opening() {
        let mut harness = dialog_over(kind, VIEWPORT, opening);
        let prompt = harness.get(By::new().label(PROMPT)).rect();
        let suppress = harness.get(By::new().label(SUPPRESS_LABEL)).rect();
        let cancel = harness.get(By::new().label(CANCEL_LABEL)).rect();

        deliver_the_result(&mut harness);

        assert_eq!(
            harness.get(By::new().label(PROMPT)).rect(),
            prompt,
            "the prompt of the {kind:?} dialog on its {opening:?} opening"
        );
        assert_eq!(
            harness.get(By::new().label(SUPPRESS_LABEL)).rect(),
            suppress,
            "the leading tickbox of the {kind:?} dialog on its {opening:?} opening"
        );
        assert_eq!(
            harness.get(By::new().label(CANCEL_LABEL)).rect(),
            cancel,
            "the cancel button of the {kind:?} dialog on its {opening:?} opening"
        );
    }
}

/// The leading control sets a preference beyond the action the row confirms,
/// and the weak color keeps it from reading as one more of the body's options.
#[test]
fn the_action_row_draws_its_leading_control_in_the_weak_text_color() {
    for (kind, opening) in every_dialog_opening() {
        let harness = dialog_over(kind, VIEWPORT, opening);

        assert_eq!(
            harness.state().leading_control_text_color,
            Some(harness.ctx.global_style().visuals.weak_text_color()),
            "the leading tickbox of the {kind:?} dialog on its {opening:?} opening"
        );
    }
}

/// The user's pointer rests on the cancel button while the result arrives, and
/// the press that follows has to reach that button and nothing behind it.
#[test]
fn a_press_where_the_pointer_rests_reaches_the_control_it_was_aimed_at() {
    for (kind, opening) in every_dialog_opening() {
        let mut harness = dialog_over(kind, VIEWPORT, opening);
        let aimed_at = harness.get(By::new().label(CANCEL_LABEL)).rect().center();
        harness.hover_at(aimed_at);
        harness.run_steps(2);

        deliver_the_result(&mut harness);
        harness.press_where_the_pointer_rests(aimed_at);

        assert!(
            harness.state().cancelled,
            "the {kind:?} dialog on its {opening:?} opening"
        );
        assert!(
            !harness.state().background_pressed,
            "the press aimed at the {kind:?} dialog on its {opening:?} opening reached the app \
             behind it"
        );
    }
}

/// A dialog wider than the screen opens at the viewport-relative cap instead:
/// centred, whole, clear of the screen edge, with its content scrolling
/// inside.
#[test]
fn the_window_stays_centred_inside_a_viewport_narrower_than_the_dialog() {
    for (kind, opening) in every_dialog_opening() {
        let mut harness = dialog_over(kind, VIEWPORT_NARROWER_THAN_EVERY_DIALOG, opening);
        deliver_the_result(&mut harness);

        let title = title_of(kind);
        harness.assert_window_fits_the_viewport(AuditedWindow::titled(&title));
        let viewport = harness.ctx.content_rect();
        let rect = window_rect(&harness, kind);
        let room_left = rect.left() - viewport.left();
        let room_right = viewport.right() - rect.right();
        assert!(
            (room_left - room_right).abs() <= CENTRING_SLACK,
            "the {kind:?} dialog on its {opening:?} opening is at {rect:?}, leaving \
             {room_left:.0} points to its left and {room_right:.0} to its right in the \
             {viewport:?} viewport"
        );
        assert!(
            room_left > 0.0 && rect.top() > viewport.top(),
            "the {kind:?} dialog on its {opening:?} opening is at {rect:?}, running to the edge \
             of the {viewport:?} viewport"
        );
    }
}

/// A user dragging the window's bottom-right corner is the one thing that may
/// resize a dialog. The window keeps that size when the result arrives.
#[test]
fn a_user_resize_is_honoured_and_kept_while_the_result_arrives() {
    for (kind, opening) in every_dialog_opening() {
        let mut harness = dialog_over(kind, VIEWPORT, opening);
        let opened_at = window_rect(&harness, kind);

        harness.press_drag_release(opened_at.max - egui::vec2(2.0, 2.0), USER_CORNER_DRAG, 4);
        harness.run_steps(4);
        let resized = window_rect(&harness, kind);
        assert!(
            resized.size().x > opened_at.size().x && resized.size().y > opened_at.size().y,
            "dragging the corner of the {kind:?} dialog on its {opening:?} opening left it at \
             {:?}, from {:?}",
            resized.size(),
            opened_at.size(),
        );

        deliver_the_result(&mut harness);

        assert_eq!(
            window_rect(&harness, kind),
            resized,
            "the {kind:?} dialog on its {opening:?} opening"
        );
    }
}
