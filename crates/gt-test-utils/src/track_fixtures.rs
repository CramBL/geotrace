//! Loaded tracks and loaded files assembled by hand, over the geometry and
//! the metadata that the track builder measures for their fixes.

use std::path::PathBuf;

use chrono::{DateTime, Duration, Utc};
use uom::si::f64::Length;
use uom::si::length::kilometer;

use gt_types::channel::Channel;
use gt_types::{
    CustomMarker, EventMarker, EventMarkerStyle, FileMetadata, FileSource, FixStats, LoadWarning,
    LoadedFile, LoadedTrack, NavPoint, TimeRange, TotalDistance, TrackAggregates, TrackGeometry,
    TrackMetadata,
};

/// Track metadata with every count zeroed, at the epoch, for tests that fill
/// in only the fields they exercise.
pub fn empty_track_metadata() -> TrackMetadata {
    TrackMetadata {
        index: 0,
        duration: Duration::zero(),
        time_range: TimeRange::new(DateTime::UNIX_EPOCH, DateTime::UNIX_EPOCH),
        has_custom_markers: false,
        tpv_count: 0,
        invalid_position_count: 0,
        satellite_report_count: 0,
        custom_marker_count: 0,
        generated_marker_count: 0,
        event_marker_count: 0,
        fix_stats: None,
    }
}

/// File metadata with an unnamed file, every measure zeroed and no time range,
/// for tests that fill in only the fields they exercise.
pub fn empty_file_metadata() -> FileMetadata {
    FileMetadata {
        filename: String::new(),
        total_distance: TotalDistance::Measured(Length::new::<kilometer>(0.0)),
        total_duration: Duration::zero(),
        time_range: None,
        fix_stats: None,
        title: None,
        device: None,
        notes: None,
        travel_mode: None,
    }
}

/// The geometry that the track builder measures for `points`, taken as a track
/// of their own under the current [`gt_track_builder::FixPlacementRule`], for a
/// test that assembles a [`LoadedTrack`] by hand.
pub fn track_geometry(points: &[NavPoint]) -> TrackGeometry {
    gt_track_builder::segment::measure_track_geometry(
        points,
        gt_track_builder::FixPlacementRule::default(),
    )
}

/// A [`LoadedTrack`] over `points`, with the geometry that the track builder
/// measures for them. Its metadata has the time range, TPV count and
/// out-of-range fix count that the builder derives from `points`, and every
/// other field of [`empty_track_metadata`]. A track of no points takes
/// [`empty_track_metadata`] whole.
pub fn loaded_track_with_points(points: Vec<NavPoint>) -> LoadedTrack {
    let geometry = track_geometry(&points);
    let (metadata, points) = match vec1::Vec1::try_from_vec(points) {
        Ok(points) => {
            let measured = gt_track_builder::segment::compute_track_metadata(0, &points, &[], &[]);
            let metadata = TrackMetadata {
                time_range: measured.time_range,
                tpv_count: measured.tpv_count,
                invalid_position_count: measured.invalid_position_count,
                ..empty_track_metadata()
            };
            (metadata, points.into_vec())
        }
        Err(_) => (empty_track_metadata(), Vec::new()),
    };
    LoadedTrack {
        metadata,
        geometry,
        points,
        lod: gt_types::track::TrackLod::default(),
        sat_label_anchors: Vec::new(),
        custom_markers: Vec::new(),
        generated_markers: Vec::new(),
        event_markers: Vec::new(),
        channels: Vec::new(),
    }
}

/// A [`LoadedFile`] over `tracks`, its aggregates measured over them, loaded
/// from an empty path and with no warning and no orphaned event marker.
pub fn loaded_file_with_tracks(tracks: Vec<LoadedTrack>) -> LoadedFile {
    let mut metadata = empty_file_metadata();
    metadata.set_track_aggregates(TrackAggregates::over_tracks(&tracks));
    LoadedFile {
        metadata,
        tracks,
        event_marker_styles: Default::default(),
        orphaned_event_markers: Vec::new(),
        source: FileSource::GtdPath(PathBuf::new()),
        load_warnings: Vec::new(),
    }
}

/// What a recording holds beside its fixes, for [`build_file`].
#[derive(Default)]
pub struct FileParts {
    pub custom_markers: Vec<CustomMarker>,
    pub event_markers: Vec<EventMarker>,
    pub event_marker_styles: Vec<EventMarkerStyle>,
    pub channels: Vec<Channel>,
    pub meta: gt_track_builder::FileMeta,
    pub load_warnings: Vec<LoadWarning>,
}

/// The recording that the track builder assembles from `points` and `parts`
/// under the default [`gt_track_builder::SegmentationConfig`], loaded from a
/// path of `name`.
pub fn build_file(
    name: &str,
    points: &[NavPoint],
    FileParts {
        custom_markers,
        event_markers,
        event_marker_styles,
        channels,
        meta,
        load_warnings,
    }: FileParts,
) -> LoadedFile {
    gt_track_builder::build_loaded_file(
        name.to_owned(),
        points,
        &custom_markers,
        event_markers,
        event_marker_styles,
        &channels,
        &gt_track_builder::SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from(name)),
        meta,
        load_warnings,
    )
}

/// How long each track of [`segmented_recording`] runs.
const SEGMENTED_TRACK_DURATION: Duration = Duration::minutes(10);

/// A recording of `track_count` tracks, numbered from one the way the track
/// builder numbers a fresh segmentation. Track 1 starts at the Unix epoch and
/// track `n` starts twenty minutes after track `n - 1`. Each track runs ten
/// minutes, measures `n` kilometres and reports `n` fix losses. The file's
/// figures are [`TrackAggregates::over_tracks`] over all of them.
pub fn segmented_recording(track_count: usize) -> LoadedFile {
    let tracks: Vec<LoadedTrack> = (0..track_count)
        .map(|position| {
            let number = u32::try_from(position).unwrap_or(0) + 1;
            let start = DateTime::<Utc>::UNIX_EPOCH + Duration::minutes(20 * i64::from(number - 1));
            let mut track =
                loaded_track_with_points(gt_types::fixtures::nav_points_from(start, 2, 60));
            if let TrackGeometry::Measured(measured) = &mut track.geometry {
                measured.distance_km = Length::new::<kilometer>(f64::from(number));
            }
            track.metadata = TrackMetadata {
                index: position + 1,
                duration: SEGMENTED_TRACK_DURATION,
                time_range: TimeRange::new(start, start + SEGMENTED_TRACK_DURATION),
                fix_stats: Some(FixStats {
                    time_with_fix: Duration::zero(),
                    time_without_fix: Duration::zero(),
                    fix_loss_count: number,
                    max_continuous_no_fix: Duration::zero(),
                }),
                ..empty_track_metadata()
            };
            track
        })
        .collect();
    loaded_file_with_tracks(tracks)
}

#[cfg(test)]
mod tests {
    use gt_types::fixtures;

    use super::*;

    #[test]
    fn loaded_track_with_points_derives_the_time_range_and_the_counts_of_its_points() {
        let start = DateTime::UNIX_EPOCH + Duration::hours(3);
        let mut points = fixtures::nav_points_from(start, 4, 60);
        points.extend(fixtures::nav_points_without_a_valid_position(2));

        let track = loaded_track_with_points(points);

        assert_eq!(track.metadata.tpv_count, 6);
        assert_eq!(track.metadata.invalid_position_count, 2);
        assert_eq!(track.metadata.time_range.start, start);
    }

    #[test]
    fn a_track_of_no_points_takes_the_empty_time_range_and_counts() {
        let track = loaded_track_with_points(Vec::new());

        assert_eq!(track.metadata.tpv_count, 0);
        assert_eq!(track.metadata.invalid_position_count, 0);
        assert_eq!(track.metadata.time_range, empty_track_metadata().time_range);
    }
}
