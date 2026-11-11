use gt_history_types::LogAttachmentId;
use gt_loaded_files::{FileHistory, LoadedFiles};
use gt_logfile::YearResolution;
use gt_pending_writes::WriteAccess;
use gt_types::AssociationWindow;

use crate::loaded_log::tests::fixtures;
use crate::{
    AttachmentSettingsStatus, InitialLogSaveOutcome, InitialLogSaveStatus, LoadedLogs,
    LogCapabilities, LogOperationError, LogSaveOutcome, PositionSourceSelection, RecordingKey,
    test_util,
};

#[rstest::rstest]
#[case::read_only(WriteAccess::ReadOnly, false, false, LogOperationError::ReadOnly)]
#[case::saved(WriteAccess::Owner, true, true, LogOperationError::AlreadySaved)]
#[case::unstored(WriteAccess::Owner, false, false, LogOperationError::TargetNotStored)]
fn save_preparation_requires_a_writable_unsaved_log_and_stored_recording(
    #[case] access: WriteAccess,
    #[case] saved: bool,
    #[case] stored: bool,
    #[case] error: LogOperationError,
) {
    let attachment = fixtures::attachment_ref();
    let mut files = LoadedFiles::new();
    files.push(
        test_util::recording_at(55.0, 10),
        if stored {
            test_util::stored_in_history(&test_util::recording_ref_of_group("2026-01-02T14-02-11"))
        } else {
            FileHistory::None
        },
    );
    let target = test_util::id_of(&files, 0);
    let mut logs = LoadedLogs::default();
    let id = if saved {
        logs.restore_attachment(
            test_util::log_of(10),
            attachment.clone(),
            fixtures::settings(),
            &files.view(),
        )
        .id()
    } else {
        logs.push(test_util::log_of(10)).id()
    };
    assert_eq!(
        logs.prepare_save(id, target, &files.view(), access)
            .unwrap_err(),
        error
    );
    assert_eq!(
        logs.get_by_id(id).unwrap().capabilities(access),
        LogCapabilities {
            select_position_source: !saved,
            save_with_recording: !saved && access.allows_writing(),
        }
    );
    if saved {
        assert_eq!(
            logs.select_position_source(
                id,
                PositionSourceSelection::Recording(target),
                &files.view()
            ),
            Err(LogOperationError::AlreadySaved)
        );
        assert_eq!(logs.get_by_id(id).unwrap().attachment(), Some(&attachment));
    } else {
        assert_eq!(
            logs.select_position_source(
                id,
                PositionSourceSelection::Recording(target),
                &files.view()
            ),
            Ok(())
        );
    }
}

#[test]
fn pending_save_rejects_other_attachment_and_position_source_operations() {
    let attachment = fixtures::attachment_ref();
    let mut files = LoadedFiles::new();
    files.push(
        test_util::recording_at(55.0, 10),
        test_util::stored_in_history(&attachment.recording),
    );
    let target = test_util::id_of(&files, 0);
    let mut logs = LoadedLogs::default();
    let id = logs.push(test_util::log_with_unresolved_years()).id();
    let request = logs
        .prepare_save(id, target, &files.view(), WriteAccess::Owner)
        .unwrap();
    assert!(logs.pending_initial_position_sources().is_empty());
    assert_eq!(request.log(), id);
    assert_eq!(request.recording(), &attachment.recording);
    assert_eq!(
        logs.get_by_id(id).unwrap().initial_save_recording(),
        Some(&attachment.recording)
    );
    assert_eq!(request.name(), logs.get_by_id(id).unwrap().name());
    assert_eq!(request.text(), logs.get_by_id(id).unwrap().parsed().text());
    assert_eq!(
        logs.get_by_id(id).unwrap().initial_save_status(),
        InitialLogSaveStatus::Saving
    );
    assert_eq!(
        logs.get_by_id(id).unwrap().capabilities(WriteAccess::Owner),
        LogCapabilities {
            select_position_source: false,
            save_with_recording: false
        }
    );
    assert_eq!(
        logs.prepare_save(id, target, &files.view(), WriteAccess::Owner)
            .unwrap_err(),
        LogOperationError::SavePending
    );
    assert_eq!(
        logs.select_position_source(id, PositionSourceSelection::NoSource, &files.view()),
        Err(LogOperationError::SavePending)
    );
    logs.get_mut_by_id(id)
        .unwrap()
        .anchor_to_loaded_recording(Some(target), &files.view());
    assert_eq!(logs.get_by_id(id).unwrap().anchor_key(), None);
    assert_eq!(
        logs.save_attachment(id, attachment.clone(), fixtures::settings(), &files.view()),
        LogSaveOutcome::AlreadySaved
    );
    assert_eq!(
        logs.get_by_id(id).unwrap().initial_save_status(),
        InitialLogSaveStatus::Saving
    );
}

#[test]
fn initial_save_preserves_new_settings_until_their_settings_acknowledgement() {
    let attachment = fixtures::attachment_ref();
    let mut files = LoadedFiles::new();
    files.push(
        test_util::recording_at(55.0, 10),
        test_util::stored_in_history(&attachment.recording),
    );
    let target = test_util::id_of(&files, 0);
    let mut logs = LoadedLogs::default();
    let id = logs.push(test_util::log_with_unresolved_years()).id();
    let request = logs
        .prepare_save(id, target, &files.view(), WriteAccess::Owner)
        .unwrap();
    let initial = request.settings().clone();
    assert_eq!(
        initial.association_window,
        Some(test_util::association_window())
    );
    let new_window = AssociationWindow::try_from_seconds(3).unwrap();
    logs.get_mut_by_id(id)
        .unwrap()
        .set_association_window(new_window, &files.view());
    assert!(
        logs.get_mut_by_id(id)
            .unwrap()
            .resolve_years_as(YearResolution::PlaceLastEntryInYear(2026), &files.view())
    );
    fixtures::add_layer_chip(&mut logs, id, "navsyncd");
    let desired = logs.get_by_id(id).unwrap().settings_snapshot();
    assert_eq!(desired.association_window, Some(new_window));
    let completion = request.complete(Ok(fixtures::save_receipt(attachment.id, initial)));
    assert_eq!(
        logs.acknowledge_initial_save(completion.clone(), &files.view()),
        Some(InitialLogSaveOutcome::Saved(id))
    );
    let log = logs.get_by_id(id).unwrap();
    assert_eq!(log.settings_snapshot(), desired);
    assert_eq!(log.attachment(), Some(&attachment));
    assert_eq!(
        log.anchor_key(),
        Some(RecordingKey::Stored(attachment.recording.clone()))
    );
    assert_eq!(
        log.settings_save_status(WriteAccess::Owner),
        AttachmentSettingsStatus::Dirty
    );
    assert_eq!(
        logs.acknowledge_initial_save(completion, &files.view()),
        None
    );
    let write = logs
        .prepare_settings_writes(WriteAccess::Owner)
        .pop()
        .unwrap();
    assert_eq!(write.settings(), &desired);
    assert!(logs.acknowledge_settings(write.complete(Ok(()))));
    assert_eq!(
        logs.get_by_id(id)
            .unwrap()
            .settings_save_status(WriteAccess::Owner),
        AttachmentSettingsStatus::Saved
    );
    let mut restored = LoadedLogs::default();
    let mut log = test_util::log_with_unresolved_years();
    assert!(log.resolve_years_as(YearResolution::PlaceLastEntryInYear(2026), &files.view()));
    let restored_id = restored
        .restore_attachment(log, attachment, desired.clone(), &files.view())
        .id();
    assert_eq!(
        restored.get_by_id(restored_id).unwrap().settings_snapshot(),
        desired
    );
}

#[test]
fn failed_save_retries_the_original_destination_with_current_settings() {
    let first = fixtures::attachment_ref();
    let second = test_util::recording_ref_of_group("2026-01-02T14-02-11");
    let mut files = LoadedFiles::new();
    for recording in [&first.recording, &second] {
        files.push(
            test_util::recording_at(55.0, 10),
            test_util::stored_in_history(recording),
        );
    }
    let target = test_util::id_of(&files, 0);
    let other = test_util::id_of(&files, 1);
    let mut logs = LoadedLogs::default();
    let id = logs.push(test_util::log_of(10)).id();
    let request = logs
        .prepare_save(id, target, &files.view(), WriteAccess::Owner)
        .unwrap();
    let failed = request.complete(Err("Disk full".to_owned()));
    assert_eq!(
        logs.acknowledge_initial_save(failed.clone(), &files.view()),
        Some(InitialLogSaveOutcome::Failed)
    );
    assert_eq!(
        logs.get_by_id(id).unwrap().initial_save_status(),
        InitialLogSaveStatus::Failed("Disk full")
    );
    logs.select_position_source(id, PositionSourceSelection::Recording(other), &files.view())
        .unwrap();
    fixtures::add_layer_chip(&mut logs, id, "entry 2");
    assert_eq!(
        logs.retry_initial_save(id, &files.view(), WriteAccess::ReadOnly)
            .unwrap_err(),
        LogOperationError::ReadOnly
    );
    assert_eq!(
        logs.get_by_id(id).unwrap().initial_save_status(),
        InitialLogSaveStatus::Failed("Disk full")
    );
    let retry = logs
        .retry_initial_save(id, &files.view(), WriteAccess::Owner)
        .unwrap();
    let desired = logs.get_by_id(id).unwrap().settings_snapshot();
    assert_eq!(retry.recording(), &first.recording);
    assert_eq!(retry.settings(), &desired);
    assert_eq!(logs.acknowledge_initial_save(failed, &files.view()), None);
    assert_eq!(
        logs.get_by_id(id).unwrap().initial_save_status(),
        InitialLogSaveStatus::Saving
    );
    let completion = retry.complete(Ok(fixtures::save_receipt(first.id, desired)));
    assert_eq!(
        logs.acknowledge_initial_save(completion, &files.view()),
        Some(InitialLogSaveOutcome::Saved(id))
    );
    assert_eq!(logs.get_by_id(id).unwrap().attachment(), Some(&first));
}

#[test]
fn initial_save_completions_cannot_bind_an_unloaded_or_reinserted_context() {
    let attachment = fixtures::attachment_ref();
    let mut files = LoadedFiles::new();
    files.push(
        test_util::recording_at(55.0, 10),
        test_util::stored_in_history(&attachment.recording),
    );
    let target = test_util::id_of(&files, 0);
    let mut logs = LoadedLogs::default();
    let id = logs.push(test_util::log_of(10)).id();
    let request = logs
        .prepare_save(id, target, &files.view(), WriteAccess::Owner)
        .unwrap();
    let settings = request.settings().clone();
    let completion = request.complete(Ok(fixtures::save_receipt(attachment.id, settings)));
    let removed = logs.remove_by_id(id).unwrap();
    assert_eq!(
        logs.acknowledge_initial_save(completion.clone(), &files.view()),
        None
    );
    let mut replacement = LoadedLogs::default();
    let replacement_id = replacement.push(removed).id();
    assert_eq!(replacement_id, id);
    assert_eq!(
        replacement.acknowledge_initial_save(completion, &files.view()),
        None
    );
    assert_eq!(replacement.get_by_id(id).unwrap().attachment(), None);
    assert_eq!(
        replacement.get_by_id(id).unwrap().initial_save_status(),
        InitialLogSaveStatus::Idle
    );
}

#[test]
fn detached_attachment_can_save_to_another_recording() {
    let attachment = fixtures::attachment_ref();
    let other = test_util::recording_ref_of_group("2026-01-02T14-02-11");
    let mut files = LoadedFiles::new();
    files.push(
        test_util::recording_at(55.0, 10),
        test_util::stored_in_history(&other),
    );
    let target = test_util::id_of(&files, 0);
    let mut logs = LoadedLogs::default();
    let id = logs
        .restore_attachment(
            test_util::log_of(10),
            attachment.clone(),
            fixtures::settings(),
            &files.view(),
        )
        .id();
    logs.forget_attachment(&attachment);
    let request = logs
        .prepare_save(id, target, &files.view(), WriteAccess::Owner)
        .unwrap();
    let receipt = fixtures::save_receipt(LogAttachmentId::new_random(), request.settings().clone());
    assert_eq!(
        logs.acknowledge_initial_save(request.complete(Ok(receipt)), &files.view()),
        Some(InitialLogSaveOutcome::Saved(id))
    );
    assert_eq!(
        logs.get_by_id(id).unwrap().attachment().unwrap().recording,
        other
    );
}

#[test]
fn unloaded_destinations_are_rejected_without_losing_failed_save_settings() {
    let attachment = fixtures::attachment_ref();
    let mut files = LoadedFiles::new();
    files.push(
        test_util::recording_at(55.0, 10),
        test_util::stored_in_history(&attachment.recording),
    );
    let target = test_util::id_of(&files, 0);
    let mut logs = LoadedLogs::default();
    let id = logs.push(test_util::log_of(10)).id();
    assert_eq!(
        logs.retry_initial_save(id, &files.view(), WriteAccess::Owner)
            .unwrap_err(),
        LogOperationError::NoFailedSave
    );
    let request = logs
        .prepare_save(id, target, &files.view(), WriteAccess::Owner)
        .unwrap();
    logs.acknowledge_initial_save(request.complete(Err("Disk full".to_owned())), &files.view());
    files.remove_file(0);
    assert_eq!(
        logs.retry_initial_save(id, &files.view(), WriteAccess::Owner)
            .unwrap_err(),
        LogOperationError::TargetUnloaded
    );
    assert_eq!(
        logs.prepare_save(id, target, &files.view(), WriteAccess::Owner)
            .unwrap_err(),
        LogOperationError::TargetUnloaded
    );
    assert_eq!(
        logs.select_position_source(
            id,
            PositionSourceSelection::Recording(target),
            &files.view()
        ),
        Err(LogOperationError::TargetUnloaded)
    );
    assert_eq!(
        logs.get_by_id(id).unwrap().initial_save_status(),
        InitialLogSaveStatus::Failed("The selected recording has been unloaded")
    );
    logs.remove_by_id(id);
    assert_eq!(
        logs.prepare_save(id, target, &files.view(), WriteAccess::Owner)
            .unwrap_err(),
        LogOperationError::LogUnloaded
    );
    assert_eq!(
        logs.retry_initial_save(id, &files.view(), WriteAccess::Owner)
            .unwrap_err(),
        LogOperationError::LogUnloaded
    );
    assert_eq!(
        logs.select_position_source(id, PositionSourceSelection::NoSource, &files.view()),
        Err(LogOperationError::LogUnloaded)
    );
}

#[test]
fn reuse_of_a_loaded_attachment_preserves_both_contexts_and_its_pending_revision() {
    let attachment = fixtures::attachment_ref();
    let mut files = LoadedFiles::new();
    files.push(
        test_util::recording_at(55.0, 10),
        test_util::stored_in_history(&attachment.recording),
    );
    let target = test_util::id_of(&files, 0);
    let mut logs = LoadedLogs::default();
    let saved = logs
        .restore_attachment(
            test_util::log_of(10),
            attachment.clone(),
            fixtures::settings(),
            &files.view(),
        )
        .id();
    fixtures::add_layer_chip(&mut logs, saved, "entry 1");
    let pending = logs
        .prepare_settings_writes(WriteAccess::Owner)
        .pop()
        .unwrap();
    let loose = logs.push(test_util::log_of(10)).id();
    fixtures::add_layer_chip(&mut logs, loose, "entry 2");
    let desired = logs.get_by_id(loose).unwrap().settings_snapshot();
    let request = logs
        .prepare_save(loose, target, &files.view(), WriteAccess::Owner)
        .unwrap();
    assert_eq!(
        logs.acknowledge_initial_save(
            request.complete(Ok(fixtures::save_receipt(
                attachment.id,
                fixtures::settings()
            ))),
            &files.view()
        ),
        Some(InitialLogSaveOutcome::Failed)
    );
    assert_eq!(logs.get_by_id(loose).unwrap().settings_snapshot(), desired);
    assert_eq!(logs.get_by_id(loose).unwrap().attachment(), None);
    assert_eq!(
        logs.get_by_id(saved).unwrap().settings_snapshot(),
        *pending.settings()
    );
    assert_eq!(
        logs.get_by_id(saved)
            .unwrap()
            .settings_save_status(WriteAccess::Owner),
        AttachmentSettingsStatus::Saving
    );
    assert!(logs.acknowledge_settings(pending.complete(Ok(()))));
}

#[test]
fn equal_settings_reuse_clears_initial_saving_and_preserves_the_owned_pending_revision() {
    let attachment = fixtures::attachment_ref();
    let mut files = LoadedFiles::new();
    files.push(
        test_util::recording_at(55.0, 10),
        test_util::stored_in_history(&attachment.recording),
    );
    let target = test_util::id_of(&files, 0);
    let mut logs = LoadedLogs::default();
    let saved = logs
        .restore_attachment(
            test_util::log_of(10),
            attachment.clone(),
            fixtures::settings(),
            &files.view(),
        )
        .id();
    fixtures::add_layer_chip(&mut logs, saved, "entry 1");
    let pending = logs
        .prepare_settings_writes(WriteAccess::Owner)
        .pop()
        .unwrap();
    let (stack, slots) = logs.filter_stack_mut_by_id(saved).unwrap();
    let chip = stack.chips().first().unwrap().id();
    stack.remove_chip(chip, slots);
    let loose = logs.push(test_util::log_of(10)).id();
    let request = logs
        .prepare_save(loose, target, &files.view(), WriteAccess::Owner)
        .unwrap();
    assert_eq!(
        logs.acknowledge_initial_save(
            request.complete(Ok(fixtures::save_receipt(
                attachment.id,
                fixtures::settings()
            ))),
            &files.view()
        ),
        Some(InitialLogSaveOutcome::AlreadyLoaded(saved))
    );
    assert_eq!(
        logs.get_by_id(loose).unwrap().initial_save_status(),
        InitialLogSaveStatus::Idle
    );
    assert_eq!(
        logs.get_by_id(saved)
            .unwrap()
            .settings_save_status(WriteAccess::Owner),
        AttachmentSettingsStatus::Saving
    );
    assert!(logs.acknowledge_settings(pending.complete(Ok(()))));
    let latest = logs
        .prepare_settings_writes(WriteAccess::Owner)
        .pop()
        .unwrap();
    assert!(latest.settings().filters.chips().is_empty());
    assert!(logs.acknowledge_settings(latest.complete(Ok(()))));
}
