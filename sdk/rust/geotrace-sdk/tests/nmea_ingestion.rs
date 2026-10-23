#![cfg(feature = "nmea")]

use std::io::{BufReader, Cursor, Error as IoError, Read, Result as IoResult};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use geotrace_sdk::{
    Constellation, NavFile, NavFileBuilder, NavFix, NavFixTime, NmeaAssociationPolicy,
    NmeaCompletion, NmeaConfigError, NmeaDiagnosticKind, NmeaError, NmeaGsvCyclePolicy,
    NmeaIngestor, NmeaIngestorBuilder, NmeaRecord, PositionProvenance, SatelliteCoverage,
    SatelliteMembership, SatelliteScope, SentenceType, SolutionDimension, SolutionMethod,
    SolutionState,
};
use proptest::prelude::*;
use rstest::rstest;

#[path = "nmea_ingestion/properties.rs"]
mod properties;

#[path = "nmea_ingestion/hardening.rs"]
mod hardening;

fn sentence(body: &str) -> String {
    let checksum = body.bytes().fold(0, |checksum, byte| checksum ^ byte);
    format!("${body}*{checksum:02X}")
}

#[expect(clippy::expect_used, reason = "test setup must succeed")]
fn ingest_bodies(builder: NmeaIngestorBuilder, bodies: &[&str]) -> NmeaCompletion {
    let mut ingestor = builder.build().expect("configuration");
    let mut records = Vec::new();
    for body in bodies {
        records.extend(ingestor.push_line(&sentence(body)));
    }
    let mut completion = ingestor.finish();
    records.append(&mut completion.records);
    completion.records = records;
    completion
}

#[expect(clippy::expect_used, reason = "test setup must succeed")]
fn ingest_captured_bodies(builder: NmeaIngestorBuilder, bodies: &[&str]) -> NmeaCompletion {
    let mut ingestor = builder.build().expect("configuration");
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("capture timestamp");
    let mut records = Vec::new();
    for body in bodies {
        records.extend(ingestor.push_line_at(&sentence(body), host));
    }
    let mut completion = ingestor.finish();
    records.append(&mut completion.records);
    completion.records = records;
    completion
}

fn fixes(completion: &NmeaCompletion) -> Vec<NavFix> {
    completion
        .records
        .iter()
        .filter_map(|record| match record {
            NmeaRecord::Navigation(fix) => Some(*fix),
            _ => None,
        })
        .collect()
}

#[expect(
    clippy::expect_used,
    reason = "test recording must be writable and readable"
)]
fn roundtrip(file: &NavFile) -> NavFile {
    let mut bytes = Vec::new();
    file.write(&mut bytes).expect("write recording");
    let read = NavFile::read(Cursor::new(bytes)).expect("read recording");
    assert!(file.equals_ignoring_build_provenance(&read));
    read
}

#[rstest]
#[case(&[GGA,GSA,GSV,RMC])]
#[case(&[RMC,GSV,GGA,GSA])]
#[case(&[GSA,GSV,GGA,RMC])]
fn navigation_epoch_aggregation_preserves_specific_fields(#[case] input: &[&str]) {
    let completion = ingest_captured_bodies(
        NmeaIngestor::builder().association_policy(NmeaAssociationPolicy::HostCaptureProximity),
        input,
    );
    let [fix] = fixes(&completion).try_into().expect("one navigation fix");
    let receiver = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("test timestamp");
    assert_eq!(
        fix.time,
        NavFixTime::Both {
            gps: receiver,
            sys: receiver
        }
    );
    assert_eq!(fix.state, Some(SolutionState::Fix));
    assert_eq!(fix.method, Some(SolutionMethod::RtkFixed));
    assert_eq!(fix.dimension, Some(SolutionDimension::ThreeD));
    assert_eq!(fix.used_count, Some(8));
    assert!((fix.hdop.expect("HDOP") - 0.7).abs() < 0.000_001);
    assert_eq!(fix.eph_m, None);
    assert_eq!(fix.lat.as_degrees(), 55.0);
    assert_eq!(fix.lon.as_degrees(), 12.0);
    assert!(
        (fix.speed.expect("speed").as_meters_per_second() - 0.514_444_444_444).abs() < 0.000_001
    );
    assert_eq!(fix.heading.expect("heading").as_degrees(), 90.0);
    assert_eq!(completion.statistics.satellite_usage_reports, 1);
    assert_eq!(completion.statistics.satellite_observation_reports, 1);
}

#[test]
fn fixture_roundtrip_preserves_scoped_usage() {
    for line in INTERLEAVED
        .lines()
        .filter(|line| !line.starts_with("$PAIR"))
    {
        nmea::parse_str(line).expect("valid complete fixture");
    }
    let result = NmeaIngestor::builder()
        .association_policy(NmeaAssociationPolicy::ReceiverEpochBracketing)
        .build()
        .expect("config")
        .ingest_reader(Cursor::new(INTERLEAVED), NavFileBuilder::new())
        .expect("recording");
    assert_eq!(
        result
            .diagnostics
            .count(NmeaDiagnosticKind::UnsupportedSentence),
        1
    );
    assert_eq!(
        result
            .diagnostics
            .count(NmeaDiagnosticKind::InferredSatelliteTimestamp),
        3
    );
    let file = roundtrip(&result.recording);
    assert_eq!(file.nav_points().len(), 1);
    let point = file.nav_points().first().expect("one point");
    let evidence = point.satellite_evidence();
    assert_eq!(evidence.usage.len(), 1);
    let usage = evidence.usage.first().expect("one usage report");
    assert_eq!(usage.coverage, SatelliteCoverage::Partial);
    assert_eq!(usage.reported_count, None);
    assert_eq!(usage.satellites.len(), 2);
    assert!(
        usage
            .satellites
            .iter()
            .all(|sat| sat.membership == SatelliteMembership::Used)
    );
    assert!(
        matches!(&usage.scope,SatelliteScope::Constellations(set) if set.as_slice() == [Constellation::Gps])
    );
}

#[test]
fn fixture_roundtrip_preserves_optional_satellite_measurements() {
    let result = NmeaIngestor::builder()
        .association_policy(NmeaAssociationPolicy::ReceiverEpochBracketing)
        .build()
        .expect("config")
        .ingest_reader(Cursor::new(INTERLEAVED), NavFileBuilder::new())
        .expect("recording");
    let file = roundtrip(&result.recording);
    let point = file.nav_points().first().expect("one point");
    let reports = &point.satellite_evidence().observations;
    assert_eq!(reports.len(), 2);
    assert!(
        reports
            .iter()
            .all(|report| report.coverage == SatelliteCoverage::Partial
                && report.reported_count == Some(5))
    );
    assert_eq!(
        reports
            .iter()
            .map(|report| report.satellites.len())
            .sum::<usize>(),
        5
    );
    let partial = reports
        .iter()
        .flat_map(|report| &report.satellites)
        .find(|sat| sat.identity.prn == 5)
        .expect("satellite five");
    assert_eq!(partial.elevation, None);
    assert_eq!(partial.azimuth, None);
    assert_eq!(partial.snr, Some(0.0));
}

#[test]
fn timestamp_prefix_preserves_independent_clocks() {
    let result = NmeaIngestor::builder()
        .timestamp_prefix_utc("%Y-%m-%d %H%M%S%.f")
        .build()
        .expect("config")
        .ingest_reader(Cursor::new(PREFIXED), NavFileBuilder::new())
        .expect("recording");
    let file = roundtrip(&result.recording);
    let [point] = file.nav_points() else {
        panic!("one point");
    };
    assert_eq!(
        point.fix.time,
        NavFixTime::Both {
            gps: "2026-10-23T12:00:00Z"
                .parse::<DateTime<Utc>>()
                .expect("test timestamp"),
            sys: "2026-10-23T12:00:00Z"
                .parse::<DateTime<Utc>>()
                .expect("test timestamp")
        }
    );
    let reports = &point.satellite_evidence().observations;
    assert_eq!(reports.len(), 2);
    let mut times: Vec<_> = reports
        .iter()
        .map(|report| report.time.effective())
        .collect();
    times.sort_unstable();
    assert_eq!(
        times,
        ["2026-10-23T12:00:00.050Z", "2026-10-23T12:00:00.100Z"]
            .map(|time| time.parse::<DateTime<Utc>>().expect("capture"))
    );
    assert_eq!(
        result
            .diagnostics
            .count(NmeaDiagnosticKind::InferredSatelliteTimestamp),
        0
    );
}

#[rstest]
#[case(false)]
#[case(true)]
fn caller_timestamp_uses_host_without_inventing_receiver_date(#[case] configured: bool) {
    let builder = if configured {
        NmeaIngestor::builder().caller_timestamps()
    } else {
        NmeaIngestor::builder()
    };
    let mut ingestor = builder.build().expect("config");
    let host = "2026-10-23T12:00:00.025Z"
        .parse::<DateTime<Utc>>()
        .expect("test timestamp");
    assert!(ingestor.push_line_at(&sentence(GGA), host).is_empty());
    let completion = ingestor.finish();
    let [fix] = fixes(&completion).try_into().expect("one fix");
    assert_eq!(fix.time, NavFixTime::Host(host));
}

#[test]
fn missing_date_and_host_produce_explicit_timeline_error() {
    let error = NmeaIngestor::builder()
        .build()
        .expect("config")
        .ingest_reader(Cursor::new(sentence(GGA)), NavFileBuilder::new())
        .expect_err("no timeline");
    let NmeaError::MissingNavigationTimeline {
        diagnostics,
        statistics,
    } = error
    else {
        panic!("timeline error");
    };
    assert_eq!(statistics.navigation_fixes, 0);
    assert_eq!(
        diagnostics.count(NmeaDiagnosticKind::UnplacedObservation),
        1
    );
}

#[test]
fn satellite_only_input_with_host_is_not_discarded_silently() {
    let input = format!("2026-10-23 120000.000 {}", sentence(GSV));
    let error = NmeaIngestor::builder()
        .timestamp_prefix_utc("%Y-%m-%d %H%M%S%.f")
        .build()
        .expect("config")
        .ingest_reader(Cursor::new(input), NavFileBuilder::new())
        .expect_err("no navigation");
    assert!(
        matches!(error,NmeaError::MissingNavigationTimeline {statistics,..} if statistics.satellite_observation_reports == 1)
    );
}

#[rstest]
#[case("GPGGA,120000.000,,,,,0,00,,,M,,M,,")]
#[case("GPRMC,120000.000,V,,,,,,,231026,,,N")]
fn no_fix_without_position_survives_recording(#[case] body: &str) {
    let input = format!("2026-10-23 120000.025 {}", sentence(body));
    let result = NmeaIngestor::builder()
        .timestamp_prefix_utc("%Y-%m-%d %H%M%S%.f")
        .build()
        .expect("config")
        .ingest_reader(Cursor::new(input), NavFileBuilder::new())
        .expect("status recording");
    let file = roundtrip(&result.recording);
    let [point] = file.nav_points() else {
        panic!("one point");
    };
    assert_eq!(point.fix.state, Some(SolutionState::NoFix));
    assert!(point.fix.lat.as_degrees().is_nan());
    assert!(point.fix.lon.as_degrees().is_nan());
    assert_eq!(point.fix.position_provenance, None);
}

#[test]
fn midnight_date_association_is_independent_of_arrival_order() {
    let gga = "GPGGA,235959.500,5500.000,N,01200.000,E,1,08,0.7,12.3,M,40.0,M,,";
    let rmc = "GPRMC,000000.000,A,5500.000,N,01200.000,E,1.0,90.0,241026,,,A";
    for bodies in [[gga, rmc], [rmc, gga]] {
        let completion = ingest_bodies(NmeaIngestor::builder(), &bodies);
        let mut timestamps: Vec<_> = fixes(&completion)
            .iter()
            .map(|fix| fix.time.effective())
            .collect();
        timestamps.sort_unstable();
        assert_eq!(
            timestamps,
            vec![
                "2026-10-23T23:59:59.500Z"
                    .parse::<DateTime<Utc>>()
                    .expect("test timestamp"),
                "2026-10-24T00:00:00Z"
                    .parse::<DateTime<Utc>>()
                    .expect("test timestamp")
            ]
        );
    }
}

#[test]
fn zda_establishes_date_for_early_gga() {
    let completion = ingest_bodies(
        NmeaIngestor::builder(),
        &[GGA, "GPZDA,120000.000,23,10,2026,00,00"],
    );
    let [fix] = fixes(&completion).try_into().expect("one fix");
    assert_eq!(
        fix.time,
        NavFixTime::Receiver(
            "2026-10-23T12:00:00Z"
                .parse::<DateTime<Utc>>()
                .expect("test timestamp")
        )
    );
}

#[rstest]
#[case(
    "GPGLL,5500.000,N,01200.000,E,120000.000,A,A",
    SolutionMethod::Autonomous
)]
#[case(
    "GPGNS,120000.000,5500.000,N,01200.000,E,D,08,0.7,12.3,40.0,,,",
    SolutionMethod::Differential
)]
fn gll_and_gns_merge_with_date_anchor(#[case] body: &str, #[case] method: SolutionMethod) {
    let completion = ingest_bodies(
        NmeaIngestor::builder(),
        &[body, "GPZDA,120000.000,23,10,2026,00,00"],
    );
    let [fix] = fixes(&completion).try_into().expect("one fix");
    assert_eq!(fix.method, Some(method));
    assert_eq!(fix.state, Some(SolutionState::Fix));
    assert_eq!(
        fix.position_provenance,
        Some(PositionProvenance::SourceProvided)
    );
}

#[test]
fn recoverable_checksum_failure_preserves_line_and_pending_epoch() {
    let mut ingestor = NmeaIngestor::builder().build().expect("config");
    let mut records = ingestor.push_line(&sentence(GGA));
    records.extend(
        ingestor.push_line("$GPRMC,120000.000,A,5500.000,N,01200.000,E,1.0,90.0,231026,,,A*00"),
    );
    records.extend(ingestor.push_line(&sentence(RMC)));
    let completion = ingestor.finish();
    assert!(records.is_empty());
    assert_eq!(completion.statistics.navigation_fixes, 1);
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::MalformedSentence),
        1
    );
    let [sample] = completion.diagnostics.samples() else {
        panic!("one diagnostic");
    };
    assert_eq!(sample.line, 2);
    assert!(sample.reason.to_lowercase().contains("checksum"));
}

#[test]
fn proprietary_interleaving_does_not_expire_pending_observations() {
    let mut ingestor = NmeaIngestor::builder()
        .max_buffered_lines(2)
        .build()
        .expect("config");
    ingestor.push_line(&sentence(GGA));
    for _ in 0..10_000 {
        assert!(ingestor.push_line("$PAIR050,1").is_empty());
    }
    ingestor.push_line(&sentence(RMC));
    let completion = ingestor.finish();
    assert_eq!(completion.statistics.navigation_fixes, 1);
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::UnsupportedSentence),
        10_000
    );
    assert_eq!(completion.diagnostics.samples().len(), 32);
    assert_eq!(
        fixes(&completion).first().expect("fix").method,
        Some(SolutionMethod::RtkFixed)
    );
}

#[rstest]
#[case(&["GPGSV,2,2,02,02,,,0","GPGSV,2,1,02,01,30,100,40"],2,SatelliteCoverage::Complete)]
#[case(&["GPGSV,2,1,02,01,30,100,40"],1,SatelliteCoverage::Partial)]
fn gsv_fragment_numbers_determine_assembly(
    #[case] bodies: &[&str],
    #[case] count: usize,
    #[case] coverage: SatelliteCoverage,
) {
    let mut input = vec![RMC];
    input.extend_from_slice(bodies);
    let completion = ingest_captured_bodies(
        NmeaIngestor::builder().gsv_cycle_policy(NmeaGsvCyclePolicy::UniqueHostTimestamp),
        &input,
    );
    let observations: Vec<_> = completion
        .records
        .iter()
        .filter_map(|record| {
            if let NmeaRecord::SatelliteObservation(report) = record {
                Some(report)
            } else {
                None
            }
        })
        .collect();
    let [report] = observations.as_slice() else {
        panic!("one report");
    };
    assert_eq!(report.satellites.len(), count);
    assert_eq!(report.coverage, coverage);
}

#[test]
fn gsv_signals_and_constellations_remain_independent_and_partial() {
    let completion = ingest_captured_bodies(
        NmeaIngestor::builder().gsv_cycle_policy(NmeaGsvCyclePolicy::UniqueHostTimestamp),
        &[
            RMC,
            "GPGSV,2,1,02,01,30,100,40,1",
            "GLGSV,1,1,01,65,30,100,42,1",
            "GPGSV,1,1,01,01,30,100,45,5",
            "GPGSV,2,2,02,02,40,200,41,1",
        ],
    );
    let reports: Vec<_> = completion
        .records
        .iter()
        .filter_map(|record| {
            if let NmeaRecord::SatelliteObservation(report) = record {
                Some(report)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(reports.len(), 3);
    assert!(
        reports
            .iter()
            .all(|report| report.coverage == SatelliteCoverage::Partial
                && report.reported_count.is_none())
    );
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::SignalDimensionOmitted),
        3
    );
    assert_eq!(
        reports
            .iter()
            .flat_map(|report| &report.satellites)
            .filter(|sat| sat.identity.prn == 1)
            .count(),
        2
    );
}

#[test]
fn repeated_gsv_fragments_are_not_repeated_measurements() {
    let completion = ingest_captured_bodies(
        NmeaIngestor::builder().gsv_cycle_policy(NmeaGsvCyclePolicy::UniqueHostTimestamp),
        &[
            RMC,
            "GPGSV,2,1,02,01,30,100,40",
            "GPGSV,2,1,02,01,30,100,40",
            "GPGSV,2,2,02,02,40,200,41",
            "GPGSV,2,2,02,02,40,200,41",
        ],
    );
    assert_eq!(completion.statistics.satellite_observation_reports, 1);
}

#[rstest]
#[case("GNGSA,A,3,01,02,,,,,,,,,,,1.2,0.7,0.9,1", Some(Constellation::Gps))]
#[case("GLGSA,A,3,65,66,,,,,,,,,,,1.2,0.7,0.9", Some(Constellation::Glonass))]
#[case("GNGSA,A,3,01,02,,,,,,,,,,,1.2,0.7,0.9", None)]
#[case("GPGSA,A,3,01,02,,,,,,,,,,,1.2,0.7,0.9,2", None)]
fn gsa_scope_requires_unambiguous_constellation(
    #[case] body: &str,
    #[case] constellation: Option<Constellation>,
) {
    let completion = ingest_captured_bodies(NmeaIngestor::builder(), &[body, RMC]);
    let reports: Vec<_> = completion
        .records
        .iter()
        .filter_map(|record| {
            if let NmeaRecord::SatelliteUsage(report) = record {
                Some(report)
            } else {
                None
            }
        })
        .collect();
    if let Some(constellation) = constellation {
        let [report] = reports.as_slice() else {
            panic!("one report");
        };
        assert!(
            matches!(&report.scope,SatelliteScope::Constellations(set) if set.as_slice() == [constellation])
        );
    } else {
        assert!(reports.is_empty());
        assert_eq!(
            completion
                .diagnostics
                .count(NmeaDiagnosticKind::UnresolvedSatelliteIdentity),
            1
        );
    }
}

#[test]
fn independent_gsa_constellations_preserve_scope() {
    let completion = ingest_captured_bodies(
        NmeaIngestor::builder(),
        &[GSA, "GLGSA,A,3,65,66,,,,,,,,,,,1.2,0.7,0.9", RMC],
    );
    assert_eq!(completion.statistics.satellite_usage_reports, 2);
    assert!(
        completion
            .records
            .iter()
            .filter_map(
                |record| if let NmeaRecord::SatelliteUsage(report) = record {
                    Some(report)
                } else {
                    None
                }
            )
            .all(|report| report.scope != SatelliteScope::Global)
    );
}

#[test]
fn ambiguous_satellite_epoch_is_diagnosed_without_reassigning_measurement() {
    let completion = ingest_bodies(
        NmeaIngestor::builder(),
        &[
            RMC,
            GSA,
            "GPRMC,120001.000,A,5500.000,N,01200.000,E,1.0,90.0,231026,,,A",
        ],
    );
    assert_eq!(completion.statistics.navigation_fixes, 2);
    assert_eq!(completion.statistics.satellite_usage_reports, 0);
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::UnplacedObservation),
        1
    );
}

#[test]
fn unsupported_mixed_gngsv_variation_is_diagnosed() {
    let completion = ingest_bodies(
        NmeaIngestor::builder(),
        &[GGA, "GNGSV,1,1,01,01,30,100,40", RMC],
    );
    assert_eq!(completion.statistics.navigation_fixes, 1);
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::UnsupportedVariation),
        1
    );
    assert_eq!(completion.statistics.satellite_observation_reports, 0);
}

#[rstest]
#[case(NmeaIngestor::builder().sentence_period(SentenceType::GGA,Duration::ZERO))]
#[case(NmeaIngestor::builder().max_pending_epochs(0))]
#[case(NmeaIngestor::builder().max_pending_reports(0))]
#[case(NmeaIngestor::builder().max_gsv_groups(0))]
#[case(NmeaIngestor::builder().gsv_assembly_timeout(Duration::ZERO))]
#[case(NmeaIngestor::builder().gsv_assembly_timeout(Duration::from_secs(12 * 60 * 60)))]
#[case(NmeaIngestor::builder().max_gsv_fragments(0))]
#[case(NmeaIngestor::builder().max_line_bytes(0))]
#[case(NmeaIngestor::builder().max_buffered_lines(0))]
#[case(NmeaIngestor::builder().max_association_lines(0))]
#[case(NmeaIngestor::builder().navigation_window(Duration::ZERO))]
#[case(NmeaIngestor::builder().satellite_window(Duration::MAX))]
#[case(NmeaIngestor::builder().timestamp_prefix_utc("%Q"))]
#[case(NmeaIngestor::builder().timestamp_prefix_utc("%Y-%m-%d %H:%M:%S %#z"))]
#[case(NmeaIngestor::builder().timestamp_prefix_utc("%H%M%S"))]
fn invalid_configuration_is_rejected_at_construction(#[case] builder: NmeaIngestorBuilder) {
    builder.build().expect_err("invalid configuration");
}

#[test]
fn unsupported_sentence_period_is_explicit_configuration_error() {
    assert!(matches!(
        NmeaIngestor::builder()
            .sentence_period(SentenceType::TXT, Duration::from_secs(1))
            .build(),
        Err(NmeaConfigError::UnsupportedSentencePeriod {
            sentence: SentenceType::TXT
        })
    ));
}

#[test]
fn diagnostic_samples_can_be_disabled_without_losing_counts() {
    let mut ingestor = NmeaIngestor::builder()
        .diagnostic_samples(0)
        .build()
        .expect("config");
    for _ in 0..100 {
        ingestor.push_line("garbage");
    }
    assert_eq!(
        ingestor
            .diagnostics()
            .count(NmeaDiagnosticKind::UnsupportedSentence),
        100
    );
    assert!(ingestor.diagnostics().samples().is_empty());
}

#[test]
fn reader_bounds_lines_and_recovers_from_invalid_utf8() {
    let input = [
        vec![b'x'; 100_000],
        b"\n\xff\n".to_vec(),
        sentence(RMC).into_bytes(),
    ]
    .concat();
    let result = NmeaIngestor::builder()
        .max_line_bytes(128)
        .build()
        .expect("config")
        .ingest_reader(
            BufReader::with_capacity(7, Cursor::new(input)),
            NavFileBuilder::new(),
        )
        .expect("recording");
    assert_eq!(result.statistics.lines, 3);
    assert_eq!(result.statistics.navigation_fixes, 1);
    assert_eq!(result.diagnostics.count(NmeaDiagnosticKind::LineTooLong), 1);
    assert_eq!(result.diagnostics.count(NmeaDiagnosticKind::InvalidText), 1);
}

#[test]
fn truncated_final_line_and_incomplete_gsv_are_reported() {
    let input = [
        sentence(RMC),
        sentence("GPGSV,2,1,02,01,30,100,40"),
        sentence(GGA),
        "$GPGGA,12".to_owned(),
    ]
    .join("\n");
    let result = NmeaIngestor::builder()
        .association_policy(NmeaAssociationPolicy::ReceiverEpochBracketing)
        .build()
        .expect("config")
        .ingest_reader(Cursor::new(input), NavFileBuilder::new())
        .expect("recording");
    assert_eq!(
        result
            .diagnostics
            .count(NmeaDiagnosticKind::MalformedSentence),
        1
    );
    assert_eq!(
        result.diagnostics.count(NmeaDiagnosticKind::IncompleteGsv),
        1
    );
    assert_eq!(result.statistics.satellite_observation_reports, 1);
}

#[test]
fn reader_io_failures_include_next_line_number() {
    let source = Cursor::new(sentence(RMC).into_bytes()).chain(FailingReader);
    let error = NmeaIngestor::builder()
        .build()
        .expect("config")
        .ingest_reader(BufReader::new(source), NavFileBuilder::new())
        .expect_err("I/O failure");
    assert!(matches!(error, NmeaError::Input { line: 1, .. }));
}

struct FailingReader;
impl Read for FailingReader {
    fn read(&mut self, _buffer: &mut [u8]) -> IoResult<usize> {
        Err(IoError::other("test input failure"))
    }
}

#[test]
fn independent_update_periods_do_not_repeat_satellite_measurements() {
    let mut ingestor = NmeaIngestor::builder()
        .caller_timestamps()
        .sentence_period(SentenceType::RMC, Duration::from_secs(1))
        .sentence_period(SentenceType::VTG, Duration::from_secs(1))
        .sentence_period(SentenceType::GSA, Duration::from_secs(2))
        .sentence_period(SentenceType::GSV, Duration::from_secs(5))
        .build()
        .expect("config");
    let base = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("test timestamp");
    let mut records = Vec::new();
    for second in 0..11 {
        let host = base + TimeDelta::seconds(second);
        let rmc = RMC.replace("120000.000", &format!("1200{second:02}.000"));
        records.extend(ingestor.push_line_at(&sentence(&rmc), host));
        records.extend(ingestor.push_line_at(&sentence("GPVTG,90.0,T,,M,1.0,N,1.852,K,A"), host));
        if second % 2 == 0 {
            records.extend(ingestor.push_line_at(&sentence(GSA), host));
        }
        if second % 5 == 0 {
            records.extend(ingestor.push_line_at(&sentence(GSV), host));
        }
    }
    let completion = ingestor.finish();
    records.extend(completion.records);
    assert_eq!(completion.statistics.navigation_fixes, 11);
    assert_eq!(completion.statistics.satellite_usage_reports, 6);
    assert_eq!(completion.statistics.satellite_observation_reports, 3);
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::MissingUpdate),
        0
    );
    let mut recorder = NavFileBuilder::new().open();
    for record in records {
        recorder.add(record);
    }
    let file = roundtrip(&recorder.finish().expect("recording"));
    assert_eq!(
        file.nav_points()
            .iter()
            .map(|point| point.satellite_evidence().observations.len())
            .sum::<usize>(),
        3
    );
}

#[test]
fn missing_expected_sentences_emit_no_fabricated_records() {
    let completion = ingest_bodies(
        NmeaIngestor::builder()
            .sentence_period(SentenceType::GGA, Duration::from_secs(1))
            .sentence_period(SentenceType::GSV, Duration::from_secs(1)),
        &[
            RMC,
            "GPRMC,120004.000,A,5500.000,N,01200.000,E,1.0,90.0,231026,,,A",
        ],
    );
    assert_eq!(completion.statistics.navigation_fixes, 2);
    assert_eq!(completion.statistics.satellite_observation_reports, 0);
    assert!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::MissingUpdate)
            >= 2
    );
}

#[test]
fn epoch_capacity_flushes_incomplete_observations() {
    let completion = ingest_bodies(
        NmeaIngestor::builder().max_pending_epochs(1),
        &[
            RMC,
            "GPRMC,120001.000,A,5500.000,N,01200.000,E,1.0,90.0,231026,,,A",
        ],
    );
    assert_eq!(completion.statistics.navigation_fixes, 2);
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::BufferLimit),
        1
    );
}

#[test]
fn late_complement_does_not_create_a_second_measurement() {
    let completion = ingest_bodies(
        NmeaIngestor::builder(),
        &[
            RMC,
            "GPRMC,120004.000,A,5500.000,N,01200.000,E,1.0,90.0,231026,,,A",
            GGA,
        ],
    );
    assert_eq!(completion.statistics.navigation_fixes, 2);
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::TimestampDiscontinuity),
        1
    );
}

#[test]
fn large_incremental_input_releases_temporal_buffers() {
    let mut ingestor = NmeaIngestor::builder()
        .max_pending_epochs(4)
        .build()
        .expect("config");
    let base = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("test timestamp");
    let mut emitted = 0;
    for offset in 0..20_000 {
        let time = base + TimeDelta::seconds(offset);
        let body = format!(
            "GPRMC,{}.000,A,5500.000,N,01200.000,E,1.0,90.0,{},,,A",
            time.format("%H%M%S"),
            time.format("%d%m%y")
        );
        emitted += ingestor.push_line(&sentence(&body)).len();
    }
    assert!(emitted >= 19_996);
    let completion = ingestor.finish();
    assert_eq!(completion.statistics.navigation_fixes, 20_000);
    assert!(completion.records.len() <= 4);
}

proptest! {
    #[test]
    fn arbitrary_text_is_recoverable(lines in prop::collection::vec(".{0,180}",0..50)) {
        let mut ingestor = NmeaIngestor::builder().max_pending_epochs(4).max_pending_reports(4).max_gsv_groups(4).diagnostic_samples(4).build().expect("config");
        for line in &lines { ingestor.push_line(line); }
        let completion = ingestor.finish();
        prop_assert_eq!(completion.statistics.lines,lines.len() as u64);
        prop_assert!(completion.diagnostics.samples().len() <= 4);
    }

    #[test]
    fn arbitrary_supported_sentence_payload_is_recoverable(header in prop::sample::select(vec!["GPGGA","GPRMC","GPGNS","GPGLL","GPGSA","GPGSV","GPVTG","GPZDA"]),payload in "[A-Za-z0-9,.-]{0,90}") {
        let mut ingestor = NmeaIngestor::builder().max_gsv_fragments(4).build().expect("config");
        ingestor.push_line(&sentence(&format!("{header},{payload}")));
        ingestor.finish();
    }
}

#[rstest]
#[case(
    "GPGGA,120000.000,5500.000,N,01200.000,E,1,08,0.7,12.3,M,40.0,M,,",
    SolutionState::Fix,
    SolutionMethod::Autonomous
)]
#[case(
    "GPGGA,120000.000,5500.000,N,01200.000,E,2,08,0.7,12.3,M,40.0,M,,",
    SolutionState::Fix,
    SolutionMethod::Differential
)]
#[case(
    "GPGGA,120000.000,5500.000,N,01200.000,E,5,08,0.7,12.3,M,40.0,M,,",
    SolutionState::Fix,
    SolutionMethod::RtkFloat
)]
#[case(
    "GPGGA,120000.000,5500.000,N,01200.000,E,6,08,0.7,12.3,M,40.0,M,,",
    SolutionState::EstimatedOnly,
    SolutionMethod::DeadReckoning
)]
#[case(
    "GPGGA,120000.000,5500.000,N,01200.000,E,7,08,0.7,12.3,M,40.0,M,,",
    SolutionState::EstimatedOnly,
    SolutionMethod::Other
)]
fn source_state_and_method_are_independent(
    #[case] body: &str,
    #[case] state: SolutionState,
    #[case] method: SolutionMethod,
) {
    let completion = ingest_bodies(
        NmeaIngestor::builder(),
        &[body, "GPZDA,120000.000,23,10,2026,00,00"],
    );
    let fix = fixes(&completion).into_iter().next().expect("one fix");
    assert_eq!(fix.state, Some(state));
    assert_eq!(fix.method, Some(method));
    assert_eq!(fix.dimension, None);
}

#[test]
fn vtg_complements_an_unambiguous_navigation_epoch() {
    let completion = ingest_bodies(
        NmeaIngestor::builder().association_policy(NmeaAssociationPolicy::ReceiverEpochBracketing),
        &[
            GGA,
            "GPVTG,360.0,T,,M,2.0,N,3.704,K,A",
            "GPZDA,120000.000,23,10,2026,00,00",
            "GPGLL,5500.000,N,01200.000,E,120000.000,A,A",
        ],
    );
    let fix = fixes(&completion).into_iter().next().expect("one fix");
    assert_eq!(fix.heading.expect("heading").as_degrees(), 0.0);
    assert!((fix.speed.expect("speed").as_knots() - 2.0).abs() < 0.000_001);
}

#[test]
fn host_capture_order_does_not_determine_navigation_merge() {
    let host = "2026-10-23T12:00:01Z"
        .parse::<DateTime<Utc>>()
        .expect("test timestamp");
    let earlier = host - TimeDelta::milliseconds(100);
    let mut outputs = Vec::new();
    for observations in [[(GGA, earlier), (RMC, host)], [(RMC, host), (GGA, earlier)]] {
        let mut ingestor = NmeaIngestor::builder().build().expect("config");
        for (body, time) in observations {
            ingestor.push_line_at(&sentence(body), time);
        }
        outputs.push(fixes(&ingestor.finish()));
    }
    assert_eq!(outputs.first(), outputs.last());
    assert_eq!(
        outputs.first().expect("result").first().expect("fix").time,
        NavFixTime::Both {
            gps: "2026-10-23T12:00:00Z"
                .parse::<DateTime<Utc>>()
                .expect("test timestamp"),
            sys: earlier
        }
    );
}

#[test]
fn old_host_only_gga_is_not_reassigned_to_a_later_date() {
    let mut ingestor = NmeaIngestor::builder().build().expect("config");
    let old = "2026-10-22T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("test timestamp");
    let current = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("test timestamp");
    let mut records = ingestor.push_line_at(&sentence(GGA), old);
    records.extend(ingestor.push_line_at(&sentence(RMC), current));
    records.extend(ingestor.finish().records);
    let fixes: Vec<_> = records
        .into_iter()
        .filter_map(|record| {
            if let NmeaRecord::Navigation(fix) = record {
                Some(fix)
            } else {
                None
            }
        })
        .collect();
    assert_eq!(fixes.len(), 2);
    assert!(fixes.iter().any(|fix| fix.time == NavFixTime::Host(old)));
    assert!(fixes.iter().any(|fix| fix.time
        == NavFixTime::Both {
            gps: current,
            sys: current
        }));
}

#[rstest]
#[case("2026-10-23 120000.000", "%Y-%m-%d %H%M%S%.f")]
#[case("2026-10-23T12:00:00.000+00:00", "%+")]
#[case("2026-10-23 12:00:00 +0000", "%Y-%m-%d %H:%M:%S %z")]
fn utc_prefix_formats_preserve_actual_timestamp(#[case] prefix: &str, #[case] format: &str) {
    let mut ingestor = NmeaIngestor::builder()
        .timestamp_prefix_utc(format)
        .build()
        .expect("format");
    ingestor.push_line(&format!("{prefix} {}", sentence(GGA)));
    let completion = ingestor.finish();
    assert_eq!(
        fixes(&completion).first().expect("fix").time,
        NavFixTime::Host(
            "2026-10-23T12:00:00Z"
                .parse::<DateTime<Utc>>()
                .expect("test timestamp")
        )
    );
}

#[test]
fn malformed_or_non_utc_prefixes_are_recoverable() {
    let mut ingestor = NmeaIngestor::builder()
        .timestamp_prefix_utc("%+")
        .build()
        .expect("format");
    ingestor.push_line(&format!("2026-10-23T12:00:00+02:00 {}", sentence(GGA)));
    ingestor.push_line(&format!("bad-prefix {}", sentence(GGA)));
    ingestor.push_line(&format!("2026-10-23T12:00:00Z {}", sentence(RMC)));
    let completion = ingestor.finish();
    assert_eq!(completion.statistics.navigation_fixes, 1);
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::InvalidHostTimestamp),
        2
    );
}

#[test]
fn caller_timestamp_mode_reports_missing_timestamp() {
    let mut ingestor = NmeaIngestor::builder()
        .caller_timestamps()
        .build()
        .expect("config");
    assert!(ingestor.push_line(&sentence(RMC)).is_empty());
    assert_eq!(
        ingestor
            .diagnostics()
            .count(NmeaDiagnosticKind::InvalidHostTimestamp),
        1
    );
}

#[test]
fn host_only_satellite_reports_preserve_unassociated_evidence() {
    let mut ingestor = NmeaIngestor::builder().build().expect("config");
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("test timestamp");
    ingestor.push_line_at(&sentence(GSA), host);
    let completion = ingestor.finish();
    assert_eq!(completion.statistics.satellite_usage_reports, 1);
    let report = completion.records.first().expect("usage report");
    assert!(
        matches!(report,NmeaRecord::SatelliteUsage(report) if report.time == NavFixTime::Host(host))
    );
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::UnplacedObservation),
        1
    );
}

#[test]
fn buffer_limits_flush_partial_gsv_and_satellite_reports() {
    let mut ingestor = NmeaIngestor::builder()
        .max_gsv_groups(1)
        .max_pending_reports(1)
        .build()
        .expect("config");
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    for body in [
        RMC,
        "GPGSV,2,1,02,01,30,100,40",
        "GLGSV,2,1,02,65,30,100,40",
        GSA,
    ] {
        ingestor.push_line_at(&sentence(body), host);
    }
    let completion = ingestor.finish();
    assert_eq!(completion.statistics.satellite_observation_reports, 2);
    assert!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::BufferLimit)
            >= 2
    );
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::IncompleteGsv),
        2
    );
}

#[test]
fn gsv_fragment_limits_reject_unbounded_sequences() {
    let completion = ingest_bodies(
        NmeaIngestor::builder().max_gsv_fragments(2),
        &[RMC, "GPGSV,65535,1,01,01,30,100,40"],
    );
    assert_eq!(completion.statistics.satellite_observation_reports, 0);
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::MalformedSentence),
        1
    );
}

#[test]
fn gsv_assembly_timeout_expires_a_missing_fragment_before_the_next_update() {
    let mut ingestor = NmeaIngestor::builder()
        .sentence_period(SentenceType::GSV, Duration::from_secs(5))
        .gsv_assembly_timeout(Duration::from_secs(2))
        .build()
        .expect("config");
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("test timestamp");
    let mut records = ingestor.push_line_at(&sentence(RMC), host);
    records.extend(ingestor.push_line_at(&sentence("GPGSV,2,1,02,01,30,100,40"), host));
    records.extend(ingestor.push_line_at(
        &sentence("GPRMC,120003.000,A,5500.000,N,01200.000,E,1.0,90.0,231026,,,A"),
        host + TimeDelta::seconds(3),
    ));
    records.extend(ingestor.push_line_at(
        &sentence("GPGSV,2,2,02,02,30,100,40"),
        host + TimeDelta::seconds(5),
    ));
    let completion = ingestor.finish();
    records.extend(completion.records);
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
    assert_eq!(reports.len(), 2);
    assert!(
        reports
            .iter()
            .all(|report| report.coverage == SatelliteCoverage::Partial)
    );
}

#[test]
fn distant_receiver_time_does_not_infer_a_date_from_a_stale_anchor() {
    let completion = ingest_bodies(
        NmeaIngestor::builder().date_anchor_tolerance(Duration::from_secs(10)),
        &[
            RMC,
            "GPGGA,130000.000,5500.000,N,01200.000,E,1,08,0.7,12.3,M,40.0,M,,",
        ],
    );
    assert_eq!(completion.statistics.navigation_fixes, 1);
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::UnplacedObservation),
        1
    );
}

#[test]
fn conflicting_navigation_fields_have_stable_source_precedence() {
    let other = "GPGGA,120000.000,5600.000,N,01300.000,E,4,08,0.8,12.3,M,40.0,M,,";
    let left = ingest_bodies(NmeaIngestor::builder(), &[GGA, other, RMC]);
    let right = ingest_bodies(NmeaIngestor::builder(), &[other, GGA, RMC]);
    assert_eq!(fixes(&left), fixes(&right));
    assert_eq!(
        left.diagnostics
            .count(NmeaDiagnosticKind::ConflictingNavigation),
        1
    );
}

#[test]
fn unicode_sentence_envelopes_are_recoverable_without_calling_ascii_parser() {
    let completion = ingest_bodies(
        NmeaIngestor::builder(),
        &["ééGSA,A,3,01,02,,,,,,,,,,,1.2,0.7,0.9", RMC],
    );
    assert_eq!(completion.statistics.navigation_fixes, 1);
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::MalformedSentence),
        1
    );
}

#[rstest]
#[case(
    "GPRMC,120000.000,A,5500.000,N,01200.000,E,1.0,90.0,231026,,",
    SolutionState::Fix,
    SolutionMethod::Autonomous
)]
#[case(
    "GPRMC,120000.000,A,5500.000,N,01200.000,E,1.0,90.0,231026,,,D",
    SolutionState::Fix,
    SolutionMethod::Differential
)]
#[case(
    "GPRMC,120000.000,A,5500.000,N,01200.000,E,1.0,90.0,231026,,,,E",
    SolutionState::EstimatedOnly,
    SolutionMethod::DeadReckoning
)]
#[case(
    "GPRMC,120000.000,A,5500.000,N,01200.000,E,1.0,90.0,231026,,,R,V",
    SolutionState::Fix,
    SolutionMethod::RtkFixed
)]
fn supported_rmc_versions_preserve_reported_solution(
    #[case] body: &str,
    #[case] state: SolutionState,
    #[case] method: SolutionMethod,
) {
    let completion = ingest_bodies(NmeaIngestor::builder(), &[body]);
    let fix = fixes(&completion).into_iter().next().expect("fix");
    assert_eq!(fix.state, Some(state));
    assert_eq!(fix.method, Some(method));
}

#[test]
fn conflicting_gsa_dop_is_omitted_in_both_sentence_orders() {
    let different = "GLGSA,A,3,65,66,,,,,,,,,,,2.2,1.7,1.9";
    for input in [[GSA, different, RMC], [different, GSA, RMC]] {
        let completion = ingest_captured_bodies(
            NmeaIngestor::builder().association_policy(NmeaAssociationPolicy::HostCaptureProximity),
            &input,
        );
        let fix = fixes(&completion).into_iter().next().expect("fix");
        assert_eq!((fix.hdop, fix.pdop, fix.vdop), (None, None, None));
        assert_eq!(
            completion
                .diagnostics
                .count(NmeaDiagnosticKind::ConflictingNavigation),
            1
        );
    }
}

#[test]
fn gsv_missing_identity_preserves_partial_evidence_and_uncertainty() {
    let completion =
        ingest_captured_bodies(NmeaIngestor::builder(), &[RMC, "GLGSV,1,1,01,,,,26.9,1"]);
    let report = completion
        .records
        .iter()
        .find_map(|record| {
            if let NmeaRecord::SatelliteObservation(report) = record {
                Some(report)
            } else {
                None
            }
        })
        .expect("report");
    assert_eq!(report.coverage, SatelliteCoverage::Partial);
    assert!(report.satellites.is_empty());
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::UnresolvedSatelliteIdentity),
        1
    );
}

#[test]
fn microsecond_receiver_and_host_precision_survives_recording() {
    let input = format!(
        "2026-10-23 120000.654321 {}",
        sentence(&RMC.replace("120000.000", "120000.123456"))
    );
    let result = NmeaIngestor::builder()
        .timestamp_prefix_utc("%Y-%m-%d %H%M%S%.f")
        .build()
        .expect("config")
        .ingest_reader(Cursor::new(input), NavFileBuilder::new())
        .expect("recording");
    let file = roundtrip(&result.recording);
    assert_eq!(
        file.nav_points().first().expect("point").fix.time,
        NavFixTime::Both {
            gps: "2026-10-23T12:00:00.123456Z"
                .parse::<DateTime<Utc>>()
                .expect("test timestamp"),
            sys: "2026-10-23T12:00:00.654321Z"
                .parse::<DateTime<Utc>>()
                .expect("test timestamp")
        }
    );
}

proptest! {
    #[test]
    fn arbitrary_input_bytes_recover_before_a_valid_navigation_sentence(bytes in prop::collection::vec(any::<u8>(),0..8_192)) {
        let input = [bytes,b"\n".to_vec(),sentence(RMC).into_bytes()].concat();
        let result = NmeaIngestor::builder().max_line_bytes(128).diagnostic_samples(4).build().expect("config").ingest_reader(BufReader::with_capacity(17,Cursor::new(input)),NavFileBuilder::new()).expect("valid trailing navigation");
        prop_assert!(result.statistics.navigation_fixes >= 1);
        prop_assert!(result.diagnostics.samples().len() <= 4);
    }
}

#[rstest]
#[case(0, 1)]
#[case(1, 2)]
fn identical_gsv_content_preserves_distinct_host_captures(
    #[case] seconds: i64,
    #[case] count: u64,
) {
    let mut ingestor = NmeaIngestor::builder().build().expect("config");
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    let mut records = ingestor.push_line_at(&sentence(RMC), host);
    records.extend(ingestor.push_line_at(&sentence(GSV), host));
    records.extend(ingestor.push_line_at(&sentence(GSV), host + TimeDelta::seconds(seconds)));
    let completion = ingestor.finish();
    records.extend(completion.records);
    assert_eq!(completion.statistics.satellite_observation_reports, count);
    let timestamps: Vec<_> = records
        .into_iter()
        .filter_map(|record| {
            if let NmeaRecord::SatelliteObservation(report) = record {
                Some(report.time)
            } else {
                None
            }
        })
        .collect();
    assert!(timestamps.contains(&NavFixTime::Host(host)));
    assert!(timestamps.contains(&NavFixTime::Host(host + TimeDelta::seconds(seconds))));
}

#[rstest]
#[case(GSA)]
#[case(GSV)]
#[case("GPVTG,180.0,T,,M,2.0,N,3.704,K,A")]
fn crossing_navigation_epochs_preserve_association_uncertainty(#[case] report: &str) {
    let next = "GPRMC,120001.000,A,5500.000,N,01200.000,E,1.0,90.0,231026,,,A";
    let gll = "GPGLL,5500.000,N,01200.000,E,120000.000,A,A";
    for input in [
        [GGA, next, report, gll],
        [gll, next, report, GGA],
        [GGA, report, next, gll],
    ] {
        let completion = ingest_bodies(NmeaIngestor::builder(), &input);
        assert_eq!(completion.statistics.satellite_usage_reports, 0);
        assert_eq!(completion.statistics.satellite_observation_reports, 0);
        assert_eq!(
            completion
                .diagnostics
                .count(NmeaDiagnosticKind::UnplacedObservation),
            1
        );
        let older = fixes(&completion)
            .into_iter()
            .find(|fix| {
                fix.time.effective()
                    == "2026-10-23T12:00:00Z"
                        .parse::<DateTime<Utc>>()
                        .expect("timestamp")
            })
            .expect("older epoch");
        assert_eq!(older.pdop, None);
        assert_eq!(older.dimension, None);
        assert_eq!(older.heading, None);
        assert_eq!(older.speed, None);
    }
}

#[test]
fn buffered_older_epoch_accepts_complement_after_newer_capacity_eviction() {
    let newer = RMC.replace("120000.000", "120002.000");
    let gga = GGA.replace("120000.000", "120001.000");
    let rmc = RMC.replace("120000.000", "120001.000");
    let completion = ingest_bodies(
        NmeaIngestor::builder().max_pending_epochs(1),
        &[&newer, &gga, &rmc],
    );
    let older = fixes(&completion)
        .into_iter()
        .find(|fix| {
            fix.time.effective()
                == "2026-10-23T12:00:01Z"
                    .parse::<DateTime<Utc>>()
                    .expect("timestamp")
        })
        .expect("older epoch");
    assert_eq!(older.method, Some(SolutionMethod::RtkFixed));
    assert!(older.speed.is_some());
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::LateObservation),
        0
    );
}

#[test]
fn rtk_position_has_priority_over_an_autonomous_gns_position() {
    let gns = "GPGNS,120000.000,5600.000,N,01300.000,E,A,08,0.9,12.3,40.0,,,";
    for input in [[GGA, gns, RMC], [gns, GGA, RMC]] {
        let completion = ingest_bodies(NmeaIngestor::builder(), &input);
        let fix = fixes(&completion).into_iter().next().expect("fix");
        assert_eq!(fix.method, Some(SolutionMethod::RtkFixed));
        assert_eq!(fix.lat.as_degrees(), 55.0);
        assert_eq!(fix.lon.as_degrees(), 12.0);
    }
}

#[test]
fn timezone_names_are_rejected_and_utc_literals_are_accepted() {
    NmeaIngestor::builder()
        .timestamp_prefix_utc("%Y-%m-%d %H:%M:%S %Z")
        .build()
        .expect_err("named zones are unsupported");
    let mut ingestor = NmeaIngestor::builder()
        .timestamp_prefix_utc("%Y-%m-%d %H:%M:%S UTC")
        .build()
        .expect("UTC literal");
    ingestor.push_line(&format!("2026-10-23 12:00:00 UTC {}", sentence(RMC)));
    let completion = ingestor.finish();
    assert_eq!(completion.statistics.navigation_fixes, 1);
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::InvalidHostTimestamp),
        0
    );
}

#[rstest]
#[case(1_500, 0)]
#[case(1_501, 1)]
#[case(2_000, 1)]
#[case(3_000, 2)]
#[case(10_000, 9)]
fn expected_schedule_counts_missing_whole_updates_exactly(
    #[case] gap_ms: i64,
    #[case] missing: u64,
) {
    let mut ingestor = NmeaIngestor::builder()
        .sentence_period(SentenceType::GSV, Duration::from_secs(1))
        .build()
        .expect("config");
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    ingestor.push_line_at(&sentence(GSV), host);
    ingestor.push_line_at(&sentence(GSV), host + TimeDelta::milliseconds(gap_ms));
    let completion = ingestor.finish();
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::MissingUpdate),
        missing
    );
}

#[test]
fn reordered_updates_within_the_lateness_window_resolve_provisional_gaps() {
    let mut ingestor = NmeaIngestor::builder()
        .satellite_window(Duration::from_secs(4))
        .sentence_period(SentenceType::GSV, Duration::from_secs(1))
        .build()
        .expect("config");
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    for second in [0, 3, 1, 2] {
        ingestor.push_line_at(&sentence(GSV), host + TimeDelta::seconds(second));
    }
    let completion = ingestor.finish();
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::MissingUpdate),
        0
    );
    assert_eq!(completion.statistics.satellite_observation_reports, 4);
}

#[test]
fn deferred_diagnostic_ranges_use_source_line_extrema() {
    let completion = ingest_bodies(NmeaIngestor::builder(), &[GGA, GSA]);
    let count = completion
        .diagnostics
        .counts()
        .get(&NmeaDiagnosticKind::UnplacedObservation)
        .expect("unplaced observations");
    assert_eq!(count.count, 2);
    assert_eq!(count.first_line, 1);
    assert_eq!(count.last_line, 2);
    assert_eq!(
        completion
            .diagnostics
            .samples()
            .first()
            .expect("deferred sample")
            .line,
        2
    );
}

#[test]
fn multipart_gsv_retransmissions_match_the_original_fragment_capture() {
    let mut ingestor = NmeaIngestor::builder()
        .gsv_cycle_policy(NmeaGsvCyclePolicy::UniqueHostTimestamp)
        .build()
        .expect("config");
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    ingestor.push_line_at(&sentence(RMC), host);
    let first = sentence("GPGSV,2,1,02,01,30,100,40");
    let second = sentence("GPGSV,2,2,02,02,40,200,41");
    ingestor.push_line_at(&first, host);
    ingestor.push_line_at(&first, host);
    ingestor.push_line_at(&second, host);
    ingestor.push_line_at(&first, host);
    let completion = ingestor.finish();
    assert_eq!(completion.statistics.satellite_observation_reports, 1);
    assert!(completion.records.iter().any(|record| matches!(record,NmeaRecord::SatelliteObservation(report) if report.coverage == SatelliteCoverage::Complete)));
}

#[test]
fn completed_capture_cache_identifies_late_retransmissions_after_another_update() {
    let mut ingestor = NmeaIngestor::builder().build().expect("config");
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    for second in [0, 1, 0] {
        ingestor.push_line_at(&sentence(GSV), host + TimeDelta::seconds(second));
    }
    assert_eq!(
        ingestor.finish().statistics.satellite_observation_reports,
        2
    );
}

const GGA: &str = "GPGGA,120000.000,5500.000,N,01200.000,E,4,08,0.7,12.3,M,40.0,M,,";
const RMC: &str = "GPRMC,120000.000,A,5500.000,N,01200.000,E,1.0,90.0,231026,,,A";
const GSA: &str = "GPGSA,A,3,01,02,,,,,,,,,,,1.2,0.7,0.9";
const GSV: &str = "GPGSV,1,1,01,01,30,100,40";
const INTERLEAVED: &str = include_str!("fixtures/nmea/interleaved.nmea");
const PREFIXED: &str = include_str!("fixtures/nmea/prefixed.log");

#[rstest]
#[case(GSA)]
#[case(GSV)]
#[case("GPVTG,180.0,T,,M,2.0,N,3.704,K,A")]
fn epoch_capacity_includes_the_incoming_epoch_in_untimed_association(#[case] report: &str) {
    let next = RMC.replace("120000.000", "120001.000");
    let completion = ingest_bodies(
        NmeaIngestor::builder().max_pending_epochs(1),
        &[GGA, RMC, report, &next],
    );
    assert_eq!(completion.statistics.navigation_fixes, 2);
    assert_eq!(completion.statistics.satellite_usage_reports, 0);
    assert_eq!(completion.statistics.satellite_observation_reports, 0);
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::UnplacedObservation),
        1
    );
    let first = fixes(&completion)
        .into_iter()
        .find(|fix| {
            fix.time.effective()
                == "2026-10-23T12:00:00Z"
                    .parse::<DateTime<Utc>>()
                    .expect("timestamp")
        })
        .expect("first epoch");
    assert_eq!(first.pdop, None);
    assert_eq!(first.dimension, None);
}

#[rstest]
#[case(&[0,1,2],false,0)]
#[case(&[0,1,3],false,1)]
#[case(&[0,3],true,2)]
fn optional_host_timestamps_keep_one_coherent_sentence_schedule(
    #[case] seconds: &[i64],
    #[case] host_on_every_update: bool,
    #[case] missing: u64,
) {
    let mut ingestor = NmeaIngestor::builder()
        .sentence_period(SentenceType::RMC, Duration::from_secs(1))
        .build()
        .expect("config");
    let host = "2026-10-23T12:01:40Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    for (index, second) in seconds.iter().copied().enumerate() {
        let body = RMC.replace("120000.000", &format!("1200{second:02}.000"));
        if host_on_every_update || index == 1 {
            ingestor.push_line_at(&sentence(&body), host + TimeDelta::seconds(second));
        } else {
            ingestor.push_line(&sentence(&body));
        }
    }
    assert_eq!(
        ingestor
            .finish()
            .diagnostics
            .count(NmeaDiagnosticKind::MissingUpdate),
        missing
    );
}

#[rstest]
#[case(false)]
#[case(true)]
fn host_only_updates_before_a_receiver_date_preserve_missing_update_counts(
    #[case] append_receiver_only_update: bool,
) {
    let mut ingestor = NmeaIngestor::builder()
        .sentence_period(SentenceType::GGA, Duration::from_secs(1))
        .build()
        .expect("config");
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    ingestor.push_line_at(&sentence(GGA), host);
    ingestor.push_line(&sentence(&RMC.replace("120000.000", "120001.000")));
    ingestor.push_line_at(
        &sentence(&GGA.replace("120000.000", "120003.000")),
        host + TimeDelta::seconds(3),
    );
    if append_receiver_only_update {
        ingestor.push_line(&sentence(&GGA.replace("120000.000", "120004.000")));
    }
    assert_eq!(
        ingestor
            .finish()
            .diagnostics
            .count(NmeaDiagnosticKind::MissingUpdate),
        2
    );
}

#[rstest]
#[case(&[1], 1)]
#[case(&[2], 1)]
#[case(&[1, 2], 0)]
fn missing_host_clock_preserves_the_lateness_window_for_previous_updates(
    #[case] late_seconds: &[i64],
    #[case] missing: u64,
) {
    let mut ingestor = NmeaIngestor::builder()
        .navigation_window(Duration::from_secs(5))
        .sentence_period(SentenceType::GGA, Duration::from_secs(1))
        .build()
        .expect("config");
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    ingestor.push_line_at(&sentence(GGA), host);
    ingestor.push_line(&sentence(&RMC.replace("120000.000", "120001.000")));
    ingestor.push_line_at(
        &sentence(&GGA.replace("120000.000", "120003.000")),
        host + TimeDelta::seconds(3),
    );
    ingestor.push_line(&sentence(&GGA.replace("120000.000", "120004.000")));
    assert_eq!(
        ingestor
            .diagnostics()
            .count(NmeaDiagnosticKind::MissingUpdate),
        0
    );
    for second in late_seconds {
        ingestor.push_line_at(
            &sentence(&GGA.replace("120000.000", &format!("1200{second:02}.000"))),
            host + TimeDelta::seconds(*second),
        );
    }
    assert_eq!(
        ingestor
            .finish()
            .diagnostics
            .count(NmeaDiagnosticKind::MissingUpdate),
        missing
    );
}

#[rstest]
#[case(false, 2)]
#[case(true, 4)]
fn repeated_host_capture_merges_new_receiver_schedule_evidence(
    #[case] finalize_initial_capture: bool,
    #[case] missing: u64,
) {
    let mut ingestor = NmeaIngestor::builder()
        .navigation_window(Duration::from_secs(5))
        .sentence_period(SentenceType::GGA, Duration::from_secs(1))
        .build()
        .expect("config");
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    ingestor.push_line_at(&sentence(GGA), host);
    ingestor.push_line(&sentence(&RMC.replace("120000.000", "120001.000")));
    ingestor.push_line_at(
        &sentence(&GGA.replace("120000.000", "120003.000")),
        host + TimeDelta::seconds(3),
    );
    if finalize_initial_capture {
        ingestor.push_line_at(
            &sentence(&RMC.replace("120000.000", "120006.000")),
            host + TimeDelta::seconds(6),
        );
    }
    ingestor.push_line_at(&sentence(GGA), host);
    assert_eq!(
        ingestor
            .finish()
            .diagnostics
            .count(NmeaDiagnosticKind::MissingUpdate),
        missing
    );
}

#[test]
fn late_receiver_update_counts_a_gap_once_after_a_missing_receiver_clock() {
    let mut ingestor = NmeaIngestor::builder()
        .navigation_window(Duration::from_secs(5))
        .sentence_period(SentenceType::GGA, Duration::from_secs(1))
        .build()
        .expect("config");
    let host = "2026-10-23T12:00:00Z"
        .parse::<DateTime<Utc>>()
        .expect("timestamp");
    ingestor.push_line(&sentence(RMC));
    for second in [0, 3] {
        ingestor.push_line_at(
            &sentence(&GGA.replace("120000.000", &format!("1200{second:02}.000"))),
            host + TimeDelta::seconds(second),
        );
    }
    ingestor.push_line_at(
        &sentence(&GGA.replace("120000.000", "")),
        host + TimeDelta::seconds(4),
    );
    ingestor.push_line_at(
        &sentence(&GGA.replace("120000.000", "120002.000")),
        host + TimeDelta::seconds(2),
    );
    assert_eq!(
        ingestor
            .finish()
            .diagnostics
            .count(NmeaDiagnosticKind::MissingUpdate),
        1
    );
}

#[rstest]
#[case(SentenceType::GSA, GSA)]
#[case(SentenceType::GSV, GSV)]
#[case(SentenceType::VTG, "GPVTG,90.0,T,,M,1.0,N,1.852,K,A")]
fn receiver_associated_updates_preserve_missing_update_counts(
    #[case] configured_sentence: SentenceType,
    #[case] body: &str,
) {
    let next = RMC.replace("120000.000", "120003.000");
    let next_gga = GGA.replace("120000.000", "120003.000");
    let completion = ingest_bodies(
        NmeaIngestor::builder()
            .association_policy(NmeaAssociationPolicy::ReceiverEpochBracketing)
            .sentence_period(configured_sentence, Duration::from_secs(1)),
        &[RMC, body, GGA, &next, body, &next_gga],
    );
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::MissingUpdate),
        2
    );
}

#[rstest]
#[case(SentenceType::GSA, GSA)]
#[case(SentenceType::GSV, GSV)]
fn unplaced_satellite_update_prevents_missing_update_counts_across_its_interval(
    #[case] configured_sentence: SentenceType,
    #[case] body: &str,
) {
    let first = RMC.replace("120000.000", "120001.000");
    let second = RMC.replace("120000.000", "120002.000");
    let next = RMC.replace("120000.000", "120003.000");
    let next_gga = GGA.replace("120000.000", "120003.000");
    let completion = ingest_bodies(
        NmeaIngestor::builder()
            .association_policy(NmeaAssociationPolicy::ReceiverEpochBracketing)
            .sentence_period(configured_sentence, Duration::from_secs(1)),
        &[
            RMC, body, GGA, &first, body, &second, &next, body, &next_gga,
        ],
    );
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::MissingUpdate),
        0
    );
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::UnplacedObservation),
        1
    );
    assert_eq!(
        completion.statistics.satellite_usage_reports
            + completion.statistics.satellite_observation_reports,
        2
    );
}

#[test]
fn incomplete_untimed_gsv_preserves_uncertainty_after_epoch_eviction() {
    let next = RMC.replace("120000.000", "120001.000");
    let completion = ingest_bodies(
        NmeaIngestor::builder().max_pending_epochs(1),
        &[
            RMC,
            "GPGSV,2,1,02,01,30,100,40",
            &next,
            "GPGSV,2,2,02,02,40,200,41",
        ],
    );
    assert_eq!(completion.statistics.satellite_observation_reports, 0);
    assert_eq!(
        completion
            .diagnostics
            .count(NmeaDiagnosticKind::UnplacedObservation),
        2
    );
}
