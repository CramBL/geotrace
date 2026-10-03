//! Interpolation requires adjacent fixes from the same track. At a track boundary,
//! association uses the nearest fix position within the time window.

use chrono::{DateTime, Duration, Utc};
use gt_geo_math::GreatCircleArc;
use gt_loaded_files::LoadedFileEntry;
use gt_types::{AddressedFix, FixRef, Latitude, Longitude};
use rayon::prelude::*;

use crate::{parse::LogEntry, pool};

/// Where an entry sits on the recording it was associated against.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EntryPlacement {
    pub position: (Latitude, Longitude),

    /// The fix the entry is attributed to: the one nearest in time, and the
    /// earlier one when the entry falls exactly between two fixes.
    pub fix: FixRef,
}

pub struct RecordingAssociationIndex<'a> {
    fixes: Vec<AddressedFix<'a>>,
}

impl<'a> RecordingAssociationIndex<'a> {
    pub fn from_recording(recording: LoadedFileEntry<'a>) -> Self {
        let mut fixes = recording.addressed_fixes();
        fixes.sort_by_key(|fix| fix.placed.fix.tpv.time().utc());
        Self { fixes }
    }

    pub fn position_at(&self, time: DateTime<Utc>, window: Duration) -> Option<EntryPlacement> {
        let fixes = &self.fixes;
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
                let nearer = if gap_before <= gap_after {
                    before
                } else {
                    after
                };
                if before.fix.track != after.fix.track {
                    return Some(EntryPlacement {
                        position: nearer.placed.resolved_position(),
                        fix: nearer.fix,
                    });
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

    pub fn associate_entries(
        &self,
        entries: &[LogEntry],
        window: Duration,
    ) -> Vec<Option<EntryPlacement>> {
        let position_of = |entry: &LogEntry| self.position_at(entry.timestamp, window);
        match pool::log_worker_pool() {
            Some(pool) if entries.len() >= PARALLEL_ASSOCIATION_MIN_ENTRIES => {
                pool.install(|| entries.par_iter().map(position_of).collect())
            }
            Some(_) | None => entries.iter().map(position_of).collect(),
        }
    }
}

/// Entries below which associating a whole log on the calling thread beats
/// handing it to [`pool::log_worker_pool`].
const PARALLEL_ASSOCIATION_MIN_ENTRIES: usize = 16 * 1024;

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;
    use gt_loaded_files::FileHistory;
    use gt_test_utils::fixtures;
    use gt_types::{FileIdx, LoadedTrack, PointIdx, TrackIdx, TrackRef};
    use proptest::{prelude::*, proptest};
    use rstest::rstest;

    use super::*;
    use crate::{TextSlice, TimestampKind, test_util};

    /// `count` fixes a second apart from `start()`, as a track of their own.
    fn track_of(count: usize) -> LoadedTrack {
        gt_test_utils::loaded_track_with_points(fixtures::nav_points_from(start(), count, 1))
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

    /// One entry logged at `time`, which is all the association reads of it.
    fn entry_at(time: DateTime<Utc>) -> LogEntry {
        LogEntry {
            timestamp: time,
            timestamp_kind: TimestampKind::Anchored,
            line_number: 1,
            message: TextSlice { offset: 0, len: 0 },
        }
    }

    #[test]
    fn a_time_between_two_fixes_lands_between_their_positions() {
        let files = test_util::loaded_recording(vec![track_of(5)]);
        let index =
            RecordingAssociationIndex::from_recording(files.view().get(0).expect("loaded fixture"));
        let time = start() + Duration::milliseconds(500);
        let (lat, lon) = index
            .position_at(time, window())
            .expect("associates")
            .position;
        assert!((lat.as_degrees() - 55.0005).abs() < POSITION_TOLERANCE_DEGREES);
        assert!((lon.as_degrees() - 12.0005).abs() < POSITION_TOLERANCE_DEGREES);
    }

    #[rstest]
    #[case::nearer_before(50, Duration::seconds(60), Some(1))]
    #[case::nearer_after(70, Duration::seconds(60), Some(2))]
    #[case::tie_uses_before(60, Duration::seconds(60), Some(1))]
    #[case::before_at_window_limit(50, Duration::seconds(40), Some(1))]
    #[case::after_at_window_limit(70, Duration::seconds(40), Some(2))]
    #[case::outside_window(60, Duration::seconds(49), None)]
    #[case::exact_boundary_fix_with_zero_window(10, Duration::zero(), Some(1))]
    fn an_entry_between_tracks_uses_the_nearest_fix_position_within_the_window(
        #[case] offset_secs: i64,
        #[case] window: Duration,
        #[case] expected_fix_index: Option<usize>,
    ) {
        let tracks = vec![
            gt_test_utils::loaded_track_with_points(fixtures::nav_points_stamped(
                start(),
                &[
                    (Latitude::new(54.0), Longitude::new(11.0)),
                    (Latitude::new(55.0), Longitude::new(12.0)),
                ],
                |index| index as i64 * 10,
            )),
            gt_test_utils::loaded_track_with_points(fixtures::nav_points_stamped(
                start() + Duration::seconds(110),
                &[
                    (Latitude::new(-33.0), Longitude::new(151.0)),
                    (Latitude::new(-34.0), Longitude::new(152.0)),
                ],
                |index| index as i64 * 10,
            )),
        ];
        let files = test_util::loaded_recording(tracks);
        let recording = files.view().get(0).expect("loaded fixture");
        let fixes = recording.addressed_fixes();
        let index = RecordingAssociationIndex::from_recording(recording);
        let expected = expected_fix_index.map(|index| {
            let fix = fixes.get(index).expect("expected fixture fix exists");
            EntryPlacement {
                position: fix.placed.resolved_position(),
                fix: fix.fix,
            }
        });

        assert_eq!(
            index.associate_entries(
                &[entry_at(start() + Duration::seconds(offset_secs))],
                window,
            ),
            vec![expected],
        );
    }

    /// Two fixes a second apart, 0.2 deg of longitude apart across the date
    /// line, on the equator.
    fn track_across_the_antimeridian() -> LoadedTrack {
        gt_test_utils::loaded_track_with_points(gt_test_utils::fixtures::nav_points_stamped(
            start(),
            &[
                (Latitude::new(0.0), Longitude::new(179.9)),
                (Latitude::new(0.0), Longitude::new(-179.9)),
            ],
            |index| index as i64,
        ))
    }

    /// Every position on the great circle between the two fixes lies at a
    /// longitude of at least 179.9 deg either side, since it runs over the
    /// date line.
    #[test]
    fn a_time_between_fixes_across_the_antimeridian_is_placed_between_them() {
        let files = test_util::loaded_recording(vec![track_across_the_antimeridian()]);
        let index =
            RecordingAssociationIndex::from_recording(files.view().get(0).expect("loaded fixture"));
        let time = start() + Duration::milliseconds(500);
        let (lat, lon) = index
            .position_at(time, window())
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
        let files = test_util::loaded_recording(vec![track]);
        let index =
            RecordingAssociationIndex::from_recording(files.view().get(0).expect("loaded fixture"));
        let (lat, lon) = index
            .position_at(start(), window())
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
        let files = test_util::loaded_recording(vec![track_of(5)]);
        let index =
            RecordingAssociationIndex::from_recording(files.view().get(0).expect("loaded fixture"));
        let placement = index
            .position_at(start() + offset, window())
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
        let files = test_util::loaded_recording(vec![track_of(5)]);
        let index =
            RecordingAssociationIndex::from_recording(files.view().get(0).expect("loaded fixture"));
        let time = start() + Duration::seconds(offset_secs);
        assert_eq!(index.position_at(time, window()).is_some(), associates);
    }

    /// The pool splits a log this long across workers: what it returns must be
    /// what one thread walking the entries in order returns.
    #[test]
    fn a_log_long_enough_for_the_pool_associates_as_one_thread_does() {
        let entry_count = PARALLEL_ASSOCIATION_MIN_ENTRIES + 1;
        let files = test_util::loaded_recording(vec![track_of(1 + entry_count / 1000)]);
        let index =
            RecordingAssociationIndex::from_recording(files.view().get(0).expect("loaded fixture"));
        let entries: Vec<LogEntry> = (0..entry_count)
            .map(|index| entry_at(start() + Duration::milliseconds(index as i64)))
            .collect();

        let associated = index.associate_entries(&entries, window());

        assert_eq!(
            associated,
            entries
                .iter()
                .map(|entry| index.position_at(entry.timestamp, window()))
                .collect::<Vec<_>>()
        );
        assert!(
            associated.iter().all(Option::is_some),
            "every entry of the fixture falls inside the recording"
        );
    }

    #[test]
    fn a_recording_without_fixes_associates_nothing() {
        let files = test_util::loaded_recording(Vec::new());
        let index =
            RecordingAssociationIndex::from_recording(files.view().get(0).expect("loaded fixture"));
        assert!(index.position_at(start(), window()).is_none());
    }

    #[test]
    fn construction_orders_one_recordings_fixes_and_preserves_equal_time_track_addresses() {
        let mut files = test_util::loaded_recording(vec![track_of(5)]);
        files.push(
            gt_test_utils::loaded_file_with_tracks(vec![
                gt_test_utils::loaded_track_with_points(fixtures::nav_points_stamped(
                    start(),
                    &[
                        (Latitude::new(54.0), Longitude::new(11.0)),
                        (Latitude::new(55.0), Longitude::new(12.0)),
                    ],
                    |index| 20 - index as i64 * 10,
                )),
                gt_test_utils::loaded_track_with_points(fixtures::nav_points_stamped(
                    start(),
                    &[
                        (Latitude::new(-33.0), Longitude::new(151.0)),
                        (Latitude::new(-34.0), Longitude::new(152.0)),
                    ],
                    |index| index as i64 * 10,
                )),
            ]),
            FileHistory::None,
        );
        let recording = files.view().get(1).expect("loaded fixture");
        let index = RecordingAssociationIndex::from_recording(recording);
        assert_eq!(
            index
                .fixes
                .iter()
                .map(|fix| (fix.placed.fix.tpv.time().utc(), fix.fix))
                .collect::<Vec<_>>(),
            [(0, 1, 0), (10, 0, 1), (10, 1, 1), (20, 0, 0)].map(|(seconds, track, point)| (
                start() + Duration::seconds(seconds),
                FixRef::new(
                    TrackRef::new(FileIdx::new(1), TrackIdx::new(track)),
                    PointIdx::new(point)
                )
            )),
        );
        assert_eq!(
            index.position_at(start() + Duration::seconds(10), Duration::zero()),
            Some(EntryPlacement {
                position: (Latitude::new(-34.0), Longitude::new(152.0)),
                fix: FixRef::new(
                    TrackRef::new(FileIdx::new(1), TrackIdx::new(1)),
                    PointIdx::new(1)
                ),
            }),
        );
        assert_eq!(
            recording
                .addressed_fixes()
                .first()
                .expect("first stored fix")
                .placed
                .fix
                .tpv
                .time()
                .utc(),
            start() + Duration::seconds(20)
        );
    }

    proptest! {
        /// Every entry takes the fix nearest its timestamp, the earlier one
        /// where two lie equally close, and no position at all where the
        /// nearest lies further from it than the window.
        #[test]
        fn an_entry_takes_the_nearest_fix_within_the_window_or_no_position(
            fix_count in 1usize..8,
            entry_offsets_secs in prop::collection::vec(-120i64..120, 0..12),
            window_secs in 0i64..30,
        ) {
            let files = test_util::loaded_recording(vec![track_of(fix_count)]);
            let recording = files.view().get(0).expect("loaded fixture");
            let fixes = recording.addressed_fixes();
            let index = RecordingAssociationIndex::from_recording(recording);
            let window = Duration::seconds(window_secs);
            let entries: Vec<LogEntry> = entry_offsets_secs
                .iter()
                .map(|offset| entry_at(start() + Duration::seconds(*offset)))
                .collect();

            let placements = index.associate_entries(&entries, window);

            prop_assert_eq!(placements.len(), entries.len());
            for (entry, placement) in entries.iter().zip(&placements) {
                let logged_at = entry.timestamp;
                let nearest = fixes.iter().enumerate().min_by_key(|(_, fix)| {
                    (fix.placed.fix.tpv.time().utc() - logged_at).abs()
                });
                let Some((point_index, nearest)) = nearest else {
                    return Err(TestCaseError::fail("the generated track has no fixes"));
                };
                let gap = (nearest.placed.fix.tpv.time().utc() - logged_at).abs();

                prop_assert_eq!(placement.is_some(), gap <= window);
                if let Some(placement) = placement {
                    prop_assert_eq!(placement.fix.point, PointIdx::new(point_index));
                }
            }
        }
    }

    /// Positions this close are the same place to within a centimetre, which
    /// covers the great circle's departure from a straight line in degrees
    /// over the fixtures' steps.
    const POSITION_TOLERANCE_DEGREES: f64 = 1e-7;
}
