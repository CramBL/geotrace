//! The aircraft-interference plot line.
//!
//! One value per fix: the share of aircraft over that fix's cell that
//! reported low navigation integrity, for the fix's own UTC day. The line
//! breaks wherever no value exists, the same treatment
//! [`super::snap_error`] gives an unsnapped point.

use std::collections::HashMap;

use egui_plot::PlotPoint;
use gt_egui_mipmap::{LevelSelection, MipMap};
use gt_types::{MetricKind, TrackRef};
use gt_ui_types::{ArcIdentity, JammingPoint, JammingSeries};

use crate::series::TrackSeries;

use super::chips::MetricKindUi;

use super::levels::track_target;
use super::lines::{NearestHoverLabel, PlotHoverLabel, add_line, series_track_ref, visible_by_x};

/// One track's interference line, rebuilt only when its source changes.
#[derive(Debug, Clone)]
pub(super) struct JammingPlotCache {
    source: ArcIdentity,
    /// Mipmapped line runs, split where the line breaks.
    runs: Vec<MipMap>,
    /// Every valued fix with its counts, ascending by x, for the hover.
    valued: Vec<(PlotPoint, Counts)>,
}

/// The counts behind one fix's share.
#[derive(Debug, Clone, Copy)]
struct Counts {
    aircraft: u32,
    bad: u32,
}

/// The hovered fix's counts, formatted with the same `gt_jam::text::cell_summary`
/// as the map's cell hover.
pub(super) struct JammingHover {
    /// Track label, shown only when more than one track is visible.
    track: Option<String>,
    lines: Vec<String>,
}

impl JammingHover {
    fn new(track_label: Option<&str>, point: PlotPoint, counts: Counts) -> Self {
        let day = chrono::DateTime::from_timestamp(point.x as i64, 0)
            .map(|time| time.date_naive().to_string())
            .unwrap_or_default();
        Self {
            track: track_label.map(ToOwned::to_owned),
            lines: gt_jam::text::cell_summary(
                &day,
                counts.aircraft.saturating_sub(counts.bad),
                counts.bad,
                point.y,
            ),
        }
    }

    pub(super) fn show(&self, ui: &mut egui::Ui) {
        ui.strong(MetricKind::Jamming.label());
        if let Some(track) = &self.track {
            ui.label(track);
        }
        for line in &self.lines {
            ui.label(line);
        }
    }
}

/// Whether any visible track has interference values, which gates the
/// metric's chip.
pub(super) fn jamming_available(
    series_cache: &[TrackSeries],
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
                .get(&series_track_ref(series_entry))
                .is_some_and(|points| points.iter().any(|point| point.percent.is_some()))
        })
}

/// Bring the per-track caches in line with the frame's series: drop tracks
/// that left it, rebuild entries whose source changed.
pub(super) fn sync_jamming_cache(
    cache: &mut HashMap<TrackRef, JammingPlotCache>,
    series: &JammingSeries,
) {
    cache.retain(|track, _| series.points_by_track.contains_key(track));
    for (&track, points) in &series.points_by_track {
        let source = ArcIdentity::of(points);
        if cache
            .get(&track)
            .is_some_and(|entry| entry.source == source)
        {
            continue;
        }
        let runs = jamming_runs(points)
            .into_iter()
            .map(|run| MipMap::build(run.iter().map(|point| [point.x, point.y]).collect()))
            .collect();
        let valued = points
            .iter()
            .filter_map(|point| {
                let percent = point.percent?;
                Some((
                    PlotPoint::new(point.x_secs, percent),
                    Counts {
                        aircraft: point.aircraft,
                        bad: point.bad,
                    },
                ))
            })
            .collect();
        cache.insert(
            track,
            JammingPlotCache {
                source,
                runs,
                valued,
            },
        );
    }
}

/// Maximal stretches of consecutive valued points. A fix whose day is not
/// archived has no value and breaks the line. Runs of one point draw nothing.
fn jamming_runs(points: &[JammingPoint]) -> Vec<Vec<PlotPoint>> {
    let mut runs = Vec::new();
    let mut run: Vec<PlotPoint> = Vec::new();
    let mut flush = |run: &mut Vec<PlotPoint>| {
        if run.len() >= 2 {
            runs.push(std::mem::take(run));
        } else {
            run.clear();
        }
    };
    for point in points {
        match point.percent {
            Some(percent) => run.push(PlotPoint::new(point.x_secs, percent)),
            None => flush(&mut run),
        }
    }
    flush(&mut run);
    runs
}

/// The viewport parameters the line selects its mipmap levels with.
#[derive(Debug, Clone, Copy)]
pub(super) struct JammingViewport {
    pub(super) x_min: f64,
    pub(super) x_max: f64,
    pub(super) width: f32,
    pub(super) cap: usize,
}

/// How the line is stroked, matching the other metrics' styling inputs.
#[derive(Debug, Clone, Copy)]
pub(super) struct JammingStyle {
    pub(super) color: egui::Color32,
    pub(super) style: egui_plot::LineStyle,
    pub(super) width: f32,
    pub(super) highlighted: bool,
}

fn select_run_levels(runs: &[MipMap], viewport: JammingViewport) -> Vec<LevelSelection> {
    runs.iter()
        .map(|run| {
            let target = track_target(
                run.x_range(),
                viewport.x_min,
                viewport.x_max,
                viewport.width,
                viewport.cap,
            );
            run.select_indices(viewport.x_min, viewport.x_max, target)
        })
        .collect()
}

/// Which track is being drawn, and where the pointer is.
#[derive(Clone, Copy)]
pub(super) struct JammingTrack<'a> {
    /// The recording's plot label, `None` while a single track is visible.
    pub(super) track_label: Option<&'a str>,
    pub(super) pointer: Option<egui::Pos2>,
}

/// Radius in pixels within which a fix is a hover target.
const HOVER_RADIUS_PX: f32 = 12.0;

/// Draw one track's interference line from its cache, and hit-test the
/// pointer against its fixes so the hover can report the counts the map's
/// cell hover reports.
pub(super) fn add_jamming_series<'a>(
    plot_ui: &mut egui_plot::PlotUi<'a>,
    prefix: &str,
    track: JammingTrack<'_>,
    cache: &'a JammingPlotCache,
    viewport: JammingViewport,
    style: JammingStyle,
    nearest: &mut NearestHoverLabel,
) {
    let JammingTrack {
        track_label,
        pointer,
    } = track;
    for (run, selection) in cache
        .runs
        .iter()
        .zip(select_run_levels(&cache.runs, viewport))
    {
        add_line(
            plot_ui,
            run.slice_at(selection),
            format!("{prefix}{}", MetricKind::Jamming.label()),
            style.color,
            style.style,
            style.width,
            style.highlighted,
        );
    }

    let Some(pointer) = pointer else {
        return;
    };
    let visible = visible_by_x(
        &cache.valued,
        |(point, _)| point.x,
        viewport.x_min,
        viewport.x_max,
    );
    for (point, counts) in visible {
        let distance = plot_ui.screen_from_plot(*point).distance(pointer);
        if distance <= HOVER_RADIUS_PX {
            nearest.offer(distance, || {
                PlotHoverLabel::Jamming(JammingHover::new(track_label, *point, *counts))
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn point(x_secs: f64, percent: Option<f64>) -> JammingPoint {
        JammingPoint {
            x_secs,
            percent,
            aircraft: if percent.is_some() { 100 } else { 0 },
            bad: if percent.is_some() { 10 } else { 0 },
        }
    }

    #[rstest::rstest]
    #[case::breaks_where_a_value_is_missing(
        &[Some(1.0), Some(2.0), None, Some(3.0), Some(4.0)],
        vec![2, 2]
    )]
    #[case::a_lone_valued_point_draws_nothing(&[None, Some(5.0), None], vec![])]
    #[case::no_values_at_all(&[None, None], vec![])]
    #[case::unbroken(&[Some(1.0), Some(2.0), Some(3.0)], vec![3])]
    #[case::trailing_break(&[Some(1.0), Some(2.0), None], vec![2])]
    fn runs_split_where_a_fix_has_no_value(
        #[case] percents: &[Option<f64>],
        #[case] expected_lengths: Vec<usize>,
    ) {
        let points: Vec<JammingPoint> = percents
            .iter()
            .enumerate()
            .map(|(index, &percent)| point(index as f64, percent))
            .collect();
        let lengths: Vec<usize> = jamming_runs(&points).iter().map(Vec::len).collect();
        assert_eq!(lengths, expected_lengths);
    }

    /// The cache keys on `Arc` identity: an unchanged series is reused, a
    /// replaced one rebuilds, and a track leaving the series is dropped.
    #[test]
    fn the_cache_syncs_by_arc_identity() {
        use std::sync::Arc;

        use gt_types::{FileIdx, TrackIdx};

        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let points = Arc::new(vec![point(0.0, Some(1.0)), point(1.0, Some(2.0))]);
        let mut series = JammingSeries::default();
        series.points_by_track.insert(track, Arc::clone(&points));

        let mut cache = HashMap::new();
        sync_jamming_cache(&mut cache, &series);
        let first = cache.get(&track).map(|entry| entry.source);
        assert!(first.is_some());

        // Same Arc: the entry is left alone.
        sync_jamming_cache(&mut cache, &series);
        assert_eq!(cache.get(&track).map(|entry| entry.source), first);

        // A new Arc with the same contents still rebuilds.
        series
            .points_by_track
            .insert(track, Arc::new(points.as_ref().clone()));
        sync_jamming_cache(&mut cache, &series);
        assert_ne!(cache.get(&track).map(|entry| entry.source), first);

        // A track that left the series is pruned.
        sync_jamming_cache(&mut cache, &JammingSeries::default());
        assert!(cache.is_empty());
    }
}
