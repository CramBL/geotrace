use std::collections::BTreeMap;

use nmea::SentenceType;
use nmea::sentences::gsa::GsaMode2;
use nmea::sentences::{GnssType, GsaData, GsvData, VtgData};

use crate::{
    Constellation, ConstellationSet, NavFixTime, SatelliteCoverage, SatelliteIdentity,
    SatelliteMembership, SatelliteObservation, SatelliteObservationReport, SatelliteScope,
    SatelliteUsage, SatelliteUsageReport,
};

use super::diagnostics::{NmeaDiagnostic, NmeaDiagnosticKind, NmeaDiagnostics};
use super::time::CaptureContext;
use super::{NmeaGsvCyclePolicy, NmeaRecord};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) struct GsvKey {
    pub talker: String,
    pub constellation: Constellation,
    pub signal: Option<u8>,
}

#[derive(Clone, Debug)]
pub(super) struct GsvGroup {
    pub key: GsvKey,
    pub first: CaptureContext,
    pub last: CaptureContext,
    pub number_of_sentences: u16,
    pub sats_in_view: u16,
    pub fragments: BTreeMap<u16, GsvData>,
    pub fragment_times: BTreeMap<u16, CaptureContext>,
    pub ambiguous_cycle: bool,
    pub navigation_association_lost: bool,
}

impl GsvGroup {
    pub fn constellation(gnss: GnssType) -> Constellation {
        match gnss {
            GnssType::Gps => Constellation::Gps,
            GnssType::Glonass => Constellation::Glonass,
            GnssType::Galileo => Constellation::Galileo,
            GnssType::Beidou => Constellation::Beidou,
            GnssType::Qzss => Constellation::Qzss,
            GnssType::NavIC => Constellation::Navic,
        }
    }

    pub fn new(key: GsvKey, time: CaptureContext, data: &GsvData) -> Self {
        Self {
            key,
            first: time,
            last: time,
            number_of_sentences: data.number_of_sentences,
            sats_in_view: data.sats_in_view,
            fragments: BTreeMap::new(),
            fragment_times: BTreeMap::new(),
            ambiguous_cycle: false,
            navigation_association_lost: false,
        }
    }

    pub fn into_pending(self, diagnostics: &mut NmeaDiagnostics) -> PendingReport {
        let complete = self.complete();
        if self.key.signal.is_some_and(|signal| signal != 0) {
            diagnostics.report(NmeaDiagnostic {
                kind: NmeaDiagnosticKind::SignalDimensionOmitted,
                line: self.first.line,
                sentence: Some(SentenceType::GSV),
                reason: "GSV signal identity is omitted from the satellite observation model"
                    .to_owned(),
            });
        }
        let mut observations: Vec<SatelliteObservation> = Vec::new();
        let mut identity_conflict = false;
        for satellite in self
            .fragments
            .values()
            .flat_map(|fragment| fragment.sats_info.iter().flatten())
        {
            let identity =
                SatelliteIdentity::new(Self::constellation(satellite.gnss_type()), satellite.prn());
            let observation = SatelliteObservation::builder()
                .identity(identity)
                .maybe_elevation(satellite.elevation())
                .maybe_azimuth(satellite.azimuth())
                .maybe_snr(satellite.snr())
                .build();
            if let Some(previous) = observations
                .iter_mut()
                .find(|previous| previous.identity == identity)
            {
                if *previous != observation {
                    identity_conflict = true;
                    diagnostics.report(NmeaDiagnostic {kind:NmeaDiagnosticKind::ConflictingGsvFragment,line:self.first.line,sentence:Some(SentenceType::GSV),reason:"Repeated satellite identity has conflicting measurements within one GSV group".to_owned()});
                }
                // A repeated identity is represented once, with the first fragment's measurements.
            } else {
                observations.push(observation);
            }
        }
        let identity_count_matches = observations.len() == usize::from(self.sats_in_view);
        if complete && !identity_count_matches {
            diagnostics.report(NmeaDiagnostic {kind:NmeaDiagnosticKind::UnresolvedSatelliteIdentity,line:self.first.line,sentence:Some(SentenceType::GSV),reason:"Completed GSV group has a distinct identity count that differs from its reported aggregate".to_owned()});
        }
        let all_signals = self.key.signal.is_none_or(|signal| signal == 0);
        let coverage = if complete
            && identity_count_matches
            && all_signals
            && !self.ambiguous_cycle
            && !identity_conflict
        {
            SatelliteCoverage::Complete
        } else {
            SatelliteCoverage::Partial
        };
        let reported_count = all_signals.then_some(u32::from(self.sats_in_view));
        PendingReport {
            navigation_association_lost: self.navigation_association_lost,
            time: self.first,
            last_sequence: self.last.sequence,
            kind: PendingKind::Observation {
                constellation: self.key.constellation,
                coverage,
                reported_count,
                satellites: observations,
            },
        }
    }

    pub fn complete(&self) -> bool {
        self.fragments.len() == usize::from(self.number_of_sentences)
            && (1..=self.number_of_sentences).all(|index| self.fragments.contains_key(&index))
    }

    pub fn into_pending_reports(
        self,
        cycle_policy: NmeaGsvCyclePolicy,
        diagnostics: &mut NmeaDiagnostics,
    ) -> Vec<PendingReport> {
        if !self.complete() {
            diagnostics.report(NmeaDiagnostic {
                kind: NmeaDiagnosticKind::IncompleteGsv,
                line: self.first.line,
                sentence: Some(SentenceType::GSV),
                reason: format!(
                    "Received {} of {} GSV fragments",
                    self.fragments.len(),
                    self.number_of_sentences
                ),
            });
        }
        let same_capture = cycle_policy == NmeaGsvCyclePolicy::UniqueHostTimestamp
            && self.first.host.is_some()
            && self
                .fragment_times
                .values()
                .all(|time| time.host == self.first.host);
        if self.fragments.len() <= 1 || same_capture {
            return vec![self.into_pending(diagnostics)];
        }
        diagnostics.report(NmeaDiagnostic {
            kind: NmeaDiagnosticKind::AmbiguousGsvCycle,
            line: self.first.line,
            sentence: Some(SentenceType::GSV),
            reason: "Multipart GSV fragments have no established common capture identity"
                .to_owned(),
        });
        self.fragments
            .iter()
            .map(|(number, fragment)| {
                let mut sample = self.clone();
                sample.fragments = BTreeMap::from([(*number, fragment.clone())]);
                if let Some(time) = self.fragment_times.get(number) {
                    sample.first = *time;
                    sample.last = *time;
                }
                sample.ambiguous_cycle = true;
                sample.into_pending(diagnostics)
            })
            .collect()
    }
}

#[derive(Debug)]
pub(super) enum PendingKind {
    Observation {
        constellation: Constellation,
        coverage: SatelliteCoverage,
        reported_count: Option<u32>,
        satellites: Vec<SatelliteObservation>,
    },
    Usage {
        data: Box<GsaData>,
        constellation: Option<Constellation>,
    },
    Velocity(VtgData),
}

#[derive(Debug)]
pub(super) struct PendingReport {
    pub navigation_association_lost: bool,
    pub time: CaptureContext,
    pub last_sequence: u64,
    pub kind: PendingKind,
}

impl PendingReport {
    pub fn gsa(data: GsaData, time: CaptureContext, diagnostics: &mut NmeaDiagnostics) -> Self {
        if data.mode2 == GsaMode2::NoFix && data.fix_sats_prn.iter().any(|prn| *prn != 0) {
            diagnostics.report(NmeaDiagnostic {
                kind: NmeaDiagnosticKind::ConflictingNavigation,
                line: time.line,
                sentence: Some(SentenceType::GSA),
                reason: "GSA reports satellite identities while navigation is unavailable"
                    .to_owned(),
            });
        }
        let constellation = data
            .system_id
            .map(GnssType::from)
            .map(GsvGroup::constellation);
        if constellation.is_none() {
            diagnostics.report(NmeaDiagnostic {
                kind: NmeaDiagnosticKind::UnresolvedSatelliteIdentity,
                line: time.line,
                sentence: Some(SentenceType::GSA),
                reason: "GSA constellation is ambiguous or conflicts with its System ID".to_owned(),
            });
        }
        Self {
            navigation_association_lost: false,
            time,
            last_sequence: time.sequence,
            kind: PendingKind::Usage {
                data: Box::new(data),
                constellation,
            },
        }
    }

    pub fn into_record(self, time: NavFixTime) -> Option<NmeaRecord> {
        match self.kind {
            PendingKind::Usage {
                data,
                constellation,
            } => {
                let constellation = constellation?;
                let mut identities: Vec<_> = data
                    .fix_sats_prn
                    .into_iter()
                    .filter(|prn| *prn != 0)
                    .collect();
                identities.sort_unstable();
                identities.dedup();
                let membership = if data.mode2 == GsaMode2::NoFix {
                    SatelliteMembership::Unknown
                } else {
                    SatelliteMembership::Used
                };
                let satellites = identities
                    .into_iter()
                    .map(|prn| {
                        SatelliteUsage::new(SatelliteIdentity::new(constellation, prn), membership)
                    })
                    .collect::<Vec<_>>();
                Some(NmeaRecord::SatelliteUsage(
                    SatelliteUsageReport::builder()
                        .time(time)
                        .scope(SatelliteScope::Constellations(ConstellationSet::new(
                            constellation,
                            [],
                        )))
                        .coverage(SatelliteCoverage::Partial)
                        .satellites(satellites)
                        .build(),
                ))
            }
            PendingKind::Observation {
                constellation,
                coverage,
                reported_count,
                satellites,
            } => Some(NmeaRecord::SatelliteObservation(
                SatelliteObservationReport::builder()
                    .time(time)
                    .scope(SatelliteScope::Constellations(ConstellationSet::new(
                        constellation,
                        [],
                    )))
                    .coverage(coverage)
                    .maybe_reported_count(reported_count)
                    .satellites(satellites)
                    .build(),
            )),
            PendingKind::Velocity(_) => None,
        }
    }

    pub fn sentence(&self) -> SentenceType {
        match self.kind {
            PendingKind::Usage { .. } => SentenceType::GSA,
            PendingKind::Observation { .. } => SentenceType::GSV,
            PendingKind::Velocity(_) => SentenceType::VTG,
        }
    }
}
