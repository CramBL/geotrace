//! What a day-keyed fetch worker is doing, and the settings rows reporting it.
//!
//! Shared by every day-keyed fetch worker: only the hover text states the
//! source.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use egui::Ui;
use gt_ui_theme::EM_DASH;

use super::environment_storage::PrunedDays;

/// What the archive holds for one UTC day a fetch worker counts.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DayArchiveState {
    /// Everything the source publishes for the day is archived.
    Archived,
    /// Something the source publishes for the day is still to be fetched.
    Awaited,
}

/// A set of UTC days one fetch worker counts, and what the archive holds for
/// each.
#[derive(Debug, Default)]
pub struct DayArchiveCoverage {
    days: BTreeMap<NaiveDate, DayArchiveState>,
}

impl DayArchiveCoverage {
    pub fn record(&mut self, day: NaiveDate, state: DayArchiveState) {
        self.days.insert(day, state);
    }

    /// Report `day` as fully archived, ignoring a day missing from this set:
    /// a backfilled day is not part of the count.
    pub fn mark_archived(&mut self, day: NaiveDate) {
        if let Some(state) = self.days.get_mut(&day) {
            *state = DayArchiveState::Archived;
        }
    }

    /// Report every day `pruned` covers as still to be fetched, for an
    /// archive those days have just left.
    pub fn mark_pruned_days_awaited(&mut self, pruned: PrunedDays) {
        for (day, state) in &mut self.days {
            if pruned.covers(*day) {
                *state = DayArchiveState::Awaited;
            }
        }
    }

    pub fn oldest_day(&self) -> Option<NaiveDate> {
        self.days.keys().next().copied()
    }

    pub fn holds(&self, day: NaiveDate) -> bool {
        self.days.contains_key(&day)
    }

    /// Take `day` out of the count, for a day that has moved to another set.
    pub fn forget(&mut self, day: NaiveDate) {
        self.days.remove(&day);
    }

    pub fn counts(&self) -> ArchivedDayCount {
        ArchivedDayCount {
            days: self.days.len(),
            archived: self
                .days
                .values()
                .filter(|state| **state == DayArchiveState::Archived)
                .count(),
        }
    }
}

/// UTC days one fetch worker counts, and how many of them the archive holds
/// everything published for.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ArchivedDayCount {
    pub days: usize,
    pub archived: usize,
}

impl ArchivedDayCount {
    /// The row's text, an absent value while there is nothing to count.
    fn line(self) -> String {
        if self.days == 0 {
            return EM_DASH.to_owned();
        }
        format!("{} of {} archived", self.archived, self.days)
    }
}

/// What the fetch worker is doing, and how far the archive covers the loaded
/// recordings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct DayFetchStatus {
    /// The day being fetched right now.
    pub fetching: Option<NaiveDate>,
    /// Days waiting behind it.
    pub queued: usize,
    /// UTC days the recordings loaded this session span, inside the source's
    /// coverage.
    pub recording_days: ArchivedDayCount,
}

impl DayFetchStatus {
    fn queue_line(&self) -> String {
        let queued = format!(
            "{} {} queued",
            self.queued,
            gt_fmt::pluralize(self.queued, "day", "days")
        );
        match (self.fetching, self.queued) {
            (Some(day), 0) => format!("Fetching {day}"),
            (Some(day), _) => format!("Fetching {day}, {queued}"),
            (None, 0) => "Idle".to_owned(),
            (None, _) => queued,
        }
    }
}

/// The hover text stating the source the rows report on.
#[derive(Debug, Clone, Copy)]
pub struct FetchRowHoverText {
    pub queue: &'static str,
    pub coverage: &'static str,
}

pub const FETCH_QUEUE_LABEL: &str = "Fetch queue";
pub const RECORDING_DAYS_LABEL: &str = "Recording days";
pub const BACKGROUND_DAYS_LABEL: &str = "Background days";

/// Two rows of a data source page's grid: what is being fetched, and what the
/// archive holds for the loaded recordings.
pub fn show_fetch_rows(ui: &mut Ui, status: DayFetchStatus, hover: FetchRowHoverText) {
    ui.label(FETCH_QUEUE_LABEL).on_hover_text(hover.queue);
    ui.label(status.queue_line()).on_hover_text(hover.queue);
    ui.end_row();

    ui.label(RECORDING_DAYS_LABEL).on_hover_text(hover.coverage);
    ui.label(status.recording_days.line())
        .on_hover_text(hover.coverage);
    ui.end_row();
}

/// One more row, for a source that also fetches the days before each recording
/// day.
pub fn show_background_day_row(ui: &mut Ui, coverage: ArchivedDayCount, hover: &str) {
    ui.label(BACKGROUND_DAYS_LABEL).on_hover_text(hover);
    ui.label(coverage.line()).on_hover_text(hover);
    ui.end_row();
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    fn day(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap_or_default()
    }

    #[rstest]
    #[case::idle(None, 0, "Idle")]
    #[case::fetching_the_last_day(Some(day(2026, 7, 20)), 0, "Fetching 2026-07-20")]
    #[case::fetching_with_a_queue(Some(day(2026, 7, 20)), 3, "Fetching 2026-07-20, 3 days queued")]
    #[case::one_day_behind(Some(day(2026, 7, 20)), 1, "Fetching 2026-07-20, 1 day queued")]
    #[case::queued_without_a_request_in_flight(None, 2, "2 days queued")]
    fn the_queue_line_states_what_is_in_flight_and_what_waits(
        #[case] fetching: Option<NaiveDate>,
        #[case] queued: usize,
        #[case] expected: &str,
    ) {
        let status = DayFetchStatus {
            fetching,
            queued,
            ..DayFetchStatus::default()
        };
        assert_eq!(status.queue_line(), expected);
    }

    /// Without a loaded recording the coverage line shows an absent value,
    /// not a count.
    #[rstest]
    #[case::nothing_loaded(0, 0, EM_DASH)]
    #[case::partly_archived(7, 5, "5 of 7 archived")]
    #[case::fully_archived(3, 3, "3 of 3 archived")]
    fn the_coverage_line_counts_the_archived_days(
        #[case] days: usize,
        #[case] archived: usize,
        #[case] expected: &str,
    ) {
        assert_eq!(ArchivedDayCount { days, archived }.line(), expected);
    }

    /// A backfill cannot report coverage the loaded recordings do not have:
    /// a day none of them spans stays out of the count.
    #[test]
    fn archiving_a_day_outside_the_loaded_recordings_changes_no_count() {
        let mut coverage = DayArchiveCoverage::default();
        coverage.record(day(2026, 7, 20), DayArchiveState::Awaited);

        coverage.mark_archived(day(2026, 7, 25));

        assert_eq!(
            coverage.counts(),
            ArchivedDayCount {
                days: 1,
                archived: 0
            }
        );
    }
}
