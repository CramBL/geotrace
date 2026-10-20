use std::cmp::Ordering;

use rustc_hash::{FxHashMap, FxHashSet};
use strum::IntoEnumIterator as _;

use crate::satellites::{Constellation, ConstellationSet, Prn, SatSample, Satellites, Snr};
use crate::time_types::{GpsTime, SysTime};
use crate::tpv::TimePositionVelocity;

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub struct SatelliteIdentity {
    pub constellation: Constellation,
    pub prn: Prn,
}

/// Declaration order defines repeated-row membership precedence.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum SatelliteMembership {
    Unknown,
    Unused,
    Used,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SatelliteCoverage {
    Complete,
    Partial,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SatelliteScope {
    Constellations(ConstellationSet),
    Global,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SatelliteQueryScope {
    Constellation(Constellation),
    Global,
}

impl SatelliteQueryScope {
    pub fn includes(self, identity: SatelliteIdentity) -> bool {
        match self {
            Self::Global => true,
            Self::Constellation(constellation) => identity.constellation == constellation,
        }
    }
}

impl SatelliteScope {
    pub fn covers(self, scope: SatelliteQueryScope) -> bool {
        match (self, scope) {
            (Self::Global, _) => true,
            (Self::Constellations(set), SatelliteQueryScope::Constellation(constellation)) => {
                set.contains(constellation)
            }
            (Self::Constellations(_), SatelliteQueryScope::Global) => false,
        }
    }

    pub fn matches_aggregate(self, scope: SatelliteQueryScope) -> bool {
        match (self, scope) {
            (Self::Global, SatelliteQueryScope::Global) => true,
            (Self::Constellations(set), SatelliteQueryScope::Constellation(constellation)) => {
                set == ConstellationSet::single(constellation)
            }
            _ => false,
        }
    }

    fn specificity(self) -> usize {
        match self {
            Self::Global => usize::MAX,
            Self::Constellations(set) => Constellation::iter()
                .filter(|constellation| set.contains(*constellation))
                .count(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SatelliteReportMetadata {
    pub gps_time: Option<GpsTime>,
    pub sys_time: Option<SysTime>,
    /// Report time on the common track clock, independent of the associated fix time.
    pub normalized_time: GpsTime,
    /// Recorded insertion order, or collection order without association metadata.
    pub input_order: u64,
    pub scope: SatelliteScope,
    pub coverage: SatelliteCoverage,
    pub reported_count: Option<u32>,
}

impl SatelliteReportMetadata {
    pub fn compare_for_fix(&self, other: &Self, fix_time: GpsTime) -> Ordering {
        (self.normalized_time - fix_time)
            .abs()
            .cmp(&(other.normalized_time - fix_time).abs())
            .then_with(|| self.scope.specificity().cmp(&other.scope.specificity()))
            .then_with(|| self.normalized_time.cmp(&other.normalized_time))
            .then_with(|| self.input_order.cmp(&other.input_order))
    }

    pub fn complete_for(&self, scope: SatelliteQueryScope) -> bool {
        self.coverage == SatelliteCoverage::Complete && self.scope.covers(scope)
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SatelliteUsage {
    pub identity: SatelliteIdentity,
    pub membership: SatelliteMembership,
}

#[derive(Clone, Debug, PartialEq)]
pub struct SatelliteUsageReport {
    pub metadata: SatelliteReportMetadata,
    pub satellites: Vec<SatelliteUsage>,
}

impl SatelliteUsageReport {
    pub fn membership(&self, identity: SatelliteIdentity) -> SatelliteMembership {
        if !self
            .metadata
            .scope
            .covers(SatelliteQueryScope::Constellation(identity.constellation))
        {
            return SatelliteMembership::Unknown;
        }
        self.satellites
            .iter()
            .filter(|satellite| satellite.identity == identity)
            .map(|satellite| satellite.membership)
            .max()
            .unwrap_or(if self.metadata.coverage == SatelliteCoverage::Complete {
                SatelliteMembership::Unused
            } else {
                SatelliteMembership::Unknown
            })
    }

    pub fn list_used_count(&self, scope: SatelliteQueryScope) -> Option<u32> {
        if !self.metadata.complete_for(scope) {
            return None;
        }
        let mut memberships = FxHashMap::default();
        for satellite in self
            .satellites
            .iter()
            .filter(|satellite| scope.includes(satellite.identity))
        {
            memberships
                .entry(satellite.identity)
                .and_modify(|membership: &mut SatelliteMembership| {
                    *membership = (*membership).max(satellite.membership)
                })
                .or_insert(satellite.membership);
        }
        if memberships
            .values()
            .any(|membership| *membership == SatelliteMembership::Unknown)
        {
            return None;
        }
        u32::try_from(
            memberships
                .values()
                .filter(|membership| **membership == SatelliteMembership::Used)
                .count(),
        )
        .ok()
    }

    pub fn has_known_used(&self) -> bool {
        self.satellites
            .iter()
            .any(|satellite| satellite.membership == SatelliteMembership::Used)
    }
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SatelliteObservation {
    pub identity: SatelliteIdentity,
    pub measurements: SatSample,
}

impl SatelliteObservation {
    pub fn absorb_repeated_row(&mut self, row: Self) {
        self.measurements.absorb_repeated_row(row.measurements);
    }

    pub fn measured_snr(&self) -> Option<Snr> {
        self.measurements.measured_snr()
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct SatelliteObservationReport {
    pub metadata: SatelliteReportMetadata,
    pub satellites: Vec<SatelliteObservation>,
}

impl SatelliteObservationReport {
    pub fn list_seen_count(&self, scope: SatelliteQueryScope) -> Option<u32> {
        if !self.metadata.complete_for(scope) {
            return None;
        }
        let identities: FxHashSet<_> = self
            .satellites
            .iter()
            .map(|satellite| satellite.identity)
            .filter(|identity| scope.includes(*identity))
            .collect();
        u32::try_from(identities.len()).ok()
    }

    pub fn has_azimuth(&self) -> bool {
        self.satellites
            .iter()
            .any(|satellite| satellite.measurements.azimuth.is_some())
    }

    pub fn has_sky_position(&self) -> bool {
        self.satellites.iter().any(|satellite| {
            satellite.measurements.azimuth.is_some() && satellite.measurements.elevation.is_some()
        })
    }
}

#[derive(Clone, Debug, Default)]
pub struct SatelliteEvidence {
    pub usage: Vec<SatelliteUsageReport>,
    pub observations: Vec<SatelliteObservationReport>,
}

impl SatelliteEvidence {
    pub fn from_legacy(report: &Satellites, fix: &TimePositionVelocity) -> Self {
        let normalized_time = match (report.gps_time(), report.sys_time(), fix.sys_time()) {
            (Some(receiver), _, _) => receiver,
            (None, Some(host), Some(fix_host)) => GpsTime::from_utc(
                fix.time()
                    .utc()
                    .checked_add_signed(host - fix_host)
                    .unwrap_or(host.utc()),
            ),
            (None, Some(host), None) => GpsTime::from_utc(host.utc()),
            (None, None, _) => fix.time(),
        };
        let metadata = SatelliteReportMetadata {
            gps_time: report.gps_time(),
            sys_time: report.sys_time(),
            normalized_time,
            input_order: 0,
            scope: SatelliteScope::Global,
            coverage: SatelliteCoverage::Complete,
            reported_count: None,
        };
        Self {
            usage: vec![SatelliteUsageReport {
                metadata,
                satellites: report
                    .satellites()
                    .map(|satellite| SatelliteUsage {
                        identity: SatelliteIdentity {
                            constellation: satellite.constellation(),
                            prn: satellite.prn(),
                        },
                        membership: if satellite.in_fix() {
                            SatelliteMembership::Used
                        } else {
                            SatelliteMembership::Unused
                        },
                    })
                    .collect(),
            }],
            observations: vec![SatelliteObservationReport {
                metadata,
                satellites: report
                    .satellites()
                    .map(|satellite| SatelliteObservation {
                        identity: SatelliteIdentity {
                            constellation: satellite.constellation(),
                            prn: satellite.prn(),
                        },
                        measurements: SatSample::of(satellite),
                    })
                    .collect(),
            }],
        }
    }

    pub fn select_usage_report(
        &self,
        fix_time: GpsTime,
        eligible: impl Fn(&SatelliteUsageReport) -> bool,
    ) -> Option<(usize, &SatelliteUsageReport)> {
        self.usage
            .iter()
            .enumerate()
            .filter(|(_, report)| eligible(report))
            .min_by(|(_, left), (_, right)| {
                left.metadata.compare_for_fix(&right.metadata, fix_time)
            })
    }

    pub fn select_observation_report(
        &self,
        fix_time: GpsTime,
        eligible: impl Fn(&SatelliteObservationReport) -> bool,
    ) -> Option<(usize, &SatelliteObservationReport)> {
        self.observations
            .iter()
            .enumerate()
            .filter(|(_, report)| eligible(report))
            .min_by(|(_, left), (_, right)| {
                left.metadata.compare_for_fix(&right.metadata, fix_time)
            })
    }
}
