//! Where a dialog's controls are while its content changes size, and what a
//! press aimed at one of them reaches.
//!
//! These tests pin the case where the pointer rests on a control for as long
//! as the user takes to decide and the dialog's content arrives in the
//! meantime. [`crate::app::anchored_dialog`] states what egui does with such a
//! press.

use std::cell::RefCell;

use chrono::Duration;
use egui_kittest::Harness;
use gt_pending_writes::WriteKind;
use gt_test_utils::window_fit::NARROW_VIEWPORT;
use gt_test_utils::{By, HarnessInteraction as _, Queryable as _};

use crate::app::log_viewer::association_dialog::{
    self, CONFIRM_LABEL, LogAssociationChoice, TITLE as ASSOCIATION_TITLE, tests::DialogState,
};

use super::{
    DELETE_ARCHIVED_DAYS_TITLE, EnvironmentPruneChoice, ForceQuitChoice,
    LOADED_RECORDINGS_MOST_LINES, PERMANENT_DELETE_LABEL, PruneScope, SnapScopeChoice, tests,
};

const CANCEL_LABEL: &str = "Cancel";

/// Items the tickbox measurement shelves, all of them tracks of one
/// stored recording.
const SHELVED_ITEMS: usize = 2;

/// The ending the two wordings of the shelve confirmation's history
/// sentence share.
const DETAIL_SENTENCE_ENDING: &str = "takes them out of the view.";

/// Fixes of each recording the association dialog lists.
const FIX_COUNT: usize = 10;

/// The attachment the history database reports for the chosen recording,
/// with a name long enough that the note about it wraps onto three lines.
const STORED_ATTACHMENT_NAME: &str = "navsyncd-export-2026-05-29-evening-run.log";

/// The recording name added to the prune confirmation once the pending
/// load finishes.
const LOADED_RECORDING: &str = "Evening ferry crossing";

/// Loaded recordings enough to fill the room the prune confirmation caps
/// at [`LOADED_RECORDINGS_MOST_LINES`].
const RECORDINGS_PAST_THE_CAPPED_ROOM: usize = 12;

/// Loaded recordings far past that cap, to read the capped room's height
/// against.
const RECORDINGS_FAR_PAST_THE_CAPPED_ROOM: usize = 40;

/// Writes still running once two of the four have finished.
const WRITES_STILL_RUNNING: usize = 2;

/// The costs the force-quit confirmation lists while four writes run.
fn four_write_costs() -> Vec<String> {
    vec![
        WriteKind::Settings.interruption_cost(),
        WriteKind::RecordingDatabase.interruption_cost(),
        WriteKind::DatabaseOpen.interruption_cost(),
        WriteKind::TakeOverRecord.interruption_cost(),
    ]
}

fn loaded_recordings(count: usize) -> RefCell<Vec<String>> {
    RefCell::new(
        (0..count)
            .map(|index| format!("Recording {index}"))
            .collect(),
    )
}

/// A list longer than the capped room scrolls inside it, and the buttons
/// stay where they are when one more name arrives.
#[test]
fn the_prune_confirmation_keeps_its_buttons_in_place_while_a_name_arrives_past_the_capped_room() {
    let listed = loaded_recordings(RECORDINGS_PAST_THE_CAPPED_ROOM);
    let choice = RefCell::new(None);
    let mut harness = tests::prune_dialog(PruneScope::Every, &listed, &choice);
    let before = harness.inner.get(By::new().label(CANCEL_LABEL)).rect();

    listed.borrow_mut().push(LOADED_RECORDING.to_owned());
    harness.inner.run_steps(4);

    assert_eq!(
        harness.inner.get(By::new().label(CANCEL_LABEL)).rect(),
        before,
        "the Cancel button of the prune confirmation moved: a press where the user aimed \
         misses it"
    );
}

/// The capped room holds the prune confirmation to one height, however
/// many recordings are loaded when it opens.
#[test]
fn the_prune_confirmation_opens_at_one_height_for_every_list_past_the_capped_room() {
    let past = loaded_recordings(RECORDINGS_PAST_THE_CAPPED_ROOM);
    let past_choice = RefCell::new(None);
    let past_harness = tests::prune_dialog(PruneScope::Every, &past, &past_choice);
    let far_past = loaded_recordings(RECORDINGS_FAR_PAST_THE_CAPPED_ROOM);
    let far_past_choice = RefCell::new(None);
    let far_past_harness = tests::prune_dialog(PruneScope::Every, &far_past, &far_past_choice);

    assert_eq!(
        far_past_harness
            .inner
            .window_rect(DELETE_ARCHIVED_DAYS_TITLE)
            .expect("the prune confirmation is shown")
            .size(),
        past_harness
            .inner
            .window_rect(DELETE_ARCHIVED_DAYS_TITLE)
            .expect("the prune confirmation is shown")
            .size(),
        "{RECORDINGS_FAR_PAST_THE_CAPPED_ROOM} loaded recordings made the prune confirmation \
         taller than {RECORDINGS_PAST_THE_CAPPED_ROOM} did: a list past the room it caps at \
         {LOADED_RECORDINGS_MOST_LINES} lines has to scroll inside that room"
    );
}

/// The user aims at Cancel and a recording finishes loading before the
/// press. Cancel deletes nothing.
#[test]
fn cancelling_the_prune_confirmation_reports_the_cancel_while_a_recording_name_arrives() {
    let loaded_recordings = RefCell::new(Vec::new());
    let choice = RefCell::new(None);
    let mut harness = tests::prune_dialog(PruneScope::Every, &loaded_recordings, &choice);
    let aimed_at = harness
        .inner
        .get(By::new().label(CANCEL_LABEL))
        .rect()
        .center();
    harness.inner.hover_at(aimed_at);
    harness.inner.run_steps(2);

    loaded_recordings
        .borrow_mut()
        .push(LOADED_RECORDING.to_owned());
    harness.inner.run_steps(2);
    harness.inner.press_where_the_pointer_rests(aimed_at);

    assert!(
        matches!(*choice.borrow(), Some(EnvironmentPruneChoice::Cancel)),
        "the press on Cancel deleted the archived days instead of reporting the cancel"
    );
}

/// The force-quit confirmation lists one line per write still running, and the
/// user aims at Cancel while two of the four finish. The press reports the
/// cancel and reaches nothing behind the confirmation. The shutdown window
/// behind it holds "Force quit…" beside "Run in background".
#[test]
fn a_press_aimed_at_the_force_quit_confirmation_cancels_it_and_reaches_nothing_behind_it() {
    let costs = RefCell::new(four_write_costs());
    let choice = RefCell::new(None);
    let background_pressed = RefCell::new(false);
    let mut harness = tests::force_quit_dialog_over(&costs, &choice, |ui| {
        if ui
            .allocate_response(ui.available_size(), egui::Sense::click())
            .clicked()
        {
            *background_pressed.borrow_mut() = true;
        }
    });
    let aimed_at = harness
        .inner
        .get(By::new().label(CANCEL_LABEL))
        .rect()
        .center();
    harness.inner.hover_at(aimed_at);
    harness.inner.run_steps(2);

    costs.borrow_mut().truncate(WRITES_STILL_RUNNING);
    harness.inner.run_steps(2);
    harness.inner.press_where_the_pointer_rests(aimed_at);

    assert!(
        matches!(*choice.borrow(), Some(ForceQuitChoice::Dismiss)),
        "the press on Cancel did not report the cancel"
    );
    assert!(
        !*background_pressed.borrow(),
        "the press aimed at Cancel reached the window under the confirmation"
    );
}

/// The snap scope dialog counts the tracks that already have snap data,
/// and states that snapping again replaces it as soon as one does. Cancel
/// uploads nothing.
#[test]
fn cancelling_the_snap_scope_dialog_reports_the_cancel_while_a_snap_result_arrives() {
    let counts = RefCell::new(tests::nothing_snapped_yet());
    let choice = RefCell::new(None);
    let mut harness = tests::snap_scope_dialog(&counts, &choice);
    let aimed_at = harness
        .inner
        .get(By::new().label(CANCEL_LABEL))
        .rect()
        .center();
    harness.inner.hover_at(aimed_at);
    harness.inner.run_steps(2);

    counts.borrow_mut().all.already_snapped = 1;
    harness.inner.run_steps(2);
    harness.inner.press_where_the_pointer_rests(aimed_at);

    assert!(
        matches!(*choice.borrow(), Some(SnapScopeChoice::Cancel)),
        "the press on Cancel uploaded the tracks instead of reporting the cancel"
    );
}

/// The shelve confirmation states in one sentence what it does in history,
/// and the permanent-delete tickbox chooses the wording. The dialog is 324
/// points wide at [`NARROW_VIEWPORT`]. "Shelves 2 tracks in 1 recording in
/// history and takes them out of the view." takes one line at that width,
/// and "Permanently deletes 2 tracks from 1 recording in history and takes
/// them out of the view." takes two.
///
/// The second line goes into the room the body already had. The window
/// keeps the height and the position it opened at, which it holds under
/// its [`AnchoredDialogKind`].
#[test]
fn the_shelve_confirmation_keeps_its_window_and_tickbox_in_place_while_the_delete_is_ticked() {
    let mut harness = tests::shelve_confirmation_at(
        NARROW_VIEWPORT,
        SHELVED_ITEMS,
        tests::PermanentDeleteTicked(false),
    );
    let window = tests::shelve_confirmation_rect(&harness, SHELVED_ITEMS);
    let tickbox = harness
        .inner
        .get(By::new().label_contains(PERMANENT_DELETE_LABEL))
        .rect();
    let one_line = harness
        .inner
        .get(By::new().label_contains(DETAIL_SENTENCE_ENDING))
        .rect()
        .height();

    harness.inner.click_at(tickbox.center());
    harness.inner.run_steps(4);

    let two_lines = harness
        .inner
        .get(By::new().label_contains(DETAIL_SENTENCE_ENDING))
        .rect()
        .height();
    assert!(
        two_lines > one_line,
        "the sentence took {two_lines} points ticked and {one_line} points unticked: this \
         measurement needs a width at which the two wordings wrap onto a different number of \
         lines"
    );
    assert_eq!(
        tests::shelve_confirmation_rect(&harness, SHELVED_ITEMS),
        window,
        "the shelve confirmation moved or resized around the longer sentence under its \
         tickbox: its edge moves past a control the user aimed at, and the press reaches \
         the app behind it"
    );
    assert_eq!(
        harness
            .inner
            .get(By::new().label_contains(PERMANENT_DELETE_LABEL))
            .rect(),
        tickbox,
        "the permanent-delete tickbox moved under the pointer that just ticked it: the press \
         that unticks it misses"
    );
}

/// The association dialog over two stored recordings: the second only
/// overlaps part of the log and is listed below the first.
fn association_dialog() -> Harness<'static, DialogState> {
    association_dialog::tests::harness_over_sized(
        vec![
            (
                association_dialog::tests::recording("alongside.gtd", Duration::zero(), FIX_COUNT),
                association_dialog::tests::stored_in_history("nav-devkit-mk2"),
            ),
            (
                association_dialog::tests::recording("late.gtd", Duration::seconds(5), FIX_COUNT),
                association_dialog::tests::stored_in_history("nav-devkit-mk4"),
            ),
        ],
        tests::DIALOG_VIEWPORT,
    )
}

/// Selecting a recording sends the duplicate-attachment query. The dialog
/// draws a line above the checkbox that stores the log when the result
/// arrives. A press aimed at a recording row must not tick that box:
/// attaching writes the log into the history database.
#[test]
fn pressing_a_recording_row_while_the_stored_attachment_line_arrives_attaches_nothing() {
    let mut harness = association_dialog();
    harness.get(By::new().label("late.gtd")).click();
    harness.run_steps(3);
    let aimed_at = harness.get(By::new().label("late.gtd")).rect().center();
    harness.hover_at(aimed_at);
    harness.run_steps(2);

    association_dialog::tests::deliver_the_stored_attachment(&mut harness, STORED_ATTACHMENT_NAME);
    harness.run_steps(2);
    harness.press_where_the_pointer_rests(aimed_at);
    harness.get(By::new().label(CONFIRM_LABEL)).click();
    harness.run_steps(2);

    assert!(
        matches!(
            harness.state().choice,
            Some(LogAssociationChoice::Confirmed { attach: false, .. })
        ),
        "the press on a recording row reported {:?}",
        harness.state().choice,
    );
}

#[test]
fn the_association_dialog_keeps_its_recording_rows_in_place_while_the_result_arrives() {
    let mut harness = association_dialog();
    harness.get(By::new().label("late.gtd")).click();
    harness.run_steps(3);
    let before = harness.get(By::new().label("late.gtd")).rect();

    association_dialog::tests::deliver_the_stored_attachment(&mut harness, STORED_ATTACHMENT_NAME);
    harness.run_steps(4);
    let after = harness.get(By::new().label("late.gtd")).rect();

    assert_eq!(
        after, before,
        "the row of the chosen recording moved in the {ASSOCIATION_TITLE} dialog: a press \
         where the user aimed misses it"
    );
}
