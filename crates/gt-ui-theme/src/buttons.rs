//! The button shapes a control takes when it sits inside a row of text: an
//! icon alone, and the table header that orders the table by its own column.
//! Both are frameless, so they read as part of the row they sit in.

use egui::{Button, CursorIcon, Layout, Response, RichText, TextWrapMode};

/// An icon as a button without a frame, the hover naming what a click does.
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
    active_direction_caret: Option<&'a str>,
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

    /// The caret shown while the table is ordered by this column, drawn beside
    /// the title and left off every other header.
    pub fn active_direction_caret(mut self, caret: &'a str) -> Self {
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
        ui.with_layout(layout, |ui| {
            let title = ui.add(button);
            if let Some(caret) = active_direction_caret {
                ui.label(RichText::new(caret).small().weak());
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
