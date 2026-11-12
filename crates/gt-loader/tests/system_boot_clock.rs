use std::error::Error;
use std::slice;
use std::sync::Arc;

use chrono::{DateTime, Duration, Utc};
use geotrace_sdk::{Angle, BootTime, DebugTimeRepair, NavFile, NavFileBuilder, NavFix, NavFixTime};
use gt_loader::DebugTimeRepairConfig;
use gt_track_builder::{FileMeta, SegmentationConfig, TrackLayoutConfig};
use gt_types::FileSource;
use rstest::rstest;

#[derive(Clone, Copy)]
struct ClockFix {
    receiver_us: Option<i64>,
    system_us: Option<i64>,
    boot_us: Option<u64>,
}

impl ClockFix {
    fn to_fix(self) -> Result<NavFix, Box<dyn Error>> {
        let timestamp = |micros| {
            DateTime::<Utc>::from_timestamp_micros(UTC_EPOCH_US + micros)
                .ok_or("invalid fixture time")
        };
        let time = match (self.receiver_us, self.system_us) {
            (Some(gps), Some(sys)) => NavFixTime::Both {
                gps: timestamp(gps)?,
                sys: timestamp(sys)?,
            },
            (Some(gps), None) => NavFixTime::Receiver(timestamp(gps)?),
            (None, Some(sys)) => NavFixTime::Host(timestamp(sys)?),
            (None, None) => return Err("a fixture requires an absolute time".into()),
        };
        Ok(NavFix::builder()
            .time(time)
            .lat(Angle::degrees(55.0))
            .lon(Angle::degrees(12.0))
            .maybe_boot_time(self.boot_us.map(BootTime::from_micros).transpose()?)
            .build())
    }
}

struct ClockRecording {
    fixes: Vec<ClockFix>,
    repair_tag: Option<Duration>,
}

impl ClockRecording {
    fn bytes(self) -> Result<Vec<u8>, Box<dyn Error>> {
        let mut recorder = NavFileBuilder::new().open();
        for fix in &self.fixes {
            recorder.add_nav_fix(fix.to_fix()?);
        }
        let mut file = recorder.finish()?;
        for (point, fix) in file.nav_points_mut().iter_mut().zip(self.fixes) {
            point.fix = fix.to_fix()?;
        }
        if let Some(threshold) = self.repair_tag {
            file.set_debug_time_repair_tag(DebugTimeRepair::new(threshold)?);
        }
        let mut bytes = Vec::new();
        file.write(&mut bytes)?;
        Ok(bytes)
    }
}

#[test]
fn full_recording_reference_survives_missing_pairs_backward_steps_and_resegmentation() {
    let bytes = ClockRecording {
        fixes: vec![
            ClockFix {
                receiver_us: Some(0),
                system_us: None,
                boot_us: Some(100),
            },
            ClockFix {
                receiver_us: None,
                system_us: Some(1),
                boot_us: None,
            },
            ClockFix {
                receiver_us: None,
                system_us: Some(2),
                boot_us: Some(0),
            },
            ClockFix {
                receiver_us: Some(3),
                system_us: None,
                boot_us: Some(1),
            },
            ClockFix {
                receiver_us: Some(4),
                system_us: Some(5),
                boot_us: Some(2),
            },
            ClockFix {
                receiver_us: Some(6),
                system_us: Some(6),
                boot_us: None,
            },
            ClockFix {
                receiver_us: Some(7),
                system_us: None,
                boot_us: Some(1),
            },
            ClockFix {
                receiver_us: Some(8),
                system_us: Some(8),
                boot_us: Some(1),
            },
        ],
        repair_tag: None,
    }
    .bytes()
    .unwrap();
    let file = gt_loader::load_bytes(&bytes, "clock.gtd".to_owned()).unwrap();
    let points: Vec<_> = file.tracks.iter().flat_map(|track| &track.points).collect();
    assert_eq!(
        points
            .iter()
            .map(|point| point.tpv.boot_time().map(BootTime::as_micros))
            .collect::<Vec<_>>(),
        [
            Some(100),
            None,
            Some(0),
            Some(1),
            Some(2),
            None,
            Some(1),
            Some(1)
        ]
    );
    assert_eq!(
        points
            .iter()
            .map(|point| point.tpv.system_boot_delta().map(|delta| delta.as_micros()))
            .collect::<Vec<_>>(),
        [None, None, Some(0), None, Some(1), None, None, Some(5)]
    );
    let reference = points[2].tpv.system_boot_comparison().unwrap().reference;
    for comparison in points
        .iter()
        .filter_map(|point| point.tpv.system_boot_comparison())
    {
        assert_eq!(comparison.reference, reference);
    }
    let steps: Vec<_> = points
        .iter()
        .filter_map(|point| point.tpv.boot_clock_backward_step())
        .collect();
    assert_eq!(
        steps
            .iter()
            .map(|step| (step.previous_record, step.record))
            .collect::<Vec<_>>(),
        [(0, 2), (4, 6)]
    );
    let warning = file
        .load_warnings
        .iter()
        .find(|warning| warning.issue == "fix(es) with a boot-clock backward step")
        .unwrap();
    assert_eq!(warning.count, 2);
    assert_eq!(
        warning.description,
        "fix 1 (100us) -> fix 3 (0us), fix 5 (2us) -> fix 7 (1us). Boot readings are preserved. The reference remains the first complete system/boot pair."
    );

    let config = SegmentationConfig {
        track_layout: TrackLayoutConfig {
            track_split_gap: Duration::microseconds(2),
            ..TrackLayoutConfig::default()
        },
        ..SegmentationConfig::default()
    };
    let rebuilt = gt_track_builder::build_loaded_file(
        "clock.gtd".to_owned(),
        &points
            .iter()
            .map(|point| (*point).clone())
            .collect::<Vec<_>>(),
        &[],
        vec![],
        vec![],
        &[],
        &config,
        FileSource::GtdBytes(Arc::from(bytes)),
        FileMeta::from(&file.metadata),
        vec![],
    );
    assert!(rebuilt.tracks.len() > file.tracks.len());
    let comparisons: Vec<_> = rebuilt
        .tracks
        .iter()
        .flat_map(|track| &track.points)
        .map(|point| point.tpv.system_boot_comparison())
        .collect();
    assert_eq!(
        comparisons,
        points
            .iter()
            .map(|point| point.tpv.system_boot_comparison())
            .collect::<Vec<_>>()
    );
    let filtered: Vec<_> = rebuilt
        .tracks
        .iter()
        .flat_map(|track| &track.points)
        .skip(4)
        .collect();
    assert_eq!(filtered[0].tpv.system_boot_delta().unwrap().as_micros(), 1);
}

#[rstest]
#[case::no_boot(ClockFix { receiver_us: Some(0), system_us: Some(0), boot_us: None }, None)]
#[case::no_system(ClockFix { receiver_us: Some(0), system_us: None, boot_us: Some(0) }, None)]
#[case::single_complete(ClockFix { receiver_us: None, system_us: Some(0), boot_us: Some(0) }, Some(0))]
fn a_single_fix_has_a_comparison_only_with_both_clocks(
    #[case] fix: ClockFix,
    #[case] expected_delta_us: Option<i128>,
) {
    let bytes = ClockRecording {
        fixes: vec![fix],
        repair_tag: None,
    }
    .bytes()
    .unwrap();
    let file = gt_loader::load_bytes(&bytes, "clock.gtd".to_owned()).unwrap();
    let point = &file.tracks[0].points[0];
    assert_eq!(
        point.tpv.system_boot_delta().map(|delta| delta.as_micros()),
        expected_delta_us
    );
    assert_eq!(point.tpv.boot_time().map(BootTime::as_micros), fix.boot_us);
    assert_eq!(file.metadata.debug_time_repair_threshold, None);
}

#[test]
fn stored_order_reference_preserves_microseconds_at_large_uptime_and_backward_system_time() {
    let bytes = ClockRecording {
        fixes: vec![
            ClockFix {
                receiver_us: Some(3),
                system_us: Some(3),
                boot_us: Some(u64::MAX - 2),
            },
            ClockFix {
                receiver_us: Some(2),
                system_us: Some(2),
                boot_us: Some(u64::MAX - 1),
            },
            ClockFix {
                receiver_us: Some(2),
                system_us: Some(2),
                boot_us: Some(u64::MAX - 1),
            },
        ],
        repair_tag: None,
    }
    .bytes()
    .unwrap();
    let file = gt_loader::load_bytes(&bytes, "clock.gtd".to_owned()).unwrap();
    let points: Vec<_> = file.tracks.iter().flat_map(|track| &track.points).collect();
    assert_eq!(points.len(), 3);
    assert_eq!(
        points
            .iter()
            .map(|point| point.tpv.system_boot_delta().unwrap().as_micros())
            .collect::<Vec<_>>(),
        [0, -2, -2]
    );
    assert_eq!(
        points[1].tpv.system_boot_delta().unwrap().as_millis_f64(),
        -0.002
    );
}

#[test]
fn permanent_deletion_preserves_retained_readings_and_selects_a_new_reference() {
    let bytes = ClockRecording {
        fixes: (0..3)
            .map(|i| ClockFix {
                receiver_us: Some(i),
                system_us: Some(i * 2),
                boot_us: Some(u64::try_from(i).unwrap()),
            })
            .collect(),
        repair_tag: Some(Duration::seconds(5)),
    }
    .bytes()
    .unwrap();
    let removed = 0..1;
    let retained = gt_loader::reencode_dropping_ranges(&bytes, slice::from_ref(&removed)).unwrap();
    let decoded = NavFile::read(retained.as_slice()).unwrap();
    assert_eq!(
        decoded
            .nav_points()
            .iter()
            .map(|point| point.fix.boot_time.unwrap().as_micros())
            .collect::<Vec<_>>(),
        [1, 2]
    );
    assert_eq!(
        decoded
            .debug_time_repair_tag()
            .unwrap()
            .backward_jump_threshold(),
        Duration::seconds(5)
    );
    let file = gt_loader::load_bytes(&retained, "retained.gtd".to_owned()).unwrap();
    assert_eq!(
        file.tracks
            .iter()
            .flat_map(|track| &track.points)
            .map(|point| point.tpv.system_boot_delta().unwrap().as_micros())
            .collect::<Vec<_>>(),
        [0, 1]
    );
}

struct RepairFixture {
    tag: Option<Duration>,
    explicit: Option<Duration>,
    last_system_offset_us: i64,
}

#[rstest]
#[case::explicit_only(RepairFixture { tag: None, explicit: Some(Duration::seconds(2)), last_system_offset_us: 5_000_000 })]
#[case::tag_only(RepairFixture { tag: Some(Duration::seconds(3)), explicit: None, last_system_offset_us: 6_000_000 })]
#[case::explicit_overrides_tag(RepairFixture { tag: Some(Duration::seconds(3)), explicit: Some(Duration::seconds(2)), last_system_offset_us: 5_000_000 })]
fn effective_repair_configuration_and_shifted_system_time_are_preserved(
    #[case] RepairFixture {
        tag,
        explicit,
        last_system_offset_us,
    }: RepairFixture,
) {
    let bytes = ClockRecording {
        fixes: vec![
            ClockFix {
                receiver_us: Some(0),
                system_us: Some(0),
                boot_us: Some(100),
            },
            ClockFix {
                receiver_us: Some(3_000_000),
                system_us: Some(3_000_000),
                boot_us: Some(100),
            },
            ClockFix {
                receiver_us: Some(0),
                system_us: Some(0),
                boot_us: Some(100),
            },
        ],
        repair_tag: tag,
    }
    .bytes()
    .unwrap();
    let config = SegmentationConfig::default();
    let file = match explicit {
        Some(threshold) => {
            gt_loader::load_gtd_bytes_with_debug_time_repair_progress(
                &bytes,
                "repaired.gtd".to_owned(),
                |_, _| {},
                &config,
                DebugTimeRepairConfig {
                    backward_jump_threshold: threshold,
                },
            )
            .unwrap()
            .file
        }
        None => gt_loader::load_bytes(&bytes, "repaired.gtd".to_owned()).unwrap(),
    };
    let effective = explicit.or(tag);
    assert_eq!(file.metadata.debug_time_repair_threshold, effective);
    let points: Vec<_> = file.tracks.iter().flat_map(|track| &track.points).collect();
    assert!(
        points
            .iter()
            .all(|point| point.tpv.boot_time().unwrap().as_micros() == 100)
    );
    assert!(
        points
            .iter()
            .all(|point| point.tpv.debug_time_repair_threshold() == effective)
    );
    let last = points.last().unwrap().tpv;
    assert_eq!(
        last.sys_time().unwrap().utc().timestamp_micros(),
        UTC_EPOCH_US + last_system_offset_us
    );
    assert_eq!(
        last.system_boot_delta().unwrap().as_micros(),
        i128::from(last_system_offset_us)
    );
    let recovered = FileMeta::from(&file.metadata);
    assert_eq!(recovered.debug_time_repair_threshold, effective);
}

const UTC_EPOCH_US: i64 = 1_700_000_000_000_000;
