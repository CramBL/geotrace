//! What a day-keyed fetch worker is doing, and the settings rows reporting it.
//!
//! Shared by every day-keyed fetch worker: only the hover text names the
//! source.

use std::collections::BTreeMap;

use chrono::NaiveDate;
use egui::Ui;
use gt_ui_theme::EM_DASH;

/// What the archive holds for one UTC day a loaded recording spans.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordingDayArchiveState {
    /// Everything the source publishes for the day is archived.
    Archived,
    /// Something the source publishes for the day is still to be fetched.
    Awaited,
}

/// The UTC days the recordings loaded this session span, and what the archive
/// holds for each.
#[derive(Debug, Default)]
pub struct RecordingDayCoverage {
    days: BTreeMap<NaiveDate, RecordingDayArchiveState>,
}

impl RecordingDayCoverage {
    pub fn record(&mut self, day: NaiveDate, state: RecordingDayArchiveState) {
        self.days.insert(day, state);
    }

    /// Report `day` as fully archived, ignoring a day no loaded recording
    /// spans: a backfilled day is not part of the count.
    pub fn mark_archived(&mut self, day: NaiveDate) {
        if let Some(state) = self.days.get_mut(&day) {
            *state = RecordingDayArchiveState::Archived;
        }
    }

    pub fn recording_days(&self) -> usize {
        self.days.len()
    }

    pub fn archived_recording_days(&self) -> usize {
        self.days
            .values()
            .filter(|state| **state == RecordingDayArchiveState::Archived)
            .count()
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
    pub recording_days: usize,
    /// Those of them the archive holds everything the source publishes for.
    pub archived_recording_days: usize,
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

    fn coverage_line(&self) -> String {
        if self.recording_days == 0 {
            return EM_DASH.to_owned();
        }
        format!(
            "{} of {} archived",
            self.archived_recording_days, self.recording_days
        )
    }
}

/// The hover text naming the source the rows report on.
#[derive(Debug, Clone, Copy)]
pub struct FetchRowHoverText {
    pub queue: &'static str,
    pub coverage: &'static str,
}

pub const FETCH_QUEUE_LABEL: &str = "Fetch queue";
pub const RECORDING_DAYS_LABEL: &str = "Recording days";

/// Two rows of a data source page's grid: what is being fetched, and what the
/// archive holds for the loaded recordings.
pub fn show_fetch_rows(ui: &mut Ui, status: DayFetchStatus, hover: FetchRowHoverText) {
    ui.label(FETCH_QUEUE_LABEL).on_hover_text(hover.queue);
    ui.label(status.queue_line()).on_hover_text(hover.queue);
    ui.end_row();

    ui.label(RECORDING_DAYS_LABEL).on_hover_text(hover.coverage);
    ui.label(status.coverage_line())
        .on_hover_text(hover.coverage);
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
    fn the_coverage_line_counts_the_archived_days_of_loaded_recordings(
        #[case] recording_days: usize,
        #[case] archived_recording_days: usize,
        #[case] expected: &str,
    ) {
        let status = DayFetchStatus {
            recording_days,
            archived_recording_days,
            ..DayFetchStatus::default()
        };
        assert_eq!(status.coverage_line(), expected);
    }

    /// A backfill cannot report coverage the loaded recordings do not have:
    /// a day none of them spans stays out of the count.
    #[test]
    fn archiving_a_day_outside_the_loaded_recordings_changes_no_count() {
        let mut coverage = RecordingDayCoverage::default();
        coverage.record(day(2026, 7, 20), RecordingDayArchiveState::Awaited);

        coverage.mark_archived(day(2026, 7, 25));

        assert_eq!(coverage.recording_days(), 1);
        assert_eq!(coverage.archived_recording_days(), 0);
    }
}
