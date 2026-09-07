//! What the loader makes of a `.gtd` report's satellite rows, read back through
//! the real file path: rows repeating a `(constellation, prn)` merge into the
//! one satellite every count taken from a report measures, the merge is named
//! in a load warning, and an SNR of ≈99 dB-Hz, the value the firmware writes
//! for "no data", arrives as the file holds it.

#![expect(
    clippy::expect_used,
    reason = "the fixture helpers beside the tests are not covered by clippy's in-test relaxations"
)]

use geotrace_sdk::{
    Angle, Constellation as SdkConstellation, DateTime, Duration, NavFileBuilder, NavFix,
    NavFixTime, Satellite as SdkSatellite, SatelliteReport, Utc,
};
use gt_analysis::{loss_of_lock, satellite_utilization};
use gt_test_utils::GOLD_BYTES;
use gt_types::satellites::{Constellation, NO_DATA_SENTINEL_DB_HZ, Satellites, SlipCause};
use gt_types::{LoadedFile, LoadedTrack};
use rstest::rstest;

/// Elevation mask, in degrees.
const MASK_DEG: f32 = 15.0;

/// SNR fall that counts as a slip, in dB-Hz.
const SNR_DROP_DB: f32 = 10.0;

/// Elevation every synthetic satellite is reported at, well above the mask.
const ELEVATION_DEG: f32 = 40.0;

const AZIMUTH_DEG: f32 = 120.0;

/// The strongest SNR the rows of one merged satellite report.
const HIGHEST_ROW_SNR_DB: f32 = 45.0;

const AZIMUTH_JUST_WEST_OF_NORTH_DEG: f32 = 359.0;

const AZIMUTH_JUST_EAST_OF_NORTH_DEG: f32 = 2.0;

/// The issue the loader lists a merged satellite under.
const MERGED_ROWS_ISSUE: &str = "satellite(s) merged from several rows of one report";

/// The satellite each report holds two rows for.
const REPEATED_PRN: u32 = 7;

/// The satellite that stays in view after the repeated one drops out.
const REMAINING_PRN: u32 = 1;

/// The GPS satellite the gold dataset's satellite-stress track reports on two
/// rows: once with the ≈99 dB-Hz no-data value, once with
/// [`GOLD_MEASURED_SNR_DB`].
const GOLD_REPEATED_PRN: u32 = 1;

/// The out-of-range PRN that identifies the gold dataset's satellite-stress track.
const GOLD_STRESS_TRACK_PRN: u32 = 0;

const GOLD_MEASURED_SNR_DB: f32 = 40.0;

/// Satellite reports the gold dataset's satellite-stress track holds.
const GOLD_STRESS_TRACK_REPORTS: usize = 5;

fn base_time() -> DateTime<Utc> {
    DateTime::from_timestamp(1_700_000_000, 0).expect("fixed timestamp is within range")
}

/// One row of a satellite report. [`SatelliteRow::gps`] holds what the rows
/// here share, and a test writes the fields it varies over it.
#[derive(Clone, Copy)]
struct SatelliteRow {
    constellation: SdkConstellation,
    prn: u32,
    elevation_deg: Option<f32>,
    azimuth_deg: f32,
    snr_db: f32,
    in_fix: bool,
}

impl SatelliteRow {
    fn gps(prn: u32, snr_db: f32, in_fix: bool) -> Self {
        Self {
            constellation: SdkConstellation::Gps,
            prn,
            elevation_deg: Some(ELEVATION_DEG),
            azimuth_deg: AZIMUTH_DEG,
            snr_db,
            in_fix,
        }
    }
}

impl From<SatelliteRow> for SdkSatellite {
    fn from(row: SatelliteRow) -> Self {
        SdkSatellite::builder()
            .constellation(row.constellation)
            .prn(row.prn)
            .maybe_elevation(row.elevation_deg)
            .azimuth(row.azimuth_deg)
            .snr(row.snr_db)
            .in_fix(row.in_fix)
            .build()
    }
}

/// A `.gtd` recording holding one fix per report, a second apart, each fix
/// carrying the rows of its report.
fn recording_reporting(reports: Vec<Vec<SatelliteRow>>) -> Vec<u8> {
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
                .tracked(tracked.into_iter().map(SdkSatellite::from).collect())
                .build(),
        );
    }
    let mut bytes = Vec::new();
    recorder
        .finish()
        .expect("the fixes build a nav file")
        .write(&mut bytes)
        .expect("writing to a vector succeeds");
    bytes
}

fn load_reporting(reports: Vec<Vec<SatelliteRow>>) -> LoadedFile {
    gt_loader::load_bytes(
        &recording_reporting(reports),
        "repeated_rows.gtd".to_owned(),
    )
    .expect("the file loads")
}

/// The one track [`load_reporting`]'s consecutive fixes form.
fn load_track_reporting(reports: Vec<Vec<SatelliteRow>>) -> LoadedTrack {
    load_reporting(reports)
        .tracks
        .into_iter()
        .next()
        .expect("the consecutive fixes form one track")
}

/// The satellite report of the track's first fix, as the loader merged it.
fn first_report(track: &LoadedTrack) -> &Satellites {
    track
        .points
        .first()
        .and_then(|point| point.satellites.as_ref())
        .expect("the first fix carries the report written beside it")
}

fn listed_warnings(file: &LoadedFile) -> Vec<(u32, &str, &str)> {
    file.load_warnings
        .iter()
        .map(|warning| {
            (
                warning.count,
                warning.issue.as_str(),
                warning.description.as_str(),
            )
        })
        .collect()
}

#[test]
fn a_satellite_reported_on_two_rows_slips_once_when_it_drops_out() {
    let track = load_track_reporting(vec![
        vec![
            SatelliteRow::gps(REPEATED_PRN, HIGHEST_ROW_SNR_DB, true),
            SatelliteRow::gps(REPEATED_PRN, 30.0, true),
        ],
        vec![SatelliteRow::gps(REMAINING_PRN, 40.0, true)],
        vec![SatelliteRow::gps(REMAINING_PRN, 40.0, true)],
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
            SatelliteRow::gps(REPEATED_PRN, HIGHEST_ROW_SNR_DB, true),
            SatelliteRow::gps(REPEATED_PRN, 30.0, false),
        ]
    };
    let track = load_track_reporting(vec![epoch(), epoch(), epoch()]);

    let util = satellite_utilization::compute_util(&track.points, MASK_DEG);

    let rates: Vec<f64> = util.all.iter().map(|point| point[1]).collect();
    assert_eq!(rates, vec![100.0; 3]);
}

/// A satellite with the no-data value on one row and a measurement on another
/// merges to the measurement, whichever row comes first.
#[test]
fn the_gold_dataset_keeps_the_measured_snr_of_the_satellite_it_also_reports_as_no_data() {
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

/// A measured SNR, for the epoch after the no-data value.
const MEASURED_SNR_DB: f32 = 40.0;

/// The no-data value reaches the app as the file holds it, and the fall from
/// it to a measured reading at the next epoch is no signal loss.
#[test]
fn a_no_data_snr_arrives_unchanged_and_is_no_slip() {
    let track = load_track_reporting(vec![
        vec![SatelliteRow::gps(
            REMAINING_PRN,
            NO_DATA_SENTINEL_DB_HZ,
            true,
        )],
        vec![SatelliteRow::gps(REMAINING_PRN, MEASURED_SNR_DB, true)],
    ]);

    let snrs: Vec<Option<f32>> = track
        .points
        .iter()
        .filter_map(|point| point.satellites.as_ref())
        .flat_map(|satellites| satellites.satellites())
        .map(|satellite| satellite.snr().map(|snr| snr.value()))
        .collect();
    assert_eq!(
        snrs,
        vec![Some(NO_DATA_SENTINEL_DB_HZ), Some(MEASURED_SNR_DB)]
    );
    assert!(loss_of_lock::detect_slip_events(&track.points, MASK_DEG, SNR_DROP_DB).is_empty());
}

/// The merged satellite takes the highest SNR measured on its rows, the first
/// elevation and azimuth reported, and is in the fix when any row was.
#[rstest]
#[case::the_highest_snr_on_the_first_row(vec![
    SatelliteRow::gps(REPEATED_PRN, HIGHEST_ROW_SNR_DB, true),
    SatelliteRow::gps(REPEATED_PRN, 30.0, false),
])]
#[case::an_elevation_on_the_second_row_alone(vec![
    SatelliteRow { elevation_deg: None, ..SatelliteRow::gps(REPEATED_PRN, 30.0, false) },
    SatelliteRow::gps(REPEATED_PRN, HIGHEST_ROW_SNR_DB, true),
])]
fn rows_repeating_a_satellite_merge_into_one(#[case] tracked: Vec<SatelliteRow>) {
    let track = load_track_reporting(vec![tracked]);

    let merged = first_report(&track);

    assert_eq!(merged.satellite_count(), 1);
    assert_eq!(merged.fix_count(), 1);
    let satellite = merged.satellites().next().expect("the merged satellite");
    assert!(satellite.in_fix());
    assert_eq!(satellite.elevation(), Some(ELEVATION_DEG));
    assert_eq!(
        satellite.snr().map(|snr| snr.value()),
        Some(HIGHEST_ROW_SNR_DB)
    );
}

/// The azimuth of the first row survives: the rows reach the merge in the
/// order the file holds them.
#[test]
fn a_repeated_satellite_keeps_the_first_azimuth_reported() {
    let row_at = |azimuth_deg: f32| SatelliteRow {
        azimuth_deg,
        ..SatelliteRow::gps(REPEATED_PRN, HIGHEST_ROW_SNR_DB, true)
    };

    let track = load_track_reporting(vec![vec![
        row_at(AZIMUTH_JUST_WEST_OF_NORTH_DEG),
        row_at(AZIMUTH_JUST_EAST_OF_NORTH_DEG),
    ]]);

    let merged = first_report(&track);
    assert_eq!(merged.satellite_count(), 1);
    assert_eq!(
        merged.satellites().next().and_then(|s| s.azimuth()),
        Some(AZIMUTH_JUST_WEST_OF_NORTH_DEG)
    );
}

/// The merge key is the constellation and the PRN together.
#[test]
fn one_prn_in_two_constellations_stays_two_satellites() {
    let gps_row = SatelliteRow::gps(REPEATED_PRN, HIGHEST_ROW_SNR_DB, true);

    let track = load_track_reporting(vec![vec![
        gps_row,
        SatelliteRow {
            constellation: SdkConstellation::Galileo,
            ..gps_row
        },
    ]]);

    let constellations: Vec<Constellation> = first_report(&track)
        .satellites()
        .map(|satellite| satellite.constellation())
        .collect();
    assert_eq!(
        constellations,
        vec![Constellation::Gps, Constellation::Galileo]
    );
}

/// Two warnings are raised for a file that repeats a satellite: the SDK's,
/// about the file being malformed, and the loader's, about what the app made
/// of the repeated rows.
#[test]
fn satellites_merged_from_several_rows_load_with_a_warning_naming_them() {
    let file = load_reporting(vec![
        vec![
            SatelliteRow::gps(REPEATED_PRN, HIGHEST_ROW_SNR_DB, true),
            SatelliteRow::gps(REPEATED_PRN, 30.0, true),
            SatelliteRow::gps(REMAINING_PRN, 40.0, true),
        ],
        vec![SatelliteRow::gps(REMAINING_PRN, 40.0, true)],
        vec![
            SatelliteRow::gps(REPEATED_PRN, HIGHEST_ROW_SNR_DB, true),
            SatelliteRow::gps(REPEATED_PRN, 30.0, true),
            SatelliteRow::gps(REPEATED_PRN, 25.0, false),
        ],
    ]);

    assert_eq!(
        listed_warnings(&file),
        vec![
            (
                2,
                MERGED_ROWS_ISSUE,
                "record 0: G07 on 2 rows, record 2: G07 on 3 rows. Every satellite \
                 count shown is one per satellite, not one per row: the merged \
                 satellite takes the highest SNR measured on its rows, the first \
                 elevation and azimuth reported, and is in the fix when any row was."
            ),
            (
                2,
                "satellite report(s) with duplicate (constellation, PRN) pairs",
                "each satellite should appear at most once per report"
            ),
        ]
    );
}

#[test]
fn two_merged_satellites_of_one_report_are_named_in_the_order_they_first_appear() {
    let file = load_reporting(vec![
        vec![
            SatelliteRow::gps(REPEATED_PRN, HIGHEST_ROW_SNR_DB, true),
            SatelliteRow::gps(REMAINING_PRN, 40.0, true),
            SatelliteRow::gps(REMAINING_PRN, 30.0, true),
            SatelliteRow::gps(REPEATED_PRN, 30.0, true),
        ],
        vec![SatelliteRow::gps(REMAINING_PRN, 40.0, true)],
    ]);

    assert_eq!(
        listed_warnings(&file)
            .into_iter()
            .find(|(_, issue, _)| *issue == MERGED_ROWS_ISSUE)
            .map(|(count, _, description)| (
                count,
                description.split_once(". ").map(|(listed, _)| listed)
            )),
        Some((2, Some("record 0: G07 on 2 rows, record 0: G01 on 2 rows")))
    );
}

/// The SDK's own warning about the value is the only one raised: the loader
/// changes nothing about a no-data reading.
#[test]
fn a_no_data_snr_loads_with_the_sdk_warning_alone() {
    let file = load_reporting(vec![
        vec![
            SatelliteRow::gps(REMAINING_PRN, NO_DATA_SENTINEL_DB_HZ, true),
            SatelliteRow::gps(REPEATED_PRN, MEASURED_SNR_DB, true),
        ],
        vec![SatelliteRow::gps(
            REMAINING_PRN,
            NO_DATA_SENTINEL_DB_HZ,
            true,
        )],
        vec![SatelliteRow::gps(REPEATED_PRN, MEASURED_SNR_DB, true)],
    ]);

    assert_eq!(
        listed_warnings(&file),
        vec![(
            2,
            "satellite(s) with SNR ≈ 99 dB-Hz",
            "common firmware sentinel for unavailable signal strength; omit \
                    the SNR field when no measurement is available"
        )]
    );
}
