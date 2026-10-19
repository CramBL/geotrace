use gt_ui_types::EligibleElementRef;

fn value<T>() -> T { loop {} }

fn main() {
    let _ = EligibleElementRef { element_ref: value(), element: value() };
}
