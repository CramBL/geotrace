use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use geotrace_sdk::{
    Angle, Constellation, NavFile, NavFileBuilder, NavFix, NavFixTime, NmeaDiagnosticKind,
    NmeaDiagnostics, NmeaIngestor, NmeaRecord, SatelliteCoverage, SatelliteIdentity,
    SatelliteMembership, SatelliteObservation, SatelliteObservationReport, SatelliteScope,
    SatelliteUsageReport, SentenceType, SolutionDimension, SolutionMethod, SolutionState,
};
use proptest::prelude::*;
use proptest::test_runner::{FileFailurePersistence, TestCaseResult};
use rstest::rstest;
use strum::{EnumIter, IntoEnumIterator as _};

use super::NavigationCase;

proptest! {
    #![proptest_config(ProptestConfig {
        failure_persistence: Some(Box::new(FileFailurePersistence::Direct(concat!(
            env!("CARGO_MANIFEST_DIR"), "/tests/nmea_ingestion/degradation.proptest-regressions"
        )))),
        max_shrink_iters: 8_192,
        ..ProptestConfig::default()
    })]

    #[test]
    fn undegraded_witnesses_retain_navigation_and_independent_satellite_facts(
        mut model in RecordingModel::strategy(),
    ) {
        model.clock_event = ClockEvent::None;
        model.host_mode = HostMode::All;
        model.capture_delays.clear();
        let input = model.complete();
        let expected_epochs: BTreeSet<_> = input.surviving().filter(|packet| packet.navigation().is_some()).map(|packet| packet.epoch.0).collect();
        let expected_usage = input.surviving().filter(|packet| matches!(packet.observation, Observation::Usage { .. })).count();
        let expected_observations = input.surviving().filter(|packet| matches!(packet.observation, Observation::Fragment { .. })).count();
        let completion = input.ingest_with_window(Duration::from_secs(120));
        prop_assert_eq!(completion.records.iter().filter(|record| matches!(record, NmeaRecord::Navigation(_))).count(), expected_epochs.len());
        prop_assert_eq!(completion.records.iter().filter(|record| matches!(record, NmeaRecord::SatelliteUsage(_))).count(), expected_usage);
        prop_assert_eq!(completion.records.iter().filter(|record| matches!(record, NmeaRecord::SatelliteObservation(_))).count(), expected_observations);
        input.assert_measurements(&completion)?;
        input.assert_timestamps(&completion)?;
        input.assert_roundtrip(&completion)?;
    }

    #[test]
    fn p1_every_degraded_measurement_has_surviving_source_evidence(
        model in RecordingModel::strategy(),
        degradation in Degradation::strategy(),
    ) {
        let input = model.degrade(&degradation);
        let completion = input.ingest();
        input.assert_measurements(&completion)?;
        input.assert_roundtrip(&completion)?;
    }

    #[test]
    fn p2_receiver_dates_and_host_captures_have_independent_segment_provenance(
        model in RecordingModel::strategy(),
        degradation in Degradation::strategy(),
    ) {
        let input = model.degrade(&degradation);
        let completion = input.ingest();
        input.assert_timestamps(&completion)?;
    }

    #[test]
    fn p3_deletion_cannot_create_measurements_or_known_unused_membership(
        model in RecordingModel::strategy(),
        choices in prop::collection::vec(any::<bool>(), 1..=128),
        omitted_range in (0_usize..128, 0_usize..64),
        omitted_identities in any::<u16>(),
    ) {
        let mut input = model.complete();
        input.items.retain(|item| {
            let index = item.source.as_ref().expect("source packet").ordinal;
            choices[index % choices.len()]
                && !(omitted_range.0..omitted_range.0.saturating_add(omitted_range.1)).contains(&index)
        });
        for item in &mut input.items {
            if let Some(packet) = &mut item.source
                && let Observation::Usage { identities, .. } = &mut packet.observation {
                    identities.retain(|identity| omitted_identities & (1 << (identity.prn % 16)) != 0);
                    item.bytes = packet.sentence().into_bytes();
            }
        }
        let completion = input.ingest();
        input.assert_measurements(&completion)?;
        input.assert_timestamps(&completion)?;
        for record in &completion.records {
            if let NmeaRecord::SatelliteUsage(report) = record {
                prop_assert_eq!(report.coverage, SatelliteCoverage::Partial);
                prop_assert_eq!(report.reported_count, None);
                prop_assert!(report.satellites.iter().all(|satellite| satellite.membership != SatelliteMembership::Unused));
            }
        }
        input.assert_roundtrip(&completion)?;
    }

    #[test]
    fn p4_compatible_sentence_permutations_preserve_navigation_semantics(
        mut model in RecordingModel::strategy(),
        priorities in prop::collection::vec(any::<u32>(), 1..=128),
    ) {
        model.clock_event = ClockEvent::None;
        model.host_mode = HostMode::All;
        model.capture_delays.clear();
        model.solution = ReportedSolution::RtkFixed;
        let original = model.complete();
        let mut reordered = original.clone();
        reordered.items.sort_by_key(|item| {
            let packet = item.source.as_ref().expect("source");
            (packet.epoch.0 / 3, priorities[packet.ordinal % priorities.len()])
        });
        let expected = original.ingest_with_window(Duration::from_secs(120));
        let actual = reordered.ingest_with_window(Duration::from_secs(120));
        let normalize = |records: &[NmeaRecord]| {
            let mut fixes: Vec<_> = records.iter().filter_map(|record| {
                if let NmeaRecord::Navigation(fix) = record { Some(*fix) } else { None }
            }).collect();
            fixes.sort_by_key(|fix| (fix.time.effective(), fix.lat.as_degrees().to_bits()));
            fixes
        };
        let expected = normalize(&expected.records);
        let actual = normalize(&actual.records);
        let expected_count = original.surviving().filter(|packet| packet.navigation().is_some()).map(|packet| packet.epoch.0).collect::<BTreeSet<_>>().len();
        prop_assert_eq!(expected.len(), expected_count);
        prop_assert_eq!(actual, expected);
    }

    #[test]
    fn p5_degraded_gsv_reports_never_combine_source_cycles(
        model in RecordingModel::strategy(),
        degradation in Degradation::strategy(),
    ) {
        let input = model.degrade(&degradation);
        let completion = input.ingest();
        for (record, visible_len) in completion.records.iter().zip(&completion.emitted_at) {
            if let NmeaRecord::SatelliteObservation(report) = record {
                input.assert_observation(report, *visible_len)?;
            }
        }
        input.assert_roundtrip(&completion)?;
    }

    #[test]
    fn p6_isolated_timestamp_and_byte_corruption_preserve_later_coherent_epochs(
        case in NavigationCase::strategy(),
        jump in prop::sample::select(vec![50_i64, -50, 86_400, -86_400, 87_000, -87_000]),
        host_clock in any::<bool>(),
        noise in prop::collection::vec(any::<u8>(), 0..=128),
    ) {
        let model = RecordingModel::recovery(case, host_clock);
        let mut input = model.complete();
        let mut outlier = input.items.first().expect("first source").clone();
        let packet = outlier.source.as_mut().expect("source");
        packet.ordinal = model.complete().items.len();
        packet.case.receiver += TimeDelta::seconds(jump);
        packet.capture = packet.capture.map(|host| host + TimeDelta::milliseconds(100));
        outlier.bytes = packet.sentence().into_bytes();
        input.items.insert(1, outlier);
        let mut damaged = b"invalid-input:".to_vec();
        damaged.extend(noise.into_iter().filter(|byte| !matches!(byte, b'\r' | b'\n')));
        input.items.insert(2, InputItem { bytes: damaged, source: None });
        let completion = input.ingest();
        prop_assert!(completion.diagnostics.count(NmeaDiagnosticKind::TimestampDiscontinuity) > 0);
        input.assert_measurements(&completion)?;
        for epoch in 15..model.epoch_count {
            let receiver = model.case.receiver + TimeDelta::seconds(epoch as i64);
            prop_assert!(completion.records.iter().any(|record| {
                matches!(record, NmeaRecord::Navigation(fix) if fix.time.gps_time() == Some(receiver))
            }), "receiver epoch {receiver}");
        }
    }

    #[test]
    fn p7_navigation_conflicts_are_deterministic_and_methods_are_supported(
        case in NavigationCase::strategy(),
        solutions in prop::array::uniform4(ReportedSolution::strategy()),
        priorities in prop::array::uniform4(any::<u32>()),
    ) {
        let kinds = [NavigationSentence::Gga, NavigationSentence::Rmc, NavigationSentence::Gns, NavigationSentence::Gll];
        let mut packets: Vec<_> = kinds.into_iter().zip(solutions).enumerate().map(|(ordinal, (kind, solution))| {
            SourcePacket {
                epoch: EpochId(0), segment: SegmentId(0), ordinal,
                capture: Some(case.receiver + TimeDelta::days(3)), case: case.clone(),
                observation: Observation::Navigation { kind, solution },
            }
        }).collect();
        let valid = solutions.iter().any(|solution| solution.valid());
        let invalid = solutions.iter().any(|solution| !solution.valid());
        let expected_state = if valid && invalid { SolutionState::Unknown } else if valid { SolutionState::Fix } else { SolutionState::NoFix };
        let mut previous = None;
        for ordering in 0..3 {
            if ordering == 1 { packets.reverse(); }
            if ordering == 2 { packets.sort_by_key(|packet| priorities[packet.ordinal]); }
            let input = DegradedInput::from_packets(packets.clone());
            let completion = input.ingest();
            let [fix] = completion.records.iter().filter_map(|record| {
                if let NmeaRecord::Navigation(fix) = record { Some(*fix) } else { None }
            }).collect::<Vec<_>>().try_into().expect("one epoch");
            prop_assert_eq!(fix.state, Some(expected_state));
            if !valid || invalid { prop_assert_eq!(fix.method, None); prop_assert_eq!(fix.dimension, None); }
            if valid && invalid { prop_assert!(completion.diagnostics.count(NmeaDiagnosticKind::ConflictingNavigation) > 0); }
            input.assert_measurements(&completion)?;
            let normalized = (fix.time, fix.state, fix.method, fix.dimension,
                fix.lat.as_degrees().to_bits(), fix.lon.as_degrees().to_bits(), fix.speed, fix.heading);
            if let Some(previous) = previous { prop_assert_eq!(normalized, previous); }
            previous = Some(normalized);
            input.assert_roundtrip(&completion)?;
        }
    }

    #[test]
    fn corrupt_input_insertion_preserves_complete_reader_observations(
        mut model in RecordingModel::strategy(),
        noise in prop::collection::vec(any::<u8>(), 0..=256),
        separator in prop::sample::select(vec![b"\n".to_vec(), b"\r\n".to_vec(), b"\r".to_vec()]),
    ) {
        model.clock_event = ClockEvent::None;
        model.host_mode = HostMode::None;
        model.capture_delays.clear();
        let clean = model.complete();
        let mut noisy = Vec::new();
        let mut corruption = b"invalid-input:".to_vec();
        corruption.extend(noise.into_iter().filter(|byte| !matches!(byte, b'\r' | b'\n')));
        for item in &clean.items {
            noisy.extend(&item.bytes);
            noisy.extend(&separator);
            noisy.extend(&corruption);
            noisy.extend(&separator);
        }
        let result = NmeaIngestor::builder().navigation_window(Duration::from_secs(120)).build().expect("config")
            .ingest_reader(Cursor::new(noisy), NavFileBuilder::new()).expect("recording");
        let expected = clean.ingest_with_window(Duration::from_secs(120));
        let expected_fixes: Vec<_> = expected.records.iter().filter_map(|record| {
            if let NmeaRecord::Navigation(fix) = record { Some(*fix) } else { None }
        }).collect();
        let actual: Vec<_> = result.recording.nav_points().iter().map(|point| point.fix).collect();
        let normalize = |mut fix: NavFix| {
            let coordinates = (fix.lat.as_degrees().to_bits(), fix.lon.as_degrees().to_bits());
            fix.lat = Angle::degrees(0.0);
            fix.lon = Angle::degrees(0.0);
            (coordinates, fix)
        };
        prop_assert_eq!(actual.into_iter().map(normalize).collect::<Vec<_>>(), expected_fixes.into_iter().map(normalize).collect::<Vec<_>>());
        prop_assert_eq!(result.diagnostics.count(NmeaDiagnosticKind::UnsupportedSentence) + result.diagnostics.count(NmeaDiagnosticKind::InvalidText) + result.diagnostics.count(NmeaDiagnosticKind::MalformedSentence), clean.items.len() as u64);
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EpochId(usize);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SegmentId(u8);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct CycleId(usize);

#[derive(Clone, Copy, Debug)]
enum ClockEvent {
    ForwardDate,
    ForwardJump,
    Gap,
    None,
    Restart,
}

#[derive(Clone, Copy, Debug)]
enum HostMode {
    All,
    Mixed,
    None,
}

#[derive(Clone, Copy, Debug)]
enum NavigationSentence {
    Gga,
    Gll,
    Gns,
    Rmc,
}

#[derive(Clone, Copy, Debug)]
enum ReportedSolution {
    Autonomous,
    Differential,
    NoFix,
    RtkFixed,
    RtkFloat,
}

impl ReportedSolution {
    fn strategy() -> impl Strategy<Value = Self> {
        prop::sample::select(vec![
            Self::Autonomous,
            Self::Differential,
            Self::NoFix,
            Self::RtkFixed,
            Self::RtkFloat,
        ])
    }

    fn valid(self) -> bool {
        !matches!(self, Self::NoFix)
    }

    fn method(self) -> Option<SolutionMethod> {
        match self {
            Self::Autonomous => Some(SolutionMethod::Autonomous),
            Self::Differential => Some(SolutionMethod::Differential),
            Self::NoFix => None,
            Self::RtkFixed => Some(SolutionMethod::RtkFixed),
            Self::RtkFloat => Some(SolutionMethod::RtkFloat),
        }
    }

    fn quality(self) -> u8 {
        match self {
            Self::Autonomous => 1,
            Self::Differential => 2,
            Self::NoFix => 0,
            Self::RtkFixed => 4,
            Self::RtkFloat => 5,
        }
    }

    fn mode(self) -> char {
        match self {
            Self::Autonomous => 'A',
            Self::Differential => 'D',
            Self::NoFix => 'N',
            Self::RtkFixed => 'R',
            Self::RtkFloat => 'F',
        }
    }
}

#[derive(Clone, Debug)]
struct RecordingModel {
    case: NavigationCase,
    epoch_count: usize,
    periods: [usize; 7],
    host_mode: HostMode,
    host_offset: i64,
    capture_delays: Vec<u16>,
    clock_event: ClockEvent,
    solution: ReportedSolution,
}

impl RecordingModel {
    fn strategy() -> impl Strategy<Value = Self> {
        (
            NavigationCase::strategy(), 3_usize..=8, prop::array::uniform7(1_usize..=3),
            prop::sample::select(vec![HostMode::All, HostMode::Mixed, HostMode::None]),
            -172_800_i64..=172_800, prop::collection::vec(0_u16..=3_000, 0..=24),
            prop_oneof![8 => Just(ClockEvent::None), 1 => Just(ClockEvent::Gap), 1 => Just(ClockEvent::Restart), 1 => Just(ClockEvent::ForwardDate), 1 => Just(ClockEvent::ForwardJump)],
            ReportedSolution::strategy(),
        ).prop_map(|(case, epoch_count, periods, host_mode, host_offset, capture_delays, clock_event, solution)| Self {
            case, epoch_count, periods, host_mode, host_offset, capture_delays, clock_event, solution,
        })
    }

    fn recovery(case: NavigationCase, host_clock: bool) -> Self {
        Self {
            case,
            epoch_count: 26,
            periods: [1; 7],
            host_mode: if host_clock {
                HostMode::All
            } else {
                HostMode::None
            },
            host_offset: 86_400,
            capture_delays: Vec::new(),
            clock_event: ClockEvent::None,
            solution: ReportedSolution::RtkFixed,
        }
    }

    fn complete(&self) -> DegradedInput {
        let mut packets = Vec::new();
        for epoch in 0..self.epoch_count {
            let changed = epoch >= self.epoch_count / 2;
            let genuine_gap = matches!(self.clock_event, ClockEvent::Gap) && changed;
            let restarted = changed
                && matches!(
                    self.clock_event,
                    ClockEvent::Restart | ClockEvent::ForwardDate | ClockEvent::ForwardJump
                );
            let elapsed = epoch as i64 + if genuine_gap { 50 } else { 0 };
            let mut case = self.case.clone();
            let clock_shift = if changed {
                match self.clock_event {
                    ClockEvent::Restart => TimeDelta::days(-1),
                    ClockEvent::ForwardDate => TimeDelta::days(1),
                    ClockEvent::ForwardJump => TimeDelta::seconds(50),
                    ClockEvent::Gap | ClockEvent::None => TimeDelta::zero(),
                }
            } else {
                TimeDelta::zero()
            };
            case.receiver += TimeDelta::seconds(elapsed) + clock_shift;
            case.latitude_minute_units = self.case.latitude_minute_units / 2 + epoch as i32 * 101;
            case.longitude_minute_units = self.case.longitude_minute_units / 2 + epoch as i32 * 113;
            case.speed_hundredths = 100 + epoch as u16 * 31;
            case.course_hundredths = 1_000 + epoch as u16 * 113;
            case.used_count = if self.solution.valid() { 8 } else { 0 };
            let dilution_factor = self.case.hdop_tenths % 10 + 1;
            case.hdop_tenths = dilution_factor * 3;
            for observation in self.observations_for_epoch(epoch) {
                let ordinal = packets.len();
                let delay = self
                    .capture_delays
                    .get(ordinal % self.capture_delays.len().max(1))
                    .copied()
                    .unwrap_or(0);
                let capture = match self.host_mode {
                    HostMode::All => true,
                    HostMode::Mixed => ordinal % 3 != 0,
                    HostMode::None => false,
                }
                .then_some(
                    self.case.receiver
                        + TimeDelta::seconds(self.host_offset + elapsed)
                        + TimeDelta::milliseconds(i64::from(delay)),
                );
                packets.push(SourcePacket {
                    epoch: EpochId(epoch),
                    segment: SegmentId(u8::from(restarted)),
                    ordinal,
                    capture,
                    case: case.clone(),
                    observation,
                });
            }
        }
        let mut input = DegradedInput::from_packets(packets);
        input.periods = self.periods;
        input
    }

    fn observations_for_epoch(&self, epoch: usize) -> Vec<Observation> {
        let mut observations = Vec::new();
        for (kind, period) in [
            NavigationSentence::Rmc,
            NavigationSentence::Gga,
            NavigationSentence::Gns,
            NavigationSentence::Gll,
        ]
        .into_iter()
        .zip(self.periods)
        {
            if epoch.is_multiple_of(period) {
                observations.push(Observation::Navigation {
                    kind,
                    solution: self.solution,
                });
            }
        }
        if self
            .periods
            .get(4)
            .is_some_and(|period| epoch.is_multiple_of(*period))
        {
            observations.push(Observation::Velocity);
        }
        if self
            .periods
            .get(5)
            .is_some_and(|period| epoch.is_multiple_of(*period))
        {
            observations.extend(self.usage_updates(epoch));
        }
        if self
            .periods
            .get(6)
            .is_some_and(|period| epoch.is_multiple_of(*period))
        {
            observations.extend(self.fragment_updates(epoch));
        }
        observations
    }

    fn usage_updates(&self, epoch: usize) -> Vec<Observation> {
        let mut observations = Vec::new();
        let dilution_factor = self.case.hdop_tenths % 10 + 1;
        for constellation in [Constellation::Gps, Constellation::Galileo] {
            let identities: Vec<_> = if self.solution.valid() {
                (1..=4)
                    .map(|prn| SatelliteIdentity::new(constellation, prn + epoch as u32 % 5))
                    .collect()
            } else {
                Vec::new()
            };
            let lists: Vec<_> = if epoch.is_multiple_of(2) {
                vec![identities]
            } else {
                identities
                    .chunks(2)
                    .map(<[SatelliteIdentity]>::to_vec)
                    .collect()
            };
            for identities in lists {
                observations.push(Observation::Usage {
                    constellation,
                    identities,
                    valid: self.solution.valid(),
                    dilution: Dilution {
                        horizontal_tenths: dilution_factor * 3,
                        vertical_tenths: dilution_factor * 4,
                        position_tenths: dilution_factor * 5,
                    },
                });
            }
        }
        observations
    }

    fn fragment_updates(&self, epoch: usize) -> Vec<Observation> {
        let mut observations = Vec::new();
        for constellation in [Constellation::Gps, Constellation::Galileo] {
            let first_signal = if epoch.is_multiple_of(2) {
                Some(1)
            } else {
                None
            };
            for signal in
                [first_signal, Some(5)]
                    .into_iter()
                    .take(if epoch % 3 == 1 { 2 } else { 1 })
            {
                let total = if epoch.is_multiple_of(3) { 1 } else { 2 };
                for fragment in 0..total {
                    let measurements = (1..=4 / total)
                        .map(|offset| {
                            let marker = epoch + offset as usize + fragment as usize;
                            SatelliteObservation::builder()
                                .identity(SatelliteIdentity::new(
                                    constellation,
                                    epoch as u32 % 5 + fragment * (4 / total) + offset,
                                ))
                                .maybe_elevation(
                                    (!marker.is_multiple_of(4))
                                        .then_some((epoch * 7 + fragment as usize) as f32),
                                )
                                .maybe_azimuth(
                                    (!marker.is_multiple_of(5))
                                        .then_some((100 + epoch * 11 + fragment as usize) as f32),
                                )
                                .maybe_snr((!marker.is_multiple_of(3)).then_some(
                                    (20 + epoch * 3
                                        + fragment as usize
                                        + usize::from(signal.unwrap_or(0)))
                                        as f32,
                                ))
                                .build()
                        })
                        .collect();
                    observations.push(Observation::Fragment {
                        cycle: CycleId(epoch),
                        constellation,
                        number: fragment + 1,
                        total,
                        signal,
                        measurements,
                    });
                }
            }
        }
        observations
    }

    fn degrade(&self, degradation: &Degradation) -> DegradedInput {
        let mut input = self.complete();
        let mut damaged = Vec::new();
        for mut item in input.items {
            let Some(packet) = item.source.as_mut() else {
                damaged.push(item);
                continue;
            };
            let ordinal = packet.ordinal;
            let choice = degradation
                .choices
                .iter()
                .cycle()
                .nth(ordinal)
                .copied()
                .unwrap_or_default();
            if choice == 18
                || (degradation.delete_range.0
                    ..degradation
                        .delete_range
                        .0
                        .saturating_add(degradation.delete_range.1))
                    .contains(&ordinal)
            {
                continue;
            }
            match choice {
                19 => {
                    if let Some(last) = item.bytes.last_mut() {
                        *last = if *last == b'0' { b'1' } else { b'0' };
                    }
                    item.source = None;
                }
                20 => {
                    item.bytes.truncate(item.bytes.len() / 2);
                    item.source = None;
                }
                21 => {
                    item.bytes = super::super::sentence(
                        "GPRMC,120000.000,A,9960.0,N,99960.0,E,0,0,231026,,,A",
                    )
                    .into_bytes();
                    item.source = None;
                }
                27 => {
                    let invalid_time = ["1200NaN", "120000junk", "120000.1tail", "120060"]
                        .get(ordinal % 4)
                        .copied()
                        .unwrap_or("1200NaN");
                    item.bytes = super::super::sentence(&format!(
                        "GPRMC,{invalid_time},A,5500.0,N,01200.0,E,1,90,231026,,,A"
                    ))
                    .into_bytes();
                    item.source = None;
                }
                22 => {
                    item.bytes = b"invalid-input:".to_vec();
                    item.bytes.extend(
                        degradation
                            .bytes
                            .iter()
                            .copied()
                            .filter(|byte| !matches!(byte, b'\n' | b'\r')),
                    );
                    item.source = None;
                }
                23 => {
                    if matches!(
                        packet.observation,
                        Observation::Navigation {
                            kind: NavigationSentence::Rmc,
                            ..
                        }
                    ) {
                        packet.case.receiver += TimeDelta::seconds(degradation.jump);
                        item.bytes = packet.sentence().into_bytes();
                    }
                }
                24 => damaged.push(item.clone()),
                25 => damaged.push(InputItem {
                    bytes: b"$PAIR062,0,1*3F".to_vec(),
                    source: None,
                }),
                26 => {
                    if matches!(packet.observation, Observation::Fragment { .. }) {
                        continue;
                    }
                }
                _ => {}
            }
            damaged.push(item);
        }
        if degradation.reorder {
            damaged.sort_by_key(|item| {
                let index = item.source.as_ref().map_or(0, |packet| packet.ordinal);
                (
                    if degradation.unbounded { 0 } else { index / 16 },
                    degradation
                        .order
                        .iter()
                        .cycle()
                        .nth(index)
                        .copied()
                        .unwrap_or_default(),
                )
            });
        }
        input.items = damaged;
        input
    }
}

#[derive(Clone, Debug)]
struct Degradation {
    choices: Vec<u8>,
    delete_range: (usize, usize),
    bytes: Vec<u8>,
    order: Vec<u32>,
    reorder: bool,
    unbounded: bool,
    jump: i64,
}

impl Degradation {
    fn strategy() -> impl Strategy<Value = Self> {
        (
            prop::collection::vec(0_u8..=27, 1..=128),
            (0_usize..128, 0_usize..32),
            prop::collection::vec(any::<u8>(), 0..=128),
            prop::collection::vec(any::<u32>(), 1..=128),
            any::<bool>(),
            any::<bool>(),
            prop::sample::select(vec![50, -50, 86_400, -86_400, 87_000, -87_000]),
        )
            .prop_map(
                |(choices, delete_range, bytes, order, reorder, unbounded, jump)| Self {
                    choices,
                    delete_range,
                    bytes,
                    order,
                    reorder,
                    unbounded,
                    jump,
                },
            )
    }
}

#[derive(Clone, Debug)]
struct SourcePacket {
    epoch: EpochId,
    segment: SegmentId,
    ordinal: usize,
    capture: Option<DateTime<Utc>>,
    case: NavigationCase,
    observation: Observation,
}

impl SourcePacket {
    #[expect(
        clippy::expect_used,
        reason = "fixed oracle fixture has a valid timestamp"
    )]
    fn oracle_navigation(kind: NavigationSentence) -> Self {
        let receiver = "2026-10-23T12:00:00Z".parse().expect("timestamp");
        Self {
            epoch: EpochId(0),
            segment: SegmentId(0),
            ordinal: usize::from(matches!(kind, NavigationSentence::Rmc)),
            capture: Some(receiver),
            case: NavigationCase {
                latitude_minute_units: 33_000_000,
                longitude_minute_units: 7_200_000,
                speed_hundredths: 100,
                course_hundredths: 9_000,
                quality: super::FixQualityCase {
                    indicator: 1,
                    method: SolutionMethod::Autonomous,
                },
                used_count: 8,
                hdop_tenths: 10,
                receiver,
            },
            observation: Observation::Navigation {
                kind,
                solution: ReportedSolution::Autonomous,
            },
        }
    }

    fn has_velocity(&self) -> bool {
        matches!(
            self.observation,
            Observation::Velocity
                | Observation::Navigation {
                    kind: NavigationSentence::Rmc,
                    ..
                }
        )
    }

    fn sentence(&self) -> String {
        let case = &self.case;
        match &self.observation {
            Observation::Navigation { kind, solution } => {
                let status = if solution.valid() { 'A' } else { 'V' };
                let mode = solution.mode();
                let time = case.receiver.format("%H%M%S%.6f");
                let coordinates = case.coordinate_fields();
                let body = match kind {
                    NavigationSentence::Gga => format!(
                        "GPGGA,{time},{coordinates},{},{},{:.1},12.3,M,40.0,M,,",
                        solution.quality(),
                        case.used_count,
                        f64::from(case.hdop_tenths) / 10.0
                    ),
                    NavigationSentence::Gll => {
                        format!("GPGLL,{coordinates},{time},{status},{mode}")
                    }
                    NavigationSentence::Gns => format!(
                        "GNGNS,{time},{coordinates},{mode},{},{:.1},12.3,40.0,,",
                        case.used_count,
                        f64::from(case.hdop_tenths) / 10.0
                    ),
                    NavigationSentence::Rmc => format!(
                        "GPRMC,{time},{status},{coordinates},{:.2},{:.2},{},,,{mode}",
                        f64::from(case.speed_hundredths) / 100.0,
                        f64::from(case.course_hundredths) / 100.0,
                        case.receiver.format("%d%m%y")
                    ),
                };
                super::super::sentence(&body)
            }
            Observation::Velocity => super::super::sentence(&format!(
                "GPVTG,{:.2},T,,M,{:.2},N,{:.5},K,A",
                f64::from(case.course_hundredths) / 100.0,
                f64::from(case.speed_hundredths) / 100.0,
                f64::from(case.speed_hundredths) / 100.0 * 1.852
            )),
            Observation::Usage {
                constellation,
                identities,
                valid,
                dilution,
            } => {
                let mut slots: Vec<_> = identities
                    .iter()
                    .map(|identity| identity.prn.to_string())
                    .collect();
                slots.resize(12, String::new());
                let id = if *constellation == Constellation::Gps {
                    1
                } else {
                    3
                };
                super::super::sentence(&format!(
                    "GNGSA,A,{},{},{:.1},{:.1},{:.1},{id}",
                    if *valid { 3 } else { 1 },
                    slots.join(","),
                    f64::from(dilution.position_tenths) / 10.0,
                    f64::from(dilution.horizontal_tenths) / 10.0,
                    f64::from(dilution.vertical_tenths) / 10.0
                ))
            }
            Observation::Fragment {
                constellation,
                number,
                total,
                signal,
                measurements,
                ..
            } => {
                let talker = if *constellation == Constellation::Gps {
                    "GP"
                } else {
                    "GA"
                };
                let rows: Vec<_> = measurements
                    .iter()
                    .map(|measurement| {
                        format!(
                            "{},{},{},{}",
                            measurement.identity.prn,
                            measurement
                                .elevation
                                .map_or(String::new(), |value| value.to_string()),
                            measurement
                                .azimuth
                                .map_or(String::new(), |value| value.to_string()),
                            measurement
                                .snr
                                .map_or(String::new(), |value| value.to_string())
                        )
                    })
                    .collect();
                let signal = signal.map_or(String::new(), |signal| format!(",{signal}"));
                super::super::sentence(&format!(
                    "{talker}GSV,{total},{number},04,{}{signal}",
                    rows.join(",")
                ))
            }
        }
    }

    fn supports_observation(&self, report: &SatelliteObservationReport) -> bool {
        if let Observation::Fragment {
            constellation,
            measurements,
            ..
        } = &self.observation
        {
            matches!(&report.scope, SatelliteScope::Constellations(set) if set.as_slice() == [*constellation])
                && report
                    .satellites
                    .iter()
                    .all(|satellite| measurements.contains(satellite))
                && self
                    .capture
                    .is_some_and(|host| report.time == NavFixTime::Host(host))
        } else {
            false
        }
    }

    fn supports_usage(&self, report: &SatelliteUsageReport) -> bool {
        if let Observation::Usage {
            constellation,
            identities,
            valid,
            ..
        } = &self.observation
        {
            matches!(&report.scope, SatelliteScope::Constellations(set) if set.as_slice() == [*constellation])
                && self
                    .capture
                    .is_some_and(|host| report.time == NavFixTime::Host(host))
                && report.satellites.iter().all(|satellite| {
                    identities.contains(&satellite.identity)
                        && satellite.membership
                            == if *valid {
                                SatelliteMembership::Used
                            } else {
                                SatelliteMembership::Unknown
                            }
                })
        } else {
            false
        }
    }

    fn navigation(&self) -> Option<(NavigationSentence, ReportedSolution)> {
        if let Observation::Navigation { kind, solution } = self.observation {
            Some((kind, solution))
        } else {
            None
        }
    }
}

#[derive(Clone, Debug)]
enum Observation {
    Fragment {
        cycle: CycleId,
        constellation: Constellation,
        number: u32,
        total: u32,
        signal: Option<u8>,
        measurements: Vec<SatelliteObservation>,
    },
    Navigation {
        kind: NavigationSentence,
        solution: ReportedSolution,
    },
    Usage {
        constellation: Constellation,
        identities: Vec<SatelliteIdentity>,
        valid: bool,
        dilution: Dilution,
    },
    Velocity,
}

#[derive(Clone, Debug)]
struct InputItem {
    bytes: Vec<u8>,
    source: Option<SourcePacket>,
}

#[derive(Clone, Debug)]
struct DegradedInput {
    items: Vec<InputItem>,
    periods: [usize; 7],
}

#[derive(Debug)]
struct DateAnchorEvidence<'a> {
    index: usize,
    source: &'a SourcePacket,
    inferred: DateTime<Utc>,
}

impl DateAnchorEvidence<'_> {
    fn permits_association(&self, packet: &SourcePacket) -> bool {
        self.inferred
            .signed_duration_since(self.source.case.receiver)
            .abs()
            <= TimeDelta::seconds(120)
            && packet
                .capture
                .zip(self.source.capture)
                .is_none_or(|(capture, anchor_capture)| {
                    (self
                        .inferred
                        .signed_duration_since(self.source.case.receiver)
                        - capture.signed_duration_since(anchor_capture))
                    .abs()
                        <= TimeDelta::seconds(5)
                })
    }

    fn has_later_paired_confirmations(&self, candidates: &[&Self], packet: &SourcePacket) -> bool {
        let Some(conflicting_capture) = self.source.capture else {
            return false;
        };
        candidates.iter().any(|earlier| {
            earlier.source.segment == packet.segment
                && earlier
                    .source
                    .capture
                    .is_some_and(|capture| capture > conflicting_capture)
                && candidates.iter().any(|later| {
                    later.source.segment == packet.segment
                        && later.source.epoch != earlier.source.epoch
                        && later.source.case.receiver > earlier.source.case.receiver
                        && later
                            .source
                            .capture
                            .zip(earlier.source.capture)
                            .is_some_and(|(later, earlier)| later > earlier)
                        && earlier.has_paired_progression(later)
                })
        })
    }

    fn has_paired_progression(&self, other: &Self) -> bool {
        self.source
            .capture
            .zip(other.source.capture)
            .is_some_and(|(capture, other_capture)| {
                (self
                    .source
                    .case
                    .receiver
                    .signed_duration_since(other.source.case.receiver)
                    - capture.signed_duration_since(other_capture))
                .abs()
                    <= TimeDelta::seconds(5)
            })
    }
}

impl DegradedInput {
    fn from_packets(packets: Vec<SourcePacket>) -> Self {
        Self {
            periods: [1; 7],
            items: packets
                .into_iter()
                .map(|packet| {
                    let sentence = packet.sentence();
                    let parsed = nmea::parse_str(&sentence);
                    assert!(
                        parsed.is_ok(),
                        "valid generated source sentence: {parsed:?}"
                    );
                    InputItem {
                        bytes: sentence.into_bytes(),
                        source: Some(packet),
                    }
                })
                .collect(),
        }
    }

    fn surviving(&self) -> impl Iterator<Item = &SourcePacket> {
        self.items.iter().filter_map(|item| item.source.as_ref())
    }

    fn ingest(&self) -> TestRecording {
        self.ingest_with_window(Duration::from_secs(5))
    }

    #[expect(
        clippy::expect_used,
        reason = "test configuration uses valid explicit windows"
    )]
    fn ingest_with_window(&self, window: Duration) -> TestRecording {
        let mut builder = NmeaIngestor::builder().navigation_window(window);
        for (sentence, period) in [
            SentenceType::RMC,
            SentenceType::GGA,
            SentenceType::GNS,
            SentenceType::GLL,
            SentenceType::VTG,
            SentenceType::GSA,
            SentenceType::GSV,
        ]
        .into_iter()
        .zip(self.periods)
        {
            builder = builder.sentence_period(sentence, Duration::from_secs(period as u64));
        }
        let mut ingestor = builder.build().expect("configuration");
        let mut records = Vec::new();
        let mut emitted_at = Vec::new();
        for (input_index, item) in self.items.iter().enumerate() {
            match std::str::from_utf8(&item.bytes) {
                Ok(line) => {
                    records.extend(
                        if let Some(host) = item.source.as_ref().and_then(|packet| packet.capture) {
                            ingestor.push_line_at(line, host)
                        } else {
                            ingestor.push_line(line)
                        },
                    );
                }
                Err(_) => {
                    records.extend(ingestor.push_line("invalid-input:non-UTF8"));
                }
            }
            emitted_at.resize(records.len(), input_index + 1);
        }
        let mut completion = ingestor.finish();
        records.append(&mut completion.records);
        emitted_at.resize(records.len(), self.items.len());
        TestRecording {
            records,
            diagnostics: completion.diagnostics,
            emitted_at,
        }
    }

    fn supports_time(&self, fix: &NavFix, packet: &SourcePacket, visible_len: usize) -> bool {
        let host_valid = fix.time.sys_time().is_none_or(|host| {
            self.visible(visible_len).into_iter().any(|candidate| {
                candidate.epoch == packet.epoch
                    && candidate.navigation().is_some()
                    && candidate.capture == Some(host)
            })
        });
        let receiver_valid = fix.time.gps_time().is_none_or(|receiver| {
            if matches!(packet.navigation(), Some((NavigationSentence::Rmc, _))) {
                return packet.case.receiver == receiver;
            }
            if packet.case.receiver.time() != receiver.time() {
                return false;
            }
            self.supports_inferred_date(receiver, packet, visible_len)
        });
        host_valid && receiver_valid
    }

    fn date_evidence<'a>(
        &'a self,
        packet: &SourcePacket,
        visible_len: usize,
    ) -> Vec<DateAnchorEvidence<'a>> {
        let mut source_identities = BTreeSet::new();
        self.visible(visible_len)
            .into_iter()
            .enumerate()
            .filter_map(|(index, anchor)| {
                if !matches!(anchor.navigation(), Some((NavigationSentence::Rmc, _)))
                    || !source_identities.insert(anchor.ordinal)
                {
                    return None;
                }
                let date = anchor.case.receiver.date_naive();
                let inferred = [date.pred_opt(), Some(date), date.succ_opt()]
                    .into_iter()
                    .flatten()
                    .map(|date| date.and_time(packet.case.receiver.time()).and_utc())
                    .min_by_key(|candidate| {
                        candidate.signed_duration_since(anchor.case.receiver).abs()
                    })?;
                Some(DateAnchorEvidence {
                    index,
                    source: anchor,
                    inferred,
                })
            })
            .collect()
    }

    fn supports_inferred_date(
        &self,
        receiver: DateTime<Utc>,
        packet: &SourcePacket,
        visible_len: usize,
    ) -> bool {
        let evidence = self.date_evidence(packet, visible_len);
        let candidates: Vec<_> = evidence
            .iter()
            .filter(|anchor| anchor.inferred == receiver && anchor.permits_association(packet))
            .collect();
        if candidates.is_empty() {
            return false;
        }
        let known_date = evidence.iter().any(|anchor| {
            anchor.source.segment == packet.segment
                && anchor.source.case.receiver.date_naive() == packet.case.receiver.date_naive()
        });
        if known_date && receiver.date_naive() != packet.case.receiver.date_naive() {
            return false;
        }
        let latest_conflict = evidence
            .iter()
            .filter(|anchor| {
                anchor.inferred.date_naive() != receiver.date_naive()
                    && !candidates
                        .iter()
                        .any(|candidate| anchor.has_paired_progression(candidate))
                    && !anchor.has_later_paired_confirmations(&candidates, packet)
            })
            .map(|anchor| anchor.index)
            .max();
        let Some(latest_conflict) = latest_conflict else {
            return true;
        };
        let coherent_epochs: BTreeSet<_> = candidates
            .iter()
            .filter(|anchor| {
                anchor.source.segment == packet.segment && anchor.index > latest_conflict
            })
            .map(|anchor| anchor.source.epoch.0)
            .collect();
        coherent_epochs.len() >= 2
    }

    fn position_matches(fix: &NavFix, packet: &SourcePacket) -> bool {
        if packet.navigation().is_none() {
            return false;
        }
        if fix.lat.as_degrees().is_nan() && fix.lon.as_degrees().is_nan() {
            return packet
                .navigation()
                .is_some_and(|(_, solution)| !solution.valid());
        }
        (fix.lat.as_degrees() - f64::from(packet.case.latitude_minute_units) / 600_000.0).abs()
            < 1e-8
            && (fix.lon.as_degrees() - f64::from(packet.case.longitude_minute_units) / 600_000.0)
                .abs()
                < 1e-8
    }

    fn assert_timestamps(&self, recording: &TestRecording) -> TestCaseResult {
        for (record, visible_len) in recording.records.iter().zip(&recording.emitted_at) {
            match record {
                NmeaRecord::Navigation(fix) => {
                    if !self.visible(*visible_len).into_iter().any(|packet| {
                        Self::position_matches(fix, packet)
                            && self.supports_time(fix, packet, *visible_len)
                    }) {
                        let trace: Vec<_> = self
                            .items
                            .iter()
                            .take(*visible_len)
                            .filter_map(|item| item.source.as_ref())
                            .map(|packet| {
                                (
                                    packet.ordinal,
                                    packet.epoch.0,
                                    packet.segment.0,
                                    packet.navigation().map(|(kind, _)| kind),
                                    packet.case.receiver,
                                    packet.capture,
                                )
                            })
                            .collect();
                        return Err(TestCaseError::fail(format!(
                            "timestamp provenance {fix:?}; trace {trace:?}"
                        )));
                    }
                }
                NmeaRecord::SatelliteObservation(report) => {
                    prop_assert!(
                        self.visible(*visible_len)
                            .into_iter()
                            .any(|packet| packet.supports_observation(report)),
                        "independent host capture"
                    );
                }
                NmeaRecord::SatelliteUsage(report) => self.assert_usage(report, *visible_len)?,
                _ => return Err(TestCaseError::fail("Unexpected generated record")),
            }
        }
        Ok(())
    }

    fn assert_measurements(&self, recording: &TestRecording) -> TestCaseResult {
        let source_count = self
            .surviving()
            .map(|packet| packet.ordinal)
            .max()
            .map_or(0, |last| last + 1);
        let mut records = Vec::new();
        let mut facts: BTreeMap<NavigationFact, Vec<Vec<usize>>> = BTreeMap::new();
        for (record, visible_len) in recording.records.iter().zip(&recording.emitted_at) {
            match record {
                NmeaRecord::SatelliteObservation(report) => {
                    self.assert_observation(report, *visible_len)?
                }
                NmeaRecord::SatelliteUsage(report) => self.assert_usage(report, *visible_len)?,
                _ => {}
            }
            let visible = self.visible(*visible_len);
            let sources: Vec<_> = visible
                .iter()
                .copied()
                .filter(|packet| match record {
                    NmeaRecord::Navigation(fix) => {
                        Self::position_matches(fix, packet)
                            && self.supports_time(fix, packet, *visible_len)
                            && self.fix_fields_supported(fix, packet.epoch, *visible_len)
                    }
                    NmeaRecord::SatelliteObservation(report) => packet.supports_observation(report),
                    NmeaRecord::SatelliteUsage(report) => packet.supports_usage(report),
                    _ => false,
                })
                .collect();
            if let NmeaRecord::Navigation(fix) = record {
                let epochs: BTreeSet<_> = sources.iter().map(|source| source.epoch.0).collect();
                for fact in NavigationFact::iter().filter(|fact| fact.present(fix)) {
                    facts.entry(fact).or_default().push(
                        visible
                            .iter()
                            .filter(|packet| {
                                epochs.contains(&packet.epoch.0) && fact.supported_by(fix, packet)
                            })
                            .map(|packet| packet.ordinal)
                            .collect(),
                    );
                }
            }
            records.push(sources.iter().map(|packet| packet.ordinal).collect());
        }
        SourceAllocation::assert_supported(records, source_count).map_err(|error| {
            TestCaseError::fail(format!("{error}; records {:?}", recording.records))
        })?;
        for (fact, compatible) in facts {
            SourceAllocation::assert_supported(compatible, source_count).map_err(|error| {
                TestCaseError::fail(format!("Unsupported {fact:?} measurement: {error}"))
            })?;
        }
        Ok(())
    }

    fn visible(&self, visible_len: usize) -> Vec<&SourcePacket> {
        self.items
            .iter()
            .take(visible_len)
            .filter_map(|item| item.source.as_ref())
            .collect()
    }

    fn fix_fields_supported(&self, fix: &NavFix, epoch: EpochId, visible_len: usize) -> bool {
        fix.eph_m.is_none()
            && NavigationFact::iter()
                .filter(|fact| fact.present(fix))
                .all(|fact| {
                    self.visible(visible_len)
                        .iter()
                        .any(|packet| packet.epoch == epoch && fact.supported_by(fix, packet))
                })
    }

    fn assert_observation(
        &self,
        report: &SatelliteObservationReport,
        visible_len: usize,
    ) -> TestCaseResult {
        let compatible: Vec<_> = self
            .visible(visible_len)
            .into_iter()
            .filter(|packet| packet.supports_observation(report))
            .collect();
        prop_assert!(
            !compatible.is_empty(),
            "GSV report has mixed or invented source observations: {report:?}"
        );
        let cycles: BTreeSet<_> = compatible
            .iter()
            .filter_map(|packet| {
                if let Observation::Fragment { cycle, .. } = packet.observation {
                    Some(cycle.0)
                } else {
                    None
                }
            })
            .collect();
        if !report.satellites.is_empty() {
            prop_assert_eq!(cycles.len(), 1);
        }
        if report.coverage == SatelliteCoverage::Complete {
            prop_assert!(compatible.iter().any(|packet| matches!(&packet.observation, Observation::Fragment { total: 1, signal: None, measurements, .. } if *measurements == report.satellites)), "complete coverage needs one exhaustive source update");
        }
        if let Some(count) = report.reported_count {
            prop_assert_eq!(count, 4);
            prop_assert!(
                compatible.iter().any(|packet| matches!(
                    &packet.observation,
                    Observation::Fragment { signal: None, .. }
                )),
                "signal aggregate has the wrong scope"
            );
        }
        Ok(())
    }

    fn assert_usage(&self, report: &SatelliteUsageReport, visible_len: usize) -> TestCaseResult {
        prop_assert_eq!(report.coverage, SatelliteCoverage::Partial);
        prop_assert_eq!(report.reported_count, None);
        prop_assert!(
            self.visible(visible_len)
                .into_iter()
                .any(|packet| packet.supports_usage(report)),
            "GSA membership has no surviving source list: {report:?}"
        );
        Ok(())
    }

    fn assert_roundtrip(&self, observed: &TestRecording) -> TestCaseResult {
        if !observed
            .records
            .iter()
            .any(|record| matches!(record, NmeaRecord::Navigation(_)))
        {
            return Ok(());
        }
        let mut recorder = NavFileBuilder::new()
            .with_satellite_window(Duration::from_secs(1_000_000))
            .open();
        for record in &observed.records {
            recorder.add(record.clone());
        }
        let recording = recorder
            .finish()
            .map_err(|error| TestCaseError::fail(format!("Recording: {error}")))?;
        let mut bytes = Vec::new();
        recording
            .write(&mut bytes)
            .map_err(|error| TestCaseError::fail(format!("Serialize: {error}")))?;
        let recording = NavFile::read(Cursor::new(bytes))
            .map_err(|error| TestCaseError::fail(format!("Deserialize: {error}")))?;
        let mut restored = Vec::new();
        for point in recording.nav_points() {
            restored.push(NmeaRecord::Navigation(point.fix));
            for report in &point.satellite_evidence().usage {
                restored.push(NmeaRecord::SatelliteUsage(report.clone()));
            }
            for report in &point.satellite_evidence().observations {
                restored.push(NmeaRecord::SatelliteObservation(report.clone()));
            }
        }
        prop_assert_eq!(restored.len(), observed.records.len());
        let mut used = vec![false; observed.records.len()];
        let emitted_at = restored
            .iter()
            .map(|record| observed.source_prefix(record, &mut used))
            .collect::<Result<Vec<_>, _>>()?;
        let restored = TestRecording {
            records: restored,
            diagnostics: observed.diagnostics.clone(),
            emitted_at,
        };
        self.assert_measurements(&restored)?;
        self.assert_timestamps(&restored)?;
        Ok(())
    }
}

#[derive(Debug)]
struct TestRecording {
    records: Vec<NmeaRecord>,
    diagnostics: NmeaDiagnostics,
    emitted_at: Vec<usize>,
}

impl TestRecording {
    fn source_prefix(
        &self,
        record: &NmeaRecord,
        used: &mut [bool],
    ) -> Result<usize, TestCaseError> {
        let input_order = match record {
            NmeaRecord::SatelliteUsage(report) => {
                report.placement.map(|placement| placement.input_order)
            }
            NmeaRecord::SatelliteObservation(report) => {
                report.placement.map(|placement| placement.input_order)
            }
            _ => None,
        };
        let originals: Vec<_> = if let Some(input_order) = input_order {
            self.records
                .iter()
                .zip(&self.emitted_at)
                .enumerate()
                .filter(|(_, (record, _))| !matches!(record, NmeaRecord::Navigation(_)))
                .nth(
                    usize::try_from(input_order)
                        .map_err(|error| TestCaseError::fail(error.to_string()))?,
                )
                .into_iter()
                .collect()
        } else {
            self.records
                .iter()
                .zip(&self.emitted_at)
                .enumerate()
                .filter(|(index, _)| used.get(*index).is_some_and(|used| !used))
                .collect()
        };
        originals
            .into_iter()
            .find_map(|(index, (original, prefix))| {
                let used = used.get_mut(index)?;
                if *used {
                    return None;
                }
                let equal = match (original, record) {
                    (NmeaRecord::Navigation(left), NmeaRecord::Navigation(right)) => {
                        let mut left = *left;
                        let mut right = *right;
                        let coordinates = (
                            left.lat.as_degrees().to_bits(),
                            left.lon.as_degrees().to_bits(),
                        ) == (
                            right.lat.as_degrees().to_bits(),
                            right.lon.as_degrees().to_bits(),
                        );
                        left.lat = Angle::degrees(0.0);
                        left.lon = Angle::degrees(0.0);
                        right.lat = Angle::degrees(0.0);
                        right.lon = Angle::degrees(0.0);
                        coordinates && left == right
                    }
                    (
                        NmeaRecord::SatelliteObservation(left),
                        NmeaRecord::SatelliteObservation(right),
                    ) => {
                        let mut restored = right.clone();
                        restored.placement = left.placement;
                        *left == restored
                    }
                    (NmeaRecord::SatelliteUsage(left), NmeaRecord::SatelliteUsage(right)) => {
                        let mut restored = right.clone();
                        restored.placement = left.placement;
                        *left == restored
                    }
                    _ => original == record,
                };
                if equal {
                    *used = true;
                    Some(*prefix)
                } else {
                    None
                }
            })
            .ok_or_else(|| {
                TestCaseError::fail(format!("Roundtrip record without an original: {record:?}"))
            })
    }
}

#[derive(Clone, Copy, Debug, EnumIter, Eq, Ord, PartialEq, PartialOrd)]
enum NavigationFact {
    Course,
    Dimension,
    Hdop,
    Method,
    Pdop,
    Position,
    Speed,
    State,
    UsedCount,
    Vdop,
}

impl NavigationFact {
    fn present(self, fix: &NavFix) -> bool {
        match self {
            Self::Position => fix.lat.as_degrees().is_finite() || fix.lon.as_degrees().is_finite(),
            Self::State => fix
                .state
                .is_some_and(|state| state != SolutionState::Unknown),
            Self::Method => fix.method.is_some(),
            Self::Dimension => fix.dimension.is_some(),
            Self::Speed => fix.speed.is_some(),
            Self::Course => fix.heading.is_some(),
            Self::Hdop => fix.hdop.is_some(),
            Self::Pdop => fix.pdop.is_some(),
            Self::Vdop => fix.vdop.is_some(),
            Self::UsedCount => fix.used_count.is_some(),
        }
    }

    fn supported_by(self, fix: &NavFix, packet: &SourcePacket) -> bool {
        match self {
            Self::Position => DegradedInput::position_matches(fix, packet),
            Self::State => match fix.state {
                Some(SolutionState::Fix) => packet.navigation().is_some_and(|(_, solution)| solution.valid()),
                Some(SolutionState::NoFix) => packet.navigation().is_some_and(|(_, solution)| !solution.valid()),
                _ => false,
            },
            Self::Method => matches!(fix.state, Some(SolutionState::Fix | SolutionState::EstimatedOnly))
                && packet.navigation().is_some_and(|(_, solution)| solution.valid() && solution.method() == fix.method),
            Self::Dimension => fix.dimension == Some(SolutionDimension::ThreeD)
                && matches!(packet.observation, Observation::Usage { valid: true, .. }),
            Self::Speed => packet.has_velocity() && fix.speed.is_some_and(|value| {
                (value.as_knots() - f64::from(packet.case.speed_hundredths) / 100.0).abs() < 1e-4
            }),
            Self::Course => packet.has_velocity() && fix.heading.is_some_and(|value| {
                (value.as_degrees() - f64::from(packet.case.course_hundredths) / 100.0).abs() < 1e-3
            }),
            Self::Hdop => fix.hdop.is_some_and(|value| match &packet.observation {
                Observation::Navigation { kind: NavigationSentence::Gga | NavigationSentence::Gns, .. } =>
                    (value - f64::from(packet.case.hdop_tenths) / 10.0).abs() < 1e-5,
                Observation::Usage { dilution, .. } => (value - f64::from(dilution.horizontal_tenths) / 10.0).abs() < 1e-5,
                _ => false,
            }),
            Self::Pdop => fix.pdop.is_some_and(|value| matches!(&packet.observation,
                Observation::Usage { dilution, .. } if (value-f64::from(dilution.position_tenths)/10.0).abs() < 1e-5)),
            Self::Vdop => fix.vdop.is_some_and(|value| matches!(&packet.observation,
                Observation::Usage { dilution, .. } if (value-f64::from(dilution.vertical_tenths)/10.0).abs() < 1e-5)),
            Self::UsedCount => matches!(packet.observation,
                Observation::Navigation { kind: NavigationSentence::Gga | NavigationSentence::Gns, .. })
                && fix.used_count == Some(u32::from(packet.case.used_count)),
        }
    }
}

#[derive(Debug)]
struct SourceAllocation {
    compatible: Vec<Vec<usize>>,
    assigned: Vec<Option<usize>>,
}

impl SourceAllocation {
    fn assert_supported(compatible: Vec<Vec<usize>>, capacity: usize) -> TestCaseResult {
        let count = compatible.len();
        let mut allocation = Self {
            compatible,
            assigned: vec![None; capacity],
        };
        for index in 0..count {
            prop_assert!(
                allocation.assign(index, &mut vec![false; capacity]),
                "no unused surviving source measurement for output {index}"
            );
        }
        Ok(())
    }

    fn assign(&mut self, record: usize, visited: &mut [bool]) -> bool {
        let Some(compatible) = self.compatible.get(record).cloned() else {
            return false;
        };
        for source in compatible {
            let Some(visited_source) = visited.get_mut(source) else {
                return false;
            };
            if *visited_source {
                continue;
            }
            *visited_source = true;
            let Some(previous) = self.assigned.get(source).copied() else {
                return false;
            };
            if previous.is_none_or(|previous| self.assign(previous, visited))
                && let Some(assigned) = self.assigned.get_mut(source)
            {
                *assigned = Some(record);
                return true;
            }
        }
        false
    }
}

#[derive(Clone, Copy, Debug)]
struct Dilution {
    horizontal_tenths: u8,
    vertical_tenths: u8,
    position_tenths: u8,
}

#[test]
fn factual_oracle_rejects_repeated_velocity_and_retransmission_budgets() {
    let input = DegradedInput::from_packets(vec![
        SourcePacket::oracle_navigation(NavigationSentence::Gga),
        SourcePacket::oracle_navigation(NavigationSentence::Rmc),
    ]);
    let baseline = input.ingest();
    for retain_speed in [true, false] {
        let mut completion = TestRecording {
            records: baseline.records.clone(),
            diagnostics: baseline.diagnostics.clone(),
            emitted_at: baseline.emitted_at.clone(),
        };
        if let Some(NmeaRecord::Navigation(fix)) = completion.records.first_mut() {
            if retain_speed {
                fix.heading = None;
            } else {
                fix.speed = None;
            }
        }
        input
            .assert_measurements(&completion)
            .expect("complementary observations");
        let original = completion.records.first().expect("navigation").clone();
        completion.records.push(original);
        completion.emitted_at.push(input.items.len());
        assert!(input.assert_measurements(&completion).is_err());
        let mut retransmitted = input.clone();
        retransmitted
            .items
            .push(input.items.get(1).expect("RMC transmission").clone());
        assert!(retransmitted.assert_measurements(&completion).is_err());
    }
}

#[test]
fn factual_oracle_rejects_velocity_evidence_after_emission() {
    let input = DegradedInput::from_packets(vec![
        SourcePacket::oracle_navigation(NavigationSentence::Gga),
        SourcePacket::oracle_navigation(NavigationSentence::Rmc),
    ]);
    let mut completion = input.ingest();
    let NmeaRecord::Navigation(fix) = completion.records.first_mut().expect("fix") else {
        panic!("navigation")
    };
    fix.time = NavFixTime::Host(fix.time.sys_time().expect("capture"));
    assert!(input.fix_fields_supported(fix, EpochId(0), input.items.len()));
    assert!(!input.fix_fields_supported(fix, EpochId(0), 1));
    completion.emitted_at.fill(1);
    assert!(input.assert_measurements(&completion).is_err());
}

#[rstest]
#[case(SegmentId(0))]
#[case(SegmentId(2))]
fn timestamp_oracle_rejects_both_unresolved_dates_after_visible_conflicts(
    #[case] segment: SegmentId,
) {
    let mut old = SourcePacket::oracle_navigation(NavigationSentence::Rmc);
    old.capture = None;
    let mut conflicting = old.clone();
    conflicting.ordinal += 1;
    conflicting.epoch = EpochId(1);
    conflicting.case.receiver += TimeDelta::days(1) + TimeDelta::seconds(1);
    let mut resumed = SourcePacket::oracle_navigation(NavigationSentence::Gga);
    resumed.ordinal = 3;
    resumed.epoch = EpochId(2);
    resumed.segment = segment;
    resumed.capture = None;
    resumed.case.receiver += TimeDelta::seconds(2);
    let input = DegradedInput::from_packets(vec![old, conflicting, resumed.clone()]);
    for date_shift in [TimeDelta::zero(), TimeDelta::days(1)] {
        let fix = NavFix::builder()
            .time(NavFixTime::Receiver(resumed.case.receiver + date_shift))
            .lat(Angle::degrees(0.0))
            .lon(Angle::degrees(0.0))
            .build();
        assert!(!input.supports_time(&fix, &resumed, input.items.len()));
    }
}

#[test]
fn timestamp_oracle_allows_a_sole_anchor_when_a_hidden_restart_has_no_dated_evidence() {
    let mut old = SourcePacket::oracle_navigation(NavigationSentence::Rmc);
    old.capture = None;
    let mut resumed = SourcePacket::oracle_navigation(NavigationSentence::Gga);
    resumed.capture = None;
    resumed.segment = SegmentId(1);
    resumed.epoch = EpochId(1);
    resumed.case.receiver += TimeDelta::days(1) + TimeDelta::seconds(1);
    let input = DegradedInput::from_packets(vec![old, resumed.clone()]);
    let fix = NavFix::builder()
        .time(NavFixTime::Receiver(
            resumed.case.receiver - TimeDelta::days(1),
        ))
        .lat(Angle::degrees(0.0))
        .lon(Angle::degrees(0.0))
        .build();
    assert!(input.supports_time(&fix, &resumed, input.items.len()));
}

#[rstest]
#[case(0)]
#[case(600)]
fn timestamp_oracle_requires_new_date_confirmations_after_the_latest_conflict(
    #[case] time_jump_seconds: i64,
) {
    let packets: Vec<_> = (0..=5)
        .map(|index| {
            let kind = if index == 3 {
                NavigationSentence::Gga
            } else {
                NavigationSentence::Rmc
            };
            let mut packet = SourcePacket::oracle_navigation(kind);
            packet.ordinal = index;
            packet.epoch = EpochId(index);
            packet.capture = None;
            packet.case.receiver += TimeDelta::seconds(index as i64);
            if index == 2 {
                packet.case.receiver += TimeDelta::days(1) + TimeDelta::seconds(time_jump_seconds);
            }
            packet
        })
        .collect();
    let input = DegradedInput::from_packets(packets);
    let packet = input
        .items
        .get(3)
        .and_then(|item| item.source.as_ref())
        .expect("time-of-day source");
    let fix = NavFix::builder()
        .time(NavFixTime::Receiver(packet.case.receiver))
        .lat(Angle::degrees(55.0))
        .lon(Angle::degrees(12.0))
        .build();
    assert!(!input.supports_time(&fix, packet, 4));
    assert!(!input.supports_time(&fix, packet, 5));
    assert!(input.supports_time(&fix, packet, 6));
    let mut retransmitted = input.clone();
    retransmitted.items.truncate(4);
    retransmitted
        .items
        .extend(input.items.iter().take(2).cloned());
    assert!(!retransmitted.supports_time(&fix, packet, retransmitted.items.len()));
    let mut genuine_gap = input.clone();
    for item in &mut genuine_gap.items {
        if let Some(source) = &mut item.source {
            source.capture = Some(source.case.receiver + TimeDelta::days(7));
        }
    }
    let packet = genuine_gap
        .items
        .get(3)
        .and_then(|item| item.source.as_ref())
        .expect("source");
    assert!(genuine_gap.supports_time(&fix, packet, 4));
}

#[rstest]
#[case(Some(0), 2, true)]
#[case(Some(0), 1, false)]
#[case(Some(2), 2, false)]
#[case(Some(3), 2, false)]
#[case(None, 2, false)]
fn timestamp_oracle_requires_newer_paired_captures_for_delayed_date_conflicts(
    #[case] conflicting_capture_seconds: Option<i64>,
    #[case] confirmations: usize,
    #[case] expected: bool,
) {
    let base = SourcePacket::oracle_navigation(NavigationSentence::Rmc);
    let mut packets: Vec<_> = (1..=confirmations)
        .map(|index| {
            let mut packet = base.clone();
            packet.ordinal = index;
            packet.epoch = EpochId(index);
            packet.case.receiver += TimeDelta::seconds(index as i64);
            packet.capture = Some(packet.case.receiver);
            packet
        })
        .collect();
    let mut conflicting = base.clone();
    conflicting.ordinal = 3;
    conflicting.case.receiver += TimeDelta::days(1) + TimeDelta::minutes(10);
    conflicting.capture =
        conflicting_capture_seconds.map(|seconds| base.case.receiver + TimeDelta::seconds(seconds));
    packets.push(conflicting);
    let mut packet = SourcePacket::oracle_navigation(NavigationSentence::Gga);
    packet.ordinal = 4;
    packet.epoch = EpochId(3);
    packet.case.receiver += TimeDelta::seconds(3);
    packet.capture = Some(packet.case.receiver);
    packets.push(packet.clone());
    let input = DegradedInput::from_packets(packets);
    let fix = NavFix::builder()
        .time(NavFixTime::Both {
            gps: packet.case.receiver,
            sys: packet.capture.expect("host capture"),
        })
        .lat(Angle::degrees(55.0))
        .lon(Angle::degrees(12.0))
        .build();
    assert_eq!(
        input.supports_time(&fix, &packet, input.items.len()),
        expected
    );
}

#[test]
fn reordered_repeated_date_outliers_preserve_source_provenance() {
    let mut case = SourcePacket::oracle_navigation(NavigationSentence::Rmc).case;
    case.receiver = "2024-01-01T00:00:00Z".parse().expect("timestamp");
    let model = RecordingModel {
        case,
        epoch_count: 7,
        periods: [1, 2, 1, 1, 1, 1, 1],
        host_mode: HostMode::All,
        host_offset: 0,
        capture_delays: Vec::new(),
        clock_event: ClockEvent::None,
        solution: ReportedSolution::Autonomous,
    };
    let mut choices = vec![0; 74];
    choices[0] = 23;
    choices[61] = 23;
    let degradation = Degradation {
        choices,
        delete_range: (74, 0),
        bytes: Vec::new(),
        order: vec![
            3_568_040_292,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            974_256_170,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            1,
            0,
            0,
            0,
            0,
            0,
            1,
            0,
        ],
        reorder: true,
        unbounded: true,
        jump: 87_000,
    };
    let mut input = model.degrade(&degradation);
    input.items.retain(|item| {
        item.source
            .as_ref()
            .is_some_and(|packet| packet.navigation().is_some())
    });
    let completion = input.ingest();
    input
        .assert_measurements(&completion)
        .expect("surviving measurements");
    input
        .assert_timestamps(&completion)
        .expect("timestamp evidence");
    input.assert_roundtrip(&completion).expect("recording");
}
