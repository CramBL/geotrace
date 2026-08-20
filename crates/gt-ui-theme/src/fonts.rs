//! The font stack GeoTrace installs, shared by the app and the snapshot
//! harness so both render from the same faces.

use egui::{FontDefinitions, FontFamily};

/// egui's default faces, the Phosphor icons the UI draws its glyphs from, and
/// egui's default monospace face last in the proportional family.
///
/// Ubuntu-Light, which egui puts first in that family, has no glyph for
/// U+207B SUPERSCRIPT MINUS, written in the exponents of the reference
/// material. epaint draws a glyph no face in the family holds as U+25FB WHITE
/// MEDIUM SQUARE.
pub fn font_definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();
    let monospace_face = fonts
        .families
        .get(&FontFamily::Monospace)
        .and_then(|faces| faces.first())
        .cloned();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    fonts
        .families
        .entry(FontFamily::Proportional)
        .or_default()
        .extend(monospace_face);
    fonts
}

#[cfg(test)]
mod tests {
    use egui::{Context, FontId, RawInput};

    use super::*;

    /// The superscripts and the multiplication sign the reference material
    /// writes in prose.
    const SUPERSCRIPT_CHARACTERS: &str = "⁰¹²³⁴⁵⁶⁷⁸⁹⁻×";

    #[test]
    fn the_proportional_family_covers_every_superscript_the_material_writes() {
        let ctx = Context::default();
        ctx.set_fonts(font_definitions());
        // The frame builds the font atlas the query below reads. Its texture
        // delta has no painter to apply it, and epaint panics on a delta
        // dropped unapplied.
        let mut output = ctx.run_ui(RawInput::default(), |_| {});
        output.textures_delta.clear();

        for character in SUPERSCRIPT_CHARACTERS.chars() {
            assert!(
                ctx.fonts_mut(|fonts| fonts.has_glyph(&FontId::proportional(14.0), character)),
                "no face of the proportional family holds {character:?}"
            );
        }
    }
}
