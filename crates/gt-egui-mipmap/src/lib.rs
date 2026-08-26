//! Min/max mipmap cascade for egui_plot time-series data.
//!
//! A `MipMap` stores a series of progressively downsampled levels of a
//! `[time_secs, value]` dataset, using [`egui_plot::PlotPoint`] as the native
//! element type so callers can pass [`egui_plot::PlotPoints::Borrowed`] directly
//! to a plot line - no per-frame allocation required.
//!
//! Each level is produced by grouping the previous level into windows of
//! `DOWNSAMPLE_WINDOW` points and emitting the minimum-value point and the
//! maximum-value point for each window in chronological order.
//!
//! This preserves all outliers (spikes and dips) at every zoom level while
//! dramatically reducing the number of points that must be sent to the GPU
//! when the plot is zoomed out.
//!
//! [`MipMap::build_wrapping`] keeps the point furthest to either side of the
//! window's own mean direction instead, for a series whose values wrap (a
//! heading, at 360°): around the wrap the linear minimum and maximum sit next
//! to each other on the circle.
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

/// Smallest mipmap level the cascade builds down to: two points, i.e. a single
/// line segment.  Driving the cascade all the way down lets a track that only
/// occupies a few screen pixels be drawn with a handful of points.  (A 2-point
/// level downsamples to itself, so [`MipMap::build`] also stops when a level
/// stops shrinking.)
const MIN_LEVEL_POINTS: usize = 2;

/// Number of input points grouped into one output pair (min + max) at each
/// downsampling step.
/// A window of 4 emits 2 points, giving a ~2× point-count reduction per level -
/// fine-grained enough that level selection can closely match the target sample
/// count rather than overshooting it by up to 4×.
const DOWNSAMPLE_WINDOW: usize = 4;

const FULL_TURN_DEGREES: f64 = 360.0;

/// Length below which the summed sample directions of a window cancel out and
/// leave its mean direction undefined.
const MEAN_RESULTANT_FLOOR: f64 = 1e-9;

/// The period at which a series' values wrap: a compass heading repeats every
/// 360°, and a `.gtd` channel declares its own.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct WrapPeriod(f64);

impl WrapPeriod {
    /// `None` unless `period` is finite and positive: anything else describes
    /// no wrap.
    pub fn new(period: f64) -> Option<Self> {
        (period.is_finite() && period > 0.0).then_some(Self(period))
    }

    /// The period of a compass quantity in degrees: a heading, a bearing, an
    /// azimuth.
    pub const fn full_turn_degrees() -> Self {
        Self(FULL_TURN_DEGREES)
    }

    /// The direction `values` point in on average, scaled back to this period,
    /// or `None` when they cancel out or one of them is not finite.
    fn mean_direction(self, values: impl Iterator<Item = f64>) -> Option<f64> {
        let scale = std::f64::consts::TAU / self.0;
        let (sines, cosines) = values.fold((0.0, 0.0), |(sines, cosines), value| {
            let radians = value * scale;
            (sines + radians.sin(), cosines + radians.cos())
        });
        (sines.hypot(cosines) > MEAN_RESULTANT_FLOOR).then(|| sines.atan2(cosines) / scale)
    }

    /// The arc from one value to another, in `[-period / 2, period / 2)`:
    /// negative counterclockwise, positive clockwise.
    fn signed_arc(self, from: f64, to: f64) -> f64 {
        let half_period = self.0 / 2.0;
        (to - from + half_period).rem_euclid(self.0) - half_period
    }

    /// Whether `value` lies within one period of the origin, `[0, period)`.
    fn contains(self, value: f64) -> bool {
        (0.0..self.0).contains(&value)
    }
}

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

impl LevelSelection {
    /// Whether this selection reads the finest (original) level - i.e. the
    /// view shows full detail and every selected point is a real data point
    /// rather than a downsampled aggregate.
    pub fn is_full_detail(&self) -> bool {
        self.level_idx == 0
    }
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
    /// Downsampling continues by ~2× per level until a level reaches the
    /// `MIN_LEVEL_POINTS` floor (or stops shrinking), so all but the very
    /// smallest inputs gain coarse levels down to a single segment.
    pub fn build(data: Vec<[f64; 2]>) -> Self {
        Self::build_levels(data, None)
    }

    /// Build a mipmap cascade from a time-sorted dataset whose values wrap
    /// every `period`.
    ///
    /// Each window keeps the point furthest to either side of the window's own
    /// mean direction, both of them input samples as at every other level.
    /// Around the wrap that keeps a southward heading between two northward
    /// ones, which the linear minimum and maximum [`Self::build`] keeps (~0°
    /// and ~359°) drop.
    ///
    /// This applies only to a window whose samples all lie within one period,
    /// `[0, period)`. A window holding a sample outside one period, or a
    /// sample that is not finite, keeps its linear minimum and maximum: a
    /// heading of 675° stays at 675° at every level. A window whose samples
    /// cancel out, leaving no mean direction, keeps its linear minimum and
    /// maximum too.
    pub fn build_wrapping(data: Vec<[f64; 2]>, period: WrapPeriod) -> Self {
        Self::build_levels(data, Some(period))
    }

    fn build_levels(data: Vec<[f64; 2]>, period: Option<WrapPeriod>) -> Self {
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
            let last_len = levels.last().map_or(0, Vec::len);
            let next = downsample(levels.last().map_or(&[][..], Vec::as_slice), period);
            // Stop at the floor, and also when a level stops shrinking: a
            // 2-point window downsamples back to 2 points, so without this guard
            // the cascade would loop forever once it reaches the bottom.
            if next.len() < MIN_LEVEL_POINTS || next.len() >= last_len {
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
    /// same `MipMap` without any rebuild in between. The caller is responsible
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
    ///
    /// Normalizes an inverted range (`x_min > x_max`) by raising `x_max` to
    /// `x_min`, restoring the `inner_start <= inner_end` invariant
    /// [`Self::select_slice`] / [`Self::select_indices`] rely on - see
    /// `select_handles_inverted_and_disjoint_ranges_without_panic` for why
    /// callers can end up passing such a range.
    fn select_level_bounds(
        &self,
        x_min: f64,
        x_max: f64,
        target_count: usize,
    ) -> (usize, usize, usize) {
        let x_max = x_max.max(x_min);
        // Try from coarsest → finest. Use the coarsest level that is dense
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
/// windows and emitting the two points each window keeps in chronological
/// order: its minimum-value and maximum-value points, or for a wrapping
/// series the furthest to either side of the window's mean direction.
///
/// Single-point windows emit only that one point (no duplication).
/// Windows where both kept points fall on the same point emit it once.
fn downsample(data: &[PlotPoint], period: Option<WrapPeriod>) -> Vec<PlotPoint> {
    let mut out = Vec::with_capacity(data.len() / DOWNSAMPLE_WINDOW * 2 + 2);
    for chunk in data.chunks(DOWNSAMPLE_WINDOW) {
        let kept = period
            .and_then(|period| extremes_either_side_of_mean_direction(chunk, period))
            .or_else(|| {
                let min_pt = chunk.iter().min_by(|a, b| a.y.total_cmp(&b.y))?;
                let max_pt = chunk.iter().max_by(|a, b| a.y.total_cmp(&b.y))?;
                Some((min_pt, max_pt))
            });
        let Some((first, second)) = kept else {
            continue;
        };

        // Emit in chronological order so the rendered line follows the
        // correct time direction and the shape of the window is preserved.
        // Compare by identity rather than `x` equality: a vertical line (two
        // distinct points sharing one timestamp) must still emit both ends.
        if std::ptr::eq(first, second) {
            // Same point - one representative.
            out.push(*first);
        } else if first.x < second.x {
            out.push(*first);
            out.push(*second);
        } else {
            out.push(*second);
            out.push(*first);
        }
    }
    out
}

/// The two points of `chunk` lying furthest to either side of its mean
/// direction, by signed arc, or `None` when a value falls outside one period
/// or the values cancel out and leave no mean direction. A single-point chunk
/// yields that point twice.
fn extremes_either_side_of_mean_direction(
    chunk: &[PlotPoint],
    period: WrapPeriod,
) -> Option<(&PlotPoint, &PlotPoint)> {
    if !chunk.iter().all(|p| period.contains(p.y)) {
        return None;
    }
    let mean = period.mean_direction(chunk.iter().map(|p| p.y))?;
    let arc_from_mean = |p: &PlotPoint| period.signed_arc(mean, p.y);
    let counterclockwise = chunk
        .iter()
        .min_by(|a, b| arc_from_mean(a).total_cmp(&arc_from_mean(b)))?;
    let clockwise = chunk
        .iter()
        .max_by(|a, b| arc_from_mean(a).total_cmp(&arc_from_mean(b)))?;
    Some((counterclockwise, clockwise))
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
    fn moderate_data_downsamples_to_coarse_levels() {
        // The cascade continues down so a short track that occupies only a few
        // pixels when zoomed out can be drawn with a handful of points.
        let m = MipMap::build(seq(50));
        assert!(
            m.level_count() > 1,
            "moderate input should produce coarse levels"
        );
        let coarsest = m.levels.last().expect("at least one level");
        assert!(
            coarsest.len() <= DOWNSAMPLE_WINDOW,
            "cascade should reduce down toward a single segment, got {}",
            coarsest.len()
        );
    }

    #[test]
    fn tiny_data_stays_single_level() {
        // Two points cannot be downsampled further (a 2-point window maps to
        // itself), so the cascade stops immediately at a lone level.
        let m = MipMap::build(vec![[0.0, 1.0], [1.0, 2.0]]);
        assert_eq!(m.level_count(), 1);
        assert_eq!(m.original_len(), 2);
    }

    #[test]
    fn large_data_produces_levels() {
        let data = seq(10_000);
        let m = MipMap::build(data);
        assert!(
            m.level_count() > 1,
            "should have at least 2 levels for 10K points"
        );
        let coarsest = m.levels.last().expect("at least one level");
        assert!(coarsest.len() >= MIN_LEVEL_POINTS);
        assert!(
            coarsest.len() <= DOWNSAMPLE_WINDOW,
            "cascade should reduce down toward a single segment, got {}",
            coarsest.len()
        );
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
        // With a small target (50) over the full range, a coarse level is
        // selected - far fewer than the 10_000 original points.
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
    //
    // These use a target larger than the data length so the finest level is
    // selected (no coarse level can meet it), making the exact integer
    // neighbours deterministic while still exercising the ±1 edge extension.
    fn int_data(n: usize) -> Vec<[f64; 2]> {
        (0..n).map(|i| [i as f64, i as f64]).collect()
    }

    /// Target that exceeds every `int_data` fixture below, forcing finest-level
    /// selection.
    const FINEST: usize = 100;

    #[test]
    fn select_slice_includes_left_neighbor() {
        let m = MipMap::build(int_data(50));
        // Viewport [10, 30]: point at x=9 must be included as the left neighbor.
        let slice = m.select_slice(10.0, 30.0, FINEST);
        assert!(
            slice.iter().any(|p| p.x as i64 == 9),
            "select_slice must include the point just left of x_min to keep the line connected"
        );
    }

    #[test]
    fn select_slice_includes_right_neighbor() {
        let m = MipMap::build(int_data(50));
        // Viewport [10, 30]: point at x=31 must be included as the right neighbor.
        let slice = m.select_slice(10.0, 30.0, FINEST);
        assert!(
            slice.iter().any(|p| p.x as i64 == 31),
            "select_slice must include the point just right of x_max to keep the line connected"
        );
    }

    #[test]
    fn select_indices_includes_left_neighbor() {
        let m = MipMap::build(int_data(50));
        let sel = m.select_indices(10.0, 30.0, FINEST);
        let slice = m.slice_at(sel);
        assert!(
            slice.iter().any(|p| p.x as i64 == 9),
            "select_indices/slice_at must include the point just left of x_min"
        );
    }

    #[test]
    fn select_indices_includes_right_neighbor() {
        let m = MipMap::build(int_data(50));
        let sel = m.select_indices(10.0, 30.0, FINEST);
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
        let slice = m.select_slice(0.0, 10.0, FINEST);
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
        let slice = m.select_slice(40.0, 49.0, FINEST);
        assert!(
            slice.iter().all(|p| p.x <= 49.0),
            "must not include out-of-range right points"
        );
        assert!(slice.last().is_some_and(|p| p.x as i64 == 49));
    }

    // Inverted-range regression tests.
    //
    // Callers sometimes derive `[x_min, x_max]` as the intersection of two
    // ranges that may not overlap at all - e.g. an active time filter and the
    // plot's visible viewport.  A naive per-side clamp of such an
    // intersection (`viewport.0.max(filter.0)`, `viewport.1.min(filter.1)`)
    // can yield `x_min > x_max` when the two ranges are disjoint.  `MipMap`
    // must treat that as a (legitimately empty) range rather than panicking
    // on an inverted slice index - see the `slice index starts at .. but ends
    // at ..` panic this guards against.
    //
    // Each case below pins both halves of the fix: `select_level_bounds`
    // must restore `clip_start <= clip_end` (the invariant `slice_at` relies
    // on - see [`Self::select_level_bounds`]), and the resulting slice must
    // stay within the original dataset.
    #[test]
    fn select_handles_inverted_and_disjoint_ranges_without_panic() {
        let small = MipMap::build(int_data(50));
        let large_n = cascading_input_len() * 4;
        let large = MipMap::build(int_data(large_n));
        let cases: [(&MipMap, f64, f64, usize, f64, &str); 3] = [
            (
                &small,
                30.0,
                10.0,
                5,
                49.0,
                "inverted range collapses to a point",
            ),
            (
                &small,
                1_000.0,
                2_000.0,
                5,
                49.0,
                "disjoint range entirely past the data",
            ),
            (
                // A multi-level cascade, so `select_level_bounds` walks
                // several levels before settling - the shape produced by
                // `viewport.max(filter_min)` / `viewport.min(filter_max)`
                // when a filter window precedes the current viewport.
                &large,
                large_n as f64 / 2.0,
                5.0,
                50,
                large_n as f64,
                "inverted range on a multi-level cascade",
            ),
        ];
        for (m, x_min, x_max, target_count, x_upper_bound, what) in cases {
            let sel = m.select_indices(x_min, x_max, target_count);
            assert!(
                sel.clip_start <= sel.clip_end,
                "{what}: select_indices must restore clip_start <= clip_end, got {sel:?}"
            );
            for slice in [m.select_slice(x_min, x_max, target_count), m.slice_at(sel)] {
                assert!(
                    slice.iter().all(|p| p.x >= 0.0 && p.x <= x_upper_bound),
                    "{what}: must not return out-of-range points, got {slice:?}"
                );
            }
        }
    }

    proptest::proptest! {
        /// `select_slice` and `select_indices`/`slice_at` must never panic and
        /// must never return points outside the original dataset, for any
        /// `(x_min, x_max)` pair - including reversed (`x_min > x_max`) and
        /// wildly out-of-range ones. This is the property that the
        /// inverted-range panic violated: two independent `.min()`/`.max()`
        /// clamps on `x_min`/`x_max` don't compose into `inner_start <=
        /// inner_end` on their own. Both APIs are driven here since
        /// `slice_at` re-clamps `clip_start`/`clip_end` independently
        /// (`.min(level.len())`) and could regress on its own.
        #[test]
        fn select_never_panics_for_arbitrary_ranges(
            x_min in -1.0e6_f64..1.0e6_f64,
            x_max in -1.0e6_f64..1.0e6_f64,
            target_count in 1_usize..100,
        ) {
            let m = MipMap::build(int_data(50));
            for slice in [
                m.select_slice(x_min, x_max, target_count),
                m.slice_at(m.select_indices(x_min, x_max, target_count)),
            ] {
                proptest::prop_assert!(slice.iter().all(|p| p.x >= 0.0 && p.x <= 49.0));
            }
        }
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

/// Downsampling a series whose values wrap, where the linear minimum and
/// maximum of a bucket sit next to each other on the circle.
#[cfg(test)]
mod wrapping_series {
    use rstest::rstest;

    use super::*;

    /// A 1 Hz series of `values`, one sample per second.
    fn sampled_per_second(values: &[f64]) -> Vec<[f64; 2]> {
        values
            .iter()
            .enumerate()
            .map(|(i, &value)| [i as f64, value])
            .collect()
    }

    /// The values of the first downsampled level, in the order they are drawn.
    fn coarse_values(mipmap: &MipMap) -> Vec<f64> {
        mipmap
            .levels
            .get(1)
            .map(|level| level.iter().map(|p| p.y).collect())
            .unwrap_or_default()
    }

    /// The mip-map preserves every bucket's outliers, and around north a
    /// southward sample is neither the linear minimum (~0°) nor the linear
    /// maximum (~359°) of its bucket.
    #[test]
    fn a_southward_sample_survives_a_bucket_of_northward_headings() {
        let mipmap = MipMap::build_wrapping(
            sampled_per_second(&[359.0, 1.0, 180.0, 2.0, 358.0, 0.0, 359.0, 1.0]),
            WrapPeriod::full_turn_degrees(),
        );
        let coarse = coarse_values(&mipmap);
        assert!(
            coarse.contains(&180.0),
            "the southward sample must survive downsampling, got {coarse:?}"
        );
    }

    /// Every sample a bucket of north jitter emits is as close to north as the
    /// samples it was given: such a bucket holds no outlier to keep.
    #[test]
    fn a_bucket_of_north_jitter_keeps_northward_samples() {
        let period = WrapPeriod::full_turn_degrees();
        let mipmap = MipMap::build_wrapping(
            sampled_per_second(&[359.0, 1.0, 0.5, 359.5, 358.5, 0.0, 1.5, 359.0]),
            period,
        );
        for value in coarse_values(&mipmap) {
            let arc_from_north = period.signed_arc(0.0, value).abs();
            assert!(
                arc_from_north <= 2.0,
                "{value}° is {arc_from_north}° from north, further than any sample"
            );
        }
    }

    /// A bucket of opposite samples keeps its linear minimum and maximum: they
    /// cancel out and leave no mean direction to measure an arc from.
    #[test]
    fn a_bucket_of_opposite_headings_keeps_its_linear_extremes() {
        let mipmap = MipMap::build_wrapping(
            sampled_per_second(&[0.0, 180.0, 0.0, 180.0, 10.0, 11.0, 12.0, 13.0]),
            WrapPeriod::full_turn_degrees(),
        );
        assert_eq!(coarse_values(&mipmap), [0.0, 180.0, 10.0, 13.0]);
    }

    /// A bucket's two largest arcs from its mean direction can both lie on the
    /// same side of it, leaving the other side undrawn. Here the mean sits at
    /// 44°, and 100° and 190° are further from it than either northward
    /// sample.
    #[test]
    fn a_bucket_keeps_the_extreme_on_each_side_of_its_mean_direction() {
        let mipmap = MipMap::build_wrapping(
            sampled_per_second(&[0.0, 0.0, 100.0, 190.0]),
            WrapPeriod::full_turn_degrees(),
        );
        assert_eq!(coarse_values(&mipmap), [0.0, 190.0]);
    }

    /// The circular rule covers a bucket whose samples all lie within one
    /// period. A bucket holding a sample outside it keeps its linear minimum
    /// and maximum: a heading past a full turn is drawn at the value the
    /// receiver wrote, 675° at 675°.
    #[rstest]
    #[case::within_one_period([359.0, 1.0, 180.0, 2.0], [359.0, 180.0])]
    #[case::past_a_full_turn([0.0, 315.0, 360.0, 675.0], [0.0, 675.0])]
    #[case::not_finite([0.0, 315.0, f64::INFINITY, 350.0], [0.0, f64::INFINITY])]
    fn a_bucket_is_downsampled_by_the_rule_its_samples_fall_under(
        #[case] headings: [f64; 4],
        #[case] expected: [f64; 2],
    ) {
        let mipmap = MipMap::build_wrapping(
            sampled_per_second(&headings),
            WrapPeriod::full_turn_degrees(),
        );
        assert_eq!(coarse_values(&mipmap), expected);
    }

    /// A period that describes no wrap.
    #[test]
    fn a_period_that_is_not_finite_and_positive_is_no_period() {
        for period in [0.0, -360.0, f64::NAN, f64::INFINITY] {
            assert_eq!(WrapPeriod::new(period), None, "{period} is no wrap period");
        }
    }

    proptest::proptest! {
        /// Every emitted point is an input sample. `select_slice`'s ±1 neighbour
        /// extension and the outlier contract both rest on that.
        #[test]
        fn every_wrapping_level_is_a_subset_of_the_finest(
            values in proptest::collection::vec(0.0_f64..360.0, 1..200),
            period_deg in 0.1_f64..1000.0,
        ) {
            let Some(period) = WrapPeriod::new(period_deg) else {
                return Err(proptest::test_runner::TestCaseError::fail(
                    format!("{period_deg} is a finite, positive period"),
                ));
            };
            let mipmap = MipMap::build_wrapping(sampled_per_second(&values), period);
            let finest = mipmap.levels.first().map_or(&[][..], Vec::as_slice);
            for level in &mipmap.levels {
                for point in level {
                    proptest::prop_assert!(
                        finest
                            .iter()
                            .any(|sample| sample.x.to_bits() == point.x.to_bits()
                                && sample.y.to_bits() == point.y.to_bits()),
                        "{point:?} is not an input sample"
                    );
                }
            }
        }

        /// A bucket holding a value outside one period keeps its linear minimum
        /// and maximum, whichever side of the period the value falls.
        #[test]
        fn a_bucket_of_out_of_range_values_keeps_its_linear_extremes(
            buckets in proptest::collection::vec(
                proptest::array::uniform4(-720.0_f64..1080.0),
                1..25,
            ),
        ) {
            let values: Vec<f64> = buckets.iter().flatten().copied().collect();
            let mipmap = MipMap::build_wrapping(
                sampled_per_second(&values),
                WrapPeriod::full_turn_degrees(),
            );
            let coarse = mipmap.levels.get(1).map_or(&[][..], Vec::as_slice);
            for (i, bucket) in buckets.iter().enumerate() {
                if bucket.iter().all(|&value| (0.0..360.0).contains(&value)) {
                    continue;
                }
                let Some(kept) = coarse.get(i * 2..i * 2 + 2) else {
                    return Err(proptest::test_runner::TestCaseError::fail(
                        format!("bucket {i} emitted no pair"),
                    ));
                };
                let mut got: Vec<f64> = kept.iter().map(|p| p.y).collect();
                got.sort_by(f64::total_cmp);
                let mut expected = bucket.to_vec();
                expected.sort_by(f64::total_cmp);
                proptest::prop_assert_eq!(
                    got,
                    vec![expected[0], expected[3]],
                    "bucket {:?} lost a linear extreme",
                    bucket
                );
            }
        }
    }
}
