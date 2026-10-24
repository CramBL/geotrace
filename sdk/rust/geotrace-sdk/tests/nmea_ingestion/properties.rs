use std::collections::BTreeMap;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use geotrace_sdk::{
    Constellation, NavFileBuilder, NavFix, NavFixTime, NmeaAssociationPolicy, NmeaDiagnosticKind,
    NmeaGsvCyclePolicy, NmeaIngestor, NmeaRecord, PositionProvenance, SatelliteCoverage,
    SatelliteIdentity, SatelliteMembership, SatelliteObservation, SatelliteScope, SentenceType,
    SolutionDimension, SolutionMethod, SolutionState,
};
use proptest::prelude::*;
use proptest::test_runner::{FileFailurePersistence, TestCaseResult};

#[path = "degradation.rs"]
mod degradation;

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(concat!(
            env!("CARGO_MANIFEST_DIR"), "/tests/nmea_ingestion/properties.proptest-regressions"
        )))),
        ..ProptestConfig::default()
    })]

    #[test]
    fn generated_navigation_preserves_values_under_sentence_permutation(
        case in NavigationCase::strategy(),
        host_offset_seconds in -86_400_i64..=86_400,
    ) {
        let host = case.receiver + TimeDelta::seconds(host_offset_seconds);
        let lower_quality_position = NavigationCase {
            latitude_minute_units: -case.latitude_minute_units,
            longitude_minute_units: -case.longitude_minute_units,
            ..case.clone()
        };
        let sentences = [case.gga(), lower_quality_position.rmc()];
        let mut previous = None;
        for reverse in [false, true] {
            let mut ingestor = NmeaIngestor::builder().build().expect("configuration");
            let mut recorder = NavFileBuilder::new().open();
            let mut ordered = sentences.clone();
            if reverse {
                ordered.reverse();
            }
            for line in ordered {
                for record in ingestor.push_line_at(&line, host) {
                    recorder.add(record);
                }
            }
            let completion = ingestor.finish();
            for record in completion.records {
                recorder.add(record);
            }
            let recording = super::roundtrip(&recorder.finish().expect("recording"));
            let [point] = recording.nav_points() else {
                return Err(TestCaseError::fail("one generated epoch"));
            };
            case.assert_fix(&point.fix)?;
            prop_assert_eq!(
                point.fix.time,
                NavFixTime::Both {
                    gps: case.receiver,
                    sys: host
                }
            );
            if let Some(previous) = previous {
                prop_assert_eq!(point.fix, previous);
            }
            previous = Some(point.fix);
        }
    }

    #[test]
    fn generated_midnight_epochs_preserve_dates_and_independent_clocks(
        case in NavigationCase::strategy(),
        before_midnight_seconds in 1_i64..=30,
        after_midnight_seconds in 0_i64..=30,
        clock_mode in prop::sample::select(vec![HostClockMode::Caller, HostClockMode::Prefix, HostClockMode::Receiver]),
    ) {
        let midnight = case
            .receiver
            .date_naive()
            .and_hms_opt(0, 0, 0)
            .expect("midnight")
            .and_utc();
        let before = midnight - TimeDelta::seconds(before_midnight_seconds);
        let after = midnight + TimeDelta::seconds(after_midnight_seconds);
        let host = midnight + TimeDelta::days(3);
        let earlier = NavigationCase {
            receiver: before,
            ..case.clone()
        };
        let later = NavigationCase {
            receiver: after,
            ..case
        };
        let mut builder = NmeaIngestor::builder()
            .navigation_window(Duration::from_secs(120))
            .sentence_period(SentenceType::RMC, Duration::from_secs(1));
        if clock_mode == HostClockMode::Prefix {
            builder = builder.timestamp_prefix_utc("%Y-%m-%d %H%M%S%.f");
        }
        let mut ingestor = builder.build().expect("configuration");
        let mut recorder = NavFileBuilder::new().open();
        for (offset, line) in [earlier.gga(), later.gga(), earlier.rmc()]
            .iter()
            .enumerate()
        {
            let capture = host + TimeDelta::milliseconds(offset as i64);
            let records = match clock_mode {
                HostClockMode::Receiver => ingestor.push_line(line),
                HostClockMode::Caller => ingestor.push_line_at(line, capture),
                HostClockMode::Prefix => {
                    ingestor.push_line(&format!("{} {line}", capture.format("%Y-%m-%d %H%M%S%.f")))
                }
            };
            for record in records {
                recorder.add(record);
            }
        }
        for record in ingestor.finish().records {
            recorder.add(record);
        }
        let recording = super::roundtrip(&recorder.finish().expect("recording"));
        prop_assert_eq!(recording.nav_points().len(), 2);
        for (offset, (point, receiver)) in recording
            .nav_points()
            .iter()
            .zip([before, after])
            .enumerate()
        {
            let expected_time = if clock_mode == HostClockMode::Receiver {
                NavFixTime::Receiver(receiver)
            } else {
                NavFixTime::Both {
                    gps: receiver,
                    sys: host + TimeDelta::milliseconds(offset as i64),
                }
            };
            prop_assert_eq!(point.fix.time, expected_time);
            prop_assert_eq!(point.fix.method, Some(earlier.quality.method));
            prop_assert_eq!(point.fix.speed.is_some(), offset == 0);
        }
    }

    #[test]
    fn generated_satellite_fields_survive_fragment_order_and_recording(
        case in SatelliteCase::strategy(),
        ordering in prop::collection::vec(any::<u32>(), 4),
        signal in prop::option::of(1_u8..=15),
        omit_last_fragment in any::<bool>(),
    ) {
        let mut fragments = case.gsv(signal);
        if omit_last_fragment && fragments.len() > 1 {
            fragments.pop();
        }
        let retained_count = (fragments.len() * GSV_SATELLITES_PER_FRAGMENT).min(case.satellites.len());
        let complete = retained_count == case.satellites.len();
        let mut sentences = vec![super::sentence(super::RMC), case.gsa()];
        sentences.extend(fragments);
        let mut ordered: Vec<_> = ordering.into_iter().zip(sentences).collect();
        ordered.sort_by_key(|(priority, _)| *priority);
        let host = "2026-10-23T12:00:00.025Z"
            .parse::<DateTime<Utc>>()
            .expect("host time");
        let mut ingestor = NmeaIngestor::builder().association_policy(NmeaAssociationPolicy::HostCaptureProximity).gsv_cycle_policy(NmeaGsvCyclePolicy::UniqueHostTimestamp).build().expect("configuration");
        let mut recorder = NavFileBuilder::new().open();
        for (_, sentence) in ordered {
            for record in ingestor.push_line_at(&sentence, host) {
                recorder.add(record);
            }
        }
        let completion = ingestor.finish();
        for record in completion.records {
            recorder.add(record);
        }
        let recording = super::roundtrip(&recorder.finish().expect("recording"));
        let [point] = recording.nav_points() else {
            return Err(TestCaseError::fail("one navigation epoch"));
        };
        let [usage] = point.satellite_evidence().usage.as_slice() else {
            return Err(TestCaseError::fail("one usage report"));
        };
        prop_assert!(
            matches!(&usage.scope, SatelliteScope::Constellations(set) if set.as_slice() == [case.constellation])
        );
        prop_assert_eq!(usage.time, NavFixTime::Host(host));
        prop_assert_eq!(usage.coverage, SatelliteCoverage::Partial);
        prop_assert_eq!(usage.reported_count, None);
        let used: Vec<_> = usage
            .satellites
            .iter()
            .map(|satellite| {
                (
                    satellite.identity.constellation,
                    satellite.identity.prn,
                    satellite.membership,
                )
            })
            .collect();
        let expected_used: Vec<_> = case
            .used_prns()
            .into_iter()
            .map(|prn| (case.constellation, prn, SatelliteMembership::Used))
            .collect();
        prop_assert_eq!(used, expected_used);
        let [observations] = point.satellite_evidence().observations.as_slice() else {
            return Err(TestCaseError::fail("one observation report"));
        };
        prop_assert!(
            matches!(&observations.scope, SatelliteScope::Constellations(set) if set.as_slice() == [case.constellation])
        );
        prop_assert_eq!(observations.time, NavFixTime::Host(host));
        prop_assert_eq!(
            observations.coverage,
            if complete && signal.is_none() {
                SatelliteCoverage::Complete
            } else {
                SatelliteCoverage::Partial
            }
        );
        prop_assert_eq!(
            observations.reported_count,
            signal.is_none().then_some(case.satellites.len() as u32)
        );
        prop_assert_eq!(observations.satellites.len(), retained_count);
        for (satellite, (prn, expected)) in observations
            .satellites
            .iter()
            .zip(case.satellites.iter().take(retained_count))
        {
            prop_assert_eq!(satellite.identity.constellation, case.constellation);
            prop_assert_eq!(satellite.identity.prn, *prn);
            prop_assert_eq!(satellite.elevation, expected.elevation.map(f32::from));
            prop_assert_eq!(satellite.azimuth, expected.azimuth.map(f32::from));
            prop_assert_eq!(satellite.snr, expected.snr.map(f32::from));
        }
        prop_assert_eq!(point.fix.dimension, Some(SolutionDimension::ThreeD));
        prop_assert!(
            (point.fix.hdop.expect("HDOP") - f64::from(case.hdop_tenths) / 10.0).abs() < DOP_ERROR
        );
        prop_assert!(
            (point.fix.vdop.expect("VDOP") - f64::from(case.vdop_tenths) / 10.0).abs() < DOP_ERROR
        );
        prop_assert!(
            (point.fix.pdop.expect("PDOP") - f64::from(case.hdop_tenths + case.vdop_tenths) / 10.0)
                .abs()
                < DOP_ERROR
        );
    }

    #[test]
    fn generated_signal_groups_preserve_each_sample_without_complete_coverage(
        case in SatelliteCase::strategy(),
        ordering in prop::collection::vec(any::<u32>(), 4),
        first_signal in 1_u8..=7,
    ) {
        let mut second_signal = case.clone();
        for satellite in second_signal.satellites.values_mut() {
            satellite.snr = satellite.snr.map(|value| 99 - value);
        }
        let fragments: Vec<_> = case
            .gsv(Some(first_signal))
            .into_iter()
            .chain(second_signal.gsv(Some(first_signal + 8)))
            .collect();
        let mut ordered: Vec<_> = ordering.into_iter().zip(fragments).collect();
        ordered.sort_by_key(|(priority, _)| *priority);
        let host = "2026-10-23T12:00:00Z"
            .parse::<DateTime<Utc>>()
            .expect("host time");
        let mut ingestor = NmeaIngestor::builder().gsv_cycle_policy(NmeaGsvCyclePolicy::UniqueHostTimestamp).build().expect("configuration");
        let mut records = ingestor.push_line_at(&super::sentence(super::RMC), host);
        for (_, sentence) in ordered {
            records.extend(ingestor.push_line_at(&sentence, host));
        }
        let completion = ingestor.finish();
        records.extend(completion.records);
        let mut recorder = NavFileBuilder::new().open();
        for record in records {
            recorder.add(record);
        }
        let recording = super::roundtrip(&recorder.finish().expect("recording"));
        let point = recording.nav_points().first().expect("navigation point");
        for report in &point.satellite_evidence().observations {
            prop_assert_eq!(report.coverage, SatelliteCoverage::Partial);
            prop_assert_eq!(report.reported_count, None);
            prop_assert_eq!(report.time, NavFixTime::Host(host));
            prop_assert!(
                matches!(&report.scope, SatelliteScope::Constellations(set) if set.as_slice() == [case.constellation])
            );
        }
        let [first, second] = point.satellite_evidence().observations.as_slice() else {
            return Err(TestCaseError::fail("two complete signal groups"));
        };
        let expected_first = case.expected_observations();
        let expected_second = second_signal.expected_observations();
        prop_assert_eq!(first.satellites.len(), case.satellites.len());
        prop_assert_eq!(second.satellites.len(), case.satellites.len());
        prop_assert!(
            (first.satellites == expected_first && second.satellites == expected_second)
                || (first.satellites == expected_second && second.satellites == expected_first)
        );
        prop_assert_eq!(
            completion
                .diagnostics
                .count(NmeaDiagnosticKind::SignalDimensionOmitted),
            2
        );
    }

    #[test]
    fn generated_navigation_is_unchanged_by_recoverable_interleaving(
        cases in prop::collection::vec(NavigationCase::strategy(), 1..=8),
        noise_counts in prop::collection::vec(0_usize..=4, 16),
    ) {
        let generated_epoch_count = cases.len();
        let expected_noise_count: usize = noise_counts.iter().take(generated_epoch_count * 2).sum();
        let base = cases.first().expect("nonempty generated sequence").receiver;
        let mut clean = NmeaIngestor::builder().build().expect("configuration");
        let mut noisy = NmeaIngestor::builder().build().expect("configuration");
        let mut clean_records = Vec::new();
        let mut noisy_records = Vec::new();
        for (index, case) in cases.into_iter().enumerate() {
            let case = NavigationCase {
                receiver: base + TimeDelta::seconds(index as i64),
                ..case
            };
            for (line, count) in [case.gga(), case.rmc()]
                .iter()
                .zip(noise_counts.iter().skip(index * 2))
            {
                clean_records.extend(clean.push_line(line));
                noisy_records.extend(noisy.push_line(line));
                for _ in 0..*count {
                    noisy_records.extend(noisy.push_line("$PAIR062,0,1*3F"));
                    let (prefix, checksum) = line.rsplit_once('*').expect("generated checksum");
                    let checksum = u8::from_str_radix(checksum, 16).expect("hexadecimal checksum");
                    let malformed = format!("{prefix}*{:02X}", checksum ^ 1);
                    noisy_records.extend(noisy.push_line(&malformed));
                }
            }
        }
        clean_records.extend(clean.finish().records);
        let completion = noisy.finish();
        noisy_records.extend(completion.records);
        let clean_fixes: Vec<_> = clean_records
            .into_iter()
            .filter_map(|record| {
                if let NmeaRecord::Navigation(fix) = record {
                    Some(fix)
                } else {
                    None
                }
            })
            .collect();
        let noisy_fixes: Vec<_> = noisy_records
            .into_iter()
            .filter_map(|record| {
                if let NmeaRecord::Navigation(fix) = record {
                    Some(fix)
                } else {
                    None
                }
            })
            .collect();
        prop_assert_eq!(clean_fixes.len(), generated_epoch_count);
        prop_assert_eq!(noisy_fixes.len(), generated_epoch_count);
        prop_assert_eq!(clean_fixes, noisy_fixes);
        prop_assert_eq!(
            completion
                .diagnostics
                .count(NmeaDiagnosticKind::UnsupportedSentence),
            expected_noise_count as u64
        );
        prop_assert_eq!(
            completion
                .diagnostics
                .count(NmeaDiagnosticKind::MalformedSentence),
            expected_noise_count as u64
        );
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum HostClockMode {
    Caller,
    Prefix,
    Receiver,
}

#[derive(Clone, Copy, Debug)]
struct FixQualityCase {
    indicator: u8,
    method: SolutionMethod,
}

#[derive(Clone, Debug)]
struct NavigationCase {
    latitude_minute_units: i32,
    longitude_minute_units: i32,
    speed_hundredths: u16,
    course_hundredths: u16,
    quality: FixQualityCase,
    used_count: u8,
    hdop_tenths: u8,
    receiver: DateTime<Utc>,
}

impl NavigationCase {
    #[expect(clippy::expect_used, reason = "fixed test timestamp must parse")]
    fn strategy() -> impl Strategy<Value = Self> {
        (
            -54_000_000_i32..=54_000_000,
            -108_000_000_i32..=108_000_000,
            0_u16..=20_000,
            0_u16..=35_999,
            prop::sample::select(vec![
                FixQualityCase {
                    indicator: 1,
                    method: SolutionMethod::Autonomous,
                },
                FixQualityCase {
                    indicator: 2,
                    method: SolutionMethod::Differential,
                },
                FixQualityCase {
                    indicator: 4,
                    method: SolutionMethod::RtkFixed,
                },
                FixQualityCase {
                    indicator: 5,
                    method: SolutionMethod::RtkFloat,
                },
            ]),
            4_u8..=32,
            1_u8..=100,
            0_i64..=1_095,
            0_i64..86_400,
            0_i64..1_000_000,
        )
            .prop_map(
                |(
                    latitude_minute_units,
                    longitude_minute_units,
                    speed_hundredths,
                    course_hundredths,
                    quality,
                    used_count,
                    hdop_tenths,
                    days,
                    seconds,
                    micros,
                )| {
                    let base = "2024-01-01T00:00:00Z"
                        .parse::<DateTime<Utc>>()
                        .expect("base time");
                    Self {
                        latitude_minute_units,
                        longitude_minute_units,
                        speed_hundredths,
                        course_hundredths,
                        quality,
                        used_count,
                        hdop_tenths,
                        receiver: base
                            + TimeDelta::days(days)
                            + TimeDelta::seconds(seconds)
                            + TimeDelta::microseconds(micros),
                    }
                },
            )
    }

    fn gga(&self) -> String {
        super::sentence(&format!(
            "GPGGA,{},{},{},{},{:.1},12.3,M,40.0,M,,",
            self.receiver.format("%H%M%S%.6f"),
            self.coordinate_fields(),
            self.quality.indicator,
            self.used_count,
            f64::from(self.hdop_tenths) / 10.0,
        ))
    }

    fn rmc(&self) -> String {
        super::sentence(&format!(
            "GPRMC,{},A,{},{}.{:02},{}.{:02},{},,,A",
            self.receiver.format("%H%M%S%.6f"),
            self.coordinate_fields(),
            self.speed_hundredths / 100,
            self.speed_hundredths % 100,
            self.course_hundredths / 100,
            self.course_hundredths % 100,
            self.receiver.format("%d%m%y"),
        ))
    }

    fn coordinate_fields(&self) -> String {
        [
            (self.latitude_minute_units, 2, 'N', 'S'),
            (self.longitude_minute_units, 3, 'E', 'W'),
        ]
        .map(|(signed_units, width, positive, negative)| {
            let units = signed_units.unsigned_abs();
            let degrees = units / MINUTE_UNITS_PER_DEGREE;
            let minutes = (units % MINUTE_UNITS_PER_DEGREE) / 10_000;
            let fraction = units % 10_000;
            let hemisphere = if signed_units < 0 { negative } else { positive };
            format!("{degrees:0width$}{minutes:02}.{fraction:04},{hemisphere}")
        })
        .join(",")
    }

    fn assert_fix(&self, fix: &NavFix) -> TestCaseResult {
        let (Some(speed), Some(heading), Some(hdop)) = (fix.speed, fix.heading, fix.hdop) else {
            return Err(TestCaseError::fail(
                "generated speed, course and HDOP must be present",
            ));
        };
        prop_assert!(
            (fix.lat.as_degrees()
                - f64::from(self.latitude_minute_units) / f64::from(MINUTE_UNITS_PER_DEGREE))
            .abs()
                < COORDINATE_ERROR_DEGREES
        );
        prop_assert!(
            (fix.lon.as_degrees()
                - f64::from(self.longitude_minute_units) / f64::from(MINUTE_UNITS_PER_DEGREE))
            .abs()
                < COORDINATE_ERROR_DEGREES
        );
        let expected_speed =
            f64::from(self.speed_hundredths) / 100.0 * METERS_PER_NAUTICAL_MILE / SECONDS_PER_HOUR;
        prop_assert!(
            (speed.as_meters_per_second() - expected_speed).abs()
                <= expected_speed.abs() * f64::from(f32::EPSILON) + f64::EPSILON
        );
        let expected_course = f64::from(self.course_hundredths) / 100.0;
        prop_assert!(
            (heading.as_degrees() - expected_course).abs()
                <= expected_course.abs() * f64::from(f32::EPSILON) + f64::EPSILON
        );
        prop_assert_eq!(fix.state, Some(SolutionState::Fix));
        prop_assert_eq!(fix.method, Some(self.quality.method));
        prop_assert_eq!(fix.used_count, Some(u32::from(self.used_count)));
        prop_assert_eq!(
            fix.position_provenance,
            Some(PositionProvenance::SourceProvided)
        );
        prop_assert!((hdop - f64::from(self.hdop_tenths) / 10.0).abs() < DOP_ERROR);
        prop_assert_eq!(fix.eph_m, None);
        Ok(())
    }
}

#[derive(Clone, Debug)]
struct SatelliteCase {
    talker: &'static str,
    system_id: u8,
    constellation: Constellation,
    satellites: BTreeMap<u32, SatelliteFields>,
    hdop_tenths: u8,
    vdop_tenths: u8,
}

impl SatelliteCase {
    fn strategy() -> impl Strategy<Value = Self> {
        (
            prop::sample::select(vec![
                ("GP", 1, Constellation::Gps),
                ("GA", 3, Constellation::Galileo),
            ]),
            prop::collection::btree_map(1_u32..=32, SatelliteFields::strategy(), 1..=8),
            1_u8..=100,
            1_u8..=100,
        )
            .prop_map(
                |((talker, system_id, constellation), satellites, hdop_tenths, vdop_tenths)| Self {
                    talker,
                    system_id,
                    constellation,
                    satellites,
                    hdop_tenths,
                    vdop_tenths,
                },
            )
    }

    fn expected_observations(&self) -> Vec<SatelliteObservation> {
        self.satellites
            .iter()
            .map(|(prn, satellite)| {
                SatelliteObservation::builder()
                    .identity(SatelliteIdentity::new(self.constellation, *prn))
                    .maybe_elevation(satellite.elevation.map(f32::from))
                    .maybe_azimuth(satellite.azimuth.map(f32::from))
                    .maybe_snr(satellite.snr.map(f32::from))
                    .build()
            })
            .collect()
    }

    fn gsa(&self) -> String {
        let mut slots: Vec<_> = self
            .used_prns()
            .iter()
            .map(|prn| format!("{prn:02}"))
            .collect();
        slots.resize(GSA_SATELLITE_SLOTS, String::new());
        super::sentence(&format!(
            "GNGSA,A,3,{},{:.1},{:.1},{:.1},{}",
            slots.join(","),
            f64::from(self.hdop_tenths + self.vdop_tenths) / 10.0,
            f64::from(self.hdop_tenths) / 10.0,
            f64::from(self.vdop_tenths) / 10.0,
            self.system_id,
        ))
    }

    fn gsv(&self, signal: Option<u8>) -> Vec<String> {
        let entries: Vec<_> = self.satellites.iter().collect();
        let count = entries.len().div_ceil(GSV_SATELLITES_PER_FRAGMENT);
        entries
            .chunks(GSV_SATELLITES_PER_FRAGMENT)
            .enumerate()
            .map(|(index, entries)| {
                let fields: Vec<_> = entries
                    .iter()
                    .map(|(prn, satellite)| {
                        format!(
                            "{prn:02},{},{},{}",
                            satellite
                                .elevation
                                .map_or(String::new(), |value| value.to_string()),
                            satellite
                                .azimuth
                                .map_or(String::new(), |value| value.to_string()),
                            satellite
                                .snr
                                .map_or(String::new(), |value| value.to_string())
                        )
                    })
                    .collect();
                let signal = signal.map_or(String::new(), |value| format!(",{value:X}"));
                super::sentence(&format!(
                    "{}GSV,{count},{},{:02},{}{signal}",
                    self.talker,
                    index + 1,
                    self.satellites.len(),
                    fields.join(",")
                ))
            })
            .collect()
    }

    fn used_prns(&self) -> Vec<u32> {
        self.satellites
            .iter()
            .enumerate()
            .filter_map(|(index, (prn, satellite))| (index == 0 || satellite.used).then_some(*prn))
            .collect()
    }
}

#[derive(Clone, Debug)]
struct SatelliteFields {
    elevation: Option<u8>,
    azimuth: Option<u16>,
    snr: Option<u8>,
    used: bool,
}

impl SatelliteFields {
    fn strategy() -> impl Strategy<Value = Self> {
        (
            prop::option::of(0_u8..=90),
            prop::option::of(0_u16..360),
            prop::option::of(0_u8..=99),
            any::<bool>(),
        )
            .prop_map(|(elevation, azimuth, snr, used)| Self {
                elevation,
                azimuth,
                snr,
                used,
            })
    }
}

const MINUTE_UNITS_PER_DEGREE: u32 = 600_000;
const COORDINATE_ERROR_DEGREES: f64 = 1e-10;
const DOP_ERROR: f64 = 1e-6;
const METERS_PER_NAUTICAL_MILE: f64 = 1852.0;
const SECONDS_PER_HOUR: f64 = 3600.0;
const GSV_SATELLITES_PER_FRAGMENT: usize = 4;
const GSA_SATELLITE_SLOTS: usize = 12;
