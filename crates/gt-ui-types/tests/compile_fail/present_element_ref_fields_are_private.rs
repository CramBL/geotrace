use gt_ui_types::PresentElementRef;

fn value<T>() -> T {
    loop {}
}

fn main() {
    let _ = PresentElementRef {
        element_ref: value(),
        element: value(),
    };
}
