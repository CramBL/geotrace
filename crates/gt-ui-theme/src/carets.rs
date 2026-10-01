use egui_phosphor::regular::CARET_DOWN as ICON_CARET_DOWN;
use egui_phosphor::regular::CARET_RIGHT as ICON_CARET_RIGHT;

pub fn expand_arrow(expanded: bool) -> &'static str {
    if expanded {
        ICON_CARET_DOWN
    } else {
        ICON_CARET_RIGHT
    }
}
