use egui::{Label, RichText};

/// Explanation shown when hovering the "Identity" label.
pub const IDENTITY: &str = "Groups related recordings together in the database. \
    Set explicitly via the SDK, or derived from file metadata \
    (shown as 'auto' when inferred automatically).";

/// Renders a domain-specific term as an underlined label with a tooltip
/// that explains it on hover.
///
/// Pass a [`RichText`] so the caller controls weight, size, and colour.
/// The underline is always added here to signal that the term is hoverable.
///
/// Help cursor on hover: the term is neither a control nor selectable text.
pub fn term_label(ui: &mut egui::Ui, text: RichText, explanation: &str) {
    ui.add(Label::new(text.underline()).sense(egui::Sense::hover()))
        .on_hover_cursor(egui::CursorIcon::Help)
        .on_hover_text(explanation);
}
