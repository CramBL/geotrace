use gt_history_types::StoredYearResolution;
use gt_logfile::YearResolution;
use gt_pending_writes::WriteAccess;
use gt_test_utils::fixtures as recording_fixtures;

use crate::filter::ClockTicks;
use crate::loaded_log::LoadedLogs;
use crate::loaded_log::tests::fixtures;
use crate::test_util;
use crate::{AttachmentSettingsStatus, FilterEffect};

#[test]
fn year_changes_and_reset_rebuild_time_views_and_preserve_filter_definitions() {
    let resolved = test_util::log_with_unresolved_years()
        .parsed()
        .resolve_years_as(YearResolution::PlaceLastEntryInYear(2026))
        .unwrap();
    let start = resolved
        .chronology()
        .resolved()
        .unwrap()
        .timestamp(0)
        .unwrap();
    let files = test_util::loaded(vec![test_util::recording_of(
        recording_fixtures::nav_points_from(start, 3, 1),
    )]);
    let mut logs = LoadedLogs::default();
    let id = logs.push(test_util::log_with_unresolved_years()).id();
    fixtures::add_layer_chip(&mut logs, id, "navsyncd");
    let (stack, slots) = logs.filter_stack_mut_by_id(id).unwrap();
    let chip = stack.chips().first().unwrap().id();
    stack.add_chip_effect(chip, FilterEffect::Table, slots);
    stack.set_chip_effect_enabled(chip, FilterEffect::Table, false);
    stack.set_live_filter_text("navsyncd");
    stack.wait_for_queries();
    logs.get_mut_by_id(id)
        .unwrap()
        .anchor_to(test_util::key_of(&files, 0), &files.view());
    let log = logs.get_by_id(id).unwrap();
    let filters = log.filters().to_stored_stack();
    let slot = fixtures::first_chip_slot(&logs, id);
    let semantic_revision = log.filters().table_semantic_revision();
    let mut visible_revision = log.filters().visible_revision();
    assert_eq!(test_util::map_matches(&mut logs, &files).match_count(), 0);

    for (policy, expected_matches) in [
        (YearResolution::PlaceLastEntryInYear(2026), 1),
        (YearResolution::PlaceLastEntryInYear(2027), 0),
        (YearResolution::Infer, 0),
        (YearResolution::PlaceLastEntryInYear(2026), 1),
    ] {
        assert!(
            logs.get_mut_by_id(id)
                .unwrap()
                .resolve_years_as(policy, &files.view())
        );
        let log = logs.get_by_id(id).unwrap();
        assert_eq!(log.filters().to_stored_stack(), filters);
        assert_eq!(log.filters().live_filter_text(), "navsyncd");
        assert_eq!(log.filters().table_semantic_revision(), semantic_revision);
        assert_ne!(log.filters().visible_revision(), visible_revision);
        visible_revision = log.filters().visible_revision();
        assert_eq!(
            *log.filters().clock_ticks(),
            ClockTicks::of(log.parsed(), log.filters().visible_entries())
        );
        assert_eq!(log.associated_entry_count(), expected_matches);
        assert_eq!(log.anchor_key(), Some(test_util::key_of(&files, 0)));
        if policy == YearResolution::Infer {
            assert_eq!(log.entry_time_range(), None);
        }
        assert_eq!(fixtures::first_chip_slot(&logs, id), slot);
        assert_eq!(
            test_util::map_matches(&mut logs, &files).match_count(),
            expected_matches
        );
    }
}

#[test]
fn applying_the_current_year_policy_preserves_the_visible_revision() {
    let files = test_util::loaded(Vec::new());
    let mut log = test_util::log_of(3);
    let original = log.parsed().clone();
    let revision = log.filters().visible_revision();
    assert!(!log.resolve_years_as(YearResolution::Infer, &files.view()));
    assert_eq!(log.parsed(), &original);
    assert_eq!(log.filters().visible_revision(), revision);
}

#[test]
fn chronology_edits_require_the_latest_acknowledgement_and_retry_preserves_reset() {
    let files = test_util::loaded(Vec::new());
    let mut logs = LoadedLogs::default();
    let id = logs
        .restore_attachment(
            test_util::log_with_unresolved_years(),
            fixtures::attachment_ref(),
            fixtures::settings(),
            &files.view(),
        )
        .id();
    logs.get_mut_by_id(id)
        .unwrap()
        .resolve_years_as(YearResolution::PlaceLastEntryInYear(2026), &files.view());
    let first = logs
        .prepare_settings_writes(WriteAccess::Owner)
        .pop()
        .unwrap();
    logs.get_mut_by_id(id)
        .unwrap()
        .resolve_years_as(YearResolution::PlaceLastEntryInYear(2027), &files.view());
    let first_completion = first.complete(Ok(()));
    assert!(logs.acknowledge_settings(first_completion.clone()));
    let log = logs.get_by_id(id).unwrap();
    assert_eq!(
        log.settings_save_status(WriteAccess::Owner),
        AttachmentSettingsStatus::Dirty
    );
    let second = logs
        .prepare_settings_writes(WriteAccess::Owner)
        .pop()
        .unwrap();
    assert_eq!(
        second.settings().year_resolution,
        StoredYearResolution::PlaceLastEntryInYear { year: 2027 }
    );
    assert!(!logs.acknowledge_settings(first_completion));
    assert!(logs.acknowledge_settings(second.complete(Err("Disk full".to_owned()))));
    logs.get_mut_by_id(id)
        .unwrap()
        .resolve_years_as(YearResolution::Infer, &files.view());
    let log = logs.get_by_id(id).unwrap();
    assert!(!log.parsed().chronology().is_resolved());
    assert_eq!(
        log.settings_save_status(WriteAccess::Owner),
        AttachmentSettingsStatus::Failed("Disk full")
    );
    assert_eq!(
        log.settings_save_status(WriteAccess::ReadOnly),
        AttachmentSettingsStatus::SessionOnly
    );
    assert!(logs.prepare_settings_writes(WriteAccess::Owner).is_empty());
    assert!(logs.retry_settings_save(id));
    let retry = logs
        .prepare_settings_writes(WriteAccess::Owner)
        .pop()
        .unwrap();
    assert_eq!(
        retry.settings().year_resolution,
        StoredYearResolution::Infer
    );
    assert!(retry.settings().year_reference.is_some());
    assert!(logs.acknowledge_settings(retry.complete(Ok(()))));
    assert_eq!(
        logs.get_by_id(id)
            .unwrap()
            .settings_save_status(WriteAccess::Owner),
        AttachmentSettingsStatus::Saved
    );
}
