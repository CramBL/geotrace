//! A minimal loaded state whose [`MapPresence`] the tests of this crate build on.
//!
//! Files and tracks come from [`gt_track_builder::build_loaded_file`], so no test
//! hand-writes a [`LoadedFile`] and a field added to a loaded recording never
//! reaches here.

#![cfg(test)]

use std::iter;
use std::path::PathBuf;

use chrono::{DateTime, TimeDelta, Utc};
use gt_filter::GlobalFilter;
use gt_types::fixtures::FixKind;
use gt_types::{
    EventMarker, EventMarkerIdx, EventMarkerRef, FileIdx, FileSource, FixRef, GeneratedMarkerIdx,
    GeneratedMarkerKindTag, GeneratedMarkerRef, Latitude, LoadedFile, Longitude, NavPoint,
    PointIdx, TrackIdx, TrackRef,
};

use crate::display_mask::DisplayMask;
use crate::event_marker_visibility::EventMarkerVisibility;
use crate::generated_marker_visibility::GeneratedMarkerVisibility;
use crate::highlight::MapElementRef;
use crate::query_matches::{QueryMatches, TrackRanges};
use crate::visibility::{MapEligibility, MapPresence, TrackDataVisibility};

/// The fixture's first point, one second per point after it.
pub fn start() -> DateTime<Utc> {
    DateTime::from_timestamp(1_748_000_000, 0).expect("fixed timestamp is valid")
}

/// One track of [`MEASURED_FIX_COUNT`] measured fixes and one ghost fix after
/// them, a second apart, built the way loading builds it.
///
/// The track has one event marker at [`EVENT_MARKER_PATH`] and one
/// [`GeneratedMarkerKindTag::GnssFixLost`] marker, which the track builder
/// places at the last measured fix.
pub fn one_track_file() -> Vec<LoadedFile> {
    let points: Vec<NavPoint> = (0..POINT_COUNT)
        .map(|index| {
            let kind = if index < MEASURED_FIX_COUNT {
                FixKind::Measured
            } else {
                FixKind::GhostWithoutSatellitesInFix
            };
            gt_types::fixtures::nav_point(
                start() + TimeDelta::seconds(index as i64),
                Latitude::new(55.0),
                Longitude::new(12.0),
                kind,
            )
        })
        .collect();
    let event_marker = EventMarker::new(
        start() + TimeDelta::seconds(1),
        EVENT_MARKER_PATH.to_owned(),
        None,
        Latitude::new(55.0),
        Longitude::new(12.0),
    );
    vec![gt_track_builder::build_loaded_file(
        "scope.gtd".to_owned(),
        &points,
        &[],
        vec![event_marker],
        Vec::new(),
        &[],
        &gt_track_builder::SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("scope.gtd")),
        gt_track_builder::FileMeta::default(),
        Vec::new(),
    )]
}

/// The fixture's only track.
pub fn track0() -> TrackRef {
    TrackRef::new(FileIdx::new(0), TrackIdx::new(0))
}

/// One TPV point of that track.
pub fn point(index: usize) -> MapElementRef {
    MapElementRef::Fix(FixRef::new(track0(), PointIdx::new(index)))
}

pub fn event_marker() -> MapElementRef {
    MapElementRef::EventMarker(EventMarkerRef::new(track0(), EventMarkerIdx::new(0)))
}

pub fn generated_marker() -> MapElementRef {
    MapElementRef::GeneratedMarker(GeneratedMarkerRef::new(
        track0(),
        GeneratedMarkerIdx::new(0),
    ))
}

/// The owned pieces a [`MapPresence`] borrows, letting a test withhold a point in
/// each of the ways the map can and evaluate the real visibility rule.
pub struct ScopeFixture {
    pub files: Vec<LoadedFile>,
    pub visibility: TrackDataVisibility,
    pub event_marker_visibility: EventMarkerVisibility,
    pub generated_marker_visibility: GeneratedMarkerVisibility,
    pub filter: GlobalFilter,
    pub display_mask: DisplayMask,
    pub query_matches: Option<QueryMatches>,
}

impl ScopeFixture {
    /// Everything drawn: the tree all on, no filter, no query run.
    pub fn all_drawn() -> Self {
        let files = one_track_file();
        Self {
            visibility: TrackDataVisibility::from_loaded(&files),
            files,
            event_marker_visibility: EventMarkerVisibility::default(),
            generated_marker_visibility: GeneratedMarkerVisibility::default(),
            filter: GlobalFilter::default(),
            display_mask: DisplayMask::default(),
            query_matches: None,
        }
    }

    /// Hide one of the fixture track's points, as a `keep`/`hide` query run
    /// would.
    pub fn hide_point(&mut self, index: usize) {
        let hidden: Vec<_> = std::iter::once(index..index + 1).collect();
        self.query_matches = Some(QueryMatches {
            hidden: TrackRanges::from_iter([(track0(), hidden)]),
            ..QueryMatches::default()
        });
    }

    /// Hide `path` in the fixture track's event marker tree, which hides every
    /// event marker at `path` or under it.
    pub fn hide_event_marker_path(&mut self, path: &str) {
        self.event_marker_visibility
            .set_hidden(track0(), iter::once(path.to_owned()));
    }

    pub fn hide_generated_marker_kind(&mut self, kind: GeneratedMarkerKindTag) {
        self.generated_marker_visibility
            .set_hidden(track0(), iter::once(kind));
    }

    /// Show every event marker path and generated marker kind again, as the
    /// tree does once the user ticks them.
    pub fn show_every_marker_type(&mut self) {
        self.event_marker_visibility.clear_all();
        self.generated_marker_visibility.clear_all();
    }

    pub fn scope(&self) -> MapPresence<'_> {
        MapEligibility::new(
            &self.files,
            &self.visibility,
            &self.filter,
            self.query_matches.as_ref(),
            &self.generated_marker_visibility,
            &self.event_marker_visibility,
        )
        .with_display_mask(self.display_mask)
    }
}

/// Points in the one fixture track.
pub const POINT_COUNT: usize = MEASURED_FIX_COUNT + 1;

const MEASURED_FIX_COUNT: usize = 4;

/// The variant path of the fixture track's event marker.
pub const EVENT_MARKER_PATH: &str = "power/boot";

/// The parent path of [`EVENT_MARKER_PATH`].
pub const EVENT_MARKER_PARENT_PATH: &str = "power";
