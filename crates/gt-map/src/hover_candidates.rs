use gt_types::DataCategory;
use gt_ui_types::MapElementRef;

use crate::viewport::PlannedElement;

/// The nearest element present on the map per category group under the cursor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct HoverCandidates {
    tpv_or_satellite_report: Option<MapElementRef>,
    event_marker: Option<MapElementRef>,
    custom_marker: Option<MapElementRef>,
    generated_marker: Option<MapElementRef>,
}

impl HoverCandidates {
    /// Keeps `candidate` when its category has no closer one yet. Callers feed
    /// candidates in nearest-first order, after [`crate::viewport::MapFramePlan`] proved
    /// that the element is present.
    pub(crate) fn keep_nearest(&mut self, candidate: PlannedElement<'_>) {
        self.keep_nearest_ref(candidate.element_ref());
    }

    fn keep_nearest_ref(&mut self, candidate: MapElementRef) {
        let Some(slot) = self.slot_for(candidate.category()) else {
            return;
        };
        slot.get_or_insert(candidate);
    }

    fn slot_for(&mut self, category: DataCategory) -> Option<&mut Option<MapElementRef>> {
        match category {
            DataCategory::Tpv | DataCategory::SatelliteReport => {
                Some(&mut self.tpv_or_satellite_report)
            }
            DataCategory::EventMarker => Some(&mut self.event_marker),
            DataCategory::CustomMarker => Some(&mut self.custom_marker),
            DataCategory::GeneratedMarker => Some(&mut self.generated_marker),
            DataCategory::Track => None,
        }
    }

    /// The candidates present, in the order tooltips and popup rows list them.
    pub(crate) fn iter(&self) -> impl Iterator<Item = MapElementRef> {
        [
            self.tpv_or_satellite_report,
            self.event_marker,
            self.custom_marker,
            self.generated_marker,
        ]
        .into_iter()
        .flatten()
    }

    /// The element a hover or a click acts on: the TPV point when it is among
    /// them, otherwise the first candidate present.
    pub(crate) fn primary(&self) -> Option<MapElementRef> {
        self.iter().next()
    }

    /// Whether several element types sit under the cursor at once, so a click
    /// cannot resolve which one the user meant.
    pub(crate) fn is_ambiguous(&self) -> bool {
        self.iter().count() > 1
    }

    pub(crate) fn every_category_filled(&self) -> bool {
        self.tpv_or_satellite_report.is_some()
            && self.event_marker.is_some()
            && self.custom_marker.is_some()
            && self.generated_marker.is_some()
    }

    #[cfg(test)]
    pub(crate) fn from_refs_for_test(candidates: impl IntoIterator<Item = MapElementRef>) -> Self {
        let mut result = Self::default();
        for candidate in candidates {
            result.keep_nearest_ref(candidate);
        }
        result
    }

    #[cfg(test)]
    pub(crate) fn tpv_or_satellite_report(self) -> Option<MapElementRef> {
        self.tpv_or_satellite_report
    }

    #[cfg(test)]
    pub(crate) fn event_marker(self) -> Option<MapElementRef> {
        self.event_marker
    }
}

#[cfg(test)]
mod tests {
    use gt_types::DataCategory;

    use super::HoverCandidates;
    use crate::test_util;

    /// The element a hover or a click acts on is the fix whenever one is among
    /// the candidates.
    #[test]
    fn the_primary_candidate_is_the_fix_when_one_is_present() {
        let tpv = test_util::point_ref(DataCategory::Tpv, 0);
        let marker = test_util::point_ref(DataCategory::EventMarker, 0);

        let marker_only = HoverCandidates::from_refs_for_test([marker]);
        assert_eq!(marker_only.primary(), Some(marker));
        assert!(!marker_only.is_ambiguous());

        let both = HoverCandidates::from_refs_for_test([tpv, marker]);
        assert_eq!(both.primary(), Some(tpv));
        assert!(both.is_ambiguous());
    }
}
