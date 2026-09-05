use std::time::Instant;

use egui_kittest::kittest::{NodeT as _, Queryable as _};
use gt_pending_writes::{PendingWrites, WriteAccess};
use gt_store::{HistoryDatabase as _, RecordingsHandle};
use gt_test_utils::window_fit::{
    CRAMPED_VIEWPORT, NARROW_VIEWPORT, OVERSIZED_ROW_COUNT, SHORT_VIEWPORT,
};
use gt_test_utils::{
    AuditedWindow, By, ControlLabel, HarnessInteraction as _, TestHarness, WindowFitAssertions as _,
};

use crate::app::history_db::{DeleteShelvedTracksScope, Response};
use crate::app::read_only_session::READ_ONLY_RECORDING_HISTORY_HOVER;
use crate::app::storage_controls::AUTO_STORE_LABEL;

use egui_phosphor::regular::CARET_RIGHT as ICON_CARET_RIGHT;
use egui_phosphor::regular::NOTE as ICON_NOTE;
use egui_phosphor::regular::PAPERCLIP as ICON_PAPERCLIP;
use egui_phosphor::regular::TRASH as ICON_TRASH;
use gt_store::{
    ChannelSummary, StoredFixPlacementRule, StoredSegmentation, StoredTrackSplitRule, TrackRange,
    TrackState,
};
use gt_ui_theme::EM_DASH;

use super::delete_shelved_prompt::{DELETE_SHELVED_TRACKS_LABEL, DELETE_SHELVED_WINDOW_TITLE};
use super::table::{
    MAX_HOVER_CHANNELS, OPEN_LOG_LABEL, UNSHELVE_ALL_LABEL, UNSHELVE_LABEL, breakdown_cell_id,
    channel_title, data_breakdown_ui, duration_text, started_at_text, time_range_text,
    track_count_text,
};
use super::test_support::{
    ShelvedTracks, TotalTracks, entry_with_identity, entry_with_shelved_tracks,
};
use super::{
    DEFAULT_WINDOW_HEIGHT_PX, DEFAULT_WINDOW_WIDTH_PX, DatabaseRef, HistorySort, HistoryWindow,
    HistoryWorker, ICON_CARET_DOWN, ICON_CARET_UP, NavPointTimeRange, PRUNE_WINDOW_TITLE,
    RecordingEntry, SortColumn, SortDirection, identity_display_parts, travel_mode_display,
};
use strum::{EnumCount as _, IntoEnumIterator as _};

/// Harness state for driving the History window: the window, a live (empty)
/// worker so the list branch renders, and the settings toggles `show` needs.
struct HistoryHarness {
    window: HistoryWindow,
    /// The frame's instant, held still so a confirmation counting down to its
    /// own close keeps the count it opened at.
    now: Instant,
    worker: HistoryWorker,
    storage: crate::settings::StorageSettings,
    /// What the app reports while its startup open runs.
    databases_opening: bool,
    /// What the session may write, which is what grays the controls that do.
    write_access: WriteAccess,
    /// The log the worker last read back for the Logs menu's "Open log".
    opened_log: Option<gt_store::AttachedLog>,
    _dir: tempfile::TempDir,
}

fn history_harness(entries: Vec<RecordingEntry>) -> HistoryHarness {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = gt_store::Recordings::open_or_create(&dir.path().join("history.h5")).expect("open db");
    let worker = HistoryWorker::spawn(
        RecordingsHandle::Owner(db),
        egui::Context::default(),
        PendingWrites::default(),
    );
    let mut window = HistoryWindow::new();
    window.open = true;
    // Populate directly so the list renders without a worker round-trip.
    window.set_entries(entries);
    HistoryHarness {
        window,
        now: Instant::now(),
        worker,
        storage: crate::settings::StorageSettings {
            auto_prune_max_bytes: 0,
            ..crate::settings::StorageSettings::default()
        },
        databases_opening: false,
        write_access: WriteAccess::Owner,
        opened_log: None,
        _dir: dir,
    }
}

fn show_history(ui: &mut egui::Ui, s: &mut HistoryHarness) {
    s.window.show(
        ui.ctx(),
        super::HistoryWindowFrame {
            now: s.now,
            worker: &s.worker,
            loaded_metas: &[],
            storage: &mut s.storage,
            databases_opening: s.databases_opening,
            write_access: s.write_access,
        },
    );
}

/// A harness backed by a real database holding one recording with `stored_logs`
/// attached to it, and no pre-seeded entries - the list arrives from the worker
/// (see [`pump_history`]).
fn history_harness_with_recording(identity: &str, stored_logs: &[&str]) -> HistoryHarness {
    use gt_store::{LogAttachments as _, LogToAttach};

    let dir = tempfile::tempdir().expect("temp dir");
    let mut db =
        gt_store::Recordings::open_or_create(&dir.path().join("history.h5")).expect("open db");
    let bytes = gt_test_utils::GOLD_BYTES;
    let meta = gt_store::extract_meta(bytes).expect("meta");
    let tracks = [TrackRange {
        start: 0,
        end: meta.nav_point_count,
        state: TrackState::Live,
    }];
    let db_ref = db
        .insert(identity, &meta, &tracks, stored_segmentation(), bytes)
        .expect("insert recording");
    for name in stored_logs {
        db.attach_log(
            &db_ref,
            &LogToAttach {
                name,
                text: &stored_log_text(name),
                filters: Vec::new(),
            },
        )
        .expect("attach a log to the recording");
    }
    let worker = HistoryWorker::spawn(
        RecordingsHandle::Owner(db),
        egui::Context::default(),
        PendingWrites::default(),
    );
    let mut window = HistoryWindow::new();
    window.open = true;
    HistoryHarness {
        window,
        now: Instant::now(),
        worker,
        storage: crate::settings::StorageSettings {
            auto_prune_max_bytes: 0,
            ..crate::settings::StorageSettings::default()
        },
        databases_opening: false,
        write_access: WriteAccess::Owner,
        opened_log: None,
        _dir: dir,
    }
}

/// A log read back is recognizable as the one that was opened: the one line it
/// is stored with holds its name.
fn stored_log_text(name: &str) -> String {
    format!("2026-05-29 18:48:25 {name}: starting\n")
}

/// Drive one frame like the app does: drain the worker's responses into the
/// window (list refresh, mutation acknowledgements, a log read back) and then
/// render it.
fn pump_history(ui: &mut egui::Ui, s: &mut HistoryHarness) {
    for resp in s.worker.poll() {
        match resp {
            Response::Listed(Ok(entries)) => s.window.set_entries(entries),
            Response::Mutated { result: Ok(()), .. } => s.window.invalidate(),
            Response::AttachedLogLoaded { log: Ok(log), .. } => s.opened_log = Some(log),
            Response::StoredTrackTableLoaded { db_ref, tracks } => {
                s.window.set_shelf_track_table(&db_ref, tracks);
            }
            _ => {}
        }
    }
    show_history(ui, s);
}

#[test]
fn rename_workflow_updates_the_listed_identity_end_to_end() {
    // Full workflow against a real worker + database: the row lists, the user
    // edits the identity inline, and after the worker's rename the list shows
    // the new name.
    let harness = history_harness_with_recording("auto:ride.gtd", &[]);
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(pump_history, harness);

    // The recording lists under its stripped identity.
    assert!(
        h.inner
            .step_until(|h| h.query_by_label_contains("ride.gtd").is_some()),
        "recording should appear in the History list"
    );

    // Open the inline editor through the identity's context menu.
    // `request_focus` applies the frame after the editor first renders,
    // so settle a couple of frames before typing.
    h.inner.get_by_label_contains("ride.gtd").click_secondary();
    h.step();
    h.inner.get_by_label("Rename").click_accesskit();
    h.step();
    h.step();
    assert!(
        h.inner.query_all_by_value("ride.gtd").next().is_some(),
        "probe: editor not open after Rename click"
    );

    // Append to the seeded name and commit with Enter.
    h.inner.event(egui::Event::Text(" v2".to_owned()));
    h.step();
    h.inner.key_press(egui::Key::Enter);
    h.step();

    // After the worker renames and the window re-lists, the new identity shows.
    assert!(
        h.inner
            .step_until(|h| h.query_by_label_contains("ride.gtd v2").is_some()),
        "the renamed identity should appear in the refreshed list"
    );
}

/// The Logs column over a real database: the count the row shows, the names
/// listed in the menu, and the log the database reads back for the row that
/// was opened.
#[test]
fn the_logs_column_counts_the_stored_logs_and_opens_the_one_that_was_chosen() {
    let harness =
        history_harness_with_recording("auto:ride.gtd", &["navsyncd.log", "hal-powerd.log"]);
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(pump_history, harness);
    let count = format!("{ICON_PAPERCLIP} 2");
    assert!(
        h.inner
            .step_until(|h| h.query_by_label(count.as_str()).is_some()),
        "the row should count the two logs the recording stores"
    );

    // The count sits where it is clicked once the table's columns settle over
    // the frames after the list arrives.
    for _ in 0..4 {
        h.run();
    }

    h.inner.get_by_label(count.as_str()).click();
    h.step();
    h.inner.get_by_label("hal-powerd.log");
    h.inner.get_by_label("navsyncd.log");
    // The first row is "hal-powerd.log": the menu lists the attachments by
    // name.
    h.inner
        .nth_matching(By::new().label(OPEN_LOG_LABEL), 0)
        .click_accesskit();

    assert!(
        h.inner.step_until(|h| h.state().opened_log.is_some()),
        "the row should ask the database for the log it names"
    );
    assert_eq!(
        h.state()
            .opened_log
            .as_ref()
            .map(|log| (log.name.as_str(), log.text.as_str())),
        Some(("hal-powerd.log", stored_log_text("hal-powerd.log").as_str()))
    );
}

/// A recording storing no log leaves its Logs cell empty: a count of zero is
/// noise on every row of a database nobody attached a log in.
#[test]
fn a_recording_storing_no_log_shows_no_count() {
    let harness = history_harness(vec![entry_with_identity("auto:ride.gtd")]);
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    h.run();

    assert!(h.inner.query_by_label_contains(ICON_PAPERCLIP).is_none());
}

/// Never hidden, per DESIGN.md: in a read-only session the row's actions that
/// write are grayed and say the recording history is left as it is, while
/// opening a recording - which writes nothing - stays live.
#[test]
fn the_row_actions_that_write_are_grayed_in_a_read_only_session() {
    let mut harness = history_harness(vec![entry_with_identity("auto:ride.gtd")]);
    harness.write_access = WriteAccess::ReadOnly;
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    h.step();

    let delete = h.inner.get_by_label("Delete");
    assert!(delete.accesskit_node().is_disabled());
    let delete_center = delete.rect().center();
    assert!(
        !h.inner.get_by_label("Open").accesskit_node().is_disabled(),
        "opening a stored recording stays live: it writes nothing"
    );
    assert!(
        h.inner
            .get_by_label_contains("Prune…")
            .accesskit_node()
            .is_disabled()
    );

    h.inner.hover_at_and_settle(delete_center, 3);

    h.inner
        .get_by_label_contains(READ_ONLY_RECORDING_HISTORY_HOVER);
}

/// The rename a double click opens is a write too: a read-only session ignores
/// the double click, and the context menu's Rename is grayed.
#[test]
fn no_rename_editor_opens_in_a_read_only_session() {
    let mut harness = history_harness(vec![entry_with_identity("auto:ride.gtd")]);
    harness.write_access = WriteAccess::ReadOnly;
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .step_dt(1.0 / 60.0)
        .ui_state(show_history, harness);
    h.run();

    h.inner.get_by_label_contains("ride.gtd").click();
    h.inner.get_by_label_contains("ride.gtd").click();
    h.run();
    h.inner.event(egui::Event::Text(" v2".to_owned()));
    h.step();
    h.step();

    assert!(
        h.inner.query_all_by_value("ride.gtd v2").next().is_none(),
        "the read-only session opened the rename editor and typing reached it"
    );
    h.inner.get_by_label_contains("ride.gtd").click_secondary();
    h.step();
    assert!(
        h.inner
            .get_by_label("Rename")
            .accesskit_node()
            .is_disabled(),
        "the read-only session offers a rename that would be rejected"
    );
}

/// The window opened during startup says the database is opening: it is not
/// unavailable, it is not open yet.
#[test]
fn the_window_reports_the_databases_still_opening() {
    let mut harness = history_harness(vec![entry_with_identity("auto:ride.gtd")]);
    harness.databases_opening = true;
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    // The spinner repaints every frame, so the harness is stepped.
    h.inner.run_steps(2);

    h.inner
        .get_by_label_contains(crate::app::history::OPENING_RECORDINGS_DATABASE);
    assert!(
        h.inner.query_by_label_contains("ride.gtd").is_none(),
        "the list waits for the database"
    );
}

/// The recordings table: identity takes the remaining width (long names
/// get the room), the value columns stay compact, headers carry the
/// resize handles.
#[test]
fn snapshot_history_window_table() {
    let mut harness = history_harness(vec![
        with_stored_logs(entry_with_identity("auto:ride.gtd"), 2),
        entry_with_identity("a much longer recording identity that needs the room"),
        entry_with_identity("survey_flight_2026_07_15.gtd"),
    ]);
    // The temporary database path differs every run, so keep it out of the
    // image.
    harness.worker.hide_path();
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    // Auto columns measure their content over the first frames, so settle
    // before snapshotting.
    for _ in 0..4 {
        h.run();
    }
    h.snapshot("history_window_table");
}

#[test]
fn double_clicking_identity_opens_inline_editor() {
    let harness = history_harness(vec![entry_with_identity("auto:ride.gtd")]);
    // Frames at 60 fps: kittest's default 0.25 s/frame clock (one frame
    // per queued event) spaces the two clicks beyond egui's 0.3 s
    // double-click window.
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .step_dt(1.0 / 60.0)
        .ui_state(show_history, harness);
    h.run();
    // Two quick clicks on the identity label register as a double click
    // and swap the cell for the inline text editor (seeded with the
    // `auto:`-stripped name).
    h.inner.get_by_label_contains("ride.gtd").click();
    h.inner.get_by_label_contains("ride.gtd").click();
    h.run();
    assert!(
        h.inner.query_all_by_value("ride.gtd").next().is_some(),
        "inline editor should show the stripped identity as its value"
    );
    // The editor holds keyboard focus: typing extends its buffer.
    h.step();
    h.inner.event(egui::Event::Text(" v2".to_owned()));
    h.step();
    h.step();
    assert!(
        h.inner.query_all_by_value("ride.gtd v2").next().is_some(),
        "typed text should reach the freshly opened editor"
    );
}

/// Segmentation settings for a recording a test stores, matching
/// `SegmentationConfig::default`.
fn stored_segmentation() -> StoredSegmentation {
    StoredSegmentation {
        track_split_gap_us: 300_000_000,
        track_split_rule: StoredTrackSplitRule::StepInEitherDirection,
        fix_placement_rule: StoredFixPlacementRule::MissingHeadingAndNothingInFix,
        detect_clock_discontinuities: true,
        clock_discontinuity_sigmas: 5.0,
    }
}

/// A harness backed by a real database holding one recording whose stored track
/// table is `tracks`, which is what the shelf reads back through the worker.
///
/// The recording is stored with every track live, and the states are written
/// after through the API that takes them.
fn history_harness_with_stored_tracks(tracks: &[TrackRange]) -> HistoryHarness {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut db =
        gt_store::Recordings::open_or_create(&dir.path().join("history.h5")).expect("open db");
    let bytes = gt_test_utils::GOLD_BYTES;
    let meta = gt_store::extract_meta(bytes).expect("meta");
    let live: Vec<TrackRange> = tracks
        .iter()
        .map(|track| TrackRange {
            state: TrackState::Live,
            ..*track
        })
        .collect();
    let db_ref = db
        .insert("auto:ride.gtd", &meta, &live, stored_segmentation(), bytes)
        .expect("insert recording");
    db.set_tracks(&db_ref, tracks, stored_segmentation())
        .expect("write the stored track states");

    let worker = HistoryWorker::spawn(
        RecordingsHandle::Owner(db),
        egui::Context::default(),
        PendingWrites::default(),
    );
    let mut window = HistoryWindow::new();
    window.open = true;
    HistoryHarness {
        window,
        now: Instant::now(),
        worker,
        storage: crate::settings::StorageSettings {
            auto_prune_max_bytes: 0,
            ..crate::settings::StorageSettings::default()
        },
        databases_opening: false,
        write_access: WriteAccess::Owner,
        opened_log: None,
        _dir: dir,
    }
}

/// A four-row stored track table: a live track, the tombstone of a track
/// deleted permanently, and two shelved tracks.
fn one_live_a_tombstone_and_two_shelved_tracks() -> Vec<TrackRange> {
    vec![
        TrackRange {
            start: 0,
            end: 10,
            state: TrackState::Live,
        },
        TrackRange {
            start: 10,
            end: 10,
            state: TrackState::Deleted,
        },
        TrackRange {
            start: 10,
            end: 25,
            state: TrackState::Shelved,
        },
        TrackRange {
            start: 25,
            end: 40,
            state: TrackState::Shelved,
        },
    ]
}

/// How far the shelf tests drag the History window's bottom-right corner down:
/// enough to show the shelf's lines under the recording's row along with the
/// footer.
const SHELF_WINDOW_GROWN_BY: egui::Vec2 = egui::vec2(0.0, 140.0);

/// Open the shelf of the only listed recording and wait for its stored track
/// table to arrive.
fn open_the_shelf(h: &mut TestHarness<HistoryHarness>) {
    assert!(
        h.inner
            .step_until(|h| h.query_by_label_contains("ride.gtd").is_some()),
        "the recording should appear in the History list"
    );
    let window = h.inner.window_rect("History").expect("the window is shown");
    h.inner
        .press_drag_release(window.max, SHELF_WINDOW_GROWN_BY, 8);
    // The caret's column position moves for a few frames after the list
    // arrives, until the table settles.
    for _ in 0..4 {
        h.run();
    }
    h.inner.get_by_label(ICON_CARET_RIGHT).click();
    assert!(
        h.inner
            .step_until(|h| h.query_all_by_label(UNSHELVE_LABEL).next().is_some()),
        "the shelf should list the recording's shelved tracks"
    );
}

/// What the side panel's shelved-track mark requests: the window opens on that
/// recording's shelf. The recording has a row for the shelf to open under: the
/// listing's filters are cleared.
#[test]
fn open_shelf_opens_the_window_on_the_recordings_shelved_tracks() {
    let harness =
        history_harness_with_stored_tracks(&one_live_a_tombstone_and_two_shelved_tracks());
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(pump_history, harness);
    assert!(
        h.inner
            .step_until(|h| h.query_by_label_contains("ride.gtd").is_some()),
        "the recording should appear in the History list"
    );
    let db_ref = h
        .state()
        .window
        .entries
        .iter()
        .flatten()
        .map(|entry| entry.db_ref.clone())
        .next()
        .expect("the listed recording");
    h.state_mut().window.open = false;
    h.state_mut().window.filter_text = "another recording".to_owned();

    h.state_mut().window.open_shelf(db_ref);

    assert!(h.state().window.open, "the window should open on the shelf");
    assert!(
        h.inner.step_until(|h| h.query_by_label("#3").is_some()),
        "the shelf should list the recording's shelved tracks"
    );
    assert!(h.state().window.filter_text.is_empty());
}

/// A shelved track keeps the number it had before an earlier track was deleted
/// permanently: a track's number is its stored row plus one.
#[test]
fn the_shelf_numbers_a_shelved_track_by_its_stored_row() {
    let harness =
        history_harness_with_stored_tracks(&one_live_a_tombstone_and_two_shelved_tracks());
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(pump_history, harness);
    open_the_shelf(&mut h);

    h.inner.get_by_label("#3");
    h.inner.get_by_label("#4");
    assert!(
        h.inner.query_all_by_label("#2").next().is_none(),
        "the tombstone row is a track the shelf leaves out"
    );
}

/// What the listing does with the open shelf once the unshelve lands.
enum ShelfAfterTheUnshelve {
    StaysOpen,
    Closes,
}

/// Unshelving through the shelf writes the stored track states, which the
/// refreshed listing reports.
#[rstest::rstest]
#[case::one_track(UNSHELVE_LABEL, "3 (1 shelved)", ShelfAfterTheUnshelve::StaysOpen)]
#[case::every_track(UNSHELVE_ALL_LABEL, "3", ShelfAfterTheUnshelve::Closes)]
fn unshelving_from_the_shelf_leaves_the_tracks_live(
    #[case] button: &str,
    #[case] expected_track_count: &str,
    #[case] expected_shelf: ShelfAfterTheUnshelve,
) {
    let harness =
        history_harness_with_stored_tracks(&one_live_a_tombstone_and_two_shelved_tracks());
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(pump_history, harness);
    open_the_shelf(&mut h);

    h.inner.nth_matching(By::new().label(button), 0).click();

    assert!(
        h.inner.step_until(|h| {
            h.state()
                .window
                .entries
                .as_ref()
                .and_then(|entries| entries.first())
                .is_some_and(|entry| track_count_text(entry) == expected_track_count)
        }),
        "the refreshed listing should report the unshelved tracks as live"
    );
    match expected_shelf {
        ShelfAfterTheUnshelve::StaysOpen => assert!(
            h.state().window.shelf.is_some(),
            "the shelf should stay open on a recording with a shelved track left"
        ),
        ShelfAfterTheUnshelve::Closes => assert!(
            h.state().window.shelf.is_none(),
            "the shelf should close once the recording has no shelved track to list"
        ),
    }
}

/// Never hidden, per DESIGN.md: a read-only session still opens the shelf and
/// reads its tracks. Its three write controls stay grayed out.
#[test]
fn the_write_controls_of_the_shelf_are_grayed_in_a_read_only_session() {
    let mut harness =
        history_harness_with_stored_tracks(&one_live_a_tombstone_and_two_shelved_tracks());
    harness.write_access = WriteAccess::ReadOnly;
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(pump_history, harness);
    open_the_shelf(&mut h);

    let unshelve = h.inner.nth_matching(By::new().label(UNSHELVE_LABEL), 0);
    assert!(unshelve.accesskit_node().is_disabled());
    let unshelve_center = unshelve.rect().center();
    assert!(
        h.inner
            .get_by_label(UNSHELVE_ALL_LABEL)
            .accesskit_node()
            .is_disabled()
    );
    let delete_shelved = h.inner.get_by_label(ICON_TRASH);
    assert!(delete_shelved.accesskit_node().is_disabled());
    let delete_shelved_center = delete_shelved.rect().center();

    h.inner.hover_at_and_settle(unshelve_center, 3);
    h.inner
        .get_by_label_contains(READ_ONLY_RECORDING_HISTORY_HOVER);

    h.inner.hover_at_and_settle(delete_shelved_center, 3);
    h.inner
        .get_by_label_contains(READ_ONLY_RECORDING_HISTORY_HOVER);
}

/// The recording stays in history with the live track it keeps: the shelf's
/// delete takes the shelved tracks of its own recording only.
#[test]
fn deleting_the_shelved_tracks_from_the_shelf_leaves_the_recording_its_live_track() {
    let harness =
        history_harness_with_stored_tracks(&one_live_a_tombstone_and_two_shelved_tracks());
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(pump_history, harness);
    open_the_shelf(&mut h);

    h.inner.get_by_label(ICON_TRASH).click();
    // The confirmation is laid out over two passes, and egui reports a click
    // on its buttons from the second pass on.
    for _ in 0..4 {
        h.run();
    }
    h.inner.get_by_label(DELETE_SHELVED_TRACKS_LABEL).click();

    // The delete re-encodes the recording, which takes longer than one wait:
    // the listing the window drops on the way is the milestone between the two.
    assert!(
        h.inner.step_until(|h| h.state().window.entries.is_none()),
        "the delete should send the window back to the database for a fresh listing"
    );
    assert!(
        h.inner.step_until(|h| {
            h.state()
                .window
                .entries
                .as_ref()
                .and_then(|entries| entries.first())
                .is_some_and(|entry| track_count_text(entry) == "1")
        }),
        "the refreshed listing should report the one live track the recording keeps"
    );
    assert!(
        h.state().window.shelf.is_none(),
        "the shelf should close once the recording has no shelved track to list"
    );
}

/// The shelf open on a recording: one line per shelved track with its number
/// and nav-point count, and the closing line that unshelves all of them.
#[test]
fn snapshot_history_window_shelf_open() {
    let mut harness =
        history_harness_with_stored_tracks(&one_live_a_tombstone_and_two_shelved_tracks());
    harness.worker.hide_path();
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(pump_history, harness);
    open_the_shelf(&mut h);
    for _ in 0..4 {
        h.run();
    }

    h.snapshot("history_window_shelf_open");
}

/// The same shelf in a read-only session, both unshelve controls grayed.
#[test]
fn snapshot_history_window_shelf_open_read_only() {
    let mut harness =
        history_harness_with_stored_tracks(&one_live_a_tombstone_and_two_shelved_tracks());
    harness.worker.hide_path();
    harness.write_access = WriteAccess::ReadOnly;
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(pump_history, harness);
    open_the_shelf(&mut h);
    for _ in 0..4 {
        h.run();
    }

    h.snapshot("history_window_shelf_open_read_only");
}

/// A listing entry with the four sortable value columns set, for the
/// ordering tests. `duration_us` is added to `start_us` to give the entry
/// its time range.
fn sortable_entry(
    identity: &str,
    start_us: i64,
    duration_us: i64,
    nav_point_count: u64,
    gtd_size_bytes: u64,
) -> RecordingEntry {
    let mut entry = entry_with_identity(identity);
    entry.meta.time_range = NavPointTimeRange::covering(&[start_us, start_us + duration_us]);
    entry.meta.nav_point_count = nav_point_count;
    entry.meta.gtd_size_bytes = gtd_size_bytes;
    entry
}

/// Three entries whose columns disagree about the order, so sorting by any
/// one of them produces a different sequence: `beta` is the oldest but the
/// longest and biggest, `alpha` the newest but the shortest and the only one
/// storing no log.
fn sortable_entries() -> Vec<RecordingEntry> {
    vec![
        sortable_entry("Alpha", 3_000, 10, 5, 50),
        with_stored_logs(sortable_entry("beta", 1_000, 300, 100, 5_000), 2),
        with_stored_logs(sortable_entry("Gamma", 2_000, 60, 40, 400), 1),
    ]
}

/// The entry with `count` logs stored with it, each under a name of its own.
fn with_stored_logs(mut entry: RecordingEntry, count: usize) -> RecordingEntry {
    entry.log_attachments = (0..count)
        .map(|index| gt_store::LogAttachmentEntry {
            id: gt_store::LogAttachmentId::new_random(),
            attachment: gt_store::LogAttachment::new(
                format!("navsyncd-{index}.log"),
                gt_store::LogContentHash::of_log_bytes(&[]),
                Vec::new(),
            ),
        })
        .collect();
    entry
}

/// The identities the sort produces, in list order.
fn sorted_identities(sort: HistorySort, entries: &[RecordingEntry]) -> Vec<&str> {
    let mut visible: Vec<&RecordingEntry> = entries.iter().collect();
    sort.apply(&mut visible);
    visible.iter().map(|e| e.db_ref.identity.as_str()).collect()
}

/// Every column orders the list by its own value, in both directions.
/// Identity compares case-insensitively on the displayed name, so `beta`
/// sorts between `Alpha` and `Gamma`.
#[rstest::rstest]
#[case(SortColumn::Identity, SortDirection::Ascending, ["Alpha", "beta", "Gamma"])]
#[case(SortColumn::Identity, SortDirection::Descending, ["Gamma", "beta", "Alpha"])]
#[case(SortColumn::Date, SortDirection::Ascending, ["beta", "Gamma", "Alpha"])]
#[case(SortColumn::Date, SortDirection::Descending, ["Alpha", "Gamma", "beta"])]
#[case(SortColumn::Duration, SortDirection::Ascending, ["Alpha", "Gamma", "beta"])]
#[case(SortColumn::Duration, SortDirection::Descending, ["beta", "Gamma", "Alpha"])]
#[case(SortColumn::Points, SortDirection::Ascending, ["Alpha", "Gamma", "beta"])]
#[case(SortColumn::Points, SortDirection::Descending, ["beta", "Gamma", "Alpha"])]
#[case(SortColumn::Size, SortDirection::Ascending, ["Alpha", "Gamma", "beta"])]
#[case(SortColumn::Size, SortDirection::Descending, ["beta", "Gamma", "Alpha"])]
#[case(SortColumn::Logs, SortDirection::Ascending, ["Alpha", "Gamma", "beta"])]
#[case(SortColumn::Logs, SortDirection::Descending, ["beta", "Gamma", "Alpha"])]
fn sorting_orders_by_the_chosen_column(
    #[case] column: SortColumn,
    #[case] direction: SortDirection,
    #[case] expected: [&str; 3],
) {
    let entries = sortable_entries();
    let sort = HistorySort { column, direction };

    assert_eq!(sorted_identities(sort, &entries), expected.to_vec());
}

/// Entries that tie on the sorted column keep one stable order regardless of
/// direction, so equal rows do not shuffle when the sort is reversed.
#[test]
fn ties_break_stably_and_independently_of_direction() {
    // Same size, different identities - only the tie-break can separate them.
    let entries = vec![
        sortable_entry("charlie", 3_000, 10, 5, 100),
        sortable_entry("alpha", 1_000, 20, 9, 100),
        sortable_entry("bravo", 2_000, 30, 7, 100),
    ];
    let by_size = |direction| {
        sorted_identities(
            HistorySort {
                column: SortColumn::Size,
                direction,
            },
            &entries,
        )
    };

    assert_eq!(
        by_size(SortDirection::Ascending),
        ["alpha", "bravo", "charlie"]
    );
    assert_eq!(
        by_size(SortDirection::Descending),
        ["alpha", "bravo", "charlie"],
        "reversing the direction must not reshuffle rows that tie on the column",
    );
}

/// Clicking the active column reverses it. Clicking another switches to it
/// in that column's own natural direction.
#[test]
fn header_clicks_reverse_then_switch_columns() {
    let mut sort = HistorySort::default();
    assert_eq!(sort.column, SortColumn::Date);
    assert_eq!(sort.direction, SortDirection::Descending);

    sort.clicked(SortColumn::Date);
    assert_eq!(
        sort.direction,
        SortDirection::Ascending,
        "re-click reverses"
    );

    sort.clicked(SortColumn::Identity);
    assert_eq!(
        (sort.column, sort.direction),
        (SortColumn::Identity, SortDirection::Ascending),
        "identity starts A to Z",
    );

    sort.clicked(SortColumn::Size);
    assert_eq!(
        (sort.column, sort.direction),
        (SortColumn::Size, SortDirection::Descending),
        "size starts largest first, not carrying identity's ascending order",
    );
}

/// Every sortable column carries its own header title and a distinct hint
/// per direction, so no variant can be added without describing itself.
#[test]
fn every_sort_column_describes_itself() {
    let columns: Vec<SortColumn> = SortColumn::iter().collect();
    assert_eq!(
        columns.len(),
        SortColumn::COUNT,
        "the iterator must cover every variant",
    );

    let mut titles: Vec<&str> = columns.iter().map(|c| c.title()).collect();
    titles.sort_unstable();
    titles.dedup();
    assert_eq!(
        titles.len(),
        SortColumn::COUNT,
        "column titles must be unique"
    );

    for column in columns {
        assert_ne!(
            column.order_hint(SortDirection::Ascending),
            column.order_hint(SortDirection::Descending),
            "{column:?} must read differently in each direction",
        );
    }
}

/// The DB hands the listing the raw `meta_travel_mode` wire value. The
/// hover must show the human spelling for known modes and the preserved
/// wire value verbatim for unknown ones.
#[rstest::rstest]
#[case("bicycle", "Bicycle")]
#[case("hovercraft", "hovercraft")]
fn travel_mode_display_humanizes_the_wire_value(#[case] wire: &str, #[case] expected: &str) {
    assert_eq!(travel_mode_display(wire), expected);
}

/// A travel mode alone must badge the row with the note icon, proving
/// `identity_cell` feeds the field into the shared metadata presence check.
#[test]
fn travel_mode_alone_shows_the_metadata_note_icon() {
    let mut entry = entry_with_identity("auto:ride.gtd");
    entry.travel_mode = Some("bicycle".to_owned());
    let harness = history_harness(vec![entry]);
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    h.run();
    assert!(
        h.inner.query_by_label(ICON_NOTE).is_some(),
        "the note icon should appear for an entry whose only metadata is a travel mode"
    );
}

/// Settled width of the History window, through the real rendering path
/// ([`HistoryWindow::show`]). A resizable window runs a sizing pass over
/// its content, the path where an un-clipped column would report its
/// full text width and stretch the window.
fn history_window_width(identity: &str) -> f32 {
    let harness = history_harness(vec![entry_with_identity(identity)]);
    let mut h = TestHarness::builder()
        .size(egui::vec2(1600.0, 500.0))
        .ui_state(show_history, harness);
    h.inner
        .settled_window_size("History", 6)
        .expect("the History window is shown")
        .x
}

/// A long recording identity truncates in the History window: a short, a long,
/// and a much longer identity all settle the resizable window at the same
/// width. Without the truncation the identity column would size to its full
/// text and the window would grow with it.
#[test]
fn long_identity_does_not_widen_history_window() {
    let short = history_window_width("auto:ride.gtd");
    let long = history_window_width(&"a/very/long/recording/identity/".repeat(4));
    let longer = history_window_width(&"a/very/long/recording/identity/".repeat(12));
    assert!(
        (long - short).abs() < 1.0 && (longer - short).abs() < 1.0,
        "identity length changed the history window width: \
         short={short}px long={long}px longer={longer}px",
    );
}

/// The metadata-width measurement is ignored during the table's sizing pass:
/// on the first frame the auto columns have not grown to their content, so
/// the reserve reads far too small and, if cached, would inflate identity and
/// stick the window permanently wide. A freshly opened window must therefore
/// settle to its content width, not a bloated one.
#[test]
fn fresh_window_settles_to_content_width_not_a_bloated_one() {
    // Room to bloat into: the screen is 1600px, the content needs well under
    // half that. A leaked sizing-pass measurement pushed this past 900px.
    let width = history_window_width("auto:ride.gtd");
    assert!(
        width < 750.0,
        "the History window settled far wider than its content ({width:.0}px); \
         the sizing-pass metadata measurement likely leaked into the identity fill",
    );
}

/// The identity filter field fills the toolbar space to the left of the
/// action controls and must yield as the window narrows, never growing into
/// them. Previously the field kept a fixed width and the "Auto-store
/// recordings" checkbox slid left underneath it, overlapping.
#[test]
fn filter_field_does_not_overlap_the_toolbar_controls() {
    let harness = history_harness(vec![entry_with_identity("auto:ride.gtd")]);
    let mut h = TestHarness::builder()
        .size(egui::vec2(1200.0, 500.0))
        .ui_state(show_history, harness);
    for _ in 0..8 {
        h.step();
    }
    // Shrink toward the window's minimum, where the overlap used to appear.
    let w = window_rect(&h);
    h.inner.press_drag_release(
        egui::pos2(w.right() - 1.0, w.bottom() - 1.0),
        egui::vec2(-500.0, 0.0),
        10,
    );
    for _ in 0..3 {
        h.step();
    }

    let checkbox_left = h.inner.get_by_label(AUTO_STORE_LABEL).rect().left();
    // The first text input in the window is the identity filter field.
    let filter_right = h
        .inner
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .map(|n| n.rect())
        .next()
        .expect("identity filter field")
        .right();
    assert!(
        filter_right <= checkbox_left + 1.0,
        "the identity filter field (right edge {filter_right:.0}px) overlaps the \
         Auto-store checkbox (left edge {checkbox_left:.0}px)",
    );
}

/// A History window sized to a wide screen, populated with long identities
/// (they clip in the identity column), settled so the auto columns have
/// measured their content.
fn resize_harness() -> TestHarness<'static, HistoryHarness> {
    let long = "a/very/long/recording/identity/that/needs/lots/of/room/".repeat(2);
    let harness = history_harness(vec![
        entry_with_identity(&long),
        entry_with_identity(&format!("{long}/2")),
        entry_with_identity(&format!("{long}/3")),
    ]);
    let mut h = TestHarness::builder()
        .size(egui::vec2(1400.0, 600.0))
        .ui_state(show_history, harness);
    // Settle the sizing pass and let the window finish auto-positioning.
    for _ in 0..10 {
        h.step();
    }
    h
}

/// The rightmost content (the Delete button) relative to the window's right
/// edge. Identity fills the leftover width, so this "gap" is only the
/// window's frame padding - at every window size.
fn content_gap_to_window_edge(h: &TestHarness<HistoryHarness>) -> f32 {
    window_rect(h).right() - last_row_delete_button_rect(h).right()
}

fn last_row_delete_button_rect(h: &TestHarness<HistoryHarness>) -> egui::Rect {
    h.inner
        .get_all_by_label("Delete")
        .last()
        .expect("the Delete button of the listing's last row")
        .rect()
}

/// Identity fills the window at every size: the metadata columns keep their
/// content width and identity takes the rest. Growing or shrinking the
/// window keeps the table flush with the right edge and every column
/// on-screen - the table is always exactly as wide as the window.
#[test]
fn identity_fills_the_window_at_every_size() {
    let mut h = resize_harness();
    let settled_gap = content_gap_to_window_edge(&h);

    // Grow the window from its bottom-right corner.
    let before = window_rect(&h);
    h.inner.press_drag_release(
        egui::pos2(before.right() - 1.0, before.bottom() - 1.0),
        egui::vec2(300.0, 0.0),
        8,
    );
    for _ in 0..3 {
        h.step();
    }
    assert!(
        window_rect(&h).width() > before.width() + 200.0,
        "the window did not grow: {:.0}px -> {:.0}px",
        before.width(),
        window_rect(&h).width(),
    );
    assert!(
        (content_gap_to_window_edge(&h) - settled_gap).abs() < 4.0,
        "growing the window left a gap on the right - identity did not fill it",
    );

    // Shrink it back down. egui clamps the drag at the content's minimum
    // width (measured by a sizing pass when the drag starts), so stay well
    // above that floor: the identity-fill invariant is what matters here.
    let grown = window_rect(&h);
    h.inner.press_drag_release(
        egui::pos2(grown.right() - 1.0, grown.bottom() - 1.0),
        egui::vec2(-80.0, 0.0),
        8,
    );
    for _ in 0..3 {
        h.step();
    }
    assert!(
        window_rect(&h).width() < grown.width() - 40.0,
        "the window did not shrink: {:.0}px -> {:.0}px",
        grown.width(),
        window_rect(&h).width(),
    );
    assert!(
        (content_gap_to_window_edge(&h) - settled_gap).abs() < 4.0,
        "shrinking the window left a gap on the right - identity did not fill it",
    );
}

/// The figures a listing row states in its metadata columns.
struct RowFigures {
    nav_points: u64,
    total_tracks: usize,
    shelved_tracks: usize,
    gtd_size_bytes: u64,
}

/// A listing entry filling every metadata column at once: a date and a
/// duration from its time range, a nav-point count with the shelved-track
/// suffix beside it, and a size.
fn row_filling_every_metadata_column(
    RowFigures {
        nav_points,
        total_tracks,
        shelved_tracks,
        gtd_size_bytes,
    }: RowFigures,
) -> RecordingEntry {
    let mut entry = entry_with_shelved_tracks(
        "auto:ride.gtd",
        TotalTracks(total_tracks),
        ShelvedTracks(shelved_tracks),
    );
    entry.meta.time_range =
        NavPointTimeRange::covering(&[1_700_000_000_000_000, 1_700_003_660_000_000]);
    entry.meta.nav_point_count = nav_points;
    entry.meta.gtd_size_bytes = gtd_size_bytes;
    entry
}

#[rstest::rstest]
#[case::hundreds_of_points(RowFigures {
    nav_points: 199,
    total_tracks: 3,
    shelved_tracks: 2,
    gtd_size_bytes: 17_306,
})]
#[case::millions_of_points(RowFigures {
    nav_points: 12_300_000,
    total_tracks: 333,
    shelved_tracks: 222,
    gtd_size_bytes: 132_746_444,
})]
fn the_action_column_stays_inside_the_window_at_the_width_it_opens_at(#[case] figures: RowFigures) {
    let harness = history_harness(vec![row_filling_every_metadata_column(figures)]);
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    // The metadata columns measure their content over the first frames, and
    // identity takes what they leave on the frame after.
    for _ in 0..6 {
        h.run();
    }

    let window = window_rect(&h);
    assert!(
        (window.width() - DEFAULT_WINDOW_WIDTH_PX).abs() < 4.0,
        "the window is {:.1}px wide, and this test measures the \
         {DEFAULT_WINDOW_WIDTH_PX:.0}px width it opens at",
        window.width(),
    );
    let delete = last_row_delete_button_rect(&h);
    // The window lays its content out inside the frame margin.
    let ctx = &h.inner.ctx;
    let margin = ctx.style_of(ctx.theme()).spacing.window_margin.right;
    let content_right = window.right() - f32::from(margin);
    assert!(
        delete.right() <= content_right + 1.0,
        "Delete ends at {:.1}px, past the window's content edge at {content_right:.1}px: \
         the listing scrolls sideways and cuts the button off",
        delete.right(),
    );
}

#[test]
fn a_window_narrower_than_the_metadata_columns_keeps_the_shelf_caret() {
    let harness = history_harness(vec![row_filling_every_metadata_column(RowFigures {
        nav_points: 199,
        total_tracks: 3,
        shelved_tracks: 2,
        gtd_size_bytes: 17_306,
    })]);
    let mut h = TestHarness::builder()
        .size(NARROW_VIEWPORT)
        .ui_state(show_history, harness);
    for _ in 0..6 {
        h.run();
    }

    let caret = h.inner.get_by_label(ICON_CARET_RIGHT).rect();
    // The identity column ends where the Date column starts.
    let identity_right = header_node(&h, "Date").rect().left();
    assert!(
        caret.right() <= identity_right,
        "the shelf caret ends at {:.1}px, past the identity column's right edge at \
         {identity_right:.1}px in a {:.0}px viewport: the column clips the caret",
        caret.right(),
        NARROW_VIEWPORT.x,
    );
}

fn window_rect(h: &TestHarness<HistoryHarness>) -> egui::Rect {
    h.inner
        .window_rect("History")
        .expect("the History window is shown")
}

/// The window can be dragged narrower than its settled width. Identity
/// yields as the window shrinks, so the table follows the window down.
#[test]
fn the_window_can_be_shrunk_narrower() {
    let mut h = resize_harness();
    let before = window_rect(&h);
    // Drag the bottom-right resize corner inward.
    let corner = egui::pos2(before.right() - 1.0, before.bottom() - 1.0);
    h.inner
        .press_drag_release(corner, egui::vec2(-200.0, 0.0), 8);
    for _ in 0..3 {
        h.step();
    }
    let after = window_rect(&h);
    assert!(
        after.width() < before.width() - 50.0,
        "the window did not shrink: {:.1}px -> {:.1}px",
        before.width(),
        after.width(),
    );
}

/// The identities the table currently lists, top to bottom, read off the
/// rendered row positions.
fn listed_order(h: &TestHarness<HistoryHarness>, identities: &[&str]) -> Vec<String> {
    let mut rows: Vec<(f32, String)> = identities
        .iter()
        .map(|identity| {
            let top = h.inner.get_by_label_contains(identity).rect().top();
            (top, (*identity).to_owned())
        })
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter().map(|(_, identity)| identity).collect()
}

/// Click the table's header for `title`. The toolbar carries an "Identity"
/// label of its own, so match on the lowest node on screen - the header row
/// sits below the toolbar.
fn click_header(h: &TestHarness<HistoryHarness>, title: &str) {
    header_node(h, title).click();
}

/// The table header labelled exactly `title`.
///
/// Takes the lowest matching node: the toolbar and filter row carry labels
/// with the same words ("Identity", "Points") and sit above the table.
fn header_node<'t>(h: &'t TestHarness<HistoryHarness>, title: &'t str) -> egui_kittest::Node<'t> {
    h.inner.bottommost_matching(By::new().label(title))
}

/// Clicking a column header reorders the rendered table, and clicking the
/// same header again reverses it - the sort reaching the actual list, not
/// just the state struct.
#[test]
fn clicking_a_header_reorders_the_rendered_rows() {
    let harness = history_harness(sortable_entries());
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    for _ in 0..4 {
        h.run();
    }

    let identities = ["Alpha", "beta", "Gamma"];
    assert_eq!(
        listed_order(&h, &identities),
        ["Alpha", "Gamma", "beta"],
        "the default order is newest first",
    );

    // Sort by identity: a first click on a new column sorts it A to Z.
    click_header(&h, "Identity");
    h.run();
    assert_eq!(listed_order(&h, &identities), ["Alpha", "beta", "Gamma"]);

    // Clicking the active column reverses it.
    click_header(&h, "Identity");
    h.run();
    assert_eq!(listed_order(&h, &identities), ["Gamma", "beta", "Alpha"]);

    // Switching to Points sorts largest first.
    click_header(&h, "Points");
    h.run();
    assert_eq!(listed_order(&h, &identities), ["beta", "Gamma", "Alpha"]);
}

/// The active column is the only one showing a caret, and the caret follows
/// the direction - so the header always says how the list is ordered.
#[test]
fn only_the_active_column_shows_a_direction_caret() {
    let harness = history_harness(sortable_entries());
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    for _ in 0..4 {
        h.run();
    }

    // Default sort is Date descending: exactly one caret, pointing down.
    assert_eq!(h.inner.query_all_by_label(ICON_CARET_DOWN).count(), 1);
    assert_eq!(h.inner.query_all_by_label(ICON_CARET_UP).count(), 0);

    // Reversing it flips the caret without adding a second one.
    click_header(&h, "Date");
    h.run();
    assert_eq!(h.inner.query_all_by_label(ICON_CARET_DOWN).count(), 0);
    assert_eq!(h.inner.query_all_by_label(ICON_CARET_UP).count(), 1);
}

/// The recording ran, rebooted to a clock an hour behind and ran on: its
/// earliest and latest nav point times are neither the first nor the last nav
/// point in file order.
#[test]
fn a_row_reads_its_date_duration_and_time_range_from_the_earliest_and_latest_nav_point() {
    let time_range = NavPointTimeRange::covering(&[
        1_700_003_600_000_000,
        1_700_003_660_000_000,
        1_700_000_000_000_000,
        1_700_000_060_000_000,
    ]);

    assert_eq!(started_at_text(time_range), "2023-11-14 22:13");
    assert_eq!(duration_text(time_range), "1h 01m");
    assert_eq!(
        time_range_text(time_range),
        "2023-11-14 22:13:20 – 23:14:20"
    );
}

/// The three texts are the row's date cell, its duration cell and the time
/// range line of its breakdown.
#[test]
fn a_row_of_a_recording_with_no_time_range_reads_em_dashes() {
    assert_eq!(started_at_text(None), EM_DASH);
    assert_eq!(duration_text(None), EM_DASH);
    assert_eq!(time_range_text(None), EM_DASH);
}

/// A recording with no time range has no date and no duration to order on, and
/// sorts before every recording that has them.
#[rstest::rstest]
#[case(SortColumn::Date)]
#[case(SortColumn::Duration)]
fn a_recording_with_no_time_range_sorts_first_ascending(#[case] column: SortColumn) {
    let mut entries = sortable_entries();
    entries.push(entry_with_identity("no_time_range"));
    let sort = HistorySort {
        column,
        direction: SortDirection::Ascending,
    };

    assert_eq!(
        sorted_identities(sort, &entries).first().copied(),
        Some("no_time_range")
    );
}

#[rstest::rstest]
#[case::from("2023-01-01", "")]
#[case::to("", "2099-01-01")]
fn a_date_filter_leaves_out_a_recording_with_no_time_range(
    #[case] filter_date_from: &str,
    #[case] filter_date_to: &str,
) {
    let dated = sortable_entry(
        "auto:ride.gtd",
        1_700_000_000_000_000,
        60_000_000,
        10,
        1_024,
    );
    let mut harness = history_harness(vec![dated, entry_with_identity("auto:blank.gtd")]);
    harness.window.filter_date_from = filter_date_from.to_owned();
    harness.window.filter_date_to = filter_date_to.to_owned();
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    for _ in 0..4 {
        h.run();
    }

    h.inner.get_by_label_contains("ride.gtd");
    assert!(
        h.inner.query_by_label_contains("blank.gtd").is_none(),
        "a recording with no start date must not pass a date filter"
    );
}

/// A recording carrying ad-hoc sensor channels: two of them, one vector and
/// one scalar, plus counts for every data kind the breakdown reports.
fn entry_with_channels() -> RecordingEntry {
    let mut entry = sortable_entry(
        "auto:sensors.gtd",
        1_700_000_000_000_000,
        3_600_000_000,
        8_940,
        4_096,
    );
    entry.meta.sat_report_count = 1_234;
    entry.meta.marker_count = 12;
    entry.meta.event_marker_count = 3;
    entry.total_tracks = 4;
    entry.channels = vec![
        ChannelSummary {
            name: "accel".to_owned(),
            unit: Some("g".to_owned()),
            description: Some("Frame IMU".to_owned()),
            components: vec!["x".to_owned(), "y".to_owned(), "z".to_owned()],
            sample_count: 12_000,
        },
        ChannelSummary {
            name: "temperature".to_owned(),
            unit: None,
            description: None,
            components: Vec::new(),
            sample_count: 512,
        },
    ];
    entry
}

/// Park the pointer on the widget labelled `label` and hold it there until
/// the hover turns into a tooltip.
fn hover_widget(h: &mut TestHarness<HistoryHarness>, label: &str) {
    let target = topmost_labelled(h, label);
    h.inner.hover_at_and_settle(target, 4);
}

/// Point at the widget labelled `label` and stop before its tooltip opens.
///
/// For reading the cursor a widget requests. A tooltip is its own layer and a
/// big one lands over the pointer, which takes the hover off the widget
/// underneath and resets the cursor - so the cursor has to be read while
/// the widget is still the thing being pointed at.
fn point_at_widget(h: &mut TestHarness<HistoryHarness>, label: &str) {
    let target = topmost_labelled(h, label);
    h.inner.hover_at_and_settle(target, 1);
}

/// Like [`point_at_widget`] for a table header (see [`header_node`]).
fn point_at_header(h: &mut TestHarness<HistoryHarness>, title: &str) {
    let target = header_node(h, title).rect().center();
    h.inner.hover_at_and_settle(target, 1);
}

/// Centre of the topmost widget whose label contains `label`. The table row
/// sits above the footer summary when both hold the same text.
fn topmost_labelled(h: &TestHarness<HistoryHarness>, label: &str) -> egui::Pos2 {
    h.inner
        .topmost_matching(By::new().label_contains(label))
        .rect()
        .center()
}

/// Snapshot the hover breakdown for `entry`, rendered through the same
/// function the tooltip calls.
///
/// Called directly, so the image is just the breakdown: what it covers is
/// everything the breakdown itself determines - which rows appear, how the
/// channels lay out, and where it truncates.
/// That the hover actually reaches it is covered separately, by the tests
/// that hover a real row.
fn snapshot_breakdown(entry: &RecordingEntry, name: &str) {
    let mut h = TestHarness::builder()
        .size(egui::vec2(420.0, 560.0))
        .ui(|ui| data_breakdown_ui(ui, entry));
    for _ in 0..3 {
        h.run();
    }
    h.snapshot(name);
}

/// The breakdown of a recording carrying ad-hoc sensor channels: its span,
/// its shape on disk, a count per kind of data, and the channels - vector
/// components, units, and sample counts included.
#[test]
fn snapshot_history_row_breakdown() {
    snapshot_breakdown(&entry_with_channels(), "history_row_breakdown");
}

/// A recording with no channels states that in its breakdown. Its shelved
/// tracks also get the note explaining where they came from.
#[test]
fn snapshot_history_row_breakdown_without_channels() {
    let mut entry = sortable_entry(
        "auto:plain.gtd",
        1_700_000_000_000_000,
        900_000_000,
        42,
        4_096,
    );
    entry.total_tracks = 3;
    entry.shelved_tracks = 1;
    snapshot_breakdown(&entry, "history_row_breakdown_no_channels");
}

/// A recording with more channels than the hover lists shows the first
/// [`MAX_HOVER_CHANNELS`] and counts the rest, so the tooltip cannot grow
/// past the screen.
#[test]
fn snapshot_history_row_breakdown_truncates_long_channel_list() {
    let mut entry = entry_with_channels();
    entry.channels = (0..MAX_HOVER_CHANNELS + 3)
        .map(|i| ChannelSummary {
            // Zero-padded so the name order matches the numeric order.
            name: format!("channel_{i:02}"),
            unit: None,
            description: None,
            components: Vec::new(),
            sample_count: 10,
        })
        .collect();
    snapshot_breakdown(&entry, "history_row_breakdown_many_channels");
}

/// A vector channel shows its component labels. A scalar one is just its
/// name.
#[rstest::rstest]
#[case(&[], "accel")]
#[case(&["x", "y", "z"], "accel (x, y, z)")]
fn channel_title_appends_vector_components(#[case] components: &[&str], #[case] expected: &str) {
    let channel = ChannelSummary {
        name: "accel".to_owned(),
        unit: None,
        description: None,
        components: components.iter().map(|s| (*s).to_owned()).collect(),
        sample_count: 0,
    };

    assert_eq!(channel_title(&channel), expected);
}

/// The cursor the window requests right now.
fn cursor_icon(h: &TestHarness<HistoryHarness>) -> egui::CursorIcon {
    h.inner.output().platform_output.cursor_icon
}

/// Each part of the window requests the cursor that matches what it does:
/// only real text entry shows the I-beam, so a column header that sorts on
/// click shows the pointing hand.
#[rstest::rstest]
// Sortable headers act on click.
#[case::points_header(point_at_header, "Points", egui::CursorIcon::PointingHand)]
#[case::identity_header(point_at_header, "Identity", egui::CursorIcon::PointingHand)]
// The identity cell renames on double-click and has a context menu.
#[case::identity_cell(point_at_widget, "sensors.gtd", egui::CursorIcon::PointingHand)]
// The toolbar's "Identity" is a term with an explanation, not a control.
#[case::term_label(point_at_widget, "Identity", egui::CursorIcon::Help)]
// Values and captions do nothing on click.
#[case::date_cell(point_at_widget, "2023-11-14 22:13", egui::CursorIcon::Default)]
#[case::duration_cell(point_at_widget, "1h 00m", egui::CursorIcon::Default)]
#[case::points_cell(point_at_widget, "8.9k", egui::CursorIcon::Default)]
#[case::static_caption(point_at_widget, "GB", egui::CursorIcon::Default)]
#[case::button(point_at_widget, "Prune…", egui::CursorIcon::Default)]
#[case::checkbox(point_at_widget, AUTO_STORE_LABEL, egui::CursorIcon::Default)]
fn elements_request_a_cursor_that_matches_what_they_do(
    #[case] hover: fn(&mut TestHarness<HistoryHarness>, &str),
    #[case] label: &str,
    #[case] expected: egui::CursorIcon,
) {
    let mut h = channel_row_harness();

    hover(&mut h, label);

    assert_eq!(
        cursor_icon(&h),
        expected,
        "hovering {label:?} should request {expected:?}",
    );
}

/// The identity filter is real text entry, so it does get the I-beam - the
/// contrast that makes the cursor meaningful everywhere else.
#[test]
fn the_filter_field_still_shows_a_text_cursor() {
    let mut h = channel_row_harness();
    let field = h
        .inner
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .map(|n| n.rect())
        .next()
        .expect("identity filter field");

    h.inner.hover_at_and_settle(field.center(), 1);

    assert_eq!(cursor_icon(&h), egui::CursorIcon::Text);
}

/// A History window showing one recording that carries channels, settled so
/// the auto columns have measured their content.
fn channel_row_harness() -> TestHarness<'static, HistoryHarness> {
    let harness = history_harness(vec![entry_with_channels()]);
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    for _ in 0..4 {
        h.run();
    }
    h
}

/// Hovering *any* of a row's value cells brings up the breakdown, not just
/// the one column whose value is being pointed at - the cells are wired up
/// individually, so each one has to be checked.
#[rstest::rstest]
#[case::date("2023-11-14 22:13")]
#[case::duration("1h 00m")]
#[case::points("8.9k")]
#[case::size("4.0 KB")]
fn hovering_any_value_cell_reveals_the_breakdown(#[case] cell_text: &str) {
    let mut h = channel_row_harness();
    assert!(
        h.inner.query_by_label_contains("custom channel").is_none(),
        "probe: the breakdown must not be visible before the hover",
    );

    hover_widget(&mut h, cell_text);

    assert!(
        h.inner
            .query_by_label_contains("2 custom channels")
            .is_some(),
        "hovering the {cell_text:?} cell should reveal the row's breakdown",
    );
}

/// The breakdown lists the recording's ad-hoc sensor channels - their
/// component labels, units, and sample counts - which no table column
/// shows. This is the whole point of the hover.
#[test]
fn the_breakdown_names_the_recordings_channels() {
    let mut h = channel_row_harness();

    hover_widget(&mut h, "8.9k");

    for expected in [
        "2 custom channels",
        "accel (x, y, z)",
        "Frame IMU",
        "temperature",
        "12,000 samples",
        "512 samples",
        "Satellite reports",
        "1,234",
    ] {
        assert!(
            h.inner.query_by_label_contains(expected).is_some(),
            "the breakdown should mention {expected:?}",
        );
    }
}

/// The identity cell keeps its own metadata hover and gains the breakdown,
/// so the tooltip shows the same content wherever along the row it opens.
#[test]
fn hovering_the_identity_cell_shows_metadata_and_the_breakdown() {
    let mut entry = entry_with_channels();
    entry.title = Some("Morning ride".to_owned());
    let harness = history_harness(vec![entry]);
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    for _ in 0..4 {
        h.run();
    }

    hover_widget(&mut h, "sensors.gtd");

    for expected in [
        "Morning ride",
        "2 custom channels",
        "Double-click to rename",
    ] {
        assert!(
            h.inner.query_by_label_contains(expected).is_some(),
            "the identity hover should mention {expected:?}",
        );
    }
}

/// An identity too long for its column opens one tooltip, not two: egui
/// offers the elided text its own tooltip, and the cell's hover already
/// leads with the full identity.
#[test]
fn hovering_a_truncated_identity_opens_a_single_tooltip() {
    let long = "auto:a-recording-identity-far-too-long-for-the-identity-column.gtd";
    let harness = history_harness(vec![entry_with_identity(long)]);
    let mut h = TestHarness::builder()
        .size(egui::vec2(520.0, 300.0))
        .ui_state(show_history, harness);
    for _ in 0..4 {
        h.run();
    }

    hover_widget(&mut h, "a-recording-identity");

    assert!(
        h.inner
            .query_by_label_contains("Double-click to rename")
            .is_some(),
        "probe: the identity hover should be open",
    );
    assert_eq!(
        visible_tooltips(&h),
        1,
        "a truncated identity should not stack egui's elided-text tooltip \
         on top of the cell's own hover",
    );
}

/// How many tooltip layers are on screen.
fn visible_tooltips(h: &TestHarness<HistoryHarness>) -> usize {
    h.inner.ctx.memory(|m| {
        m.areas()
            .visible_layer_ids()
            .iter()
            .filter(|layer| layer.order == egui::Order::Tooltip)
            .count()
    })
}

/// A recording with no channels says so on hover.
#[test]
fn hovering_a_channel_free_row_says_it_has_none() {
    let harness = history_harness(vec![sortable_entry(
        "auto:plain.gtd",
        1_700_000_000_000_000,
        900_000_000,
        42,
        4_096,
    )]);
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    for _ in 0..4 {
        h.run();
    }

    hover_widget(&mut h, "42");

    assert!(
        h.inner
            .query_by_label_contains("No custom channels")
            .is_some(),
        "the breakdown should state that the recording carries no channels",
    );
}

/// Every value cell of a row gets its own breakdown widget id: one per
/// column, and different between rows. Dropping either part of the salt
/// would silently merge neighbouring cells' interaction state.
#[test]
fn breakdown_cell_ids_are_distinct_per_cell() {
    let entries = sortable_entries();
    let first = entries.first().expect("first entry");
    let second = entries.get(1).expect("second entry");

    let cells: Vec<egui::Id> = SortColumn::iter()
        .flat_map(|column| {
            [
                breakdown_cell_id(first, column),
                breakdown_cell_id(second, column),
            ]
        })
        .collect();
    let unique: std::collections::HashSet<egui::Id> = cells.iter().copied().collect();

    assert_eq!(
        unique.len(),
        cells.len(),
        "two breakdown cells share a widget id: {} cells produced {} ids",
        cells.len(),
        unique.len(),
    );
}

/// The track row calls out shelved tracks, and stays quiet when there are
/// none - it is the only place the breakdown mentions them.
#[rstest::rstest]
#[case(4, 0, "4")]
#[case(4, 1, "4 (1 shelved)")]
#[case(0, 0, "0")]
fn track_count_text_states_the_shelved_tracks(
    #[case] total_tracks: usize,
    #[case] shelved_tracks: usize,
    #[case] expected: &str,
) {
    let mut entry = entry_with_identity("auto:ride.gtd");
    entry.total_tracks = total_tracks;
    entry.shelved_tracks = shelved_tracks;

    assert_eq!(track_count_text(&entry), expected);
}

#[test]
fn identity_display_keeps_full_manual_identity_visible() {
    let identity = "/example.invalid/history/identity/with/slashes/";

    assert_eq!(identity_display_parts(identity), (identity, false));
}

#[test]
fn identity_display_marks_auto_identity_without_losing_original() {
    let identity = "auto:recording-2026-07-09.gtd";

    assert_eq!(
        identity_display_parts(identity),
        ("recording-2026-07-09.gtd", true)
    );
}

/// A History window filled with more recordings than any screen shows at once,
/// each under a long identity, so both axes overflow.
fn crowded_history_harness() -> HistoryHarness {
    let identity = gt_test_utils::oversized_text('r');
    let entries = (0..OVERSIZED_ROW_COUNT)
        .map(|index| {
            let mut entry = entry_with_identity(&format!("{identity}/{index}"));
            entry.meta.gtd_size_bytes = CROWDED_RECORDING_BYTES;
            entry
        })
        .collect();
    history_harness(entries)
}

/// Size given to each of [`crowded_history_harness`]'s recordings, so the footer
/// states a total.
const CROWDED_RECORDING_BYTES: u64 = 1024;

/// The stats line the footer ends on for [`crowded_history_harness`].
const CROWDED_FOOTER_STATS: &str = "200 recordings - 200.0 KB";

/// The History window keeps its footer reachable at any viewport: the listing
/// takes the room that is left and scrolls its own rows.
#[rstest::rstest]
fn history_window_fits_every_viewport(
    #[values(CRAMPED_VIEWPORT, NARROW_VIEWPORT, SHORT_VIEWPORT)] viewport: egui::Vec2,
) {
    let mut h = TestHarness::builder()
        .size(viewport)
        .ui_state(show_history, crowded_history_harness());
    for _ in 0..8 {
        h.step();
    }
    h.inner
        .assert_window_fits_the_viewport(AuditedWindow::titled("History"));
    h.inner.assert_control_is_reachable(
        AuditedWindow::titled("History"),
        ControlLabel(CROWDED_FOOTER_STATS),
    );
}

/// The prune dialog with a preview of far more recordings than any viewport
/// lists, each named at length.
fn crowded_prune_harness() -> HistoryHarness {
    let identity = gt_test_utils::oversized_text('r');
    let mut harness = history_harness(Vec::new());
    harness.window.prune.open = true;
    harness.window.set_prune_preview(
        (0..OVERSIZED_ROW_COUNT)
            .map(|index| DatabaseRef {
                identity: format!("{identity}/{index}"),
                group_name: format!("rec{index}"),
            })
            .collect(),
    );
    harness
}

/// The prune dialog keeps its destructive action reachable at any viewport: the
/// preview list scrolls its own rows.
#[rstest::rstest]
fn prune_dialog_fits_every_viewport(
    #[values(CRAMPED_VIEWPORT, NARROW_VIEWPORT, SHORT_VIEWPORT)] viewport: egui::Vec2,
) {
    let mut h = TestHarness::builder()
        .size(viewport)
        .ui_state(show_history, crowded_prune_harness());
    for _ in 0..8 {
        h.step();
    }
    h.inner
        .assert_window_fits_the_viewport(AuditedWindow::titled(PRUNE_WINDOW_TITLE));
    h.inner.assert_control_is_reachable(
        AuditedWindow::titled(PRUNE_WINDOW_TITLE),
        ControlLabel("Cancel"),
    );
}

/// Raise the confirmation over `scope`, at what the harness's own listing
/// reports the delete would take.
fn open_the_delete_shelved_confirmation(
    harness: &mut HistoryHarness,
    scope: DeleteShelvedTracksScope,
) {
    let window = &mut harness.window;
    let listing = window.entries.as_deref().unwrap_or_default();
    window.delete_shelved_prompt.open(scope, listing);
}

#[test]
fn snapshot_delete_shelved_confirmation() {
    let mut harness = history_harness(vec![entry_with_shelved_tracks(
        "auto:ride.gtd",
        TotalTracks(12),
        ShelvedTracks(3),
    )]);
    // The temporary database path differs every run.
    harness.worker.hide_path();
    open_the_delete_shelved_confirmation(&mut harness, DeleteShelvedTracksScope::EveryRecording);
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    for _ in 0..4 {
        h.run();
    }
    h.snapshot("delete_shelved_confirmation");
}

/// The confirmation states which recordings the delete removes from history
/// entirely, and writes them out under that line. A recording is one of them
/// when it holds only shelved tracks.
#[test]
fn snapshot_delete_shelved_confirmation_deleting_recordings_whole() {
    let entries = vec![
        entry_with_shelved_tracks("auto:ride.gtd", TotalTracks(4), ShelvedTracks(1)),
        entry_with_shelved_tracks("auto:walk.gtd", TotalTracks(2), ShelvedTracks(2)),
        entry_with_shelved_tracks("auto:sail.gtd", TotalTracks(3), ShelvedTracks(3)),
    ];
    let mut harness = history_harness(entries);
    harness.worker.hide_path();
    open_the_delete_shelved_confirmation(&mut harness, DeleteShelvedTracksScope::EveryRecording);
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    for _ in 0..4 {
        h.run();
    }
    h.snapshot("delete_shelved_confirmation_deleting_recordings_whole");
}

/// The confirmation the shelf raises: it states the one recording it takes the
/// tracks from, and that the delete removes this recording entirely.
#[test]
fn snapshot_delete_shelved_confirmation_for_one_recording() {
    let entries = vec![
        entry_with_shelved_tracks("auto:ride.gtd", TotalTracks(4), ShelvedTracks(1)),
        entry_with_shelved_tracks("auto:walk.gtd", TotalTracks(2), ShelvedTracks(2)),
    ];
    let walk = DatabaseRef {
        identity: "auto:walk.gtd".to_owned(),
        group_name: "rec0".to_owned(),
    };
    let mut harness = history_harness(entries);
    harness.worker.hide_path();
    open_the_delete_shelved_confirmation(
        &mut harness,
        DeleteShelvedTracksScope::OneRecording(walk),
    );
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    for _ in 0..4 {
        h.run();
    }
    h.snapshot("delete_shelved_confirmation_for_one_recording");
}

/// The confirmation once the last shelved track has gone from the recording
/// list: it stays up, states that there is nothing left to delete, grays the
/// delete out, and counts down on its Close button.
#[test]
fn snapshot_delete_shelved_confirmation_with_every_track_live() {
    let mut harness = history_harness(vec![entry_with_shelved_tracks(
        "auto:ride.gtd",
        TotalTracks(12),
        ShelvedTracks(3),
    )]);
    // The temporary database path differs every run.
    harness.worker.hide_path();
    open_the_delete_shelved_confirmation(&mut harness, DeleteShelvedTracksScope::EveryRecording);
    // The last shelved track goes while the confirmation is up.
    harness.window.set_entries(vec![entry_with_shelved_tracks(
        "auto:ride.gtd",
        TotalTracks(12),
        ShelvedTracks(0),
    )]);
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    // The test steps through the frames. The confirmation requests a repaint
    // every frame it counts down, and `h.run()` never settles while it does.
    for _ in 0..6 {
        h.step();
    }
    h.snapshot("delete_shelved_confirmation_every_track_live");
}

/// The delete-shelved confirmation stays inside the screen and keeps its
/// buttons reachable however many tracks it lists and however many recordings
/// it removes entirely.
#[rstest::rstest]
fn delete_shelved_confirmation_fits_every_viewport(
    #[values(CRAMPED_VIEWPORT, NARROW_VIEWPORT, SHORT_VIEWPORT)] viewport: egui::Vec2,
) {
    let identity = gt_test_utils::oversized_text('r');
    let entries: Vec<RecordingEntry> = (0..OVERSIZED_ROW_COUNT)
        .map(|index| {
            entry_with_shelved_tracks(
                &format!("{identity}/{index}"),
                TotalTracks(OVERSIZED_ROW_COUNT),
                ShelvedTracks(OVERSIZED_ROW_COUNT),
            )
        })
        .collect();
    let mut harness = history_harness(entries);
    open_the_delete_shelved_confirmation(&mut harness, DeleteShelvedTracksScope::EveryRecording);
    let mut h = TestHarness::builder()
        .size(viewport)
        .ui_state(show_history, harness);
    for _ in 0..8 {
        h.step();
    }
    h.inner
        .assert_window_fits_the_viewport(AuditedWindow::titled(DELETE_SHELVED_WINDOW_TITLE));
    h.inner.assert_control_is_reachable(
        AuditedWindow::titled(DELETE_SHELVED_WINDOW_TITLE),
        ControlLabel("Cancel"),
    );
}

/// Screen the height audit runs against: taller than a handful of recordings
/// need, shorter than [`OVERSIZED_ROW_COUNT`] of them.
const HEIGHT_AUDIT_VIEWPORT: egui::Vec2 = egui::vec2(1000.0, 800.0);

/// Settled height of the History window listing `rows` recordings, through the
/// real rendering path ([`HistoryWindow::show`]).
fn settled_history_window_height(rows: usize) -> f32 {
    let mut h = TestHarness::builder()
        .size(HEIGHT_AUDIT_VIEWPORT)
        .ui_state(show_history, history_harness_for_the_height_audit(rows));
    h.inner
        .settled_window_size("History", 10)
        .expect("the History window is shown")
        .y
}

/// A short listing leaves the History window at the height of its rows, well
/// under the height it opens at.
#[test]
fn a_short_list_settles_the_window_at_its_content_height() {
    let height = settled_history_window_height(SHORT_LIST_ROWS);
    assert!(
        height < 350.0,
        "the History window settled at {height:.0}px listing {SHORT_LIST_ROWS} \
         recordings, far more than three rows and the footer need: it stopped tracking \
         its content",
    );
}

/// More recordings than the window shows leave it at the height it opened at,
/// the rows scrolling inside it from there on.
#[test]
fn a_list_longer_than_the_window_leaves_it_at_the_height_it_opened_at() {
    let height = settled_history_window_height(OVERSIZED_ROW_COUNT);
    assert!(
        (height - DEFAULT_WINDOW_HEIGHT_PX).abs() < 1.0,
        "the History window settled at {height:.0}px listing {OVERSIZED_ROW_COUNT} \
         recordings, not the {DEFAULT_WINDOW_HEIGHT_PX:.0}px it opens at: its rows grew \
         it instead of scrolling inside it",
    );
}

/// A window that grew for a long listing fits a short one again: its height
/// matches the rows it draws now.
#[test]
fn a_window_that_listed_more_rows_than_it_shows_fits_a_short_list_again() {
    let mut h = TestHarness::builder().size(HEIGHT_AUDIT_VIEWPORT).ui_state(
        show_history,
        history_harness_for_the_height_audit(OVERSIZED_ROW_COUNT),
    );
    h.inner.run_steps(8);
    let grown = h.inner.window_rect("History").expect("the window is shown");

    h.inner.state_mut().window.set_entries(
        (0..SHORT_LIST_ROWS)
            .map(|index| entry_with_identity(&format!("auto:ride{index}.gtd")))
            .collect(),
    );
    h.inner.run_steps(8);
    let shrunk = h.inner.window_rect("History").expect("the window is shown");

    let opened_on_three = settled_history_window_height(SHORT_LIST_ROWS);
    assert!(
        (shrunk.height() - opened_on_three).abs() < 1.0,
        "the History window is {:.0}px tall listing {SHORT_LIST_ROWS} recordings, where a \
         window opened on the same {SHORT_LIST_ROWS} is {opened_on_three:.0}px: the \
         {OVERSIZED_ROW_COUNT} it listed before left it {:.0}px tall and it stayed there",
        shrunk.height(),
        grown.height(),
    );
}

/// A drag on the bottom edge shortens the window, and the listing scrolls in
/// what is left.
#[test]
fn a_drag_on_the_bottom_edge_shortens_the_window_for_good() {
    let mut h = TestHarness::builder().size(HEIGHT_AUDIT_VIEWPORT).ui_state(
        show_history,
        history_harness_for_the_height_audit(OVERSIZED_ROW_COUNT),
    );
    h.inner.run_steps(8);
    let before = h.inner.window_rect("History").expect("the window is shown");

    h.inner.press_drag_release(
        egui::pos2(before.center().x, before.bottom()),
        egui::vec2(0.0, -DRAGGED_UP_BY_PX),
        8,
    );
    h.inner.run_steps(8);
    let after = h.inner.window_rect("History").expect("the window is shown");

    assert!(
        (after.height() - (before.height() - DRAGGED_UP_BY_PX)).abs() < 1.0,
        "the History window is {:.0}px tall after a {DRAGGED_UP_BY_PX:.0}px drag up from \
         {:.0}px: the drag was undone by its rows",
        after.height(),
        before.height(),
    );
}

/// How far the drag audits pull the window's bottom edge up.
const DRAGGED_UP_BY_PX: f32 = 200.0;

/// Recordings the height audits call a short listing: fewer than the window
/// shows at once.
const SHORT_LIST_ROWS: usize = 3;

/// A harness listing `rows` recordings under names sorting in the order they
/// are built, with the database path kept out of the image.
fn history_harness_for_the_height_audit(rows: usize) -> HistoryHarness {
    let entries = (0..rows)
        .map(|index| {
            let mut entry = entry_with_identity(&format!("auto:ride{index:03}.gtd"));
            entry.meta.gtd_size_bytes = CROWDED_RECORDING_BYTES;
            entry
        })
        .collect();
    let mut harness = history_harness(entries);
    harness.worker.hide_path();
    harness
}

/// The History window listing more recordings than the screen holds.
#[test]
fn snapshot_listing_longer_than_the_screen() {
    let mut h = TestHarness::builder().size(HEIGHT_AUDIT_VIEWPORT).ui_state(
        show_history,
        history_harness_for_the_height_audit(OVERSIZED_ROW_COUNT),
    );
    for _ in 0..8 {
        h.run();
    }
    h.snapshot_loose("history_listing_longer_than_the_screen");
}

/// The History window after the user drags its bottom edge up while it lists
/// more recordings than the screen holds.
#[test]
fn snapshot_window_dragged_shorter_than_its_listing() {
    let mut h = TestHarness::builder().size(HEIGHT_AUDIT_VIEWPORT).ui_state(
        show_history,
        history_harness_for_the_height_audit(OVERSIZED_ROW_COUNT),
    );
    for _ in 0..8 {
        h.run();
    }
    let window = h.inner.window_rect("History").expect("the window is shown");
    h.inner.press_drag_release(
        egui::pos2(window.center().x, window.bottom()),
        egui::vec2(0.0, -DRAGGED_UP_BY_PX),
        8,
    );
    // No hover highlight or scrollbar is drawn over the listing: the hover
    // point is away from the rows.
    h.inner
        .hover_at_and_settle(egui::pos2(HEIGHT_AUDIT_VIEWPORT.x - 1.0, 1.0), 8);
    h.snapshot_loose("history_window_dragged_shorter");
}
