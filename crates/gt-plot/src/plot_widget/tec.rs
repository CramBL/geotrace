//! The ionospheric TEC chip gate and hover label.
//!
//! The line itself is drawn by [`super::context`] across the plot's whole
//! span. The chip is gated on the loaded recordings: it enables when a
//! visible track's own day is archived.

use chrono::DateTime;
use egui_plot::PlotPoint;
use gt_ionex::tec::TotalElectronContent;
use gt_types::TrackRef;
use gt_ui_types::TecSeries;

use super::lines::HOVER_INSTANT_FORMAT;

/// Whether any visible track has TEC values, gating the chip.
pub(super) fn tec_available(
    visible_tracks: impl Iterator<Item = TrackRef>,
    series: &TecSeries,
) -> bool {
    visible_tracks
        .filter_map(|track| series.points_by_track.get(&track))
        .any(|points| points.iter().any(|point| point.tecu.is_some()))
}

/// The value under the pointer, worded by [`gt_ionex::text::value_summary`]
/// so the plot says what every other surface says.
pub(super) struct TecHover {
    lines: Vec<String>,
}

impl TecHover {
    /// The line's own value at the hovered instant, which runs between the
    /// two map epochs bracketing it.
    pub(super) fn of_line_point(point: PlotPoint) -> Self {
        let instant = DateTime::from_timestamp(point.x as i64, 0)
            .map(|time| time.format(HOVER_INSTANT_FORMAT).to_string())
            .unwrap_or_default();
        Self {
            lines: gt_ionex::text::value_summary(
                TotalElectronContent::from_tecu(point.y),
                &instant,
            ),
        }
    }

    pub(super) fn show(&self, ui: &mut egui::Ui) {
        ui.strong(gt_ionex::text::LAYER_LABEL);
        for line in &self.lines {
            ui.label(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gt_types::{FileIdx, TrackIdx};
    use gt_ui_types::TecPoint;

    use super::*;

    fn series_of(points: Vec<TecPoint>) -> (TrackRef, TecSeries) {
        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let mut series = TecSeries::default();
        series.points_by_track.insert(track, Arc::new(points));
        (track, series)
    }

    /// A track whose day is not archived leaves the chip disabled, and a
    /// track outside the visible set offers nothing either.
    #[test]
    fn availability_needs_a_valued_visible_track() {
        let (track, valued) = series_of(vec![TecPoint {
            x_secs: 0.0,
            tecu: Some(12.0),
        }]);
        let (_, unvalued) = series_of(vec![TecPoint {
            x_secs: 0.0,
            tecu: None,
        }]);

        assert!(tec_available([track].into_iter(), &valued));
        assert!(!tec_available([track].into_iter(), &unvalued));
        assert!(!tec_available(std::iter::empty(), &valued));
    }

    /// The hover label leads with the value, then the range it delays L1 by,
    /// then the instant it was interpolated at.
    #[test]
    fn the_hover_label_states_the_value_its_delay_and_its_instant() {
        let hover = TecHover::of_line_point(PlotPoint::new(1_715_364_000.0, 42.3));
        assert_eq!(
            hover.lines,
            [
                "TEC 42.3 TECU",
                "L1 delay about 6.9m",
                "Interpolated between maps at 2024-05-10T18:00:00 (UTC)",
            ]
        );
    }
}
