use std::fmt::Write as _;
use std::io::{BufReader, Cursor};
use std::iter;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use geotrace_sdk::{
    Constellation, ConstellationSet, NavFileBuilder, NavFixTime, NmeaAssociationPolicy,
    NmeaCompletion, NmeaDiagnosticKind, NmeaIngestor, NmeaRecord, SatelliteCoverage,
    SatelliteMembership, SatelliteScope, SentenceType, SolutionDimension, SolutionMethod,
    SolutionState,
};
use rstest::rstest;

use super::{GGA, GSA, RMC};

#[rstest]
#[case(false)]
#[case(true)]
fn repeated_date_outliers_require_capture_evidence_before_retired_segment_attribution(
    #[case] delayed_capture: bool,
) {
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    let mut ingestor = NmeaIngestor::builder()
        .navigation_window(Duration::from_secs(5))
        .build()
        .expect("configuration");
    let first_outlier = RMC
        .replace("231026", "241026")
        .replace("120000.000", "121000.000");
    let mut records = ingestor.push_line_at(&super::sentence(&first_outlier), host);
    for second in 1..=4 {
        let body = RMC.replace("120000.000", &format!("1200{second:02}.000"));
        records.extend(
            ingestor.push_line_at(&super::sentence(&body), host + TimeDelta::seconds(second)),
        );
    }
    let (receiver_time, capture) = if delayed_capture {
        ("121000.500", host + TimeDelta::milliseconds(500))
    } else {
        ("121005.000", host + TimeDelta::seconds(5))
    };
    let repeated_outlier = RMC
        .replace("231026", "241026")
        .replace("120000.000", receiver_time);
    records.extend(ingestor.push_line_at(&super::sentence(&repeated_outlier), capture));
    let position = GGA
        .replace("120000.000", "120005.000")
        .replace("5500.000", "5600.000");
    records
        .extend(ingestor.push_line_at(&super::sentence(&position), host + TimeDelta::seconds(5)));
    records.extend(ingestor.push_line_at(
        &super::sentence(&RMC.replace("120000.000", "120006.000")),
        host + TimeDelta::seconds(6),
    ));
    records.extend(ingestor.finish().records);
    let fixes: Vec<_> = records
        .iter()
        .filter_map(|record| {
            if let NmeaRecord::Navigation(fix) = record {
                (fix.lat.as_degrees() > 55.5).then_some(fix)
            } else {
                None
            }
        })
        .collect();
    let [fix] = fixes.as_slice() else {
        panic!("one independently captured position: {fixes:?}");
    };
    let capture = host + TimeDelta::seconds(5);
    let expected = if delayed_capture {
        NavFixTime::Both {
            gps: capture,
            sys: capture,
        }
    } else {
        NavFixTime::Host(capture)
    };
    assert_eq!(fix.time, expected);
}

#[test]
fn dated_boundary_is_processed_before_host_age_expires_inferred_epochs() {
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    let mut ingestor = NmeaIngestor::builder()
        .navigation_window(Duration::from_secs(5))
        .build()
        .expect("configuration");
    let mut records = ingestor.push_line_at(&super::sentence(RMC), host);
    let position = GGA
        .replace("120000.000", "120001.000")
        .replace("5500.000", "5600.000");
    records
        .extend(ingestor.push_line_at(&super::sentence(&position), host + TimeDelta::seconds(1)));
    let date_change = RMC
        .replace("231026", "221026")
        .replace("120000.000", "120006.000");
    records.extend(ingestor.push_line_at(
        &super::sentence(&date_change),
        host + TimeDelta::milliseconds(6_100),
    ));
    records.extend(ingestor.finish().records);
    let fix = records
        .iter()
        .find_map(|record| {
            if let NmeaRecord::Navigation(fix) = record {
                (fix.lat.as_degrees() > 55.5).then_some(fix)
            } else {
                None
            }
        })
        .expect("source position");
    assert_eq!(fix.time, NavFixTime::Host(host + TimeDelta::seconds(1)));
}

#[test]
fn delayed_time_of_day_after_a_date_restart_retains_only_its_host_clock() {
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    let mut ingestor = NmeaIngestor::builder()
        .navigation_window(Duration::from_secs(5))
        .build()
        .expect("configuration");
    let mut records = ingestor.push_line_at(&super::sentence(RMC), host);
    for second in [4, 5] {
        let body = RMC
            .replace("231026", "221026")
            .replace("120000.000", &format!("1200{second:02}.000"));
        records.extend(
            ingestor.push_line_at(&super::sentence(&body), host + TimeDelta::seconds(second)),
        );
    }
    let delayed = GGA
        .replace("120000.000", "120002.000")
        .replace("5500.000", "5600.000");
    records.extend(ingestor.push_line_at(&super::sentence(&delayed), host + TimeDelta::seconds(2)));
    records.extend(ingestor.finish().records);
    let fix = records
        .iter()
        .find_map(|record| {
            if let NmeaRecord::Navigation(fix) = record {
                (fix.lat.as_degrees() > 55.5).then_some(fix)
            } else {
                None
            }
        })
        .expect("delayed source position");
    assert_eq!(fix.time, NavFixTime::Host(host + TimeDelta::seconds(2)));
}

#[test]
fn delayed_date_evidence_after_an_initial_wrong_date_prevents_further_inference() {
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    let mut ingestor = NmeaIngestor::builder()
        .navigation_window(Duration::from_secs(5))
        .build()
        .expect("configuration");
    let mut records =
        ingestor.push_line_at(&super::sentence(&RMC.replace("231026", "241026")), host);
    let later = GGA
        .replace("120000.000", "120004.000")
        .replace("5500.000", "5600.000");
    records.extend(ingestor.push_line_at(&super::sentence(&later), host + TimeDelta::seconds(4)));
    records.extend(ingestor.push_line_at(
        &super::sentence(&RMC.replace("120000.000", "120003.000")),
        host + TimeDelta::seconds(3),
    ));
    records.extend(ingestor.push_line_at(&super::sentence(&later), host + TimeDelta::seconds(4)));
    records.extend(ingestor.finish().records);
    let fixes: Vec<_> = records
        .iter()
        .filter_map(|record| {
            if let NmeaRecord::Navigation(fix) = record {
                (fix.lat.as_degrees() > 55.5).then_some(fix)
            } else {
                None
            }
        })
        .collect();
    assert!(!fixes.is_empty());
    assert!(
        fixes
            .iter()
            .all(|fix| fix.time == NavFixTime::Host(host + TimeDelta::seconds(4)))
    );
}

#[rstest]
#[case(1000)]
#[case(1500)]
fn delayed_host_captures_cannot_assign_old_velocity_or_dop_to_a_new_epoch(
    #[case] delay_millis: i64,
) {
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    let mut ingestor = NmeaIngestor::builder().build().expect("config");
    let mut records = ingestor.push_line_at(&super::sentence(RMC), host);
    let next = GGA
        .replace("120000.000", "120001.000")
        .replace("5500.000", "5600.000");
    records.extend(ingestor.push_line_at(&super::sentence(&next), host + TimeDelta::seconds(1)));
    records.extend(ingestor.push_line_at(
        &super::sentence("GPVTG,180.0,T,,M,2.0,N,3.704,K,A"),
        host + TimeDelta::milliseconds(delay_millis),
    ));
    records.extend(ingestor.push_line_at(
        &super::sentence(GSA),
        host + TimeDelta::milliseconds(delay_millis),
    ));
    let mut completion = ingestor.finish();
    records.append(&mut completion.records);
    completion.records = records;
    let fixes = super::fixes(&completion);
    let fix = fixes
        .iter()
        .find(|fix| fix.lat.as_degrees() > 55.5)
        .expect("new position");
    assert_eq!(fix.speed, None);
    assert_eq!(fix.dimension, None);
    let report = completion
        .records
        .iter()
        .find_map(|record| {
            if let NmeaRecord::SatelliteUsage(report) = record {
                Some(report)
            } else {
                None
            }
        })
        .expect("independent report");
    assert_eq!(
        report.time,
        NavFixTime::Host(host + TimeDelta::milliseconds(delay_millis))
    );
}

#[test]
fn delayed_rmc_without_host_evidence_cannot_merge_across_an_overlapping_restart() {
    let mut bodies = vec![RMC.to_owned()];
    for second in 1..=10 {
        bodies.push(GGA.replace("120000.000", &format!("1200{second:02}.000")));
    }
    for second in 0..=1 {
        bodies.push(
            GGA.replace("120000.000", &format!("1200{second:02}.000"))
                .replace("5500.000", "5600.000"),
        );
    }
    bodies.push(RMC.to_owned());
    let references: Vec<_> = bodies.iter().map(String::as_str).collect();
    let completion = super::ingest_bodies(NmeaIngestor::builder(), &references);
    let restarted: Vec<_> = super::fixes(&completion)
        .into_iter()
        .filter(|fix| fix.lat.as_degrees() > 55.5)
        .collect();
    assert!(restarted.iter().all(|fix| fix.speed.is_none()));
    assert!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::UnplacedObservation)
            > 0
    );
}

#[rstest]
#[case(&[(0,0),(3,3),(1,5),(9,9)], 6)]
#[case(&[(0,0),(3,3),(50,4),(51,5),(1,1),(2,2)], 0)]
fn clock_segment_gap_counts_include_all_buffered_late_observations(
    #[case] samples: &[(i64, i64)],
    #[case] expected: u64,
) {
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    let mut ingestor = NmeaIngestor::builder()
        .navigation_window(Duration::from_secs(5))
        .sentence_period(SentenceType::RMC, Duration::from_secs(1))
        .build()
        .expect("configuration");
    for (receiver, capture) in samples {
        let body = RMC.replace("120000.000", &format!("1200{receiver:02}.000"));
        ingestor.push_line_at(&super::sentence(&body), host + TimeDelta::seconds(*capture));
    }
    let completion = ingestor.finish();
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::MissingUpdate),
        expected
    );
}

#[rstest]
#[case(1)]
#[case(2)]
#[case(5)]
#[case(10)]
#[case(20)]
fn navigation_outages_do_not_attribute_untimed_measurements_to_an_old_epoch(#[case] cycles: u32) {
    for resume_reversed in [false, true] {
        let mut bodies = vec![RMC.to_owned()];
        for _ in 0..cycles {
            bodies.extend(
                [super::GSA, super::GSV, "GPVTG,180.0,T,,M,2.0,N,3.704,K,A"].map(str::to_owned),
            );
        }
        let resumed = RMC.replace("120000.000", &format!("1200{:02}.000", cycles + 1));
        let gga = GGA.replace("120000.000", &format!("1200{:02}.000", cycles + 1));
        bodies.extend(if resume_reversed {
            [gga.clone(), super::GSA.to_owned(), resumed.clone()]
        } else {
            [resumed.clone(), super::GSA.to_owned(), gga.clone()]
        });
        let references: Vec<_> = bodies.iter().map(String::as_str).collect();
        let completion = super::ingest_bodies(NmeaIngestor::builder(), &references);
        let old = "2026-10-23T12:00:00Z"
            .parse::<DateTime<Utc>>()
            .expect("timestamp");
        assert!(!completion.records.iter().any(|record| match record {
            NmeaRecord::SatelliteUsage(report) => report.time.gps_time() == Some(old),
            NmeaRecord::SatelliteObservation(report) => report.time.gps_time() == Some(old),
            _ => false,
        }));
        assert!(
            completion
                .diagnostics
                .count(NmeaDiagnosticKind::UnplacedObservation)
                >= u64::from(cycles)
        );
        assert!(super::fixes(&completion).iter().all(|fix| {
            fix.speed
                .is_none_or(|speed| speed.as_meters_per_second() < 1.0)
        }));
    }
}

#[rstest]
#[case(1)]
#[case(2)]
#[case(5)]
#[case(10)]
#[case(20)]
fn gga_preserves_epochs_while_rmc_is_missing(#[case] cycles: u32) {
    let mut bodies = vec![RMC.to_owned()];
    for second in 1..=cycles {
        let gga = GGA.replace("120000.000", &format!("1200{second:02}.000"));
        bodies.extend([gga.clone(), super::GSA.to_owned(), gga]);
    }
    let references: Vec<_> = bodies.iter().map(String::as_str).collect();
    let completion = super::ingest_bodies(NmeaIngestor::builder(), &references);
    let base = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    let fixes = super::fixes(&completion);
    assert_eq!(fixes.len(), usize::try_from(cycles).expect("count") + 1);
    for second in 0..=cycles {
        assert!(
            fixes.iter().any(
                |fix| fix.time.gps_time() == Some(base + TimeDelta::seconds(i64::from(second)))
            )
        );
    }
    assert_eq!(completion.statistics.satellite_usage_reports, 0);
}

#[rstest]
#[case(false)]
#[case(true)]
fn isolated_future_receiver_timestamp_does_not_poison_fifty_following_epochs(
    #[case] host_clock: bool,
) {
    let base = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    let mut ingestor = NmeaIngestor::builder().build().expect("configuration");
    let mut records = Vec::new();
    for (capture, second) in [0, 50].into_iter().chain(1..=50).enumerate() {
        let line = super::sentence(&RMC.replace("120000.000", &format!("1200{second:02}.000")));
        records.extend(if host_clock {
            ingestor.push_line_at(
                &line,
                base + TimeDelta::days(5)
                    + TimeDelta::seconds(i64::try_from(capture).expect("capture")),
            )
        } else {
            ingestor.push_line(&line)
        });
    }
    let mut completion = ingestor.finish();
    records.append(&mut completion.records);
    completion.records = records;
    let fixes = super::fixes(&completion);
    for second in 0..=50 {
        assert_eq!(
            fixes
                .iter()
                .filter(|fix| fix.time.gps_time() == Some(base + TimeDelta::seconds(second)))
                .count(),
            1
        );
    }
    assert!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::TimestampDiscontinuity)
            >= 1
    );
    if host_clock {
        assert!(
            fixes.iter().any(|fix| fix.time
                == NavFixTime::Host(base + TimeDelta::days(5) + TimeDelta::seconds(1)))
        );
    }
}

#[rstest]
#[case(&[0, 50, 51], &[0, 50, 51])]
#[case(&[10, 5, 11, 12], &[10, 11, 12])]
#[case(&[50, 0, 1, 2], &[50, 0, 1, 2])]
fn coherent_gaps_and_restarts_recover_without_using_an_isolated_backward_timestamp(
    #[case] seconds: &[u32],
    #[case] expected: &[u32],
) {
    let bodies: Vec<_> = seconds
        .iter()
        .map(|second| RMC.replace("120000.000", &format!("1200{second:02}.000")))
        .collect();
    let references: Vec<_> = bodies.iter().map(String::as_str).collect();
    let completion = super::ingest_bodies(NmeaIngestor::builder(), &references);
    let base = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    let fixes = super::fixes(&completion);
    assert_eq!(fixes.len(), expected.len());
    for second in expected {
        assert!(
            fixes
                .iter()
                .any(|fix| fix.time.gps_time()
                    == Some(base + TimeDelta::seconds(i64::from(*second))))
        );
    }
    assert!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::TimestampDiscontinuity)
            > 0
    );
}

#[test]
fn a_future_date_does_not_supply_receiver_dates_to_intermediate_host_only_gga() {
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    let mut ingestor = NmeaIngestor::builder().build().expect("configuration");
    let bodies = [
        RMC.to_owned(),
        RMC.replace("231026", "241026"),
        GGA.replace("120000.000", "120001.000"),
        RMC.replace("120000.000", "120002.000"),
        GGA.replace("120000.000", "120003.000"),
    ];
    let mut recorder = NavFileBuilder::new().open();
    for (capture, body) in bodies.iter().enumerate() {
        for record in ingestor.push_line_at(
            &super::sentence(body),
            host + TimeDelta::seconds(i64::try_from(capture).expect("capture")),
        ) {
            recorder.add(record);
        }
    }
    for record in ingestor.finish().records {
        recorder.add(record);
    }
    let recording = super::roundtrip(&recorder.finish().expect("recording"));
    assert!(recording.nav_points().iter().all(|point| {
        point
            .fix
            .time
            .gps_time()
            .is_none_or(|gps| gps.date_naive() == host.date_naive())
    }));
    assert!(
        recording
            .nav_points()
            .iter()
            .any(|point| point.fix.time == NavFixTime::Host(host + TimeDelta::seconds(2)))
    );
    assert!(
        recording
            .nav_points()
            .iter()
            .any(|point| point.fix.time == NavFixTime::Host(host + TimeDelta::seconds(4)))
    );
}

#[test]
fn confirming_a_new_receiver_date_preserves_previously_unresolved_host_only_epochs() {
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    let mut ingestor = NmeaIngestor::builder().build().expect("configuration");
    let bodies = [
        RMC.to_owned(),
        RMC.replace("231026", "241026"),
        GGA.replace("120000.000", "120001.000"),
        RMC.replace("231026", "241026")
            .replace("120000.000", "120001.000"),
    ];
    let mut records = Vec::new();
    for (capture, body) in bodies.iter().enumerate() {
        records.extend(ingestor.push_line_at(
            &super::sentence(body),
            host + TimeDelta::milliseconds(i64::try_from(capture).expect("capture") * 100),
        ));
    }
    let mut completion = ingestor.finish();
    records.append(&mut completion.records);
    completion.records = records;
    assert!(
        super::fixes(&completion)
            .iter()
            .any(|fix| fix.time == NavFixTime::Host(host + TimeDelta::milliseconds(200)))
    );
}

#[test]
fn a_second_discontinuity_does_not_confirm_the_first_outlying_date() {
    let future_date = RMC.replace("231026", "241026");
    let fifty = RMC.replace("120000.000", "120050.000");
    let fifty_one = RMC.replace("120000.000", "120051.000");
    let completion = super::ingest_bodies(
        NmeaIngestor::builder(),
        &[RMC, &future_date, &fifty, &fifty_one],
    );
    let fixes = super::fixes(&completion);
    assert_eq!(fixes.len(), 3);
    assert!(fixes.iter().all(|fix| {
        fix.time.effective().date_naive()
            == "2026-10-23T12:00:00Z"
                .parse::<DateTime<Utc>>()
                .expect("timestamp")
                .date_naive()
    }));
}

#[test]
fn receiver_time_of_day_requires_a_new_date_after_a_clock_jump() {
    let gga50 = GGA.replace("120000.000", "120050.000");
    let gga51 = GGA.replace("120000.000", "120051.000");
    let gga52 = GGA.replace("120000.000", "120052.000");
    let completion = super::ingest_bodies(NmeaIngestor::builder(), &[RMC, &gga50, &gga51, &gga52]);
    assert_eq!(super::fixes(&completion).len(), 1);
    assert!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::TimestampDiscontinuity)
            > 0
    );
}

#[test]
fn coarse_host_timestamps_do_not_identify_gsv_update_cycles() {
    let completion = super::ingest_captured_bodies(
        NmeaIngestor::builder(),
        &[
            RMC,
            "GPGSV,2,1,02,01,10,100,20",
            super::GSA,
            "GPGSV,2,2,02,02,60,200,50",
            RMC,
        ],
    );
    let reports: Vec<_> = completion
        .records
        .iter()
        .filter_map(|record| match record {
            NmeaRecord::SatelliteObservation(report) => Some(report),
            _ => None,
        })
        .collect();
    assert_eq!(reports.len(), 2);
    assert!(reports.iter().all(
        |report| report.coverage == SatelliteCoverage::Partial && report.satellites.len() == 1
    ));
    assert_eq!(
        reports
            .first()
            .expect("first")
            .satellites
            .first()
            .expect("satellite")
            .snr,
        Some(20.0)
    );
    assert_eq!(
        reports
            .last()
            .expect("second")
            .satellites
            .first()
            .expect("satellite")
            .snr,
        Some(50.0)
    );
}

#[rstest]
#[case(599, false)]
#[case(600, false)]
#[case(601, true)]
fn gsv_assembly_timeout_is_independent_of_the_update_period(
    #[case] millis: i64,
    #[case] expired: bool,
) {
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    let mut ingestor = NmeaIngestor::builder()
        .sentence_period(SentenceType::GSV, Duration::from_secs(1))
        .gsv_assembly_timeout(Duration::from_millis(600))
        .build()
        .expect("configuration");
    ingestor.push_line_at(&super::sentence(RMC), host);
    ingestor.push_line_at(&super::sentence("GPGSV,2,1,02,01,10,100,20"), host);
    ingestor.push_line_at(
        &super::sentence("GPGSV,2,2,02,02,60,200,50"),
        host + TimeDelta::milliseconds(millis),
    );
    let completion = ingestor.finish();
    assert_eq!(completion.statistics.satellite_observation_reports, 2);
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::IncompleteGsv)
            > 0,
        expired
    );
    assert!(
        completion
            .records
            .iter()
            .filter_map(|record| match record {
                NmeaRecord::SatelliteObservation(report) => Some(report),
                _ => None,
            })
            .all(|report| report.coverage == SatelliteCoverage::Partial)
    );
}

#[rstest]
#[case(GGA.replace(",4,08,", ",0,08,"), RMC.to_owned())]
#[case(GGA.to_owned(), RMC.replace(",A,5500", ",V,5500"))]
#[case("GNGNS,120000.000,5500.000,N,01200.000,E,N,08,0.7,0.0,0.0,,".to_owned(), RMC.to_owned())]
fn contradictory_navigation_validity_is_order_independent_and_unresolved(
    #[case] left: String,
    #[case] right: String,
) {
    for reversed in [false, true] {
        let bodies = if reversed {
            [&right, &left]
        } else {
            [&left, &right]
        };
        let completion = super::ingest_bodies(NmeaIngestor::builder(), &bodies.map(String::as_str));
        let [fix] = super::fixes(&completion).try_into().expect("one fix");
        assert_eq!(fix.state, Some(SolutionState::Unknown));
        assert_eq!(fix.method, None);
        assert_eq!(fix.dimension, None);
        assert!(
            completion
                .diagnostics
                .count(NmeaDiagnosticKind::ConflictingNavigation)
                > 0
        );
    }
}

#[test]
fn consistent_complementary_validity_preserves_rtk_method() {
    let completion = super::ingest_bodies(NmeaIngestor::builder(), &[GGA, RMC]);
    let [fix] = super::fixes(&completion).try_into().expect("one fix");
    assert_eq!(fix.state, Some(SolutionState::Fix));
    assert_eq!(fix.method, Some(SolutionMethod::RtkFixed));
}

#[test]
fn repeated_constellation_usage_does_not_establish_exhaustive_membership() {
    let alternate = super::GSA.replace(",01,02,", ",03,04,");
    let completion =
        super::ingest_captured_bodies(NmeaIngestor::builder(), &[RMC, super::GSA, &alternate, RMC]);
    let mut recorder = NavFileBuilder::new().open();
    for record in completion.records {
        recorder.add(record);
    }
    let recording = super::roundtrip(&recorder.finish().expect("recording"));
    let point = recording.nav_points().first().expect("point");
    assert_eq!(point.satellite_evidence().usage.len(), 2);
    assert!(point.satellite_evidence().usage.iter().all(|report| {
        report.coverage == SatelliteCoverage::Partial
            && report.reported_count.is_none()
            && report
                .satellites
                .iter()
                .all(|satellite| satellite.membership == SatelliteMembership::Used)
    }));
}

#[rstest]
#[case("GNGSA", Some(5), Some(Constellation::Qzss), None)]
#[case("GNGSA", Some(6), Some(Constellation::Navic), None)]
#[case("GQGSA", None, Some(Constellation::Qzss), None)]
#[case("QZGSA", None, Some(Constellation::Qzss), None)]
#[case("QZGSA", Some(5), Some(Constellation::Qzss), None)]
#[case(
    "QZGSA",
    Some(1),
    None,
    Some(NmeaDiagnosticKind::UnresolvedSatelliteIdentity)
)]
#[case("GIGSA", None, Some(Constellation::Navic), None)]
#[case(
    "GPGSA",
    Some(5),
    None,
    Some(NmeaDiagnosticKind::UnresolvedSatelliteIdentity)
)]
#[case("GPGSA", Some(7), None, Some(NmeaDiagnosticKind::MalformedSentence))]
#[case(
    "GNGSA",
    None,
    None,
    Some(NmeaDiagnosticKind::UnresolvedSatelliteIdentity)
)]
fn gsa_system_identity_preserves_explicit_unknown_and_conflicting_evidence(
    #[case] header: &str,
    #[case] system_id: Option<u8>,
    #[case] expected: Option<Constellation>,
    #[case] diagnostic: Option<NmeaDiagnosticKind>,
) {
    let mut body = super::GSA.replacen("GPGSA", header, 1);
    if let Some(id) = system_id {
        write!(&mut body, ",{id}").expect("write System ID");
    }
    let completion = super::ingest_captured_bodies(NmeaIngestor::builder(), &[RMC, &body, RMC]);
    let reports: Vec<_> = completion
        .records
        .iter()
        .filter_map(|record| match record {
            NmeaRecord::SatelliteUsage(report) => Some(report),
            _ => None,
        })
        .collect();
    if let Some(expected) = expected {
        let [report] = reports.as_slice() else {
            panic!("one usage report")
        };
        assert_eq!(
            report.scope,
            SatelliteScope::Constellations(ConstellationSet::new(expected, []))
        );
        assert_eq!(report.coverage, SatelliteCoverage::Partial);
    } else {
        assert_eq!(reports.len(), 0);
    }
    for kind in [
        NmeaDiagnosticKind::MalformedSentence,
        NmeaDiagnosticKind::UnresolvedSatelliteIdentity,
    ] {
        assert_eq!(
            completion.diagnostics.count(kind),
            u64::from(diagnostic == Some(kind))
        );
    }
}

#[rstest]
#[case(b"\n")]
#[case(b"\r\n")]
#[case(b"\r")]
fn reader_recovers_after_corrupt_boundaries_with_supported_terminators(#[case] separator: &[u8]) {
    for capacity in [1, 3, 64] {
        let mut bytes = Vec::new();
        let valid = super::sentence(RMC);
        let later = super::sentence(&RMC.replace("120000.000", "120001.000"));
        for body in [
            valid.as_bytes().to_vec(),
            Vec::new(),
            b"$GPRMC,truncated".to_vec(),
            [valid.as_bytes(), valid.as_bytes()].concat(),
            [valid.as_bytes(), b"garbage"].concat(),
            vec![0xff, 0xfe],
            vec![b'x'; 200],
            later.as_bytes().to_vec(),
        ] {
            bytes.extend(body);
            bytes.extend(separator);
        }
        bytes.truncate(bytes.len() - separator.len());
        let result = NmeaIngestor::builder()
            .max_line_bytes(160)
            .build()
            .expect("configuration")
            .ingest_reader(
                BufReader::with_capacity(capacity, Cursor::new(bytes)),
                NavFileBuilder::new(),
            )
            .expect("recording");
        let recording = super::roundtrip(&result.recording);
        assert_eq!(recording.nav_points().len(), 2);
        assert_eq!(
            result
                .diagnostics
                .count(NmeaDiagnosticKind::MalformedSentence),
            3
        );
        assert_eq!(result.diagnostics.count(NmeaDiagnosticKind::InvalidText), 1);
        assert_eq!(result.diagnostics.count(NmeaDiagnosticKind::LineTooLong), 1);
    }
}

#[test]
fn host_timestamps_preserve_satellite_measurements_during_a_navigation_outage() {
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    let mut ingestor = NmeaIngestor::builder().build().expect("configuration");
    let mut records = ingestor.push_line_at(&super::sentence(RMC), host);
    for second in 1..=20 {
        records.extend(ingestor.push_line_at(
            &super::sentence(super::GSA),
            host + TimeDelta::seconds(second),
        ));
    }
    let mut completion: NmeaCompletion = ingestor.finish();
    records.append(&mut completion.records);
    completion.records = records;
    assert_eq!(completion.statistics.satellite_usage_reports, 20);
    assert!(
        completion
            .records
            .iter()
            .filter_map(|record| match record {
                NmeaRecord::SatelliteUsage(report) => Some(report),
                _ => None,
            })
            .all(|report| matches!(report.time, NavFixTime::Host(_)))
    );
    assert!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::UnplacedObservation)
            > 0
    );
}

#[test]
fn host_progression_establishes_a_genuine_gap_with_an_independent_clock_offset() {
    let receiver = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    let host = receiver + TimeDelta::days(8);
    let mut ingestor = NmeaIngestor::builder()
        .sentence_period(SentenceType::RMC, Duration::from_secs(1))
        .build()
        .expect("configuration");
    let mut records = Vec::new();
    for second in [0, 50, 51] {
        let body = RMC.replace("120000.000", &format!("1200{second:02}.000"));
        records.extend(
            ingestor.push_line_at(&super::sentence(&body), host + TimeDelta::seconds(second)),
        );
    }
    let mut completion = ingestor.finish();
    records.append(&mut completion.records);
    completion.records = records;
    let fixes = super::fixes(&completion);
    assert_eq!(fixes.len(), 3);
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::MissingUpdate),
        49
    );
    for second in [0, 50, 51] {
        assert!(fixes.iter().any(|fix| fix.time
            == NavFixTime::Both {
                gps: receiver + TimeDelta::seconds(second),
                sys: host + TimeDelta::seconds(second),
            }));
    }
}

#[rstest]
#[case(1)]
#[case(2)]
#[case(5)]
#[case(10)]
#[case(20)]
fn delayed_navigation_duplicates_and_complements_do_not_establish_untimed_epoch_membership(
    #[case] cycles: u32,
) {
    for ending in [RMC, GGA] {
        let mut bodies = vec![RMC.to_owned()];
        for cycle in 0..cycles {
            bodies.extend([
                super::GSA.to_owned(),
                format!("GPGSV,1,1,01,01,30,100,{}", 20 + cycle),
                "GPVTG,180.0,T,,M,2.0,N,3.704,K,A".to_owned(),
            ]);
        }
        bodies.push(ending.to_owned());
        let references: Vec<_> = bodies.iter().map(String::as_str).collect();
        let completion = super::ingest_bodies(NmeaIngestor::builder(), &references);
        assert_eq!(completion.statistics.satellite_usage_reports, 0);
        assert_eq!(completion.statistics.satellite_observation_reports, 0);
        assert!(
            completion
                .diagnostics
                .count(NmeaDiagnosticKind::UnplacedObservation)
                >= u64::from(cycles) * 3
        );
        let [fix] = super::fixes(&completion).try_into().expect("one epoch");
        assert_eq!(fix.speed.expect("RMC speed").as_knots(), 1.0);
    }
}

#[rstest]
#[case(&[RMC, GSA, RMC])]
#[case(&[RMC, GGA, GSA, GGA])]
fn duplicate_sentences_do_not_extend_opt_in_navigation_brackets(#[case] bodies: &[&str]) {
    let completion = super::ingest_bodies(
        NmeaIngestor::builder().association_policy(NmeaAssociationPolicy::ReceiverEpochBracketing),
        bodies,
    );
    assert_eq!(completion.statistics.satellite_usage_reports, 0);
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::UnplacedObservation),
        1
    );
}

#[rstest]
#[case(false)]
#[case(true)]
fn gga_only_restarts_preserve_host_captures_without_reusing_previous_dates(
    #[case] host_clock: bool,
) {
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    let mut ingestor = NmeaIngestor::builder().build().expect("configuration");
    let bodies = iter::once(RMC.to_owned()).chain(
        (1..=10)
            .chain(0..=5)
            .map(|second| GGA.replace("120000.000", &format!("1200{second:02}.000"))),
    );
    let mut records = Vec::new();
    for (capture, body) in bodies.enumerate() {
        let line = super::sentence(&body);
        records.extend(if host_clock {
            ingestor.push_line_at(
                &line,
                host + TimeDelta::seconds(i64::try_from(capture).expect("capture")),
            )
        } else {
            ingestor.push_line(&line)
        });
    }
    let mut completion = ingestor.finish();
    records.append(&mut completion.records);
    completion.records = records;
    let fixes = super::fixes(&completion);
    assert_eq!(fixes.len(), if host_clock { 17 } else { 11 });
    if host_clock {
        assert_eq!(
            fixes
                .iter()
                .filter(|fix| matches!(fix.time, NavFixTime::Host(_)))
                .count(),
            6
        );
    }
    for second in 0..=5 {
        assert_eq!(
            fixes
                .iter()
                .filter(|fix| fix.time.gps_time() == Some(host + TimeDelta::seconds(second)))
                .count(),
            1
        );
    }
    assert!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::TimestampDiscontinuity)
            > 0
    );
}

#[test]
fn a_midnight_gap_requires_independent_evidence_before_date_attribution() {
    let before = RMC.replace("120000.000", "235959.000");
    let bodies = vec![
        before,
        GGA.replace("120000.000", "000049.000"),
        GGA.replace("120000.000", "000050.000"),
        GGA.replace("120000.000", "000051.000"),
    ];
    let references: Vec<_> = bodies.iter().map(String::as_str).collect();
    let completion = super::ingest_bodies(NmeaIngestor::builder(), &references);
    let fixes = super::fixes(&completion);
    assert_eq!(fixes.len(), 1);
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::UnplacedObservation),
        3
    );
    let recovered = RMC
        .replace("120000.000", "000052.000")
        .replace("231026", "241026");
    let mut recovered_bodies = bodies;
    recovered_bodies.push(recovered);
    let references: Vec<_> = recovered_bodies.iter().map(String::as_str).collect();
    let completion = super::ingest_bodies(NmeaIngestor::builder(), &references);
    assert!(super::fixes(&completion).iter().any(|fix| {
        fix.time.gps_time()
            == Some(
                "2026-10-24T00:00:52Z"
                    .parse::<DateTime<Utc>>()
                    .expect("date anchor"),
            )
    }));
}

#[test]
fn host_only_epochs_accept_independently_timed_velocity_and_usage_through_recording() {
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    let mut ingestor = NmeaIngestor::builder()
        .association_policy(NmeaAssociationPolicy::HostCaptureProximity)
        .build()
        .expect("configuration");
    let mut recorder = NavFileBuilder::new().open();
    for (millis, body) in [
        (0, GGA),
        (100, super::GSA),
        (200, "GPVTG,360.0,T,,M,2.0,N,3.704,K,A"),
    ] {
        for record in ingestor.push_line_at(
            &super::sentence(body),
            host + TimeDelta::milliseconds(millis),
        ) {
            recorder.add(record);
        }
    }
    for record in ingestor.finish().records {
        recorder.add(record);
    }
    let recording = super::roundtrip(&recorder.finish().expect("recording"));
    let [point] = recording.nav_points() else {
        panic!("one epoch")
    };
    assert_eq!(point.fix.time, NavFixTime::Host(host));
    assert_eq!(point.fix.speed.expect("speed").as_knots(), 2.0);
    assert_eq!(point.fix.heading.expect("heading").as_degrees(), 0.0);
    assert_eq!(point.fix.dimension, Some(SolutionDimension::ThreeD));
    assert!((point.fix.pdop.expect("PDOP") - 1.2).abs() < 0.000_001);
    assert_eq!(point.satellite_evidence().usage.len(), 1);
}

#[test]
fn an_isolated_receiver_outlier_preserves_continuous_host_schedule_evidence() {
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    let mut ingestor = NmeaIngestor::builder()
        .sentence_period(SentenceType::RMC, Duration::from_secs(1))
        .build()
        .expect("configuration");
    for (capture, second) in [0, 50, 1, 2, 3].into_iter().enumerate() {
        let body = RMC.replace("120000.000", &format!("1200{second:02}.000"));
        ingestor.push_line_at(
            &super::sentence(&body),
            host + TimeDelta::seconds(i64::try_from(capture).expect("capture")),
        );
    }
    assert_eq!(
        ingestor
            .finish()
            .diagnostics
            .count(NmeaDiagnosticKind::MissingUpdate),
        0
    );
}

#[rstest]
#[case(&[], 2)]
#[case(&[1], 1)]
#[case(&[2], 1)]
#[case(&[1, 2], 0)]
fn a_receiver_discontinuity_preserves_provisional_gaps_for_late_updates(
    #[case] late_seconds: &[i64],
    #[case] missing: u64,
) {
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    let mut ingestor = NmeaIngestor::builder()
        .navigation_window(Duration::from_secs(5))
        .sentence_period(SentenceType::RMC, Duration::from_secs(1))
        .build()
        .expect("configuration");
    for (receiver_second, host_second) in [(0, 0), (3, 3), (50, 4)] {
        let body = RMC.replace("120000.000", &format!("1200{receiver_second:02}.000"));
        ingestor.push_line_at(
            &super::sentence(&body),
            host + TimeDelta::seconds(host_second),
        );
    }
    assert_eq!(
        ingestor
            .diagnostics()
            .count(NmeaDiagnosticKind::MissingUpdate),
        0
    );
    for second in late_seconds {
        let body = RMC.replace("120000.000", &format!("1200{second:02}.000"));
        ingestor.push_line_at(&super::sentence(&body), host + TimeDelta::seconds(*second));
    }
    let body = RMC.replace("120000.000", "120004.000");
    ingestor.push_line_at(&super::sentence(&body), host + TimeDelta::seconds(5));
    assert_eq!(
        ingestor
            .finish()
            .diagnostics
            .count(NmeaDiagnosticKind::MissingUpdate),
        missing
    );
}

#[rstest]
#[case(&[])]
#[case(&[1, 2])]
fn independent_host_progress_finalizes_receiver_gaps_after_lateness_expires(
    #[case] late_seconds: &[i64],
) {
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    let mut ingestor = NmeaIngestor::builder()
        .navigation_window(Duration::from_secs(5))
        .sentence_period(SentenceType::RMC, Duration::from_secs(1))
        .build()
        .expect("configuration");
    for (receiver_second, host_second) in [(0, 0), (3, 3), (50, 4), (20, 20)] {
        let body = RMC.replace("120000.000", &format!("1200{receiver_second:02}.000"));
        ingestor.push_line_at(
            &super::sentence(&body),
            host + TimeDelta::seconds(host_second),
        );
    }
    assert_eq!(
        ingestor
            .diagnostics()
            .count(NmeaDiagnosticKind::MissingUpdate),
        2
    );
    for second in late_seconds {
        let body = RMC.replace("120000.000", &format!("1200{second:02}.000"));
        ingestor.push_line_at(&super::sentence(&body), host + TimeDelta::seconds(*second));
    }
    assert_eq!(
        ingestor
            .finish()
            .diagnostics
            .count(NmeaDiagnosticKind::MissingUpdate),
        17
    );
}

#[test]
fn old_host_captures_do_not_replace_navigation_from_a_restarted_receiver_clock() {
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    let mut ingestor = NmeaIngestor::builder().build().expect("configuration");
    let mut records = ingestor.push_line_at(&super::sentence(RMC), host);
    for second in 1..=10 {
        let body = GGA.replace("120000.000", &format!("1200{second:02}.000"));
        records.extend(
            ingestor.push_line_at(&super::sentence(&body), host + TimeDelta::seconds(second)),
        );
    }
    for second in 0..=2 {
        let body = GGA
            .replace("120000.000", &format!("1200{second:02}.000"))
            .replace("5500.000", "5600.000");
        records.extend(ingestor.push_line_at(
            &super::sentence(&body),
            host + TimeDelta::seconds(11 + second),
        ));
    }
    let old = GGA.replace("120000.000", "120002.000");
    records.extend(ingestor.push_line_at(&super::sentence(&old), host + TimeDelta::seconds(2)));
    let mut completion = ingestor.finish();
    records.append(&mut completion.records);
    completion.records = records;
    let fixes = super::fixes(&completion);
    let fix = fixes
        .iter()
        .find(|fix| fix.time == NavFixTime::Host(host + TimeDelta::seconds(13)))
        .expect("restarted receiver epoch");
    assert_eq!(fix.lat.as_degrees(), 56.0);
}

#[test]
fn a_backward_date_restart_preserves_missing_counts_from_the_new_clock_segment() {
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    let mut ingestor = NmeaIngestor::builder()
        .sentence_period(SentenceType::RMC, Duration::from_secs(1))
        .build()
        .expect("configuration");
    for (receiver_second, host_second, date) in [
        (0, 0, "241026"),
        (1, 1, "241026"),
        (0, 2, "231026"),
        (1, 3, "231026"),
        (4, 6, "231026"),
        (5, 7, "231026"),
        (8, 10, "231026"),
        (9, 11, "231026"),
    ] {
        let body = RMC
            .replace("120000.000", &format!("1200{receiver_second:02}.000"))
            .replace("231026", date);
        ingestor.push_line_at(
            &super::sentence(&body),
            host + TimeDelta::seconds(host_second),
        );
    }
    assert_eq!(
        ingestor
            .finish()
            .diagnostics
            .count(NmeaDiagnosticKind::MissingUpdate),
        4
    );
}

#[rstest]
#[case("GQ", 5, Constellation::Qzss, 193)]
#[case("GI", 6, Constellation::Navic, 1)]
#[case("GL", 2, Constellation::Glonass, 65)]
fn gsa_and_gsv_preserve_the_same_upstream_satellite_identity(
    #[case] talker: &str,
    #[case] system_id: u8,
    #[case] constellation: Constellation,
    #[case] prn: u32,
) {
    let gsa = format!("GNGSA,A,3,{prn},,,,,,,,,,,,1.2,0.7,0.9,{system_id}");
    let gsv = format!("{talker}GSV,1,1,01,{prn},30,100,40");
    let completion = super::ingest_captured_bodies(NmeaIngestor::builder(), &[RMC, &gsa, &gsv]);
    let usage = completion
        .records
        .iter()
        .find_map(|record| match record {
            NmeaRecord::SatelliteUsage(report) => report.satellites.first(),
            _ => None,
        })
        .expect("usage");
    let observation = completion
        .records
        .iter()
        .find_map(|record| match record {
            NmeaRecord::SatelliteObservation(report) => report.satellites.first(),
            _ => None,
        })
        .expect("observation");
    assert_eq!(usage.identity, observation.identity);
    assert_eq!(usage.identity.constellation, constellation);
    assert_eq!(usage.identity.prn, prn);
}

#[test]
fn no_fix_gsa_identities_do_not_establish_used_membership() {
    let no_fix = super::GSA.replacen(",A,3,", ",A,1,", 1);
    let completion = super::ingest_captured_bodies(
        NmeaIngestor::builder().association_policy(NmeaAssociationPolicy::HostCaptureProximity),
        &[RMC, &no_fix],
    );
    assert!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::ConflictingNavigation)
            > 0
    );
    let report = completion
        .records
        .iter()
        .find_map(|record| match record {
            NmeaRecord::SatelliteUsage(report) => Some(report),
            _ => None,
        })
        .expect("usage");
    assert_eq!(report.coverage, SatelliteCoverage::Partial);
    assert!(
        report
            .satellites
            .iter()
            .all(|satellite| satellite.membership == SatelliteMembership::Unknown)
    );
    let [fix] = super::fixes(&completion).try_into().expect("one fix");
    assert_eq!(fix.state, Some(SolutionState::Unknown));
    assert_eq!(fix.method, None);
}

#[rstest]
#[case("9000.0001", "N", "01200.000", "E")]
#[case("5560.000", "N", "01200.000", "E")]
#[case("-5500.000", "N", "01200.000", "E")]
#[case("5500.000", "N", "18000.0001", "E")]
#[case("5500.000", "N", "01260.000", "E")]
fn invalid_upstream_coordinates_are_recoverable_without_producing_measurements(
    #[case] latitude: &str,
    #[case] north_south: &str,
    #[case] longitude: &str,
    #[case] east_west: &str,
) {
    let invalid = format!(
        "GPRMC,120001.000,A,{latitude},{north_south},{longitude},{east_west},1.0,90.0,231026,,,A"
    );
    let later = RMC.replace("120000.000", "120002.000");
    let completion = super::ingest_bodies(NmeaIngestor::builder(), &[RMC, &invalid, &later]);
    assert_eq!(completion.statistics.navigation_fixes, 2);
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::MalformedSentence),
        1
    );
}

#[rstest]
#[case("120000e0", 0)]
#[case("1200+0", 0)]
#[case("120000.1e0", 100)]
fn supported_upstream_numeric_times_preserve_receiver_epochs(
    #[case] time: &str,
    #[case] elapsed_ms: i64,
) {
    let body = RMC.replace("120000.000", time);
    let completion = super::ingest_bodies(NmeaIngestor::builder(), &[&body]);
    let [NmeaRecord::Navigation(fix)] = completion.records.as_slice() else {
        panic!("one supported navigation epoch")
    };
    let receiver = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("receiver time");
    assert_eq!(
        fix.time,
        NavFixTime::Receiver(receiver + TimeDelta::milliseconds(elapsed_ms))
    );
    assert_eq!(completion.statistics.navigation_fixes, 1);
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::MalformedSentence),
        0
    );
}

#[rstest]
#[case("1200NaN")]
#[case("120000junk")]
#[case("120000.1tail")]
#[case("120060")]
fn malformed_receiver_time_cannot_record_a_fix_or_establish_a_date(#[case] time: &str) {
    let malformed = RMC.replace("120000.000", time);
    let mut ingestor = NmeaIngestor::builder().build().expect("configuration");
    assert!(ingestor.push_line(&super::sentence(&malformed)).is_empty());
    assert!(ingestor.push_line(&super::sentence(GGA)).is_empty());
    let completion = ingestor.finish();
    assert!(completion.records.is_empty());
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::MalformedSentence),
        1
    );
    assert!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::UnplacedObservation)
            > 0
    );
    let valid = RMC.replace("120000.000", "120001.000");
    let completion = super::ingest_bodies(NmeaIngestor::builder(), &[&malformed, &valid]);
    let [NmeaRecord::Navigation(fix)] = completion.records.as_slice() else {
        panic!("one surviving valid navigation epoch")
    };
    assert_eq!(
        fix.time.gps_time(),
        Some("2026-10-23T12:00:01Z".parse().expect("timestamp"))
    );
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::MalformedSentence),
        1
    );
}

#[test]
fn repeated_gsa_at_one_capture_does_not_create_a_second_usage_measurement() {
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("capture");
    let mut ingestor = NmeaIngestor::builder().build().expect("configuration");
    let mut records = ingestor.push_line_at(&super::sentence(RMC), host);
    for capture in [host, host, host + TimeDelta::seconds(1)] {
        records.extend(ingestor.push_line_at(&super::sentence(GSA), capture));
    }
    records.extend(ingestor.finish().records);
    let captures: Vec<_> = records
        .iter()
        .filter_map(|record| {
            if let NmeaRecord::SatelliteUsage(report) = record {
                Some(report.time)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        captures,
        vec![
            NavFixTime::Host(host),
            NavFixTime::Host(host + TimeDelta::seconds(1))
        ]
    );
}

#[test]
fn repeated_partial_gsv_after_expiration_preserves_one_source_observation() {
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("capture");
    let fragment = super::sentence("GPGSV,2,1,02,01,30,100,40");
    let mut ingestor = NmeaIngestor::builder().build().expect("configuration");
    let mut records = ingestor.push_line_at(&super::sentence(RMC), host);
    records.extend(ingestor.push_line_at(&fragment, host));
    records.extend(ingestor.push_line_at(
        &super::sentence(&RMC.replace("120000.000", "120009.000")),
        host + TimeDelta::seconds(9),
    ));
    records.extend(ingestor.push_line_at(&fragment, host));
    records.extend(ingestor.finish().records);
    let reports: Vec<_> = records
        .iter()
        .filter_map(|record| {
            if let NmeaRecord::SatelliteObservation(report) = record {
                Some(report)
            } else {
                None
            }
        })
        .collect();
    let [report] = reports.as_slice() else {
        panic!("one source observation")
    };
    assert_eq!(report.coverage, SatelliteCoverage::Partial);
    assert_eq!(report.time, NavFixTime::Host(host));
}

#[test]
fn delayed_gga_retransmission_after_a_clock_boundary_does_not_repeat_its_position() {
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("capture");
    let delayed = super::sentence(
        &GGA.replace("120000.000", "120002.000")
            .replace("5500.000", "5600.000"),
    );
    let capture = host + TimeDelta::milliseconds(3_001);
    let mut ingestor = NmeaIngestor::builder().build().expect("configuration");
    let mut records = ingestor.push_line_at(&super::sentence(RMC), host);
    records.extend(ingestor.push_line_at(&delayed, capture));
    for second in [4, 5] {
        let restarted = RMC
            .replace("231026", "221026")
            .replace("120000.000", &format!("1200{second:02}.000"));
        records.extend(ingestor.push_line_at(
            &super::sentence(&restarted),
            host + TimeDelta::seconds(second),
        ));
    }
    records.extend(ingestor.push_line_at(&delayed, capture));
    records.extend(ingestor.finish().records);
    assert_eq!(
        records
            .iter()
            .filter(|record| matches!(record,
        NmeaRecord::Navigation(fix) if fix.lat.as_degrees() > 55.5))
            .count(),
        1
    );
}

#[rstest]
#[case(false)]
#[case(true)]
fn expired_capture_identity_cannot_become_a_fresh_measurement(#[case] host_clock: bool) {
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("capture");
    let mut ingestor = NmeaIngestor::builder()
        .max_buffered_lines(1)
        .build()
        .expect("configuration");
    let mut records = Vec::new();
    for second in [0, 1, 0, 2] {
        let line = super::sentence(&RMC.replace("120000.000", &format!("1200{second:02}.000")));
        records.extend(if host_clock {
            ingestor.push_line_at(&line, host + TimeDelta::seconds(second))
        } else {
            ingestor.push_line(&line)
        });
    }
    let completion = ingestor.finish();
    records.extend(completion.records);
    let receivers: Vec<_> = records
        .iter()
        .filter_map(|record| {
            if let NmeaRecord::Navigation(fix) = record {
                fix.time.gps_time()
            } else {
                None
            }
        })
        .collect();
    assert_eq!(
        receivers,
        vec![
            host,
            host + TimeDelta::seconds(1),
            host + TimeDelta::seconds(2)
        ]
    );
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::UnplacedObservation),
        1
    );
}

#[test]
fn overlapping_retired_date_evidence_prevents_redating_a_time_of_day_observation() {
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("capture");
    let mut ingestor = NmeaIngestor::builder().build().expect("configuration");
    let previous = RMC
        .replace("231026", "221026")
        .replace("120000.000", "120002.000");
    let mut records =
        ingestor.push_line_at(&super::sentence(&previous), host + TimeDelta::seconds(2));
    for second in [0, 1] {
        let delayed = RMC.replace("120000.000", &format!("1200{second:02}.000"));
        records.extend(ingestor.push_line_at(
            &super::sentence(&delayed),
            host + TimeDelta::milliseconds(second * 1_000 + 1_773),
        ));
    }
    let ambiguous = GGA
        .replace("120000.000", "120002.000")
        .replace("5500.000", "5600.000");
    let capture = host + TimeDelta::milliseconds(3_773);
    records.extend(ingestor.push_line_at(&super::sentence(&ambiguous), capture));
    records.extend(ingestor.finish().records);
    let [fix] = records
        .iter()
        .filter_map(|record| {
            if let NmeaRecord::Navigation(fix) = record {
                (fix.lat.as_degrees() > 55.5).then_some(fix)
            } else {
                None
            }
        })
        .collect::<Vec<_>>()
        .try_into()
        .expect("one source position");
    assert_eq!(fix.time, NavFixTime::Host(capture));
}
