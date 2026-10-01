use egui::emath::GuiRounding as _;
use egui::{CursorIcon, Frame, Sense, UiBuilder};

#[derive(Clone, Copy)]
pub struct FractionalSection {
    pub id: egui::Id,
    pub sizing: FractionalSectionSizing,
    pub frame: Frame,
}

#[derive(Clone, Copy)]
pub struct FractionalSectionSizing {
    pub preferred_fraction: f32,
    pub minimum_height: f32,
    pub maximum_fraction: f32,
}

pub struct FractionalSectionResponse<R> {
    pub inner: R,
    pub response: egui::Response,
    pub divider_response: egui::Response,
    pub region_rect: egui::Rect,
    pub changed_fraction: Option<f32>,
}

impl FractionalSection {
    /// Requires a bounded parent region with a top-down layout. Only divider
    /// movement changes the preferred fraction.
    pub fn show_ui<R>(
        self,
        ui: &mut egui::Ui,
        body: impl FnOnce(&mut egui::Ui) -> R,
    ) -> FractionalSectionResponse<R> {
        debug_assert!((0.0..=1.0).contains(&self.sizing.preferred_fraction));
        debug_assert!((0.0..=1.0).contains(&self.sizing.maximum_fraction));
        debug_assert!(self.sizing.minimum_height.is_finite() && self.sizing.minimum_height >= 0.0);
        let region_rect = ui.available_rect_before_wrap();
        debug_assert!(region_rect.is_finite());
        debug_assert_eq!(ui.layout().main_dir, egui::Direction::TopDown);
        let region_height = region_rect.height().max(0.0);
        let minimum_height = self.sizing.minimum_height.min(region_height);
        let maximum_height = (region_height * self.sizing.maximum_fraction).max(minimum_height);
        let mut height =
            (region_height * self.sizing.preferred_fraction).clamp(minimum_height, maximum_height);
        let changed_fraction =
            self.divider_drag_height(ui, region_rect, height)
                .map(|requested_height| {
                    height = requested_height.clamp(minimum_height, maximum_height);
                    height / region_height
                });
        let section_rect =
            egui::Rect::from_min_size(region_rect.min, egui::vec2(region_rect.width(), height))
                .round_ui();
        let content_rect = section_rect - self.frame.total_margin();
        ui.painter().add(self.frame.paint(content_rect));
        let mut child = ui.new_child(
            UiBuilder::new()
                .id_salt(self.id)
                .max_rect(content_rect)
                .layout(egui::Layout::top_down(egui::Align::Min)),
        );
        child.set_clip_rect(ui.clip_rect().intersect(content_rect));
        let inner = body(&mut child);
        let item_spacing = ui.spacing().item_spacing;
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.advance_cursor_after_rect(section_rect);
        ui.spacing_mut().item_spacing = item_spacing;
        let response = ui.interact(section_rect, self.id, Sense::hover());
        let divider_rect =
            egui::Rect::from_min_max(section_rect.left_bottom(), section_rect.right_bottom())
                .expand2(egui::vec2(
                    0.0,
                    ui.style().interaction.resize_grab_radius_side,
                ));
        let divider_response = ui
            .interact(divider_rect, self.id.with(DIVIDER_ID_SALT), Sense::drag())
            .on_hover_cursor(CursorIcon::ResizeVertical);
        let stroke = if divider_response.dragged() {
            ui.visuals().widgets.active.bg_stroke
        } else if divider_response.hovered() {
            ui.visuals().widgets.hovered.bg_stroke
        } else {
            ui.visuals().widgets.noninteractive.bg_stroke
        };
        ui.painter()
            .hline(section_rect.x_range(), section_rect.bottom(), stroke);
        FractionalSectionResponse {
            inner,
            response,
            divider_response,
            region_rect,
            changed_fraction,
        }
    }

    fn divider_drag_height(
        &self,
        ui: &egui::Ui,
        region_rect: egui::Rect,
        rendered_height: f32,
    ) -> Option<f32> {
        let divider_id = self.id.with(DIVIDER_ID_SALT);
        let grab_offset_id = divider_id.with(GRAB_OFFSET_ID_SALT);
        let interaction = ui.ctx().read_response(divider_id)?;
        if interaction.drag_started()
            && let Some(origin) = ui.input(|input| input.pointer.press_origin())
        {
            ui.ctx().data_mut(|data| {
                data.insert_temp(
                    grab_offset_id,
                    origin.y - region_rect.top() - rendered_height,
                );
            });
        }
        let mut dragged_height = None;
        if (interaction.dragged() || interaction.drag_stopped())
            && ui.input(|input| input.pointer.delta().y != 0.0)
            && region_rect.height() > 0.0
            && let Some(pointer) = interaction.interact_pointer_pos()
            && let Some(grab_offset) = ui.ctx().data(|data| data.get_temp::<f32>(grab_offset_id))
        {
            let requested_height = pointer.y - region_rect.top() - grab_offset;
            if !interaction.drag_started()
                || (requested_height - rendered_height).abs() > f32::EPSILON
            {
                dragged_height = Some(requested_height);
            }
        }
        if interaction.drag_stopped() {
            ui.ctx().data_mut(|data| data.remove::<f32>(grab_offset_id));
        }
        dragged_height
    }
}

const DIVIDER_ID_SALT: &str = "divider";
const GRAB_OFFSET_ID_SALT: &str = "grab_offset";
