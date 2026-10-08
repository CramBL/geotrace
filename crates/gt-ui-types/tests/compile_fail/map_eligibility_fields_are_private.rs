use gt_ui_types::MapEligibility;

fn value<T>() -> T {
    loop {}
}

fn main() {
    let _ = MapEligibility {
        files: value(),
        visibility: value(),
        filter: value(),
        query_matches: None,
        generated_marker_visibility: value(),
        event_marker_visibility: value(),
    };
}
