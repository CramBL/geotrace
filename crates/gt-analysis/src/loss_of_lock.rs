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
/// the epoch timestamps are measured in. Public so callers converting in the
/// opposite direction share the same factor.
pub const SECS_PER_MIN: f64 = 60.0;

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
    pub navic: Vec<[f64; 2]>,
    pub qzss: Vec<[f64; 2]>,
}

/// Slip rate, in slips per minute, at each epoch in `epochs`, counting the slip
/// events in `events` that fall inside the trailing window `(t - window_secs, t]`
/// and dividing by `per_min` minutes.
///
/// A track's fix timestamps are not guaranteed to ascend (a receiver resuming
/// after a gap can stamp a fix earlier than the one before it), so epochs and
/// events out of ascending order are sorted before the sweep and the rates are
/// written back into the order `epochs` was given in.  Returns no points when
/// `per_min` is not positive (the rate would be undefined).
fn windowed_rate(epochs: &[f64], events: &[f64], window_secs: f64, per_min: f64) -> Vec<[f64; 2]> {
    if per_min <= 0.0 {
        return Vec::new();
    }
    if epochs.is_sorted() && events.is_sorted() {
        return windowed_rate_of_ascending_epochs_and_events(epochs, events, window_secs, per_min);
    }

    let mut epochs_in_time_order: Vec<(f64, usize)> =
        epochs.iter().enumerate().map(|(i, &t)| (t, i)).collect();
    epochs_in_time_order.sort_by(|(a, _), (b, _)| a.total_cmp(b));
    let sorted_epochs: Vec<f64> = epochs_in_time_order.iter().map(|&(t, _)| t).collect();
    let mut sorted_events = events.to_vec();
    sorted_events.sort_by(f64::total_cmp);

    let swept = windowed_rate_of_ascending_epochs_and_events(
        &sorted_epochs,
        &sorted_events,
        window_secs,
        per_min,
    );
    let mut out = vec![[0.0; 2]; epochs.len()];
    for (&(_, original_index), point) in epochs_in_time_order.iter().zip(swept) {
        if let Some(slot) = out.get_mut(original_index) {
            *slot = point;
        }
    }
    out
}

/// [`windowed_rate`] for the common case of both slices already sorted
/// ascending in time, where the two cursors advance monotonically for an O(n)
/// sweep with no allocation beyond the result.
fn windowed_rate_of_ascending_epochs_and_events(
    epochs: &[f64],
    events: &[f64],
    window_secs: f64,
    per_min: f64,
) -> Vec<[f64; 2]> {
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

/// Report epochs and slip events of one track, shared by the time-keyed and
/// per-point rate forms so their values cannot drift.
#[derive(Default)]
struct SlipEvents {
    /// Timestamp of every point with a satellite report, in point order.
    epochs: Vec<f64>,
    /// Index into `points` of each entry in `epochs`.
    epoch_points: Vec<usize>,
    all: Vec<f64>,
    gps: Vec<f64>,
    glonass: Vec<f64>,
    galileo: Vec<f64>,
    beidou: Vec<f64>,
    navic: Vec<f64>,
    qzss: Vec<f64>,
}

fn collect_slip_events(points: &[NavPoint], mask_deg: f32, snr_drop_db: f32) -> SlipEvents {
    let mut out = SlipEvents::default();
    let mut prev: Option<&Satellites> = None;
    for (pi, point) in points.iter().enumerate() {
        let Some(sats) = &point.satellites else {
            continue;
        };
        let t = point.tpv.time().as_secs_f64_with_subseconds();
        out.epochs.push(t);
        out.epoch_points.push(pi);
        if let Some(prev_sats) = prev {
            for slip in slips_between(prev_sats, sats, mask_deg, snr_drop_db) {
                out.all.push(t);
                match slip.constellation {
                    Constellation::Gps => out.gps.push(t),
                    Constellation::Glonass => out.glonass.push(t),
                    Constellation::Galileo => out.galileo.push(t),
                    Constellation::Beidou => out.beidou.push(t),
                    Constellation::Navic => out.navic.push(t),
                    Constellation::Qzss => out.qzss.push(t),
                }
            }
        }
        prev = Some(sats);
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
    let events = collect_slip_events(points, mask_deg, snr_drop_db);
    let window_secs = f64::from(window_min) * SECS_PER_MIN;
    let per_min = f64::from(window_min);
    SlipSeries {
        all: windowed_rate(&events.epochs, &events.all, window_secs, per_min),
        gps: windowed_rate(&events.epochs, &events.gps, window_secs, per_min),
        glonass: windowed_rate(&events.epochs, &events.glonass, window_secs, per_min),
        galileo: windowed_rate(&events.epochs, &events.galileo, window_secs, per_min),
        beidou: windowed_rate(&events.epochs, &events.beidou, window_secs, per_min),
        navic: windowed_rate(&events.epochs, &events.navic, window_secs, per_min),
        qzss: windowed_rate(&events.epochs, &events.qzss, window_secs, per_min),
    }
}

/// Per-point slip rates (slips per minute), aligned with `points` by index.
///
/// Entry i is `None` when point i has no satellite report, or when
/// `window_min` is not positive (the rate is undefined). The index alignment
/// is what the query evaluator needs; the plot uses the time-keyed
/// [`slip_rate_series`] instead.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SlipRatePerPoint {
    pub all: Vec<Option<f64>>,
    pub gps: Vec<Option<f64>>,
    pub glonass: Vec<Option<f64>>,
    pub galileo: Vec<Option<f64>>,
    pub beidou: Vec<Option<f64>>,
    pub navic: Vec<Option<f64>>,
    pub qzss: Vec<Option<f64>>,
}

/// Compute per-point slip rates for a track's `points`. Same values as
/// [`slip_rate_series`], keyed by point index.
pub fn slip_rate_per_point(
    points: &[NavPoint],
    mask_deg: f32,
    snr_drop_db: f32,
    window_min: f32,
) -> SlipRatePerPoint {
    let events = collect_slip_events(points, mask_deg, snr_drop_db);
    let window_secs = f64::from(window_min) * SECS_PER_MIN;
    let per_min = f64::from(window_min);
    let scatter = |ev: &[f64]| {
        let mut out = vec![None; points.len()];
        let rates = windowed_rate(&events.epochs, ev, window_secs, per_min);
        for (k, [_, rate]) in rates.iter().enumerate() {
            if let Some(slot) = events.epoch_points.get(k).and_then(|&pi| out.get_mut(pi)) {
                *slot = Some(*rate);
            }
        }
        out
    };
    SlipRatePerPoint {
        all: scatter(&events.all),
        gps: scatter(&events.gps),
        glonass: scatter(&events.glonass),
        galileo: scatter(&events.galileo),
        beidou: scatter(&events.beidou),
        navic: scatter(&events.navic),
        qzss: scatter(&events.qzss),
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
    fn epochs_and_events_out_of_order_count_the_windows_they_would_when_sorted() {
        // Epochs 0, 60 and 120 with events at 10 and 70, all given out of
        // order: t=0 counts nothing, t=60 counts the event at 10, t=120 the one
        // at 70. Each rate comes back at its epoch's own position.
        let out = windowed_rate(&[120.0, 0.0, 60.0], &[70.0, 10.0], 60.0, 1.0);
        assert_eq!(rates(&out), vec![1.0, 0.0, 1.0]);
        assert_eq!(out.first().map(|p| p[0]), Some(120.0));
    }

    #[test]
    fn rate_divides_the_window_count_by_the_window_length() {
        // Three events all inside a 2-minute window -> 1.5 per minute.
        let events = [10.0, 20.0, 30.0];
        let out = windowed_rate(&[120.0], &events, 120.0, 2.0);
        assert_eq!(rates(&out), vec![1.5]);
    }

    /// Window length of the generated cases, in seconds.
    const PROPERTY_WINDOW_SECS: f64 = 60.0;

    /// Window length of the generated cases, in minutes - the divisor that
    /// turns a window count into a rate.
    const PROPERTY_PER_MIN: f64 = 1.0;

    proptest::proptest! {
        /// Every epoch's rate counts exactly the events inside its own
        /// `(t - window, t]` window, whatever order the epochs and the events
        /// arrive in, and each rate comes back at its epoch's own position.
        #[test]
        fn every_epoch_counts_the_events_in_its_own_window(
            epochs in proptest::collection::vec(-1000.0f64..1000.0, 0..40),
            events in proptest::collection::vec(-1000.0f64..1000.0, 0..40),
        ) {
            let expected: Vec<[f64; 2]> = epochs
                .iter()
                .map(|&t| {
                    let counted = events
                        .iter()
                        .filter(|&&e| e > t - PROPERTY_WINDOW_SECS && e <= t)
                        .count();
                    [t, counted as f64 / PROPERTY_PER_MIN]
                })
                .collect();

            let out = windowed_rate(&epochs, &events, PROPERTY_WINDOW_SECS, PROPERTY_PER_MIN);

            proptest::prop_assert_eq!(out, expected);
        }
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

    /// The per-point form is index-aligned: reportless points hold `None`,
    /// and every value matches the time-keyed series.
    #[test]
    fn slip_rate_per_point_aligns_with_series() {
        let reportless = |secs: i64| {
            let time = GpsTime::from_utc(Utc.timestamp_opt(secs, 0).single().expect("valid"));
            let tpv = TimePositionVelocity::builder()
                .time(time)
                .lat(Latitude::new(55.0))
                .lon(Longitude::new(12.0))
                .build();
            NavPoint::new(tpv, None)
        };
        let points = vec![
            point(0, vec![gps(1, 40.0, 45.0), gps(2, 30.0, 40.0)]),
            reportless(1),
            point(2, vec![gps(1, 40.0, 45.0)]),
        ];

        let per_point = slip_rate_per_point(&points, 15.0, 10.0, 1.0);
        assert_eq!(per_point.all, vec![Some(0.0), None, Some(1.0)]);
        assert_eq!(per_point.gps, vec![Some(0.0), None, Some(1.0)]);

        let series = slip_rate_series(&points, 15.0, 10.0, 1.0);
        let series_values: Vec<f64> = series.all.iter().map(|[_, v]| *v).collect();
        let aligned_values: Vec<f64> = per_point.all.iter().copied().flatten().collect();
        assert_eq!(series_values, aligned_values);
    }

    /// A non-positive window yields no defined rates rather than a division
    /// artifact.
    #[test]
    fn slip_rate_per_point_with_zero_window_is_all_none() {
        let points = vec![point(0, vec![gps(1, 40.0, 45.0)])];
        let per_point = slip_rate_per_point(&points, 15.0, 10.0, 0.0);
        assert_eq!(per_point.all, vec![None]);
    }

    /// A NavIC slip lands in `s.navic`, a QZSS slip in `s.qzss`, and neither
    /// leaks into the GPS series - the new constellations get their own buckets.
    #[test]
    fn navic_and_qzss_slips_route_into_their_own_series() {
        let nav = |prn, el, snr| {
            Satellite::new(Constellation::Navic, prn, Some(el), None, Some(snr), true)
        };
        let qzs = |prn, el, snr| {
            Satellite::new(Constellation::Qzss, prn, Some(el), None, Some(snr), true)
        };
        let points = vec![
            point(0, vec![nav(3, 40.0, 45.0), qzs(5, 35.0, 44.0)]),
            // Both lost at t=1 while above the mask -> one slip each.
            point(1, vec![]),
        ];
        let s = slip_rate_series(&points, 15.0, 10.0, 1.0);
        assert_eq!(s.navic, vec![[0.0, 0.0], [1.0, 1.0]]);
        assert_eq!(s.qzss, vec![[0.0, 0.0], [1.0, 1.0]]);
        assert!(s.gps.iter().all(|p| p[1] == 0.0));
        // Two slips total at the same epoch.
        assert_eq!(s.all, vec![[0.0, 0.0], [1.0, 2.0]]);
    }
}
