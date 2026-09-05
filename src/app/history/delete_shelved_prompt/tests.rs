use std::time::{Duration, Instant};

use egui_kittest::kittest::{NodeT as _, Queryable as _};
use gt_store::{DatabaseRef, RecordingEntry};
use gt_test_utils::{By, HarnessInteraction as _, TestHarness};
use rstest::rstest;

use crate::app::history::test_support::{ShelvedTracks, TotalTracks, entry_with_shelved_tracks};
use crate::app::history_db::DeleteShelvedTracksScope;
use crate::app::modals::{
    COUNT_A_REPORTING_DIALOG_RUNS_BEFORE_IT_CLOSES, PointerOverTheDialog, TimeUntilTheClose,
};

use super::{
    DELETE_SHELVED_TRACKS_LABEL, DeleteShelvedTracksChoice, DeleteShelvedTracksPrompt,
    DeleteShelvedTracksPromptContents, ShelvedTracksToDelete,
};

const CANCEL_LABEL: &str = "Cancel";

/// Nothing the test measures is clipped by the screen. This is wider and
/// taller than the confirmation.
const VIEWPORT: egui::Vec2 = egui::vec2(640.0, 480.0);

/// Two recordings, one of them with a live track left: three shelved tracks in
/// all, and one recording that a sweep would delete whole.
fn listing() -> Vec<RecordingEntry> {
    vec![
        entry_with_shelved_tracks("ride.gtd", TotalTracks(4), ShelvedTracks(1)),
        entry_with_shelved_tracks("walk.gtd", TotalTracks(2), ShelvedTracks(2)),
    ]
}

/// The shelved tracks of the listing's first recording.
fn first_recording() -> DeleteShelvedTracksScope {
    DeleteShelvedTracksScope::OneRecording(DatabaseRef {
        identity: "ride.gtd".to_owned(),
        group_name: "rec0".to_owned(),
    })
}

fn confirming_prompt() -> DeleteShelvedTracksPrompt {
    let mut prompt = DeleteShelvedTracksPrompt::default();
    prompt.open(DeleteShelvedTracksScope::EveryRecording, &listing());
    prompt
}

fn prompt_reporting_every_track_live_at(now: Instant) -> DeleteShelvedTracksPrompt {
    let mut prompt = confirming_prompt();
    prompt.contents_to_show(now, Some(&[]));
    prompt
}

fn shelved_tracks(
    tracks: usize,
    recordings_deleted_whole: &[&str],
) -> Option<DeleteShelvedTracksPromptContents> {
    Some(DeleteShelvedTracksPromptContents::ShelvedTracks {
        scope: DeleteShelvedTracksScope::EveryRecording,
        shelved: ShelvedTracksToDelete {
            tracks,
            recordings_deleted_whole: recordings_deleted_whole
                .iter()
                .map(|name| (*name).to_owned())
                .collect(),
        },
    })
}

fn every_track_is_live(
    time_until_the_close: Duration,
) -> Option<DeleteShelvedTracksPromptContents> {
    Some(DeleteShelvedTracksPromptContents::EveryTrackIsLive(
        TimeUntilTheClose(time_until_the_close),
    ))
}

#[test]
fn a_closed_confirmation_shows_nothing() {
    let mut prompt = DeleteShelvedTracksPrompt::default();

    assert_eq!(
        prompt.contents_to_show(Instant::now(), Some(&listing())),
        None
    );
}

#[test]
fn the_confirmation_counts_the_shelved_tracks_the_recording_list_last_reported() {
    let mut prompt = confirming_prompt();

    assert_eq!(
        prompt.contents_to_show(Instant::now(), Some(&listing())),
        shelved_tracks(3, &["walk.gtd/rec0"])
    );
}

/// A delete raised from one recording's shelf leaves the shelved tracks of
/// every other recording alone, and the confirmation states only what it takes.
#[test]
fn a_confirmation_over_one_recording_counts_only_that_recordings_shelved_tracks() {
    let mut prompt = DeleteShelvedTracksPrompt::default();
    prompt.open(first_recording(), &listing());

    assert_eq!(
        prompt.contents_to_show(Instant::now(), Some(&listing())),
        Some(DeleteShelvedTracksPromptContents::ShelvedTracks {
            scope: first_recording(),
            shelved: ShelvedTracksToDelete {
                tracks: 1,
                recordings_deleted_whole: Vec::new(),
            },
        })
    );
}

/// A recording that holds only shelved tracks is one the delete removes
/// entirely. The listing reports such a recording as an equal total and shelved
/// count.
#[rstest]
#[case::a_live_track_stays(TotalTracks(4), ShelvedTracks(1), &[])]
#[case::every_track_is_shelved(TotalTracks(2), ShelvedTracks(2), &["ride.gtd/rec0"])]
fn the_confirmation_lists_the_recordings_the_delete_removes_entirely(
    #[case] total_tracks: TotalTracks,
    #[case] ShelvedTracks(shelved_tracks): ShelvedTracks,
    #[case] expected: &[&str],
) {
    let listing = [entry_with_shelved_tracks(
        "ride.gtd",
        total_tracks,
        ShelvedTracks(shelved_tracks),
    )];

    let shelved = ShelvedTracksToDelete::of(&DeleteShelvedTracksScope::EveryRecording, &listing);

    assert_eq!(shelved.tracks, shelved_tracks);
    assert_eq!(shelved.recordings_deleted_whole, expected);
}

/// The count is `None` between a mutation and the list that follows it: a
/// recording finishing its load, an auto-prune, or a track unshelved elsewhere
/// all send the window back to the database for a fresh list.
#[test]
fn the_confirmation_keeps_its_count_while_a_recording_list_request_is_in_flight() {
    let mut prompt = confirming_prompt();

    assert_eq!(
        prompt.contents_to_show(Instant::now(), None),
        shelved_tracks(3, &["walk.gtd/rec0"])
    );
}

#[test]
fn every_track_being_live_again_leaves_the_confirmation_up_reporting_it() {
    let mut prompt = confirming_prompt();

    assert_eq!(
        prompt.contents_to_show(Instant::now(), Some(&[])),
        every_track_is_live(COUNT_A_REPORTING_DIALOG_RUNS_BEFORE_IT_CLOSES)
    );
    assert!(prompt.is_up());
}

#[test]
fn a_confirmation_reporting_that_every_track_is_live_never_counts_tracks_again() {
    let now = Instant::now();
    let mut prompt = prompt_reporting_every_track_live_at(now);

    assert_eq!(
        prompt.contents_to_show(now, Some(&listing())),
        every_track_is_live(COUNT_A_REPORTING_DIALOG_RUNS_BEFORE_IT_CLOSES)
    );
}

#[rstest]
#[case::the_count_ran_out_and_the_pointer_is_away(
    COUNT_A_REPORTING_DIALOG_RUNS_BEFORE_IT_CLOSES,
    PointerOverTheDialog::Away,
    false
)]
#[case::the_pointer_rests_over_the_confirmation(
    COUNT_A_REPORTING_DIALOG_RUNS_BEFORE_IT_CLOSES * 10,
    PointerOverTheDialog::Resting,
    true
)]
#[case::the_count_has_not_run_out_yet(
    COUNT_A_REPORTING_DIALOG_RUNS_BEFORE_IT_CLOSES / 2,
    PointerOverTheDialog::Away,
    true
)]
fn the_confirmation_reporting_that_every_track_is_live_closes_itself(
    #[case] since_the_last_shelved_track_went: Duration,
    #[case] pointer: PointerOverTheDialog,
    #[case] expected_up: bool,
) {
    let reported_at = Instant::now();
    let mut prompt = prompt_reporting_every_track_live_at(reported_at);

    prompt.advance_the_countdown_and_close_when_it_runs_out(
        reported_at + since_the_last_shelved_track_went,
        pointer,
    );

    assert_eq!(prompt.is_up(), expected_up);
}

#[rstest]
#[case::delete(
    DeleteShelvedTracksChoice::Delete,
    Some(DeleteShelvedTracksScope::EveryRecording)
)]
#[case::dismiss(DeleteShelvedTracksChoice::Dismiss, None)]
fn the_users_choice_closes_the_confirmation(
    #[case] choice: DeleteShelvedTracksChoice,
    #[case] expected: Option<DeleteShelvedTracksScope>,
) {
    let mut prompt = confirming_prompt();

    assert_eq!(prompt.record_choice(choice), expected);
    assert!(!prompt.is_up());
}

struct PromptUnderTest {
    prompt: DeleteShelvedTracksPrompt,
    now: Instant,
    listing: Option<Vec<RecordingEntry>>,
    background_pressed: bool,
    delete_requested: bool,
}

fn prompt_ui(ui: &mut egui::Ui, state: &mut PromptUnderTest) {
    // The History window under the confirmation: what a press that misses the
    // confirmation reaches.
    if ui
        .allocate_response(ui.available_size(), egui::Sense::click())
        .clicked()
    {
        state.background_pressed = true;
    }
    if state
        .prompt
        .show(ui.ctx(), state.now, state.listing.as_deref())
        .is_some()
    {
        state.delete_requested = true;
    }
}

/// The confirmation over the window it is drawn on, with the pointer resting
/// on Cancel and the last shelved track gone from the recording list.
fn confirmation_once_every_track_is_live() -> (TestHarness<'static, PromptUnderTest>, egui::Pos2) {
    let mut harness = TestHarness::builder().size(VIEWPORT).ui_state(
        prompt_ui,
        PromptUnderTest {
            prompt: confirming_prompt(),
            // Held still: the count is what the confirmation opened at
            // however long the frames take.
            now: Instant::now(),
            listing: Some(listing()),
            background_pressed: false,
            delete_requested: false,
        },
    );
    harness.run();
    let aimed_at = harness
        .inner
        .get(By::new().label(CANCEL_LABEL))
        .rect()
        .center();
    harness.inner.hover_at(aimed_at);
    harness.inner.run_steps(2);

    harness.state_mut().listing = Some(Vec::new());
    harness.inner.run_steps(2);
    (harness, aimed_at)
}

#[test]
fn a_press_aimed_at_the_confirmation_does_not_reach_the_window_behind_it() {
    let (mut harness, aimed_at) = confirmation_once_every_track_is_live();

    harness.inner.press_where_the_pointer_rests(aimed_at);

    assert!(
        !harness.state().background_pressed,
        "the press aimed at Cancel reached the window under the confirmation"
    );
    assert!(!harness.state().delete_requested);
}

#[test]
fn the_confirmation_reporting_that_every_track_is_live_grays_its_delete_out() {
    let (harness, _aimed_at) = confirmation_once_every_track_is_live();

    assert!(harness.state().prompt.is_up());
    assert!(
        harness
            .inner
            .get_by_label(DELETE_SHELVED_TRACKS_LABEL)
            .accesskit_node()
            .is_disabled()
    );
}
