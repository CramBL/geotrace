use strum::{EnumCount, IntoEnumIterator};

/// One kind of element the map draws from the loaded recordings.
///
/// The unit of the [`DisplayMask`]: every piece of map ink belongs to
/// exactly one category, and hiding a category removes that ink without
/// touching which data is in scope (tree visibility, filters, deletion).
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    strum::Display,
    strum::EnumString,
    strum::EnumCount,
    strum::EnumIter,
    serde::Serialize,
    serde::Deserialize,
)]
#[strum(serialize_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum DisplayCategory {
    /// The tracklines.
    Tracks,
    /// The per-fix icons (arrows, ghost chevrons) and the quality line.
    TrackPoints,
    /// The satellite-count labels.
    SatelliteLabels,
    /// User-authored markers.
    CustomMarkers,
    /// Detection-generated markers (slips, fix regained, …).
    GeneratedMarkers,
    /// Markers imported from event logs.
    EventMarkers,
    /// Query-match halos and rings.
    QueryHighlights,
    /// The snapped-track polylines (the snap-to-road reference geometry).
    SnappedTracks,
    /// The per-point sky glyphs (directions of the satellites in the fix).
    SkyGlyphs,
    /// The aircraft-interference cells drawn beneath the track ink.
    JammingHexes,
    /// The ionospheric TEC grid drawn beneath the track ink.
    TecHeatmap,
}

impl DisplayCategory {
    /// The category's bit in a [`DisplayMask`]. Cannot overflow: the const
    /// assert below pins `COUNT` to the mask's width.
    fn bit(self) -> u16 {
        1 << (self as u16)
    }
}

/// The display category that masks the ink of a side-panel tree category.
/// One-way: [`DisplayCategory::QueryHighlights`] has no tree counterpart.
/// Satellite reports map to the satellite labels, the only ink they draw
/// beyond the track points themselves.
impl From<gt_types::DataCategory> for DisplayCategory {
    fn from(category: gt_types::DataCategory) -> Self {
        match category {
            gt_types::DataCategory::Track => Self::Tracks,
            gt_types::DataCategory::Tpv => Self::TrackPoints,
            gt_types::DataCategory::SatelliteReport => Self::SatelliteLabels,
            gt_types::DataCategory::CustomMarker => Self::CustomMarkers,
            gt_types::DataCategory::GeneratedMarker => Self::GeneratedMarkers,
            gt_types::DataCategory::EventMarker => Self::EventMarkers,
        }
    }
}

/// Every category's bit set - the `hide_all` state.
const ALL_HIDDEN: u16 = u16::MAX >> (u16::BITS as usize - DisplayCategory::COUNT);

/// The categories hidden on a fresh install.
///
/// [`DisplayCategory::JammingHexes`] and [`DisplayCategory::TecHeatmap`] are
/// opt-in: they colour the whole map from data the user did not record.
const DEFAULT_HIDDEN: [DisplayCategory; 2] =
    [DisplayCategory::JammingHexes, DisplayCategory::TecHeatmap];

const _: () = assert!(
    DisplayCategory::COUNT <= u16::BITS as usize,
    "DisplayMask stores one bit per category in a u16"
);

/// Global per-category visibility of the map's ink - the render-side AND
/// on top of the per-track tree visibility.
///
/// An element draws only when its track is visible *and* its category is
/// visible here. The mask never feeds filtering, deletion, stats or exports:
/// showing a hidden category restores everything instantly.
///
/// Serialized as the list of hidden categories, so an empty or absent
/// list means everything is visible and categories added later default
/// to visible on old settings files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(from = "ChangedCategories", into = "ChangedCategories")]
pub struct DisplayMask {
    /// Bit set (per [`DisplayCategory::bit`]) of the hidden categories.
    hidden: u16,
}

impl Default for DisplayMask {
    fn default() -> Self {
        let mut mask = Self { hidden: 0 };
        for category in DEFAULT_HIDDEN {
            mask.set_visible(category, false);
        }
        mask
    }
}

impl DisplayMask {
    pub fn is_visible(self, category: DisplayCategory) -> bool {
        self.hidden & category.bit() == 0
    }

    pub fn set_visible(&mut self, category: DisplayCategory, visible: bool) {
        if visible {
            self.hidden &= !category.bit();
        } else {
            self.hidden |= category.bit();
        }
    }

    pub fn toggle(&mut self, category: DisplayCategory) {
        self.hidden ^= category.bit();
    }

    pub fn show_all(&mut self) {
        self.hidden = 0;
    }

    pub fn hide_all(&mut self) {
        self.hidden = ALL_HIDDEN;
    }

    /// Show only the given category, hiding every other one.
    pub fn solo(&mut self, category: DisplayCategory) {
        self.hide_all();
        self.set_visible(category, true);
    }

    pub fn any_hidden(self) -> bool {
        self.hidden != 0
    }

    pub fn hidden_count(self) -> usize {
        DisplayCategory::iter()
            .filter(|&c| !self.is_visible(c))
            .count()
    }
}

/// The wire form of [`DisplayMask`]: the categories whose visibility
/// differs from the default, in declaration order.
///
/// Every category except [`DEFAULT_HIDDEN`] defaults to visible, so settings
/// files written before a category existed still load correctly.
#[derive(serde::Serialize, serde::Deserialize)]
struct ChangedCategories(Vec<DisplayCategory>);

impl From<ChangedCategories> for DisplayMask {
    fn from(changed: ChangedCategories) -> Self {
        let mut mask = Self::default();
        for category in changed.0 {
            mask.toggle(category);
        }
        mask
    }
}

impl From<DisplayMask> for ChangedCategories {
    fn from(mask: DisplayMask) -> Self {
        let default = DisplayMask::default();
        Self(
            DisplayCategory::iter()
                .filter(|&category| mask.is_visible(category) != default.is_visible(category))
                .collect(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::Deserialize;
    use serde::de::IntoDeserializer;
    use serde::de::value::{Error as DeError, StrDeserializer};
    use std::str::FromStr;

    /// Locks the on-disk spelling of every variant (settings files persist
    /// the hidden-category list), and that `strum`'s string form agrees
    /// with `serde`'s. Asserts the table is exhaustive so a new variant
    /// without a wire-name entry fails here.
    #[test]
    fn wire_names_are_stable() {
        let expected = [
            (DisplayCategory::Tracks, "tracks"),
            (DisplayCategory::TrackPoints, "track_points"),
            (DisplayCategory::SatelliteLabels, "satellite_labels"),
            (DisplayCategory::CustomMarkers, "custom_markers"),
            (DisplayCategory::GeneratedMarkers, "generated_markers"),
            (DisplayCategory::EventMarkers, "event_markers"),
            (DisplayCategory::QueryHighlights, "query_highlights"),
            (DisplayCategory::SnappedTracks, "snapped_tracks"),
            (DisplayCategory::SkyGlyphs, "sky_glyphs"),
            (DisplayCategory::JammingHexes, "jamming_hexes"),
            (DisplayCategory::TecHeatmap, "tec_heatmap"),
        ];
        assert_eq!(expected.len(), DisplayCategory::COUNT);
        for (category, wire) in expected {
            let de: StrDeserializer<'_, DeError> = wire.into_deserializer();
            assert_eq!(
                DisplayCategory::deserialize(de),
                Ok(category),
                "deserializing {wire:?}"
            );
            assert_eq!(category.to_string(), wire);
            assert_eq!(DisplayCategory::from_str(wire), Ok(category));
        }
    }

    #[rstest::rstest]
    #[case(gt_types::DataCategory::Track, DisplayCategory::Tracks)]
    #[case(gt_types::DataCategory::Tpv, DisplayCategory::TrackPoints)]
    #[case(
        gt_types::DataCategory::SatelliteReport,
        DisplayCategory::SatelliteLabels
    )]
    #[case(gt_types::DataCategory::CustomMarker, DisplayCategory::CustomMarkers)]
    #[case(
        gt_types::DataCategory::GeneratedMarker,
        DisplayCategory::GeneratedMarkers
    )]
    #[case(gt_types::DataCategory::EventMarker, DisplayCategory::EventMarkers)]
    fn data_category_maps_to_its_display_category(
        #[case] data: gt_types::DataCategory,
        #[case] expected: DisplayCategory,
    ) {
        assert_eq!(DisplayCategory::from(data), expected);
    }

    /// A settings file written before the interference category existed
    /// lists only categories the user hid, and loads with each of those
    /// hidden and the new one at its own default.
    #[test]
    fn an_older_settings_file_loads_its_hidden_categories() {
        let stored = r#"["custom_markers","query_highlights"]"#;
        let changed: ChangedCategories = serde_json::from_str(stored).expect("changed list");
        let mask = DisplayMask::from(changed);

        assert!(!mask.is_visible(DisplayCategory::CustomMarkers));
        assert!(!mask.is_visible(DisplayCategory::QueryHighlights));
        assert!(
            !mask.is_visible(DisplayCategory::JammingHexes),
            "still default"
        );
        assert!(mask.is_visible(DisplayCategory::Tracks));
    }

    /// Showing a default-hidden category survives a restart.
    #[test]
    fn showing_a_default_hidden_category_round_trips() {
        let mut mask = DisplayMask::default();
        assert!(!mask.is_visible(DisplayCategory::JammingHexes));

        mask.set_visible(DisplayCategory::JammingHexes, true);
        let json = serde_json::to_string(&mask).expect("serialize");
        assert_eq!(json, r#"["jamming_hexes"]"#);
        assert_eq!(
            serde_json::from_str::<DisplayMask>(&json).expect("deserialize"),
            mask
        );
    }

    /// The default writes nothing, so a fresh settings file carries no
    /// display list at all.
    #[test]
    fn the_default_serializes_to_an_empty_list() {
        let json = serde_json::to_string(&DisplayMask::default()).expect("serialize");
        assert_eq!(json, "[]");
    }

    /// Hiding a default-visible category still writes its name, so old and
    /// new files mean the same thing for every category but the new one.
    #[test]
    fn hiding_a_default_visible_category_lists_it() {
        let mut mask = DisplayMask::default();
        mask.set_visible(DisplayCategory::Tracks, false);

        let json = serde_json::to_string(&mask).expect("serialize");
        assert_eq!(
            json, r#"["tracks"]"#,
            "the opt-in category is at its default"
        );
        assert_eq!(
            serde_json::from_str::<DisplayMask>(&json).expect("deserialize"),
            mask
        );
    }

    #[test]
    fn default_shows_everything_but_the_opt_in_categories() {
        let mask = DisplayMask::default();
        assert_eq!(mask.hidden_count(), DEFAULT_HIDDEN.len());
        for category in DisplayCategory::iter() {
            assert_eq!(
                mask.is_visible(category),
                !DEFAULT_HIDDEN.contains(&category)
            );
        }
    }

    #[test]
    fn set_toggle_and_show_all_round_trip() {
        let mut mask = DisplayMask::default();
        let hidden_by_default = mask.hidden_count();
        mask.set_visible(DisplayCategory::GeneratedMarkers, false);
        assert!(!mask.is_visible(DisplayCategory::GeneratedMarkers));
        assert!(mask.is_visible(DisplayCategory::Tracks));
        assert_eq!(mask.hidden_count(), hidden_by_default + 1);

        mask.toggle(DisplayCategory::GeneratedMarkers);
        assert_eq!(mask.hidden_count(), hidden_by_default);

        mask.hide_all();
        assert_eq!(mask.hidden_count(), DisplayCategory::COUNT);
        mask.show_all();
        assert!(!mask.any_hidden());
    }

    #[test]
    fn solo_shows_exactly_one_category() {
        let mut mask = DisplayMask::default();
        mask.solo(DisplayCategory::SatelliteLabels);
        for category in DisplayCategory::iter() {
            assert_eq!(
                mask.is_visible(category),
                category == DisplayCategory::SatelliteLabels
            );
        }
    }

    #[test]
    fn wire_form_lists_exactly_the_hidden_categories() {
        let mut mask = DisplayMask::default();
        mask.set_visible(DisplayCategory::CustomMarkers, false);
        mask.set_visible(DisplayCategory::QueryHighlights, false);

        let wire = ChangedCategories::from(mask);
        assert_eq!(
            wire.0,
            vec![
                DisplayCategory::CustomMarkers,
                DisplayCategory::QueryHighlights
            ]
        );
        assert_eq!(DisplayMask::from(wire), mask);

        // An empty list (the missing-key default) shows everything.
        assert_eq!(
            DisplayMask::from(ChangedCategories(Vec::new())),
            DisplayMask::default()
        );
    }
}
