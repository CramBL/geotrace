use std::collections::HashSet;

use strum::IntoEnumIterator as _;

use crate::{Constellation, NavFixTime, Satellite, SatelliteReport};

/// Constellation and PRN identify one satellite within a report.
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
#[non_exhaustive]
pub struct SatelliteIdentity {
    /// GNSS constellation.
    pub constellation: Constellation,
    /// Satellite identifier within the constellation.
    pub prn: u32,
}

impl SatelliteIdentity {
    /// Construct an identity without imposing a PRN range.
    pub const fn new(constellation: Constellation, prn: u32) -> Self {
        Self { constellation, prn }
    }
}

/// Membership in the navigation solution, independent of observation measurements.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SatelliteMembership {
    /// The source did not establish membership.
    Unknown,
    /// The source reported that the satellite was unused.
    Unused,
    /// The source reported that the satellite contributed to the solution.
    Used,
}

/// Completeness applies to the identity list within the report's declared scope.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SatelliteCoverage {
    /// The list covers every identity within the declared scope.
    Complete,
    /// Omitted identities have unreported evidence.
    Partial,
}

/// A nonempty set in constellation declaration order, with repeated entries removed.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ConstellationSet {
    constellations: Vec<Constellation>,
}

impl ConstellationSet {
    /// Repeated constellations are removed and sorted in declaration order.
    pub fn new(first: Constellation, additional: impl IntoIterator<Item = Constellation>) -> Self {
        let mut constellations: Vec<_> = additional.into_iter().collect();
        constellations.push(first);
        Self {
            constellations: Constellation::iter()
                .filter(|constellation| constellations.contains(constellation))
                .collect(),
        }
    }

    /// Whether the constellation belongs to this set.
    pub fn contains(&self, constellation: Constellation) -> bool {
        self.constellations.contains(&constellation)
    }

    /// Constellations in declaration order.
    pub fn as_slice(&self) -> &[Constellation] {
        &self.constellations
    }
}

/// A set of every supported constellation remains distinct from global scope.
#[derive(Clone, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum SatelliteScope {
    /// An explicit set of one or more constellations.
    Constellations(ConstellationSet),
    /// All constellations, including identities omitted from a complete list.
    Global,
}

impl SatelliteScope {
    /// Whether the declared scope includes this constellation.
    pub fn contains(&self, constellation: Constellation) -> bool {
        match self {
            Self::Global => true,
            Self::Constellations(constellations) => constellations.contains(constellation),
        }
    }
}

/// One identity's reported membership, including explicit uncertainty.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub struct SatelliteUsage {
    /// Constellation and PRN.
    pub identity: SatelliteIdentity,
    /// Source-reported membership.
    pub membership: SatelliteMembership,
}

impl SatelliteUsage {
    /// Construct a row with explicit membership.
    pub const fn new(identity: SatelliteIdentity, membership: SatelliteMembership) -> Self {
        Self {
            identity,
            membership,
        }
    }
}

/// Measurements are optional independently of identity and membership.
#[derive(Clone, Copy, Debug, PartialEq, bon::Builder)]
#[non_exhaustive]
pub struct SatelliteObservation {
    /// Constellation and PRN.
    pub identity: SatelliteIdentity,
    /// Degrees above the horizon, when reported.
    pub elevation: Option<f32>,
    /// Degrees clockwise from true north, when reported.
    pub azimuth: Option<f32>,
    /// Measured dB-Hz, including zero. An unreported signal is `None`.
    pub snr: Option<f32>,
}

/// Complete coverage makes omitted identities known-unused within `scope`.
/// An explicit unknown membership remains unknown under complete coverage.
#[derive(Clone, Debug, PartialEq, bon::Builder)]
#[non_exhaustive]
pub struct SatelliteUsageReport {
    /// Original receiver and host timestamps.
    pub time: NavFixTime,
    /// Explicit global or constellation-set scope.
    pub scope: SatelliteScope,
    /// Completeness of the identity list within the scope.
    pub coverage: SatelliteCoverage,
    /// Exact for `scope`, independent of list coverage or conflicting rows.
    pub reported_count: Option<u32>,
    /// Source rows in input order, including repeated identities.
    pub satellites: Vec<SatelliteUsage>,
}

/// Observation coverage and aggregate counts are independent recorded facts.
#[derive(Clone, Debug, PartialEq, bon::Builder)]
#[non_exhaustive]
pub struct SatelliteObservationReport {
    /// Original receiver and host timestamps.
    pub time: NavFixTime,
    /// Explicit global or constellation-set scope.
    pub scope: SatelliteScope,
    /// Completeness of the identity list within the scope.
    pub coverage: SatelliteCoverage,
    /// Exact for `scope`, independent of list coverage or conflicting rows.
    pub reported_count: Option<u32>,
    /// Source rows in input order, including repeated identities.
    pub satellites: Vec<SatelliteObservation>,
}

/// Usage and observation samples in input order, with independent timestamps.
#[derive(Clone, Debug, Default, PartialEq)]
#[non_exhaustive]
pub struct SatelliteEvidence {
    /// Independent usage samples in input order.
    pub usage: Vec<SatelliteUsageReport>,
    /// Independent observation samples in input order.
    pub observations: Vec<SatelliteObservationReport>,
}

impl SatelliteEvidence {
    /// Preserve all source samples without merging timestamps or repeated identities.
    pub fn new(
        usage: Vec<SatelliteUsageReport>,
        observations: Vec<SatelliteObservationReport>,
    ) -> Self {
        Self {
            usage,
            observations,
        }
    }

    /// Projection requires one complete global pair with identical recorded timestamps.
    /// Unknown membership, conflicting aggregates and unobserved used identities prevent projection.
    pub fn legacy_report(&self) -> Option<SatelliteReport> {
        let ([usage], [observations]) = (self.usage.as_slice(), self.observations.as_slice())
        else {
            return None;
        };
        if usage.time != observations.time
            || usage.scope != SatelliteScope::Global
            || observations.scope != SatelliteScope::Global
            || usage.coverage != SatelliteCoverage::Complete
            || observations.coverage != SatelliteCoverage::Complete
            || usage
                .satellites
                .iter()
                .any(|satellite| satellite.membership == SatelliteMembership::Unknown)
            || usage.satellites.iter().any(|satellite| {
                satellite.membership == SatelliteMembership::Used
                    && !observations
                        .satellites
                        .iter()
                        .any(|observation| observation.identity == satellite.identity)
            })
        {
            return None;
        }

        let seen_identities: HashSet<_> = observations
            .satellites
            .iter()
            .map(|observation| observation.identity)
            .collect();
        let used_identities: HashSet<_> = usage
            .satellites
            .iter()
            .filter(|satellite| satellite.membership == SatelliteMembership::Used)
            .map(|satellite| satellite.identity)
            .collect();
        if usage
            .reported_count
            .is_some_and(|count| usize::try_from(count).ok() != Some(used_identities.len()))
            || observations
                .reported_count
                .is_some_and(|count| usize::try_from(count).ok() != Some(seen_identities.len()))
        {
            return None;
        }

        // Legacy conversion preserves row correspondence for repeated identities.
        let paired_rows = usage.satellites.len() == observations.satellites.len()
            && usage
                .satellites
                .iter()
                .zip(&observations.satellites)
                .all(|(satellite, observation)| satellite.identity == observation.identity);
        let tracked = observations
            .satellites
            .iter()
            .enumerate()
            .map(|(row, observation)| {
                let membership = if paired_rows {
                    usage.satellites.get(row)?.membership
                } else {
                    let mut memberships = usage
                        .satellites
                        .iter()
                        .filter(|satellite| satellite.identity == observation.identity)
                        .map(|satellite| satellite.membership);
                    let membership = memberships.next().unwrap_or(SatelliteMembership::Unused);
                    if memberships.any(|other| other != membership) {
                        return None;
                    }
                    membership
                };
                Some(Satellite {
                    constellation: observation.identity.constellation,
                    prn: observation.identity.prn,
                    in_fix: membership == SatelliteMembership::Used,
                    elevation: observation.elevation,
                    azimuth: observation.azimuth,
                    snr: observation.snr,
                })
            })
            .collect::<Option<Vec<_>>>()?;
        Some(SatelliteReport {
            time: observations.time,
            tracked,
        })
    }
}

impl From<&SatelliteReport> for SatelliteEvidence {
    /// Split every legacy row into complete global usage and observation reports.
    fn from(report: &SatelliteReport) -> Self {
        Self {
            usage: vec![SatelliteUsageReport {
                time: report.time,
                scope: SatelliteScope::Global,
                coverage: SatelliteCoverage::Complete,
                reported_count: None,
                satellites: report
                    .tracked
                    .iter()
                    .map(|satellite| SatelliteUsage {
                        identity: SatelliteIdentity {
                            constellation: satellite.constellation,
                            prn: satellite.prn,
                        },
                        membership: if satellite.in_fix {
                            SatelliteMembership::Used
                        } else {
                            SatelliteMembership::Unused
                        },
                    })
                    .collect(),
            }],
            observations: vec![SatelliteObservationReport {
                time: report.time,
                scope: SatelliteScope::Global,
                coverage: SatelliteCoverage::Complete,
                reported_count: None,
                satellites: report
                    .tracked
                    .iter()
                    .map(|satellite| SatelliteObservation {
                        identity: SatelliteIdentity {
                            constellation: satellite.constellation,
                            prn: satellite.prn,
                        },
                        elevation: satellite.elevation,
                        azimuth: satellite.azimuth,
                        snr: satellite.snr,
                    })
                    .collect(),
            }],
        }
    }
}
