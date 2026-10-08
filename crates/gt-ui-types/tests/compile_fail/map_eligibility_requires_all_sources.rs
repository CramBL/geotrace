use gt_ui_types::MapEligibility;

fn value<T>() -> T {
    loop {}
}

fn main() {
    let _ = MapEligibility::new(value(), value(), value(), None, value());
}
