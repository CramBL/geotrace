//! Placing a log entry on a recorded track: it takes the position of the fix
//! nearest in time, interpolated along the great circle between the two fixes
//! it falls between, and is attributed to the nearer of those two.

use chrono::{DateTime, Duration, Utc};
use gt_geo_math::GreatCircleArc;
use gt_types::{AddressedFix, FixRef, Latitude, Longitude};
use rayon::prelude::*;

use crate::{parse::LogEntry, pool};

/// Entries below which associating a whole log on the calling thread beats
/// handing it to [`pool::log_worker_pool`].
const PARALLEL_ASSOCIATION_MIN_ENTRIES: usize = 16 * 1024;

/// Where an entry sits on the recording it was associated against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntryPlacement {
    pub position: (Latitude, Longitude),

    /// The fix the entry is attributed to: the one nearest in time, and the
    /// earlier one when the entry falls exactly between two fixes.
    pub fix: FixRef,
}

/// Where `time` sits on `fixes`, or `None` when no fix of `fixes` lies within
/// `window` of it.
///
/// `fixes` must be in ascending time order.
pub fn associate_position(
    time: DateTime<Utc>,
    fixes: &[AddressedFix<'_>],
    window: Duration,
) -> Option<EntryPlacement> {
    let fix_time = |fix: &AddressedFix<'_>| fix.placed.fix.tpv.time().utc();
    let index = fixes.partition_point(|fix| fix_time(fix) <= time);
    let before = index.checked_sub(1).and_then(|i| fixes.get(i));
    let after = fixes.get(index);

    match (before, after) {
        (Some(before), Some(after)) => {
            let gap_before = (time - fix_time(before)).abs();
            let gap_after = (fix_time(after) - time).abs();
            if gap_before.min(gap_after) > window {
                return None;
            }
            let span = (fix_time(after) - fix_time(before))
                .num_microseconds()
                .unwrap_or(1);
            let elapsed = (time - fix_time(before)).num_microseconds().unwrap_or(0);
            let fraction = if span == 0 {
                0.0f64
            } else {
                elapsed as f64 / span as f64
            };
            let nearer = if gap_before <= gap_after {
                before
            } else {
                after
            };
            Some(EntryPlacement {
                position: GreatCircleArc {
                    start: before.placed.resolved_position(),
                    end: after.placed.resolved_position(),
                }
                .position_at_ratio(fraction),
                fix: nearer.fix,
            })
        }
        (Some(nearest), None) | (None, Some(nearest)) => {
            if (time - fix_time(nearest)).abs() > window {
                return None;
            }
            Some(EntryPlacement {
                position: nearest.placed.resolved_position(),
                fix: nearest.fix,
            })
        }
        (None, None) => None,
    }
}

/// Where every entry of `entries` sits on `fixes`, in entry order, `None` for
/// an entry no fix lies within `window` of.
///
/// `fixes` are those of the recording the log is anchored to alone: a log is
/// never associated against the fixes of several recordings at once.
pub fn associate_entries(
    entries: &[LogEntry],
    fixes: &[AddressedFix<'_>],
    window: Duration,
) -> Vec<Option<EntryPlacement>> {
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
    use gt_types::{FileIdx, LoadedTrack, PointIdx, TrackIdx, TrackRef};
    use rstest::rstest;

    use super::*;
    use crate::{TextSlice, TimestampKind};

    /// `count` fixes a second apart from `start()`, as a track of their own.
    fn track_of(count: usize) -> LoadedTrack {
        gt_test_utils::loaded_track_with_points(nav_points_from(start(), count, 1))
    }

    /// The track's fixes with where the builder places each of them,
    /// addressed as the only track of the only loaded recording.
    fn placed_fixes(track: &LoadedTrack) -> Vec<AddressedFix<'_>> {
        track
            .placed_points()
            .expect("every fixture fix has a recorded position")
            .iter()
            .enumerate()
            .map(|(pi, placed)| AddressedFix {
                fix: FixRef::new(
                    TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
                    PointIdx::new(pi),
                ),
                placed,
            })
            .collect()
    }

    /// Positions this close are the same place to within a centimetre, which
    /// covers the great circle's departure from a straight line in degrees
    /// over the fixtures' steps.
    const POSITION_TOLERANCE_DEGREES: f64 = 1e-7;

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
        let (lat, lon) = associate_position(time, &placed_fixes(&track), window())
            .expect("associates")
            .position;
        assert!((lat.as_degrees() - 55.0005).abs() < POSITION_TOLERANCE_DEGREES);
        assert!((lon.as_degrees() - 12.0005).abs() < POSITION_TOLERANCE_DEGREES);
    }

    /// Two fixes a second apart, 0.2 deg of longitude apart across the date
    /// line, on the equator.
    fn track_across_the_antimeridian() -> LoadedTrack {
        gt_test_utils::loaded_track_with_points(gt_test_utils::nav_points_at_positions(
            start(),
            &[
                (Latitude::new(0.0), Longitude::new(179.9)),
                (Latitude::new(0.0), Longitude::new(-179.9)),
            ],
        ))
    }

    /// Every position on the great circle between the two fixes lies at a
    /// longitude of at least 179.9 deg either side, since it runs over the
    /// date line.
    #[test]
    fn a_time_between_fixes_across_the_antimeridian_is_placed_between_them() {
        let track = track_across_the_antimeridian();
        let time = start() + Duration::milliseconds(500);
        let (lat, lon) = associate_position(time, &placed_fixes(&track), window())
            .expect("associates")
            .position;
        let lon = lon.as_degrees();
        assert!(
            lon.abs() >= 179.9,
            "entry placed at lon {lon}, expected it between 179.9 and -179.9 across the date line"
        );
        assert!(lat.as_degrees().abs() < POSITION_TOLERANCE_DEGREES);
    }

    #[rstest]
    #[case::walking_north_east(track_of(5), (Latitude::new(55.0), Longitude::new(12.0)))]
    #[case::across_the_antimeridian(
        track_across_the_antimeridian(),
        (Latitude::new(0.0), Longitude::new(179.9))
    )]
    fn a_time_on_a_fix_takes_that_fixs_position(
        #[case] track: LoadedTrack,
        #[case] (expected_lat, expected_lon): (Latitude, Longitude),
    ) {
        let (lat, lon) = associate_position(start(), &placed_fixes(&track), window())
            .expect("associates")
            .position;
        assert!((lat.as_degrees() - expected_lat.as_degrees()).abs() < POSITION_TOLERANCE_DEGREES);
        assert!((lon.as_degrees() - expected_lon.as_degrees()).abs() < POSITION_TOLERANCE_DEGREES);
    }

    /// The five fixes run from `start()` to `start() + 4 s`, and the entry
    /// exactly between two of them takes the earlier one.
    #[rstest]
    #[case::nearer_the_fix_before(Duration::milliseconds(400), 0)]
    #[case::nearer_the_fix_after(Duration::milliseconds(600), 1)]
    #[case::exactly_between_two_fixes(Duration::milliseconds(500), 0)]
    #[case::before_the_first_fix(Duration::seconds(-30), 0)]
    #[case::after_the_last_fix(Duration::seconds(30), 4)]
    fn an_entry_is_attributed_to_the_fix_nearest_in_time(
        #[case] offset: Duration,
        #[case] expected_point: usize,
    ) {
        let track = track_of(5);
        let placement = associate_position(start() + offset, &placed_fixes(&track), window())
            .expect("associates");
        assert_eq!(placement.fix.point, PointIdx::new(expected_point));
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
