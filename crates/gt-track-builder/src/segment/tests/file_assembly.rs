use std::path::PathBuf;

use gt_types::track::{FileSource, TravelMode};

use crate::segment::{self, FileMeta, SegmentationConfig};
use crate::test_util;

#[test]
fn build_loaded_file_empty_points() {
    let f = segment::build_loaded_file(
        "test.gtd".to_owned(),
        &[],
        &[],
        vec![],
        vec![],
        &[],
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("test.gtd")),
        FileMeta::default(),
        vec![],
    );
    assert!(f.tracks.is_empty());
    assert_eq!(f.metadata.filename, "test.gtd");
    assert_eq!(f.metadata.time_range, None);
}

#[test]
fn build_loaded_file_numbers_two_tracks_from_one_recording() {
    let pts = vec![
        test_util::fix_without_a_satellite_report(0),
        test_util::fix_without_a_satellite_report(60),
        test_util::fix_without_a_satellite_report(3600), // gap → new track
        test_util::fix_without_a_satellite_report(3660),
    ];
    let f = segment::build_loaded_file(
        "ride.gtd".to_owned(),
        &pts,
        &[],
        vec![],
        vec![],
        &[],
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("ride.gtd")),
        FileMeta::default(),
        vec![],
    );
    assert_eq!(f.tracks.len(), 2);
    assert_eq!(f.tracks[0].points.len(), 2);
    assert_eq!(f.tracks[1].points.len(), 2);
    assert_eq!(f.tracks[0].metadata.index, 1);
    assert_eq!(f.tracks[1].metadata.index, 2);
}

#[test]
fn build_loaded_file_carries_file_meta() {
    let pts = vec![
        test_util::fix_with_a_satellite_in_fix(0),
        test_util::fix_with_a_satellite_in_fix(60),
    ];
    let file_meta = FileMeta {
        title: Some("Morning ride".to_owned()),
        device: Some("uBlox F9P".to_owned()),
        notes: Some("cross-town".to_owned()),
        travel_mode: Some(TravelMode::Bicycle),
    };
    let f = segment::build_loaded_file(
        "ride.gtd".to_owned(),
        &pts,
        &[],
        vec![],
        vec![],
        &[],
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("ride.gtd")),
        file_meta,
        vec![],
    );
    assert_eq!(f.metadata.title.as_deref(), Some("Morning ride"));
    assert_eq!(f.metadata.device.as_deref(), Some("uBlox F9P"));
    assert_eq!(f.metadata.notes.as_deref(), Some("cross-town"));
    assert_eq!(f.metadata.travel_mode, Some(TravelMode::Bicycle));

    // Round-trip: rebuilding from the built metadata (the re-segmentation
    // path) preserves the fields.
    let recovered = FileMeta::from(&f.metadata);
    assert_eq!(recovered.title.as_deref(), Some("Morning ride"));
    assert_eq!(recovered.device.as_deref(), Some("uBlox F9P"));
    assert_eq!(recovered.notes.as_deref(), Some("cross-town"));
    assert_eq!(recovered.travel_mode, Some(TravelMode::Bicycle));
}
