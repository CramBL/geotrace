use std::ops::Range;

#[derive(Clone, Copy)]
pub(super) struct RowGrid {
    pub(super) height: f32,
}

impl RowGrid {
    pub(super) fn offset(self, row: usize) -> f32 {
        row as f32 * self.height
    }

    #[expect(clippy::cast_sign_loss, reason = "The cast input is nonnegative")]
    pub(super) fn row_at(self, offset: f32) -> usize {
        (offset.max(0.0) / self.height).floor() as usize
    }

    pub(super) fn offset_within_row(self, offset: f32) -> f32 {
        offset.max(0.0) % self.height
    }
}

pub(super) struct ShownRows {
    pub(super) grid: RowGrid,
    pub(super) rows: Range<usize>,
    pub(super) bounds: egui::Rect,
}

impl ShownRows {
    pub(super) fn screen_y(&self, offset: f32) -> f32 {
        self.bounds.top() + (offset - self.grid.offset(self.rows.start))
    }

    pub(super) fn row_top(&self, row: usize) -> f32 {
        let distance = self.grid.offset(row.abs_diff(self.rows.start));
        if row < self.rows.start {
            self.bounds.top() - distance
        } else {
            self.bounds.top() + distance
        }
    }

    pub(super) fn row_bounds(&self, row: usize) -> egui::Rect {
        let top = self.row_top(row);
        egui::Rect::from_x_y_ranges(self.bounds.x_range(), top..=top + self.grid.height)
    }

    /// The bottom edge of the clip range is exclusive.
    #[expect(
        clippy::cast_sign_loss,
        reason = "The intersecting row count is nonnegative"
    )]
    pub(super) fn hit_row(&self, clip: egui::Rangef, pointer_y: f32) -> Option<usize> {
        let top = self.bounds.top();
        let intersecting = ((clip.max - top).max(0.0) / self.grid.height).ceil() as usize;
        let last = intersecting.min(self.rows.len()).checked_sub(1)?;
        let row = self.grid.row_at(pointer_y.clamp(clip.min, clip.max) - top);
        Some(self.rows.start + row.min(last))
    }
}

#[cfg(test)]
mod tests {
    use super::{RowGrid, ShownRows};

    #[test]
    fn row_bounds_preserve_local_offsets_at_large_row_indices() {
        let first = 1 << f32::MANTISSA_DIGITS;
        let shown = ShownRows {
            grid: RowGrid { height: 20.0 },
            rows: first..first + 2,
            bounds: egui::Rect::from_min_size(egui::pos2(0.0, 30.0), egui::vec2(100.0, 40.0)),
        };

        assert_eq!(shown.row_bounds(first - 1).top(), 10.0);
        assert_eq!(shown.row_bounds(first + 1).top(), 50.0);
    }

    #[test]
    fn reading_offset_stays_within_one_row_at_large_scroll_offsets() {
        let grid = RowGrid { height: 20.0 };
        let offset = grid.offset(1 << f32::MANTISSA_DIGITS) + 96.0;
        assert_eq!(grid.offset_within_row(offset), 16.0);
    }
}
