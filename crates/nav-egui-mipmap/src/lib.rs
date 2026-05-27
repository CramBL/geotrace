//! Min/max mipmap cascade for egui_plot time-series data.
//!
//! A `MipMap` stores a series of progressively downsampled levels of a
//! `[time_secs, value]` dataset, using [`egui_plot::PlotPoint`] as the native
//! element type so callers can pass [`egui_plot::PlotPoints::Borrowed`] directly
//! to a plot line — no per-frame allocation required.
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

use std::cmp::Ordering;

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
    /// O(1) — the finest level is sorted by x, so the range is just its
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
    /// Falls back to the finest level when no level meets the target — this
    /// happens when the plot is zoomed into a very narrow time window.
    ///
    /// The returned slice borrows directly from the internal buffer, so it has
    /// the lifetime of `&self`.
    /// Pass it to [`egui_plot::PlotPoints::Borrowed`] for zero-copy rendering.
    pub fn select_slice(&self, x_min: f64, x_max: f64, target_count: usize) -> &[PlotPoint] {
        let idx = self.select_level_idx(x_min, x_max, target_count);
        let level = self.levels.get(idx).map_or(&[][..], Vec::as_slice);
        clip_to_range(level, x_min, x_max)
    }

    /// Compute a `LevelSelection` that records which level and which sub-range
    /// of that level to use for the given view bounds and target point count.
    ///
    /// Pass the result to [`Self::slice_at`] to obtain the data slice.
    /// Storing the `LevelSelection` and calling `slice_at` each frame avoids
    /// repeating the binary searches inside `select_slice` when the view bounds
    /// have not changed.
    pub fn select_indices(&self, x_min: f64, x_max: f64, target_count: usize) -> LevelSelection {
        if self.levels.is_empty() {
            return LevelSelection::default();
        }
        let level_idx = self.select_level_idx(x_min, x_max, target_count);
        let level = self.levels.get(level_idx).map_or(&[][..], Vec::as_slice);
        let clip_start = level.partition_point(|p| p.x < x_min);
        let clip_end = level.partition_point(|p| p.x <= x_max);
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

    /// Index of the coarsest level with enough points in `[x_min, x_max]`.
    /// Returns `0` (finest) when no level meets the target.
    fn select_level_idx(&self, x_min: f64, x_max: f64, target_count: usize) -> usize {
        // Try from coarsest → finest; use the coarsest level that is dense
        // enough for the target count in the visible range.
        for (i, level) in self.levels.iter().enumerate().rev() {
            if count_in_range(level, x_min, x_max) >= target_count {
                return i;
            }
        }
        0
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
        let Some(min_pt) = chunk
            .iter()
            .min_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(Ordering::Equal))
        else {
            continue;
        };
        let Some(max_pt) = chunk
            .iter()
            .max_by(|a, b| a.y.partial_cmp(&b.y).unwrap_or(Ordering::Equal))
        else {
            continue;
        };

        // Emit in chronological order so the rendered line follows the
        // correct time direction and the max/min shape is preserved.
        #[expect(
            clippy::float_cmp,
            reason = "comparing time coordinates that came from the same source data; \
                      NaN-free and exact equality is intentional to detect same-point min/max"
        )]
        if min_pt.x == max_pt.x {
            // Same timestamp — one representative point.
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

/// Count points in `data` (sorted by x) within the closed interval `[x_min, x_max]`.
fn count_in_range(data: &[PlotPoint], x_min: f64, x_max: f64) -> usize {
    let start = data.partition_point(|p| p.x < x_min);
    let end = data.partition_point(|p| p.x <= x_max);
    end.saturating_sub(start)
}

/// Return the sub-slice of `data` within `[x_min, x_max]`.
fn clip_to_range(data: &[PlotPoint], x_min: f64, x_max: f64) -> &[PlotPoint] {
    let start = data.partition_point(|p| p.x < x_min);
    let end = data.partition_point(|p| p.x <= x_max);
    // `start` and `end` come from `partition_point`, which guarantees
    // `0 <= start <= end <= data.len()` — the slice is always in bounds.
    #[expect(
        clippy::indexing_slicing,
        reason = "start and end are partition_point results, always within 0..=data.len()"
    )]
    &data[start..end]
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
        #[expect(
            clippy::expect_used,
            reason = "test invariant: build succeeded so levels is non-empty"
        )]
        let coarsest = m.levels.last().expect("at least one level");
        assert!(coarsest.len() >= MIN_LEVEL_POINTS);
    }

    #[test]
    fn select_slice_clips_to_range() {
        let data: Vec<[f64; 2]> = (0..1000).map(|i| [i as f64, i as f64]).collect();
        let m = MipMap::build(data);
        let slice = m.select_slice(100.0, 200.0, 10);
        assert!(slice.iter().all(|p| p.x >= 100.0 && p.x <= 200.0));
    }

    #[test]
    fn outliers_preserved_in_downsampled_level() {
        // Build a series where there's a clear spike at index 4.
        let mut data: Vec<[f64; 2]> = (0..800).map(|i| [i as f64, 1.0]).collect();
        data[4] = [4.0, 1000.0]; // spike
        let m = MipMap::build(data);
        if m.level_count() > 1 {
            // The spike should appear in the coarsest level.
            #[expect(
                clippy::expect_used,
                reason = "test invariant: level_count > 1 guarantees levels is non-empty"
            )]
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
}
