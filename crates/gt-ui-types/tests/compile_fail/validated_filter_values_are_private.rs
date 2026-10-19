use gt_filter::{BoundedTimeWindow, MinimumDistance, MinimumDuration, MinimumSpread};

fn value<T>() -> T { loop {} }

fn main() {
    let _ = BoundedTimeWindow(value());
    let _ = MinimumDistance(value());
    let _ = MinimumDuration(value());
    let _ = MinimumSpread(value());
}
