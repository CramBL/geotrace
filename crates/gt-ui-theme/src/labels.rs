//! The label shapes every surface builds its readouts from: a label whose
//! hover says what the label itself has no room for, and the count line the
//! summaries are written in.

use egui::text::LayoutJob;
use egui::{
    CursorIcon, Label, Response, RichText, Sense, TextFormat, TextStyle, TextWrapMode, WidgetText,
};

use crate::MIDDLE_DOT;

/// A label with a hover that carries what the label leaves out: a term's
/// explanation, or the full form of a line that had to truncate.
///
/// The terminal method picks the cursor: Help where the hover explains the
/// label, the arrow where it repeats it.
pub struct LabelWithHover {
    text: WidgetText,
    wrap_mode: Option<TextWrapMode>,
}

impl LabelWithHover {
    /// A domain term under the underline that marks it as explained on hover.
    pub fn underlined_term(text: RichText) -> Self {
        Self::plain(text.underline())
    }

    /// A line that marks itself, or needs no mark.
    pub fn plain(text: impl Into<WidgetText>) -> Self {
        Self {
            text: text.into(),
            wrap_mode: None,
        }
    }

    pub fn wrap_mode(mut self, wrap_mode: TextWrapMode) -> Self {
        self.wrap_mode = Some(wrap_mode);
        self
    }

    /// Lay the text out to one line elided at the width it is given.
    pub fn truncate(self) -> Self {
        self.wrap_mode(TextWrapMode::Truncate)
    }

    /// Shows the label with `explanation` on hover, under the Help cursor.
    pub fn explanation_ui(self, ui: &mut egui::Ui, explanation: &str) -> Response {
        self.show(ui)
            .on_hover_cursor(CursorIcon::Help)
            .on_hover_text(explanation)
    }

    /// Shows the label with an explanation the caller lays out itself, under
    /// the Help cursor.
    pub fn explanation_tooltip_ui(
        self,
        ui: &mut egui::Ui,
        tooltip_contents: impl FnOnce(&mut egui::Ui),
    ) -> Response {
        self.show(ui)
            .on_hover_cursor(CursorIcon::Help)
            .on_hover_ui(tooltip_contents)
    }

    /// Shows the label with `full` on hover, keeping the arrow cursor: the
    /// hover repeats the line.
    pub fn stated_in_full_ui(self, ui: &mut egui::Ui, full: &str) -> Response {
        self.show(ui).on_hover_text(full)
    }

    fn show(self, ui: &mut egui::Ui) -> Response {
        let Self { text, wrap_mode } = self;
        let mut label = Label::new(text).sense(Sense::hover());
        if let Some(wrap_mode) = wrap_mode {
            label = label.wrap_mode(wrap_mode);
        }
        ui.add(label)
    }
}

/// A line stating what something counted: the numbers in the text colour, the
/// words around them dimmed, [`MIDDLE_DOT`] between one count and the next.
///
/// The whole line lays out as one label, so it truncates and hovers as one
/// line.
pub struct CountLine {
    job: LayoutJob,
    number_format: TextFormat,
    words_format: TextFormat,
}

impl CountLine {
    pub fn new(ui: &egui::Ui) -> Self {
        let font = TextStyle::Body.resolve(ui.style());
        Self {
            job: LayoutJob::default(),
            number_format: TextFormat {
                font_id: font.clone(),
                color: ui.visuals().text_color(),
                ..Default::default()
            },
            words_format: TextFormat {
                font_id: font,
                color: ui.visuals().weak_text_color(),
                ..Default::default()
            },
        }
    }

    pub fn number(mut self, number: usize) -> Self {
        self.job
            .append(&number.to_string(), 0.0, self.number_format.clone());
        self
    }

    pub fn words(mut self, words: &str) -> Self {
        self.job.append(words, 0.0, self.words_format.clone());
        self
    }

    /// A number and what it counts.
    pub fn count(self, count: usize, noun: &str) -> Self {
        self.number(count).words(&format!(" {noun}"))
    }

    /// The dot between two counts.
    pub fn dot(self) -> Self {
        self.words(&format!(" {MIDDLE_DOT} "))
    }

    pub fn into_job(self) -> LayoutJob {
        self.job
    }
}
