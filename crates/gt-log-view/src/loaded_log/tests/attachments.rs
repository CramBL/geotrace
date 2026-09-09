use gt_history_types::{LogAttachmentId, StoredLogFilter, StoredLogFilterMode};
use gt_loaded_files::{FileHistory, LoadedFiles};

use crate::anchor::RecordingKey;
use crate::attachment::LogAttachmentRef;
use crate::loaded_log::tests::fixtures;
use crate::loaded_log::{LoadedLog, LoadedLogs, RestoredAttachmentAdoption};
use crate::test_util;

/// An attachment is written again only for the edits the database has not
/// seen, and the log stays loaded once the attachment is gone.
#[test]
fn an_attached_log_reports_the_filter_stack_edits_the_database_has_not_seen() {
    let attachment = fixtures::attachment_ref();
    let recordings = test_util::loaded(Vec::new());
    let mut logs = LoadedLogs::default();
    let mut log = test_util::log_of(10);
    log.record_attachment(attachment.clone(), Vec::new(), &recordings.view());
    let id = logs.push(log).id();

    assert!(
        logs.take_filter_stack_edits_to_store().is_empty(),
        "the database holds the stack the log was attached with"
    );

    fixtures::add_layer_chip(&mut logs, id, "entry 1");

    let edits = logs.take_filter_stack_edits_to_store();
    assert_eq!(
        edits
            .iter()
            .map(|(attachment, filters)| (attachment.id, filters.as_slice()))
            .collect::<Vec<_>>(),
        [(
            attachment.id,
            [StoredLogFilter {
                text: "entry 1".to_owned(),
                regex: false,
                enabled: true,
                mode: StoredLogFilterMode::Layer { color_slot: 0 },
            }]
            .as_slice()
        )],
        "the added chip is what the attachment has yet to be written"
    );
    assert!(
        logs.take_filter_stack_edits_to_store().is_empty(),
        "an edit that was written is not written again"
    );

    if let Some(log) = logs.get_mut_by_id(id) {
        log.forget_attachment();
    }
    assert_eq!(logs.len(), 1);
    assert_eq!(logs.get_by_id(id).and_then(LoadedLog::attachment), None);
}

/// A log restored from an attachment comes back with its chips, drawing in
/// the colours it was stored with.
#[test]
fn a_restored_attachment_puts_back_the_stack_it_was_stored_with() {
    let stored = vec![
        StoredLogFilter {
            text: "entry 1".to_owned(),
            regex: false,
            enabled: true,
            mode: StoredLogFilterMode::Layer { color_slot: 2 },
        },
        StoredLogFilter {
            text: "entry".to_owned(),
            regex: false,
            enabled: false,
            mode: StoredLogFilterMode::Refine,
        },
    ];
    let attachment = fixtures::attachment_ref();
    let recordings = test_util::loaded(Vec::new());
    let mut logs = LoadedLogs::default();
    let mut log = test_util::log_of(10);
    log.restore_attachment(attachment.clone(), stored.clone(), &recordings.view());
    let id = logs.push(log).id();
    fixtures::wait_for_scans(&mut logs);

    let log = logs.get_by_id(id).expect("the restored log is loaded");
    assert_eq!(log.attachment(), Some(&attachment));
    assert_eq!(
        log.filters().to_stored_filters(),
        stored,
        "the restored stack is the stored one, colours and all"
    );
    assert!(
        logs.take_filter_stack_edits_to_store().is_empty(),
        "a stack that came back as it was stored needs no write-back"
    );
}

/// The anchor and attachment the loaded log holds when
/// [`LoadedLog::adopt_restored_attachment`] is called on it.
#[derive(Debug, Clone, Copy)]
enum LoadedLogBeforeTheRestore {
    AnchoredToTheRestoringRecording,
    AnchoredToAnotherRecording,
    HoldingAnAttachmentOfItsOwn,
}

/// A recording load reads back an attachment holding the text of a log the
/// session already has. That log takes the attachment where it is anchored
/// to that recording and holds none: an anchor moves only where the user
/// moves it.
#[rstest::rstest]
#[case(
    LoadedLogBeforeTheRestore::AnchoredToTheRestoringRecording,
    RestoredAttachmentAdoption::Recorded
)]
#[case(
    LoadedLogBeforeTheRestore::AnchoredToAnotherRecording,
    RestoredAttachmentAdoption::NotAnchoredToThatRecording
)]
#[case(
    LoadedLogBeforeTheRestore::HoldingAnAttachmentOfItsOwn,
    RestoredAttachmentAdoption::AlreadyAttached
)]
fn a_restored_attachment_reaches_the_loaded_log_anchored_to_the_recording_holding_it(
    #[case] before: LoadedLogBeforeTheRestore,
    #[case] expected: RestoredAttachmentAdoption,
) {
    let db_ref = test_util::stored_recording_ref();
    let mut files = LoadedFiles::new();
    files.push(
        test_util::recording_at(55.0, 10),
        test_util::stored_in_history(&db_ref),
    );
    files.push(test_util::recording_at(60.0, 10), FileHistory::None);
    let restored = LogAttachmentRef {
        recording: db_ref.clone(),
        id: LogAttachmentId::new_random(),
    };
    let mut log = test_util::log_of(10);
    match before {
        LoadedLogBeforeTheRestore::AnchoredToTheRestoringRecording => {
            log.anchor_to(RecordingKey::Stored(db_ref), &files.view());
        }
        LoadedLogBeforeTheRestore::AnchoredToAnotherRecording => {
            test_util::anchor_to(&mut log, &files, 1);
        }
        LoadedLogBeforeTheRestore::HoldingAnAttachmentOfItsOwn => {
            log.record_attachment(fixtures::attachment_ref(), Vec::new(), &files.view());
        }
    }
    let anchored_before = log.anchor_key().cloned();

    let adoption = log.adopt_restored_attachment(restored.clone(), Vec::new(), &files.view());

    assert_eq!(adoption, expected);
    assert_eq!(
        log.attachment() == Some(&restored),
        expected == RestoredAttachmentAdoption::Recorded,
        "only the log anchored to that recording takes the attachment"
    );
    assert_eq!(
        log.anchor_key(),
        anchored_before.as_ref(),
        "the log keeps the anchor it had either way"
    );
}

/// The adopting log keeps the chips the user is reading it under, and
/// writes that stack to the attachment it took.
#[test]
fn a_log_that_takes_a_restored_attachment_keeps_its_own_filter_stack() {
    let db_ref = test_util::stored_recording_ref();
    let mut files = LoadedFiles::new();
    files.push(
        test_util::recording_at(55.0, 10),
        test_util::stored_in_history(&db_ref),
    );
    let mut logs = LoadedLogs::default();
    let mut log = test_util::log_of(10);
    log.anchor_to(RecordingKey::Stored(db_ref.clone()), &files.view());
    let id = logs.push(log).id();
    fixtures::add_layer_chip(&mut logs, id, "entry 1");
    let stored = vec![StoredLogFilter {
        text: "entry 2".to_owned(),
        regex: false,
        enabled: true,
        mode: StoredLogFilterMode::Layer { color_slot: 0 },
    }];
    let restored = LogAttachmentRef {
        recording: db_ref,
        id: LogAttachmentId::new_random(),
    };

    if let Some(log) = logs.get_mut_by_id(id) {
        log.adopt_restored_attachment(restored.clone(), stored, &files.view());
    }

    assert_eq!(
        logs.get_by_id(id)
            .map(|log| log.filters().to_stored_filters()),
        Some(vec![StoredLogFilter {
            text: "entry 1".to_owned(),
            regex: false,
            enabled: true,
            mode: StoredLogFilterMode::Layer { color_slot: 0 },
        }])
    );
    assert_eq!(
        logs.take_filter_stack_edits_to_store()
            .into_iter()
            .map(|(attachment, filters)| (
                attachment,
                filters.into_iter().map(|filter| filter.text).collect()
            ))
            .collect::<Vec<(LogAttachmentRef, Vec<String>)>>(),
        [(restored, vec!["entry 1".to_owned()])],
        "the stack the user is reading is what the attachment is written"
    );
}
