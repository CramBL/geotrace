//! Mipmap level selection: the per-frame sample budget and the cached
//! per-track level choices every metric line draws from.

use chrono::{DateTime, Utc};
use gt_analysis::clock_offset::{ClockOffsetPlacement, ExcursionSample};
use gt_egui_mipmap::{IndexedMipMap, LevelSelection, MipMap, SelectionRange};
use gt_types::{LoadedTrack, MetricKind};
use rustc_hash::FxHashMap;

use super::FilterTimeWindow;
use crate::series::{FixMetricRun, TrackSeries};

/// Full-resolution sample target for a single track filling the plot width:
/// ~2 samples per pixel, floored so a very narrow plot still has usable detail.
pub(super) fn single_target(available_width: f32) -> usize {
    #[expect(
        clippy::cast_sign_loss,
        reason = "available_width is always ≥ 0 in practice; .max(0.0) makes it explicit"
    )]
    let px = available_width.max(0.0) as usize;
    (px * 2).max(400)
}

/// Upper bound on any single track's sample target.
///
/// Tracks that overlap in time all span the full plot width, so without a cap
/// N of them would each request [`single_target`] points.  Sharing a budget of
/// `single × BUDGET_TRACK_MULTIPLE` across the visible tracks bounds the total
/// handed to egui_plot in that worst case.  Tracks that occupy only part of the
/// width get far less via [`track_target`].  This cap only bites when many tracks
/// pile up in the same time range.
pub(super) fn budget_cap(available_width: f32, visible_count: usize) -> usize {
    let single = single_target(available_width);
    let count = visible_count.max(1);
    (single.saturating_mul(BUDGET_TRACK_MULTIPLE) / count).clamp(2, single)
}

/// Sample target for one track: ~2 points per pixel of the track's *visible*
/// width within the current view, capped by `cap` and floored at 2 (a single
/// segment).
pub(super) fn track_target(
    x_range: Option<(f64, f64)>,
    x_min: f64,
    x_max: f64,
    available_width: f32,
    cap: usize,
) -> usize {
    let view = x_max - x_min;
    let Some((lo, hi)) = x_range else { return 2 };
    if !view.is_finite() || view <= 0.0 {
        return cap;
    }
    let visible = (hi.min(x_max) - lo.max(x_min)).max(0.0);
    let pixels = f64::from(available_width) * (visible / view);
    #[expect(
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation,
        reason = "pixels is finite and ≥ 0; truncating a ~2-per-pixel count to an integer is intended"
    )]
    let want = (2.0 * pixels) as usize;
    want.clamp(2, cap)
}

/// The viewport one line selects its mipmap levels against, shared by every
/// metric whose line is built from per-fix values.
///
/// `x_min` and `x_max` are the plot's visible x range intersected with
/// `time_window`, and every marker overlay clips itself against them.
#[derive(Debug, Clone, Copy)]
pub(super) struct LineViewport {
    pub(super) plot_x: [f64; 2],
    pub(super) x_min: f64,
    pub(super) x_max: f64,
    pub(super) time_window: FilterTimeWindow,
    pub(super) width: f32,
    pub(super) cap: usize,
}

impl LineViewport {
    pub(super) fn selection_range(self) -> SelectionRange {
        SelectionRange::within_viewport(self.x_min..=self.x_max)
            .bounded_by(self.time_window.hard_bound())
    }

    /// The level each run draws at, in the order the runs are given.
    pub(super) fn select_run_levels(self, runs: &[MipMap]) -> Vec<LevelSelection> {
        let range = self.selection_range();
        runs.iter()
            .map(|run| {
                let target =
                    track_target(run.x_range(), self.x_min, self.x_max, self.width, self.cap);
                run.select_indices(range, target)
            })
            .collect()
    }

    fn select_recorded(
        self,
        run: &IndexedMipMap,
        target: usize,
        sample_time: impl Fn(usize) -> Option<DateTime<Utc>>,
    ) -> LevelSelection {
        let [start, end] = self.plot_x;
        if self.time_window == FilterTimeWindow::default() {
            return run
                .geometry()
                .select_indices(SelectionRange::within_viewport(start..=end), target);
        }
        run.select_indices(start..=end, target, |index| {
            self.time_window.compare_sample(sample_time(index))
        })
    }

    fn target_for(self, run: &IndexedMipMap) -> usize {
        track_target(
            run.geometry().x_range(),
            self.x_min,
            self.x_max,
            self.width,
            self.cap,
        )
    }

    /// Whether any of `run` lies inside the viewport.
    ///
    /// A run outside the viewport still selects the points around it, so slice
    /// emptiness cannot identify one.
    pub(super) fn shows(self, run: &MipMap) -> bool {
        run.x_range()
            .is_some_and(|(lo, hi)| lo <= self.x_max && hi >= self.x_min)
    }

    pub(super) fn shows_recorded(self, run: &IndexedMipMap) -> bool {
        self.intersects_plot(run.geometry().x_range())
    }

    fn intersects_plot(self, x_range: Option<(f64, f64)>) -> bool {
        let [start, end] = self.plot_x;
        x_range.is_some_and(|(lo, hi)| lo <= end && hi >= start)
    }

    pub(super) fn visible_fix_runs<'runs>(
        self,
        runs: &'runs [FixMetricRun],
        levels: &[FixRunLevel],
    ) -> impl Iterator<Item = (&'runs FixMetricRun, LevelSelection)> {
        runs.iter()
            .zip(levels)
            .filter(move |(_, level)| self.intersects_plot(level.filtered_extent))
            .map(|(run, level)| (run, level.selection))
    }
}

/// The mipmap level each chronological run of one channel component draws at,
/// in the component's run order.
type ComponentRunLevels = Vec<LevelSelection>;

/// Cached level selections for every metric of one track's series, plus one
/// per channel component (dynamic, hence no `Copy`).
#[derive(Debug, Clone, Default)]
pub(super) struct TrackLevelCache {
    pub(super) metrics: FxHashMap<MetricKind, Vec<LevelSelection>>,
    pub(super) fixes: FxHashMap<MetricKind, Vec<FixRunLevel>>,
    /// One entry per channel of the track, then one per component of that
    /// channel.
    pub(super) channels: Vec<Vec<ComponentRunLevels>>,
}

#[derive(Debug, Clone)]
pub(super) struct FixRunLevel {
    selection: LevelSelection,
    filtered_extent: Option<(f64, f64)>,
}

impl TrackSeries {
    pub(super) fn has_samples_in(
        &self,
        track: &LoadedTrack,
        kind: MetricKind,
        window: FilterTimeWindow,
    ) -> bool {
        let fix_time = |index: usize| track.points.get(index).map(|point| point.tpv.time().utc());
        let available = if let Some(metric) = self.metrics.get(&kind) {
            metric.runs.iter().any(|run| {
                run.mipmap
                    .has_samples(|index| window.compare_sample(run.sample_time(track, index)))
            })
        } else {
            self.fix_mipmaps_for(kind).iter().any(|run| {
                run.mipmap
                    .has_samples(|index| window.compare_sample(fix_time(index)))
            })
        };
        if available {
            return true;
        }
        if kind == MetricKind::ClockDeltaMs {
            let bound = window.hard_bound();
            let samples_in_window = |samples: &[ExcursionSample]| {
                samples.iter().any(|sample| bound.contains(&sample.t))
            };
            let off_scale = match &self.clock_offset_placement {
                ClockOffsetPlacement::BaselineOffScale(baseline) => {
                    samples_in_window(&baseline.samples)
                }
                ClockOffsetPlacement::BaselineOnScale(excursions) => excursions
                    .iter()
                    .any(|excursion| samples_in_window(&excursion.samples)),
            };
            if off_scale {
                return true;
            }
        }
        false
    }

    pub(super) fn fix_mipmaps_for(&self, kind: MetricKind) -> &[FixMetricRun] {
        self.fix_metrics()
            .into_iter()
            .find_map(|(metric, runs)| (metric == kind).then_some(runs))
            .unwrap_or_default()
    }

    fn fix_metrics(&self) -> [(MetricKind, &[FixMetricRun]); 4] {
        [
            (MetricKind::Velocity, &self.velocity_kmh),
            (MetricKind::Eph, &self.eph_m),
            (MetricKind::HeadingDeg, &self.heading_deg),
            (MetricKind::ClockDeltaMs, &self.clock_delta_ms),
        ]
    }
}

/// Compute fresh level selections for all metrics of one track's series.
///
/// The sample target is derived per track from how many pixels the track
/// occupies in the current view ([`track_target`]), so a track that is only a
/// few pixels wide selects a coarse mipmap level with just a few points.
pub(super) fn compute_level_cache(
    series: &TrackSeries,
    track: &LoadedTrack,
    viewport: LineViewport,
) -> TrackLevelCache {
    let fixed_target = track_target(
        series.x_range,
        viewport.x_min,
        viewport.x_max,
        viewport.width,
        viewport.cap,
    );
    let fix_time = |index: usize| track.points.get(index).map(|point| point.tpv.time().utc());
    let fixed = series.fix_metrics().into_iter().map(|(kind, runs)| {
        let mut levels: Vec<_> = runs
            .iter()
            .map(|run| FixRunLevel {
                selection: LevelSelection::default(),
                filtered_extent: if viewport.time_window == FilterTimeWindow::default() {
                    run.mipmap.geometry().x_range()
                } else {
                    run.mipmap.clipped_x_range(|index| {
                        viewport.time_window.compare_sample(fix_time(index))
                    })
                },
            })
            .collect();
        let visible_count = levels
            .iter()
            .filter(|level| viewport.intersects_plot(level.filtered_extent))
            .count();
        let target = fixed_target.div_ceil(visible_count.max(1)).max(2);
        for (run, level) in runs.iter().zip(&mut levels) {
            level.selection = viewport.select_recorded(&run.mipmap, target, fix_time);
        }
        (kind, levels)
    });
    let dynamic = series.metrics.iter().map(|(kind, metric)| {
        let levels = metric
            .runs
            .iter()
            .map(|run| {
                viewport.select_recorded(&run.mipmap, viewport.target_for(&run.mipmap), |index| {
                    run.sample_time(track, index)
                })
            })
            .collect();
        (*kind, levels)
    });
    TrackLevelCache {
        metrics: dynamic.collect(),
        fixes: fixed.collect(),
        channels: series
            .channels
            .iter()
            .zip(&track.channels)
            .map(|(channel, source)| {
                channel
                    .components
                    .iter()
                    .map(|component| {
                        component
                            .runs
                            .iter()
                            .map(|run| {
                                viewport.select_recorded(run, viewport.target_for(run), |index| {
                                    source.times.get(index).copied()
                                })
                            })
                            .collect()
                    })
                    .collect()
            })
            .collect(),
    }
}

/// Overlap budget expressed as a multiple of the single-track target
/// (`≈ 2 × plot_width_px`).  Tracks that overlap in time each span the full
/// width.  This many of them can do so at full resolution before [`budget_cap`]
/// starts sharing the budget between them.  See [`budget_cap`].
const BUDGET_TRACK_MULTIPLE: usize = 8;

#[cfg(test)]
mod tests {
    use std::ops::RangeInclusive;

    use chrono::{DateTime, TimeDelta, Utc};
    use gt_filter::GlobalFilter;
    use gt_types::coordinates::{Latitude, Longitude};
    use gt_types::time_types::{GpsTime, SysTime};
    use gt_types::tpv::TimePositionVelocity;
    use gt_types::{FileSource, LoadedFile, NavPoint};
    use rustc_hash::FxHashMap;

    use super::*;
    use crate::AnalysisConfig;

    fn at_second(offset: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(FIRST_FIX_SECS + offset, 0).expect("a valid timestamp")
    }

    /// A recording of one track over `points`.
    fn recording(points: Vec<NavPoint>) -> LoadedFile {
        LoadedFile {
            metadata: gt_test_utils::empty_file_metadata(),
            tracks: vec![gt_test_utils::loaded_track_with_points(points)],
            event_marker_styles: FxHashMap::default(),
            orphaned_event_markers: Vec::new(),
            source: FileSource::GtdBytes([].into()),
            load_warnings: Vec::new(),
        }
    }

    /// The x values the velocity line of a plot over `points` draws from, in
    /// seconds from the first fix, under a time `window` given in the same
    /// seconds.
    fn drawn_seconds(points: Vec<NavPoint>, window: RangeInclusive<i64>) -> Vec<i64> {
        let files = [recording(points)];
        let placed = crate::series::build_all_series(&files, AnalysisConfig::default());
        let [placed] = placed.as_slice() else {
            panic!("one track builds one series");
        };
        let (x_min, x_max) = (
            at_second(*window.start()).timestamp() as f64,
            at_second(*window.end()).timestamp() as f64,
        );
        let cache = compute_level_cache(
            &placed.series,
            placed.track_ref().resolve(&files).unwrap(),
            LineViewport {
                plot_x: [x_min, x_max],
                x_min,
                x_max,
                time_window: FilterTimeWindow::from(&GlobalFilter::default().with_time_bounds(
                    Some(at_second(*window.start())),
                    Some(at_second(*window.end())),
                )),
                width: PLOT_WIDTH_PX,
                cap: single_target(PLOT_WIDTH_PX),
            },
        );
        placed
            .series
            .fix_mipmaps_for(MetricKind::Velocity)
            .iter()
            .zip(&cache.fixes[&MetricKind::Velocity])
            .flat_map(|(run, level)| run.mipmap.geometry().slice_at(level.selection))
            .map(|point| point.x as i64 - FIRST_FIX_SECS)
            .collect()
    }

    /// `gt-map` draws exactly the fixes the window keeps, and the plot's line
    /// draws the same fixes.
    #[test]
    fn a_fix_outside_the_time_window_is_not_drawn() {
        let points = gt_test_utils::fixtures::nav_points_from(at_second(0), 10, 1);
        assert_eq!(drawn_seconds(points, 3..=5), [3, 4, 5]);
    }

    #[test]
    fn a_backward_fix_time_step_keeps_the_two_recorded_plot_runs() {
        let points = [0, 2, 1, 3]
            .into_iter()
            .enumerate()
            .map(|(index, second)| {
                let time = at_second(second);
                NavPoint::new(
                    TimePositionVelocity::builder()
                        .time(GpsTime::from_utc(time))
                        .sys_time(SysTime::from_utc(
                            time - TimeDelta::milliseconds(index as i64),
                        ))
                        .lat(Latitude::new(55.0))
                        .lon(Longitude::new(12.0))
                        .eph_m(index as f32)
                        .build(),
                    None,
                )
            })
            .collect();
        let files = [recording(points)];
        let placed = crate::series::build_all_series(&files, AnalysisConfig::default());
        let placed = placed.first().unwrap();
        let series = &placed.series;
        let filter =
            GlobalFilter::default().with_time_bounds(Some(at_second(1)), Some(at_second(3)));
        let x_min = FIRST_FIX_SECS as f64;
        let x_max = x_min + 3.0;
        let viewport = LineViewport {
            plot_x: [x_min, x_max],
            x_min,
            x_max,
            time_window: FilterTimeWindow::from(&filter),
            width: PLOT_WIDTH_PX,
            cap: single_target(PLOT_WIDTH_PX),
        };
        let cache = compute_level_cache(
            series,
            placed.track_ref().resolve(&files).unwrap(),
            viewport,
        );
        for kind in [MetricKind::Eph, MetricKind::ClockDeltaMs] {
            let selected: Vec<Vec<(i64, f64)>> = series
                .fix_mipmaps_for(kind)
                .iter()
                .zip(&cache.fixes[&kind])
                .map(|(run, level)| {
                    run.mipmap
                        .geometry()
                        .slice_at(level.selection)
                        .iter()
                        .map(|point| (point.x as i64 - FIRST_FIX_SECS, point.y))
                        .collect()
                })
                .collect();
            assert_eq!(selected, [vec![(2, 1.0)], vec![(1, 2.0), (3, 3.0)]]);
        }
    }

    #[rstest::rstest]
    #[case::disjoint_geometry(999, None)]
    #[case::disjoint_filtered_geometry(0, Some(10))]
    fn offscreen_fix_runs_leave_the_visible_run_its_full_sample_target(
        #[case] offscreen_run_start: i64,
        #[case] lower_time_bound: Option<i64>,
    ) {
        let visible_start = 10;
        let visible_count = 128;
        let offscreen_runs = 32;
        let mut points = Vec::new();
        for _ in 0..offscreen_runs {
            points.extend(gt_test_utils::fixtures::nav_points_from(
                at_second(offscreen_run_start),
                2,
                1_000 - offscreen_run_start,
            ));
        }
        points.extend(gt_test_utils::fixtures::nav_points_from(
            at_second(visible_start),
            visible_count,
            1,
        ));
        let files = [recording(points)];
        let placed = crate::series::build_all_series(&files, AnalysisConfig::default());
        let placed = placed.first().unwrap();
        let track = placed.track_ref().resolve(&files).unwrap();
        let x_min = at_second(visible_start).timestamp() as f64;
        let x_max = x_min + (visible_count - 1) as f64;
        let filter =
            GlobalFilter::default().with_time_bounds(lower_time_bound.map(at_second), None);
        let viewport = LineViewport {
            plot_x: [x_min, x_max],
            x_min,
            x_max,
            time_window: FilterTimeWindow::from(&filter),
            width: PLOT_WIDTH_PX,
            cap: 32,
        };
        let cache = compute_level_cache(&placed.series, track, viewport);
        let kind = MetricKind::Velocity;
        let runs = placed.series.fix_mipmaps_for(kind);
        assert_eq!(runs.len(), offscreen_runs + 1);
        let drawn: Vec<_> = viewport
            .visible_fix_runs(runs, &cache.fixes[&kind])
            .collect();
        let [(run, selection)] = drawn.as_slice() else {
            panic!("one run intersects the viewport");
        };
        let samples = run.mipmap.geometry().slice_at(*selection);
        assert_eq!(samples.len(), viewport.cap);
        assert_eq!(samples.first().unwrap().x, x_min);
        assert_eq!(samples.last().unwrap().x, x_max);
    }

    #[rstest::rstest]
    #[case::an_equal_bound(0, &[0, 1])]
    #[case::a_later_bound(1, &[1])]
    fn a_time_filter_distinguishes_recorded_samples_with_the_same_plot_x(
        #[case] start_offset_ns: i64,
        #[case] expected_sources: &[usize],
    ) {
        let first = at_second(0);
        let last = first + TimeDelta::nanoseconds(2);
        let mut points = gt_test_utils::fixtures::nav_points_from(first, 1, 1);
        points.extend(gt_test_utils::fixtures::nav_points_from(last, 1, 1));
        for point in &mut points {
            point.navigation.used_count = Some(8);
        }
        let mut file = recording(points);
        file.tracks.first_mut().unwrap().channels = vec![gt_test_utils::fixtures::scalar_channel(
            "accel",
            None,
            vec![first, last],
            vec![1.0, 2.0],
        )];
        let files = [file];
        let track = files.first().unwrap().tracks.first().unwrap();
        let series = crate::series::build_all_series(&files, AnalysisConfig::default());
        let series = &series.first().unwrap().series;
        let start = first + TimeDelta::nanoseconds(start_offset_ns);
        let filter = GlobalFilter::default().with_time_bounds(Some(start), Some(last));
        let viewport = LineViewport {
            plot_x: [FIRST_FIX_SECS as f64; 2],
            x_min: FIRST_FIX_SECS as f64,
            x_max: FIRST_FIX_SECS as f64,
            time_window: FilterTimeWindow::from(&filter),
            width: PLOT_WIDTH_PX,
            cap: single_target(PLOT_WIDTH_PX),
        };
        let cache = compute_level_cache(series, track, viewport);
        let gps_count = &series.metrics[&MetricKind::SatsFix].runs[0].mipmap;
        let channel = &series.channels[0].components[0].runs[0];
        for (mipmap, selection) in [
            (
                &series.fix_mipmaps_for(MetricKind::Velocity)[0].mipmap,
                cache.fixes[&MetricKind::Velocity][0].selection,
            ),
            (gps_count, cache.metrics[&MetricKind::SatsFix][0]),
            (channel, cache.channels[0][0][0]),
        ] {
            let samples = mipmap.geometry().slice_at(selection);
            assert_eq!(samples.len(), expected_sources.len());
            for (offset, &expected) in expected_sources.iter().enumerate() {
                assert_eq!(mipmap.source_at(selection, offset), Some(expected));
            }
        }
    }

    #[test]
    fn a_window_between_equal_x_samples_has_no_metric_values() {
        let first = at_second(0);
        let window_time = first + TimeDelta::nanoseconds(1);
        let last = first + TimeDelta::nanoseconds(2);
        let mut points = gt_test_utils::fixtures::nav_points_from(first, 1, 1);
        points.extend(gt_test_utils::fixtures::nav_points_from(last, 1, 1));
        let files = [recording(points)];
        let track = files.first().unwrap().tracks.first().unwrap();
        let series = crate::series::build_all_series(&files, AnalysisConfig::default());
        let series = &series.first().unwrap().series;
        let filter = GlobalFilter::default().with_time_bounds(Some(window_time), Some(window_time));
        assert!(!series.has_samples_in(
            track,
            MetricKind::Velocity,
            FilterTimeWindow::from(&filter)
        ));
    }

    #[test]
    fn a_window_inside_a_recording_gap_draws_no_line() {
        let mut points = gt_test_utils::fixtures::nav_points_from(at_second(0), 4, 1);
        points.extend(gt_test_utils::fixtures::nav_points_from(
            at_second(600),
            4,
            1,
        ));
        assert_eq!(drawn_seconds(points, 300..=400), Vec::<i64>::new());
    }

    #[test]
    fn track_target_scales_with_visible_pixels() {
        let width = 1000.0;
        let cap = single_target(width);

        // A track spanning the whole view gets ~2 points per pixel (width is
        // 1000 px, so a full-width track should exceed that).
        let full = track_target(Some((0.0, 100.0)), 0.0, 100.0, width, cap);
        assert!(full > 1000, "full-width track should be ~2 pts/pixel");
        assert!(full <= cap);

        // A track occupying ~1% of the view (~10 px) hands over only a handful
        // of points.
        let tiny = track_target(Some((0.0, 1.0)), 0.0, 100.0, width, cap);
        assert!(tiny >= 2);
        assert!(
            tiny <= 32,
            "few-pixel track must hand over few points, got {tiny}"
        );

        // An empty track stays minimal.  A degenerate (zero-width) view never
        // divides by zero and falls back to the cap.
        assert_eq!(track_target(None, 0.0, 100.0, width, cap), 2);
        assert_eq!(track_target(Some((0.0, 1.0)), 5.0, 5.0, width, cap), cap);
    }

    #[test]
    fn budget_cap_bounds_overlapping_tracks() {
        let width = 1000.0;
        let single = single_target(width);
        let budget = single * BUDGET_TRACK_MULTIPLE;

        // Up to BUDGET_TRACK_MULTIPLE overlapping tracks keep full resolution.
        assert_eq!(budget_cap(width, 1), single);
        assert_eq!(budget_cap(width, BUDGET_TRACK_MULTIPLE), single);

        // Beyond that the cap shares the budget so total full-width points stay
        // bounded (allowing the integer-division remainder).
        for count in [BUDGET_TRACK_MULTIPLE + 1, 50, 500] {
            let cap = budget_cap(width, count);
            assert!((2..=single).contains(&cap));
            assert!(
                cap * count <= budget + count,
                "cap {cap} × {count} exceeds budget {budget}"
            );
        }

        // Zero visible count must not divide by zero.
        assert_eq!(budget_cap(width, 0), single);
    }

    #[test]
    fn count_runs_remain_separate_under_coarse_selection_and_window_clipping() {
        let mut points = gt_test_utils::fixtures::nav_points_from(at_second(0), 1_100, 1);
        for (index, point) in points.iter_mut().enumerate() {
            point.navigation.used_count = (!(500..600).contains(&index)).then_some(8);
        }
        let files = [recording(points)];
        let track = files.first().unwrap().tracks.first().unwrap();
        let placed = crate::series::build_all_series(&files, AnalysisConfig::default());
        let series = &placed.first().expect("one track").series;
        let base = FIRST_FIX_SECS as f64;
        let viewport = LineViewport {
            plot_x: [base, base + 1_099.0],
            x_min: base,
            x_max: base + 1_099.0,
            time_window: FilterTimeWindow::default(),
            width: 1.0,
            cap: 2,
        };
        let cache = compute_level_cache(series, track, viewport);
        let selections = cache
            .metrics
            .get(&MetricKind::SatsFix)
            .expect("count selections");
        let runs = &series
            .metrics
            .get(&MetricKind::SatsFix)
            .expect("count series")
            .runs;
        assert_eq!(selections.len(), 2);
        assert!(
            selections
                .iter()
                .all(|selection| !selection.is_full_detail())
        );
        for (index, (run, selection)) in runs.iter().zip(selections).enumerate() {
            let selected = run.mipmap.geometry().slice_at(*selection);
            assert!(!selected.is_empty());
            assert!(selected.iter().all(|sample| {
                if index == 0 {
                    sample.x < base + 500.0
                } else {
                    sample.x >= base + 600.0
                }
            }));
        }
        let in_gap = LineViewport {
            plot_x: [base + 500.0, base + 599.0],
            x_min: base + 500.0,
            x_max: base + 599.0,
            time_window: FilterTimeWindow::from(
                &GlobalFilter::default()
                    .with_time_bounds(Some(at_second(500)), Some(at_second(599))),
            ),
            ..viewport
        };
        let cache = compute_level_cache(series, track, in_gap);
        for (run, selection) in runs.iter().zip(
            cache
                .metrics
                .get(&MetricKind::SatsFix)
                .expect("count selections"),
        ) {
            assert!(run.mipmap.geometry().slice_at(*selection).is_empty());
            assert!(!in_gap.shows_recorded(&run.mipmap));
        }
    }

    /// Plot width in points. Wide enough that a window of a few seconds still
    /// reads the finest mipmap level.
    const PLOT_WIDTH_PX: f32 = 800.0;

    /// 2024-01-15 12:00:00 UTC, the first fix of every track.
    const FIRST_FIX_SECS: i64 = 1_705_320_000;
}
