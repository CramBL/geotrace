use std::ops::{Range, RangeInclusive};

use egui_plot::PlotPoint;

/// Viewport and hard-bound x range for [`crate::MipMap::select_indices`].
///
/// The selection extends one point past each viewport edge where the level has
/// one so lines stay connected across the viewport edge. The selection never
/// includes a point outside the hard bound.
#[derive(Clone, Copy, Debug)]
pub struct SelectionRange {
    viewport_min: f64,
    viewport_max: f64,
    hard_min: f64,
    hard_max: f64,
}

impl SelectionRange {
    pub fn within_viewport(viewport: RangeInclusive<f64>) -> Self {
        Self {
            viewport_min: *viewport.start(),
            viewport_max: *viewport.end(),
            hard_min: f64::NEG_INFINITY,
            hard_max: f64::INFINITY,
        }
    }

    pub fn bounded_by(self, hard_bound: RangeInclusive<f64>) -> Self {
        Self {
            hard_min: *hard_bound.start(),
            hard_max: *hard_bound.end(),
            ..self
        }
    }

    /// Viewport span intersected with the hard bound, collapsed to a single x
    /// when inverted or disjoint.
    pub(crate) fn counted_span(self) -> (f64, f64) {
        let min = self.viewport_min.max(self.hard_min);
        (min, self.viewport_max.min(self.hard_max).max(min))
    }

    pub(crate) fn hard_indices(self, level: &[PlotPoint]) -> Range<usize> {
        let start = level.partition_point(|point| point.x < self.hard_min);
        let end = level.partition_point(|point| point.x <= self.hard_max);
        start..end.max(start)
    }
}

/// Cached level index and clipped sub-range for O(1) slicing via `slice_at`.
#[derive(Clone, Copy, Debug, Default)]
pub struct LevelSelection {
    pub(crate) level_idx: usize,
    pub(crate) clip_start: usize,
    pub(crate) clip_end: usize,
}

impl LevelSelection {
    /// Whether this selection reads the finest (original) level.
    pub fn is_full_detail(&self) -> bool {
        self.level_idx == 0
    }

    pub(crate) fn slice_points(self, level: &[PlotPoint]) -> &[PlotPoint] {
        let start = self.clip_start.min(level.len());
        let end = self.clip_end.min(level.len()).max(start);
        level.get(start..end).unwrap_or_default()
    }

    pub(crate) fn element_index(self, offset: usize) -> Option<usize> {
        let index = self.clip_start.checked_add(offset)?;
        (index < self.clip_end).then_some(index)
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum KeyBounds<K> {
    #[default]
    All,
    Between {
        start: K,
        end: K,
    },
    Empty,
    From(K),
    Until(K),
}

impl<K: Copy + Ord> KeyBounds<K> {
    pub fn from_bounds(bounds: Option<(Option<K>, Option<K>)>) -> Self {
        match bounds {
            None => Self::Empty,
            Some((None, None)) => Self::All,
            Some((Some(start), None)) => Self::From(start),
            Some((None, Some(end))) => Self::Until(end),
            Some((Some(start), Some(end))) => Self::inclusive(start..=end),
        }
    }

    pub fn inclusive(range: RangeInclusive<K>) -> Self {
        let (start, end) = range.into_inner();
        if start > end {
            Self::Empty
        } else {
            Self::Between { start, end }
        }
    }

    pub fn is_empty(self) -> bool {
        match self {
            Self::Empty => true,
            Self::Between { start, end } => start > end,
            Self::All | Self::From(_) | Self::Until(_) => false,
        }
    }

    pub fn contains(self, key: K) -> bool {
        match self {
            Self::All => true,
            Self::Between { start, end } => key >= start && key <= end,
            Self::Empty => false,
            Self::From(start) => key >= start,
            Self::Until(end) => key <= end,
        }
    }

    pub(crate) fn is_before(self, key: K) -> bool {
        match self {
            Self::Between { start, .. } | Self::From(start) => key < start,
            Self::All | Self::Empty | Self::Until(_) => false,
        }
    }

    pub(crate) fn is_after(self, key: K) -> bool {
        match self {
            Self::Between { end, .. } | Self::Until(end) => key > end,
            Self::All | Self::Empty | Self::From(_) => false,
        }
    }

    pub(crate) fn clip_subslice(self, keys: &[K], viewport: Range<usize>) -> Range<usize> {
        if matches!(self, Self::All) || viewport.start == viewport.end {
            return viewport;
        }
        let Some(sub_keys) = keys.get(viewport.clone()) else {
            return viewport.start..viewport.start;
        };
        let (Some(&first), Some(&last)) = (sub_keys.first(), sub_keys.last()) else {
            return viewport.start..viewport.start;
        };
        if self.is_after(first) {
            return viewport.start..viewport.start;
        }
        if self.is_before(last) {
            return viewport.end..viewport.end;
        }
        let in_start = if self.is_before(first) {
            viewport.start + sub_keys.partition_point(|&key| self.is_before(key))
        } else {
            viewport.start
        };
        let in_end = if self.is_after(last) {
            viewport.start + sub_keys.partition_point(|&key| !self.is_after(key))
        } else {
            viewport.end
        };
        in_start..in_end.max(in_start)
    }
}
