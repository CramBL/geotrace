//! Satellite utilization rate: the share of in-view satellites (above an
//! elevation mask) that the receiver actually used in the fix, combined and per
//! constellation, plus the satellites it used *below* the mask (surfaced as
//! anomalies rather than silently lowering the rate).

use gt_types::nav_point::NavPoint;
use gt_types::satellites::{Constellation, Prn, Satellite, Satellites};

/// Y-position for an anomaly marker when the masked baseline is empty (no
/// in-view satellite above the mask), where the rate is undefined.  The marker
/// lands at the 0 % line - no above-mask satellite was usable, let alone used.
const ANOMALY_FALLBACK_RATE: f64 = 0.0;

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
/// Surfaced as plot anomaly markers so this exclusion stays visible rather than
/// silently lowering the rate.  Satellites without a reported elevation are not
/// included (their elevation can be neither shown nor compared against the mask).
pub fn masked_out_in_fix(sats: &Satellites, mask_deg: f32) -> impl Iterator<Item = &Satellite> {
    sats.satellites()
        .filter(move |s| s.in_fix() && s.elevation().is_some_and(|e| e < mask_deg))
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
    /// marker's y-position so it sits on the all-constellations line.  Falls back
    /// to the 0 % line when the masked baseline is empty (no in-view satellite
    /// above the mask), which would otherwise leave the rate undefined.
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

/// Utilization rate as a percentage, or `None` when the masked baseline is empty
/// (division undefined - the epoch contributes no line point).
fn rate_percent(num: usize, den: usize) -> Option<f64> {
    (den > 0).then(|| (num as f64 / den as f64) * 100.0)
}

/// Compute the utilization-rate series and masked-satellite anomalies for one
/// track's `points` at the given elevation mask.
pub fn compute_util(points: &[NavPoint], mask_deg: f32) -> UtilPoints {
    let mut out = UtilPoints::default();

    for point in points {
        let Some(sats) = &point.satellites else {
            continue;
        };
        let t = point.tpv.time().as_secs_f64();

        let all_rate = rate_percent(
            in_fix_above_mask(sats, None, mask_deg),
            in_view_above_mask(sats, None, mask_deg),
        );
        if let Some(r) = all_rate {
            out.all.push([t, r]);
        }

        let rate_for = |c: Constellation| {
            rate_percent(
                in_fix_above_mask(sats, Some(c), mask_deg),
                in_view_above_mask(sats, Some(c), mask_deg),
            )
        };
        if let Some(r) = rate_for(Constellation::Gps) {
            out.gps.push([t, r]);
        }
        if let Some(r) = rate_for(Constellation::Glonass) {
            out.glonass.push([t, r]);
        }
        if let Some(r) = rate_for(Constellation::Galileo) {
            out.galileo.push([t, r]);
        }
        if let Some(r) = rate_for(Constellation::Beidou) {
            out.beidou.push([t, r]);
        }
        if let Some(r) = rate_for(Constellation::Navic) {
            out.navic.push([t, r]);
        }
        if let Some(r) = rate_for(Constellation::Qzss) {
            out.qzss.push([t, r]);
        }

        let mut masked: Vec<MaskedSat> = masked_out_in_fix(sats, mask_deg)
            // `elevation` is guaranteed `Some` by `masked_out_in_fix`'s predicate.
            .filter_map(|s| {
                s.elevation().map(|e| MaskedSat {
                    constellation: s.constellation(),
                    prn: s.prn(),
                    elevation: e,
                })
            })
            .collect();
        if !masked.is_empty() {
            masked.sort_by(|a, b| a.elevation.total_cmp(&b.elevation));
            out.anomalies.push(UtilAnomaly {
                t,
                value: all_rate.unwrap_or(ANOMALY_FALLBACK_RATE),
                masked,
            });
        }
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `(constellation, elevation, in_fix)` -> `Satellite`, with a throwaway PRN.
    fn sat(c: Constellation, elevation: Option<f32>, in_fix: bool) -> Satellite {
        Satellite::new(c, 1, elevation, None, None, in_fix)
    }

    fn report(sats: Vec<Satellite>) -> Satellites {
        Satellites::new(None, None, sats)
    }

    #[test]
    fn in_view_above_mask_excludes_sub_mask_and_unknown_elevation() {
        let r = report(vec![
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
        let r = report(vec![
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

    #[test]
    fn masked_out_in_fix_flags_used_sub_mask_satellites_only() {
        let r = report(vec![
            sat(Constellation::Gps, Some(5.0), true), // used and below mask -> flagged
            sat(Constellation::Gps, Some(20.0), true), // used, above mask
            sat(Constellation::Gps, Some(3.0), false), // below mask but not used
            sat(Constellation::Gps, None, true),      // used, unknown elevation -> not flagged
        ]);
        let flagged: Vec<f32> = masked_out_in_fix(&r, 15.0)
            .filter_map(Satellite::elevation)
            .collect();
        assert_eq!(flagged, vec![5.0]);
    }

    /// A satellite used below the mask is excluded from both the numerator and
    /// the denominator, so the rate stays within 100 % without clamping. The
    /// excluded satellite is surfaced as an anomaly instead.
    #[test]
    fn utilization_stays_within_one_hundred_percent() {
        let r = report(vec![
            sat(Constellation::Gps, Some(20.0), true),
            sat(Constellation::Gps, Some(5.0), true), // used but below mask
        ]);
        let num = in_fix_above_mask(&r, None, 15.0);
        let den = in_view_above_mask(&r, None, 15.0);
        assert_eq!(num, 1);
        assert_eq!(den, 1);
        assert!((num as f64) / (den as f64) <= 1.0);
        assert_eq!(masked_out_in_fix(&r, 15.0).count(), 1);
    }
}
