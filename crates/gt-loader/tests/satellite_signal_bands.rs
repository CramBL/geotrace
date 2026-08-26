//! What a receiver that reports one satellite once per signal band means for
//! the estimators reading the loaded track: the slip count and the utilization
//! baseline count satellites, not the signals each was tracked on.

#![expect(
    clippy::expect_used,
    reason = "the fixture helper beside the tests is not covered by clippy's in-test relaxations"
)]

use geotrace_sdk::{
    Angle, Constellation as SdkConstellation, DateTime, Duration, NavFileBuilder, NavFix,
    Satellite as SdkSatellite, SatelliteReport, Utc,
};
use gt_analysis::{loss_of_lock, satellite_utilization};
use gt_types::LoadedTrack;
use gt_types::satellites::SlipCause;

/// Elevation mask, in degrees.
const MASK_DEG: f32 = 15.0;

/// SNR fall that counts as a slip, in dB-Hz.
const SNR_DROP_DB: f32 = 10.0;

/// Elevation every synthetic satellite is reported at, well above the mask.
const ELEVATION_DEG: f32 = 40.0;

const AZIMUTH_DEG: f32 = 120.0;

/// The satellite the receiver tracks on two signal bands.
const TWO_BAND_PRN: u32 = 7;

/// The satellite that stays in view after the two-band one drops out.
const REMAINING_PRN: u32 = 1;

fn base_time() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).expect("fixed timestamp is within range")
}

fn signal_band_of(prn: u32, snr_db: f32, in_fix: bool) -> SdkSatellite {
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
                .gps_time(time)
                .lat(Angle::degrees(55.0))
                .lon(Angle::degrees(12.0))
                .heading(Angle::degrees(0.0))
                .build(),
        );
        recorder.add_satellite_report(
            SatelliteReport::builder()
                .gps_time(time)
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
        gt_loader::load_bytes(&bytes, "signal_bands.gtd".to_owned()).expect("the file loads");
    file.tracks
        .into_iter()
        .next()
        .expect("the consecutive fixes form one track")
}

/// A satellite that drops out slips once, whatever number of signal bands the
/// receiver tracked it on.
#[test]
fn a_satellite_tracked_on_two_signal_bands_slips_once_when_it_drops_out() {
    let track = load_track_reporting(vec![
        vec![
            signal_band_of(TWO_BAND_PRN, 45.0, true),
            signal_band_of(TWO_BAND_PRN, 30.0, true),
        ],
        vec![signal_band_of(REMAINING_PRN, 40.0, true)],
        vec![signal_band_of(REMAINING_PRN, 40.0, true)],
    ]);

    let events = loss_of_lock::detect_slip_events(&track.points, MASK_DEG, SNR_DROP_DB);

    let slips: Vec<(u32, SlipCause)> = events
        .iter()
        .flat_map(|(_, slips)| slips)
        .map(|slip| (slip.prn.value(), slip.cause))
        .collect();
    assert_eq!(slips, vec![(TWO_BAND_PRN, SlipCause::LostLock)]);
}

/// The receiver solves with one of the two bands it tracks a satellite on, and
/// that satellite is one satellite in view: the utilization rate reads 100 %.
#[test]
fn a_satellite_solved_on_one_of_its_two_signal_bands_is_fully_utilized() {
    let epoch = || {
        vec![
            signal_band_of(TWO_BAND_PRN, 45.0, true),
            signal_band_of(TWO_BAND_PRN, 30.0, false),
        ]
    };
    let track = load_track_reporting(vec![epoch(), epoch(), epoch()]);

    let util = satellite_utilization::compute_util(&track.points, MASK_DEG);

    let rates: Vec<f64> = util.all.iter().map(|point| point[1]).collect();
    assert_eq!(rates, vec![100.0; 3]);
}
