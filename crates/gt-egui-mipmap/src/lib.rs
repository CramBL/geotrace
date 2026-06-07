//! Min/max mipmap cascade for egui_plot time-series data.
//!
//! A `MipMap` stores a series of progressively downsampled levels of a
//! `[time_secs, value]` dataset, using [`egui_plot::PlotPoint`] as the native
//! element type so callers can pass [`egui_plot::PlotPoints::Borrowed`] directly
//! to a plot line - no per-frame allocation required.
//!
//! Each level is produced by grouping the previous level into windows of
//! [`DOWNSAMPLE_WINDOW`] points and emitting the minimum-value point and the
//! maximum-value point for each window in chronological order.
//!
//! This preserves all outliers (spikes and dips) at every zoom level while
//! dramatically reducing the number of points that must be sent to the GPU
//! when the plot is zoomed out.
//!
//! ## Selecting the right level
//!
//! `MipMap::select_slice` returns the coarsest level that has at least
//! `target_count` data points within the requested x range.
//! Caller passes the plot's current x bounds and approximately
//! `screen_width_px * 2` as the target to keep rendering fast without
//! sacrificing visual fidelity.
//!
//! ## Complexity
//!
//! | Operation        | Time          | Space      |
//! |-----------------|---------------|------------|
//! | `build`          | O(n)          | O(n)       |
//! | `select_slice`   | O(log n)      | O(1)       |

use egui_plot::PlotPoint;

/// Minimum number of points in a mipmap level.
/// Levels smaller than this are not added to the cascade.
const MIN_LEVEL_POINTS: usize = 200;

/// Number of input points grouped into one output pair (min + max) at each
/// downsampling step.
/// This gives a ~4× point-count reduction per level.
const DOWNSAMPLE_WINDOW: usize = 8;

/// A cached level selection produced by [`MipMap::select_indices`].
///
/// Stores the level index and the pre-computed byte positions of the clipped
/// sub-range so that [`MipMap::slice_at`] can return the data in O(1) without
/// repeating any binary searches.
#[derive(Debug, Clone, Copy, Default)]
pub struct LevelSelection {
    level_idx: usize,
    clip_start: usize,
    clip_end: usize,
}

/// A cascade of progressively downsampled time-series levels.
///
/// `levels[0]` is the original (finest) data.
/// `levels.last()` is the coarsest (most downsampled) data.
/// All levels are sorted by the x coordinate (time, in seconds).
///
/// Each point is stored as a [`PlotPoint`] so the slice returned by
/// [`Self::select_slice`] / [`Self::slice_at`] can be handed directly to
/// [`egui_plot::PlotPoints::Borrowed`] for zero-copy rendering.
#[derive(Debug, Clone)]
pub struct MipMap {
    levels: Vec<Vec<PlotPoint>>,
}

impl MipMap {
    /// Build a mipmap cascade from a time-sorted dataset.
    ///
    /// `data` must be sorted by `x` (ascending).
    /// If it has fewer than `MIN_LEVEL_POINTS` points no downsampling is
    /// performed and the cascade contains only the original data as its single
    /// level.
    pub fn build(data: Vec<[f64; 2]>) -> Self {
        if data.is_empty() {
            return Self { levels: Vec::new() };
        }
        let points: Vec<PlotPoint> = data.into_iter().map(|[x, y]| PlotPoint { x, y }).collect();
        let mut levels = vec![points];
        loop {
            // `levels` is guaranteed non-empty: it was initialised with one element
            // and we only append.  `map_or` avoids an `expect` while remaining
            // correct: an empty fallback produces an empty downsample, which has
            // len < MIN_LEVEL_POINTS and immediately breaks.
            let next = downsample(levels.last().map_or(&[][..], Vec::as_slice));
            if next.len() < MIN_LEVEL_POINTS {
                break;
            }
            levels.push(next);
        }
        Self { levels }
    }

    /// `true` when the finest level has no data points.
    pub fn is_empty(&self) -> bool {
        self.levels.is_empty()
    }

    /// Return the `[x_min, x_max]` of the full dataset, or `None` when empty.
    ///
    /// O(1) - the finest level is sorted by x, so the range is just its
    /// first and last element.
    pub fn x_range(&self) -> Option<(f64, f64)> {
        let level = self.levels.first()?;
        Some((level.first()?.x, level.last()?.x))
    }

    /// Total number of points in the finest (original) level.
    pub fn original_len(&self) -> usize {
        self.levels.first().map_or(0, Vec::len)
    }

    /// Number of mipmap levels, including the finest.
    pub fn level_count(&self) -> usize {
        self.levels.len()
    }

    /// Return the coarsest mipmap level that has at least `target_count` data
    /// points within `[x_min, x_max]`, clipped to that range.
    ///
    /// Falls back to the finest level when no level meets the target - this
    /// happens when the plot is zoomed into a very narrow time window.
    ///
    /// The returned slice borrows directly from the internal buffer, so it has
    /// the lifetime of `&self`.
    /// Pass it to [`egui_plot::PlotPoints::Borrowed`] for zero-copy rendering.
    pub fn select_slice(&self, x_min: f64, x_max: f64, target_count: usize) -> &[PlotPoint] {
        let (level_idx, inner_start, inner_end) =
            self.select_level_bounds(x_min, x_max, target_count);
        let level = self.levels.get(level_idx).map_or(&[][..], Vec::as_slice);
        let start = inner_start.saturating_sub(1);
        let end = (inner_end + 1).min(level.len());
        // `start` and `end` are derived from `partition_point` results on this
        // same level, clamped to `0..=level.len()`, so the slice is in bounds.
        #[expect(
            clippy::indexing_slicing,
            reason = "start and end are partition_point results clamped to 0..=level.len()"
        )]
        &level[start..end]
    }

    /// Compute a `LevelSelection` that records which level and which sub-range
    /// of that level to use for the given view bounds and target point count.
    ///
    /// Pass the result to [`Self::slice_at`] to obtain the data slice.
    /// Storing the `LevelSelection` and calling `slice_at` each frame avoids
    /// repeating the binary searches inside `select_slice` when the view bounds
    /// have not changed.
    ///
    /// The selection always extends one point beyond each edge of `[x_min, x_max]`
    /// when such a point exists, so the rendered line stays connected to the
    /// data outside the visible viewport.
    pub fn select_indices(&self, x_min: f64, x_max: f64, target_count: usize) -> LevelSelection {
        if self.levels.is_empty() {
            return LevelSelection::default();
        }
        let (level_idx, inner_start, inner_end) =
            self.select_level_bounds(x_min, x_max, target_count);
        let level = self.levels.get(level_idx).map_or(&[][..], Vec::as_slice);
        let clip_start = inner_start.saturating_sub(1);
        let clip_end = (inner_end + 1).min(level.len());
        LevelSelection {
            level_idx,
            clip_start,
            clip_end,
        }
    }

    /// Return the data slice described by a previously computed `LevelSelection`.
    ///
    /// The selection must have been produced by [`Self::select_indices`] on the
    /// same `MipMap` without any rebuild in between; the caller is responsible
    /// for invalidating cached selections whenever the source data changes.
    pub fn slice_at(&self, sel: LevelSelection) -> &[PlotPoint] {
        let Some(level) = self.levels.get(sel.level_idx) else {
            return &[];
        };
        // `clip_start` and `clip_end` came from `partition_point` on this same
        // level, so they are guaranteed to be in `0..=level.len()`.  Clamp
        // defensively in case `sel` was produced from a different generation.
        let start = sel.clip_start.min(level.len());
        let end = sel.clip_end.min(level.len());
        #[expect(
            clippy::indexing_slicing,
            reason = "start and end are clamped to level.len() above"
        )]
        &level[start..end]
    }

    /// Find the coarsest level with enough points in `[x_min, x_max]`, and
    /// return `(level_idx, inner_start, inner_end)` where `inner_start`/
    /// `inner_end` are that level's `partition_point` bounds for the range.
    ///
    /// Returning the bounds alongside the index lets [`Self::select_slice`]
    /// and [`Self::select_indices`] reuse them directly instead of repeating
    /// the same two binary searches on the chosen level.
    ///
    /// Falls back to `(0, inner_start, inner_end)` for the finest level when
    /// no level meets the target - the reverse iteration always visits level 0
    /// last, so its bounds are already on hand to serve as that fallback.
    fn select_level_bounds(
        &self,
        x_min: f64,
        x_max: f64,
        target_count: usize,
    ) -> (usize, usize, usize) {
        // Try from coarsest → finest; use the coarsest level that is dense
        // enough for the target count in the visible range.
        let mut bounds = (0, 0, 0);
        for (i, level) in self.levels.iter().enumerate().rev() {
            let inner_start = level.partition_point(|p| p.x < x_min);
            let inner_end = level.partition_point(|p| p.x <= x_max);
            bounds = (i, inner_start, inner_end);
            if inner_end.saturating_sub(inner_start) >= target_count {
                return bounds;
            }
        }
        bounds
    }
}

/// Produce the next (coarser) mipmap level from `data` by grouping into
/// windows and emitting the minimum-value and maximum-value points in
/// chronological order.
///
/// Single-point windows emit only that one point (no duplication).
/// Windows where min and max fall on the same point emit it once.
fn downsample(data: &[PlotPoint]) -> Vec<PlotPoint> {
    let mut out = Vec::with_capacity(data.len() / DOWNSAMPLE_WINDOW * 2 + 2);
    for chunk in data.chunks(DOWNSAMPLE_WINDOW) {
        let Some(min_pt) = chunk.iter().min_by(|a, b| a.y.total_cmp(&b.y)) else {
            continue;
        };
        let Some(max_pt) = chunk.iter().max_by(|a, b| a.y.total_cmp(&b.y)) else {
            continue;
        };

        // Emit in chronological order so the rendered line follows the
        // correct time direction and the max/min shape is preserved.
        // Compare by identity rather than `x` equality: a vertical line (two
        // distinct points sharing one timestamp) must still emit both ends.
        if std::ptr::eq(min_pt, max_pt) {
            // Same point - one representative.
            out.push(*min_pt);
        } else if min_pt.x < max_pt.x {
            out.push(*min_pt);
            out.push(*max_pt);
        } else {
            out.push(*max_pt);
            out.push(*min_pt);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn seq(n: usize) -> Vec<[f64; 2]> {
        (0..n).map(|i| [i as f64, (i % 10) as f64]).collect()
    }

    #[test]
    fn empty_mipmap() {
        let m = MipMap::build(Vec::new());
        assert!(m.is_empty());
        assert_eq!(m.original_len(), 0);
        let empty: &[PlotPoint] = &[];
        assert_eq!(m.select_slice(0.0, 100.0, 10), empty);
    }

    #[test]
    fn small_data_no_downsampling() {
        let data = seq(50);
        let m = MipMap::build(data.clone());
        assert_eq!(m.level_count(), 1);
        assert_eq!(m.original_len(), 50);
    }

    #[test]
    fn large_data_produces_levels() {
        let data = seq(10_000);
        let m = MipMap::build(data);
        assert!(
            m.level_count() > 1,
            "should have at least 2 levels for 10K points"
        );
        // Coarsest level should have >= MIN_LEVEL_POINTS points
        let coarsest = m.levels.last().expect("at least one level");
        assert!(coarsest.len() >= MIN_LEVEL_POINTS);
    }

    #[test]
    fn select_slice_clips_to_range() {
        let data: Vec<[f64; 2]> = (0..1000).map(|i| [i as f64, i as f64]).collect();
        let m = MipMap::build(data);
        let slice = m.select_slice(100.0, 200.0, 10);
        // All returned points must come from the original data range (no garbage).
        assert!(slice.iter().all(|p| p.x >= 0.0 && p.x <= 999.0));
        // The slice must include points from within the viewport.
        assert!(slice.iter().any(|p| p.x >= 100.0 && p.x <= 200.0));
    }

    #[test]
    fn outliers_preserved_in_downsampled_level() {
        // Build a series where there's a clear spike at index 4.
        let mut data: Vec<[f64; 2]> = (0..800).map(|i| [i as f64, 1.0]).collect();
        data[4] = [4.0, 1000.0]; // spike
        let m = MipMap::build(data);
        if m.level_count() > 1 {
            // The spike should appear in the coarsest level.
            let has_spike = m
                .levels
                .last()
                .expect("at least one")
                .iter()
                .any(|p| p.y > 500.0);
            assert!(has_spike, "min/max downsampling must preserve spikes");
        }
    }

    #[test]
    fn select_uses_coarser_level_for_wide_view() {
        let data: Vec<[f64; 2]> = (0..10_000).map(|i| [i as f64, 1.0]).collect();
        let m = MipMap::build(data);
        let total_range_slice = m.select_slice(0.0, 9_999.0, 50);
        // With a small target (50), the coarsest level should be selected.
        // The coarsest level has MIN_LEVEL_POINTS ≥ 200 points total,
        // but in the full range that's still < 10_000.
        assert!(
            total_range_slice.len() < 5_000,
            "should use a coarser level"
        );
    }

    // --- Neighbor-inclusion regression tests (the "broken line at viewport edge" bug) ---
    //
    // When the plot is zoomed in so that the series extends beyond both sides of
    // the viewport, the rendered line must include one point on each side of the
    // visible range.  Without that, egui_plot draws the line only between the
    // points that fall inside the viewport, producing a visually disconnected
    // segment instead of a continuous line.
    fn int_data(n: usize) -> Vec<[f64; 2]> {
        (0..n).map(|i| [i as f64, i as f64]).collect()
    }

    #[test]
    fn select_slice_includes_left_neighbor() {
        let m = MipMap::build(int_data(50));
        // Viewport [10, 30]: point at x=9 must be included as the left neighbor.
        let slice = m.select_slice(10.0, 30.0, 5);
        assert!(
            slice.iter().any(|p| p.x as i64 == 9),
            "select_slice must include the point just left of x_min to keep the line connected"
        );
    }

    #[test]
    fn select_slice_includes_right_neighbor() {
        let m = MipMap::build(int_data(50));
        // Viewport [10, 30]: point at x=31 must be included as the right neighbor.
        let slice = m.select_slice(10.0, 30.0, 5);
        assert!(
            slice.iter().any(|p| p.x as i64 == 31),
            "select_slice must include the point just right of x_max to keep the line connected"
        );
    }

    #[test]
    fn select_indices_includes_left_neighbor() {
        let m = MipMap::build(int_data(50));
        let sel = m.select_indices(10.0, 30.0, 5);
        let slice = m.slice_at(sel);
        assert!(
            slice.iter().any(|p| p.x as i64 == 9),
            "select_indices/slice_at must include the point just left of x_min"
        );
    }

    #[test]
    fn select_indices_includes_right_neighbor() {
        let m = MipMap::build(int_data(50));
        let sel = m.select_indices(10.0, 30.0, 5);
        let slice = m.slice_at(sel);
        assert!(
            slice.iter().any(|p| p.x as i64 == 31),
            "select_indices/slice_at must include the point just right of x_max"
        );
    }

    #[test]
    fn select_slice_no_left_neighbor_at_start() {
        // x_min is at the very beginning: no left neighbor exists, must not panic.
        let m = MipMap::build(int_data(50));
        let slice = m.select_slice(0.0, 10.0, 5);
        assert!(
            slice.iter().all(|p| p.x >= 0.0),
            "must not include out-of-range left points"
        );
        assert!(slice.first().is_some_and(|p| p.x as i64 == 0));
    }

    #[test]
    fn select_slice_no_right_neighbor_at_end() {
        // x_max is at the very end: no right neighbor exists, must not panic.
        let m = MipMap::build(int_data(50));
        let slice = m.select_slice(40.0, 49.0, 5);
        assert!(
            slice.iter().all(|p| p.x <= 49.0),
            "must not include out-of-range right points"
        );
        assert!(slice.last().is_some_and(|p| p.x as i64 == 49));
    }

    /// Smallest input length for which `MipMap::build` is guaranteed to append
    /// a real downsampled level as `levels[1]`, regardless of the current
    /// values of `MIN_LEVEL_POINTS` / `DOWNSAMPLE_WINDOW`.
    fn cascading_input_len() -> usize {
        MIN_LEVEL_POINTS * DOWNSAMPLE_WINDOW
    }

    /// Guards against using `min_pt.x == max_pt.x` to detect "min and max are
    /// the same point": a vertical line - two distinct points that share a
    /// timestamp but have different `y` values - also satisfies that
    /// equality, so the comparison wrongly collapses both ends down to one
    /// and silently drops the other.
    #[test]
    fn vertical_line_min_and_max_both_survive_downsampling() {
        let n = cascading_input_len();
        let mut data: Vec<[f64; 2]> = (0..n).map(|i| [i as f64, i as f64]).collect();

        // Plant a vertical line - two distinct points sharing one timestamp,
        // far outside the background range - in the second window so they
        // become that window's unique min and max.
        let t = DOWNSAMPLE_WINDOW;
        data[t] = [t as f64, 1.0e6]; // spike: this window's max
        data[t + 1] = [t as f64, -1.0e6]; // dip: this window's min, same timestamp

        let m = MipMap::build(data);
        assert!(
            m.level_count() >= 2,
            "a {n}-point input must produce a downsampled level (got {})",
            m.level_count()
        );

        let downsampled = &m.levels[1];
        #[expect(
            clippy::float_cmp,
            reason = "checking that planted constants survive the pipeline by \
                      copy, bit-for-bit - exact equality is intentional"
        )]
        {
            assert!(
                downsampled.iter().any(|p| p.y == 1.0e6),
                "spike lost: a vertical line's max must survive downsampling even \
                 though min and max share a timestamp"
            );
            assert!(
                downsampled.iter().any(|p| p.y == -1.0e6),
                "dip lost: a vertical line's min must survive downsampling even \
                 though min and max share a timestamp"
            );
        }
    }

    /// Guards against using `partial_cmp(...).unwrap_or(Ordering::Equal)` to
    /// rank `y` values: `partial_cmp` returns `None` for any comparison
    /// involving NaN, which the `unwrap_or` turns into `Equal`. A NaN that
    /// becomes `min_by`'s running accumulator then compares `Equal` to every
    /// later candidate, so `min_by` keeps it as the "winner" forever and the
    /// window's genuine minimum never displaces it.
    #[test]
    fn nan_does_not_corrupt_min_tracking() {
        let n = cascading_input_len();
        let mut data: Vec<[f64; 2]> = (0..n).map(|i| [i as f64, 0.0]).collect();

        // Place NaN as the first element of a window and a genuine dip later
        // in the same window so the corruption above would manifest.
        let t = DOWNSAMPLE_WINDOW;
        data[t] = [t as f64, f64::NAN];
        data[t + 3] = [(t + 3) as f64, -1.0e6]; // this window's genuine min

        let m = MipMap::build(data);
        assert!(
            m.level_count() >= 2,
            "a {n}-point input must produce a downsampled level (got {})",
            m.level_count()
        );

        let downsampled = &m.levels[1];
        #[expect(
            clippy::float_cmp,
            reason = "checking that a planted constant survives the pipeline by \
                      copy, bit-for-bit - exact equality is intentional"
        )]
        {
            assert!(
                downsampled.iter().any(|p| p.y == -1.0e6),
                "dip lost: a NaN sample elsewhere in the window must not corrupt min-tracking"
            );
        }
    }
}
