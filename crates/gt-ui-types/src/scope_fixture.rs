//! A minimal loaded state whose [`MapScope`] the tests of this crate build on.
//!
//! Files and tracks come from [`gt_track_builder::build_loaded_file`], so no test
//! hand-writes a [`LoadedFile`] and a field added to a loaded recording never
//! reaches here.

use std::collections::HashMap;
use std::path::PathBuf;

use chrono::{DateTime, TimeDelta, Utc};
use gt_filter::GlobalFilter;
use gt_types::{
    DataCategory, FileIdx, FileSource, GpsTime, Latitude, LoadedFile, Longitude, NavPoint,
    PointIdx, TimePositionVelocity, TrackIdx, TrackRef,
};

use crate::display_mask::DisplayMask;
use crate::highlight::DataPointRef;
use crate::query_matches::QueryMatches;
use crate::visibility::{MapScope, TrackDataVisibility};

/// Points in the one fixture track.
pub const POINT_COUNT: usize = 4;

/// The fixture's first point, one second per point after it.
pub fn start() -> DateTime<Utc> {
    DateTime::from_timestamp(1_748_000_000, 0).expect("fixed timestamp is valid")
}

/// One track of [`POINT_COUNT`] points, a second apart, built the way loading
/// builds it.
pub fn one_track_file() -> Vec<LoadedFile> {
    let points: Vec<NavPoint> = (0..POINT_COUNT)
        .map(|index| {
            let tpv = TimePositionVelocity::builder()
                .time(GpsTime::from_utc(
                    start() + TimeDelta::seconds(index as i64),
                ))
                .lat(Latitude::new(55.0))
                .lon(Longitude::new(12.0))
                .build();
            NavPoint::new(tpv, None)
        })
        .collect();
    vec![gt_track_builder::build_loaded_file(
        "scope.gtd".to_owned(),
        &points,
        &[],
        Vec::new(),
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
pub fn point(index: usize) -> DataPointRef {
    DataPointRef {
        track: track0(),
        category: DataCategory::Tpv,
        point_index: PointIdx::new(index),
    }
}

/// The owned pieces a [`MapScope`] borrows, so a test can withhold a point in
/// each of the ways the map can and then ask the real rule what it does.
pub struct ScopeFixture {
    pub files: Vec<LoadedFile>,
    pub visibility: TrackDataVisibility,
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
            hidden: HashMap::from([(track0(), hidden)]),
            ..QueryMatches::default()
        });
    }

    pub fn scope(&self) -> MapScope<'_> {
        MapScope {
            files: &self.files,
            visibility: &self.visibility,
            filter: &self.filter,
            display_mask: self.display_mask,
            query_matches: self.query_matches.as_ref(),
        }
    }
}
