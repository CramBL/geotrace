//! What the loader makes of a `.gtd` report's satellite rows, read back through
//! the real file path: rows repeating a `(constellation, prn)` merge into the
//! one satellite every count taken from a report measures, and an SNR of ≈99
//! dB-Hz, the firmware's sentinel value for "no data", arrives as no SNR at all.

#![expect(
    clippy::expect_used,
    reason = "the fixture helper beside the tests is not covered by clippy's in-test relaxations"
)]

use geotrace_sdk::{
    Angle, Constellation as SdkConstellation, DateTime, Duration, NavFileBuilder, NavFix,
    NavFixTime, Satellite as SdkSatellite, SatelliteReport, Utc,
};
use gt_analysis::{loss_of_lock, satellite_utilization};
use gt_test_utils::GOLD_BYTES;
use gt_types::LoadedTrack;
use gt_types::satellites::{Constellation, SlipCause};
use rstest::rstest;

/// Elevation mask, in degrees.
const MASK_DEG: f32 = 15.0;

/// SNR fall that counts as a slip, in dB-Hz.
const SNR_DROP_DB: f32 = 10.0;

/// Elevation every synthetic satellite is reported at, well above the mask.
const ELEVATION_DEG: f32 = 40.0;

const AZIMUTH_DEG: f32 = 120.0;

/// The satellite each report holds two rows for.
const REPEATED_PRN: u32 = 7;

/// The satellite that stays in view after the repeated one drops out.
const REMAINING_PRN: u32 = 1;

/// The GPS satellite the gold dataset's satellite-stress track reports on two
/// rows: once with the ≈99 dB-Hz sentinel value, once with [`GOLD_MEASURED_SNR_DB`].
const GOLD_REPEATED_PRN: u32 = 1;

/// The out-of-range PRN that identifies the gold dataset's satellite-stress track.
const GOLD_STRESS_TRACK_PRN: u32 = 0;

const GOLD_MEASURED_SNR_DB: f32 = 40.0;

/// Satellite reports the gold dataset's satellite-stress track holds.
const GOLD_STRESS_TRACK_REPORTS: usize = 5;

fn base_time() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).expect("fixed timestamp is within range")
}

fn satellite_row(prn: u32, snr_db: f32, in_fix: bool) -> SdkSatellite {
    SdkSatellite::builder()
        .constellation(SdkConstellation::Gps)
        .prn(prn)
        .elevation(ELEVATION_DEG)
        .azimuth(AZIMUTH_DEG)
        .snr(snr_db)
        .in_fix(in_fix)
        .build()
}

/// Write a `.gtd` holding one fix per report, a second apart, and load it back
/// as a single track.
fn load_track_reporting(reports: Vec<Vec<SdkSatellite>>) -> LoadedTrack {
    let t0 = base_time();
    let mut recorder = NavFileBuilder::new().open();
    for (second, tracked) in (0i64..).zip(reports) {
        let time = t0 + Duration::seconds(second);
        recorder.add_nav_fix(
            NavFix::builder()
                .time(NavFixTime::Receiver(time))
                .lat(Angle::degrees(55.0))
                .lon(Angle::degrees(12.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
        recorder.add_satellite_report(
            SatelliteReport::builder()
                .time(NavFixTime::Receiver(time))
                .tracked(tracked)
                .build(),
        );
    }
    let mut bytes = Vec::new();
    recorder
        .finish()
        .expect("the fixes build a nav file")
        .write(&mut bytes)
        .expect("writing to a vector succeeds");

    let file =
        gt_loader::load_bytes(&bytes, "repeated_rows.gtd".to_owned()).expect("the file loads");
    file.tracks
        .into_iter()
        .next()
        .expect("the consecutive fixes form one track")
}

#[test]
fn a_satellite_reported_on_two_rows_slips_once_when_it_drops_out() {
    let track = load_track_reporting(vec![
        vec![
            satellite_row(REPEATED_PRN, 45.0, true),
            satellite_row(REPEATED_PRN, 30.0, true),
        ],
        vec![satellite_row(REMAINING_PRN, 40.0, true)],
        vec![satellite_row(REMAINING_PRN, 40.0, true)],
    ]);

    let events = loss_of_lock::detect_slip_events(&track.points, MASK_DEG, SNR_DROP_DB);

    let slips: Vec<(u32, SlipCause)> = events
        .iter()
        .flat_map(|(_, slips)| slips)
        .map(|slip| (slip.prn.value(), slip.cause))
        .collect();
    assert_eq!(slips, vec![(REPEATED_PRN, SlipCause::LostLock)]);
}

#[test]
fn a_satellite_reported_on_two_rows_and_in_the_fix_on_one_is_fully_utilized() {
    let epoch = || {
        vec![
            satellite_row(REPEATED_PRN, 45.0, true),
            satellite_row(REPEATED_PRN, 30.0, false),
        ]
    };
    let track = load_track_reporting(vec![epoch(), epoch(), epoch()]);

    let util = satellite_utilization::compute_util(&track.points, MASK_DEG);

    let rates: Vec<f64> = util.all.iter().map(|point| point[1]).collect();
    assert_eq!(rates, vec![100.0; 3]);
}

#[test]
fn the_gold_dataset_keeps_the_measured_snr_of_the_satellite_it_also_reports_as_a_sentinel() {
    let file =
        gt_loader::load_bytes(GOLD_BYTES, "gold.gtd".to_owned()).expect("the gold file loads");

    let stress_track_snrs: Vec<Vec<f32>> = file
        .tracks
        .iter()
        .flat_map(|track| &track.points)
        .filter_map(|point| point.satellites.as_ref())
        .filter(|satellites| {
            satellites.satellites().any(|satellite| {
                satellite.constellation() == Constellation::Gps
                    && satellite.prn() == GOLD_STRESS_TRACK_PRN
            })
        })
        .map(|satellites| {
            satellites
                .satellites()
                .filter(|satellite| {
                    satellite.constellation() == Constellation::Gps
                        && satellite.prn() == GOLD_REPEATED_PRN
                })
                .filter_map(|satellite| satellite.snr().map(|snr| snr.value()))
                .collect()
        })
        .collect();

    assert_eq!(
        stress_track_snrs,
        vec![vec![GOLD_MEASURED_SNR_DB]; GOLD_STRESS_TRACK_REPORTS]
    );
}

#[rstest]
#[case::inside_the_sentinel_band(99.4, None)]
#[case::just_outside_the_sentinel_band(98.5, Some(98.5))]
#[case::high_but_measured(60.0, Some(60.0))]
#[case::zero_is_a_measurement(0.0, Some(0.0))]
fn an_snr_within_half_a_db_of_the_sentinel_arrives_as_no_snr(
    #[case] reported_snr_db: f32,
    #[case] expected_snr_db: Option<f32>,
) {
    let epoch = || vec![satellite_row(REMAINING_PRN, reported_snr_db, true)];
    let track = load_track_reporting(vec![epoch(), epoch(), epoch()]);

    let snrs: Vec<Option<f32>> = track
        .points
        .iter()
        .filter_map(|point| point.satellites.as_ref())
        .flat_map(|satellites| satellites.satellites())
        .map(|satellite| satellite.snr().map(|snr| snr.value()))
        .collect();
    assert_eq!(snrs, vec![expected_snr_db; 3]);
}
