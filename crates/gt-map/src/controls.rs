use egui::{DragPanButtons, Event, InputState, PointerButton};
use serde::{Deserialize, Serialize};
use strum::{Display, EnumIter};

#[derive(Clone, Copy, Debug, Default, Deserialize, Eq, PartialEq, Serialize)]
#[serde(default)]
pub struct MapControls {
    pub scroll: ScrollBehavior,
    pub double_click: DoubleClickBehavior,
    pub drag_pan: DragPanButton,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Display, EnumIter, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ScrollBehavior {
    #[strum(to_string = "Pan — Ctrl/Cmd + scroll zooms")]
    Pan,
    #[default]
    #[strum(to_string = "Zoom")]
    Zoom,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Display, EnumIter, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DoubleClickBehavior {
    #[strum(to_string = "Fit visible data")]
    FitVisible,
    #[strum(to_string = "Do nothing")]
    None,
    #[default]
    #[strum(to_string = "Center and zoom in")]
    ZoomAndCenter,
}

#[derive(Clone, Copy, Debug, Default, Deserialize, Display, EnumIter, Eq, PartialEq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DragPanButton {
    #[strum(to_string = "Middle button")]
    Middle,
    #[default]
    #[strum(to_string = "Primary button")]
    Primary,
    #[strum(to_string = "Primary or middle button")]
    PrimaryOrMiddle,
}

impl DragPanButton {
    pub(crate) fn effective_buttons(self, source: MapPointerSource) -> DragPanButtons {
        let mouse_buttons = match self {
            Self::Middle => DragPanButtons::MIDDLE,
            Self::Primary => DragPanButtons::PRIMARY,
            Self::PrimaryOrMiddle => DragPanButtons::PRIMARY | DragPanButtons::MIDDLE,
        };
        if matches!(source, MapPointerSource::Touch) {
            mouse_buttons | DragPanButtons::PRIMARY
        } else {
            mouse_buttons
        }
    }
}

#[derive(Clone, Copy, Default)]
pub(crate) enum MapPointerSource {
    #[default]
    Mouse,
    Touch,
}

impl MapPointerSource {
    pub(crate) fn update(&mut self, input: &InputState) {
        // egui-winit emits touch events before synthesized pointer events at the same position.
        let mut touch_position = None;
        for event in &input.events {
            match event {
                Event::Touch { pos, .. } => touch_position = Some(*pos),
                Event::PointerMoved(pos)
                | Event::PointerButton {
                    pos,
                    button: PointerButton::Primary,
                    ..
                } => {
                    *self = if touch_position == Some(*pos) {
                        Self::Touch
                    } else {
                        Self::Mouse
                    };
                }
                Event::PointerGone | Event::PointerButton { .. } => {
                    *self = Self::Mouse;
                    touch_position = None;
                }
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests;
