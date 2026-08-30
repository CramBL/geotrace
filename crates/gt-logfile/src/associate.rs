//! Placing a log entry on a recorded track: it takes the position of the fix
//! nearest in time, interpolated between the two fixes it falls between.

use chrono::{DateTime, Duration, Utc};
use gt_types::{Latitude, Longitude, PlacedPoint};
use rayon::prelude::*;

use crate::{parse::LogEntry, pool};

/// Entries below which associating a whole log on the calling thread beats
/// handing it to [`pool::log_worker_pool`].
const PARALLEL_ASSOCIATION_MIN_ENTRIES: usize = 16 * 1024;

/// The recorded position at `time`, or `None` when no fix of `fixes` lies
/// within `window` of it.
///
/// `fixes` must be in ascending time order.
pub fn associate_position(
    time: DateTime<Utc>,
    fixes: &[PlacedPoint<'_>],
    window: Duration,
) -> Option<(Latitude, Longitude)> {
    let index = fixes.partition_point(|point| point.fix.tpv.time().utc() <= time);
    let before = index.checked_sub(1).and_then(|i| fixes.get(i));
    let after = fixes.get(index);

    match (before, after) {
        (Some(before), Some(after)) => {
            let gap_before = (time - before.fix.tpv.time().utc()).abs();
            let gap_after = (after.fix.tpv.time().utc() - time).abs();
            if gap_before.min(gap_after) > window {
                return None;
            }
            let span = (after.fix.tpv.time() - before.fix.tpv.time())
                .num_microseconds()
                .unwrap_or(1);
            let elapsed = (time - before.fix.tpv.time().utc())
                .num_microseconds()
                .unwrap_or(0);
            let fraction = if span == 0 {
                0.0f64
            } else {
                elapsed as f64 / span as f64
            };
            let (before_lat, before_lon) = before.resolved_position();
            let (after_lat, after_lon) = after.resolved_position();
            let lat =
                before_lat.as_degrees() * (1.0 - fraction) + after_lat.as_degrees() * fraction;
            let lon =
                before_lon.as_degrees() * (1.0 - fraction) + after_lon.as_degrees() * fraction;
            Some((Latitude::new(lat), Longitude::new(lon)))
        }
        (Some(nearest), None) | (None, Some(nearest)) => {
            if (time - nearest.fix.tpv.time().utc()).abs() > window {
                return None;
            }
            Some(nearest.resolved_position())
        }
        (None, None) => None,
    }
}

/// The position of every entry of `entries` against `fixes`, in entry order,
/// `None` for an entry no fix lies within `window` of.
///
/// `fixes` are those of the log's association target alone: a log is never
/// associated against the fixes of several recordings at once.
pub fn associate_entries(
    entries: &[LogEntry],
    fixes: &[PlacedPoint<'_>],
    window: Duration,
) -> Vec<Option<(Latitude, Longitude)>> {
    let position_of = |entry: &LogEntry| associate_position(entry.timestamp, fixes, window);
    match pool::log_worker_pool() {
        Some(pool) if entries.len() >= PARALLEL_ASSOCIATION_MIN_ENTRIES => {
            pool.install(|| entries.par_iter().map(position_of).collect())
        }
        Some(_) | None => entries.iter().map(position_of).collect(),
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;
    use gt_test_utils::nav_points_from;
    use gt_types::LoadedTrack;
    use rstest::rstest;

    use super::*;
    use crate::{TextSlice, TimestampKind};

    /// `count` fixes a second apart from `start()`, as a track of their own.
    fn track_of(count: usize) -> LoadedTrack {
        gt_test_utils::loaded_track_with_points(nav_points_from(start(), count, 1))
    }

    /// The track's fixes with where the builder places each of them.
    fn placed_fixes(track: &LoadedTrack) -> Vec<PlacedPoint<'_>> {
        track
            .placed_points()
            .expect("every fixture fix has a recorded position")
            .iter()
            .collect()
    }

    /// The window every test runs with, matching the app's default.
    fn window() -> Duration {
        Duration::seconds(60)
    }

    fn start() -> DateTime<Utc> {
        Utc.with_ymd_and_hms(2026, 1, 1, 0, 0, 0)
            .single()
            .expect("valid")
    }

    #[test]
    fn a_time_between_two_fixes_lands_between_their_positions() {
        let track = track_of(5);
        let time = start() + Duration::milliseconds(500);
        let (lat, lon) =
            associate_position(time, &placed_fixes(&track), window()).expect("associates");
        assert!((lat.as_degrees() - 55.0005).abs() < 1e-9);
        assert!((lon.as_degrees() - 12.0005).abs() < 1e-9);
    }

    #[test]
    fn a_time_on_a_fix_takes_that_fixs_position() {
        let track = track_of(5);
        let (lat, lon) =
            associate_position(start(), &placed_fixes(&track), window()).expect("associates");
        assert!((lat.as_degrees() - 55.0).abs() < 1e-9);
        assert!((lon.as_degrees() - 12.0).abs() < 1e-9);
    }

    /// The five fixes run from `start()` to `start() + 4 s`.
    #[rstest]
    #[case::just_after_the_last_fix(4 + 59, true)]
    #[case::past_the_window_after_the_last_fix(4 + 61, false)]
    #[case::just_before_the_first_fix(-30, true)]
    #[case::past_the_window_before_the_first_fix(-61, false)]
    fn a_time_outside_the_recording_associates_only_within_the_window(
        #[case] offset_secs: i64,
        #[case] associates: bool,
    ) {
        let track = track_of(5);
        let time = start() + Duration::seconds(offset_secs);
        assert_eq!(
            associate_position(time, &placed_fixes(&track), window()).is_some(),
            associates
        );
    }

    /// The pool splits a log this long across workers: what it returns must be
    /// what one thread walking the entries in order returns.
    #[test]
    fn a_log_long_enough_for_the_pool_associates_as_one_thread_does() {
        let entry_count = PARALLEL_ASSOCIATION_MIN_ENTRIES + 1;
        let track = track_of(1 + entry_count / 1000);
        let points = placed_fixes(&track);
        let entries: Vec<LogEntry> = (0..entry_count)
            .map(|index| LogEntry {
                timestamp: start() + Duration::milliseconds(index as i64),
                timestamp_kind: TimestampKind::Anchored,
                line_number: 1,
                message: TextSlice { offset: 0, len: 0 },
            })
            .collect();

        let associated = associate_entries(&entries, &points, window());

        assert_eq!(
            associated,
            entries
                .iter()
                .map(|entry| associate_position(entry.timestamp, &points, window()))
                .collect::<Vec<_>>()
        );
        assert!(
            associated.iter().all(Option::is_some),
            "every entry of the fixture falls inside the recording"
        );
    }

    #[test]
    fn a_recording_without_fixes_associates_nothing() {
        assert!(associate_position(start(), &[], window()).is_none());
    }
}
