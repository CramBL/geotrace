use gt_filter::{GlobalFilter, TimeWindow};

fn main() {
    let mut filter = GlobalFilter::default();
    filter.time_window = TimeWindow::Empty;
    filter.minimum_distance = None;
    filter.minimum_duration = None;
    filter.minimum_spread = None;
}
