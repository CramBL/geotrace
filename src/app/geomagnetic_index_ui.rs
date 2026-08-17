//! The fetch status rows of the settings dialog's "Geomagnetic indices"
//! section.
//!
//! Renders without reaching the archive or the host: the scheduler fills
//! [`GeomagneticIndexFetchStatus`].

use chrono::NaiveDate;
use egui::Ui;
use gt_ui_theme::EM_DASH;

const QUEUE_HOVER: &str = "Index days waiting to be downloaded. One day is requested at a time, \
                           and one day costs one request per index.";

const COVERAGE_HOVER: &str = "UTC days the recordings loaded this session span, and how many of \
                              them the archive holds every published index for. Days downloaded \
                              by a backfill are not counted here.";

/// What the fetch worker is doing, and how far the archive covers the loaded
/// recordings.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct GeomagneticIndexFetchStatus {
    /// The day being fetched right now.
    pub fetching: Option<NaiveDate>,
    /// Days waiting behind it.
    pub queued: usize,
    /// UTC days the recordings loaded this session span, inside index
    /// coverage.
    pub recording_days: usize,
    /// Those of them the archive holds every published index for.
    pub archived_recording_days: usize,
}

impl GeomagneticIndexFetchStatus {
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

/// Two rows of the section's grid: what is being fetched, and what the
/// archive holds for the loaded recordings.
pub fn show_fetch_rows(ui: &mut Ui, status: GeomagneticIndexFetchStatus) {
    ui.label("Fetch queue").on_hover_text(QUEUE_HOVER);
    ui.label(status.queue_line()).on_hover_text(QUEUE_HOVER);
    ui.end_row();

    ui.label("Recording days").on_hover_text(COVERAGE_HOVER);
    ui.label(status.coverage_line())
        .on_hover_text(COVERAGE_HOVER);
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
        let status = GeomagneticIndexFetchStatus {
            fetching,
            queued,
            ..GeomagneticIndexFetchStatus::default()
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
        let status = GeomagneticIndexFetchStatus {
            recording_days,
            archived_recording_days,
            ..GeomagneticIndexFetchStatus::default()
        };
        assert_eq!(status.coverage_line(), expected);
    }
}
