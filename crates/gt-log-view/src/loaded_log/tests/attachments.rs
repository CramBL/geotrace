use std::sync::Arc;

use chrono::{Datelike as _, Duration};
use gt_history_types::{LogAttachmentId, StoredLogFilter, StoredLogFilterMode};
use gt_loaded_files::{FileHistory, LoadedFiles};
use gt_types::FileIdx;

use crate::anchor::RecordingKey;
use crate::attachment::LogAttachmentRef;
use crate::loaded_log::tests::fixtures;
use crate::loaded_log::{LoadedLog, LoadedLogs, LogContextOrigin, LogPushOutcome, LogSaveOutcome};
use crate::test_util;

#[rstest::rstest]
#[case(true, FileHistory::None)]
#[case(
    false,
    test_util::stored_in_history(&test_util::recording_ref_of_group("2026-01-02T14-02-11"))
)]
fn an_attached_log_keeps_its_recording_after_another_source_is_selected(
    #[case] attached_recording_loaded: bool,
    #[case] other_recording_history: FileHistory,
) {
    let attachment = fixtures::attachment_ref();
    let mut files = LoadedFiles::new();
    if attached_recording_loaded {
        files.push(
            test_util::recording_at(55.0, 10),
            test_util::stored_in_history(&attachment.recording),
        );
    }
    files.push(test_util::recording_at(60.0, 10), other_recording_history);
    let other_key = test_util::key_of(&files, files.len() - 1);
    let other = test_util::id_of(&files, files.len() - 1);
    let mut log = test_util::log_of(10);
    log.anchor_to_loaded_recording(Some(other), &files.view());

    log.record_attachment(attachment.clone(), Vec::new(), &files.view());

    let key = RecordingKey::Stored(attachment.recording.clone());
    let placement = log.entry_placement(0);
    let associated = log.associated_recording();
    assert_eq!(log.anchor_key().as_ref(), Some(&key));
    assert_eq!(
        log.associated_entry_count(),
        if attached_recording_loaded { 10 } else { 0 }
    );

    log.anchor_to_loaded_recording(Some(other), &files.view());

    assert_eq!(log.anchor_key().as_ref(), Some(&key));
    assert_eq!(log.attachment(), Some(&attachment));
    assert_eq!(log.associated_recording(), associated);
    assert_eq!(log.entry_placement(0), placement);

    log.forget_attachment();
    log.anchor_to_loaded_recording(Some(other), &files.view());

    assert_eq!(log.attachment(), None);
    assert_eq!(log.associated_recording(), Some(other));
    assert_eq!(log.anchor_key().as_ref(), Some(&other_key));
    assert_eq!(log.associated_entry_count(), 10);
}

#[test]
fn an_attached_log_accepts_a_reloaded_instance_of_its_recording() {
    let attachment = fixtures::attachment_ref();
    let mut files = LoadedFiles::new();
    files.push(
        test_util::recording_at(55.0, 10),
        test_util::stored_in_history(&attachment.recording),
    );
    let previous = test_util::id_of(&files, 0);
    let mut log = test_util::log_of(10);
    log.restore_attachment(attachment.clone(), Vec::new(), &files.view());
    files.remove_file(0);
    log.reassociate(&files.view());
    assert_eq!(log.associated_recording(), None);
    files.push(
        test_util::recording_at(55.0, 10),
        test_util::stored_in_history(&attachment.recording),
    );
    let reloaded = test_util::id_of(&files, 0);

    log.anchor_to_loaded_recording(Some(reloaded), &files.view());

    assert_ne!(reloaded, previous);
    assert_eq!(
        log.anchor_key().as_ref(),
        Some(&RecordingKey::Stored(attachment.recording.clone()))
    );
    assert_eq!(log.attachment(), Some(&attachment));
    assert_eq!(log.associated_recording(), Some(reloaded));
    assert_eq!(log.associated_entry_count(), 10);
}

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

    logs.forget_attachment(&attachment);
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

#[rstest::rstest]
#[case(true)]
#[case(false)]
fn identical_saved_logs_keep_distinct_attachment_contexts(#[case] same_recording: bool) {
    let first = fixtures::attachment_ref();
    let second = LogAttachmentRef {
        recording: if same_recording {
            first.recording.clone()
        } else {
            test_util::recording_ref_of_group("2026-01-02T14-02-11")
        },
        id: LogAttachmentId::new_random(),
    };
    let mut files = LoadedFiles::new();
    files.push(
        test_util::recording_named("walk.gtd", 55.0, 10),
        test_util::stored_in_history(&first.recording),
    );
    if !same_recording {
        files.push(
            test_util::recording_named("drive.gtd", 60.0, 10),
            test_util::stored_in_history(&second.recording),
        );
    }
    let mut logs = LoadedLogs::default();
    let contexts = [
        (first.clone(), "entry 1", 0),
        (second.clone(), "entry 2", 1),
    ];
    let mut ids = Vec::new();
    for (attachment, pattern, slot) in &contexts {
        let stored = vec![StoredLogFilter {
            text: (*pattern).to_owned(),
            regex: false,
            enabled: true,
            mode: StoredLogFilterMode::Layer { color_slot: *slot },
        }];
        let mut log = test_util::log_of(10);
        log.restore_attachment(attachment.clone(), stored.clone(), &files.view());
        let outcome = logs.push(log);
        assert!(matches!(outcome, LogPushOutcome::NewlyLoaded(_)));
        let id = outcome.id();
        ids.push(id);
        let log = logs.get_by_id(id).expect("saved log is loaded");
        assert_eq!(log.attachment(), Some(attachment));
        assert_eq!(
            log.anchor_key().as_ref(),
            Some(&RecordingKey::Stored(attachment.recording.clone()))
        );
        assert_eq!(log.filters().to_stored_filters(), stored);
        assert!(logs.any_loaded_log_holds(attachment));
        let mut duplicate = test_util::log_of(10);
        duplicate.restore_attachment(attachment.clone(), Vec::new(), &files.view());
        assert_eq!(logs.push(duplicate), LogPushOutcome::AlreadyLoaded(id));
    }
    let first_id = *ids.first().expect("first log is loaded");
    let second_id = *ids.last().expect("second log is loaded");
    assert_ne!(first_id, second_id);
    assert_eq!(logs.len(), 2);
    let loose = logs.push(test_util::log_of(10));
    assert!(matches!(loose, LogPushOutcome::NewlyLoaded(_)));
    assert_eq!(
        logs.push(test_util::log_of(10)),
        LogPushOutcome::AlreadyLoaded(loose.id())
    );
    fixtures::wait_for_scans(&mut logs);
    assert!(logs.take_filter_stack_edits_to_store().is_empty());
    let layers = test_util::map_matches(&mut logs, &files).layers();
    assert_eq!(
        layers.iter().map(|layer| layer.log.id).collect::<Vec<_>>(),
        ids
    );
    assert_eq!(
        layers
            .iter()
            .map(|layer| layer
                .matches
                .first()
                .expect("layer has a match")
                .entry_index)
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(
        layers
            .iter()
            .map(|layer| layer
                .matches
                .first()
                .expect("layer has a match")
                .fix
                .track
                .fi)
            .collect::<Vec<_>>(),
        [FileIdx::new(0), FileIdx::new(usize::from(!same_recording))]
    );

    logs.forget_attachment(&first);
    assert_eq!(logs.id_of_attachment(&first), None);
    assert_eq!(logs.id_of_attachment(&second), Some(second_id));
    logs.save_attachment(first_id, first.clone(), Vec::new(), &files.view());
    assert_eq!(logs.id_of_attachment(&first), Some(first_id));
    logs.remove_by_id(first_id);
    assert_eq!(
        test_util::map_matches(&mut logs, &files)
            .layers()
            .iter()
            .map(|layer| layer.log.id)
            .collect::<Vec<_>>(),
        [second_id]
    );
}

#[test]
fn saving_an_attachment_reuses_its_existing_context_and_preserves_both_filter_stacks() {
    let mut files = LoadedFiles::new();
    let attachment = fixtures::attachment_ref();
    files.push(
        test_util::recording_at(55.0, 10),
        test_util::stored_in_history(&attachment.recording),
    );
    let mut logs = LoadedLogs::default();
    let first = logs.push(test_util::log_of(10)).id();
    fixtures::add_layer_chip(&mut logs, first, "entry 1");
    assert_eq!(
        logs.save_attachment(first, attachment.clone(), Vec::new(), &files.view()),
        LogSaveOutcome::Saved(first)
    );
    let loose = logs.push(test_util::log_of(10)).id();
    fixtures::add_layer_chip(&mut logs, loose, "entry 2");
    let first_filters = logs
        .get_by_id(first)
        .expect("saved log")
        .filters()
        .to_stored_filters();
    let loose_filters = logs
        .get_by_id(loose)
        .expect("loose log")
        .filters()
        .to_stored_filters();
    assert_eq!(
        logs.save_attachment(loose, attachment.clone(), Vec::new(), &files.view()),
        LogSaveOutcome::AlreadyLoaded(first)
    );
    assert_eq!(logs.get_by_id(loose).expect("loose log").attachment(), None);
    assert_eq!(
        logs.get_by_id(loose)
            .expect("loose log")
            .filters()
            .to_stored_filters(),
        loose_filters
    );
    assert_eq!(
        logs.get_by_id(first)
            .expect("saved log")
            .filters()
            .to_stored_filters(),
        first_filters
    );
    assert_eq!(
        logs.restore_attachment(
            test_util::log_of(10),
            attachment.clone(),
            Vec::new(),
            &files.view()
        ),
        LogPushOutcome::AlreadyLoaded(first)
    );
    let other = LogAttachmentRef {
        id: LogAttachmentId::new_random(),
        ..attachment.clone()
    };
    assert_eq!(
        logs.save_attachment(first, other.clone(), Vec::new(), &files.view()),
        LogSaveOutcome::AlreadySaved
    );
    assert_eq!(logs.id_of_attachment(&other), None);
}

#[test]
fn removing_an_attachment_retains_its_context_independently_of_loose_and_restored_logs() {
    let mut files = LoadedFiles::new();
    let attachment = fixtures::attachment_ref();
    files.push(
        test_util::recording_at(55.0, 10),
        test_util::stored_in_history(&attachment.recording),
    );
    let mut logs = LoadedLogs::default();
    let first = logs
        .restore_attachment(
            test_util::log_of(10),
            attachment.clone(),
            Vec::new(),
            &files.view(),
        )
        .id();
    fixtures::add_layer_chip(&mut logs, first, "entry 1");
    let first_filters = logs
        .get_by_id(first)
        .expect("saved log")
        .filters()
        .to_stored_filters();
    let loose = logs.push(test_util::log_of(10)).id();
    let placement = logs.get_by_id(first).expect("saved log").entry_placement(1);
    logs.forget_attachment(&attachment);
    let detached = logs.get_by_id(first).expect("detached log");
    assert_eq!(
        detached.context_origin(),
        LogContextOrigin::DetachedAttachment
    );
    assert_eq!(
        detached.anchor_key(),
        Some(RecordingKey::Stored(attachment.recording.clone()))
    );
    assert_eq!(detached.entry_placement(1), placement);
    assert_eq!(detached.filters().to_stored_filters(), first_filters);
    assert_eq!(
        logs.push(test_util::log_of(10)),
        LogPushOutcome::AlreadyLoaded(loose)
    );
    let restored = logs
        .restore_attachment(
            test_util::log_of(10),
            attachment.clone(),
            Vec::new(),
            &files.view(),
        )
        .id();
    assert_ne!(restored, first);
    assert_ne!(restored, loose);
    assert_eq!(logs.len(), 3);
    assert_eq!(logs.id_of_attachment(&attachment), Some(restored));
    fixtures::wait_for_scans(&mut logs);
    assert_eq!(
        test_util::map_matches(&mut logs, &files)
            .layers()
            .iter()
            .map(|layer| layer.log.id)
            .collect::<Vec<_>>(),
        [first]
    );
    logs.get_mut_by_id(first)
        .expect("detached log")
        .anchor_to_loaded_recording(None, &files.view());
    assert_eq!(
        logs.get_by_id(first).expect("detached log").anchor_key(),
        None
    );
    assert_eq!(
        logs.get_by_id(restored).expect("saved log").anchor_key(),
        Some(RecordingKey::Stored(attachment.recording))
    );
}

#[rstest::rstest]
fn documents_are_shared_only_for_equal_resolved_parses(
    #[values(true, false)] same_reference: bool,
    #[values(true, false)] loose_first: bool,
) {
    let text = "Jan  1 14:02:11 entry 0\n";
    let saved_reference = test_util::start();
    let loose_reference = if same_reference {
        saved_reference
    } else {
        saved_reference + Duration::days(365)
    };
    let saved_parse = gt_logfile::parse_log(text.into(), saved_reference).expect("saved parse");
    let loose_parse = gt_logfile::parse_log(text.into(), loose_reference).expect("loose parse");
    let saved_timestamp = saved_parse.entries().first().expect("entry").timestamp;
    let loose_timestamp = loose_parse.entries().first().expect("entry").timestamp;
    assert_eq!(saved_timestamp.year(), 2026);
    assert_eq!(
        loose_timestamp.year(),
        if same_reference { 2026 } else { 2027 }
    );
    let saved_log = LoadedLog::new(
        Some("saved.log".to_owned()),
        saved_parse,
        test_util::association_window(),
    );
    let loose_log = LoadedLog::new(
        Some("loose.log".to_owned()),
        loose_parse,
        test_util::association_window(),
    );
    let mut logs = LoadedLogs::default();
    let files = test_util::loaded(Vec::new());
    let attachment = fixtures::attachment_ref();
    let (saved, loose) = if loose_first {
        let loose = logs.push(loose_log).id();
        let saved = logs
            .restore_attachment(saved_log, attachment, Vec::new(), &files.view())
            .id();
        (saved, loose)
    } else {
        let saved = logs
            .restore_attachment(saved_log, attachment, Vec::new(), &files.view())
            .id();
        let loose = logs.push(loose_log).id();
        (saved, loose)
    };
    let saved = logs.get_by_id(saved).expect("saved context");
    let loose = logs.get_by_id(loose).expect("loose context");
    assert_eq!(saved.content_hash(), loose.content_hash());
    assert_eq!(
        Arc::ptr_eq(&saved.document, &loose.document),
        same_reference
    );
    assert_eq!(saved.parsed().year_reference(), saved_reference);
    assert_eq!(loose.parsed().year_reference(), loose_reference);
    assert_eq!(
        saved.parsed().entries().first().expect("entry").timestamp,
        saved_timestamp
    );
    assert_eq!(
        loose.parsed().entries().first().expect("entry").timestamp,
        loose_timestamp
    );
}
