//! The environment data section of the settings window's Application page:
//! what each archive holds, and the controls that delete days from them.

use chrono::{Datelike as _, Days, NaiveDate, Utc};
use egui::{Button, Grid, RichText, Ui};
use egui_extras::DatePickerButton;
use egui_phosphor::regular::BROOM as ICON_BROOM;
use gt_store::ArchiveUsage;
use gt_ui_theme::EM_DASH;
use jiff::civil::Date;
use strum::IntoEnumIterator as _;

use super::civil_date;
use super::environment_storage::{
    CoveredDayCounts, EnvironmentArchive, EnvironmentUsage, PruneRequest, PruneScope, PrunedDays,
};

pub const ENVIRONMENT_DATA_LABEL: &str = "Environment data";
pub const PRUNE_LABEL: &str = "Prune days older than";
/// The button that opens the confirmation. The suffix marks a control that
/// needs further input before it acts, as DESIGN.md has it.
pub const PRUNE_BUTTON_LABEL: &str = "Prune…";
/// Suffixed like the prune button: it opens a confirmation before it acts.
pub const DELETE_ALL_LABEL: &str = "Delete all…";
const TOTAL_LABEL: &str = "Total";

/// Days back from today the cutoff starts at, the range the download controls
/// also open on.
const DEFAULT_CUTOFF_DAYS: u64 = 30;

/// Earliest year the cutoff picker offers: archives key their days from the
/// Unix epoch.
const EARLIEST_PICKABLE_YEAR: i16 = 1970;

/// Hover text of every control a running delete grays.
const PRUNE_RUNNING: &str = "Wait for the running delete to finish";

/// Session state of the section: the day its prune deletes everything before.
///
/// Held as the [`jiff::civil::Date`] [`DatePickerButton`] edits.
pub struct EnvironmentStorageUi {
    cutoff: Date,
}

impl Default for EnvironmentStorageUi {
    fn default() -> Self {
        Self::with_today(Utc::now().date_naive())
    }
}

impl EnvironmentStorageUi {
    /// Seeded [`DEFAULT_CUTOFF_DAYS`] before `today`.
    ///
    /// `today` is a parameter so a snapshot of the settings window does not
    /// change every day.
    pub fn with_today(today: NaiveDate) -> Self {
        Self {
            cutoff: civil_date::to_jiff(
                today
                    .checked_sub_days(Days::new(DEFAULT_CUTOFF_DAYS))
                    .unwrap_or(today),
            ),
        }
    }

    /// The day the prune deletes everything before, or [`None`] for a picked
    /// date that is not one.
    pub fn cutoff(&self) -> Option<NaiveDate> {
        civil_date::to_chrono(self.cutoff)
    }

    /// What each archive holds and the controls acting on it. Returns the
    /// delete the user pressed for, which the caller confirms first.
    pub fn ui(&mut self, ui: &mut Ui, state: EnvironmentStorageState<'_>) -> Option<PruneRequest> {
        let mut requested = None;
        Grid::new("environment_data_grid")
            .num_columns(5)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                for archive in EnvironmentArchive::iter() {
                    if let Some(request) = archive_row(ui, archive, state) {
                        requested = Some(request);
                    }
                }
                total_row(ui, state.usage.total());
            });
        ui.add_space(4.0);
        if let Some(request) = self.prune_controls(ui, state) {
            requested = Some(request);
        }
        requested
    }

    /// The cutoff picker and the button that requests the prune.
    fn prune_controls(
        &mut self,
        ui: &mut Ui,
        state: EnvironmentStorageState<'_>,
    ) -> Option<PruneRequest> {
        let mut requested = None;
        ui.horizontal(|ui| {
            ui.label(PRUNE_LABEL)
                .on_hover_text("UTC days. Every archive loses the days before this one.");
            let today = Utc::now().date_naive();
            let years = EARLIEST_PICKABLE_YEAR..=i16::try_from(today.year()).unwrap_or(i16::MAX);
            ui.add_enabled(
                !state.prune_running,
                DatePickerButton::new(&mut self.cutoff)
                    .id_salt("environment_prune_cutoff")
                    .start_end_years(years)
                    .highlight_weekends(false),
            )
            .on_disabled_hover_text(PRUNE_RUNNING);

            let request = self.cutoff().map(|cutoff| PruneRequest {
                scope: PruneScope::Every,
                days: PrunedDays::Before(cutoff),
            });
            let covered = state.days_before_cutoff.total();
            let enabled = !state.prune_running && covered > 0;
            let button = ui.add_enabled(
                enabled,
                Button::new(format!("{ICON_BROOM} {PRUNE_BUTTON_LABEL}")),
            );
            if button
                .on_hover_text(format!(
                    "Delete the {} the archives hold before that day",
                    day_count(covered)
                ))
                .on_disabled_hover_text(if state.prune_running {
                    PRUNE_RUNNING
                } else {
                    "No archive holds a day before the one picked"
                })
                .clicked()
            {
                requested = request;
            }
        });
        requested
    }
}

/// What the section draws.
#[derive(Clone, Copy)]
pub struct EnvironmentStorageState<'a> {
    pub usage: &'a EnvironmentUsage,
    /// How many days each archive holds before the cutoff the picker shows.
    pub days_before_cutoff: CoveredDayCounts,
    pub prune_running: bool,
}

/// One archive's row: what it holds, and the button that empties it.
fn archive_row(
    ui: &mut Ui,
    archive: EnvironmentArchive,
    state: EnvironmentStorageState<'_>,
) -> Option<PruneRequest> {
    let usage = state.usage.of(archive);
    ui.label(archive.label());
    ui.label(size_line(usage));
    ui.label(day_line(usage));
    ui.label(span_line(usage));

    let holds_days = usage.is_some_and(|usage| !usage.is_empty());
    let enabled = holds_days && !state.prune_running;
    let clicked = ui
        .add_enabled(enabled, Button::new(DELETE_ALL_LABEL).small())
        .on_hover_text("Delete every day this archive holds")
        .on_disabled_hover_text(if state.prune_running {
            PRUNE_RUNNING
        } else if usage.is_none() {
            "This archive could not be opened"
        } else {
            "This archive holds no day"
        })
        .clicked();
    ui.end_row();

    clicked.then_some(PruneRequest {
        scope: PruneScope::One(archive),
        days: PrunedDays::All,
    })
}

/// The row adding up every archive.
fn total_row(ui: &mut Ui, total: Option<ArchiveUsage>) {
    ui.label(RichText::new(TOTAL_LABEL).strong());
    ui.label(RichText::new(size_line(total)).strong());
    ui.label(day_line(total));
    ui.label(span_line(total));
    ui.end_row();
}

fn size_line(usage: Option<ArchiveUsage>) -> String {
    usage
        .and_then(|usage| usage.bytes)
        .map_or_else(|| EM_DASH.to_owned(), gt_fmt::format_bytes)
}

fn day_line(usage: Option<ArchiveUsage>) -> String {
    usage.map_or_else(|| EM_DASH.to_owned(), |usage| day_count(usage.days))
}

fn day_count(days: usize) -> String {
    format!("{days} {}", gt_fmt::pluralize(days, "day", "days"))
}

fn span_line(usage: Option<ArchiveUsage>) -> String {
    usage.and_then(|usage| usage.span).map_or_else(
        || EM_DASH.to_owned(),
        |span| format!("{} to {}", span.oldest, span.newest),
    )
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;

    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    use gt_store::ArchivedDaySpan;
    use gt_test_utils::TestHarness;
    use rstest::rstest;

    use super::*;

    fn day(offset: i64) -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 1).unwrap_or_default() + chrono::TimeDelta::days(offset)
    }

    fn usage(bytes: u64, days: usize, span: Option<(i64, i64)>) -> ArchiveUsage {
        ArchiveUsage {
            bytes: Some(bytes),
            days,
            span: span.map(|(oldest, newest)| ArchivedDaySpan {
                oldest: day(oldest),
                newest: day(newest),
            }),
        }
    }

    /// Every archive open and holding days, which is the state the user sees
    /// after a session with recordings loaded.
    fn filled_usage() -> EnvironmentUsage {
        EnvironmentUsage {
            interference: Some(usage(24_576, 2, Some((0, 1)))),
            geomagnetic_indices: Some(usage(12_288, 3, Some((0, 2)))),
            tec_maps: Some(usage(3_670_016, 29, Some((-27, 1)))),
            solar_flares: Some(usage(8_192, 1, Some((1, 1)))),
        }
    }

    fn state(usage: &EnvironmentUsage) -> EnvironmentStorageState<'_> {
        EnvironmentStorageState {
            usage,
            days_before_cutoff: CoveredDayCounts::default(),
            prune_running: false,
        }
    }

    /// Renders the section and reports the delete it requested.
    fn section(
        state: EnvironmentStorageState<'_>,
        act: impl FnOnce(&mut TestHarness<'_, ()>),
    ) -> Option<PruneRequest> {
        let mut ui_state = EnvironmentStorageUi::with_today(day(2));
        let requested = RefCell::new(None);
        let mut harness = TestHarness::builder().ui(|ui| {
            if let Some(request) = ui_state.ui(ui, state) {
                *requested.borrow_mut() = Some(request);
            }
        });
        harness.run();
        act(&mut harness);
        harness.run();
        drop(harness);
        requested.into_inner()
    }

    #[rstest]
    #[case::a_filled_archive(Some(usage(24_576, 2, Some((0, 1)))), "24.0 KB", "2 days", "2026-07-01 to 2026-07-02")]
    #[case::an_empty_archive(Some(ArchiveUsage { bytes: Some(2_048), days: 0, span: None }), "2.0 KB", "0 days", EM_DASH)]
    #[case::an_archive_that_would_not_open(None, EM_DASH, EM_DASH, EM_DASH)]
    fn a_row_states_what_its_archive_holds(
        #[case] usage: Option<ArchiveUsage>,
        #[case] size: &str,
        #[case] days: &str,
        #[case] span: &str,
    ) {
        assert_eq!(size_line(usage), size);
        assert_eq!(day_line(usage), days);
        assert_eq!(span_line(usage), span);
    }

    /// The total adds the sizes and the days up, and covers every span.
    #[test]
    fn the_total_row_adds_the_archives_up() {
        let usage = filled_usage();
        let total = usage.total().expect("every archive is open");

        assert_eq!(total.bytes, Some(24_576 + 12_288 + 3_670_016 + 8_192));
        assert_eq!(total.days, 2 + 3 + 29 + 1);
        assert_eq!(
            total.span,
            Some(ArchivedDaySpan {
                oldest: day(-27),
                newest: day(2)
            })
        );
    }

    /// Never hidden, per DESIGN.md: an archive holding nothing grays its
    /// delete.
    #[rstest]
    #[case::holding_days(Some(usage(2_048, 2, Some((0, 1)))), true)]
    #[case::empty(Some(ArchiveUsage { bytes: Some(2_048), days: 0, span: None }), false)]
    #[case::not_opened(None, false)]
    fn deleting_one_archive_needs_a_day_to_delete(
        #[case] archive: Option<ArchiveUsage>,
        #[case] enabled: bool,
    ) {
        let usage = EnvironmentUsage {
            interference: archive,
            ..EnvironmentUsage::default()
        };
        let mut ui_state = EnvironmentStorageUi::with_today(day(2));
        let mut harness = TestHarness::builder().ui(|ui| {
            ui_state.ui(ui, state(&usage));
        });
        harness.run();

        let deletes = harness
            .inner
            .query_all_by_label_contains(DELETE_ALL_LABEL)
            .next()
            .expect("every archive has a delete");
        assert_eq!(!deletes.accesskit_node().is_disabled(), enabled);
    }

    #[test]
    fn deleting_one_archive_requests_only_that_archive() {
        let usage = filled_usage();
        let request = section(state(&usage), |harness| {
            harness
                .inner
                .query_all_by_label_contains(DELETE_ALL_LABEL)
                .next()
                .expect("the first archive's delete")
                .click();
        });

        assert_eq!(
            request,
            Some(PruneRequest {
                scope: PruneScope::One(EnvironmentArchive::AircraftInterference),
                days: PrunedDays::All,
            })
        );
    }

    /// The prune button requests the picked cutoff across every archive.
    #[test]
    fn pruning_requests_the_days_before_the_cutoff() {
        let usage = filled_usage();
        let mut state = state(&usage);
        state.days_before_cutoff = CoveredDayCounts {
            tec_maps: 27,
            ..CoveredDayCounts::default()
        };

        let request = section(state, |harness| {
            harness
                .inner
                .get_by_label_contains(PRUNE_BUTTON_LABEL)
                .click();
        });

        assert_eq!(
            request,
            Some(PruneRequest {
                scope: PruneScope::Every,
                days: PrunedDays::Before(
                    day(2)
                        - chrono::TimeDelta::days(
                            i64::try_from(DEFAULT_CUTOFF_DAYS).unwrap_or_default()
                        )
                ),
            })
        );
    }

    /// A cutoff every archived day is at or after leaves the button grayed.
    #[test]
    fn a_cutoff_no_archive_reaches_back_past_disables_the_prune() {
        let usage = filled_usage();
        let mut ui_state = EnvironmentStorageUi::with_today(day(2));
        let mut harness = TestHarness::builder().ui(|ui| {
            ui_state.ui(ui, state(&usage));
        });
        harness.run();

        assert!(
            harness
                .inner
                .get_by_label_contains(PRUNE_BUTTON_LABEL)
                .accesskit_node()
                .is_disabled()
        );
    }

    /// A running delete leaves every control grayed: a second one would rewrite
    /// the same columns underneath it.
    #[test]
    fn a_running_delete_grays_every_control() {
        let usage = filled_usage();
        let mut state = state(&usage);
        state.prune_running = true;
        state.days_before_cutoff = CoveredDayCounts {
            tec_maps: 27,
            ..CoveredDayCounts::default()
        };
        let mut ui_state = EnvironmentStorageUi::with_today(day(2));
        let mut harness = TestHarness::builder().ui(|ui| {
            ui_state.ui(ui, state);
        });
        harness.run();

        assert!(
            harness
                .inner
                .get_by_label_contains(PRUNE_BUTTON_LABEL)
                .accesskit_node()
                .is_disabled()
        );
        for delete in harness.inner.query_all_by_label_contains(DELETE_ALL_LABEL) {
            assert!(delete.accesskit_node().is_disabled());
        }
    }
}
