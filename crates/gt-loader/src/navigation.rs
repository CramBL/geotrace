use std::fmt;

use chrono::Duration;
use geotrace_sdk::{
    NavFix, NavPoint as SdkNavPoint, PositionProvenance as SdkPositionProvenance,
    SatelliteCoverage as SdkSatelliteCoverage, SatelliteMembership as SdkSatelliteMembership,
    SatelliteObservationReport as SdkObservationReport, SatelliteReportPlacement,
    SatelliteScope as SdkSatelliteScope, SatelliteUsageReport as SdkUsageReport,
    SolutionDimension as SdkSolutionDimension, SolutionMethod as SdkSolutionMethod,
    SolutionState as SdkSolutionState,
};
use gt_types::satellites::{ConstellationSet, SatSample};
use gt_types::{
    AlterationWording, GpsTime, LoadWarning, NavigationMetadata, PositionProvenance, Prn,
    SatelliteCoverage, SatelliteEvidence, SatelliteIdentity, SatelliteMembership,
    SatelliteObservation, SatelliteObservationReport, SatelliteReportMetadata, SatelliteScope,
    SatelliteUsage, SatelliteUsageReport, Snr, SolutionDimension, SolutionMethod, SolutionState,
};

use super::{LoadError, SatelliteInRecord};

#[derive(Default)]
pub(super) struct EvidenceAlterations {
    repeated: Vec<RepeatedEvidenceRows>,
}

impl EvidenceAlterations {
    pub(super) fn warning(&self) -> Option<LoadWarning> {
        MERGED_EVIDENCE_ROWS.load_warning(&self.repeated)
    }
}

pub(super) fn convert_navigation(fix: &NavFix) -> NavigationMetadata {
    NavigationMetadata {
        state: fix.state.map(|state| match state {
            SdkSolutionState::Fix => SolutionState::Fix,
            SdkSolutionState::NoFix => SolutionState::NoFix,
            SdkSolutionState::EstimatedOnly => SolutionState::EstimatedOnly,
            _ => SolutionState::Unknown,
        }),
        method: fix.method.map(|method| match method {
            SdkSolutionMethod::Autonomous => SolutionMethod::Autonomous,
            SdkSolutionMethod::Differential => SolutionMethod::Differential,
            SdkSolutionMethod::RtkFloat => SolutionMethod::RtkFloat,
            SdkSolutionMethod::RtkFixed => SolutionMethod::RtkFixed,
            SdkSolutionMethod::DeadReckoning => SolutionMethod::DeadReckoning,
            SdkSolutionMethod::Hybrid => SolutionMethod::Hybrid,
            _ => SolutionMethod::Other,
        }),
        dimension: fix.dimension.and_then(|dimension| match dimension {
            SdkSolutionDimension::TwoD => Some(SolutionDimension::TwoD),
            SdkSolutionDimension::ThreeD => Some(SolutionDimension::ThreeD),
            _ => None,
        }),
        position_provenance: fix
            .position_provenance
            .and_then(|provenance| match provenance {
                SdkPositionProvenance::SourceProvided => Some(PositionProvenance::SourceProvided),
                SdkPositionProvenance::SdkSynthesized => Some(PositionProvenance::SdkSynthesized),
                _ => None,
            }),
        used_count: fix.used_count,
        hdop: fix.hdop,
        vdop: fix.vdop,
        pdop: fix.pdop,
    }
}

pub(super) fn convert_evidence(
    point: &SdkNavPoint,
    record: usize,
    alterations: &mut EvidenceAlterations,
) -> Result<SatelliteEvidence, LoadError> {
    let conversion = ReportConversion { point, record };
    Ok(SatelliteEvidence {
        usage: point
            .satellite_evidence()
            .usage
            .iter()
            .enumerate()
            .map(|(sample, report)| conversion.usage(report, sample, alterations))
            .collect::<Result<_, _>>()?,
        observations: point
            .satellite_evidence()
            .observations
            .iter()
            .enumerate()
            .map(|(sample, report)| conversion.observations(report, sample, alterations))
            .collect::<Result<_, _>>()?,
    })
}

struct ReportConversion<'a> {
    point: &'a SdkNavPoint,
    record: usize,
}

impl ReportConversion<'_> {
    fn metadata(
        &self,
        metadata: ReportMetadataInput<'_>,
    ) -> Result<SatelliteReportMetadata, LoadError> {
        let normalized_time = if let Some(placement) = metadata.placement {
            placement.normalized_time
        } else if let Some(receiver) = metadata.time.gps_time() {
            receiver
        } else {
            let clock_delta = self
                .point
                .fix
                .time
                .gps_time()
                .zip(self.point.fix.time.sys_time())
                .map_or(Duration::zero(), |(gps, host)| gps - host);
            metadata
                .time
                .effective()
                .checked_add_signed(clock_delta)
                .ok_or(LoadError::ReportTimeOutOfRange {
                    record: self.record,
                    kind: metadata.sample_kind,
                    sample: metadata.sample,
                })?
        };
        Ok(SatelliteReportMetadata {
            gps_time: metadata.time.gps_time().map(GpsTime::from_utc),
            sys_time: metadata.time.sys_time().map(gt_types::SysTime::from_utc),
            normalized_time: GpsTime::from_utc(normalized_time),
            input_order: metadata
                .placement
                .map_or(metadata.sample as u64, |placement| placement.input_order),
            scope: match metadata.scope {
                SdkSatelliteScope::Global => SatelliteScope::Global,
                SdkSatelliteScope::Constellations(set) => SatelliteScope::Constellations(
                    set.as_slice()
                        .iter()
                        .map(|constellation| super::convert_constellation(*constellation))
                        .collect::<ConstellationSet>(),
                ),
                _ => SatelliteScope::Constellations(ConstellationSet::empty()),
            },
            coverage: match metadata.coverage {
                SdkSatelliteCoverage::Complete => SatelliteCoverage::Complete,
                _ => SatelliteCoverage::Partial,
            },
            reported_count: metadata.reported_count,
        })
    }

    fn usage(
        &self,
        report: &SdkUsageReport,
        sample: usize,
        alterations: &mut EvidenceAlterations,
    ) -> Result<SatelliteUsageReport, LoadError> {
        let mut satellites: Vec<SatelliteUsage> = Vec::new();
        let mut repeated = Vec::new();
        for row in &report.satellites {
            let identity = SatelliteIdentity {
                constellation: super::convert_constellation(row.identity.constellation),
                prn: Prn::new(row.identity.prn),
            };
            let membership = match row.membership {
                SdkSatelliteMembership::Used => SatelliteMembership::Used,
                SdkSatelliteMembership::Unused => SatelliteMembership::Unused,
                _ => SatelliteMembership::Unknown,
            };
            if let Some(satellite) = satellites
                .iter_mut()
                .find(|satellite| satellite.identity == identity)
            {
                satellite.membership = satellite.membership.max(membership);
                self.record_repeat(
                    &mut repeated,
                    identity,
                    SampleIndex {
                        kind: "usage",
                        index: sample,
                    },
                );
            } else {
                satellites.push(SatelliteUsage {
                    identity,
                    membership,
                });
            }
        }
        self.record_repeated_rows(repeated, alterations);
        Ok(SatelliteUsageReport {
            metadata: self.metadata(ReportMetadataInput {
                time: report.time,
                placement: report.placement,
                scope: &report.scope,
                coverage: report.coverage,
                reported_count: report.reported_count,
                sample,
                sample_kind: "usage",
            })?,
            satellites,
        })
    }

    fn observations(
        &self,
        report: &SdkObservationReport,
        sample: usize,
        alterations: &mut EvidenceAlterations,
    ) -> Result<SatelliteObservationReport, LoadError> {
        let mut satellites: Vec<SatelliteObservation> = Vec::new();
        let mut repeated = Vec::new();
        for row in &report.satellites {
            let identity = SatelliteIdentity {
                constellation: super::convert_constellation(row.identity.constellation),
                prn: Prn::new(row.identity.prn),
            };
            let converted = SatelliteObservation {
                identity,
                measurements: SatSample {
                    elevation: row.elevation,
                    azimuth: row.azimuth,
                    snr: row.snr.map(Snr::new),
                },
            };
            if let Some(satellite) = satellites
                .iter_mut()
                .find(|satellite| satellite.identity == identity)
            {
                satellite.absorb_repeated_row(converted);
                self.record_repeat(
                    &mut repeated,
                    identity,
                    SampleIndex {
                        kind: "observation",
                        index: sample,
                    },
                );
            } else {
                satellites.push(converted);
            }
        }
        self.record_repeated_rows(repeated, alterations);
        Ok(SatelliteObservationReport {
            metadata: self.metadata(ReportMetadataInput {
                time: report.time,
                placement: report.placement,
                scope: &report.scope,
                coverage: report.coverage,
                reported_count: report.reported_count,
                sample,
                sample_kind: "observation",
            })?,
            satellites,
        })
    }

    fn record_repeat(
        &self,
        repeated: &mut Vec<RepeatedEvidenceRows>,
        identity: SatelliteIdentity,
        sample: SampleIndex,
    ) {
        if let Some(entry) = repeated.iter_mut().find(|entry| {
            entry.satellite.constellation == identity.constellation
                && entry.satellite.prn == identity.prn.value()
        }) {
            entry.rows += 1;
        } else {
            repeated.push(RepeatedEvidenceRows {
                satellite: SatelliteInRecord {
                    record: self.record,
                    constellation: identity.constellation,
                    prn: identity.prn.value(),
                },
                sample_kind: sample.kind,
                sample: sample.index,
                rows: 2,
            });
        }
    }

    fn record_repeated_rows(
        &self,
        repeated: Vec<RepeatedEvidenceRows>,
        alterations: &mut EvidenceAlterations,
    ) {
        alterations
            .repeated
            .extend(repeated.into_iter().filter(|entry| {
                let projected_rows = self.point.satellites().as_ref().map_or(0, |report| {
                    report
                        .tracked
                        .iter()
                        .filter(|row| {
                            super::convert_constellation(row.constellation)
                                == entry.satellite.constellation
                                && row.prn == entry.satellite.prn
                        })
                        .count()
                });
                projected_rows != entry.rows
            }));
    }
}

#[derive(Clone, Copy)]
struct SampleIndex {
    kind: &'static str,
    index: usize,
}

#[derive(Clone, Copy)]
struct ReportMetadataInput<'a> {
    time: geotrace_sdk::NavFixTime,
    placement: Option<SatelliteReportPlacement>,
    scope: &'a SdkSatelliteScope,
    coverage: SdkSatelliteCoverage,
    reported_count: Option<u32>,
    sample: usize,
    sample_kind: &'static str,
}

struct RepeatedEvidenceRows {
    satellite: SatelliteInRecord,
    sample_kind: &'static str,
    sample: usize,
    rows: usize,
}

impl fmt::Display for RepeatedEvidenceRows {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "{}, {} sample {} on {} rows",
            self.satellite, self.sample_kind, self.sample, self.rows
        )
    }
}

const MERGED_EVIDENCE_ROWS: AlterationWording = AlterationWording {
    issue: "satellite(s) merged from repeated evidence rows",
    consequence: "Used membership takes precedence over unused and unknown membership. The loader preserves the first elevation and azimuth and the highest measured SNR within each sample.",
};
