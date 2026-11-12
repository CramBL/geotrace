use std::cell::Cell;

use gt_logfile::YearResolution;
use rstest::rstest;

use super::*;
use crate::{FilterEffect, LoadedLog, test_util};

struct Fixture {
    logs: LoadedLogs,
    id: LoadedLogId,
}

impl Fixture {
    fn of(text: &str) -> Self {
        let mut logs = LoadedLogs::default();
        let id = logs
            .push(LoadedLog::new(
                None,
                test_util::parsed_log_of_text(text),
                test_util::association_window(),
            ))
            .id();
        Self { logs, id }
    }

    fn finish(&self, search: &mut SourceSearch, text: &str, scope: SourceSearchScope) {
        search
            .request(&self.logs, text, scope)
            .unwrap()
            .run(&search.shared);
        assert!(search.take_completed());
        assert_eq!(search.status(), &SourceSearchStatus::ShowResults);
    }
}

#[rstest]
#[case::timestamp("2026-01-01", vec![(1, 0..10), (4, 0..10)])]
#[case::unicode("café", vec![(1, 25..30), (1, 31..36)])]
#[case::repeated("aba", vec![(1, 37..40), (1, 41..44)])]
#[case::literal("[x]", vec![(1, 45..48)])]
#[case::raw("\t", vec![(2, 1..2)])]
#[case::structural("reboot", vec![(3, 11..17)])]
#[case::missing("missing", vec![])]
fn source_search_returns_original_physical_spans(
    #[case] text: &str,
    #[case] expected: Vec<(u32, Range<usize>)>,
) {
    let fixture = Fixture::of(SOURCE);
    let mut search = SourceSearch::new(fixture.id);
    fixture.finish(&mut search, text, SourceSearchScope::SearchWholeLog);
    assert_eq!(
        search
            .occurrences()
            .iter()
            .map(|hit| (hit.line_number(), hit.byte_range()))
            .collect::<Vec<_>>(),
        expected
    );
    for hit in search.occurrences() {
        assert_eq!(hit.log(), fixture.id);
        assert_eq!(
            hit.matched_text(&fixture.logs).unwrap().to_lowercase(),
            text.to_lowercase()
        );
    }
    assert!(
        search
            .request(&fixture.logs, text, SourceSearchScope::SearchWholeLog)
            .is_none()
    );
}

#[test]
fn navigation_wraps_and_empty_queries_cancel_pending_work() {
    let fixture = Fixture::of(SOURCE);
    let mut search = SourceSearch::new(fixture.id);
    fixture.finish(&mut search, "aba", SourceSearchScope::SearchWholeLog);
    assert_eq!(search.current_index(), Some(0));
    assert_eq!(search.select_previous().unwrap().byte_range(), 41..44);
    assert_eq!(search.select_next().unwrap().byte_range(), 37..40);
    let work = search
        .request(&fixture.logs, "beta", SourceSearchScope::SearchWholeLog)
        .unwrap();
    assert!(
        search
            .request(&fixture.logs, "", SourceSearchScope::SearchWholeLog)
            .is_none()
    );
    work.run(&search.shared);
    assert!(!search.take_completed());
    assert_eq!(search.status(), &SourceSearchStatus::ReportEmptyQuery);
    assert_eq!(search.select_next(), None);
    assert_eq!(search.select_previous(), None);
}

#[test]
fn obsolete_jobs_and_completions_cannot_replace_newer_results() {
    let fixture = Fixture::of(SOURCE);
    let mut search = SourceSearch::new(fixture.id);
    let first = search
        .request(&fixture.logs, "aba", SourceSearchScope::SearchWholeLog)
        .unwrap();
    let old_result = first.scan(|| true).unwrap();
    let generation = first.generation;
    let latest = search
        .request(&fixture.logs, "beta", SourceSearchScope::SearchWholeLog)
        .unwrap();
    first.run(&search.shared);
    search.shared.complete(generation, old_result);
    assert!(!search.take_completed());
    latest.run(&search.shared);
    assert!(search.take_completed());
    assert_eq!(
        search.current().unwrap().matched_text(&fixture.logs),
        Some("beta")
    );
    let work = search
        .request(&fixture.logs, "a", SourceSearchScope::SearchWholeLog)
        .unwrap();
    let checks = Cell::new(0);
    assert_eq!(
        work.scan(|| {
            checks.set(checks.get() + 1);
            checks.get() < 4
        }),
        None
    );
    let shared = Arc::clone(&search.shared);
    drop(search);
    work.run(&shared);
    assert!(shared.result.lock().is_none());
}

#[test]
fn filtered_search_waits_for_scan_landings_and_preserves_selected_source() {
    let mut fixture = Fixture::of(SOURCE);
    let mut search = SourceSearch::new(fixture.id);
    fixture.finish(
        &mut search,
        "2026",
        SourceSearchScope::SearchFilteredEntries,
    );
    search.select_next();
    let selected = search.current().unwrap().clone();
    let old = search
        .request(
            &fixture.logs,
            "beta",
            SourceSearchScope::SearchFilteredEntries,
        )
        .unwrap();
    let (stack, _) = fixture.logs.filter_stack_mut_by_id(fixture.id).unwrap();
    stack.set_live_filter_text("beta");
    assert!(
        search
            .request(
                &fixture.logs,
                "2026",
                SourceSearchScope::SearchFilteredEntries
            )
            .is_none()
    );
    assert_eq!(search.status(), &SourceSearchStatus::AwaitFilters);
    old.run(&search.shared);
    assert!(!search.take_completed());
    fixture
        .logs
        .filter_stack_mut_by_id(fixture.id)
        .unwrap()
        .0
        .wait_for_queries();
    fixture.finish(
        &mut search,
        "2026",
        SourceSearchScope::SearchFilteredEntries,
    );
    assert_eq!(search.current(), Some(&selected));
    assert_eq!(search.occurrences().len(), 1);
    let (stack, _) = fixture.logs.filter_stack_mut_by_id(fixture.id).unwrap();
    stack.set_live_filter_effect(FilterEffect::Map);
    stack.set_live_filter_text("aba");
    stack.wait_for_queries();
    fixture.finish(
        &mut search,
        "2026",
        SourceSearchScope::SearchFilteredEntries,
    );
    let before = fixture
        .logs
        .get_by_id(fixture.id)
        .unwrap()
        .filters()
        .to_stored_stack();
    fixture.finish(&mut search, "reboot", SourceSearchScope::SearchWholeLog);
    assert_eq!(search.current().unwrap().line_number(), 3);
    assert_eq!(
        fixture
            .logs
            .get_by_id(fixture.id)
            .unwrap()
            .filters()
            .to_stored_stack(),
        before
    );
}

#[test]
fn chronology_rebuilds_preserve_source_results_and_unload_cancels_work() {
    let mut fixture = Fixture::of(gt_test_utils::UNRESOLVED_LEAP_DAYS_LOG);
    let mut search = SourceSearch::new(fixture.id);
    fixture.finish(&mut search, "Feb", SourceSearchScope::SearchWholeLog);
    let selected = search.select_previous().unwrap().clone();
    let files = test_util::loaded(Vec::new());
    assert!(
        fixture
            .logs
            .get_mut_by_id(fixture.id)
            .unwrap()
            .resolve_years_as(YearResolution::PlaceLastEntryInYear(2024), &files.view())
    );
    assert!(
        search
            .request(&fixture.logs, "Feb", SourceSearchScope::SearchWholeLog)
            .is_none()
    );
    assert_eq!(search.current(), Some(&selected));
    let work = search
        .request(&fixture.logs, "29", SourceSearchScope::SearchWholeLog)
        .unwrap();
    fixture.logs.remove_by_id(fixture.id);
    assert!(
        search
            .request(&fixture.logs, "29", SourceSearchScope::SearchWholeLog)
            .is_none()
    );
    work.run(&search.shared);
    assert!(!search.take_completed());
    assert_eq!(search.status(), &SourceSearchStatus::ReportUnavailableLog);
    assert_eq!(selected.source_text(&fixture.logs), None);
    let mut empty = SourceSearch::new(fixture.id);
    assert!(empty.update(&fixture.logs, "", SourceSearchScope::SearchWholeLog));
    assert_eq!(empty.status(), &SourceSearchStatus::ReportUnavailableLog);
}

const SOURCE: &str = "2026-01-01 00:00:00 svc: café CAFÉ aba aba [x]\r\n \t\r\n--- Device reboot ---\n2026-01-01 00:00:01 svc: beta\n";
