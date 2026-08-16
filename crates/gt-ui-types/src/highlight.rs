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

#[derive(Debug, Clone, Copy)]
pub struct MapHighlight {
    pub hover: Option<HighlightScope>,
    pub sticky: Option<DataPointRef>,
    /// All hovered candidates within the cursor radius, one per category group.
    /// Indices: 0 = Tpv/SatelliteReport, 1 = EventMarker, 2 = CustomMarker,
    /// 3 = GeneratedMarker. Used so renderers can show tooltips for secondary
    /// candidates even when a Tpv point is the primary hover.
    pub hover_candidates: [Option<DataPointRef>; 4],
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
            hover_candidates: [None; 4],
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
