//! The aircraft-interference chip gate and hover label.
//!
//! The line itself is drawn by [`super::context`] across the plot's whole
//! span. The chip is gated on the loaded recordings: it enables when a
//! visible track's own day is archived.

use gt_types::MetricKind;
use gt_ui_types::{JammingContextSample, JammingSeries};

use crate::series::PlacedTrackSeries;

use super::chips::MetricKindUi;

/// Whether any visible track has interference values, which gates the
/// metric's chip.
pub(super) fn jamming_available(
    series_cache: &[PlacedTrackSeries],
    visible: &[bool],
    series: &JammingSeries,
) -> bool {
    series_cache
        .iter()
        .zip(visible)
        .filter(|&(_, &visible)| visible)
        .any(|(series_entry, _)| {
            series
                .points_by_track
                .get(&series_entry.track_ref())
                .is_some_and(|points| points.iter().any(|point| point.percent.is_some()))
        })
}

/// The hovered day's counts, formatted with the same
/// `gt_jam::text::cell_summary` as the map's cell hover.
pub(super) struct JammingHover {
    lines: Vec<String>,
}

impl JammingHover {
    pub(super) fn of_archived_day(sample: JammingContextSample) -> Self {
        let day = chrono::DateTime::from_timestamp(sample.start_secs as i64, 0)
            .map(|time| time.date_naive().to_string())
            .unwrap_or_default();
        Self {
            lines: gt_jam::text::cell_summary(
                &day,
                sample.aircraft.saturating_sub(sample.bad),
                sample.bad,
                sample.percent.unwrap_or_default(),
            ),
        }
    }

    pub(super) fn show(&self, ui: &mut egui::Ui) {
        ui.strong(MetricKind::Jamming.label());
        for line in &self.lines {
            ui.label(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The hover reads the day off the sample's own midnight, and leads with
    /// the counts behind the share.
    #[test]
    fn the_hover_label_states_the_counts_and_the_day() {
        let hover = JammingHover::of_archived_day(JammingContextSample {
            start_secs: 1_715_299_200.0,
            percent: Some(0.72),
            aircraft: 415,
            bad: 3,
        });
        assert_eq!(
            hover.lines,
            [
                "3 of 415 aircraft reported low navigation accuracy",
                "0.7 % over 2024-05-10 (UTC)",
            ]
        );
    }
}
