use chrono::{DateTime, Utc};
use gt_types::{DataCategory, FileIdx, PointIdx, TrackIdx, TrackRef};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct DataPointRef {
    pub track: TrackRef,
    pub category: DataCategory,
    pub point_index: PointIdx,
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
    /// Time currently hovered on the track plot; used to cross-highlight the
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
    /// Only when this is `true` does the map overlay activate for plot hover —
    /// prevents the map from dimming the moment the cursor crosses the plot
    /// boundary, before it is actually near any data.
    pub plot_hover_snapped: bool,
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
            suppress_hover_labels: false,
            fading_enabled: true,
        }
    }
}
