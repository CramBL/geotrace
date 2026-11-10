use gt_logfile::YearResolution;

use crate::filter::ClockTicks;
use crate::loaded_log::LoadedLogs;
use crate::loaded_log::tests::fixtures;
use crate::test_util;

#[test]
fn resolving_the_years_of_a_log_keeps_its_chips_and_recomputes_its_clock_ticks() {
    let files = test_util::loaded(Vec::new());
    let mut logs = LoadedLogs::default();
    let id = logs.push(test_util::log_with_unresolved_years()).id();
    fixtures::add_layer_chip(&mut logs, id, "navsyncd");
    let unresolved_revision = logs
        .get_by_id(id)
        .map(|log| log.filters().visible_revision());

    let resolved = logs.get_mut_by_id(id).is_some_and(|mut log| {
        log.resolve_years_as(YearResolution::PlaceLastEntryInYear(2026), &files.view())
    });

    assert!(resolved);
    let log = logs.get_by_id(id).expect("the log stays loaded");
    assert!(log.parsed().chronology().is_resolved());
    assert_eq!(log.filters().chips().len(), 1);
    assert_ne!(Some(log.filters().visible_revision()), unresolved_revision);
    assert_ne!(*log.filters().clock_ticks(), ClockTicks::default());
    assert_eq!(
        *log.filters().clock_ticks(),
        ClockTicks::of(log.parsed(), log.filters().visible_entries())
    );
}

#[test]
fn resolving_the_years_of_a_resolved_log_changes_nothing() {
    let files = test_util::loaded(Vec::new());
    let mut log = test_util::log_of(3);
    let timestamps = log
        .parsed()
        .chronology()
        .resolved()
        .map(|resolved| resolved.timestamps().to_vec());

    let resolved_again =
        log.resolve_years_as(YearResolution::PlaceLastEntryInYear(2020), &files.view());

    assert!(!resolved_again);
    assert_eq!(log.parsed().year_resolution(), YearResolution::Infer);
    assert_eq!(
        log.parsed()
            .chronology()
            .resolved()
            .map(|resolved| resolved.timestamps().to_vec()),
        timestamps
    );
}
