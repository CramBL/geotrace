//! The button shapes a control takes when it sits inside a row of text: an
//! icon alone, and the table header that orders the table by its own column.
//! Both are frameless, so they read as part of the row they sit in.

use egui::{Button, CursorIcon, Layout, Response, RichText, TextStyle, TextWrapMode};
use egui_phosphor::regular::CARET_DOWN as ICON_CARET_DOWN;
use egui_phosphor::regular::CARET_UP as ICON_CARET_UP;

use crate::labels;

/// An icon as a button without a frame, the hover stating what a click does.
///
/// Disabled, it grays out and its hover states the reason (DESIGN.md,
/// "Controls and conditional state").
pub struct FramelessIconButton {
    icon: RichText,
    enabled: bool,
}

impl FramelessIconButton {
    pub fn new(icon: impl Into<RichText>) -> Self {
        Self {
            icon: icon.into(),
            enabled: true,
        }
    }

    pub fn enabled(mut self, enabled: bool) -> Self {
        self.enabled = enabled;
        self
    }

    /// Shows the button with `hover`, which a disabled button states as the
    /// reason a click is unavailable.
    pub fn hover_text_ui(self, ui: &mut egui::Ui, hover: &str) -> Response {
        let enabled = self.enabled;
        let response = self.show(ui);
        if enabled {
            response
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_text(hover)
        } else {
            response.on_disabled_hover_text(hover)
        }
    }

    /// Shows the button with a hover the caller lays out itself.
    pub fn hover_tooltip_ui(
        self,
        ui: &mut egui::Ui,
        tooltip_contents: impl FnOnce(&mut egui::Ui),
    ) -> Response {
        let enabled = self.enabled;
        let response = self.show(ui);
        if enabled {
            response
                .on_hover_cursor(CursorIcon::PointingHand)
                .on_hover_ui(tooltip_contents)
        } else {
            response.on_disabled_hover_ui(tooltip_contents)
        }
    }

    /// The width the button lays out to, for a caller reserving room for a
    /// button that is drawn in some of a column's rows only.
    pub fn width(self, ui: &egui::Ui) -> f32 {
        labels::text_width(ui, self.icon, TextStyle::Button)
    }

    fn show(self, ui: &mut egui::Ui) -> Response {
        let Self { icon, enabled } = self;
        ui.add_enabled(enabled, Button::new(icon).frame(false))
    }
}

/// A table header that orders the table by its own column, its hover closing
/// with the order a click produces.
///
/// The pointing hand is set here: egui buttons set no cursor of their own.
pub struct SortHeaderButton<'a> {
    title: RichText,
    active_direction_caret: Option<SortCaret>,
    term_explanation: Option<&'a str>,
    wrap_mode: Option<TextWrapMode>,
}

impl<'a> SortHeaderButton<'a> {
    pub fn new(title: &str) -> Self {
        Self {
            title: RichText::new(title).strong(),
            active_direction_caret: None,
            term_explanation: None,
            wrap_mode: None,
        }
    }

    /// The caret shown while the table is ordered by this column. A click that
    /// moves the sort moves no column edge: every other header keeps the room
    /// for the caret empty.
    pub fn active_direction_caret(mut self, caret: SortCaret) -> Self {
        self.active_direction_caret = Some(caret);
        self
    }

    /// The column's glossary explanation, which underlines the title and leads
    /// the hover, the way [`crate::labels::LabelWithHover::underlined_term`]
    /// marks a term.
    pub fn term_explanation(mut self, explanation: &'a str) -> Self {
        self.title = self.title.underline();
        self.term_explanation = Some(explanation);
        self
    }

    pub fn wrap_mode(mut self, wrap_mode: TextWrapMode) -> Self {
        self.wrap_mode = Some(wrap_mode);
        self
    }

    /// Shows the header in `layout`, e.g. right to left for a column of
    /// numbers, which puts the caret left of the title.
    pub fn show(self, ui: &mut egui::Ui, layout: Layout, order_a_click_produces: &str) -> Response {
        let Self {
            title,
            active_direction_caret,
            term_explanation,
            wrap_mode,
        } = self;
        let mut button = Button::new(title).frame(false);
        if let Some(wrap_mode) = wrap_mode {
            button = button.wrap_mode(wrap_mode);
        }
        let widest_caret = widest_caret_width(ui);
        ui.with_layout(layout, |ui| {
            let title = ui.add(button);
            // The gap before the caret is the layout's own item spacing, which
            // the cursor takes after the title. The pad for the narrower caret
            // goes before it: a pad after the last widget of the row would take
            // that widget's own trailing spacing into the header's width too.
            match active_direction_caret {
                Some(caret) => {
                    ui.add_space(widest_caret - caret.width(ui));
                    ui.label(RichText::new(caret.glyph()).small().weak());
                }
                None => ui.add_space(widest_caret),
            }
            title
        })
        .inner
        .on_hover_cursor(CursorIcon::PointingHand)
        .on_hover_ui(|ui| {
            if let Some(explanation) = term_explanation {
                ui.label(explanation);
            }
            ui.label(
                RichText::new(format!("Click to sort {order_a_click_produces}"))
                    .small()
                    .color(ui.visuals().weak_text_color()),
            );
        })
    }
}

/// Which way the values of the column a table is ordered by run, as the caret
/// its header draws for them.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[expect(clippy::exhaustive_enums, reason = "the set is complete")]
pub enum SortCaret {
    Ascending,
    Descending,
}

impl SortCaret {
    pub fn glyph(self) -> &'static str {
        match self {
            Self::Ascending => ICON_CARET_UP,
            Self::Descending => ICON_CARET_DOWN,
        }
    }

    fn width(self, ui: &egui::Ui) -> f32 {
        labels::text_width(ui, self.glyph(), TextStyle::Small)
    }
}

/// The width a sort header lays out to: its title and the caret slot beside it.
pub fn sort_header_width(ui: &egui::Ui, title: &str) -> f32 {
    labels::text_width(ui, title, TextStyle::Button) + sort_caret_slot_width(ui)
}

/// The width a sort header keeps beside its title for the caret, whether it
/// draws one or not: the gap between the two, and the wider of the two carets.
fn sort_caret_slot_width(ui: &egui::Ui) -> f32 {
    ui.spacing().item_spacing.x + widest_caret_width(ui)
}

fn widest_caret_width(ui: &egui::Ui) -> f32 {
    SortCaret::Ascending
        .width(ui)
        .max(SortCaret::Descending.width(ui))
}

/// The width `Button::new(label)` lays out to, small or not: a small button
/// drops the padding above and below its label and keeps the padding beside
/// it.
pub fn button_width(ui: &egui::Ui, label: &str) -> f32 {
    labels::text_width(ui, label, TextStyle::Button) + 2.0 * ui.spacing().button_padding.x
}

#[cfg(test)]
mod tests {
    use egui::{Align, Context, RawInput};

    use super::*;
    use crate::fonts;

    /// The width one sort header lays out to in `ui`, drawing `caret` when it
    /// is the column the table is ordered by.
    fn header_width(ui: &mut egui::Ui, caret: Option<SortCaret>) -> f32 {
        let mut header = SortHeaderButton::new(HEADER_TITLE);
        if let Some(caret) = caret {
            header = header.active_direction_caret(caret);
        }
        ui.scope(|ui| {
            header.show(ui, Layout::left_to_right(Align::Center), "newest first");
        })
        .response
        .rect
        .width()
    }

    /// A click that moves the sort moves no column edge: the header a caret
    /// leaves and the header it arrives at keep the width they had, which is
    /// the width [`sort_header_width`] states for a column to reserve.
    #[test]
    fn a_sort_header_takes_the_same_width_whichever_caret_it_draws() {
        let ctx = Context::default();
        ctx.set_fonts(fonts::font_definitions());
        // The first frame lays the glyphs out and builds the atlas. epaint
        // reports a glyph it lays out for the first time from the font's own
        // advance, and the pixel-snapped width from the next frame on.
        let mut warm_up = ctx.run_ui(RawInput::default(), |ui| {
            header_width(ui, Some(SortCaret::Ascending));
        });
        warm_up.textures_delta.clear();

        let mut measured = ctx.run_ui(RawInput::default(), |ui| {
            let reserved = sort_header_width(ui, HEADER_TITLE);
            for caret in [
                None,
                Some(SortCaret::Ascending),
                Some(SortCaret::Descending),
            ] {
                let width = header_width(ui, caret);
                assert!(
                    (width - reserved).abs() < WIDTH_TOLERANCE_PX,
                    "the header drawing {caret:?} is {width}px wide, where a column reserves \
                     {reserved}px for it",
                );
            }
        });
        measured.textures_delta.clear();
    }

    /// A title of a column whose header is wider than the values under it,
    /// which is where a caret that takes width of its own moves a column edge.
    const HEADER_TITLE: &str = "Duration";

    /// How far a header may stand from the width a column reserves for it.
    const WIDTH_TOLERANCE_PX: f32 = 0.01;
}
