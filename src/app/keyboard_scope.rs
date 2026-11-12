use egui::{Event, Key, Popup};

#[derive(Clone, Copy, Default)]
pub(super) enum KeyboardAccess {
    #[default]
    Block,
    Permit,
}

impl KeyboardAccess {
    pub(super) fn available(self, ctx: &egui::Context) -> bool {
        matches!(self, Self::Permit)
            && ctx.input(|input| input.focused)
            && !Popup::is_any_open(ctx)
            && ctx.memory(|memory| memory.top_modal_layer().is_none())
    }

    pub(super) fn field_owns_keyboard(self, ui: &egui::Ui, field: egui::Id) -> bool {
        self.available(ui.ctx())
            && ui.memory(|memory| {
                memory.focused() == Some(field)
                    || (memory.focused().is_none() && memory.had_focus_last_frame(field))
            })
    }
}

#[derive(Default)]
pub(super) struct WindowKeyboardScope {
    activated: bool,
    focused_widget: Option<egui::Id>,
}

impl WindowKeyboardScope {
    pub(super) fn remember_focus(&mut self, ctx: &egui::Context, layer: egui::LayerId) {
        // egui can clear focus when `Context::read_response` evaluates a pointer click.
        if ctx.input(|input| input.pointer.any_pressed() || input.pointer.any_click()) {
            if self.activated {
                self.focused_widget = ctx.memory(|memory| memory.focused());
            }
            return;
        }
        if let Some(focused) = ctx.memory(|memory| memory.focused()) {
            let owns_focus = ctx
                .read_response(focused)
                .is_some_and(|response| response.layer_id == layer);
            self.focused_widget = owns_focus.then_some(focused);
            self.activated = owns_focus;
        } else if ctx.input(|input| {
            input.raw.events.iter().any(|event| {
                matches!(
                    event,
                    Event::Key {
                        key: Key::Escape,
                        pressed: true,
                        ..
                    }
                )
            })
        }) {
            if self
                .focused_widget
                .is_some_and(|id| !ctx.memory(|memory| memory.had_focus_last_frame(id)))
            {
                self.activated = false;
                self.focused_widget = None;
            }
        } else {
            self.focused_widget = None;
        }
    }

    pub(super) fn owns_keyboard(&mut self, ctx: &egui::Context, layer: egui::LayerId) -> bool {
        if ctx.input(|input| input.pointer.any_pressed()) {
            self.focused_widget = None;
            self.activated = ctx
                .input(|input| input.pointer.interact_pos())
                .is_some_and(|position| ctx.layer_id_at(position) == Some(layer));
        }
        self.remember_focus(ctx, layer);
        self.activated
            && (ctx.memory(|memory| memory.focused()).is_some()
                || self
                    .focused_widget
                    .is_none_or(|id| ctx.memory(|memory| memory.had_focus_last_frame(id))))
    }
}
