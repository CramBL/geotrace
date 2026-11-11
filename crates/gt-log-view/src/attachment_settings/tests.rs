use chrono::DateTime;
use gt_history_types::{
    LogAttachmentId, StoredLogFilter, StoredLogFilterCondition, StoredLogFilterEffects,
    StoredLogFilterStack, StoredLogSettings, StoredYearResolution,
};
use gt_pending_writes::WriteAccess;
use rstest::rstest;

use crate::test_util;
use crate::{
    AttachmentSettingsState, AttachmentSettingsStatus, AttachmentSettingsWrite, LogAttachmentRef,
};

fn attachment() -> LogAttachmentRef {
    LogAttachmentRef {
        recording: test_util::stored_recording_ref(),
        id: LogAttachmentId::new_random(),
    }
}

fn settings(marker: i8) -> StoredLogSettings {
    StoredLogSettings {
        filters: StoredLogFilterStack::single_all_group(vec![StoredLogFilter {
            group_id: 0,
            condition: StoredLogFilterCondition::Message {
                text: marker.to_string(),
                regex: false,
            },
            effects: StoredLogFilterEffects::Table { enabled: true },
        }]),
        year_reference: DateTime::from_timestamp(i64::from(marker), 123_456_789),
        year_resolution: StoredYearResolution::PlaceLastEntryInYear {
            year: 2026 + i32::from(marker),
        },
    }
}

#[rstest]
#[case::later_edit(2)]
#[case::revert_to_acknowledged(0)]
fn edits_during_a_write_require_their_own_acknowledgement(#[case] latest: i8) {
    let reference = attachment();
    let mut state = AttachmentSettingsState::new(reference.clone(), settings(0));
    assert_eq!(state.attachment(), &reference);
    state.set_desired(settings(1));
    assert_eq!(
        state.status(WriteAccess::Owner),
        AttachmentSettingsStatus::Dirty
    );
    let first = state
        .prepare_write(WriteAccess::Owner)
        .expect("first write");
    assert_eq!(first.attachment(), &reference);
    assert_eq!(first.settings(), &settings(1));
    assert_eq!(state.acknowledged(), &settings(0));
    state.set_desired(settings(latest));
    assert_eq!(
        state.status(WriteAccess::Owner),
        AttachmentSettingsStatus::Saving
    );
    assert!(state.prepare_write(WriteAccess::Owner).is_none());

    let completion = first.complete(Ok(()));
    assert!(state.acknowledge(completion.clone()));
    assert_eq!(state.acknowledged(), &settings(1));
    assert_eq!(
        state.status(WriteAccess::Owner),
        AttachmentSettingsStatus::Dirty
    );
    let second = state
        .prepare_write(WriteAccess::Owner)
        .expect("latest write");
    assert_eq!(second.settings(), &settings(latest));
    assert!(!state.acknowledge(completion));
    assert_eq!(
        state.status(WriteAccess::Owner),
        AttachmentSettingsStatus::Saving
    );
    assert!(state.acknowledge(second.complete(Ok(()))));
    assert_eq!(state.acknowledged(), &settings(latest));
    assert_eq!(
        state.status(WriteAccess::Owner),
        AttachmentSettingsStatus::Saved
    );
}

#[rstest]
#[case::unchanged(1)]
#[case::further_edit(2)]
#[case::acknowledged_value(0)]
fn retry_writes_the_latest_desired_snapshot_with_a_new_token(#[case] latest: i8) {
    let reference = attachment();
    let mut state = AttachmentSettingsState::new(reference.clone(), settings(0));
    state.set_desired(settings(1));
    let failed = state.prepare_write(WriteAccess::Owner).expect("write");
    let completion = failed.clone().complete(Err("Disk full".to_owned()));
    assert_eq!(completion.attachment(), &reference);
    assert_eq!(completion.error(), Some("Disk full"));
    assert!(state.acknowledge(completion));
    state.set_desired(settings(latest));
    assert_eq!(state.acknowledged(), &settings(0));
    assert_eq!(state.desired(), &settings(latest));
    assert_eq!(
        state.status(WriteAccess::Owner),
        AttachmentSettingsStatus::Failed("Disk full")
    );
    assert!(state.prepare_write(WriteAccess::Owner).is_none());

    assert!(state.retry());
    assert!(!state.retry());
    let retry = state.prepare_write(WriteAccess::Owner).expect("retry");
    assert_eq!(retry.settings(), &settings(latest));
    assert_ne!(retry.revision(), failed.revision());
    assert!(!state.acknowledge(failed.complete(Ok(()))));
    assert!(state.acknowledge(retry.complete(Ok(()))));
    assert_eq!(
        state.status(WriteAccess::Owner),
        AttachmentSettingsStatus::Saved
    );
}

#[rstest]
#[case::replacement_context(true)]
#[case::other_attachment(false)]
fn completions_from_another_context_are_ignored(#[case] same_attachment: bool) {
    let reference = attachment();
    let mut original = AttachmentSettingsState::new(reference.clone(), settings(0));
    original.set_desired(settings(1));
    let stale = original
        .prepare_write(WriteAccess::Owner)
        .expect("original write");
    let mut replacement = AttachmentSettingsState::new(
        if same_attachment {
            reference
        } else {
            attachment()
        },
        settings(0),
    );
    assert!(!replacement.acknowledge(stale.clone().complete(Ok(()))));
    replacement.set_desired(settings(1));
    let current = replacement
        .prepare_write(WriteAccess::Owner)
        .expect("replacement write");
    assert_ne!(current.revision(), stale.revision());
    assert!(!replacement.acknowledge(stale.complete(Ok(()))));
    assert_eq!(replacement.acknowledged(), &settings(0));
    assert_eq!(
        replacement.status(WriteAccess::Owner),
        AttachmentSettingsStatus::Saving
    );
    assert!(replacement.acknowledge(current.complete(Ok(()))));
}

#[rstest]
#[case::initial_read_only(false)]
#[case::downgrade_during_a_write(true)]
fn read_only_access_retains_edits_and_prevents_new_writes(#[case] initially_writable: bool) {
    let mut state = AttachmentSettingsState::new(attachment(), settings(0));
    state.set_desired(settings(1));
    let pending =
        initially_writable.then(|| state.prepare_write(WriteAccess::Owner).expect("write"));
    state.set_desired(settings(2));
    assert_eq!(
        state.status(WriteAccess::ReadOnly),
        AttachmentSettingsStatus::SessionOnly
    );
    assert!(state.prepare_write(WriteAccess::ReadOnly).is_none());
    if let Some(pending) = pending {
        assert!(state.acknowledge(pending.complete(Ok(()))));
        assert_eq!(state.acknowledged(), &settings(1));
    } else {
        assert_eq!(state.acknowledged(), &settings(0));
    }
    assert!(state.prepare_write(WriteAccess::ReadOnly).is_none());
    assert_eq!(state.desired(), &settings(2));
    assert_eq!(
        state.status(WriteAccess::Owner),
        AttachmentSettingsStatus::Dirty
    );
}

#[test]
fn reverting_edits_before_submission_preserves_the_acknowledged_snapshot() {
    let mut state = AttachmentSettingsState::new(attachment(), settings(0));
    assert_eq!(
        state.status(WriteAccess::Owner),
        AttachmentSettingsStatus::Saved
    );
    assert!(!state.retry());
    state.set_desired(settings(1));
    state.set_desired(settings(0));
    assert_eq!(
        state.status(WriteAccess::Owner),
        AttachmentSettingsStatus::Saved
    );
    assert!(state.prepare_write(WriteAccess::Owner).is_none());
}

proptest::proptest! {
    #[test]
    fn arbitrary_edit_and_completion_sequences_preserve_acknowledged_settings(
        events in proptest::collection::vec((0_u8..5, -5_i8..=5, proptest::bool::ANY), 1..100),
    ) {
        let mut state = AttachmentSettingsState::new(attachment(), settings(0));
        let mut desired = 0;
        let mut persisted = 0;
        let mut pending: Option<(AttachmentSettingsWrite, i8)> = None;
        let mut completed = Vec::new();
        let mut failed = false;
        let mut retry_requested = false;
        for (action, marker, owner) in events {
            let access = if owner { WriteAccess::Owner } else { WriteAccess::ReadOnly };
            match action {
                0 => {
                    desired = marker;
                    state.set_desired(settings(desired));
                }
                1 => {
                    let eligible = owner && pending.is_none() && !failed
                        && (desired != persisted || retry_requested);
                    let write = state.prepare_write(access);
                    proptest::prop_assert_eq!(write.is_some(), eligible);
                    if let Some(write) = write {
                        proptest::prop_assert_eq!(write.settings(), &settings(desired));
                        pending = Some((write, desired));
                        retry_requested = false;
                    }
                }
                2 => {
                    if let Some((write, submitted)) = pending.take() {
                        let result = if owner { Ok(()) } else { Err("Disk full".to_owned()) };
                        let completion = write.complete(result);
                        proptest::prop_assert!(state.acknowledge(completion.clone()));
                        completed.push(completion);
                        if owner { persisted = submitted; } else { failed = true; }
                    }
                }
                3 => {
                    proptest::prop_assert_eq!(state.retry(), failed);
                    if failed {
                        retry_requested = true;
                        failed = false;
                    }
                }
                _ => {
                    for completion in &completed {
                        proptest::prop_assert!(!state.acknowledge(completion.clone()));
                    }
                }
            }
            proptest::prop_assert_eq!(state.desired(), &settings(desired));
            proptest::prop_assert_eq!(state.acknowledged(), &settings(persisted));
            let expected = if !owner {
                AttachmentSettingsStatus::SessionOnly
            } else if pending.is_some() {
                AttachmentSettingsStatus::Saving
            } else if failed {
                AttachmentSettingsStatus::Failed("Disk full")
            } else if desired != persisted || retry_requested {
                AttachmentSettingsStatus::Dirty
            } else {
                AttachmentSettingsStatus::Saved
            };
            proptest::prop_assert_eq!(state.status(access), expected);
        }
    }
}
