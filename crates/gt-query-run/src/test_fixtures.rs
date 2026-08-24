//! Shared fixtures for this crate's tests: two nav points, channels over their
//! time span, and the loaded files carrying them.

use std::ops::Range;
use std::path::PathBuf;
use std::sync::Arc;

use chrono::DateTime;
use geotrace_sdk_units::ChannelUnit;
use gt_track_builder::{FileMeta, SegmentationConfig};
use gt_types::coordinates::{Latitude, Longitude};
use gt_types::satellites::{Constellation, Satellite, Satellites};
use gt_types::time_types::GpsTime;
use gt_types::tpv::TimePositionVelocity;
use gt_types::{Channel, FileSource, LoadedFile, LoadedTrack, NavPoint, TrackLod};
use rustc_hash::FxHashMap;
use uom::si::angle::degree;
use uom::si::f64::{Angle, Velocity};
use uom::si::velocity::kilometer_per_hour;

/// The Unix epoch the fixtures place their first point and sample at.
pub(crate) const TEST_EPOCH: i64 = 1_700_000_000;

/// A range built from arguments, so a single-element `vec![rng(0, 1)]` does not
/// trip clippy's `single_range_in_vec_init`.
pub(crate) fn rng(start: usize, end: usize) -> Range<usize> {
    start..end
}

/// One point with a satellite report at 36 km/h, one without any of it,
/// exercising the provider's unit conversions and count folds.
pub(crate) fn test_points() -> Vec<NavPoint> {
    let time = |secs: i64| {
        GpsTime::from_utc(DateTime::from_timestamp(TEST_EPOCH + secs, 0).expect("valid timestamp"))
    };
    let sat = |constellation, in_fix| {
        Satellite::new(constellation, 1, Some(45.0), None, Some(40.0), in_fix)
    };
    let with_sats = TimePositionVelocity::builder()
        .time(time(0))
        .lat(Latitude::new(55.5))
        .lon(Longitude::new(12.25))
        .velocity(Velocity::new::<kilometer_per_hour>(36.0))
        .heading(Angle::new::<degree>(90.0))
        .eph_m(2.5)
        .build();
    let bare = TimePositionVelocity::builder()
        .time(time(1))
        .lat(Latitude::new(55.6))
        .lon(Longitude::new(12.35))
        .build();
    vec![
        NavPoint::new(
            with_sats,
            Some(Satellites::new(
                Some(time(0)),
                None,
                vec![
                    sat(Constellation::Gps, true),
                    sat(Constellation::Gps, false),
                    sat(Constellation::Galileo, true),
                ],
            )),
        ),
        NavPoint::new(bare, None),
    ]
}

/// A scalar channel named `name` with `unit`, sampled at `TEST_EPOCH + secs`
/// for each `(secs, value)` pair.
pub(crate) fn scalar_channel(name: &str, unit: Option<&str>, samples: &[(i64, f64)]) -> Channel {
    Channel {
        name: name.to_owned(),
        unit: unit.map(ChannelUnit::from_file_label),
        period: None,
        description: None,
        components: vec![],
        times: sample_times(samples.iter().map(|&(secs, _)| secs)),
        values: samples.iter().map(|&(_, value)| value).collect(),
    }
}

/// A 3-component vector channel, each row `[x, y, z]` at `TEST_EPOCH + secs`.
pub(crate) fn vector_channel(
    name: &str,
    unit: Option<&str>,
    components: &[&str],
    samples: &[(i64, [f64; 3])],
) -> Channel {
    Channel {
        name: name.to_owned(),
        unit: unit.map(ChannelUnit::from_file_label),
        period: None,
        description: None,
        components: components.iter().map(|c| (*c).to_owned()).collect(),
        times: sample_times(samples.iter().map(|&(secs, _)| secs)),
        values: samples.iter().flat_map(|&(_, row)| row).collect(),
    }
}

fn sample_times(offsets: impl Iterator<Item = i64>) -> Vec<DateTime<chrono::Utc>> {
    offsets
        .map(|secs| DateTime::from_timestamp(TEST_EPOCH + secs, 0).expect("valid timestamp"))
        .collect()
}

/// A single-track file carrying `channels` over [`test_points`].
pub(crate) fn file_with_channels(channels: Vec<Channel>) -> LoadedFile {
    LoadedFile {
        metadata: gt_test_utils::empty_file_metadata(),
        tracks: vec![LoadedTrack {
            metadata: gt_test_utils::empty_track_metadata(),
            points: test_points(),
            lod: TrackLod::default(),
            sat_label_anchors: Vec::new(),
            custom_markers: vec![],
            generated_markers: vec![],
            event_markers: vec![],
            channels,
        }],
        event_marker_styles: FxHashMap::default(),
        orphaned_event_markers: vec![],
        source: FileSource::GtdBytes(Arc::from(Vec::<u8>::new())),
        load_warnings: vec![],
    }
}

/// A file built the way the loader builds one, over the shared nav fixture.
pub(crate) fn loaded_file() -> LoadedFile {
    let points = gt_test_utils::nav_test_data();
    gt_track_builder::build_loaded_file(
        "ride.gtd".to_owned(),
        &points,
        &[],
        vec![],
        vec![],
        &[],
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("ride.gtd")),
        FileMeta::default(),
        vec![],
    )
}
