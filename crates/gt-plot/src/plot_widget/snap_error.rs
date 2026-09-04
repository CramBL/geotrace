//! The snap error series: per-run mipmap cascades, the snapped-point
//! anchor markers, the unsnapped crosses, and their caches.

use chrono::DateTime;
use egui_plot::{MarkerShape, PlotPoint, PlotPoints, Points};
use gt_egui_mipmap::{LevelSelection, MipMap};
use gt_types::{MetricKind, TrackRef};
use gt_ui_types::{ArcIdentity, SnapErrorKind, SnapErrorPoint, SnapErrorSeries};
use rustc_hash::FxHashMap;

use super::chips::MetricKindUi;
use super::levels::LineViewport;
use super::lines::{
    ANOMALY_HOVER_RADIUS_PX, ANOMALY_MARKER_RADIUS, LineStroke, NearestHoverLabel, PlotHoverLabel,
    add_line, nearest_fix_under_pointer, visible_by_x,
};
use crate::series::PlacedTrackSeries;

/// Whether any visible track has an entry in the snap error series.
pub(super) fn snap_error_available(
    series_cache: &[PlacedTrackSeries],
    visible: &[bool],
    snap_error: &SnapErrorSeries,
) -> bool {
    series_cache
        .iter()
        .zip(visible.iter())
        .any(|(series, &is_vis)| {
            is_vis && snap_error.points_by_track.contains_key(&series.track_ref())
        })
}

/// Plot y of an unsnapped point's marker: rejected points have no error value.
const UNSNAPPED_MARKER_Y: f64 = 0.0;

/// Radius of the snapped-point markers on the snap error line. Small - the
/// markers annotate the line's anchor points, they are not anomaly flags.
const SNAPPED_MARKER_RADIUS: f32 = 2.5;

/// Per-track plot-side cache of a snap error series: the line runs as
/// mipmap cascades (downsampled like every other metric), plus the raw
/// per-kind point lists for the marker overlays. Rebuilt only when the
/// track's series [`Arc`] changes - the app hands out one `Arc` per
/// completed run, so this rebuilds once per run, not per frame.
#[derive(Debug, Clone)]
pub(crate) struct SnapErrorPlotCache {
    /// Identity of the source series, for invalidation.
    source: ArcIdentity,
    /// One cascade per drawable line run, i.e. per maximal valued stretch (see
    /// [`snap_error_runs`]).
    runs: Vec<MipMap>,
    /// Snapped-kind points, ascending by x - the anchor markers.
    snapped: Vec<PlotPoint>,
    /// Unsnapped points at the baseline, ascending by x.
    unsnapped: Vec<PlotPoint>,
}

/// Bring the per-track snap caches in line with the frame's series: drop
/// tracks that left the series, (re)build entries whose source changed.
pub(super) fn sync_snap_error_cache(
    cache: &mut FxHashMap<TrackRef, SnapErrorPlotCache>,
    series: &SnapErrorSeries,
) {
    cache.retain(|track, _| series.points_by_track.contains_key(track));
    for (&track, points) in &series.points_by_track {
        let source = ArcIdentity::of(points);
        if cache.get(&track).is_some_and(|c| c.source == source) {
            continue;
        }
        let runs = snap_error_runs(points)
            .into_iter()
            .map(|run| MipMap::build(run.iter().map(|p| [p.x, p.y]).collect()))
            .collect();
        let snapped = points
            .iter()
            .filter(|p| p.kind == SnapErrorKind::Snapped)
            .filter_map(|p| p.error_m.map(|e| PlotPoint::new(p.x_secs, e)))
            .collect();
        let unsnapped = points
            .iter()
            .filter(|p| p.kind == SnapErrorKind::Unsnapped)
            .map(|p| PlotPoint::new(p.x_secs, UNSNAPPED_MARKER_Y))
            .collect();
        cache.insert(
            track,
            SnapErrorPlotCache {
                source,
                runs,
                snapped,
                unsnapped,
            },
        );
    }
}

/// The line's stroke, plus the theme the unsnapped crosses take their own
/// color from.
#[derive(Clone, Copy)]
pub(super) struct SnapErrorStyle {
    pub(super) stroke: LineStroke,
    pub(super) dark_mode: bool,
}

/// Pre-formatted tooltip contents for one hovered unsnapped marker, the only
/// snap point with a custom hover. Snapped and interpolated points hover
/// natively through egui_plot's labels.
pub(super) struct SnapErrorHover {
    /// Track label, shown only when more than one track is visible.
    track: Option<String>,
    time: String,
}

impl SnapErrorHover {
    fn new(track_label: Option<&str>, x_secs: f64) -> Self {
        let time = DateTime::from_timestamp(x_secs as i64, 0)
            .map(|dt| dt.format("%H:%M:%S").to_string())
            .unwrap_or_default();
        Self {
            track: track_label.map(ToOwned::to_owned),
            time,
        }
    }

    pub(super) fn show(&self, ui: &mut egui::Ui) {
        ui.strong("Unsnapped");
        if let Some(track) = &self.track {
            ui.label(track);
        }
        ui.label(&self.time);
        ui.label("The road network rejected this point");
    }
}

/// The drawable line runs of a snap error series: maximal stretches of
/// consecutive valued points, split wherever a point carries no error (the
/// road network rejected it) and wherever a point follows a gap in the run
/// (the receiver was dead reckoning there, or a chunk failed), so the line
/// never spans data the run does not have. Runs of a single point have
/// no visible line geometry and are dropped - the point's value stays
/// reachable through the custom hover.
fn snap_error_runs(points: &[SnapErrorPoint]) -> Vec<Vec<PlotPoint>> {
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
        if point.follows_gap {
            flush(&mut run);
        }
        match point.error_m {
            Some(error) => run.push(PlotPoint::new(point.x_secs, error)),
            None => flush(&mut run),
        }
    }
    flush(&mut run);
    runs
}

/// Whether every run the viewport shows reads its finest level. The anchor
/// markers only draw then - coarser levels merge points, so a marker would no
/// longer name a real point. A run outside the viewport has zero visible
/// width, which forces a coarse level, and does not veto the markers.
fn every_shown_run_is_at_full_detail(
    runs: &[MipMap],
    selections: &[LevelSelection],
    viewport: LineViewport,
) -> bool {
    runs.iter()
        .zip(selections)
        .filter(|(run, _)| viewport.shows(run))
        .all(|(_, selection)| selection.is_full_detail())
}

/// Draw one track's snap error series from its plot cache: mipmapped line runs
/// split at unsnapped points, snapped-point anchor markers while zoomed to full
/// detail, and a baseline cross per unsnapped point.
///
/// Only the unsnapped crosses keep a custom tooltip. The line and the anchor
/// markers hover natively through egui_plot's name/time/value label.
#[expect(
    clippy::too_many_arguments,
    reason = "matches add_series_lines' argument list. A struct would only relabel it"
)]
pub(super) fn add_snap_error_series<'a>(
    plot_ui: &mut egui_plot::PlotUi<'a>,
    prefix: &str,
    track_label: Option<&str>,
    cache: &'a SnapErrorPlotCache,
    viewport: LineViewport,
    pointer: Option<egui::Pos2>,
    nearest: &mut NearestHoverLabel,
    style: SnapErrorStyle,
) {
    let selections = viewport.select_run_levels(&cache.runs);
    let full_detail = every_shown_run_is_at_full_detail(&cache.runs, &selections, viewport);
    for (run, selection) in cache.runs.iter().zip(selections) {
        add_line(
            plot_ui,
            run.slice_at(selection),
            format!("{prefix}{}", MetricKind::SnapError.label()),
            style.stroke,
        );
    }

    if full_detail && !cache.snapped.is_empty() {
        let visible = visible_by_x(&cache.snapped, |p| p.x, viewport.x_min, viewport.x_max);
        if !visible.is_empty() {
            plot_ui.points(
                Points::new(
                    format!("{prefix}{} - snapped", MetricKind::SnapError.label()),
                    PlotPoints::Borrowed(visible),
                )
                .shape(MarkerShape::Circle)
                .color(style.stroke.color)
                .radius(SNAPPED_MARKER_RADIUS)
                .highlight(style.stroke.highlighted),
            );
        }
    }

    let visible_unsnapped = visible_by_x(&cache.unsnapped, |p| p.x, viewport.x_min, viewport.x_max);
    if !visible_unsnapped.is_empty() {
        plot_ui.points(
            Points::new("Unsnapped points", PlotPoints::Borrowed(visible_unsnapped))
                .shape(MarkerShape::Cross)
                .color(gt_ui_theme::error_indicator(style.dark_mode))
                .radius(ANOMALY_MARKER_RADIUS)
                .allow_hover(false),
        );
    }

    let Some(pointer) = pointer else {
        return;
    };
    if let Some((distance, point)) = nearest_fix_under_pointer(
        plot_ui,
        visible_unsnapped,
        |point| *point,
        pointer,
        ANOMALY_HOVER_RADIUS_PX,
    ) {
        nearest.offer(distance, || {
            PlotHoverLabel::SnapError(SnapErrorHover::new(track_label, point.x))
        });
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use gt_types::{FileIdx, TrackIdx};

    use super::super::FilterTimeWindow;
    use super::*;

    /// A viewport over `0..=x_max` seconds, `width` px wide, with a per-track
    /// sample cap of `cap`.
    fn viewport(x_max: f64, width: f32, cap: usize) -> LineViewport {
        LineViewport {
            x_min: 0.0,
            x_max,
            time_window: FilterTimeWindow::default(),
            width,
            cap,
        }
    }

    /// A run cascade over `count` one-per-second points starting at
    /// `start` seconds.
    fn run_from(start: usize, count: usize) -> MipMap {
        MipMap::build((0..count).map(|i| [(start + i) as f64, 1.0]).collect())
    }

    /// A run cascade over `count` one-per-second points from time zero.
    fn run_of(count: usize) -> MipMap {
        run_from(0, count)
    }

    /// The anchor-marker gate: dots draw only while every viewport-visible
    /// run reads its finest mipmap level. A downsampled run vetoes. A run
    /// entirely outside the viewport neither draws nor vetoes. No runs at all
    /// leave the gate open.
    #[rstest::rstest]
    #[case::no_runs(vec![], viewport(100.0, 800.0, 4096), true)]
    #[case::single_run_at_full_detail(vec![run_of(100)], viewport(100.0, 800.0, 4096), true)]
    #[case::single_run_downsampled(vec![run_of(4096)], viewport(4096.0, 100.0, 64), false)]
    #[case::downsampled_run_vetoes_the_full_one(
        vec![run_of(100), run_of(4096)],
        viewport(4096.0, 100.0, 64),
        false
    )]
    #[case::off_viewport_run_does_not_veto(
        vec![run_of(64), run_from(100_000, 4096)],
        viewport(100.0, 800.0, 4096),
        true
    )]
    fn marker_gate_requires_full_detail_on_every_visible_run(
        #[case] runs: Vec<MipMap>,
        #[case] viewport: LineViewport,
        #[case] expected: bool,
    ) {
        let selections = viewport.select_run_levels(&runs);
        assert_eq!(selections.len(), runs.len(), "one selection per run");
        assert_eq!(
            every_shown_run_is_at_full_detail(&runs, &selections, viewport),
            expected
        );
    }

    /// The plot-side snap cache follows the series by `Arc` identity: an
    /// unchanged `Arc` is reused, a replaced one rebuilds its entry, and a
    /// track that left the series is pruned.
    #[test]
    fn snap_cache_syncs_by_arc_identity() {
        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let points = Arc::new(vec![
            sep(0.0, Some(1.0)),
            sep(1.0, Some(2.0)),
            sep(2.0, None),
            sep(3.0, Some(3.0)),
            sep(4.0, Some(4.0)),
        ]);
        let mut series = SnapErrorSeries::default();
        series.points_by_track.insert(track, Arc::clone(&points));

        let mut cache = FxHashMap::default();
        sync_snap_error_cache(&mut cache, &series);
        let entry = cache.get(&track).expect("entry built");
        assert_eq!(entry.runs.len(), 2, "one cascade per line run");
        assert_eq!(entry.snapped.len(), 4);
        assert_eq!(entry.unsnapped.len(), 1);
        let built_source = entry.source;

        // Same Arc: the entry is reused, not rebuilt.
        sync_snap_error_cache(&mut cache, &series);
        assert_eq!(
            cache.get(&track).map(|e| e.source),
            Some(built_source),
            "an unchanged series keeps its cache entry"
        );

        // A new run (new Arc) rebuilds. A removed track prunes.
        series.points_by_track.insert(
            track,
            Arc::new(vec![sep(0.0, Some(1.0)), sep(1.0, Some(2.0))]),
        );
        sync_snap_error_cache(&mut cache, &series);
        assert_ne!(cache.get(&track).map(|e| e.source), Some(built_source));
        assert_eq!(cache.get(&track).map(|e| e.runs.len()), Some(1));

        series.points_by_track.clear();
        sync_snap_error_cache(&mut cache, &series);
        assert!(cache.is_empty(), "tracks that left the series are pruned");
    }

    /// Shorthand for a snap error point at time `x` with error `error_m`.
    fn sep(x: f64, error_m: Option<f64>) -> SnapErrorPoint {
        SnapErrorPoint {
            x_secs: x,
            error_m,
            kind: if error_m.is_some() {
                SnapErrorKind::Snapped
            } else {
                SnapErrorKind::Unsnapped
            },
            follows_gap: false,
        }
    }

    /// A point that follows a gap in the run - the plan dropped a ghost
    /// stretch there, or a chunk failed - starts a new line run, so the
    /// plot never draws an error trend across a stretch with no data.
    #[test]
    fn snap_error_runs_split_at_a_gap() {
        let mut points = vec![
            sep(0.0, Some(1.0)),
            sep(1.0, Some(2.0)),
            sep(2.0, Some(3.0)),
            sep(3.0, Some(4.0)),
        ];
        if let Some(after_gap) = points.get_mut(2) {
            after_gap.follows_gap = true;
        }
        let lengths: Vec<usize> = snap_error_runs(&points).iter().map(Vec::len).collect();
        assert_eq!(lengths, vec![2, 2]);
    }

    /// The line runs split exactly at valueless points, and runs of a single
    /// point (leading, interior, or trailing) are dropped - one point has
    /// no line to draw and would clutter the legend.
    #[rstest::rstest]
    #[case::empty(&[], &[])]
    #[case::one_unbroken_run(&[(0.0, Some(1.0)), (1.0, Some(2.0)), (2.0, Some(3.0))], &[3])]
    #[case::interior_break(
        &[(0.0, Some(1.0)), (1.0, Some(2.0)), (2.0, None), (3.0, Some(4.0)), (4.0, Some(5.0))],
        &[2, 2]
    )]
    #[case::leading_single_point_dropped(
        &[(0.0, Some(1.0)), (1.0, None), (2.0, Some(3.0)), (3.0, Some(4.0))],
        &[2]
    )]
    #[case::trailing_single_point_dropped(
        &[(0.0, Some(1.0)), (1.0, Some(2.0)), (2.0, None), (3.0, Some(4.0))],
        &[2]
    )]
    #[case::all_unsnapped(&[(0.0, None), (1.0, None)], &[])]
    fn snap_error_runs_split_at_valueless_points(
        #[case] input: &[(f64, Option<f64>)],
        #[case] expected_run_lengths: &[usize],
    ) {
        let points: Vec<SnapErrorPoint> = input.iter().map(|&(x, e)| sep(x, e)).collect();
        let runs = snap_error_runs(&points);
        let lengths: Vec<usize> = runs.iter().map(Vec::len).collect();
        assert_eq!(lengths, expected_run_lengths);
        // Every emitted vertex carries its point's own x and error value.
        for run in &runs {
            for vertex in run {
                // The test x values are small exact-in-f64 literals, so
                // bit-equality is the right lookup here.
                let source = points
                    .iter()
                    .find(|p| p.x_secs.to_bits() == vertex.x.to_bits())
                    .expect("vertex maps to a source point");
                assert_eq!(source.error_m, Some(vertex.y));
            }
        }
    }
}
