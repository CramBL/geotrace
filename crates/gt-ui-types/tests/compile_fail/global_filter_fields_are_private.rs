use gt_filter::{GlobalFilter, TimeWindow};
use gt_types::MarkerRequirement;

fn main() {
    let _ = GlobalFilter {
        time_window: TimeWindow::All,
        minimum_distance: None,
        minimum_duration: None,
        minimum_spread: None,
        marker_requirement: MarkerRequirement::default(),
    };
}
