//! The ionospheric TEC plot line.
//!
//! One value per fix: the vertical total electron content over the fix's own
//! position and time, interpolated from the maps archived for its UTC day.
//! The line breaks wherever no value exists, the same treatment
//! [`super::jamming`] gives a fix whose day is not archived.

use std::collections::HashMap;

use chrono::DateTime;
use egui_plot::PlotPoint;
use gt_egui_mipmap::MipMap;
use gt_ionex::tec::TotalElectronContent;
use gt_types::{MetricKind, TrackRef};
use gt_ui_types::{ArcIdentity, TecSeries};

use super::chips::MetricKindUi;
use super::levels::LineViewport;
use super::lines::{
    HOVER_INSTANT_FORMAT, HOVER_RADIUS_PX, LineStroke, NearestHoverLabel, PlotHoverLabel, add_line,
    line_runs, nearest_fix_under_pointer, visible_by_x,
};

/// One track's TEC line, rebuilt only when its source changes.
#[derive(Debug, Clone)]
pub(super) struct TecPlotCache {
    source: ArcIdentity,
    /// The mipmapped runs the line draws, split where it breaks.
    runs: Vec<MipMap>,
    /// Every valued fix, ascending by x, as the hover hit-tests them.
    valued: Vec<PlotPoint>,
}

/// Whether any visible track has TEC values, gating the chip.
pub(super) fn tec_available(
    visible_tracks: impl Iterator<Item = TrackRef>,
    series: &TecSeries,
) -> bool {
    visible_tracks
        .filter_map(|track| series.points_by_track.get(&track))
        .any(|points| points.iter().any(|point| point.tecu.is_some()))
}

/// Bring the per-track caches in line with the frame's series: drop tracks
/// that left it, rebuild entries whose source changed.
pub(super) fn sync_tec_cache(cache: &mut HashMap<TrackRef, TecPlotCache>, series: &TecSeries) {
    cache.retain(|track, _| series.points_by_track.contains_key(track));
    for (&track, points) in &series.points_by_track {
        let source = ArcIdentity::of(points);
        if cache
            .get(&track)
            .is_some_and(|entry| entry.source == source)
        {
            continue;
        }
        let runs = line_runs(
            points
                .iter()
                .map(|point| point.tecu.map(|tecu| PlotPoint::new(point.x_secs, tecu))),
        )
        .into_iter()
        .map(|run| MipMap::build(run.iter().map(|point| [point.x, point.y]).collect()))
        .collect();
        let valued = points
            .iter()
            .filter_map(|point| Some(PlotPoint::new(point.x_secs, point.tecu?)))
            .collect();
        cache.insert(
            track,
            TecPlotCache {
                source,
                runs,
                valued,
            },
        );
    }
}

/// The hovered fix's value, worded by [`gt_ionex::text::fix_summary`] so the
/// plot says what every other surface says.
pub(super) struct TecHover {
    /// Track label, shown only when more than one track is visible.
    track: Option<String>,
    lines: Vec<String>,
}

impl TecHover {
    fn new(track_label: Option<&str>, point: PlotPoint) -> Self {
        let instant = DateTime::from_timestamp(point.x as i64, 0)
            .map(|time| time.format(HOVER_INSTANT_FORMAT).to_string())
            .unwrap_or_default();
        Self {
            track: track_label.map(ToOwned::to_owned),
            lines: gt_ionex::text::fix_summary(TotalElectronContent::from_tecu(point.y), &instant),
        }
    }

    pub(super) fn show(&self, ui: &mut egui::Ui) {
        ui.strong(gt_ionex::text::LAYER_LABEL);
        if let Some(track) = &self.track {
            ui.label(track);
        }
        for line in &self.lines {
            ui.label(line);
        }
    }
}

#[derive(Clone, Copy)]
pub(super) struct TecTrack<'a> {
    /// The recording's plot label, `None` while a single track is visible.
    pub(super) track_label: Option<&'a str>,
    pub(super) pointer: Option<egui::Pos2>,
}

/// Draw one track's TEC line from its mipmapped runs, and hit-test the pointer
/// against its valued fixes so the hover can report the value and its delay.
pub(super) fn add_tec_series<'a>(
    plot_ui: &mut egui_plot::PlotUi<'a>,
    prefix: &str,
    track: TecTrack<'_>,
    cache: &'a TecPlotCache,
    viewport: LineViewport,
    stroke: LineStroke,
    nearest: &mut NearestHoverLabel,
) {
    for (run, selection) in cache
        .runs
        .iter()
        .zip(viewport.select_run_levels(&cache.runs))
    {
        add_line(
            plot_ui,
            run.slice_at(selection),
            format!("{prefix}{}", MetricKind::Tec.label()),
            stroke,
        );
    }

    let Some(pointer) = track.pointer else {
        return;
    };
    let visible = visible_by_x(
        &cache.valued,
        |point| point.x,
        viewport.x_min,
        viewport.x_max,
    );
    if let Some((distance, &point)) =
        nearest_fix_under_pointer(plot_ui, visible, |&point| point, pointer, HOVER_RADIUS_PX)
    {
        nearest.offer(distance, || {
            PlotHoverLabel::Tec(TecHover::new(track.track_label, point))
        });
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

    fn point(x_secs: f64, tecu: Option<f64>) -> TecPoint {
        TecPoint { x_secs, tecu }
    }

    /// A fix without a value splits the line, and every valued fix is a hover
    /// target. A storm-day value is drawn as published, never clamped.
    #[test]
    fn a_fix_without_a_value_breaks_the_line() {
        let (track, series) = series_of(vec![
            point(0.0, Some(12.0)),
            point(1.0, Some(14.0)),
            point(2.0, None),
            point(3.0, Some(175.5)),
            point(4.0, Some(174.0)),
        ]);

        let mut cache = HashMap::new();
        sync_tec_cache(&mut cache, &series);
        let entry = cache.get(&track).expect("the track is cached");
        assert_eq!(
            entry.runs.iter().map(MipMap::x_range).collect::<Vec<_>>(),
            [Some((0.0, 1.0)), Some((3.0, 4.0))]
        );
        assert_eq!(
            entry.valued.iter().map(|point| point.y).collect::<Vec<_>>(),
            [12.0, 14.0, 175.5, 174.0]
        );
    }

    /// A track whose day is not archived leaves the chip disabled, and a
    /// track outside the visible set offers nothing either.
    #[test]
    fn availability_needs_a_valued_visible_track() {
        let (track, valued) = series_of(vec![point(0.0, Some(12.0))]);
        let (_, unvalued) = series_of(vec![point(0.0, None)]);

        assert!(tec_available([track].into_iter(), &valued));
        assert!(!tec_available([track].into_iter(), &unvalued));
        assert!(!tec_available(std::iter::empty(), &valued));
    }

    /// The hover label leads with the value, then the range it delays L1 by,
    /// then the instant it was interpolated at.
    #[test]
    fn the_hover_label_states_the_value_its_delay_and_its_instant() {
        let hover = TecHover::new(Some("morning drive"), PlotPoint::new(1_715_364_000.0, 42.3));
        assert_eq!(
            hover.lines,
            [
                "TEC 42.3 TECU",
                "L1 delay about 6.9 m",
                "Interpolated between maps at 2024-05-10T18:00:00 (UTC)",
            ]
        );
    }

    /// The cache keys on `Arc` identity: an unchanged series is reused, a
    /// replaced one rebuilds, and a track leaving the series is dropped.
    #[test]
    fn the_cache_syncs_by_arc_identity() {
        let (track, mut series) = series_of(vec![point(0.0, Some(12.0)), point(1.0, Some(14.0))]);

        let mut cache = HashMap::new();
        sync_tec_cache(&mut cache, &series);
        let first = cache.get(&track).map(|entry| entry.source);
        assert!(first.is_some());

        sync_tec_cache(&mut cache, &series);
        assert_eq!(cache.get(&track).map(|entry| entry.source), first);

        series
            .points_by_track
            .insert(track, Arc::new(vec![point(0.0, Some(12.0))]));
        sync_tec_cache(&mut cache, &series);
        assert_ne!(cache.get(&track).map(|entry| entry.source), first);

        sync_tec_cache(&mut cache, &TecSeries::default());
        assert!(cache.is_empty());
    }
}
