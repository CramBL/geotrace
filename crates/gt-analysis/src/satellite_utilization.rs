//! Satellite utilization rate: the share of in-view satellites (above an
//! elevation mask) that the receiver actually used in the fix, combined and per
//! constellation, plus the satellites it used *below* the mask (surfaced as
//! anomalies).

use gt_types::nav_point::NavPoint;
use gt_types::satellites::{Constellation, Prn, Satellites};
use gt_types::{SatelliteMembership, SatelliteQueryScope, UtilizationEvidence};

use crate::{DiagnosticRates, ObservationEpoch};

/// The masked "in view" baseline - denominator of the utilization rate.
///
/// Satellites with no reported elevation are excluded: their position above the
/// mask cannot be confirmed.  Pass `constellation = None` to count across every
/// constellation, or `Some(c)` to restrict to one.
pub fn in_view_above_mask(
    sats: &Satellites,
    constellation: Option<Constellation>,
    mask_deg: f32,
) -> usize {
    sats.satellites()
        .filter(|s| constellation.is_none_or(|c| s.constellation() == c))
        .filter(|s| s.elevation().is_some_and(|e| e >= mask_deg))
        .count()
}

/// In-fix satellites at or above the elevation mask - numerator of the
/// utilization rate.
///
/// Shares the mask predicate with [`in_view_above_mask`], so the result is
/// always a subset of that denominator: the rate stays within [0, 1] by
/// construction, without clamping.  A satellite used below the mask is excluded
/// here and surfaced via [`masked_out_in_fix`] instead.
pub fn in_fix_above_mask(
    sats: &Satellites,
    constellation: Option<Constellation>,
    mask_deg: f32,
) -> usize {
    sats.satellites()
        .filter(|s| constellation.is_none_or(|c| s.constellation() == c))
        .filter(|s| s.in_fix() && s.elevation().is_some_and(|e| e >= mask_deg))
        .count()
}

/// Satellites in the fix yet below the elevation mask - used by the receiver but
/// excluded from the utilization rate by the mask.
///
/// Surfaced as plot anomaly markers so this exclusion stays visible.  Satellites
/// without a reported elevation are not included (their elevation can be neither
/// shown nor compared against the mask).
pub fn masked_out_in_fix(sats: &Satellites, mask_deg: f32) -> impl Iterator<Item = MaskedSat> {
    sats.satellites().filter_map(move |s| {
        let elevation = s.elevation().filter(|&e| e < mask_deg)?;
        s.in_fix().then(|| MaskedSat {
            constellation: s.constellation(),
            prn: s.prn(),
            elevation,
        })
    })
}

/// A satellite that was used in the fix while sitting below the elevation mask.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MaskedSat {
    pub constellation: Constellation,
    pub prn: Prn,
    pub elevation: f32,
}

/// One epoch at which at least one [`MaskedSat`] was used in the fix.
#[derive(Debug, Clone, PartialEq)]
pub struct UtilAnomaly {
    /// Epoch, in Unix seconds (the marker's x-position).
    pub t: f64,
    /// All-constellations utilization-rate percentage at this epoch, used as the
    /// marker's y-position. An unavailable rate uses the 0% line.
    pub value: f64,
    /// The used sub-mask satellites, sorted by ascending elevation.
    pub masked: Vec<MaskedSat>,
}

/// Per-track utilization point series plus the masked-satellite anomalies, all
/// derived together from one pass over the track at a given elevation mask.
///
/// Each series is `[unix_seconds, percent]` points at the epochs that have a
/// satellite report and a non-empty masked baseline.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UtilPoints {
    pub all: Vec<[f64; 2]>,
    pub gps: Vec<[f64; 2]>,
    pub glonass: Vec<[f64; 2]>,
    pub galileo: Vec<[f64; 2]>,
    pub beidou: Vec<[f64; 2]>,
    pub navic: Vec<[f64; 2]>,
    pub qzss: Vec<[f64; 2]>,
    pub anomalies: Vec<UtilAnomaly>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct UtilizationEpoch {
    pub source: ObservationEpoch,
    pub rates: DiagnosticRates,
    pub usage_indices: Vec<(SatelliteQueryScope, usize)>,
    pub masked: Vec<MaskedSat>,
}

impl UtilizationEpoch {
    pub fn into_anomaly(self) -> Option<UtilAnomaly> {
        if self.masked.is_empty() {
            return None;
        }
        Some(UtilAnomaly {
            t: self
                .source
                .metadata
                .normalized_time
                .as_secs_f64_with_subseconds(),
            value: self.rates.all.unwrap_or(ANOMALY_FALLBACK_RATE),
            masked: self.masked,
        })
    }
}

pub fn utilization_epochs(points: &[NavPoint], mask_deg: f32) -> Vec<UtilizationEpoch> {
    points
        .iter()
        .enumerate()
        .flat_map(|(point_index, point)| {
            point
                .satellite_evidence
                .observations
                .iter()
                .enumerate()
                .map(move |(observation_index, observation)| {
                    let mut usage_indices = Vec::new();
                    let rates = DiagnosticRates::for_scopes(|scope| {
                        let evidence =
                            point.utilization_evidence_at(observation_index, scope, mask_deg)?;
                        let rate = UtilizationCalculation {
                            evidence,
                            scope,
                            elevation_mask_deg: mask_deg,
                        }
                        .percent()?;
                        usage_indices.push((scope, evidence.usage_index));
                        Some(rate)
                    });
                    let mut masked: Vec<_> = observation
                        .satellites
                        .iter()
                        .filter_map(|satellite| {
                            let elevation = satellite
                                .measurements
                                .elevation
                                .filter(|elevation| *elevation < mask_deg)?;
                            (point.reported_membership(satellite.identity)
                                == SatelliteMembership::Used)
                                .then_some(MaskedSat {
                                    constellation: satellite.identity.constellation,
                                    prn: satellite.identity.prn,
                                    elevation,
                                })
                        })
                        .collect();
                    masked.sort_by(|left, right| left.elevation.total_cmp(&right.elevation));
                    UtilizationEpoch {
                        source: ObservationEpoch {
                            point_index,
                            observation_index,
                            metadata: observation.metadata,
                        },
                        rates,
                        usage_indices,
                        masked,
                    }
                })
        })
        .collect()
}

/// Per-point utilization rates (percent), aligned with `points` by index.
///
/// Missing evidence and an empty above-mask baseline produce `None`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct UtilPerPoint {
    pub all: Vec<Option<f64>>,
    pub gps: Vec<Option<f64>>,
    pub glonass: Vec<Option<f64>>,
    pub galileo: Vec<Option<f64>>,
    pub beidou: Vec<Option<f64>>,
    pub navic: Vec<Option<f64>>,
    pub qzss: Vec<Option<f64>>,
}

/// Select eligible attached evidence for each point and requested scope.
pub fn util_per_point(points: &[NavPoint], mask_deg: f32) -> UtilPerPoint {
    let mut out = UtilPerPoint::default();
    for point in points {
        let rates = DiagnosticRates::for_scopes(|scope| {
            UtilizationCalculation {
                evidence: point.utilization_evidence(scope, mask_deg)?,
                scope,
                elevation_mask_deg: mask_deg,
            }
            .percent()
        });
        out.all.push(rates.all);
        out.gps
            .push(rates.get(SatelliteQueryScope::Constellation(Constellation::Gps)));
        out.glonass
            .push(rates.get(SatelliteQueryScope::Constellation(Constellation::Glonass)));
        out.galileo
            .push(rates.get(SatelliteQueryScope::Constellation(Constellation::Galileo)));
        out.beidou
            .push(rates.get(SatelliteQueryScope::Constellation(Constellation::Beidou)));
        out.navic
            .push(rates.get(SatelliteQueryScope::Constellation(Constellation::Navic)));
        out.qzss
            .push(rates.get(SatelliteQueryScope::Constellation(Constellation::Qzss)));
    }
    out
}

/// Compute the utilization-rate series and masked-satellite anomalies for one
/// track's `points` at elevation mask `mask_deg`.
pub fn compute_util(points: &[NavPoint], mask_deg: f32) -> UtilPoints {
    let mut out = UtilPoints::default();

    for epoch in utilization_epochs(points, mask_deg) {
        let t = epoch
            .source
            .metadata
            .normalized_time
            .as_secs_f64_with_subseconds();
        let rates = &epoch.rates;
        for (series, rate) in [
            (&mut out.all, rates.all),
            (
                &mut out.gps,
                rates.get(SatelliteQueryScope::Constellation(Constellation::Gps)),
            ),
            (
                &mut out.glonass,
                rates.get(SatelliteQueryScope::Constellation(Constellation::Glonass)),
            ),
            (
                &mut out.galileo,
                rates.get(SatelliteQueryScope::Constellation(Constellation::Galileo)),
            ),
            (
                &mut out.beidou,
                rates.get(SatelliteQueryScope::Constellation(Constellation::Beidou)),
            ),
            (
                &mut out.navic,
                rates.get(SatelliteQueryScope::Constellation(Constellation::Navic)),
            ),
            (
                &mut out.qzss,
                rates.get(SatelliteQueryScope::Constellation(Constellation::Qzss)),
            ),
        ] {
            if let Some(rate) = rate {
                series.push([t, rate]);
            }
        }
        if let Some(anomaly) = epoch.into_anomaly() {
            out.anomalies.push(anomaly);
        }
    }

    out
}

struct UtilizationCalculation<'a> {
    evidence: UtilizationEvidence<'a>,
    scope: SatelliteQueryScope,
    elevation_mask_deg: f32,
}

impl UtilizationCalculation<'_> {
    fn percent(&self) -> Option<f64> {
        if !self.evidence.observation.metadata.complete_for(self.scope) {
            return None;
        }
        let mut used = 0_u32;
        let mut observed = 0_u32;
        for satellite in self
            .evidence
            .observation
            .satellites
            .iter()
            .filter(|satellite| self.scope.includes(satellite.identity))
        {
            if satellite.measurements.elevation? >= self.elevation_mask_deg {
                observed += 1;
                match self.evidence.usage.membership(satellite.identity) {
                    SatelliteMembership::Unknown => return None,
                    SatelliteMembership::Unused => (),
                    SatelliteMembership::Used => used += 1,
                }
            }
        }
        (observed > 0).then(|| f64::from(used) / f64::from(observed) * PERCENT_PER_UNIT)
    }
}

const PERCENT_PER_UNIT: f64 = 100.0;

/// Anomaly position when the utilization rate is unavailable.
const ANOMALY_FALLBACK_RATE: f64 = 0.0;

#[cfg(test)]
mod tests {
    use gt_types::coordinates::{Latitude, Longitude};
    use gt_types::fixtures;
    use gt_types::satellites::{Prn, Satellite};

    use super::*;
    use crate::test_util;

    /// One satellite of `constellation` at `elevation_deg`, in the fix or only
    /// in view, with a throwaway PRN and no signal quality: every rule below
    /// reads the elevation and the fix flag alone.
    fn sat(constellation: Constellation, elevation_deg: Option<f32>, in_fix: bool) -> Satellite {
        fixtures::satellite(constellation, 1, elevation_deg, None, in_fix)
    }

    #[test]
    fn in_view_above_mask_excludes_sub_mask_and_unknown_elevation() {
        let r = test_util::report(vec![
            sat(Constellation::Gps, Some(20.0), false),
            sat(Constellation::Gps, Some(15.0), false), // exactly at the mask counts
            sat(Constellation::Gps, Some(5.0), false),  // below mask
            sat(Constellation::Gps, None, false),       // unknown elevation
        ]);
        assert_eq!(in_view_above_mask(&r, None, 15.0), 2);
        assert_eq!(in_view_above_mask(&r, Some(Constellation::Gps), 15.0), 2);
        assert_eq!(
            in_view_above_mask(&r, Some(Constellation::Galileo), 15.0),
            0
        );
        // A 0 deg mask still drops unknown-elevation satellites.
        assert_eq!(in_view_above_mask(&r, None, 0.0), 3);
    }

    #[test]
    fn in_fix_above_mask_excludes_used_sub_mask_and_unknown_elevation() {
        let r = test_util::report(vec![
            sat(Constellation::Gps, Some(20.0), true), // used, above mask
            sat(Constellation::Gps, Some(5.0), true),  // used but below mask -> excluded
            sat(Constellation::Glonass, Some(40.0), true),
            sat(Constellation::Gps, Some(50.0), false), // seen, not used
            sat(Constellation::Gps, None, true),        // used, unknown elevation -> excluded
        ]);
        assert_eq!(in_fix_above_mask(&r, None, 15.0), 2);
        assert_eq!(in_fix_above_mask(&r, Some(Constellation::Gps), 15.0), 1);
        assert_eq!(in_fix_above_mask(&r, Some(Constellation::Glonass), 15.0), 1);
    }

    /// [`compute_util`] reports every satellite this flags as an anomaly.
    #[test]
    fn masked_out_in_fix_flags_used_sub_mask_satellites_only() {
        let r = test_util::report(vec![
            sat(Constellation::Gps, Some(5.0), true), // used and below mask -> flagged
            sat(Constellation::Gps, Some(20.0), true), // used, above mask
            sat(Constellation::Gps, Some(3.0), false), // below mask but not used
            sat(Constellation::Gps, None, true),      // used, unknown elevation -> not flagged
        ]);
        let flagged: Vec<f32> = masked_out_in_fix(&r, 15.0).map(|s| s.elevation).collect();
        assert_eq!(flagged, vec![5.0]);
    }

    /// The per-point form is index-aligned: reportless points hold `None`,
    /// and every value matches the time-keyed series.
    #[test]
    fn util_per_point_aligns_with_compute_util() {
        let point = |secs: i64, sats: Option<Satellites>| {
            let time =
                chrono::DateTime::from_timestamp(1_700_000_000 + secs, 0).expect("valid timestamp");
            let position = fixtures::nav_point(
                time,
                Latitude::new(55.0),
                Longitude::new(12.0),
                fixtures::FixKind::GhostWithoutHeading,
            );
            NavPoint::new(position.tpv, sats)
        };
        let with_sats = |used: bool| {
            test_util::report(vec![
                sat(Constellation::Gps, Some(40.0), used),
                fixtures::satellite(Constellation::Gps, 2, Some(30.0), None, true),
            ])
        };
        let points = vec![
            point(0, Some(with_sats(true))),
            point(1, None),
            point(2, Some(with_sats(false))),
        ];

        let per_point = util_per_point(&points, 15.0);
        assert_eq!(per_point.all, vec![Some(100.0), None, Some(50.0)]);
        assert_eq!(per_point.gps.len(), points.len());

        // Same values as the time-keyed series, in order.
        let series = compute_util(&points, 15.0);
        let series_values: Vec<f64> = series.all.iter().map(|[_, v]| *v).collect();
        let aligned_values: Vec<f64> = per_point.all.iter().copied().flatten().collect();
        assert_eq!(series_values, aligned_values);
    }

    #[test]
    fn an_anomaly_without_an_above_mask_satellite_sits_at_zero_percent() {
        let points = [test_util::point_at(
            0,
            vec![sat(Constellation::Gps, Some(5.0), true)],
        )];

        let util = compute_util(&points, 15.0);

        assert_eq!(
            util.anomalies,
            [UtilAnomaly {
                t: 0.0,
                value: 0.0,
                masked: vec![MaskedSat {
                    constellation: Constellation::Gps,
                    prn: Prn::new(1),
                    elevation: 5.0,
                }],
            }]
        );
        assert!(
            util.all.is_empty(),
            "an empty masked baseline contributes no rate point"
        );
    }
}
