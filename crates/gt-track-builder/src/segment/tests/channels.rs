use std::path::PathBuf;

use chrono::{DateTime, TimeZone as _, Utc};
use geotrace_sdk_units::Unit;
use gt_types::channel::Channel;
use gt_types::track::FileSource;

use crate::segment::{self, FileMeta, SegmentationConfig};
use crate::test_util;

fn utc(secs: i64) -> DateTime<Utc> {
    Utc.timestamp_opt(secs, 0)
        .single()
        .expect("valid timestamp")
}

#[test]
fn channels_partition_to_tracks_by_timestamp() {
    // Two tracks: [0, 60] and [3600, 3660]. A scalar channel with samples in
    // track 1 (0, 30), the gap (1800), and track 2 (3600).
    let pts = vec![
        test_util::fix_without_a_satellite_report(0),
        test_util::fix_without_a_satellite_report(60),
        test_util::fix_without_a_satellite_report(3600),
        test_util::fix_without_a_satellite_report(3660),
    ];
    let channel = Channel {
        name: "incline".to_owned(),
        unit: Some(Unit::DEG.into()),
        period: None,
        description: None,
        components: vec![],
        times: vec![utc(0), utc(30), utc(1800), utc(3600)],
        values: vec![1.0, 2.0, 9.0, 3.0],
    };
    let f = segment::build_loaded_file(
        "ride.gtd".to_owned(),
        &pts,
        &[],
        vec![],
        vec![],
        std::slice::from_ref(&channel),
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("ride.gtd")),
        FileMeta::default(),
        vec![],
    );
    assert_eq!(f.tracks.len(), 2);

    // Track 1 keeps the two in-range samples. The gap sample (1800) is
    // dropped.
    let t0 = &f.tracks[0].channels;
    assert_eq!(t0.len(), 1);
    assert_eq!(t0[0].name, "incline");
    assert_eq!(t0[0].times, vec![utc(0), utc(30)]);
    assert_eq!(t0[0].values, vec![1.0, 2.0]);

    // Track 2 keeps its single sample.
    let t1 = &f.tracks[1].channels;
    assert_eq!(t1.len(), 1);
    assert_eq!(t1[0].times, vec![utc(3600)]);

    // Reassembly concatenates the per-track slices back in time order.
    // The dropped gap sample stays dropped.
    let reassembled = segment::reassemble_channels(&f.tracks);
    assert_eq!(reassembled.len(), 1);
    assert_eq!(reassembled[0].times, vec![utc(0), utc(30), utc(3600)]);
    assert_eq!(reassembled[0].values, vec![1.0, 2.0, 3.0]);

    // The one gap sample (1800) that landed in no track is surfaced as a warning.
    let warning = f
        .load_warnings
        .iter()
        .find(|w| w.issue.contains("outside every track"))
        .expect("dropped gap sample should be reported");
    assert_eq!(warning.count, 1);
}

#[test]
fn a_vector_channel_partitions_and_reassembles_with_columns_aligned() {
    // Two tracks split at the 3600s gap. A 3-component accel channel with two
    // samples in track 1, one in the gap (dropped), and one in track 2.
    let pts = vec![
        test_util::fix_without_a_satellite_report(0),
        test_util::fix_without_a_satellite_report(60),
        test_util::fix_without_a_satellite_report(3600),
        test_util::fix_without_a_satellite_report(3660),
    ];
    let channel = Channel {
        name: "accel".to_owned(),
        unit: Some(Unit::G.into()),
        period: None,
        description: None,
        components: vec!["x".to_owned(), "y".to_owned(), "z".to_owned()],
        times: vec![utc(0), utc(30), utc(1800), utc(3600)],
        // Row-major: four samples of (x, y, z).
        values: vec![
            0.0, 0.1, 1.0, // t=0
            1.0, 1.1, 2.0, // t=30
            8.0, 8.1, 8.2, // t=1800 (gap, dropped)
            3.0, 3.1, 4.0, // t=3600
        ],
    };
    let f = segment::build_loaded_file(
        "ride.gtd".to_owned(),
        &pts,
        &[],
        vec![],
        vec![],
        std::slice::from_ref(&channel),
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("ride.gtd")),
        FileMeta::default(),
        vec![],
    );

    // Track 1 keeps rows 0 and 1 with their columns intact.
    let t0 = &f.tracks[0].channels[0];
    assert_eq!(t0.components, ["x", "y", "z"]);
    assert_eq!(t0.times, vec![utc(0), utc(30)]);
    assert_eq!(t0.values, vec![0.0, 0.1, 1.0, 1.0, 1.1, 2.0]);
    // Track 2 keeps the last row.
    assert_eq!(f.tracks[1].channels[0].values, vec![3.0, 3.1, 4.0]);

    // Reassembly restores the surviving rows in time order, columns aligned.
    let reassembled = segment::reassemble_channels(&f.tracks);
    assert_eq!(reassembled[0].components, ["x", "y", "z"]);
    assert_eq!(reassembled[0].times, vec![utc(0), utc(30), utc(3600)]);
    assert_eq!(
        reassembled[0].values,
        vec![0.0, 0.1, 1.0, 1.0, 1.1, 2.0, 3.0, 3.1, 4.0]
    );
}

#[test]
fn a_channel_absent_from_a_track_is_not_attached() {
    // Channel samples only in track 2's range. Track 1 has no channel.
    let pts = vec![
        test_util::fix_without_a_satellite_report(0),
        test_util::fix_without_a_satellite_report(3600),
        test_util::fix_without_a_satellite_report(3660),
    ];
    let channel = Channel {
        name: "accel".to_owned(),
        unit: None,
        period: None,
        description: None,
        components: vec![],
        times: vec![utc(3600), utc(3660)],
        values: vec![1.0, 2.0],
    };
    let f = segment::build_loaded_file(
        "ride.gtd".to_owned(),
        &pts,
        &[],
        vec![],
        vec![],
        std::slice::from_ref(&channel),
        &SegmentationConfig::default(),
        FileSource::GtdPath(PathBuf::from("ride.gtd")),
        FileMeta::default(),
        vec![],
    );
    assert_eq!(f.tracks.len(), 2);
    assert!(f.tracks[0].channels.is_empty());
    assert_eq!(f.tracks[1].channels.len(), 1);
}
