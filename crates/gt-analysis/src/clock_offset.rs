//! Clock offset excursions: the samples whose GPS−system clock offset leaves
//! the track's baseline and returns to it.
//!
//! A receiver resuming from a recording gap can report a pre-gap GPS epoch for
//! its first fix while the host stamps that fix on resume, which puts the whole
//! gap into a single sample's offset.  A tracker whose clock starts each boot
//! at its default holds that default over the fixes until the receiver corrects
//! it.  Those offsets are real and stay in the data.  Separating them here lets
//! a plot keep them off the shared y-axis and mark them explicitly.
//!
//! An offset that steps and stays to the end of the track is a clock
//! discontinuity, not an excursion, and is deliberately not matched here: a run
//! of out-of-band samples is an excursion only when the offset returns to the
//! baseline after it.  That leaves the level shifts on the line where they
//! belong.

use gt_types::nav_point::NavPoint;
use vec1::Vec1;

/// Default deviation from the track baseline, in seconds, above which a sample
/// counts as an excursion.
///
/// Well above the sub-second offsets of a healthy logger and the few-second
/// offsets of a host clock left to drift, and far below the minutes-to-hours
/// excursions a resume-from-gap sample produces.
pub const DEFAULT_EXCURSION_THRESHOLD_S: f32 = 10.0;

/// Ceiling on the configured threshold, in seconds.  The clamp is what keeps
/// the conversion to milliseconds in range.  Any deviation this large is beyond
/// what a plot axis can meaningfully show anyway.
const MAX_THRESHOLD_S: f32 = 86_400.0;

/// One sample whose offset sits outside the baseline band.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ExcursionSample {
    /// Index of the sample's nav point within the track.
    pub index: usize,
    /// Epoch, in Unix seconds (the marker's x-position).
    pub t: f64,
    /// GPS−system offset at this sample, in milliseconds.
    pub offset_ms: i64,
}

/// A run of consecutive samples that departed from the track's baseline offset
/// and returned to it.
#[derive(Debug, Clone, PartialEq)]
pub struct ClockOffsetExcursion {
    /// The out-of-band samples, in ascending index order.
    pub samples: Vec1<ExcursionSample>,
    /// The track's baseline offset (median over all its samples), in
    /// milliseconds - what the offset departed from and returns to.
    pub baseline_ms: i64,
}

impl ClockOffsetExcursion {
    /// The sample that departed furthest from the baseline.
    pub fn peak(&self) -> &ExcursionSample {
        self.samples
            .iter()
            .max_by_key(|s| deviation(s.offset_ms, self.baseline_ms).saturating_abs())
            .unwrap_or_else(|| self.samples.first())
    }

    /// Signed departure of the [`Self::peak`] sample from the baseline, in
    /// milliseconds.
    pub fn deviation_ms(&self) -> i64 {
        deviation(self.peak().offset_ms, self.baseline_ms)
    }
}

/// Nav-point indices covered by `excursions`, ascending - the samples a plot
/// keeps off its line and marks on its own.
pub fn excursion_indices(excursions: &[ClockOffsetExcursion]) -> Vec<usize> {
    excursions
        .iter()
        .flat_map(|e| e.samples.iter().map(|s| s.index))
        .collect()
}

/// Find the clock offset excursions in `points`, using `threshold_s` seconds of
/// deviation from the track's baseline as the bar.
///
/// The baseline is the median offset over the whole track, so a large but
/// *steady* offset (a host clock hours off all recording) sits at the baseline
/// and is never flagged - only departures from a track's own normal are.  A run
/// of out-of-band samples counts as a departure only once an in-band sample
/// follows it: a run that reaches the last sample is a level shift the line
/// keeps.
///
/// Returned in ascending sample order.  A track whose samples are more than
/// half out-of-band yields nothing: with that little agreement there is no
/// baseline to depart from.
pub fn detect_excursions(points: &[NavPoint], threshold_s: f32) -> Vec<ClockOffsetExcursion> {
    let samples: Vec<ExcursionSample> = points
        .iter()
        .enumerate()
        .filter_map(|(index, p)| {
            Some(ExcursionSample {
                index,
                t: p.tpv.time().as_secs_f64_with_subseconds(),
                offset_ms: p.tpv.gps_system_clock_offset()?.num_milliseconds(),
            })
        })
        .collect();

    let sample_count = samples.len();
    let offsets: Vec<i64> = samples.iter().map(|s| s.offset_ms).collect();
    let Some(baseline_ms) = crate::robust::median_i64(&offsets) else {
        return Vec::new();
    };
    let threshold_ms = threshold_ms(threshold_s);

    let mut excursions = Vec::new();
    let mut run: Vec<ExcursionSample> = Vec::new();
    for sample in samples {
        if deviation(sample.offset_ms, baseline_ms).saturating_abs() > threshold_ms {
            run.push(sample);
            continue;
        }
        // The run before this sample departed and returned, now that this
        // sample is back at the baseline.  A run still open when the loop ends
        // never returned: that is a level shift, and the line keeps it.
        if let Ok(samples) = Vec1::try_from_vec(std::mem::take(&mut run)) {
            excursions.push(ClockOffsetExcursion {
                samples,
                baseline_ms,
            });
        }
    }

    // The total is checked too: a median only stands for a baseline while most
    // of the track lies near it, and on a track split evenly between two levels
    // the median lands between them and each run returns to it.
    let flagged: usize = excursions.iter().map(|e| e.samples.len()).sum();
    if flagged * 2 > sample_count {
        return Vec::new();
    }
    excursions
}

/// Signed departure of `offset_ms` from `baseline_ms`.  Saturating: offsets come
/// from a parsed binary format, so the arithmetic is kept structurally
/// overflow-proof for adversarial timestamps.
fn deviation(offset_ms: i64, baseline_ms: i64) -> i64 {
    offset_ms.saturating_sub(baseline_ms)
}

/// The configured threshold in milliseconds, clamped to [`MAX_THRESHOLD_S`].
fn threshold_ms(threshold_s: f32) -> i64 {
    let ms = f64::from(threshold_s.clamp(0.0, MAX_THRESHOLD_S)) * 1000.0;
    #[expect(
        clippy::cast_possible_truncation,
        reason = "clamped to MAX_THRESHOLD_S above, so the product is far inside i64"
    )]
    let ms = ms as i64;
    ms
}

#[cfg(test)]
mod tests {
    use chrono::{TimeZone, Utc};
    use gt_types::coordinates::{Latitude, Longitude};
    use gt_types::time_types::{FixTimestamp, GpsTime, SysTime};
    use gt_types::tpv::TimePositionVelocity;

    use super::*;

    /// A point at GPS second `gps_secs` whose system clock is `sys_ahead_ms`
    /// ahead of GPS (so the GPS−system offset is `-sys_ahead_ms`).
    fn point(gps_secs: i64, sys_ahead_ms: i64) -> NavPoint {
        let gps = GpsTime::from_utc(Utc.timestamp_opt(gps_secs, 0).single().expect("valid"));
        let sys = SysTime::from_utc(
            Utc.timestamp_millis_opt(gps_secs * 1000 + sys_ahead_ms)
                .single()
                .expect("valid"),
        );
        let tpv = TimePositionVelocity::builder()
            .time(gps)
            .lat(Latitude::new(55.0))
            .lon(Longitude::new(12.0))
            .sys_time(sys)
            .build();
        NavPoint::new(tpv, None)
    }

    /// The host clock stamped this point at `sys_secs`, the receiver having
    /// reported no GPS time.
    fn point_without_gps_lock(sys_secs: i64) -> NavPoint {
        let host = SysTime::from_utc(Utc.timestamp_opt(sys_secs, 0).single().expect("valid"));
        let tpv = TimePositionVelocity::builder()
            .time(FixTimestamp::FromHostClock(host))
            .lat(Latitude::new(55.0))
            .lon(Longitude::new(12.0))
            .sys_time(host)
            .build();
        NavPoint::new(tpv, None)
    }

    /// A point with no system timestamp - no offset to measure.
    fn point_without_sys(gps_secs: i64) -> NavPoint {
        let gps = GpsTime::from_utc(Utc.timestamp_opt(gps_secs, 0).single().expect("valid"));
        let tpv = TimePositionVelocity::builder()
            .time(gps)
            .lat(Latitude::new(55.0))
            .lon(Longitude::new(12.0))
            .build();
        NavPoint::new(tpv, None)
    }

    /// A steady offset with one sample carrying a whole recording gap - the
    /// `gnss.h5.gtd` shape, where the receiver reported a pre-gap GPS epoch for
    /// its first fix after resuming.
    fn resume_from_gap() -> Vec<NavPoint> {
        vec![
            point(1000, 210),
            point(1001, 227),
            point(1002, 240),
            point(1003, 234),
            point(1004, 4_127_054),
            point(1005, 240),
            point(1006, 215),
            point(1007, 235),
        ]
    }

    #[test]
    fn a_resume_from_gap_sample_is_one_excursion() {
        let excursions = detect_excursions(&resume_from_gap(), DEFAULT_EXCURSION_THRESHOLD_S);
        let [excursion] = excursions.as_slice() else {
            panic!("expected exactly one excursion, got {}", excursions.len());
        };
        assert_eq!(excursion.samples.len(), 1);
        assert_eq!(excursion.peak().index, 4);
        assert_eq!(excursion.peak().offset_ms, -4_127_054);
        assert_eq!(excursion.baseline_ms, -234);
        assert_eq!(excursion.deviation_ms(), -4_126_820);
        assert_eq!(excursion_indices(&excursions), vec![4]);
    }

    #[test]
    fn a_steady_large_offset_is_the_baseline_not_an_excursion() {
        // Host clock five minutes behind GPS for the whole track.
        let points: Vec<NavPoint> = (0..8).map(|i| point(1000 + i, -300_000)).collect();
        assert!(detect_excursions(&points, DEFAULT_EXCURSION_THRESHOLD_S).is_empty());
    }

    #[test]
    fn a_permanent_step_is_left_on_the_line() {
        let mut points: Vec<NavPoint> = (0..6).map(|i| point(1000 + i, 200)).collect();
        points.extend((6..12).map(|i| point(1000 + i, 3_600_000)));
        assert!(detect_excursions(&points, DEFAULT_EXCURSION_THRESHOLD_S).is_empty());
    }

    /// A tracker holds its RTC default for tens of fixes after each boot,
    /// until the receiver corrects the host clock. Every one of those fixes
    /// belongs to the same departure from the track's baseline.
    #[test]
    fn a_long_run_that_returns_to_the_baseline_is_one_excursion() {
        const RUN_LEN: i64 = 40;
        let mut points: Vec<NavPoint> = (0..RUN_LEN).map(|i| point(1000 + i, 3_600_000)).collect();
        points.extend((0..160).map(|i| point(1000 + RUN_LEN + i, 200)));
        let excursions = detect_excursions(&points, DEFAULT_EXCURSION_THRESHOLD_S);
        let [excursion] = excursions.as_slice() else {
            panic!("expected one excursion, got {}", excursions.len());
        };
        assert_eq!(excursion.samples.len() as i64, RUN_LEN);
    }

    #[test]
    fn two_separate_excursions_stay_separate() {
        let mut points: Vec<NavPoint> = (0..4).map(|i| point(1000 + i, 200)).collect();
        points.push(point(1004, 3_600_000));
        points.extend((0..4).map(|i| point(1005 + i, 200)));
        points.push(point(1009, -3_600_000));
        points.extend((0..4).map(|i| point(1010 + i, 200)));
        let excursions = detect_excursions(&points, DEFAULT_EXCURSION_THRESHOLD_S);
        assert_eq!(
            excursions
                .iter()
                .map(|e| e.peak().index)
                .collect::<Vec<_>>(),
            vec![4, 9]
        );
    }

    #[test]
    fn the_threshold_determines_what_counts_as_a_departure() {
        let mut points: Vec<NavPoint> = (0..8).map(|i| point(1000 + i, 200)).collect();
        points.push(point(1008, 30_200));
        points.extend((0..8).map(|i| point(1009 + i, 200)));
        assert_eq!(detect_excursions(&points, 10.0).len(), 1, "30 s > 10 s bar");
        assert!(
            detect_excursions(&points, 60.0).is_empty(),
            "30 s stays inside a 60 s bar"
        );
    }

    /// A fix taken without a GPS lock has a structural zero GPS−system
    /// difference: its time field holds the host timestamp. That zero is no
    /// departure from the baseline of a track whose host clock runs an hour
    /// behind GPS. A real departure beside it still is one.
    #[test]
    fn a_fix_without_a_gps_lock_is_not_an_excursion() {
        const HOST_BEHIND_S: i64 = 3600;
        let behind_ms = -HOST_BEHIND_S * 1000;
        let mut points: Vec<NavPoint> = (0..4).map(|i| point(1000 + i, behind_ms)).collect();
        points.push(point_without_gps_lock(1004 - HOST_BEHIND_S));
        points.extend((5..9).map(|i| point(1000 + i, behind_ms)));
        points.push(point(1009, behind_ms + 60_000));
        points.extend((10..14).map(|i| point(1000 + i, behind_ms)));

        let excursions = detect_excursions(&points, DEFAULT_EXCURSION_THRESHOLD_S);

        assert_eq!(excursion_indices(&excursions), vec![9]);
    }

    #[test]
    fn samples_without_a_system_timestamp_are_skipped() {
        let mut points: Vec<NavPoint> = (0..4).map(|i| point(1000 + i, 200)).collect();
        points.push(point_without_sys(1004));
        points.push(point(1005, 3_600_000));
        points.extend((0..4).map(|i| point(1006 + i, 200)));
        let excursions = detect_excursions(&points, DEFAULT_EXCURSION_THRESHOLD_S);
        let [excursion] = excursions.as_slice() else {
            panic!("expected exactly one excursion, got {}", excursions.len());
        };
        assert_eq!(excursion.peak().index, 5, "index is into the nav points");
    }

    /// A track that opens out of band has no baseline sample before its first
    /// run. The return after it is what makes that run an excursion.
    #[test]
    fn a_run_at_the_start_of_a_track_is_an_excursion() {
        let mut points: Vec<NavPoint> = (0..2).map(|i| point(1000 + i, 3_600_000)).collect();
        points.extend((0..3).map(|i| point(1002 + i, 200)));
        let excursions = detect_excursions(&points, DEFAULT_EXCURSION_THRESHOLD_S);
        let [excursion] = excursions.as_slice() else {
            panic!("expected one excursion, got {}", excursions.len());
        };
        assert_eq!(
            excursion
                .samples
                .iter()
                .map(|s| s.index)
                .collect::<Vec<_>>(),
            vec![0, 1]
        );
    }

    /// Which level is the baseline is determined by the majority, not by which
    /// came first: six samples at one offset and two at another makes the two
    /// the excursion, however large the gap between them.
    #[test]
    fn the_majority_of_a_track_defines_its_baseline() {
        let mut points: Vec<NavPoint> = (0..3).map(|i| point(1000 + i, 3_600_000)).collect();
        points.extend((0..2).map(|i| point(1003 + i, 200)));
        points.extend((0..3).map(|i| point(1005 + i, 3_600_000)));
        let excursions = detect_excursions(&points, DEFAULT_EXCURSION_THRESHOLD_S);
        let [excursion] = excursions.as_slice() else {
            panic!("expected one excursion, got {}", excursions.len());
        };
        assert_eq!(excursion.baseline_ms, -3_600_000);
        assert_eq!(
            excursion
                .samples
                .iter()
                .map(|s| s.index)
                .collect::<Vec<_>>(),
            vec![3, 4]
        );
    }

    #[test]
    fn an_evenly_split_track_has_no_baseline_to_depart_from() {
        // The median lands between the two levels, so every sample is out of
        // band. Neither half is the track's normal, so neither is an excursion.
        let mut points: Vec<NavPoint> = (0..3).map(|i| point(1000 + i, 200)).collect();
        points.extend((0..3).map(|i| point(1003 + i, 3_600_000)));
        assert!(detect_excursions(&points, DEFAULT_EXCURSION_THRESHOLD_S).is_empty());
    }

    #[test]
    fn a_track_with_too_few_samples_yields_nothing() {
        // Two samples, one wild: there is no majority to form a baseline, so
        // neither is called an excursion.
        let points = vec![point(1000, 200), point(1001, 3_600_000)];
        assert!(detect_excursions(&points, DEFAULT_EXCURSION_THRESHOLD_S).is_empty());
    }
}
