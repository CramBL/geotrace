use std::mem;

use egui::{Key, PointerButton};
use gt_ui_types::LogMatchGlyph;

use crate::controls::DoubleClickBehavior;
use crate::hover_candidates::HoverCandidates;

pub(crate) struct PrimaryMapClick {
    pub(crate) position: egui::Pos2,
    pub(crate) candidates: HoverCandidates,
    pub(crate) log_glyph: Option<LogMatchGlyph>,
}

pub(crate) enum MapClickGesture {
    Double,
    None,
    Single(PrimaryMapClick),
}

#[derive(Default)]
pub(crate) enum PrimaryClickState {
    #[default]
    Idle,
    Pending {
        deadline: f64,
        click: PrimaryMapClick,
    },
}

impl PrimaryClickState {
    pub(crate) fn classify(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        candidates: HoverCandidates,
        log_glyph: Option<&LogMatchGlyph>,
        double_click: DoubleClickBehavior,
    ) -> MapClickGesture {
        if double_click != DoubleClickBehavior::None
            && response.double_clicked_by(PointerButton::Primary)
        {
            *self = Self::Idle;
            return MapClickGesture::Double;
        }

        if ui.input(|input| input.key_pressed(Key::Escape)) {
            *self = Self::Idle;
            return MapClickGesture::None;
        }

        let now = ui.input(|input| input.time);
        let previous = mem::take(self);
        if response.clicked_by(PointerButton::Primary) {
            let click = PrimaryMapClick {
                position: response
                    .interact_pointer_pos()
                    .unwrap_or(response.rect.center()),
                candidates,
                log_glyph: log_glyph.cloned(),
            };
            if double_click == DoubleClickBehavior::None {
                return MapClickGesture::Single(click);
            }
            let delay = ui
                .ctx()
                .options(|options| options.input_options.max_double_click_delay);
            *self = Self::Pending {
                deadline: now + delay,
                click,
            };
            ui.ctx().request_repaint_after_secs(delay as f32);
            return match previous {
                Self::Idle => MapClickGesture::None,
                Self::Pending { click, .. } => MapClickGesture::Single(click),
            };
        }

        match previous {
            Self::Pending { deadline, click } if now >= deadline => MapClickGesture::Single(click),
            Self::Pending { deadline, click } => {
                ui.ctx().request_repaint_after_secs((deadline - now) as f32);
                *self = Self::Pending { deadline, click };
                MapClickGesture::None
            }
            Self::Idle => MapClickGesture::None,
        }
    }
}

#[cfg(test)]
mod tests;
