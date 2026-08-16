use gt_ui_types::{DrawLayerMask, PointVisibility};

/// One point of one track, as the map currently reads it.
///
/// [`PointClass::visibility`] comes from
/// [`gt_ui_types::MapScope::point_visibility`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PointClass {
    pub visibility: PointVisibility,
    /// The `draw` layers whose halo covers the point, from
    /// [`gt_ui_types::QueryMatches::draw_mask`].
    pub draw_layers: DrawLayerMask,
    /// Covered by the match hovered in the results table.
    pub hover_matched: bool,
    /// The point whose popup is pinned.
    pub selected: bool,
}

impl PointClass {
    pub fn is_shown(&self) -> bool {
        self.visibility.is_shown()
    }

    /// The single character this point contributes to a track's picture: its
    /// halo layer when it carries exactly one, else why it is not shown.
    ///
    /// A shown point with no halo is `.`; with one halo it is that layer's index
    /// in base 16 (the mask holds [`DrawLayerMask::MAX_LAYERS`] = 16 layers, so
    /// one digit always suffices); with several halos it is `*`. The withheld
    /// glyphs stay clear of the base-16 digits, which a test pins down.
    pub fn glyph(&self) -> char {
        match self.visibility {
            PointVisibility::Shown => self.halo_glyph(),
            PointVisibility::NoSuchElement => '?',
            PointVisibility::TrackNotShown => 'o',
            PointVisibility::CategoryHidden => 'm',
            PointVisibility::HiddenByQuery => 'x',
            PointVisibility::OutsideTimeFilter => '-',
        }
    }

    fn halo_glyph(&self) -> char {
        match self.draw_layers.count() {
            0 => '.',
            1 => (0..DrawLayerMask::MAX_LAYERS)
                .find(|&i| self.draw_layers.contains(i))
                .and_then(|i| char::from_digit(i as u32, 16))
                .unwrap_or('?'),
            _ => '*',
        }
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;
    use strum::EnumCount as _;

    use super::*;

    fn class(visibility: PointVisibility, layers: &[usize]) -> PointClass {
        let mut draw_layers = DrawLayerMask::default();
        for &layer in layers {
            draw_layers.insert(layer);
        }
        PointClass {
            visibility,
            draw_layers,
            hover_matched: false,
            selected: false,
        }
    }

    #[rstest]
    #[case(PointVisibility::Shown, &[], '.')]
    #[case(PointVisibility::Shown, &[0], '0')]
    #[case(PointVisibility::Shown, &[2], '2')]
    // The last representable layer still fits one base-16 digit.
    #[case(PointVisibility::Shown, &[DrawLayerMask::MAX_LAYERS - 1], 'f')]
    #[case(PointVisibility::Shown, &[0, 1], '*')]
    #[case(PointVisibility::NoSuchElement, &[], '?')]
    #[case(PointVisibility::TrackNotShown, &[], 'o')]
    #[case(PointVisibility::CategoryHidden, &[], 'm')]
    #[case(PointVisibility::HiddenByQuery, &[], 'x')]
    #[case(PointVisibility::OutsideTimeFilter, &[], '-')]
    fn each_state_has_its_own_glyph(
        #[case] visibility: PointVisibility,
        #[case] layers: &[usize],
        #[case] expected: char,
    ) {
        assert_eq!(class(visibility, layers).glyph(), expected);
    }

    /// Every way the map can withhold a point has its own glyph, so a picture
    /// can never conflate two of them. A new one fails here.
    #[test]
    fn the_glyphs_cover_every_visibility_and_stay_distinct() {
        let all = [
            PointVisibility::Shown,
            PointVisibility::NoSuchElement,
            PointVisibility::TrackNotShown,
            PointVisibility::CategoryHidden,
            PointVisibility::HiddenByQuery,
            PointVisibility::OutsideTimeFilter,
        ];
        assert_eq!(
            all.len(),
            PointVisibility::COUNT,
            "a new visibility needs a glyph"
        );
        let mut glyphs: Vec<char> = all.iter().map(|&v| class(v, &[]).glyph()).collect();
        let distinct = glyphs.len();
        glyphs.sort_unstable();
        glyphs.dedup();
        assert_eq!(glyphs.len(), distinct, "glyphs must not collide");
    }

    /// A halo digit must never collide with a not-shown glyph, or a picture
    /// would read a layer as a withheld point.
    #[test]
    fn halo_digits_never_collide_with_the_not_shown_glyphs() {
        let reserved = ['?', 'o', 'm', 'x', '-', '.', '*'];
        for layer in 0..DrawLayerMask::MAX_LAYERS {
            let glyph = class(PointVisibility::Shown, &[layer]).glyph();
            assert!(
                !reserved.contains(&glyph),
                "layer {layer} renders as the reserved {glyph:?}"
            );
        }
    }
}
