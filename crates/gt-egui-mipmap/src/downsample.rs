use std::iter;
use std::ops::Range;

use egui_plot::PlotPoint;

use crate::wrap::WrapPeriod;

pub(crate) fn initial_buffer_capacity(hint: usize) -> usize {
    if hint > MIN_LEVEL_POINTS {
        hint.saturating_mul(2)
    } else {
        hint
    }
}

pub(crate) fn build_cascade(
    points: &mut Vec<PlotPoint>,
    period: Option<WrapPeriod>,
    mut on_keep: impl FnMut(usize),
) -> Vec<Range<usize>> {
    let finest_len = points.len();
    if finest_len == 0 {
        return Vec::new();
    }
    if finest_len > MIN_LEVEL_POINTS {
        points.reserve(finest_len);
    }
    let level_capacity = (usize::BITS - finest_len.leading_zeros()) as usize + 1;
    let mut level_ranges = Vec::with_capacity(level_capacity);
    level_ranges.push(0..finest_len);

    let mut prev_range = 0..finest_len;
    while prev_range.len() > MIN_LEVEL_POINTS {
        let next_start = points.len();
        let mut cursor = prev_range.start;
        while cursor < prev_range.end {
            let chunk_end = (cursor + DOWNSAMPLE_WINDOW).min(prev_range.end);
            let Some(chunk) = points.get(cursor..chunk_end) else {
                break;
            };
            if let Some((first, second)) = window_extreme_offsets(chunk, period) {
                for offset in iter::once(first).chain(second) {
                    let src = cursor + offset;
                    if let Some(&point) = points.get(src) {
                        points.push(point);
                        on_keep(src);
                    }
                }
            }
            cursor = chunk_end;
        }
        let next_range = next_start..points.len();
        if next_range.len() < MIN_LEVEL_POINTS || next_range.len() >= prev_range.len() {
            points.truncate(next_start);
            break;
        }
        level_ranges.push(next_range.clone());
        prev_range = next_range;
    }
    level_ranges
}

/// One or two offsets within `chunk` kept by downsampling, in ascending input
/// order so equal-`x` samples preserve their original sequence.
fn window_extreme_offsets(
    chunk: &[PlotPoint],
    period: Option<WrapPeriod>,
) -> Option<(usize, Option<usize>)> {
    let [first, second] = period
        .and_then(|period| period.extremes_either_side_of_mean_direction(chunk))
        .or_else(|| {
            let (min_idx, _) = chunk
                .iter()
                .enumerate()
                .min_by(|(_, a), (_, b)| a.y.total_cmp(&b.y))?;
            let (max_idx, _) = chunk
                .iter()
                .enumerate()
                .max_by(|(_, a), (_, b)| a.y.total_cmp(&b.y))?;
            Some([min_idx, max_idx])
        })?;
    let [first, second] = if first <= second {
        [first, second]
    } else {
        [second, first]
    };
    Some((first, (first != second).then_some(second)))
}

/// Smallest mipmap level built by the cascade: two points (one line segment).
pub(crate) const MIN_LEVEL_POINTS: usize = 2;

/// Input points grouped into one min/max pair per downsampling step (~2x
/// reduction per level).
pub(crate) const DOWNSAMPLE_WINDOW: usize = 4;
