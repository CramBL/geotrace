//! Mipmap level selection: the per-frame sample budget and the cached
//! per-track level choices every metric line draws from.

use std::ops::RangeInclusive;

use gt_analysis::clock_offset::{ClockOffsetPlacement, ExcursionSample};
use gt_egui_mipmap::{LevelSelection, MipMap, SelectionRange};
use gt_types::MetricKind;
use rustc_hash::FxHashMap;

use super::FilterTimeWindow;
use crate::series::TrackSeries;

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
    pub(super) x_min: f64,
    pub(super) x_max: f64,
    /// The global filter's window, passed on as the hard bound of every level
    /// selection: a line ends at the window's edge.
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

    /// Whether any of `run` lies inside the viewport.
    ///
    /// A run outside the viewport still selects the points around it, so slice
    /// emptiness cannot identify one.
    pub(super) fn shows(self, run: &MipMap) -> bool {
        run.x_range()
            .is_some_and(|(lo, hi)| lo <= self.x_max && hi >= self.x_min)
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
    velocity_kmh: LevelSelection,
    eph_m: LevelSelection,
    heading_deg: LevelSelection,
    clock_delta_ms: LevelSelection,
    /// One entry per channel of the track, then one per component of that
    /// channel.
    pub(super) channels: Vec<Vec<ComponentRunLevels>>,
}

impl TrackLevelCache {
    /// Selection for a metric with one continuous series.
    pub(super) fn level_for(&self, kind: MetricKind) -> Option<LevelSelection> {
        Some(match kind {
            MetricKind::Velocity => self.velocity_kmh,
            MetricKind::Eph => self.eph_m,
            MetricKind::HeadingDeg => self.heading_deg,
            MetricKind::ClockDeltaMs => self.clock_delta_ms,
            MetricKind::Hdop
            | MetricKind::Vdop
            | MetricKind::Pdop
            | MetricKind::SatsSeen
            | MetricKind::SatsFix
            | MetricKind::GpsSeen
            | MetricKind::GpsFix
            | MetricKind::GlonassSeen
            | MetricKind::GlonassFix
            | MetricKind::GalileoSeen
            | MetricKind::GalileoFix
            | MetricKind::BeidouSeen
            | MetricKind::BeidouFix
            | MetricKind::NavicSeen
            | MetricKind::NavicFix
            | MetricKind::QzssSeen
            | MetricKind::QzssFix
            | MetricKind::UtilAll
            | MetricKind::UtilGps
            | MetricKind::UtilGlonass
            | MetricKind::UtilGalileo
            | MetricKind::UtilBeidou
            | MetricKind::UtilNavic
            | MetricKind::UtilQzss
            | MetricKind::SlipAll
            | MetricKind::SlipGps
            | MetricKind::SlipGlonass
            | MetricKind::SlipGalileo
            | MetricKind::SlipBeidou
            | MetricKind::SlipNavic
            | MetricKind::SlipQzss
            | MetricKind::SnapError
            | MetricKind::Jamming
            | MetricKind::Hp30
            | MetricKind::Kp
            | MetricKind::Tec => {
                return None;
            }
        })
    }
}

impl TrackSeries {
    pub(super) fn has_samples_in(&self, kind: MetricKind, window: RangeInclusive<f64>) -> bool {
        if self
            .metrics
            .get(&kind)
            .into_iter()
            .flat_map(|metric| &metric.runs)
            .chain(self.mipmap_for(kind))
            .any(|run| {
                let samples = run.finest_level();
                let start = samples.partition_point(|sample| sample.x < *window.start());
                samples
                    .get(start)
                    .is_some_and(|sample| window.contains(&sample.x))
            })
        {
            return true;
        }
        if kind == MetricKind::ClockDeltaMs {
            let samples_in_window = |samples: &[ExcursionSample]| {
                samples.iter().any(|sample| window.contains(&sample.t))
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

    /// `None` for metrics with no mipmap, matching
    /// [`TrackLevelCache::level_for`].
    pub(super) fn mipmap_for(&self, kind: MetricKind) -> Option<&gt_egui_mipmap::MipMap> {
        Some(match kind {
            MetricKind::Velocity => &self.velocity_kmh,
            MetricKind::Eph => &self.eph_m,
            MetricKind::HeadingDeg => &self.heading_deg,
            MetricKind::ClockDeltaMs => &self.clock_delta_ms,
            MetricKind::Hdop
            | MetricKind::Vdop
            | MetricKind::Pdop
            | MetricKind::SatsSeen
            | MetricKind::SatsFix
            | MetricKind::GpsSeen
            | MetricKind::GpsFix
            | MetricKind::GlonassSeen
            | MetricKind::GlonassFix
            | MetricKind::GalileoSeen
            | MetricKind::GalileoFix
            | MetricKind::BeidouSeen
            | MetricKind::BeidouFix
            | MetricKind::NavicSeen
            | MetricKind::NavicFix
            | MetricKind::QzssSeen
            | MetricKind::QzssFix
            | MetricKind::UtilAll
            | MetricKind::UtilGps
            | MetricKind::UtilGlonass
            | MetricKind::UtilGalileo
            | MetricKind::UtilBeidou
            | MetricKind::UtilNavic
            | MetricKind::UtilQzss
            | MetricKind::SlipAll
            | MetricKind::SlipGps
            | MetricKind::SlipGlonass
            | MetricKind::SlipGalileo
            | MetricKind::SlipBeidou
            | MetricKind::SlipNavic
            | MetricKind::SlipQzss
            | MetricKind::SnapError
            | MetricKind::Jamming
            | MetricKind::Hp30
            | MetricKind::Kp
            | MetricKind::Tec => {
                return None;
            }
        })
    }
}
/// Compute fresh level selections for all metrics of one track's series.
///
/// The sample target is derived per track from how many pixels the track
/// occupies in the current view ([`track_target`]), so a track that is only a
/// few pixels wide selects a coarse mipmap level with just a few points.
pub(super) fn compute_level_cache(series: &TrackSeries, viewport: LineViewport) -> TrackLevelCache {
    let target = track_target(
        series.x_range,
        viewport.x_min,
        viewport.x_max,
        viewport.width,
        viewport.cap,
    );
    let range = viewport.selection_range();
    let sel = |mm: &MipMap| mm.select_indices(range, target);
    TrackLevelCache {
        metrics: series
            .metrics
            .iter()
            .map(|(kind, metric)| (*kind, viewport.select_run_levels(&metric.runs)))
            .collect(),
        velocity_kmh: sel(&series.velocity_kmh),
        eph_m: sel(&series.eph_m),
        heading_deg: sel(&series.heading_deg),
        clock_delta_ms: sel(&series.clock_delta_ms),
        channels: series
            .channels
            .iter()
            .map(|channel| {
                channel
                    .components
                    .iter()
                    .map(|component| viewport.select_run_levels(&component.runs))
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

    use chrono::{DateTime, Utc};
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
            LineViewport {
                x_min,
                x_max,
                time_window: FilterTimeWindow {
                    start: Some(x_min),
                    end: Some(x_max),
                },
                width: PLOT_WIDTH_PX,
                cap: single_target(PLOT_WIDTH_PX),
            },
        );
        let level = cache
            .level_for(MetricKind::Velocity)
            .expect("velocity has a mipmap");
        let mipmap = placed
            .series
            .mipmap_for(MetricKind::Velocity)
            .expect("velocity has a mipmap");
        mipmap
            .slice_at(level)
            .iter()
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
        let placed =
            crate::series::build_all_series(&[recording(points)], AnalysisConfig::default());
        let series = &placed.first().expect("one track").series;
        let base = FIRST_FIX_SECS as f64;
        let viewport = LineViewport {
            x_min: base,
            x_max: base + 1_099.0,
            time_window: FilterTimeWindow {
                start: None,
                end: None,
            },
            width: 1.0,
            cap: 2,
        };
        let cache = compute_level_cache(series, viewport);
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
            let selected = run.slice_at(*selection);
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
            x_min: base + 500.0,
            x_max: base + 599.0,
            time_window: FilterTimeWindow {
                start: Some(base + 500.0),
                end: Some(base + 599.0),
            },
            ..viewport
        };
        let cache = compute_level_cache(series, in_gap);
        for (run, selection) in runs.iter().zip(
            cache
                .metrics
                .get(&MetricKind::SatsFix)
                .expect("count selections"),
        ) {
            assert!(run.slice_at(*selection).is_empty());
            assert!(!in_gap.shows(run));
        }
    }

    /// Plot width in points. Wide enough that a window of a few seconds still
    /// reads the finest mipmap level.
    const PLOT_WIDTH_PX: f32 = 800.0;

    /// 2024-01-15 12:00:00 UTC, the first fix of every track.
    const FIRST_FIX_SECS: i64 = 1_705_320_000;
}
