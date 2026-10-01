use egui::{Frame, Label, Layout, RichText, TextStyle, Tooltip};

#[derive(Clone, Copy, Debug)]
pub struct DetailRow<'a> {
    pub caption: &'a str,
    pub value: &'a str,
}

pub struct DetailsLayout<'a> {
    rows: &'a [DetailRow<'a>],
}

impl<'a> DetailsLayout<'a> {
    pub const fn new(rows: &'a [DetailRow<'a>]) -> Self {
        Self { rows }
    }

    /// Values wrap within the parent width, including during an invisible sizing pass.
    pub fn show_ui(self, ui: &mut egui::Ui) -> egui::Response {
        let width = ui.available_width().max(0.0);
        let font = TextStyle::Body.resolve(ui.style());
        let caption_width = self
            .rows
            .iter()
            .fold(0.0_f32, |widest, row| {
                widest.max(
                    ui.painter()
                        .layout_no_wrap(
                            row.caption.to_owned(),
                            font.clone(),
                            ui.visuals().weak_text_color(),
                        )
                        .size()
                        .x,
                )
            })
            .min(width * MAX_CAPTION_FRACTION);
        let gap = COLUMN_SPACING.min((width - caption_width).max(0.0));
        let value_width = (width - caption_width - gap).max(0.0);
        ui.scope(|ui| {
            ui.spacing_mut().item_spacing = egui::vec2(gap, ROW_SPACING);
            for row in self.rows {
                ui.horizontal_top(|ui| {
                    ui.allocate_ui_with_layout(
                        egui::vec2(caption_width, 0.0),
                        Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(caption_width);
                            ui.add(
                                Label::new(RichText::new(row.caption).weak())
                                    .wrap()
                                    .selectable(false),
                            );
                        },
                    );
                    ui.allocate_ui_with_layout(
                        egui::vec2(value_width, 0.0),
                        Layout::top_down(egui::Align::Min),
                        |ui| {
                            ui.set_width(value_width);
                            ui.add(Label::new(row.value).wrap().selectable(true));
                        },
                    );
                });
            }
        })
        .response
    }
}

pub struct DetailsTooltip<'a> {
    response: &'a egui::Response,
}

impl<'a> DetailsTooltip<'a> {
    pub const fn new(response: &'a egui::Response) -> Self {
        Self { response }
    }

    /// Width is established before the complete tooltip body is measured.
    pub fn show<R>(self, body: impl FnOnce(&mut egui::Ui) -> R) -> Option<egui::InnerResponse<R>> {
        let ctx = &self.response.ctx;
        let frame = Frame::popup(&ctx.global_style());
        let viewport_width = (ctx.content_rect().width() - frame.total_margin().sum().x).max(0.0);
        let width = ctx
            .global_style()
            .spacing
            .tooltip_width
            .clamp(MIN_TOOLTIP_WIDTH, MAX_TOOLTIP_WIDTH)
            .min(viewport_width);
        Tooltip::for_enabled(self.response).width(width).show(|ui| {
            ui.set_width(width);
            body(ui)
        })
    }
}

const COLUMN_SPACING: f32 = 12.0;
const MAX_CAPTION_FRACTION: f32 = 0.4;
const MAX_TOOLTIP_WIDTH: f32 = 480.0;
const MIN_TOOLTIP_WIDTH: f32 = 320.0;
const ROW_SPACING: f32 = 6.0;
