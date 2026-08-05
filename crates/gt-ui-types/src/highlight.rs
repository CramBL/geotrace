use chrono::{DateTime, Utc};
use gt_types::{DataCategory, FileIdx, PointIdx, TrackIdx, TrackRef};

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
    /// track plot as a vertical time line so playback reads there the same way
    /// hovering a track point does. `None` when the sky-trails window is not
    /// driving a scrub. Set by that window and read one frame behind (it draws
    /// after the plot), the same way [`Self::hover_match`] is.
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
    /// Pin `point_ref`'s popup, or unpin it when it is already the sticky
    /// point, and report whether it ended up pinned - the caller places the
    /// popup only for one that opened.
    ///
    /// Shared by every way of selecting a point (the map click, the
    /// disambiguation popup, the side-panel and query-table rows) so they
    /// cannot disagree on what a second click does.
    pub fn toggle_sticky(&mut self, point_ref: DataPointRef) -> bool {
        let pinned = self.sticky != Some(point_ref);
        self.sticky = pinned.then_some(point_ref);
        pinned
    }
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
    use super::*;

    fn point(index: usize) -> DataPointRef {
        DataPointRef {
            track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
            category: DataCategory::Tpv,
            point_index: PointIdx::new(index),
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
