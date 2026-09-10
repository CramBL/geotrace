use std::path::Path;

use egui_kittest::{Harness, kittest::NodeT as _, kittest::Queryable as _};
use gt_instance_lock::TakeOverRecord;
use gt_jam_store::schema;
use gt_pending_writes::WriteAccess;
use gt_store::{
    EnvironmentArchive, InterruptedDelete, JamStore, ReadOnlyDayArchive as _, ReadOnlyJamStore,
};
use gt_test_utils::day_archive::{self, GroupPath};
use gt_test_utils::{By, HarnessInteraction as _, TestHarness};

use crate::app::App;
use crate::app::archive_recovery::{
    self, ARCHIVE_IN_USE_BUTTON_LABEL, ArchiveUnavailable, InspectedArchives,
    InterruptedDeleteFinding, LEAVE_UNRECOVERED_BUTTON_LABEL, RECOVER_BUTTON_LABEL,
    WRITE_ACCESS_TAKEN_FROM,
};
use crate::app::archives_unreachable::ArchivesUnreachable;
use crate::app::backfill_ui::DOWNLOAD_HISTORY_LABEL;
use crate::app::environment_storage::PrunedDays;
use crate::app::environment_storage_ui::{
    AUTO_PRUNE_LABEL as ENVIRONMENT_AUTO_PRUNE_LABEL, DELETE_ALL_LABEL, DeleteBlocker,
    PRUNE_BUTTON_LABEL,
};
use crate::app::read_only_session::READ_ONLY_RECORDING_HISTORY_HOVER;
use crate::app::settings_ui::SettingsPage;
use crate::app::storage_controls::AUTO_STORE_LABEL;
use crate::app::test_util;
use crate::app::ui_tests;

/// The interference archive's day index, which is where a delete records
/// that it is part-way through.
const INTERFERENCE_DAYS: GroupPath<'static> = GroupPath(schema::DAYS_GROUP);

/// The instance a recorded take-over in these cases took write access from.
const TAKEN_FROM_PROCESS_ID: u32 = 4321;

/// Stamps the take-over these cases record as one the archive was not written
/// after: it is later than the modification time of any archive written during
/// the test.
const TAKE_OVER_STAMPED_AHEAD_OF_THIS_MACHINES_CLOCK: u64 = 2_000_000_000;

/// A data directory whose interference archive holds two days with a delete
/// marked part-way through it, as an instance killed mid-delete leaves it.
fn data_directory_with_an_interrupted_interference_delete() -> tempfile::TempDir {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let path = store.archive_path::<JamStore>();
    {
        let archive = store
            .open_or_create_archive::<JamStore>()
            .expect("the interference archive");
        for offset in 0..2 {
            let day = chrono::NaiveDate::from_ymd_opt(2026, 7, 20).unwrap_or_default()
                + chrono::TimeDelta::days(offset);
            test_util::day_archive::archive_an_empty_interference_day(&archive, day);
        }
    }
    drop(store);
    day_archive::mark_delete_in_flight(&path, INTERFERENCE_DAYS).expect("mark the delete");
    dir
}

/// The app part-way through the open a take-over runs: the archives under
/// `root` have been read against the take-over the data directory recorded
/// before this one, and the prompts for what that found are up.
fn app_asking_about_the_archives_under<'a>(
    root: &Path,
    previous_take_over: Option<TakeOverRecord>,
) -> Harness<'a, App> {
    app_asking_about(archive_recovery::inspect_archives_under(
        root.to_owned(),
        previous_take_over,
    ))
}

/// The same, for findings this process cannot produce on its own: libhdf5
/// hands one process the same open file twice.
fn app_asking_about<'a>(inspected: InspectedArchives) -> Harness<'a, App> {
    let (mut harness, _databases) = ui_tests::app_with_the_databases_still_opening(&[]);
    harness
        .state_mut()
        .storage_open
        .inspect_archives_for_test(inspected);
    harness.run_steps(3);
    harness
}

fn wait_for_the_archives_to_open(harness: &mut Harness<'_, App>) {
    assert!(
        harness.step_until(|harness| harness.state().storage_open.databases_pending().is_none()),
        "the open the choices started never finished"
    );
}

/// The recovery an instance that is gone left behind is the user's to make
/// after a take-over: the prompt states the archive and what recovering costs,
/// and recovering opens it with those days discarded.
#[test]
fn recovering_after_a_take_over_opens_the_archive_with_its_days_discarded() {
    let dir = data_directory_with_an_interrupted_interference_delete();
    let path = gt_store::Store::open_in(dir.path()).archive_path::<JamStore>();
    let mut harness = app_asking_about_the_archives_under(dir.path(), None);

    harness.get_by_label_contains("Recover the aircraft interference archive?");
    harness.get_by_label_contains("discards the 2 archived days it holds");
    harness.get_by_label(RECOVER_BUTTON_LABEL).click();
    wait_for_the_archives_to_open(&mut harness);

    assert!(
        harness.state().jamming.writable_archive().is_some(),
        "the recovered archive is open"
    );
    assert_eq!(
        harness
            .state()
            .jamming
            .archived_days_covered(PrunedDays::All),
        0,
        "the recovery discarded the days the archive held"
    );
    assert_eq!(
        ReadOnlyJamStore::interrupted_delete_at(&path).expect("read the archive"),
        None,
        "the archive was opened with the interrupted delete still in it"
    );
    assert_eq!(
        harness.state().unavailable_archives[EnvironmentArchive::AircraftInterference],
        None
    );
}

/// A delete interrupted in an archive nothing has written since a take-over
/// is put to the user with when write access was taken and from which
/// process.
#[test]
fn the_recovery_prompt_states_the_take_over_the_archive_was_not_written_since() {
    let dir = data_directory_with_an_interrupted_interference_delete();

    let harness = app_asking_about_the_archives_under(
        dir.path(),
        Some(TakeOverRecord {
            taken_by_process_id: 1234,
            taken_from_process_id: Some(TAKEN_FROM_PROCESS_ID),
            written_at: Some(TAKE_OVER_STAMPED_AHEAD_OF_THIS_MACHINES_CLOCK),
        }),
    );

    harness.get_by_label_contains("Recover the aircraft interference archive?");
    harness.get_by_label_contains(
        "Write access to this data directory was taken from another GeoTrace (process 4321) on \
         2033-05-18 03:33 UTC.",
    );
}

#[test]
fn snapshot_recover_archive_prompt() {
    let dir = data_directory_with_an_interrupted_interference_delete();
    let harness = app_asking_about_the_archives_under(
        dir.path(),
        Some(TakeOverRecord {
            taken_by_process_id: 1234,
            taken_from_process_id: Some(TAKEN_FROM_PROCESS_ID),
            written_at: Some(TAKE_OVER_STAMPED_AHEAD_OF_THIS_MACHINES_CLOCK),
        }),
    );

    let mut harness = TestHarness::from_harness(harness);
    harness.snapshot_with_color_tolerance("recover_archive_prompt");
}

#[test]
fn snapshot_archive_in_use_prompt() {
    let dir = tempfile::tempdir().expect("temp dir");
    let harness = app_asking_about(InspectedArchives::of_findings_under(
        dir.path().to_owned(),
        vec![(
            EnvironmentArchive::AircraftInterference,
            InterruptedDeleteFinding::HeldByTheOtherInstance,
        )],
    ));

    let mut harness = TestHarness::from_harness(harness);
    harness.snapshot_with_color_tolerance("archive_in_use_prompt");
}

/// A take-over the archive was written after says nothing about the state
/// that write left, and a data directory may have no take-over recorded at all.
#[rstest::rstest]
#[case::no_take_over_recorded(None)]
#[case::a_take_over_the_archive_was_written_after(Some(TakeOverRecord {
    taken_by_process_id: 1234,
    taken_from_process_id: Some(TAKEN_FROM_PROCESS_ID),
    written_at: Some(1_700_000_000),
}))]
fn the_recovery_prompt_states_no_take_over_that_leaves_the_archive_unexplained(
    #[case] previous_take_over: Option<TakeOverRecord>,
) {
    let dir = data_directory_with_an_interrupted_interference_delete();

    let harness = app_asking_about_the_archives_under(dir.path(), previous_take_over);

    harness.get_by_label_contains("Recover the aircraft interference archive?");
    assert!(
        harness
            .query_by_label_contains(WRITE_ACCESS_TAKEN_FROM)
            .is_none(),
        "the prompt states a take-over that does not explain the interrupted delete"
    );
}

/// Leaving it alone costs the archive for the session and nothing on disk:
/// the file is byte-for-byte what it was, and the archives the user was never
/// asked about open beside it.
#[test]
fn leaving_an_interrupted_delete_unrecovered_writes_nothing_to_the_archive() {
    let dir = data_directory_with_an_interrupted_interference_delete();
    let path = gt_store::Store::open_in(dir.path()).archive_path::<JamStore>();
    let untouched = std::fs::read(&path).expect("the archive as the delete left it");
    let mut harness = app_asking_about_the_archives_under(dir.path(), None);

    harness.get_by_label(LEAVE_UNRECOVERED_BUTTON_LABEL).click();
    wait_for_the_archives_to_open(&mut harness);

    assert_eq!(
        std::fs::read(&path).expect("read the archive"),
        untouched,
        "the archive the user left alone was written to"
    );
    assert_eq!(
        ReadOnlyJamStore::interrupted_delete_at(&path).expect("read the archive"),
        Some(InterruptedDelete { archived_days: 2 }),
        "the days are gone, or the delete no longer reads as interrupted"
    );
    assert!(
        !harness.state().jamming.archive_available(),
        "the archive was opened after the user left it unrecovered"
    );
    assert_eq!(
        harness.state().unavailable_archives[EnvironmentArchive::AircraftInterference],
        Some(ArchiveUnavailable::InterruptedDeleteLeftUnrecovered)
    );
    assert!(
        harness.state().tec_maps.archive_available(),
        "one archive left closed closed the others too"
    );
}

/// Escape makes the same choice as the button that discards nothing, as it
/// does for every other destructive confirmation.
#[test]
fn escape_leaves_the_interrupted_delete_unrecovered() {
    let dir = data_directory_with_an_interrupted_interference_delete();
    let path = gt_store::Store::open_in(dir.path()).archive_path::<JamStore>();
    let mut harness = app_asking_about_the_archives_under(dir.path(), None);

    harness.key_press(egui::Key::Escape);
    wait_for_the_archives_to_open(&mut harness);

    assert_eq!(
        ReadOnlyJamStore::interrupted_delete_at(&path).expect("read the archive"),
        Some(InterruptedDelete { archived_days: 2 }),
        "escape recovered the interrupted delete"
    );
    assert_eq!(
        harness.state().unavailable_archives[EnvironmentArchive::AircraftInterference],
        Some(ArchiveUnavailable::InterruptedDeleteLeftUnrecovered)
    );
}

/// An archive the other GeoTrace still has open cannot be recovered here, so
/// no recovery is offered: the user is told what it costs and the open goes
/// on without it.
#[test]
fn an_archive_the_other_instance_holds_is_reported_as_in_use() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut harness = app_asking_about(InspectedArchives::of_findings_under(
        dir.path().to_owned(),
        vec![(
            EnvironmentArchive::AircraftInterference,
            InterruptedDeleteFinding::HeldByTheOtherInstance,
        )],
    ));

    harness.get_by_label_contains("The aircraft interference archive is in use");
    assert!(
        harness.query_by_label(RECOVER_BUTTON_LABEL).is_none(),
        "a recovery was offered for an archive this instance cannot open"
    );
    harness.get_by_label(ARCHIVE_IN_USE_BUTTON_LABEL).click();
    wait_for_the_archives_to_open(&mut harness);

    assert_eq!(
        harness.state().unavailable_archives[EnvironmentArchive::AircraftInterference],
        Some(ArchiveUnavailable::HeldByTheOtherInstance)
    );
    assert!(
        !harness.state().jamming.archive_available(),
        "the archive the other instance holds was opened here"
    );
}

/// Escape makes the in-use notice's one choice, so a stray keypress cannot
/// open an archive the other GeoTrace holds.
#[test]
fn escape_leaves_the_archive_the_other_instance_holds_alone() {
    let dir = tempfile::tempdir().expect("temp dir");
    let mut harness = app_asking_about(InspectedArchives::of_findings_under(
        dir.path().to_owned(),
        vec![(
            EnvironmentArchive::AircraftInterference,
            InterruptedDeleteFinding::HeldByTheOtherInstance,
        )],
    ));

    harness.key_press(egui::Key::Escape);
    wait_for_the_archives_to_open(&mut harness);

    assert_eq!(
        harness.state().unavailable_archives[EnvironmentArchive::AircraftInterference],
        Some(ArchiveUnavailable::HeldByTheOtherInstance)
    );
    assert!(
        !harness.state().jamming.archive_available(),
        "escape opened the archive the other instance holds"
    );
}

/// A delete interrupted after the archives were read is not recovered behind
/// the user's back: the open declines what the user was never asked about,
/// and the archive keeps its days.
#[test]
fn an_interrupted_delete_nobody_was_asked_about_is_declined() {
    let dir = data_directory_with_an_interrupted_interference_delete();
    let path = gt_store::Store::open_in(dir.path()).archive_path::<JamStore>();
    let mut harness = app_asking_about(InspectedArchives::of_findings_under(
        dir.path().to_owned(),
        Vec::new(),
    ));

    wait_for_the_archives_to_open(&mut harness);

    assert_eq!(
        ReadOnlyJamStore::interrupted_delete_at(&path).expect("read the archive"),
        Some(InterruptedDelete { archived_days: 2 }),
        "the open recovered a delete nobody was asked about"
    );
    assert_eq!(
        harness.state().unavailable_archives[EnvironmentArchive::AircraftInterference],
        Some(ArchiveUnavailable::InterruptedDeleteLeftUnrecovered)
    );
    assert!(harness.state().tec_maps.archive_available());
}

/// Never merely empty, per DESIGN.md: the controls that need an archive left
/// unrecovered are grayed and say why it is not there.
#[test]
fn an_archive_left_unrecovered_says_why_on_the_controls_that_need_it() {
    let dir = data_directory_with_an_interrupted_interference_delete();
    let mut harness = app_asking_about_the_archives_under(dir.path(), None);
    harness.get_by_label(LEAVE_UNRECOVERED_BUTTON_LABEL).click();
    wait_for_the_archives_to_open(&mut harness);

    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::AircraftInterference;
    harness.run_steps(3);
    harness.hover_and_settle(By::new().label_contains(DOWNLOAD_HISTORY_LABEL), 3);
    harness.get_by_label_contains(
        "The interference archive is unavailable this session: an interrupted delete in it was \
         left unrecovered",
    );

    harness.state_mut().settings_page = SettingsPage::Application;
    harness.run_steps(3);
    let interference_row = harness
        .topmost_matching(By::new().label_contains(DELETE_ALL_LABEL))
        .rect()
        .center();
    harness.hover_at_and_settle(interference_row, 3);
    harness.get_by_label_contains(
        &DeleteBlocker::ArchiveUnavailable(ArchiveUnavailable::InterruptedDeleteLeftUnrecovered)
            .hover_text(),
    );
}

/// The prompts are not a trap either: the window closes on request, and an
/// app on its way out leaves the archives closed.
#[test]
fn a_window_closed_while_an_interrupted_delete_is_asked_about_opens_nothing() {
    let dir = data_directory_with_an_interrupted_interference_delete();
    let path = gt_store::Store::open_in(dir.path()).archive_path::<JamStore>();
    let mut harness = app_asking_about_the_archives_under(dir.path(), None);
    harness.get_by_label_contains("Recover the aircraft interference archive?");
    assert!(
        harness.state().pending_writes.is_idle(),
        "a write is registered while the open waits on a person, which a close would wait for"
    );

    ui_tests::request_window_close(&mut harness);

    assert!(
        harness.step_until(ui_tests::closed_the_window),
        "the window never closed"
    );
    assert_eq!(
        ReadOnlyJamStore::interrupted_delete_at(&path).expect("read the archive"),
        Some(InterruptedDelete { archived_days: 2 }),
        "a closing app recovered the interrupted delete"
    );
    assert!(!harness.state().jamming.archive_available());
}

/// Never hidden, per DESIGN.md: the controls that need an archive are grayed
/// while the archives open, and say what they are waiting for.
#[test]
fn the_environment_controls_are_grayed_while_the_archives_open() {
    let (mut harness, databases) = ui_tests::app_with_the_databases_still_opening(&[]);
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::Application;
    harness.run_steps(3);

    for delete in harness.query_all_by_label_contains(DELETE_ALL_LABEL) {
        assert!(delete.accesskit_node().is_disabled());
    }
    let prune = harness.get_by_label_contains(PRUNE_BUTTON_LABEL);
    assert!(prune.accesskit_node().is_disabled());

    harness.hover_and_settle(By::new().label_contains(PRUNE_BUTTON_LABEL), 3);
    harness.get_by_label_contains(
        &DeleteBlocker::ArchivesUnreachable(ArchivesUnreachable::ArchivesOpening).hover_text(),
    );

    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    // A day to delete, or the control stays grayed for having nothing to act on.
    let day = chrono::NaiveDate::from_ymd_opt(2026, 7, 20).expect("date");
    test_util::day_archive::archive_an_empty_interference_day(
        &store
            .open_or_create_archive::<JamStore>()
            .expect("open the archive"),
        day,
    );
    ui_tests::land_the_databases(&mut harness, &databases, &store);
    harness.run_steps(3);

    assert!(
        !harness
            .get_by_label_contains(PRUNE_BUTTON_LABEL)
            .accesskit_node()
            .is_disabled(),
        "the archives landed, so the delete is live again"
    );
    assert!(
        harness
            .query_by_label_contains(
                &DeleteBlocker::ArchivesUnreachable(ArchivesUnreachable::ArchivesOpening)
                    .hover_text()
            )
            .is_none(),
        "the opening hover text outlived the open"
    );
}

/// Never hidden, per DESIGN.md: in a read-only session every control that
/// would write to an archive is grayed and says the session changes none.
#[test]
fn the_environment_controls_are_grayed_in_a_read_only_session() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    // A day to delete, or the controls stay grayed for having nothing to act
    // on, whatever the session may write.
    let day = chrono::NaiveDate::from_ymd_opt(2026, 7, 20).expect("date");
    test_util::day_archive::archive_an_empty_interference_day(
        &store
            .open_or_create_archive::<JamStore>()
            .expect("open the archive"),
        day,
    );
    let (mut harness, databases) =
        ui_tests::app_with_the_databases_still_opening_for(&[], WriteAccess::ReadOnly);
    ui_tests::land_the_databases(&mut harness, &databases, &store);
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::Application;
    harness.run_steps(3);

    assert_eq!(
        harness.state().environment_deletes_blocked_by(),
        Some(DeleteBlocker::ArchivesUnreachable(
            ArchivesUnreachable::ReadOnlySession
        ))
    );
    for delete in harness.query_all_by_label_contains(DELETE_ALL_LABEL) {
        assert!(delete.accesskit_node().is_disabled());
    }
    assert!(
        harness
            .get_by_label_contains(PRUNE_BUTTON_LABEL)
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        harness
            .get_by_label_contains(ENVIRONMENT_AUTO_PRUNE_LABEL)
            .accesskit_node()
            .is_disabled(),
        "the setting takes no input: a read-only session auto-prunes nothing"
    );
    harness.hover_and_settle(By::new().label_contains(PRUNE_BUTTON_LABEL), 3);
    harness.get_by_label_contains(
        &DeleteBlocker::ArchivesUnreachable(ArchivesUnreachable::ReadOnlySession).hover_text(),
    );

    harness.state_mut().settings_page = SettingsPage::AircraftInterference;
    harness.run_steps(3);

    assert!(
        harness
            .get_by_label_contains(DOWNLOAD_HISTORY_LABEL)
            .accesskit_node()
            .is_disabled()
    );
    harness.hover_and_settle(By::new().label_contains(DOWNLOAD_HISTORY_LABEL), 3);
    harness.get_by_label_contains("This session is read-only: nothing is downloaded into the");
}

/// The recording storage controls are grayed the same way: a read-only
/// session never stores or prunes a recording.
#[test]
fn the_recording_storage_controls_are_grayed_in_a_read_only_session() {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let (mut harness, databases) =
        ui_tests::app_with_the_databases_still_opening_for(&[], WriteAccess::ReadOnly);
    ui_tests::land_the_databases(&mut harness, &databases, &store);
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::Application;
    harness.run_steps(3);

    assert!(
        harness
            .get_by_label_contains(AUTO_STORE_LABEL)
            .accesskit_node()
            .is_disabled()
    );
    harness.hover_and_settle(By::new().label_contains(AUTO_STORE_LABEL), 3);
    harness.get_by_label_contains(READ_ONLY_RECORDING_HISTORY_HOVER);

    for auto_prune in ["Auto-prune when over", "Confirm before pruning"] {
        assert!(
            harness
                .get_by_label_contains(auto_prune)
                .accesskit_node()
                .is_disabled(),
            "{auto_prune} is live in a session that stores no recording"
        );
    }
}

/// The download control on a source page is grayed the same way: there is
/// nowhere to download to until the archive is open.
#[test]
fn the_download_control_is_grayed_while_the_archive_opens() {
    let (mut harness, _databases) = ui_tests::app_with_the_databases_still_opening(&[]);
    harness.state_mut().settings_open = true;
    harness.state_mut().settings_page = SettingsPage::AircraftInterference;
    harness.run_steps(3);

    let download = harness.get_by_label_contains(DOWNLOAD_HISTORY_LABEL);
    assert!(download.accesskit_node().is_disabled());

    harness.hover_and_settle(By::new().label_contains(DOWNLOAD_HISTORY_LABEL), 3);
    harness.get_by_label_contains("The interference archive is still opening");
}
