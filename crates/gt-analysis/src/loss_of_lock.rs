//! Loss-of-lock (cycle slip) detection and the slip-rate-per-minute series.
//!
//! A slip is recorded for a satellite, between two consecutive satellite
//! reports, when either:
//! - it was in view above the elevation mask in the previous report but is
//!   absent in the current one ([`SlipCause::LostLock`]), or
//! - it stays in view above the mask but its SNR fell by more than the
//!   configured threshold between the two ([`SlipCause::SnrDrop`]).
//!
//! Satellites below the mask, or with unknown elevation, are not considered:
//! horizon satellites set and re-acquire routinely and would otherwise swamp
//! the rate with false slips.  A satellite can yield at most one slip per
//! transition (the two causes are mutually exclusive: a lost-lock satellite is
//! absent, so it cannot also report an SNR drop).

use gt_types::nav_point::NavPoint;
use gt_types::satellites::{Constellation, SatSample, Satellites, Slip, SlipCause};

/// Seconds per minute, for converting the slip window (minutes) to the seconds
/// the epoch timestamps are measured in.
const SECS_PER_MIN: f64 = 60.0;

/// Detect the slips at the current report `curr` relative to the previous one
/// `prev`, under the elevation mask and SNR-drop threshold.
pub fn slips_between(
    prev: &Satellites,
    curr: &Satellites,
    mask_deg: f32,
    snr_drop_db: f32,
) -> Vec<Slip> {
    let mut slips = Vec::new();
    for p in prev.satellites() {
        // Only satellites solidly in view last epoch can slip.
        if !p.elevation().is_some_and(|e| e >= mask_deg) {
            continue;
        }
        let current = curr
            .satellites()
            .find(|c| c.constellation() == p.constellation() && c.prn() == p.prn());
        let (cause, to) = match current {
            None => (Some(SlipCause::LostLock), None),
            Some(c) => {
                let snr_dropped = c.elevation().is_some_and(|e| e >= mask_deg)
                    && matches!(
                        (p.snr(), c.snr()),
                        (Some(ps), Some(cs)) if ps.value() - cs.value() > snr_drop_db
                    );
                (
                    snr_dropped.then_some(SlipCause::SnrDrop),
                    Some(SatSample::of(c)),
                )
            }
        };
        if let Some(cause) = cause {
            slips.push(Slip {
                constellation: p.constellation(),
                prn: p.prn(),
                cause,
                from: SatSample::of(p),
                to,
            });
        }
    }
    slips
}

/// Detect slips across a track's `points`, grouped per epoch: each returned
/// entry pairs a point index with every satellite that slipped at that epoch.
///
/// The previous epoch is the most recent earlier point that carried a satellite
/// report; points without one are skipped without breaking continuity.  Epochs
/// with no slip produce no entry.  Used by the generated-marker pipeline, which
/// reads each event's position and time from `points[index]` and emits one
/// marker per epoch (not one per slipped satellite).
pub fn detect_slip_events(
    points: &[NavPoint],
    mask_deg: f32,
    snr_drop_db: f32,
) -> Vec<(usize, Vec<Slip>)> {
    let mut out = Vec::new();
    let mut prev: Option<&Satellites> = None;
    for (i, point) in points.iter().enumerate() {
        let Some(sats) = &point.satellites else {
            continue;
        };
        if let Some(prev_sats) = prev {
            let slips = slips_between(prev_sats, sats, mask_deg, snr_drop_db);
            if !slips.is_empty() {
                out.push((i, slips));
            }
        }
        prev = Some(sats);
    }
    out
}

/// Slip-rate-per-minute point series, combined and per constellation.  Each
/// series is `[unix_seconds, slips_per_minute]` at every epoch with a satellite
/// report.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SlipSeries {
    pub all: Vec<[f64; 2]>,
    pub gps: Vec<[f64; 2]>,
    pub glonass: Vec<[f64; 2]>,
    pub galileo: Vec<[f64; 2]>,
    pub beidou: Vec<[f64; 2]>,
}

/// Slip rate, in slips per minute, at each epoch in `epochs`, counting the slip
/// events in `events` that fall inside the trailing window `(t - window_secs, t]`
/// and dividing by `per_min` minutes.
///
/// Both slices must be sorted ascending in time, which lets the two cursors
/// advance monotonically for an O(n) sweep.  Returns no points when `per_min` is
/// not positive (the rate would be undefined).
fn windowed_rate(epochs: &[f64], events: &[f64], window_secs: f64, per_min: f64) -> Vec<[f64; 2]> {
    if per_min <= 0.0 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(epochs.len());
    let mut lo = 0usize;
    let mut hi = 0usize;
    for &t in epochs {
        let lower = t - window_secs;
        // Advance `hi` past every event at or before `t`, and `lo` past every
        // event at or before the window's lower edge; the gap is the count in
        // the half-open window.
        while events.get(hi).is_some_and(|&e| e <= t) {
            hi += 1;
        }
        while events.get(lo).is_some_and(|&e| e <= lower) {
            lo += 1;
        }
        out.push([t, hi.saturating_sub(lo) as f64 / per_min]);
    }
    out
}

/// Compute the slip-rate-per-minute series for a track's `points`: detect slips
/// between consecutive reports, then turn them into a trailing-window rate at
/// every epoch.
pub fn slip_rate_series(
    points: &[NavPoint],
    mask_deg: f32,
    snr_drop_db: f32,
    window_min: f32,
) -> SlipSeries {
    let mut epochs: Vec<f64> = Vec::new();
    let mut ev_all: Vec<f64> = Vec::new();
    let mut ev_gps: Vec<f64> = Vec::new();
    let mut ev_glonass: Vec<f64> = Vec::new();
    let mut ev_galileo: Vec<f64> = Vec::new();
    let mut ev_beidou: Vec<f64> = Vec::new();

    let mut prev: Option<&Satellites> = None;
    for point in points {
        let Some(sats) = &point.satellites else {
            continue;
        };
        let t = point.tpv.time().as_secs_f64();
        epochs.push(t);
        if let Some(prev_sats) = prev {
            for slip in slips_between(prev_sats, sats, mask_deg, snr_drop_db) {
                ev_all.push(t);
                match slip.constellation {
                    Constellation::Gps => ev_gps.push(t),
                    Constellation::Glonass => ev_glonass.push(t),
                    Constellation::Galileo => ev_galileo.push(t),
                    Constellation::Beidou => ev_beidou.push(t),
                }
            }
        }
        prev = Some(sats);
    }

    let window_secs = f64::from(window_min) * SECS_PER_MIN;
    let per_min = f64::from(window_min);
    SlipSeries {
        all: windowed_rate(&epochs, &ev_all, window_secs, per_min),
        gps: windowed_rate(&epochs, &ev_gps, window_secs, per_min),
        glonass: windowed_rate(&epochs, &ev_glonass, window_secs, per_min),
        galileo: windowed_rate(&epochs, &ev_galileo, window_secs, per_min),
        beidou: windowed_rate(&epochs, &ev_beidou, window_secs, per_min),
    }
}

#[cfg(test)]
mod detection_tests {
    use super::*;
    use gt_types::satellites::Satellite;

    /// `(constellation, prn, elevation, snr)` -> tracked `Satellite`.
    fn sat(c: Constellation, prn: u32, elevation: Option<f32>, snr: Option<f32>) -> Satellite {
        Satellite::new(c, prn, elevation, None, snr, false)
    }

    fn report(sats: Vec<Satellite>) -> Satellites {
        Satellites::new(None, None, sats)
    }

    #[test]
    fn lost_lock_when_above_mask_satellite_disappears() {
        let prev = report(vec![sat(Constellation::Gps, 7, Some(40.0), Some(45.0))]);
        let curr = report(vec![]);
        let slips = slips_between(&prev, &curr, 15.0, 10.0);
        assert_eq!(slips.len(), 1);
        let slip = slips.first().expect("one slip");
        assert_eq!(slip.cause, SlipCause::LostLock);
        assert_eq!(slip.constellation, Constellation::Gps);
        assert_eq!(slip.prn, 7);
        assert_eq!(slip.from.elevation, Some(40.0));
        assert_eq!(slip.to, None);
    }

    #[test]
    fn no_lost_lock_for_satellite_below_mask_or_unknown_elevation() {
        // A satellite setting below the mask, then gone, is a natural set.
        let prev = report(vec![
            sat(Constellation::Gps, 7, Some(5.0), Some(45.0)),
            sat(Constellation::Gps, 8, None, Some(45.0)),
        ]);
        let curr = report(vec![]);
        assert!(slips_between(&prev, &curr, 15.0, 10.0).is_empty());
    }

    #[test]
    fn snr_drop_above_threshold_is_a_slip_and_records_from_to() {
        let prev = report(vec![sat(Constellation::Galileo, 3, Some(30.0), Some(45.0))]);
        let curr = report(vec![sat(Constellation::Galileo, 3, Some(28.0), Some(30.0))]);
        let slips = slips_between(&prev, &curr, 15.0, 10.0);
        assert_eq!(slips.len(), 1);
        let slip = slips.first().expect("one slip");
        assert_eq!(slip.cause, SlipCause::SnrDrop);
        assert_eq!(slip.from.snr.map(|s| s.value()), Some(45.0));
        assert_eq!(slip.to.and_then(|t| t.snr).map(|s| s.value()), Some(30.0));
        assert_eq!(slip.to.map(|t| t.elevation), Some(Some(28.0)));
    }

    #[test]
    fn snr_drop_at_or_below_threshold_is_not_a_slip() {
        // Exactly the threshold does not count (strictly greater required).
        let prev = report(vec![sat(Constellation::Gps, 1, Some(30.0), Some(45.0))]);
        let curr = report(vec![sat(Constellation::Gps, 1, Some(30.0), Some(35.0))]);
        assert!(slips_between(&prev, &curr, 15.0, 10.0).is_empty());
    }

    #[test]
    fn snr_drop_ignored_when_satellite_falls_below_mask() {
        // Big SNR drop, but the satellite dipped under the mask this epoch, so
        // it is treated as a natural fade rather than a slip.
        let prev = report(vec![sat(Constellation::Gps, 1, Some(20.0), Some(45.0))]);
        let curr = report(vec![sat(Constellation::Gps, 1, Some(5.0), Some(20.0))]);
        assert!(slips_between(&prev, &curr, 15.0, 10.0).is_empty());
    }

    #[test]
    fn rising_snr_and_steady_lock_yield_no_slips() {
        let prev = report(vec![sat(Constellation::Beidou, 5, Some(40.0), Some(30.0))]);
        let curr = report(vec![sat(Constellation::Beidou, 5, Some(41.0), Some(48.0))]);
        assert!(slips_between(&prev, &curr, 15.0, 10.0).is_empty());
    }
}

#[cfg(test)]
mod windowed_rate_tests {
    use super::windowed_rate;

    /// Just the y-values (rates), for terser assertions.
    fn rates(pts: &[[f64; 2]]) -> Vec<f64> {
        pts.iter().map(|p| p[1]).collect()
    }

    #[test]
    fn empty_epochs_yield_no_points() {
        assert!(windowed_rate(&[], &[1.0, 2.0], 60.0, 1.0).is_empty());
    }

    #[test]
    fn no_events_give_a_zero_rate_at_every_epoch() {
        let out = windowed_rate(&[0.0, 60.0, 120.0], &[], 60.0, 1.0);
        assert_eq!(rates(&out), vec![0.0, 0.0, 0.0]);
    }

    #[test]
    fn non_positive_window_minutes_yield_no_points() {
        // The rate would divide by zero, so the series is empty by construction.
        assert!(windowed_rate(&[0.0, 60.0], &[10.0], 60.0, 0.0).is_empty());
    }

    #[test]
    fn event_at_the_epoch_is_counted_lower_edge_is_exclusive() {
        // Window is half-open `(t - window_secs, t]`: an event exactly at `t`
        // counts, one exactly at the lower edge does not.
        let epochs = [100.0];
        let window_secs = 60.0;
        // Event at t=100 (counted), at the lower edge t=40 (excluded), and inside.
        let events = [40.0, 70.0, 100.0];
        let out = windowed_rate(&epochs, &events, window_secs, 1.0);
        // Two events in (40, 100], over a 1-minute window -> 2 per minute.
        assert_eq!(rates(&out), vec![2.0]);
    }

    #[test]
    fn events_roll_off_as_the_trailing_window_advances() {
        // One event at t=0. With a 60 s window it is in range at t=0 and t=59,
        // but at t=60 it lands on the exclusive lower edge and drops out.
        let events = [0.0];
        let out = windowed_rate(&[0.0, 59.0, 60.0], &events, 60.0, 1.0);
        assert_eq!(rates(&out), vec![1.0, 1.0, 0.0]);
    }

    #[test]
    fn rate_divides_the_window_count_by_the_window_length() {
        // Three events all inside a 2-minute window -> 1.5 per minute.
        let events = [10.0, 20.0, 30.0];
        let out = windowed_rate(&[120.0], &events, 120.0, 2.0);
        assert_eq!(rates(&out), vec![1.5]);
    }
}

#[cfg(test)]
mod series_tests {
    use super::*;
    use chrono::{TimeZone, Utc};
    use gt_types::coordinates::{Latitude, Longitude};
    use gt_types::satellites::Satellite;
    use gt_types::time_types::GpsTime;
    use gt_types::tpv::TimePositionVelocity;

    fn point(secs: i64, sats: Vec<Satellite>) -> NavPoint {
        let time = GpsTime::from_utc(Utc.timestamp_opt(secs, 0).single().expect("valid"));
        let tpv = TimePositionVelocity::builder()
            .time(time)
            .lat(Latitude::new(55.0))
            .lon(Longitude::new(12.0))
            .build();
        NavPoint::new(tpv, Some(Satellites::new(Some(time), None, sats)))
    }

    fn gps(prn: u32, elevation: f32, snr: f32) -> Satellite {
        Satellite::new(
            Constellation::Gps,
            prn,
            Some(elevation),
            None,
            Some(snr),
            true,
        )
    }

    #[test]
    fn detect_slip_events_groups_all_slips_at_one_epoch() {
        // Sat 1 above mask throughout; sats 2 and 3 both lost at index 1, so they
        // form a single grouped event at that epoch.
        let points = vec![
            point(
                0,
                vec![gps(1, 40.0, 45.0), gps(2, 30.0, 40.0), gps(3, 25.0, 38.0)],
            ),
            point(1, vec![gps(1, 40.0, 45.0)]),
        ];
        let events = detect_slip_events(&points, 15.0, 10.0);
        assert_eq!(events.len(), 1, "one grouped event, not one per satellite");
        let (index, slips) = events.first().expect("one event");
        assert_eq!(*index, 1);
        assert_eq!(slips.len(), 2);
        let mut prns: Vec<u32> = slips.iter().map(|s| s.prn.value()).collect();
        prns.sort_unstable();
        assert_eq!(prns, vec![2, 3]);
        assert!(slips.iter().all(|s| s.cause == SlipCause::LostLock));
    }

    #[test]
    fn slip_rate_series_counts_one_slip_per_minute_window() {
        // One lost-lock slip at t=1 s; a 1-minute window gives a 1/min rate from
        // that epoch onward, on the all and GPS series.
        let points = vec![
            point(0, vec![gps(1, 40.0, 45.0), gps(2, 30.0, 40.0)]),
            point(1, vec![gps(1, 40.0, 45.0)]),
        ];
        let s = slip_rate_series(&points, 15.0, 10.0, 1.0);
        assert_eq!(s.all, vec![[0.0, 0.0], [1.0, 1.0]]);
        assert_eq!(s.gps, vec![[0.0, 0.0], [1.0, 1.0]]);
        assert!(s.glonass.iter().all(|p| p[1] == 0.0));
    }
}
