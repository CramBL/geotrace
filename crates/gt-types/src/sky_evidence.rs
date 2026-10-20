use std::collections::BTreeSet;

use chrono::Duration;

use crate::satellites::{Constellation, Prn, SatSample, Satellite, Snr};
use crate::{
    LoadedTrack, NavPoint, NavigationEvidenceSource, PointIdx, ResolvedSatelliteCount,
    SKY_REPORT_MAX_AGE_SECS, SatelliteIdentity, SatelliteMembership, SatelliteObservationReport,
    SatelliteQueryScope, SatelliteUsageReport,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SkyGeometry {
    Identities,
    Azimuth,
    Position,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct SkySatellite {
    pub identity: SatelliteIdentity,
    pub membership: SatelliteMembership,
    pub usage_report_index: Option<usize>,
    pub measurements: SatSample,
}

impl SkySatellite {
    pub fn from_legacy(satellite: &Satellite) -> Self {
        Self {
            identity: SatelliteIdentity {
                constellation: satellite.constellation(),
                prn: satellite.prn(),
            },
            membership: if satellite.in_fix() {
                SatelliteMembership::Used
            } else {
                SatelliteMembership::Unused
            },
            usage_report_index: None,
            measurements: SatSample::of(satellite),
        }
    }

    pub fn constellation(&self) -> Constellation {
        self.identity.constellation
    }

    pub fn prn(&self) -> Prn {
        self.identity.prn
    }

    pub fn in_fix(&self) -> bool {
        self.membership == SatelliteMembership::Used
    }

    pub fn elevation(&self) -> Option<f32> {
        self.measurements.elevation
    }

    pub fn azimuth(&self) -> Option<f32> {
        self.measurements.azimuth
    }

    pub fn snr(&self) -> Option<Snr> {
        self.measurements.snr
    }

    pub fn sky_position(&self) -> Option<(f32, f32)> {
        Some((self.azimuth()?, self.elevation()?))
    }
}

#[derive(Clone, Copy, Debug)]
pub struct SkyEvidence<'a> {
    pub point: &'a NavPoint,
    pub usage_context: Option<(usize, &'a SatelliteUsageReport)>,
    pub used_count: Option<ResolvedSatelliteCount>,
    pub seen_count: Option<ResolvedSatelliteCount>,
    pub observation: Option<(usize, &'a SatelliteObservationReport)>,
}

#[derive(Clone, Copy, Debug)]
pub struct NearbySkyEvidence<'a> {
    pub point_index: PointIdx,
    pub evidence: SkyEvidence<'a>,
    pub age: Duration,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SkyCounts {
    pub used: Option<u32>,
    pub seen: Option<u32>,
}

impl<'a> SkyEvidence<'a> {
    fn for_point(
        point: &'a NavPoint,
        observation: Option<(usize, &'a SatelliteObservationReport)>,
        usage_context: Option<(usize, &'a SatelliteUsageReport)>,
    ) -> Self {
        Self {
            point,
            observation,
            usage_context,
            used_count: point.resolved_used_count(SatelliteQueryScope::Global),
            seen_count: point.resolved_seen_count(SatelliteQueryScope::Global),
        }
    }

    pub fn satellites(&self) -> Vec<SkySatellite> {
        let mut satellites: Vec<_> = self
            .observation
            .into_iter()
            .flat_map(|(_, report)| &report.satellites)
            .map(|satellite| {
                let resolved = self.point.resolved_satellite_membership(satellite.identity);
                SkySatellite {
                    identity: satellite.identity,
                    membership: resolved.membership,
                    usage_report_index: resolved.usage_report_index,
                    measurements: satellite.measurements,
                }
            })
            .collect();
        for satellite in self
            .point
            .satellite_evidence
            .usage
            .iter()
            .flat_map(|report| &report.satellites)
        {
            if satellites
                .iter()
                .any(|row| row.identity == satellite.identity)
            {
                continue;
            }
            let resolved = self.point.resolved_satellite_membership(satellite.identity);
            satellites.push(SkySatellite {
                identity: satellite.identity,
                membership: resolved.membership,
                usage_report_index: resolved.usage_report_index,
                measurements: SatSample {
                    elevation: None,
                    azimuth: None,
                    snr: None,
                },
            });
        }
        satellites
            .sort_by_key(|satellite| (satellite.identity.constellation, satellite.identity.prn));
        satellites
    }

    pub fn usage_reports(&self) -> Vec<(usize, &'a SatelliteUsageReport)> {
        let mut indices: BTreeSet<_> = self
            .satellites()
            .iter()
            .filter_map(|satellite| satellite.usage_report_index)
            .collect();
        if let Some((index, _)) = self.usage_context {
            indices.insert(index);
        }
        if let Some(count) = self.used_count {
            match count.evidence {
                NavigationEvidenceSource::UsageAggregate(index)
                | NavigationEvidenceSource::UsageList(index) => {
                    indices.insert(index);
                }
                _ => {}
            }
        }
        indices
            .into_iter()
            .filter_map(|index| {
                self.point
                    .satellite_evidence
                    .usage
                    .get(index)
                    .map(|report| (index, report))
            })
            .collect()
    }

    pub fn counts(&self) -> SkyCounts {
        SkyCounts {
            used: self.used_count.map(|count| count.count),
            seen: self.seen_count.map(|count| count.count),
        }
    }
}

impl SatelliteObservationReport {
    pub fn supports_sky_geometry(&self, geometry: SkyGeometry) -> bool {
        match geometry {
            SkyGeometry::Identities => true,
            SkyGeometry::Azimuth => self.has_azimuth(),
            SkyGeometry::Position => self.has_sky_position(),
        }
    }
}

impl NavPoint {
    pub fn sky_evidence(&self, geometry: SkyGeometry) -> Option<SkyEvidence<'_>> {
        let observation = self
            .satellite_evidence
            .select_observation_report(self.tpv.time(), |report| {
                report.supports_sky_geometry(geometry)
            });
        if observation.is_none()
            && (geometry != SkyGeometry::Identities || self.satellite_evidence.usage.is_empty())
        {
            return None;
        }
        let usage_context = if observation.is_none() {
            self.satellite_evidence
                .usage
                .iter()
                .enumerate()
                .min_by(|(_, left), (_, right)| {
                    left.metadata
                        .compare_for_fix(&right.metadata, self.tpv.time())
                })
        } else {
            None
        };
        Some(SkyEvidence::for_point(self, observation, usage_context))
    }

    pub fn supports_sky_geometry(&self, geometry: SkyGeometry) -> bool {
        (geometry == SkyGeometry::Identities && !self.satellite_evidence.usage.is_empty())
            || self
                .satellite_evidence
                .observations
                .iter()
                .any(|report| report.supports_sky_geometry(geometry))
    }
}

impl LoadedTrack {
    pub fn nearest_sky_evidence(
        &self,
        point_index: PointIdx,
        geometry: SkyGeometry,
    ) -> Option<NearbySkyEvidence<'_>> {
        let time = point_index.get(&self.points)?.tpv.time();
        let max_age = Duration::seconds(SKY_REPORT_MAX_AGE_SECS);
        let observations = self.points.iter().enumerate().flat_map(|(index, point)| {
            point
                .satellite_evidence
                .observations
                .iter()
                .enumerate()
                .filter_map(move |(report_index, report)| {
                    if !report.supports_sky_geometry(geometry)
                        || (time - report.metadata.normalized_time).abs() > max_age
                    {
                        return None;
                    }
                    Some((
                        report.metadata,
                        PointIdx::new(index),
                        point,
                        Some((report_index, report)),
                        None,
                    ))
                })
        });
        let usage_only = self.points.iter().enumerate().flat_map(|(index, point)| {
            point
                .satellite_evidence
                .usage
                .iter()
                .enumerate()
                .filter_map(move |(report_index, report)| {
                    if geometry != SkyGeometry::Identities
                        || (time - report.metadata.normalized_time).abs() > max_age
                    {
                        return None;
                    }
                    Some((
                        report.metadata,
                        PointIdx::new(index),
                        point,
                        None,
                        Some((report_index, report)),
                    ))
                })
        });
        observations
            .chain(usage_only)
            .min_by(|left, right| left.0.compare_for_fix(&right.0, time))
            .map(
                |(metadata, point_index, point, observation, usage)| NearbySkyEvidence {
                    point_index,
                    evidence: SkyEvidence::for_point(point, observation, usage),
                    age: time - metadata.normalized_time,
                },
            )
    }
}
