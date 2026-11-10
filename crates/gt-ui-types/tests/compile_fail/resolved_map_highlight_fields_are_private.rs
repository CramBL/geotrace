use gt_ui_types::{MapHighlight, ResolvedMapHighlight};

fn main() {
    let _ = ResolvedMapHighlight {
        highlight: &MapHighlight::default(),
        plot_hover: None,
    };
}
