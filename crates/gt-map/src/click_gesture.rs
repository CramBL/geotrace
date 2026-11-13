use egui::{Key, PointerButton};
use gt_ui_types::{LogMatchGlyph, MapElementRef};
use walkers::{MapMemory, Position};

use crate::controls::DoubleClickBehavior;
use crate::hover_candidates::HoverCandidates;

pub(crate) struct PrimaryMapClick {
    pub(crate) position: egui::Pos2,
    pub(crate) candidates: HoverCandidates,
}

pub(crate) enum MapClickGesture {
    Double {
        position: egui::Pos2,
        restore: ClickSelection,
    },
    None,
    Single(PrimaryMapClick),
}

pub(crate) struct MapClickContext<'a> {
    pub(crate) candidates: HoverCandidates,
    pub(crate) log_glyph: Option<&'a LogMatchGlyph>,
    pub(crate) double_click: DoubleClickBehavior,
    pub(crate) selection: ClickSelection,
    pub(crate) camera: &'a MapMemory,
}

pub(crate) struct ClassifiedMapClick {
    pub(crate) gesture: MapClickGesture,
    pub(crate) confirmed_log: Option<LogMatchGlyph>,
}

#[derive(Clone, Copy, PartialEq)]
pub(crate) struct ClickSelection {
    pub(crate) sticky: Option<MapElementRef>,
    pub(crate) sticky_pos: egui::Pos2,
    pub(crate) candidates: HoverCandidates,
    pub(crate) disambiguation_pos: egui::Pos2,
}

#[derive(Default)]
pub(crate) struct PrimaryClickState {
    pending: Option<PendingClick>,
}

struct PendingClick {
    deadline: f64,
    position: egui::Pos2,
    before: ClickSelection,
    after: ClickSelection,
    camera: ClickCamera,
    log_glyph: Option<LogMatchGlyph>,
}

#[derive(PartialEq)]
struct ClickCamera {
    center: Option<Position>,
    zoom_bits: u64,
}

impl ClickCamera {
    fn of(memory: &MapMemory) -> Self {
        Self {
            center: memory.detached(),
            zoom_bits: memory.zoom().to_bits(),
        }
    }
}

impl PendingClick {
    fn receives_popup_double_click(&self, ui: &egui::Ui, response: &egui::Response) -> bool {
        let Some(position) = ui.input(|input| input.pointer.interact_pos()) else {
            return false;
        };
        let layer = if self.after.candidates.is_ambiguous() {
            Some(egui::LayerId::new(
                egui::Order::Foreground,
                egui::Id::unique(DISAMBIGUATION_POPUP_ID),
            ))
        } else {
            self.after
                .sticky
                .filter(|sticky| Some(*sticky) != self.before.sticky)
                .map(|sticky| {
                    egui::LayerId::new(
                        egui::Order::Middle,
                        egui::Id::unique((PINNED_POPUP_ID, sticky)),
                    )
                })
        };
        let max_click_dist = ui
            .ctx()
            .options(|options| options.input_options.max_click_dist);
        response.rect.contains(position)
            && layer.is_some_and(|layer| ui.ctx().layer_id_at(position) == Some(layer))
            && position.distance_sq(self.position) < max_click_dist.powi(2)
            && ui.input(|input| input.pointer.button_double_clicked(PointerButton::Primary))
    }
}

impl PrimaryClickState {
    pub(crate) fn classify(
        &mut self,
        ui: &egui::Ui,
        response: &egui::Response,
        MapClickContext {
            candidates,
            log_glyph,
            double_click,
            selection,
            camera,
        }: MapClickContext<'_>,
    ) -> ClassifiedMapClick {
        if ui.input(|input| input.key_pressed(Key::Escape)) {
            self.pending = None;
            return ClassifiedMapClick {
                gesture: MapClickGesture::None,
                confirmed_log: None,
            };
        }
        let camera = ClickCamera::of(camera);
        let now = ui.input(|input| input.time);
        let pending = self
            .pending
            .take()
            .filter(|pending| pending.after == selection && pending.camera == camera);
        let position = response
            .interact_pointer_pos()
            .or_else(|| ui.input(|input| input.pointer.interact_pos()));
        let covered_double_click = pending
            .as_ref()
            .is_some_and(|pending| pending.receives_popup_double_click(ui, response));
        if double_click != DoubleClickBehavior::None
            && (response.double_clicked_by(PointerButton::Primary) || covered_double_click)
            && let Some(pending) = &pending
            && let Some(position) = position
        {
            return ClassifiedMapClick {
                gesture: MapClickGesture::Double {
                    position,
                    restore: pending.before,
                },
                confirmed_log: None,
            };
        }

        if response.clicked_by(PointerButton::Primary) {
            let position = position.unwrap_or(response.rect.center());
            let confirmed_log = pending.and_then(|pending| pending.log_glyph);
            if double_click != DoubleClickBehavior::None {
                let delay = ui
                    .ctx()
                    .options(|options| options.input_options.max_double_click_delay);
                self.pending = Some(PendingClick {
                    deadline: now + delay,
                    position,
                    before: selection,
                    after: selection,
                    camera,
                    log_glyph: log_glyph.cloned(),
                });
                if log_glyph.is_some() {
                    ui.ctx().request_repaint_after_secs(delay as f32);
                }
            }
            return ClassifiedMapClick {
                gesture: MapClickGesture::Single(PrimaryMapClick {
                    position,
                    candidates,
                }),
                confirmed_log: if double_click == DoubleClickBehavior::None {
                    log_glyph.cloned().or(confirmed_log)
                } else {
                    confirmed_log
                },
            };
        }

        let confirmed_log = match pending {
            Some(pending) if now >= pending.deadline => pending.log_glyph,
            Some(pending) => {
                if pending.log_glyph.is_some() {
                    ui.ctx()
                        .request_repaint_after_secs((pending.deadline - now) as f32);
                }
                self.pending = Some(pending);
                None
            }
            None => None,
        };
        ClassifiedMapClick {
            gesture: MapClickGesture::None,
            confirmed_log,
        }
    }

    pub(crate) fn record_selection(&mut self, selection: ClickSelection) {
        if let Some(pending) = &mut self.pending {
            pending.after = selection;
        }
    }
}

pub(crate) const DISAMBIGUATION_POPUP_ID: &str = "map_disambig";
pub(crate) const PINNED_POPUP_ID: &str = "sticky_popup";

#[cfg(test)]
mod tests;
