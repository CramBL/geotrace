//! A short text on a rounded backplate: the satellite counts beside a fix,
//! and how many flags a cluster flag stands for.
//!
//! The log hexagons state their count as plain text, since the hexagon under
//! it separates it from the map on its own.

use egui::{Align2, Color32, FontId, Pos2, Ui};

/// One badge, drawn by [`TextBadge::draw`].
pub(crate) struct TextBadge {
    pub(crate) text: String,
    pub(crate) font: FontId,
    pub(crate) text_color: Color32,
    pub(crate) fill: Color32,
    /// Room between the text and the edge of the backplate.
    pub(crate) padding_pt: f32,
    pub(crate) corner_radius_pt: f32,
    pub(crate) plate_height: BadgePlateHeight,
}

impl TextBadge {
    /// Draws the badge with the text at `align` of `anchor`, the backplate
    /// under it.
    pub(crate) fn draw(self, ui: &Ui, anchor: Pos2, align: Align2) {
        let font_height = self.font.size;
        let painter = ui.painter();
        let galley = painter.layout_no_wrap(self.text, self.font, self.text_color);
        let text = align.anchor_size(anchor, galley.size());
        let plate_height = match self.plate_height {
            BadgePlateHeight::Font => font_height,
            BadgePlateHeight::LaidOutText => galley.size().y,
        };
        let plate =
            egui::Rect::from_center_size(text.center(), egui::vec2(galley.size().x, plate_height))
                .expand(self.padding_pt);
        painter.rect_filled(plate, self.corner_radius_pt, self.fill);
        painter.galley(text.min, galley, self.text_color);
    }
}

/// What a badge's backplate takes its height from.
pub(crate) enum BadgePlateHeight {
    /// The font's height, which ends the backplate at the glyphs.
    Font,
    /// The height of the laid-out text, leading above and below the glyphs
    /// included.
    LaidOutText,
}
