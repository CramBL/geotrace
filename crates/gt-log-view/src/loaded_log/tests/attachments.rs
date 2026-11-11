use std::sync::Arc;

use chrono::{Datelike as _, Duration};
use gt_history_types::{
    LogAttachmentId, StoredLogFilter, StoredLogFilterCondition, StoredLogFilterEffects,
    StoredLogFilterStack, StoredLogSettings,
};
use gt_loaded_files::{FileHistory, LoadedFiles};
use gt_logfile::ParsedLog;
use gt_pending_writes::WriteAccess;
use gt_types::FileIdx;

use crate::anchor::RecordingKey;
use crate::attachment::LogAttachmentRef;
use crate::loaded_log::tests::fixtures;
use crate::loaded_log::{
    LoadedLog, LoadedLogs, LogContext, LogContextOrigin, LogPushOutcome, LogSaveOutcome,
    PositionSourceState,
};
use crate::{AttachmentSettingsStatus, FilterEffect, FilterGroupOperator, test_util};

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

    log.record_attachment(attachment.clone(), fixtures::settings(), &files.view());

    let key = RecordingKey::Stored(attachment.recording.clone());
    let placement = log.entry_placement(0);
    let associated = log.associated_recording();
    assert_eq!(
        log.position_source_state(),
        PositionSourceState::Recording(key.clone())
    );
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
    log.restore_attachment(attachment.clone(), fixtures::settings(), &files.view());
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

#[test]
fn filter_edits_during_a_pending_write_remain_dirty_after_its_acknowledgement() {
    let attachment = fixtures::attachment_ref();
    let recordings = test_util::loaded(Vec::new());
    let mut logs = LoadedLogs::default();
    let mut log = test_util::log_of(10);
    log.restore_attachment(attachment.clone(), fixtures::settings(), &recordings.view());
    let id = logs.push(log).id();
    fixtures::add_layer_chip(&mut logs, id, "entry 1");
    let first = logs
        .prepare_settings_writes(WriteAccess::Owner)
        .pop()
        .unwrap();
    assert_eq!(first.attachment(), &attachment);
    assert_eq!(
        first.settings(),
        &logs.get_by_id(id).unwrap().settings_snapshot()
    );
    fixtures::add_layer_chip(&mut logs, id, "entry 2");
    assert!(logs.prepare_settings_writes(WriteAccess::Owner).is_empty());
    assert!(logs.acknowledge_settings(first.clone().complete(Ok(()))));
    assert_eq!(
        logs.get_by_id(id)
            .unwrap()
            .settings_save_status(WriteAccess::Owner),
        AttachmentSettingsStatus::Dirty
    );
    let latest = logs
        .prepare_settings_writes(WriteAccess::Owner)
        .pop()
        .unwrap();
    assert_eq!(
        latest.settings(),
        &logs.get_by_id(id).unwrap().settings_snapshot()
    );
    assert!(!logs.acknowledge_settings(first.complete(Ok(()))));
    assert!(logs.acknowledge_settings(latest.complete(Ok(()))));
    assert_eq!(
        logs.get_by_id(id)
            .unwrap()
            .settings_save_status(WriteAccess::Owner),
        AttachmentSettingsStatus::Saved
    );
    logs.forget_attachment(&attachment);
    assert_eq!(logs.len(), 1);
    assert_eq!(logs.get_by_id(id).and_then(LoadedLog::attachment), None);
    assert!(logs.prepare_settings_writes(WriteAccess::Owner).is_empty());
}

#[test]
fn restored_legacy_references_remain_missing_until_settings_change() {
    let files = test_util::loaded(Vec::new());
    let mut log = test_util::log_of(10);
    let persisted = StoredLogSettings {
        year_reference: None,
        ..fixtures::settings()
    };
    log.restore_attachment(fixtures::attachment_ref(), persisted.clone(), &files.view());
    assert!(log.prepare_settings_write(WriteAccess::Owner).is_none());
    let LogContext::SavedAttachment(state) = &log.context else {
        panic!("saved context");
    };
    assert_eq!(state.acknowledged(), &persisted);
    let mut logs = LoadedLogs::default();
    let id = logs.push(log).id();
    fixtures::add_layer_chip(&mut logs, id, "entry 1");
    let write = logs
        .prepare_settings_writes(WriteAccess::Owner)
        .pop()
        .unwrap();
    assert_eq!(write.settings().year_reference, Some(test_util::start()));
}

#[rstest::rstest]
#[case::detach(false)]
#[case::unload(true)]
fn settings_completions_are_ignored_after_attachment_context_replacement(#[case] unload: bool) {
    let files = test_util::loaded(Vec::new());
    let attachment = fixtures::attachment_ref();
    let mut logs = LoadedLogs::default();
    let id = logs
        .restore_attachment(
            test_util::log_of(10),
            attachment.clone(),
            fixtures::settings(),
            &files.view(),
        )
        .id();
    fixtures::add_layer_chip(&mut logs, id, "entry 1");
    let stale = logs
        .prepare_settings_writes(WriteAccess::Owner)
        .pop()
        .unwrap();
    if unload {
        logs.remove_by_id(id);
    } else {
        logs.forget_attachment(&attachment);
    }
    let replacement = logs
        .restore_attachment(
            test_util::log_of(10),
            attachment,
            fixtures::settings(),
            &files.view(),
        )
        .id();
    fixtures::add_layer_chip(&mut logs, replacement, "entry 2");
    let current = logs
        .prepare_settings_writes(WriteAccess::Owner)
        .pop()
        .unwrap();
    assert!(!logs.acknowledge_settings(stale.complete(Ok(()))));
    assert_eq!(
        logs.get_by_id(replacement)
            .unwrap()
            .settings_save_status(WriteAccess::Owner),
        AttachmentSettingsStatus::Saving
    );
    assert!(logs.acknowledge_settings(current.complete(Ok(()))));
}

#[test]
fn independent_effect_edits_are_persisted_and_restored_once() {
    let attachment = fixtures::attachment_ref();
    let recordings = test_util::loaded(Vec::new());
    let mut log = test_util::log_of(10);
    log.record_attachment(attachment.clone(), fixtures::settings(), &recordings.view());
    let mut logs = LoadedLogs::default();
    let id = logs.push(log).id();
    fixtures::add_layer_chip(&mut logs, id, "entry 1");
    let initial = logs
        .prepare_settings_writes(WriteAccess::Owner)
        .pop()
        .unwrap();
    assert!(logs.acknowledge_settings(initial.complete(Ok(()))));
    let (stack, slots) = logs.filter_stack_mut_by_id(id).unwrap();
    let chip = stack.chips().first().unwrap().id();
    stack.add_chip_effect(chip, FilterEffect::Table, slots);
    stack.set_chip_effect_enabled(chip, FilterEffect::Map, false);
    let expected = stack.to_stored_stack();
    let edits = logs.prepare_settings_writes(WriteAccess::Owner);
    assert_eq!(
        edits
            .first()
            .map(|write| (write.attachment(), &write.settings().filters)),
        Some((&attachment, &expected))
    );
    for write in edits {
        assert!(logs.acknowledge_settings(write.complete(Ok(()))));
    }
    assert!(logs.prepare_settings_writes(WriteAccess::Owner).is_empty());
    let mut restored = test_util::log_of(10);
    restored.restore_attachment(
        attachment,
        StoredLogSettings {
            filters: expected.clone(),
            ..fixtures::settings()
        },
        &recordings.view(),
    );
    assert_eq!(restored.filters().to_stored_stack(), expected);
    assert!(
        restored
            .prepare_settings_write(WriteAccess::Owner)
            .is_none()
    );
}

#[test]
fn group_only_edits_are_stored_once_and_restored_with_an_empty_stack() {
    let attachment = fixtures::attachment_ref();
    let recordings = test_util::loaded(Vec::new());
    let mut logs = LoadedLogs::default();
    let mut log = test_util::log_of(10);
    log.restore_attachment(attachment.clone(), fixtures::settings(), &recordings.view());
    let id = logs.push(log).id();
    let stack = logs.filter_stack_mut_by_id(id).unwrap().0;
    let selected = stack.create_group();
    stack.select_group(selected);
    stack.set_group_operator(stack.selected_group(), FilterGroupOperator::Any);
    stack.create_group();
    stack.select_group(selected);
    let expected = stack.to_stored_stack();
    let edits = logs.prepare_settings_writes(WriteAccess::Owner);
    assert_eq!(edits.len(), 1);
    let write = edits.into_iter().next().unwrap();
    let reference = write.attachment().clone();
    let stored = write.settings().clone();
    assert_eq!(reference, attachment);
    assert_eq!(stored.filters, expected);
    assert_eq!(stored.filters.groups().len(), 3);
    assert!(stored.filters.chips().is_empty());
    assert!(logs.acknowledge_settings(write.complete(Ok(()))));
    assert!(logs.prepare_settings_writes(WriteAccess::Owner).is_empty());
    let mut restored = test_util::log_of(10);
    restored.restore_attachment(reference, stored, &recordings.view());
    assert_eq!(restored.filters().to_stored_stack(), expected);
    assert!(
        restored
            .prepare_settings_write(WriteAccess::Owner)
            .is_none()
    );
}

/// A log restored from an attachment comes back with its chips, drawing in
/// the colours it was stored with.
#[test]
fn a_restored_attachment_puts_back_the_stack_it_was_stored_with() {
    let stored = StoredLogFilterStack::single_all_group(vec![
        StoredLogFilter {
            group_id: 0,
            condition: StoredLogFilterCondition::Message {
                text: "entry 1".to_owned(),
                regex: false,
            },
            effects: StoredLogFilterEffects::Map {
                enabled: true,
                color_slot: 2,
            },
        },
        StoredLogFilter {
            group_id: 0,
            condition: StoredLogFilterCondition::Message {
                text: "entry".to_owned(),
                regex: false,
            },
            effects: StoredLogFilterEffects::Table { enabled: false },
        },
    ]);
    let attachment = fixtures::attachment_ref();
    let recordings = test_util::loaded(Vec::new());
    let mut logs = LoadedLogs::default();
    let mut log = test_util::log_of(10);
    log.restore_attachment(
        attachment.clone(),
        StoredLogSettings {
            filters: stored.clone(),
            ..fixtures::settings()
        },
        &recordings.view(),
    );
    let id = logs.push(log).id();
    fixtures::wait_for_scans(&mut logs);

    let log = logs.get_by_id(id).expect("the restored log is loaded");
    assert_eq!(log.attachment(), Some(&attachment));
    assert_eq!(
        log.filters().to_stored_stack(),
        stored,
        "the restored stack is the stored one, colours and all"
    );
    assert!(
        logs.prepare_settings_writes(WriteAccess::Owner).is_empty(),
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
        let stored = StoredLogFilterStack::single_all_group(vec![StoredLogFilter {
            group_id: 0,
            condition: StoredLogFilterCondition::Message {
                text: (*pattern).to_owned(),
                regex: false,
            },
            effects: StoredLogFilterEffects::Map {
                enabled: true,
                color_slot: *slot,
            },
        }]);
        let mut log = test_util::log_of(10);
        log.restore_attachment(
            attachment.clone(),
            StoredLogSettings {
                filters: stored.clone(),
                ..fixtures::settings()
            },
            &files.view(),
        );
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
        assert_eq!(log.filters().to_stored_stack(), stored);
        assert!(logs.any_loaded_log_holds(attachment));
        let mut duplicate = test_util::log_of(10);
        duplicate.restore_attachment(attachment.clone(), fixtures::settings(), &files.view());
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
    assert!(logs.prepare_settings_writes(WriteAccess::Owner).is_empty());
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
    logs.save_attachment(first_id, first.clone(), fixtures::settings(), &files.view());
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
        logs.save_attachment(
            first,
            attachment.clone(),
            fixtures::settings(),
            &files.view()
        ),
        LogSaveOutcome::Saved(first)
    );
    let loose = logs.push(test_util::log_of(10)).id();
    fixtures::add_layer_chip(&mut logs, loose, "entry 2");
    let first_filters = logs
        .get_by_id(first)
        .expect("saved log")
        .filters()
        .to_stored_stack();
    let loose_filters = logs
        .get_by_id(loose)
        .expect("loose log")
        .filters()
        .to_stored_stack();
    assert_eq!(
        logs.save_attachment(
            loose,
            attachment.clone(),
            fixtures::settings(),
            &files.view()
        ),
        LogSaveOutcome::AlreadyLoaded(first)
    );
    assert_eq!(logs.get_by_id(loose).expect("loose log").attachment(), None);
    assert_eq!(
        logs.get_by_id(loose)
            .expect("loose log")
            .filters()
            .to_stored_stack(),
        loose_filters
    );
    assert_eq!(
        logs.get_by_id(first)
            .expect("saved log")
            .filters()
            .to_stored_stack(),
        first_filters
    );
    assert_eq!(
        logs.restore_attachment(
            test_util::log_of(10),
            attachment.clone(),
            fixtures::settings(),
            &files.view()
        ),
        LogPushOutcome::AlreadyLoaded(first)
    );
    let other = LogAttachmentRef {
        id: LogAttachmentId::new_random(),
        ..attachment.clone()
    };
    assert_eq!(
        logs.save_attachment(first, other.clone(), fixtures::settings(), &files.view()),
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
            fixtures::settings(),
            &files.view(),
        )
        .id();
    fixtures::add_layer_chip(&mut logs, first, "entry 1");
    let first_filters = logs
        .get_by_id(first)
        .expect("saved log")
        .filters()
        .to_stored_stack();
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
    assert_eq!(detached.filters().to_stored_stack(), first_filters);
    assert_eq!(
        logs.push(test_util::log_of(10)),
        LogPushOutcome::AlreadyLoaded(loose)
    );
    let restored = logs
        .restore_attachment(
            test_util::log_of(10),
            attachment.clone(),
            fixtures::settings(),
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
    let first_timestamp = |parsed: &ParsedLog| {
        parsed
            .chronology()
            .resolved()
            .and_then(|resolved| resolved.timestamp(0))
            .expect("the fixture entry resolves")
    };
    let saved_timestamp = first_timestamp(&saved_parse);
    let loose_timestamp = first_timestamp(&loose_parse);
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
            .restore_attachment(saved_log, attachment, fixtures::settings(), &files.view())
            .id();
        (saved, loose)
    } else {
        let saved = logs
            .restore_attachment(saved_log, attachment, fixtures::settings(), &files.view())
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
    assert_eq!(first_timestamp(saved.parsed()), saved_timestamp);
    assert_eq!(first_timestamp(loose.parsed()), loose_timestamp);
}
