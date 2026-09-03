//! The geomagnetic index chip gates and hover label.
//!
//! The lines themselves are drawn by [`super::context`] across the plot's
//! whole span. The chips are gated on the loaded recordings: one enables
//! when a visible track's own day is archived for that index.

use chrono::DateTime;
use gt_solar::GeomagneticIndex;
use gt_solar::activity::GeomagneticActivity;
use gt_types::TrackRef;
use gt_ui_types::{GeomagneticSeries, IndexContextSample};

/// Which index lines have values for the visible tracks, gating their chips.
#[derive(Debug, Clone, Copy)]
pub(super) struct GeomagneticAvailability {
    pub(super) hp30: bool,
    pub(super) kp: bool,
}

pub(super) fn geomagnetic_availability(
    visible_tracks: impl Iterator<Item = TrackRef>,
    series: &GeomagneticSeries,
) -> GeomagneticAvailability {
    let mut availability = GeomagneticAvailability {
        hp30: false,
        kp: false,
    };
    for track in visible_tracks {
        let Some(points) = series.points_by_track.get(&track) else {
            continue;
        };
        for point in points.iter() {
            availability.hp30 |= point.hp30.is_some();
            availability.kp |= point.kp.is_some();
            if availability.hp30 && availability.kp {
                return availability;
            }
        }
    }
    availability
}

/// The hovered period, worded by [`gt_solar::text::period_summary`] so the
/// plot says what the settings section says.
pub(super) struct GeomagneticHover {
    lines: Vec<String>,
}

impl GeomagneticHover {
    pub(super) fn of_archived_period(index: GeomagneticIndex, sample: IndexContextSample) -> Self {
        let period_start =
            DateTime::from_timestamp(sample.start_secs as i64, 0).unwrap_or_default();
        let activity = sample
            .value
            .and_then(|value| GeomagneticActivity::from_published_value(index, value));
        Self {
            lines: gt_solar::text::period_summary(index, activity, period_start),
        }
    }

    pub(super) fn show(&self, ui: &mut egui::Ui) {
        ui.strong(gt_solar::text::LAYER_LABEL);
        for line in &self.lines {
            ui.label(line);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gt_types::{FileIdx, TrackIdx};
    use gt_ui_types::GeomagneticPoint;

    use super::*;

    fn series_of(points: Vec<GeomagneticPoint>) -> (TrackRef, GeomagneticSeries) {
        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let mut series = GeomagneticSeries::default();
        series.points_by_track.insert(track, Arc::new(points));
        (track, series)
    }

    /// A track whose day archived Kp alone still offers the Kp chip, and
    /// leaves the Hp30 chip disabled.
    #[test]
    fn availability_is_reported_per_index() {
        let (track, series) = series_of(vec![GeomagneticPoint {
            x_secs: 0.0,
            hp30: None,
            kp: Some(2.667),
        }]);

        let availability = geomagnetic_availability([track].into_iter(), &series);
        assert!(availability.kp);
        assert!(!availability.hp30);
    }

    /// A track left out of the visible set enables neither chip.
    #[test]
    fn an_invisible_track_offers_no_values() {
        let (_, series) = series_of(vec![GeomagneticPoint {
            x_secs: 0.0,
            hp30: Some(3.0),
            kp: Some(2.667),
        }]);

        let availability = geomagnetic_availability(std::iter::empty(), &series);
        assert!(!availability.kp);
        assert!(!availability.hp30);
    }

    /// The hover label leads with the index and its value, then the storm
    /// class, then the period the value covers. Hp30 above 9 is a published
    /// value and stays one.
    #[test]
    fn the_hover_label_states_the_value_its_class_and_its_period() {
        let hover = GeomagneticHover::of_archived_period(
            GeomagneticIndex::Hp30,
            IndexContextSample {
                start_secs: 1_715_364_000.0,
                value: Some(11.333),
            },
        );
        assert_eq!(
            hover.lines,
            [
                "Hp30 11.333",
                "G5 extreme storm",
                "30min from 2024-05-10 18:00 UTC",
            ]
        );
    }
}
