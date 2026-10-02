use std::collections::BTreeMap;

use egui::emath::GuiRounding as _;
use egui::{ScrollArea, TextStyle, Window};

#[derive(Clone, Copy, Debug)]
pub struct AnchoredWindowSizing {
    pub preferred_width: f32,
    pub maximum_viewport_fraction: f32,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AnchoredWindowPhase {
    HoldHeight,
    MeasureContent,
}

pub struct AnchoredWindow {
    pub layout_id: egui::Id,
    pub window_id: egui::Id,
    pub title: String,
    pub sizing: AnchoredWindowSizing,
}

impl AnchoredWindow {
    pub fn regions(&self) -> FrozenRegions {
        FrozenRegions {
            id: self.layout_id.with(FROZEN_REGIONS),
        }
    }

    /// egui hit-tests pointer presses against previous-pass widget rectangles.
    /// The closure must fill the held height after measurement and freeze regions with changing
    /// content.
    pub fn show_ui<R>(
        self,
        ctx: &egui::Context,
        open: Option<&mut bool>,
        content: impl FnOnce(&mut egui::Ui, AnchoredWindowPhase) -> R,
    ) -> Option<egui::InnerResponse<Option<R>>> {
        if open.as_deref().is_some_and(|open| !open) {
            return None;
        }
        let Self {
            layout_id,
            window_id,
            title,
            sizing,
        } = self;
        let held_id = layout_id.with(HELD_LAYOUT);
        let pass = ctx.cumulative_pass_nr();
        let mut held = ctx
            .data(|data| data.get_temp::<HeldLayout>(held_id))
            .unwrap_or_default();
        if held.last_drawn_pass + 1 < pass {
            held = HeldLayout::default();
            ctx.data_mut(|data| {
                data.remove::<FrozenRegionHeights>(layout_id.with(FROZEN_REGIONS));
            });
        }
        held.last_drawn_pass = pass;

        let viewport = ctx.content_rect();
        let cap = viewport.size() * sizing.maximum_viewport_fraction.clamp(0.0, 1.0);
        let width = sizing.preferred_width.max(0.0).min(cap.x);
        let mut window = Window::new(title)
            .id(window_id)
            .collapsible(false)
            .resizable(true)
            .constrain_to(viewport)
            .min_width(width)
            // Zero initial height measures content before the held layout fills the window.
            .default_size(egui::vec2(width, 0.0));
        let phase = match held.size {
            Some(size) => {
                let height = if size.capped { cap.y } else { size.height };
                window = window
                    .fixed_pos(size.position)
                    .max_size(egui::vec2(cap.x, height));
                AnchoredWindowPhase::HoldHeight
            }
            #[expect(
                clippy::disallowed_methods,
                reason = "Anchored windows center content during opening measurement"
            )]
            None => {
                window = window
                    .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
                    .max_size(cap);
                AnchoredWindowPhase::MeasureContent
            }
        };
        if let Some(open) = open {
            window = window.open(open);
        }
        let laid_out = window.show(ctx, |ui| content(ui, phase));
        match held.size {
            None => {
                if let Some(rect) = laid_out.as_ref().map(|window| window.response.rect) {
                    let measured = rect.size().min(cap);
                    held.size = Some(HeldSize {
                        height: measured.y,
                        position: egui::Rect::from_center_size(viewport.center(), measured)
                            .left_top()
                            .round_ui(),
                        capped: false,
                    });
                }
            }
            Some(size) => {
                held.size = Some(HeldSize {
                    capped: true,
                    ..size
                });
            }
        }
        ctx.data_mut(|data| data.insert_temp(held_id, held));
        laid_out
    }
}

#[derive(Clone, Copy)]
pub struct HeldBodyLines {
    at_least: u8,
    at_most: Option<u8>,
}

impl HeldBodyLines {
    pub fn measured_content() -> Self {
        Self {
            at_least: 0,
            at_most: None,
        }
    }

    pub fn at_least(lines: u8) -> Self {
        Self {
            at_least: lines,
            at_most: None,
        }
    }

    pub fn and_at_most(self, lines: u8) -> Self {
        debug_assert!(
            lines >= self.at_least,
            "a region cannot hold at most {lines} lines: it already holds at least {}",
            self.at_least
        );
        Self {
            at_most: Some(lines),
            ..self
        }
    }
}

#[derive(Clone, Copy)]
pub struct FrozenRegions {
    id: egui::Id,
}

impl FrozenRegions {
    pub fn freeze_at_open_ui<R>(
        self,
        ui: &mut egui::Ui,
        salt: &'static str,
        lines: HeldBodyLines,
        content: impl FnOnce(&mut egui::Ui) -> R,
    ) -> R {
        let frozen = ui.data(|data| {
            data.get_temp::<FrozenRegionHeights>(self.id)
                .and_then(|heights| heights.0.get(salt).copied())
        });
        if let Some(height) = frozen {
            return ScrollArea::vertical()
                .id_salt(salt)
                .auto_shrink(false)
                // egui defaults to 64 points, which exceeds short frozen regions.
                .min_scrolled_height(0.0)
                .max_height(height)
                .show(ui, content)
                .inner;
        }
        let line_height = ui.text_style_height(&TextStyle::Body);
        let laid_out = ui.scope(|ui| match lines.at_most {
            Some(most) => {
                let ceiling = f32::from(most) * line_height;
                ScrollArea::vertical()
                    .id_salt(salt)
                    .auto_shrink([false, true])
                    // The opening pass has no available height until content is measured.
                    .min_scrolled_height(ceiling)
                    .max_height(ceiling)
                    .show(ui, content)
                    .inner
            }
            None => content(ui),
        });
        let drawn = laid_out.response.rect.height();
        let held = drawn.max(f32::from(lines.at_least) * line_height);
        ui.add_space(held - drawn);
        // The sizing pass uses minimum widget heights before the first visible layout.
        if !ui.is_sizing_pass() {
            ui.data_mut(|data| {
                data.get_temp_mut_or_default::<FrozenRegionHeights>(self.id)
                    .0
                    .insert(salt, held);
            });
        }
        laid_out.inner
    }
}

#[derive(Clone, Default)]
struct HeldLayout {
    last_drawn_pass: u64,
    size: Option<HeldSize>,
}

#[derive(Clone, Copy)]
struct HeldSize {
    height: f32,
    position: egui::Pos2,
    // egui retains content growth. The second pass caps it before user resizing is enabled.
    capped: bool,
}

#[derive(Clone, Default)]
struct FrozenRegionHeights(BTreeMap<&'static str, f32>);

const FROZEN_REGIONS: &str = "frozen_regions";
const HELD_LAYOUT: &str = "held_layout";
