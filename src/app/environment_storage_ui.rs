//! The environment data section of the settings window's Application page:
//! what each archive holds, and the controls that delete days from them.

use chrono::{Datelike as _, Days, NaiveDate, Utc};
use egui::{Button, Checkbox, DragValue, Grid, RichText, Ui};
use egui_extras::DatePickerButton;
use egui_phosphor::regular::BROOM as ICON_BROOM;
use gt_pending_writes::WriteAccess;
use gt_store::{ArchiveUsage, EnvironmentArchive};
use gt_ui_theme::EM_DASH;
use jiff::civil::Date;
use strum::IntoEnumIterator as _;

use super::archive_recovery::{ArchiveUnavailable, UnavailableArchives};
use super::archives_unreachable::ArchivesUnreachable;
use super::civil_date;
use super::environment_storage::{
    CoveredDayCounts, EnvironmentUsage, PruneRequest, PruneScope, PrunedDays,
};
use super::read_only_session::READ_ONLY_ARCHIVES_HOVER;
use crate::settings::EnvironmentStorageSettings;

pub const ENVIRONMENT_DATA_LABEL: &str = "Environment data";
pub const PRUNE_LABEL: &str = "Prune days older than";
pub const AUTO_PRUNE_LABEL: &str = "Auto-prune days older than";
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

/// Why the controls that delete archived days are grayed out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DeleteBlocker {
    /// There is no archive to delete from.
    ArchivesUnreachable(ArchivesUnreachable),
    /// A delete is already rewriting the same columns.
    DeleteRunning,
    /// This one archive is closed for the session, on the choice the user
    /// made for it.
    ArchiveUnavailable(ArchiveUnavailable),
}

impl DeleteBlocker {
    pub fn hover_text(self) -> String {
        match self {
            Self::ArchivesUnreachable(reason) => match reason {
                ArchivesUnreachable::ReadOnlySession => READ_ONLY_ARCHIVES_HOVER,
                ArchivesUnreachable::WaitingForTheDataDirectory => {
                    "Wait for the data directory to become available"
                }
                ArchivesUnreachable::AwaitingAnInterruptedDeleteChoice => {
                    "Choose what to do about the interrupted delete"
                }
                ArchivesUnreachable::ArchivesOpening => "Wait for the archives to finish opening",
            }
            .to_owned(),
            Self::DeleteRunning => "Wait for the running delete to finish".to_owned(),
            Self::ArchiveUnavailable(reason) => format!(
                "This archive is unavailable this session: {}",
                reason.explanation()
            ),
        }
    }
}

/// Hover text of the age control while auto-pruning is off.
const ENABLE_AUTO_PRUNE_FIRST: &str = "Tick 'Auto-prune days older than' to configure this";

/// The auto-prune switch and the age past which it deletes archived days.
///
/// This row starts nothing itself and stays live while a delete runs:
/// auto-pruning acts at startup and after a recording finishes loading. The
/// row is grayed out in a read-only session: it prunes at neither point.
pub fn show_auto_prune_age(
    ui: &mut Ui,
    settings: &mut EnvironmentStorageSettings,
    write_access: WriteAccess,
) {
    let writes_archives = write_access.allows_writing();
    ui.horizontal(|ui| {
        ui.add_enabled(
            writes_archives,
            Checkbox::new(&mut settings.auto_prune_enabled, AUTO_PRUNE_LABEL),
        )
        .on_hover_text(
            "Delete days past this age from every archive on startup and after a recording \
             loads. Days a loaded recording needs are kept.",
        )
        .on_disabled_hover_text(READ_ONLY_ARCHIVES_HOVER);

        let auto_prune_on = settings.auto_prune_enabled && writes_archives;
        ui.add_enabled(
            auto_prune_on,
            DragValue::new(&mut settings.auto_prune_max_age_months)
                .range(EnvironmentStorageSettings::AUTO_PRUNE_AGE_MONTHS_RANGE),
        )
        .on_hover_text("Age an archived day reaches before it is deleted")
        .on_disabled_hover_text(if writes_archives {
            ENABLE_AUTO_PRUNE_FIRST
        } else {
            READ_ONLY_ARCHIVES_HOVER
        });

        ui.label("months");
    });
}

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
                state.deletes_blocked_by.is_none(),
                DatePickerButton::new(&mut self.cutoff)
                    .id_salt("environment_prune_cutoff")
                    .start_end_years(years)
                    .highlight_weekends(false),
            )
            .on_disabled_hover_text(
                state
                    .deletes_blocked_by
                    .map_or_else(String::new, DeleteBlocker::hover_text),
            );

            let request = self.cutoff().map(|cutoff| PruneRequest {
                scope: PruneScope::Every,
                days: PrunedDays::Before(cutoff),
            });
            let covered = state.days_before_cutoff.total();
            let enabled = state.deletes_blocked_by.is_none() && covered > 0;
            let button = ui.add_enabled(
                enabled,
                Button::new(format!("{ICON_BROOM} {PRUNE_BUTTON_LABEL}")),
            );
            if button
                .on_hover_text(format!(
                    "Delete the {} the archives hold before that day",
                    day_count(covered)
                ))
                .on_disabled_hover_text(state.deletes_blocked_by.map_or_else(
                    || "No archive holds a day before the one picked".to_owned(),
                    DeleteBlocker::hover_text,
                ))
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
    /// What stops the delete controls from taking input, when something does.
    pub deletes_blocked_by: Option<DeleteBlocker>,
    /// Why the app has none of an archive this session, where it has none.
    pub unavailable_archives: UnavailableArchives,
}

impl EnvironmentStorageState<'_> {
    /// What stops one archive's delete from taking input: whatever blocks
    /// every delete, or that archive being closed for the session.
    fn delete_blocked_by(self, archive: EnvironmentArchive) -> Option<DeleteBlocker> {
        self.deletes_blocked_by
            .or_else(|| self.unavailable_archives[archive].map(DeleteBlocker::ArchiveUnavailable))
    }
}

/// One archive's row: what it holds, and the button that empties it.
fn archive_row(
    ui: &mut Ui,
    archive: EnvironmentArchive,
    state: EnvironmentStorageState<'_>,
) -> Option<PruneRequest> {
    let usage = state.usage[archive];
    ui.label(archive.label());
    ui.label(size_line(usage));
    ui.label(day_line(usage));
    ui.label(span_line(usage));

    let holds_days = usage.is_some_and(|usage| !usage.is_empty());
    let blocked_by = state.delete_blocked_by(archive);
    let clicked = ui
        .add_enabled(
            holds_days && blocked_by.is_none(),
            Button::new(DELETE_ALL_LABEL).small(),
        )
        .on_hover_text("Delete every day this archive holds")
        .on_disabled_hover_text(blocked_by.map_or_else(
            || {
                if usage.is_none() {
                    "This archive could not be opened".to_owned()
                } else {
                    "This archive holds no day".to_owned()
                }
            },
            DeleteBlocker::hover_text,
        ))
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

    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    use gt_store::ArchivedDaySpan;
    use gt_test_utils::{By, HarnessInteraction as _, TestHarness};
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
        let mut filled = EnvironmentUsage::default();
        filled[EnvironmentArchive::AircraftInterference] = Some(usage(24_576, 2, Some((0, 1))));
        filled[EnvironmentArchive::GeomagneticIndices] = Some(usage(12_288, 3, Some((0, 2))));
        filled[EnvironmentArchive::IonosphericTec] = Some(usage(3_670_016, 29, Some((-27, 1))));
        filled[EnvironmentArchive::SolarFlares] = Some(usage(8_192, 1, Some((1, 1))));
        filled
    }

    fn state(usage: &EnvironmentUsage) -> EnvironmentStorageState<'_> {
        EnvironmentStorageState {
            usage,
            days_before_cutoff: CoveredDayCounts::default(),
            deletes_blocked_by: None,
            unavailable_archives: UnavailableArchives::default(),
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

    /// Never hidden, per DESIGN.md: the age is grayed while auto-pruning is
    /// off.
    #[rstest]
    #[case::auto_pruning_off(false)]
    #[case::auto_pruning_on(true)]
    fn the_auto_prune_age_takes_input_only_while_auto_pruning_is_on(
        #[case] auto_prune_enabled: bool,
    ) {
        let mut settings = EnvironmentStorageSettings {
            auto_prune_enabled,
            ..EnvironmentStorageSettings::default()
        };
        let mut harness = TestHarness::builder()
            .ui(|ui| show_auto_prune_age(ui, &mut settings, WriteAccess::Owner));
        harness.run();

        let age = harness.inner.get(By::new().role(Role::SpinButton));
        assert_eq!(!age.accesskit_node().is_disabled(), auto_prune_enabled);
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
        let mut usage = EnvironmentUsage::default();
        usage[EnvironmentArchive::AircraftInterference] = archive;
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
        state.days_before_cutoff[EnvironmentArchive::IonosphericTec] = 27;

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

    /// Never hidden, per DESIGN.md: whatever blocks a delete grays every
    /// control that starts one, and the hover says which it was.
    #[rstest]
    #[case::waiting_for_the_data_directory(DeleteBlocker::ArchivesUnreachable(
        ArchivesUnreachable::WaitingForTheDataDirectory
    ))]
    #[case::archives_opening(DeleteBlocker::ArchivesUnreachable(
        ArchivesUnreachable::ArchivesOpening
    ))]
    #[case::delete_running(DeleteBlocker::DeleteRunning)]
    fn a_blocked_delete_grays_every_control(#[case] blocker: DeleteBlocker) {
        let usage = filled_usage();
        let mut state = state(&usage);
        state.deletes_blocked_by = Some(blocker);
        state.days_before_cutoff[EnvironmentArchive::IonosphericTec] = 27;
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

        harness
            .inner
            .hover_and_settle(By::new().label_contains(PRUNE_BUTTON_LABEL), 3);
        harness.inner.get_by_label_contains(&blocker.hover_text());
    }
}
