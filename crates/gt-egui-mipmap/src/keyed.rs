use std::ops::{Range, RangeInclusive};

use egui_plot::PlotPoint;

use crate::downsample::{self, MIN_LEVEL_POINTS};
use crate::selection::{KeyBounds, LevelSelection};
use crate::wrap::WrapPeriod;

#[derive(Clone, Copy, Debug)]
pub struct KeyedSample<K, M = ()> {
    pub point: [f64; 2],
    pub key: K,
    pub meta: M,
}

impl<K> KeyedSample<K, ()> {
    pub const fn new(point: [f64; 2], key: K) -> Self {
        Self {
            point,
            key,
            meta: (),
        }
    }
}

impl<K, M> KeyedSample<K, M> {
    pub const fn with_meta(point: [f64; 2], key: K, meta: M) -> Self {
        Self { point, key, meta }
    }
}

/// Mipmap cascade storing an ascending clipping key and optional per-sample
/// metadata in flat parallel buffers alongside each [`PlotPoint`].
#[derive(Clone, Debug)]
pub struct KeyedMipMap<K, M = ()> {
    points: Vec<PlotPoint>,
    keys: Vec<K>,
    meta: Vec<M>,
    level_ranges: Vec<Range<usize>>,
}

impl<K: Copy + Ord, M: Clone> KeyedMipMap<K, M> {
    /// Input order is ascending by plot `x` and by `key`.
    pub fn build(samples: impl IntoIterator<Item = KeyedSample<K, M>>) -> Self {
        Self::build_with_period(samples, None)
    }

    /// Input order is ascending by plot `x` and by `key`.
    pub fn build_with_period(
        samples: impl IntoIterator<Item = KeyedSample<K, M>>,
        period: Option<WrapPeriod>,
    ) -> Self {
        let iter = samples.into_iter();
        let initial_capacity = downsample::initial_buffer_capacity(iter.size_hint().0);
        let mut points = Vec::with_capacity(initial_capacity);
        let mut keys = Vec::with_capacity(initial_capacity);
        let mut meta = Vec::with_capacity(initial_capacity);
        for sample in iter {
            points.push(PlotPoint::from(sample.point));
            keys.push(sample.key);
            meta.push(sample.meta);
        }
        let finest_len = points.len();
        if finest_len == 0 {
            return Self::default();
        }
        if finest_len > MIN_LEVEL_POINTS {
            keys.reserve(finest_len);
            meta.reserve(finest_len);
        }
        let level_ranges = downsample::build_cascade(&mut points, period, |src| {
            if let (Some(&key), Some(item_meta)) = (keys.get(src), meta.get(src).cloned()) {
                keys.push(key);
                meta.push(item_meta);
            }
        });
        keys.truncate(points.len());
        meta.truncate(points.len());
        Self {
            points,
            keys,
            meta,
            level_ranges,
        }
    }
}

impl<K, M> KeyedMipMap<K, M> {
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

    pub fn slice_at(&self, sel: LevelSelection) -> &[PlotPoint] {
        let Some(level) = self.level_points(sel.level_idx) else {
            return &[];
        };
        sel.slice_points(level)
    }

    pub fn meta_at(&self, sel: LevelSelection, offset: usize) -> Option<&M> {
        let range = self.level_ranges.get(sel.level_idx)?;
        let index = sel.element_index(offset)?;
        self.meta.get(range.clone())?.get(index)
    }

    fn level_points(&self, level_idx: usize) -> Option<&[PlotPoint]> {
        let range = self.level_ranges.get(level_idx)?;
        self.points.get(range.clone())
    }

    fn level_keys(&self, level_idx: usize) -> Option<&[K]> {
        let range = self.level_ranges.get(level_idx)?;
        self.keys.get(range.clone())
    }

    #[cfg(test)]
    pub(crate) fn level_slice(&self, level_idx: usize) -> Option<(&[PlotPoint], &[K])> {
        Some((self.level_points(level_idx)?, self.level_keys(level_idx)?))
    }
}

impl<K: Copy + Ord, M> KeyedMipMap<K, M> {
    pub fn key_at(&self, sel: LevelSelection, offset: usize) -> Option<K> {
        let range = self.level_ranges.get(sel.level_idx)?;
        let index = sel.element_index(offset)?;
        self.keys.get(range.clone())?.get(index).copied()
    }

    pub fn select_slice(
        &self,
        viewport: RangeInclusive<f64>,
        bounds: KeyBounds<K>,
        target_count: usize,
    ) -> &[PlotPoint] {
        self.slice_at(self.select_indices(viewport, bounds, target_count))
    }

    pub fn select_indices(
        &self,
        viewport: RangeInclusive<f64>,
        bounds: KeyBounds<K>,
        target_count: usize,
    ) -> LevelSelection {
        if self.level_ranges.is_empty() || bounds.is_empty() {
            return LevelSelection::default();
        }
        let x_min = *viewport.start();
        let x_max = (*viewport.end()).max(x_min);
        for level_idx in (0..self.level_ranges.len()).rev() {
            let (Some(points), Some(keys)) =
                (self.level_points(level_idx), self.level_keys(level_idx))
            else {
                continue;
            };
            let vp_start = points.partition_point(|point| point.x < x_min);
            let vp_end = points
                .partition_point(|point| point.x <= x_max)
                .max(vp_start);
            if vp_end - vp_start < target_count && level_idx > 0 {
                continue;
            }
            let inside = bounds.clip_subslice(keys, vp_start..vp_end);
            if inside.len() >= target_count || level_idx == 0 {
                let clip_start = if inside.start == vp_start
                    && let Some(prev) = inside.start.checked_sub(1)
                    && keys.get(prev).is_some_and(|&key| bounds.contains(key))
                {
                    prev
                } else {
                    inside.start
                };
                let clip_end = if inside.end == vp_end
                    && keys
                        .get(inside.end)
                        .is_some_and(|&key| bounds.contains(key))
                {
                    inside.end + 1
                } else {
                    inside.end
                };
                return LevelSelection {
                    level_idx,
                    clip_start,
                    clip_end,
                };
            }
        }
        LevelSelection::default()
    }

    pub fn has_samples_in(&self, bounds: KeyBounds<K>) -> bool {
        if bounds.is_empty() {
            return false;
        }
        let Some(keys) = self.level_keys(0) else {
            return false;
        };
        let (Some(&first), Some(&last)) = (keys.first(), keys.last()) else {
            return false;
        };
        if bounds.is_after(first) || bounds.is_before(last) {
            return false;
        }
        if bounds.contains(first) || bounds.contains(last) {
            return true;
        }
        let idx = keys.partition_point(|&key| bounds.is_before(key));
        keys.get(idx).is_some_and(|&key| bounds.contains(key))
    }

    pub fn clipped_x_range(&self, bounds: KeyBounds<K>) -> Option<(f64, f64)> {
        if bounds.is_empty() {
            return None;
        }
        let points = self.level_points(0)?;
        let keys = self.level_keys(0)?;
        let range = bounds.clip_subslice(keys, 0..keys.len());
        let clipped = points.get(range)?;
        Some((clipped.first()?.x, clipped.last()?.x))
    }
}

impl<K, M> Default for KeyedMipMap<K, M> {
    fn default() -> Self {
        Self {
            points: Vec::new(),
            keys: Vec::new(),
            meta: Vec::new(),
            level_ranges: Vec::new(),
        }
    }
}
