use gt_ui_types::{MapElementRef, MapHighlight};

fn value<T>() -> T {
    loop {}
}

fn main() {
    let raw: MapElementRef = value();
    let mut highlight = MapHighlight::default();
    highlight.toggle_sticky(raw);
}
