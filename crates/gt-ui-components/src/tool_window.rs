use egui::{Align2, Area, AreaState, Window};

#[derive(Clone, Copy, Debug)]
pub struct ToolWindowSizing {
    pub preferred_fraction: egui::Vec2,
    pub minimum_size: egui::Vec2,
    pub maximum_fraction: egui::Vec2,
}

pub struct ToolWindow<'a> {
    pub id: egui::Id,
    pub title: &'a str,
    pub viewport: egui::Rect,
    pub sizing: ToolWindowSizing,
    pub movable: bool,
    pub resizable: bool,
}

impl ToolWindowSizing {
    pub fn constrain_size(self, viewport: egui::Rect, requested_size: egui::Vec2) -> egui::Vec2 {
        let maximum = viewport.size()
            * self
                .maximum_fraction
                .clamp(egui::Vec2::ZERO, egui::Vec2::splat(1.0));
        let minimum = self.minimum_size.max(egui::Vec2::ZERO).min(maximum);
        requested_size.clamp(minimum, maximum)
    }
}

impl ToolWindow<'_> {
    pub fn transfer_pointer_drag(ctx: &egui::Context, window_id: egui::Id) {
        // egui 0.36 uses this private title-bar ID for pointer deltas before area restoration.
        ctx.set_dragged_id(window_id.with(EGUI_TITLE_DRAG_ID));
    }

    /// egui preserves subsequent user geometry. Short content can reduce the initial height.
    /// A pending rectangle applies once when the window is open.
    pub fn show_ui<R>(
        self,
        ctx: &egui::Context,
        open: &mut bool,
        pending_rect: Option<&mut Option<egui::Rect>>,
        body: impl FnOnce(&mut egui::Ui) -> R,
    ) -> Option<egui::InnerResponse<Option<R>>> {
        if !*open {
            return None;
        }
        let Self {
            id,
            title,
            viewport,
            sizing,
            movable,
            resizable,
        } = self;
        let maximum = sizing.constrain_size(viewport, viewport.size());
        let minimum = sizing.constrain_size(viewport, egui::Vec2::ZERO);
        let preferred =
            sizing.constrain_size(viewport, viewport.size() * sizing.preferred_fraction);
        let area_state = AreaState::load(ctx, id);
        let placement_id = id.with(INITIAL_CENTER_ID);
        if area_state.is_none() {
            ctx.data_mut(|data| data.insert_temp(placement_id, true));
        }
        let center_initial_frame =
            ctx.data(|data| data.get_temp::<bool>(placement_id).unwrap_or(false));
        let current_rect = pending_rect.and_then(Option::take).map(|rect| {
            let size = sizing.constrain_size(viewport, rect.size());
            let center = rect
                .center()
                .clamp(viewport.min + size * 0.5, viewport.max - size * 0.5);
            egui::Rect::from_center_size(center, size)
        });
        if let Some(rect) = current_rect {
            self.establish_current_rect(ctx, rect);
            ctx.data_mut(|data| data.remove::<bool>(placement_id));
        }
        let initial_top_left = area_state
            .filter(|state| !center_initial_frame && state.pivot != Align2::LEFT_TOP)
            .map(|state| state.rect().min);
        let center_initial_frame = center_initial_frame && current_rect.is_none();
        let pivot = if center_initial_frame {
            Align2::CENTER_CENTER
        } else {
            Align2::LEFT_TOP
        };
        let default_position = if center_initial_frame {
            viewport.center()
        } else {
            viewport.center() - preferred * 0.5
        };
        let mut window = Window::new(title)
            .id(id)
            .open(open)
            .movable(movable)
            .resizable(resizable)
            .pivot(pivot)
            .default_pos(default_position)
            .default_size(preferred)
            .min_size(minimum)
            .max_size(maximum)
            .constrain_to(viewport);
        if let Some(rect) = current_rect {
            window = window
                .current_pos(rect.min)
                .fixed_size(rect.size())
                .movable(false);
            ctx.request_repaint();
        } else if let Some(position) = initial_top_left {
            window = window.current_pos(position).movable(false);
            ctx.request_repaint();
        }
        window.show(ctx, |ui| {
            if center_initial_frame && ui.is_visible() {
                ctx.data_mut(|data| data.remove::<bool>(placement_id));
                ctx.request_repaint();
            }
            if current_rect.is_some() {
                ui.set_min_size(ui.available_size());
            }
            body(ui)
        })
    }

    fn establish_current_rect(&self, ctx: &egui::Context, rect: egui::Rect) {
        // The invisible area pass sets current geometry before egui restores its title pivot.
        Area::new(self.id)
            .pivot(Align2::LEFT_TOP)
            .current_pos(rect.min)
            .default_size(rect.size())
            .sizing_pass(true)
            .constrain_to(self.viewport)
            .show(ctx, |ui| ui.set_min_size(rect.size()));
    }
}

const INITIAL_CENTER_ID: &str = "initial-center";
const EGUI_TITLE_DRAG_ID: &str = "__title_click";
