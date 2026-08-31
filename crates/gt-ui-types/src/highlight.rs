use chrono::{DateTime, Utc};
use gt_types::{DataCategory, FileIdx, PointIdx, TrackIdx, TrackRef};

use crate::visibility::{MapScope, PointVisibility};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DataPointRef {
    pub track: TrackRef,
    pub category: DataCategory,
    pub point_index: PointIdx,
}

/// A query-result match hovered in the results table: one track's matched
/// point range, echoed on the map as a halo band and on the plot as a shaded
/// time band. Stores the range as two indices (not a [`std::ops::Range`]) so
/// it stays `Copy` like the rest of [`MapHighlight`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MatchHighlight {
    pub track: TrackRef,
    /// First matched point index.
    pub start: usize,
    /// One past the last matched point index.
    pub end: usize,
}

impl MatchHighlight {
    pub fn new(track: TrackRef, range: &std::ops::Range<usize>) -> Self {
        Self {
            track,
            start: range.start,
            end: range.end,
        }
    }

    /// Whether the match covers `point_index` of its track.
    pub fn contains(&self, point_index: usize) -> bool {
        (self.start..self.end).contains(&point_index)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightScope {
    File {
        file_index: FileIdx,
    },
    Track(TrackRef),
    TrackCategory {
        track: TrackRef,
        category: DataCategory,
    },
    Point(DataPointRef),
}

/// The nearest visible element per category group under the cursor.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct HoverCandidates {
    pub tpv_or_satellite_report: Option<DataPointRef>,
    pub event_marker: Option<DataPointRef>,
    pub custom_marker: Option<DataPointRef>,
    pub generated_marker: Option<DataPointRef>,
}

impl HoverCandidates {
    /// Keeps `candidate` when its category has no closer one yet: callers feed
    /// candidates in nearest-first order.
    pub fn keep_nearest(&mut self, candidate: DataPointRef) {
        let Some(slot) = self.slot_for(candidate.category) else {
            return;
        };
        slot.get_or_insert(candidate);
    }

    fn slot_for(&mut self, category: DataCategory) -> Option<&mut Option<DataPointRef>> {
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
    pub fn iter(&self) -> impl Iterator<Item = DataPointRef> {
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
    pub fn primary(&self) -> Option<DataPointRef> {
        self.iter().next()
    }

    /// Whether several element types sit under the cursor at once, so a click
    /// cannot resolve which one the user meant.
    pub fn is_ambiguous(&self) -> bool {
        self.iter().count() > 1
    }

    /// Whether a marker of any kind is under the cursor. A marker takes the
    /// hover from the log hexagons: its pin draws over them.
    pub fn any_marker(&self) -> bool {
        self.event_marker.is_some()
            || self.custom_marker.is_some()
            || self.generated_marker.is_some()
    }

    pub fn every_category_filled(&self) -> bool {
        self.tpv_or_satellite_report.is_some()
            && self.event_marker.is_some()
            && self.custom_marker.is_some()
            && self.generated_marker.is_some()
    }
}

#[derive(Debug, Clone, Copy)]
pub struct MapHighlight {
    pub hover: Option<HighlightScope>,
    pub sticky: Option<DataPointRef>,
    /// Every element within the cursor radius, one per category group, so
    /// renderers can show tooltips for secondary candidates even when a TPV
    /// point is the primary hover.
    pub hover_candidates: HoverCandidates,
    /// Time currently hovered on the track plot. Used to cross-highlight the
    /// closest TPV point on the map. `None` when the plot cursor is inactive.
    pub plot_hover_time: Option<DateTime<Utc>>,
    /// Pre-computed `(FileIdx, TrackIdx, PointIdx)` of the TPV point closest to
    /// `plot_hover_time`, set by the app layer alongside that field.
    /// `TpvRenderer` reads this directly instead of re-scanning all points.
    /// `None` when `plot_hover_time` is `None`.
    pub plot_hover_point: Option<(FileIdx, TrackIdx, PointIdx)>,
    /// `true` when the plot cursor is within the snap-distance threshold of
    /// `plot_hover_point` (approximately 25 px in time on-screen).
    ///
    /// Only when this is `true` does the map overlay activate for plot hover.
    /// Prevents the map from dimming the moment the cursor crosses the plot
    /// boundary, before it is actually near any data.
    pub plot_hover_snapped: bool,
    /// The sky-trails scrubber's current instant, cross-highlighted on the
    /// track plot as a vertical time line. `None` when the sky-trails window is
    /// not driving a scrub. Set by that window and read one frame behind (it
    /// draws after the plot), the same way [`Self::hover_match`] is.
    pub scrub_time: Option<DateTime<Utc>>,
    /// When `true`, renderers must not draw their individual hover labels.
    ///
    /// Set by `NavMap` in two situations: when the disambiguation popup is open
    /// (the popup occupies that screen region) and when multiple hover candidates
    /// are active simultaneously (the map layer draws a single compact stacked
    /// label instead of having each renderer place one near the cursor).
    pub suppress_hover_labels: bool,
    /// When `false`, the track/map fading animation and background dimming are
    /// disabled.
    pub fading_enabled: bool,
    /// The match hovered in the query results table, cross-highlighted on the
    /// map and plot. Cleared by the app each frame before the query window
    /// renders; the map and plot read it one frame behind (the query window
    /// draws after both).
    pub hover_match: Option<MatchHighlight>,
}

impl MapHighlight {
    /// Pin `point_ref`'s popup, or unpin it when it is already the sticky point.
    /// Returns whether it ended up pinned: the caller places the popup only for
    /// one that opened.
    pub fn toggle_sticky(&mut self, point_ref: DataPointRef) -> bool {
        let pinned = self.sticky != Some(point_ref);
        self.sticky = pinned.then_some(point_ref);
        pinned
    }

    /// [`Self::toggle_sticky`] for a point the map draws, and nothing at all for
    /// one it does not, reporting whether the popup ended up pinned.
    pub fn toggle_sticky_if_drawn(&mut self, scope: MapScope<'_>, point_ref: DataPointRef) -> bool {
        scope.draws(point_ref) && self.toggle_sticky(point_ref)
    }

    /// Whether a renderer draws its own hover label for `candidate`. The pinned
    /// popup, any open popup, and the compound multi-hover label each take that
    /// label's place.
    pub fn shows_hover_label(&self, candidate: DataPointRef, any_popup_open: bool) -> bool {
        self.sticky != Some(candidate) && !any_popup_open && !self.suppress_hover_labels
    }

    pub fn primary_hover_is_tpv(&self) -> bool {
        matches!(self.hover, Some(HighlightScope::Point(r)) if r.category == DataCategory::Tpv)
    }

    /// The track the plot cursor points at, and `None` until the cursor has
    /// snapped to a data point (see [`Self::plot_hover_snapped`]).
    pub fn snapped_plot_hover_track(&self) -> Option<TrackRef> {
        let (fi, ti, _) = self.plot_hover_snapped.then_some(self.plot_hover_point)??;
        Some(TrackRef::new(fi, ti))
    }

    /// Whether the hover is on `track` as a whole, and not on one of its
    /// points or categories.
    pub fn hovers_the_whole_track(&self, track: TrackRef) -> bool {
        self.hover.is_some_and(
            |scope| matches!(scope, HighlightScope::Track(hovered) if hovered == track),
        )
    }

    /// Whether the hover or the plot cursor is on a point of `track`.
    pub fn hovers_a_point_of_track(&self, track: TrackRef) -> bool {
        self.hover.is_some_and(
            |scope| matches!(scope, HighlightScope::Point(point) if point.track == track),
        ) || self.snapped_plot_hover_track() == Some(track)
    }

    /// Whether the hover or the plot cursor is on anything belonging to
    /// `file`.
    pub fn hovers_anything_in_file(&self, file: FileIdx) -> bool {
        self.hover.is_some_and(|scope| match scope {
            HighlightScope::Point(point) => point.track.fi == file,
            HighlightScope::Track(track) | HighlightScope::TrackCategory { track, .. } => {
                track.fi == file
            }
            HighlightScope::File { file_index } => file_index == file,
        }) || self
            .snapped_plot_hover_track()
            .is_some_and(|track| track.fi == file)
    }

    /// What the pinned popup does this frame, dropping a pin whose element is
    /// gone. Called once per frame by the map, and by the headless tests.
    pub fn pin_this_frame(&mut self, scope: MapScope<'_>) -> Option<PinnedPopup> {
        let pinned = self.sticky?;
        let withheld = |reason| Some(PinnedPopup::Withheld { pinned, reason });
        match scope.point_visibility(pinned) {
            PointVisibility::Shown => Some(PinnedPopup::Drawn(pinned)),
            // The element itself is gone (its file unloaded, or the array it
            // indexed shrank), so the pin is dropped before whatever later
            // occupies that index can rebind it.
            PointVisibility::NoSuchElement => {
                self.sticky = None;
                None
            }
            PointVisibility::TrackNotShown => withheld(PinWithheld::TrackNotShown),
            PointVisibility::CategoryHidden => withheld(PinWithheld::CategoryHidden),
            PointVisibility::HiddenByQuery => withheld(PinWithheld::HiddenByQuery),
            PointVisibility::OutsideTimeFilter => withheld(PinWithheld::OutsideTimeFilter),
        }
    }
}

/// What the pin does while a point is pinned, from
/// [`MapHighlight::pin_this_frame`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinnedPopup {
    /// The map draws the element, so its popup opens.
    Drawn(DataPointRef),
    /// The pin is remembered but shows nothing, because the map does not draw
    /// the point. Widening the filter or clearing the query brings the popup
    /// back.
    Withheld {
        pinned: DataPointRef,
        reason: PinWithheld,
    },
}

/// Why a pinned popup shows nothing: the ways the map can withhold a point that
/// still exists.
///
/// [`PointVisibility`] also covers "drawn" and "no such element", which are not
/// withholdings.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PinWithheld {
    /// The file or track is off in the tree, or the track fails the filter.
    TrackNotShown,
    /// The element's category is off in the tree or in the display mask.
    CategoryHidden,
    /// A `keep` or `hide` query removed the point.
    HiddenByQuery,
    /// Outside the global time filter's window.
    OutsideTimeFilter,
}

impl Default for MapHighlight {
    fn default() -> Self {
        Self {
            hover: None,
            sticky: None,
            hover_candidates: HoverCandidates::default(),
            plot_hover_time: None,
            plot_hover_point: None,
            plot_hover_snapped: false,
            scrub_time: None,
            suppress_hover_labels: false,
            fading_enabled: true,
            hover_match: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeDelta;
    use gt_filter::GlobalFilter;

    use super::*;
    use crate::display_mask::DisplayCategory;
    use crate::scope_fixture::{self, POINT_COUNT, ScopeFixture};

    fn point(index: usize) -> DataPointRef {
        DataPointRef {
            track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
            category: DataCategory::Tpv,
            point_index: PointIdx::new(index),
        }
    }

    /// Each way the map can stop drawing a pinned point: a point that still
    /// exists keeps its pin and shows nothing, one that does not loses the pin.
    #[rstest::rstest]
    #[case::drawn(|_: &mut ScopeFixture| {}, Some(PinnedPopup::Drawn(scope_fixture::point(1))))]
    #[case::hidden_by_query(
        |fixture: &mut ScopeFixture| fixture.hide_point(1),
        Some(PinnedPopup::Withheld {
            pinned: scope_fixture::point(1),
            reason: PinWithheld::HiddenByQuery,
        })
    )]
    #[case::outside_the_time_filter(
        |fixture: &mut ScopeFixture| {
            fixture.filter.time_start = Some(scope_fixture::start() + TimeDelta::seconds(2));
        },
        Some(PinnedPopup::Withheld {
            pinned: scope_fixture::point(1),
            reason: PinWithheld::OutsideTimeFilter,
        })
    )]
    #[case::track_switched_off(
        |fixture: &mut ScopeFixture| fixture.visibility.files[0].tracks[0].enabled = false,
        Some(PinnedPopup::Withheld {
            pinned: scope_fixture::point(1),
            reason: PinWithheld::TrackNotShown,
        })
    )]
    #[case::category_masked(
        |fixture: &mut ScopeFixture| {
            fixture
                .display_mask
                .set_visible(DisplayCategory::TrackPoints, false);
        },
        Some(PinnedPopup::Withheld {
            pinned: scope_fixture::point(1),
            reason: PinWithheld::CategoryHidden,
        })
    )]
    fn a_pin_reports_what_the_map_does_with_its_point(
        #[case] withhold: fn(&mut ScopeFixture),
        #[case] expected: Option<PinnedPopup>,
    ) {
        let mut fixture = ScopeFixture::all_drawn();
        withhold(&mut fixture);
        let mut highlight = MapHighlight {
            sticky: Some(scope_fixture::point(1)),
            ..MapHighlight::default()
        };
        assert_eq!(highlight.pin_this_frame(fixture.scope()), expected);
        assert_eq!(
            highlight.sticky,
            Some(scope_fixture::point(1)),
            "a point that still exists keeps its pin"
        );
    }

    /// The recording a hover belongs to, over every scope the map and the plot
    /// can report.
    #[rstest::rstest]
    #[case::point_of_the_file(Some(HighlightScope::Point(point(1))), false, true)]
    #[case::track_of_the_file(
        Some(HighlightScope::Track(TrackRef::new(FileIdx::new(0), TrackIdx::new(0)))),
        false,
        true
    )]
    #[case::category_of_a_track_of_the_file(
        Some(HighlightScope::TrackCategory {
            track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
            category: DataCategory::Tpv,
        }),
        false,
        true
    )]
    #[case::the_file_itself(Some(HighlightScope::File { file_index: FileIdx::new(0) }), false, true)]
    #[case::track_of_another_file(
        Some(HighlightScope::Track(TrackRef::new(FileIdx::new(1), TrackIdx::new(0)))),
        false,
        false
    )]
    #[case::another_file(Some(HighlightScope::File { file_index: FileIdx::new(1) }), false, false)]
    #[case::nothing_hovered(None, false, false)]
    #[case::plot_cursor_on_a_track_of_the_file(None, true, true)]
    fn hovers_anything_in_file_covers_every_hover_scope(
        #[case] hover: Option<HighlightScope>,
        #[case] plot_cursor_on_the_file: bool,
        #[case] expected: bool,
    ) {
        let highlight = MapHighlight {
            hover,
            plot_hover_point: plot_cursor_on_the_file.then_some((
                FileIdx::new(0),
                TrackIdx::new(0),
                PointIdx::new(1),
            )),
            plot_hover_snapped: plot_cursor_on_the_file,
            ..MapHighlight::default()
        };
        assert_eq!(highlight.hovers_anything_in_file(FileIdx::new(0)), expected);
    }

    /// A pin whose element is gone is dropped, so a later load cannot rebind it
    /// to whatever occupies that index next.
    #[test]
    fn a_pin_on_a_vanished_element_is_dropped() {
        let fixture = ScopeFixture::all_drawn();
        let stale = scope_fixture::point(POINT_COUNT);
        let mut highlight = MapHighlight {
            sticky: Some(stale),
            ..MapHighlight::default()
        };
        assert_eq!(highlight.pin_this_frame(fixture.scope()), None);
        assert_eq!(highlight.sticky, None);
    }

    #[test]
    fn no_pin_reports_nothing() {
        let fixture = ScopeFixture::all_drawn();
        let mut highlight = MapHighlight::default();
        assert_eq!(highlight.pin_this_frame(fixture.scope()), None);
    }

    /// A click pins only what the map draws - the rule every click site shares.
    #[test]
    fn a_click_on_a_withheld_point_pins_nothing() {
        let mut fixture = ScopeFixture::all_drawn();
        fixture.hide_point(1);
        let mut highlight = MapHighlight::default();
        assert!(
            !highlight.toggle_sticky_if_drawn(fixture.scope(), scope_fixture::point(1)),
            "the hidden point does not pin"
        );
        assert_eq!(highlight.sticky, None);
        assert!(
            highlight.toggle_sticky_if_drawn(fixture.scope(), scope_fixture::point(0)),
            "the drawn point next to it does"
        );
        assert_eq!(highlight.sticky, Some(scope_fixture::point(0)));
    }

    /// The fixture's own filter default draws everything, so a case that changes
    /// nothing must not be silently passing for the wrong reason.
    #[test]
    fn the_fixture_draws_every_point_by_default() {
        let fixture = ScopeFixture::all_drawn();
        assert_eq!(fixture.filter, GlobalFilter::default());
        for index in 0..POINT_COUNT {
            assert!(
                fixture.scope().draws(scope_fixture::point(index)),
                "point {index} must be drawn before a case withholds it"
            );
        }
    }

    #[test]
    fn toggling_the_same_point_unpins_it_and_another_takes_over() {
        let mut highlight = MapHighlight::default();
        assert!(highlight.toggle_sticky(point(3)), "a first click pins");
        assert_eq!(highlight.sticky, Some(point(3)));
        assert!(
            !highlight.toggle_sticky(point(3)),
            "clicking the pinned point unpins it"
        );
        assert_eq!(highlight.sticky, None);
        highlight.toggle_sticky(point(3));
        assert!(
            highlight.toggle_sticky(point(7)),
            "another point takes the pin over"
        );
        assert_eq!(highlight.sticky, Some(point(7)));
    }

    #[test]
    fn match_highlight_contains_is_start_inclusive_end_exclusive() {
        let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
        let hm = MatchHighlight::new(track, &(150..300));
        assert!(hm.contains(150));
        assert!(hm.contains(299));
        assert!(!hm.contains(149));
        assert!(!hm.contains(300));
    }
}
