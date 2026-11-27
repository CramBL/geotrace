use std::ops::Range;

use egui_plot::PlotPoint;

use crate::downsample;
use crate::selection::{LevelSelection, SelectionRange};
use crate::wrap::WrapPeriod;

/// Cascade of progressively downsampled `[x, y]` levels stored in one
/// contiguous buffer for zero-copy [`egui_plot::PlotPoints::Borrowed`] slices.
#[derive(Clone, Debug, Default)]
pub struct MipMap {
    points: Vec<PlotPoint>,
    level_ranges: Vec<Range<usize>>,
}

impl MipMap {
    /// Input order is ascending by `x`.
    pub fn build(data: impl IntoIterator<Item = [f64; 2]>) -> Self {
        Self::build_levels(data, None)
    }

    /// Input order is ascending by `x`. Windows whose samples lie in
    /// `[0, period)` keep the two points furthest to either side of the
    /// window's circular mean direction. Other windows keep linear extremes.
    pub fn build_wrapping(data: impl IntoIterator<Item = [f64; 2]>, period: WrapPeriod) -> Self {
        Self::build_levels(data, Some(period))
    }

    fn build_levels(data: impl IntoIterator<Item = [f64; 2]>, period: Option<WrapPeriod>) -> Self {
        let iter = data.into_iter();
        let mut points =
            Vec::with_capacity(downsample::initial_buffer_capacity(iter.size_hint().0));
        points.extend(iter.map(PlotPoint::from));
        let level_ranges = downsample::build_cascade(&mut points, period, |_| {});
        Self {
            points,
            level_ranges,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.level_ranges.is_empty()
    }

    pub fn x_range(&self) -> Option<(f64, f64)> {
        let level = self.level_points(0)?;
        Some((level.first()?.x, level.last()?.x))
    }

    pub fn finest_level(&self) -> &[PlotPoint] {
        self.level_points(0).unwrap_or_default()
    }

    pub fn original_len(&self) -> usize {
        self.finest_level().len()
    }

    pub fn level_count(&self) -> usize {
        self.level_ranges.len()
    }

    pub fn select_slice(&self, range: SelectionRange, target_count: usize) -> &[PlotPoint] {
        self.slice_at(self.select_indices(range, target_count))
    }

    pub fn select_indices(&self, range: SelectionRange, target_count: usize) -> LevelSelection {
        let (x_min, x_max) = range.counted_span();
        for level_idx in (0..self.level_ranges.len()).rev() {
            let Some(level) = self.level_points(level_idx) else {
                continue;
            };
            let start = level.partition_point(|point| point.x < x_min);
            let end = level.partition_point(|point| point.x <= x_max).max(start);
            if end - start >= target_count || level_idx == 0 {
                let clip = range.hard_indices(level);
                return LevelSelection {
                    level_idx,
                    clip_start: start.saturating_sub(1).clamp(clip.start, clip.end),
                    clip_end: end.saturating_add(1).clamp(clip.start, clip.end),
                };
            }
        }
        LevelSelection::default()
    }

    /// `sel` must come from this [`MipMap`] at its current generation.
    pub fn slice_at(&self, sel: LevelSelection) -> &[PlotPoint] {
        let Some(level) = self.level_points(sel.level_idx) else {
            return &[];
        };
        sel.slice_points(level)
    }

    fn level_points(&self, level_idx: usize) -> Option<&[PlotPoint]> {
        let range = self.level_ranges.get(level_idx)?;
        self.points.get(range.clone())
    }

    #[cfg(test)]
    pub(crate) fn level(&self, level_idx: usize) -> Option<&[PlotPoint]> {
        self.level_points(level_idx)
    }

    #[cfg(test)]
    pub(crate) fn coarsest_level(&self) -> Option<&[PlotPoint]> {
        let last_idx = self.level_ranges.len().checked_sub(1)?;
        self.level_points(last_idx)
    }
}
