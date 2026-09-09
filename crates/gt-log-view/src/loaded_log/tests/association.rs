use chrono::Duration;
use gt_loaded_files::LoadedFiles;

use crate::anchor::RecordingKey;
use crate::loaded_log::tests::fixtures;
use crate::loaded_log::{LoadedLog, LoadedLogs};
use crate::test_util;

/// The single-target rule: two recordings run at the same time in different
/// places, and the log takes its positions from the one it was pointed at.
#[test]
fn entries_take_their_position_from_the_chosen_recording_alone() {
    let files = test_util::loaded(vec![
        test_util::recording_at(55.0, 10),
        test_util::recording_at(60.0, 10),
    ]);
    let mut log = test_util::log_of(10);

    test_util::anchor_to(&mut log, &files, 1);

    assert_eq!(log.associated_entry_count(), 10);
    let latitudes: Vec<f64> = (0..10)
        .filter_map(|entry| log.entry_placement(entry))
        .map(|placement| placement.position.0.as_degrees())
        .collect();
    assert!(
        latitudes.iter().all(|lat| *lat >= 60.0),
        "every entry must land in the chosen recording, got {latitudes:?}"
    );
}

#[test]
fn widening_the_window_associates_the_entries_the_narrower_one_missed() {
    let files = test_util::loaded(vec![test_util::recording_at(55.0, 3)]);
    let mut log = test_util::log_of(10);

    log.set_association_window(Duration::seconds(1), &files.view());
    test_util::anchor_to(&mut log, &files, 0);
    assert_eq!(log.associated_entry_count(), 4);
    assert_eq!(log.unassociated_entry_count(), 6);

    log.set_association_window(Duration::seconds(60), &files.view());
    assert_eq!(log.associated_entry_count(), 10);
    assert_eq!(log.unassociated_entry_count(), 0);
}

/// Unloading the anchored recording strands the log, and never hands it to
/// the other loaded recording.
#[test]
fn unloading_the_anchored_recording_leaves_the_log_anchored_without_positions() {
    let mut files = test_util::loaded(vec![
        test_util::recording_at(55.0, 10),
        test_util::recording_at(60.0, 10),
    ]);
    let anchored = test_util::key_of(&files, 1);
    let mut logs = LoadedLogs::default();
    let mut log = test_util::log_of(10);
    test_util::anchor_to(&mut log, &files, 1);
    let id = logs.push(log).id();

    files.remove_file(1);
    logs.reassociate_all(&files.view());

    let log = logs.get_by_id(id).expect("the log stays loaded");
    assert_eq!(log.anchor_key(), Some(&anchored));
    assert_eq!(log.associated_recording(), None);
    assert_eq!(log.associated_entry_count(), 0);
    assert_eq!(log.entry_placement(0), None);
}

/// A recording in history is the same recording every time it is opened,
/// however many session identities it goes through.
#[test]
fn a_log_anchored_to_a_stored_recording_associates_again_when_it_is_opened_again() {
    let db_ref = test_util::stored_recording_ref();
    let mut files = LoadedFiles::new();
    files.push(
        test_util::recording_at(55.0, 10),
        test_util::stored_in_history(&db_ref),
    );
    let first_load = test_util::id_of(&files, 0);
    let mut log = test_util::log_of(10);
    test_util::anchor_to(&mut log, &files, 0);
    assert_eq!(log.associated_entry_count(), 10);

    files.remove_file(0);
    log.reassociate(&files.view());
    assert_eq!(log.associated_entry_count(), 0);

    files.push(
        test_util::recording_at(55.0, 10),
        test_util::stored_in_history(&db_ref),
    );
    log.reassociate(&files.view());

    assert_eq!(log.anchor_key(), Some(&RecordingKey::Stored(db_ref)));
    assert_eq!(log.associated_entry_count(), 10);
    assert_eq!(
        log.associated_recording(),
        Some(test_util::id_of(&files, 0))
    );
    assert_ne!(
        log.associated_recording(),
        Some(first_load),
        "the recording came back under a session identity of its own"
    );
}

/// The recording an attached log is stored with is the recording it takes
/// its positions from, until the attachment is removed.
#[test]
fn an_attached_log_keeps_its_anchor() {
    let files = test_util::loaded(vec![test_util::recording_at(55.0, 10)]);
    let mut log = test_util::log_of(10);
    test_util::anchor_to(&mut log, &files, 0);
    log.record_attachment(fixtures::attachment_ref(), Vec::new(), &files.view());
    let anchored = log.anchor_key().cloned();

    log.remove_anchor();

    assert_eq!(log.anchor_key(), anchored.as_ref());

    log.forget_attachment();
    log.remove_anchor();

    assert_eq!(log.anchor_key(), None);
}

/// A recording leaving the session takes the logs anchored to it with it,
/// and leaves the rest of the session's logs loaded.
#[test]
fn unloading_a_recording_unloads_the_logs_anchored_to_it() {
    let files = test_util::loaded(vec![
        test_util::recording_at(55.0, 10),
        test_util::recording_at(60.0, 10),
    ]);
    let mut logs = LoadedLogs::default();
    let mut anchored = test_util::log_of(10);
    test_util::anchor_to(&mut anchored, &files, 0);
    logs.push(anchored);
    let mut elsewhere = test_util::log_of_service("hal-powerd", 10);
    test_util::anchor_to(&mut elsewhere, &files, 1);
    let elsewhere = logs.push(elsewhere).id();

    let unloaded = logs.unload_anchored_to(&[test_util::key_of(&files, 0)]);

    assert_eq!(
        unloaded.iter().map(LoadedLog::name).collect::<Vec<_>>(),
        ["navsyncd.log"]
    );
    assert_eq!(
        logs.iter_with_ids().map(|(id, _)| id).collect::<Vec<_>>(),
        [elsewhere]
    );
}
