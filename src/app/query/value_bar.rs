//! The bar behind a value cell of the points table: how far along its column's
//! range the value sits, and how that bar is painted.

use egui::{Color32, emath};

use super::results::ROW_PADDING;

/// How round the ends of a value bar are.
const BAR_CORNER_RADIUS_PX: f32 = 2.0;

/// The lowest and highest value one column of the points table takes over every
/// matched row of its query in the run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(super) struct ColumnValueRange {
    lowest: f64,
    highest: f64,
}

impl ColumnValueRange {
    /// The range `values` span, absent where none of them is finite.
    pub(super) fn of_values(values: impl IntoIterator<Item = f64>) -> Option<Self> {
        values.into_iter().filter(|value| value.is_finite()).fold(
            None,
            |range: Option<Self>, value| {
                Some(match range {
                    Some(range) => Self {
                        lowest: range.lowest.min(value),
                        highest: range.highest.max(value),
                    },
                    None => Self {
                        lowest: value,
                        highest: value,
                    },
                })
            },
        )
    }

    /// How much of the cell the bar behind `value` fills: nothing at the
    /// column's lowest value, all of it at its highest. A column whose values
    /// are all the same spans nothing to place a value in and returns [`None`].
    pub(super) fn bar_fraction(self, value: Option<f64>) -> Option<f32> {
        emath::inverse_lerp(self.lowest..=self.highest, value?)
            .map(|fraction| (fraction as f32).clamp(0.0, 1.0))
    }
}

/// The bar painted behind one value cell, in the halo colour of the query that
/// matched the row.
#[derive(Debug, Clone, Copy)]
pub(super) struct ValueBar {
    /// How much of the cell's width the bar fills, in `[0, 1]`.
    fraction: f32,
    color: Color32,
}

impl ValueBar {
    pub(super) fn new(fraction: f32, color: Color32) -> Self {
        Self { fraction, color }
    }

    /// Fill the cell from its left edge, as high as the text between the
    /// padding a row adds above and below it.
    pub(super) fn paint(self, ui: &egui::Ui) {
        let cell = ui.max_rect().shrink2(egui::vec2(0.0, ROW_PADDING / 2.0));
        let bar = egui::Rect::from_min_size(
            cell.min,
            egui::vec2(cell.width() * self.fraction, cell.height()),
        );
        ui.painter()
            .rect_filled(bar, BAR_CORNER_RADIUS_PX, self.color);
    }
}

/// The value range every column of a run spans, computed once for the run that
/// produced them. Picking another match of a query leaves its bars at the
/// scale they had: the range covers every match of that query.
#[derive(Debug, Default)]
pub(super) struct RunColumnRanges {
    /// The run the ranges below were computed from. The first run of a session
    /// always recomputes them: no run is numbered 0.
    run: u64,
    /// One entry per query of the run in editor order, holding one range per
    /// column of that query. A column of times or blanks holds none.
    queries: Vec<Vec<Option<ColumnValueRange>>>,
}

impl RunColumnRanges {
    /// Compute the ranges of `run` unless they are the ones already held.
    pub(super) fn refresh(
        &mut self,
        run: u64,
        compute: impl FnOnce() -> Vec<Vec<Option<ColumnValueRange>>>,
    ) {
        if self.run == run {
            return;
        }
        self.run = run;
        self.queries = compute();
    }

    /// The ranges of the columns the query at `query_index` tables, empty for a
    /// query missing from the run.
    pub(super) fn of_query(&self, query_index: usize) -> &[Option<ColumnValueRange>] {
        self.queries.get(query_index).map_or(&[], Vec::as_slice)
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    /// A value's bar fills the cell in proportion to where the value sits
    /// between the lowest and the highest its column takes.
    #[rstest]
    #[case::lowest(Some(-4.0), Some(0.0))]
    #[case::middle(Some(3.0), Some(0.5))]
    #[case::highest(Some(10.0), Some(1.0))]
    // The bar clamps to the cell width for a value outside the column's range.
    #[case::above_the_highest(Some(24.0), Some(1.0))]
    #[case::below_the_lowest(Some(-9.0), Some(0.0))]
    #[case::without_a_value(None, None)]
    fn a_bar_fills_the_cell_in_proportion_to_the_value(
        #[case] value: Option<f64>,
        #[case] expected: Option<f32>,
    ) {
        let range = ColumnValueRange::of_values([3.0, -4.0, 10.0]).expect("finite values");
        assert_eq!(range.bar_fraction(value), expected);
    }

    #[test]
    fn a_column_whose_values_are_all_the_same_paints_no_bar() {
        let range = ColumnValueRange::of_values([7.5, 7.5]).expect("finite values");
        assert_eq!(range.bar_fraction(Some(7.5)), None);
    }

    #[test]
    fn a_column_without_a_finite_value_spans_no_range() {
        assert_eq!(ColumnValueRange::of_values(std::iter::empty()), None);
        assert_eq!(ColumnValueRange::of_values([f64::NAN, f64::INFINITY]), None);
        assert_eq!(
            ColumnValueRange::of_values([f64::NAN, 2.0, 5.0]),
            Some(ColumnValueRange {
                lowest: 2.0,
                highest: 5.0
            })
        );
    }

    /// The ranges are computed once per run: another frame of the same run
    /// reads the ones held, and a rerun stamps a new number and recomputes.
    #[test]
    fn the_ranges_are_computed_once_per_run() {
        let mut ranges = RunColumnRanges::default();
        let mut computed = 0;
        let of_highest = |highest: f64| {
            vec![vec![Some(ColumnValueRange {
                lowest: 0.0,
                highest,
            })]]
        };
        let highest_held = |ranges: &RunColumnRanges| {
            ranges
                .of_query(0)
                .first()
                .copied()
                .flatten()
                .map(|range| range.highest)
        };

        ranges.refresh(1, || {
            computed += 1;
            of_highest(10.0)
        });
        ranges.refresh(1, || {
            computed += 1;
            of_highest(20.0)
        });
        assert_eq!(computed, 1);
        assert_eq!(highest_held(&ranges), Some(10.0));

        ranges.refresh(2, || {
            computed += 1;
            of_highest(20.0)
        });
        assert_eq!(computed, 2);
        assert_eq!(highest_held(&ranges), Some(20.0));
        assert!(ranges.of_query(1).is_empty());
    }
}
