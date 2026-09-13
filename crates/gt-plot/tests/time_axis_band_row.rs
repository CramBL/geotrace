//! The row of calendar bands under the plot's tick labels: which bands a view gets a label
//! for, and where the row sits under the plot frame.

use gt_plot::PlotState;
use support::{DrawnPlot, PlotSources};

mod support;

/// Fixes ten minutes apart over a day and a half from the first fix.
const FIX_COUNT: usize = 217;

const FIX_STEP_SECS: i64 = 600;

/// Seconds from the first fix to the end of the view. The view holds one midnight, with half
/// a day on either side of it: the first fix is at 12:00 UTC.
const VIEW_END_SECS: i64 = 24 * 60 * 60;

fn drawn_across_a_midnight() -> DrawnPlot {
    let files = vec![support::recording(
        support::fixes(FIX_COUNT, FIX_STEP_SECS),
        Vec::new(),
    )];
    support::drawn_plot(
        files,
        PlotSources::default().pinned_to_map_view(0..=VIEW_END_SECS),
        PlotState::default(),
    )
}

#[test]
fn a_view_across_a_midnight_labels_the_day_on_either_side_of_it() {
    let plot = drawn_across_a_midnight();

    let dates: Vec<String> = plot
        .painted_texts()
        .into_iter()
        .filter(|text| text.starts_with("2024-"))
        .collect();

    assert_eq!(dates, ["2024-01-15", "2024-01-16"]);
}

#[test]
fn snapshot_the_band_row_draws_under_the_tick_row() {
    drawn_across_a_midnight().snapshot("time_axis_band_row");
}
