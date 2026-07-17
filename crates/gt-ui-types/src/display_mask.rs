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

const _: () = assert!(
    DisplayCategory::COUNT <= u16::BITS as usize,
    "DisplayMask stores one bit per category in a u16"
);

/// Global per-category visibility of the map's ink - the render-side AND
/// on top of the per-track tree visibility.
///
/// An element draws only when its track is visible *and* its category is
/// visible here. The mask never feeds filtering, deletion, stats, or
/// exports; showing a hidden category restores everything instantly.
///
/// Serialized as the list of hidden categories, so an empty or absent
/// list means everything is visible and categories added later default
/// to visible on old settings files.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(from = "HiddenCategories", into = "HiddenCategories")]
pub struct DisplayMask {
    /// Bit set (per [`DisplayCategory::bit`]) of the hidden categories.
    /// Hidden rather than visible bits so that `Default` (all zero) means
    /// all visible.
    hidden: u16,
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

/// The wire form of [`DisplayMask`]: the hidden categories, in
/// declaration order.
#[derive(serde::Serialize, serde::Deserialize)]
struct HiddenCategories(Vec<DisplayCategory>);

impl From<HiddenCategories> for DisplayMask {
    fn from(hidden: HiddenCategories) -> Self {
        let mut mask = Self::default();
        for category in hidden.0 {
            mask.set_visible(category, false);
        }
        mask
    }
}

impl From<DisplayMask> for HiddenCategories {
    fn from(mask: DisplayMask) -> Self {
        Self(
            DisplayCategory::iter()
                .filter(|&c| !mask.is_visible(c))
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

    #[test]
    fn default_shows_everything() {
        let mask = DisplayMask::default();
        assert!(!mask.any_hidden());
        assert_eq!(mask.hidden_count(), 0);
        for category in DisplayCategory::iter() {
            assert!(mask.is_visible(category));
        }
    }

    #[test]
    fn set_toggle_and_show_all_round_trip() {
        let mut mask = DisplayMask::default();
        mask.set_visible(DisplayCategory::GeneratedMarkers, false);
        assert!(!mask.is_visible(DisplayCategory::GeneratedMarkers));
        assert!(mask.is_visible(DisplayCategory::Tracks));
        assert_eq!(mask.hidden_count(), 1);

        mask.toggle(DisplayCategory::GeneratedMarkers);
        assert!(!mask.any_hidden());

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

        let wire = HiddenCategories::from(mask);
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
            DisplayMask::from(HiddenCategories(Vec::new())),
            DisplayMask::default()
        );
    }
}
