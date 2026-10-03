//! Ranking the loaded recordings a log could be associated against.

use std::cmp::Reverse;

use chrono::Duration;
use gt_loaded_files::{LoadedFileId, LoadedFilesView};
use gt_types::TimeRange;

/// The recordings a log could associate against, ranked by how much of the log
/// each of them covers, longest overlap first.
#[derive(Debug, Clone, PartialEq)]
pub struct AssociationCandidates(Vec<AssociationCandidate>);

impl AssociationCandidates {
    pub(crate) fn rank(log_range: TimeRange, recordings: &LoadedFilesView<'_>) -> Self {
        let mut candidates: Vec<AssociationCandidate> = recordings
            .entries()
            .map(|entry| {
                AssociationCandidate::of_track_coverage(
                    entry.id(),
                    entry
                        .file()
                        .tracks
                        .iter()
                        .map(|track| track.metadata.time_range),
                    log_range,
                )
            })
            .collect();
        candidates.sort_by(|left, right| {
            right
                .overlap
                .cmp(&left.overlap)
                .then(left.recording.cmp(&right.recording))
        });
        Self(candidates)
    }

    pub(crate) fn none() -> Self {
        Self(Vec::new())
    }

    pub fn rank_with_recording_order(mut self, recordings: &[LoadedFileId]) -> Self {
        self.0 = recordings
            .iter()
            .filter_map(|recording| {
                self.0
                    .iter()
                    .find(|candidate| candidate.recording == *recording)
                    .copied()
            })
            .collect();
        self.0.sort_by_key(|candidate| Reverse(candidate.overlap));
        self
    }

    /// Every loaded recording, best candidate first. A recording that misses
    /// the log entirely is ranked last but stays listed: a clock-skewed source
    /// is still a recording the user may pick.
    pub fn ranked(&self) -> &[AssociationCandidate] {
        &self.0
    }

    /// The one loaded recording overlapping the log, when exactly one does.
    ///
    /// With several overlapping recordings the user must choose. Users
    /// routinely have time-overlapping recordings from unrelated sources
    /// loaded, and anchoring a log to the wrong one would mislead whoever
    /// debugs with it.
    pub fn unambiguous_target(&self) -> Option<LoadedFileId> {
        let mut overlapping = self
            .0
            .iter()
            .filter(|candidate| candidate.overlaps_the_log());
        let only = overlapping.next()?;
        overlapping.next().is_none().then_some(only.recording)
    }
}

/// One recording a log could associate against, with how much of the log the
/// recording ran alongside.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AssociationCandidate {
    pub recording: LoadedFileId,

    /// How long the recording and the log ran at the same time.
    pub overlap: Duration,

    /// The share of the log's time span the overlap covers, 0.0 to 1.0.
    pub fraction_of_log: f64,
}

impl AssociationCandidate {
    fn of_track_coverage(
        recording: LoadedFileId,
        track_ranges: impl Iterator<Item = TimeRange>,
        log_range: TimeRange,
    ) -> Self {
        let mut shared_ranges: Vec<_> = track_ranges
            .filter_map(|range| log_range.intersection(range))
            .collect();
        shared_ranges.sort_unstable();
        let mut shared_ranges = shared_ranges.into_iter();
        let Some(mut covered_range) = shared_ranges.next() else {
            return Self {
                recording,
                overlap: Duration::zero(),
                fraction_of_log: 0.0,
            };
        };

        let mut overlap = Duration::zero();
        for range in shared_ranges {
            if range.start <= covered_range.end {
                covered_range = covered_range.union(range);
            } else {
                overlap += covered_range.duration();
                covered_range = range;
            }
        }
        overlap += covered_range.duration();

        let log_micros = log_range.duration().num_microseconds().unwrap_or(0);
        let fraction_of_log = if log_micros > 0 {
            overlap.num_microseconds().unwrap_or(0) as f64 / log_micros as f64
        } else {
            // A single-instant log has full coverage when a track contains that instant.
            1.0
        };
        Self {
            recording,
            overlap,
            fraction_of_log,
        }
    }

    pub fn overlaps_the_log(&self) -> bool {
        self.fraction_of_log > 0.0
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;
    use rstest::rstest;

    use crate::test_util;

    use super::*;

    #[test]
    fn recordings_rank_by_overlap_and_the_ones_missing_the_log_stay_listed() {
        let files = test_util::loaded(vec![
            test_util::recording_from(Duration::seconds(5), 10),
            test_util::recording_from(Duration::zero(), 10),
            test_util::recording_from(Duration::seconds(100), 5),
            test_util::recording_from(Duration::seconds(5), 10),
        ]);
        let log = test_util::log_of(10);

        let candidates = log.rank_association_candidates(&files.view());
        let ranked = candidates.ranked();

        assert_eq!(
            ranked
                .iter()
                .map(|candidate| candidate.recording)
                .collect::<Vec<_>>(),
            vec![
                test_util::id_of(&files, 1),
                test_util::id_of(&files, 0),
                test_util::id_of(&files, 3),
                test_util::id_of(&files, 2)
            ]
        );
        assert_eq!(
            ranked
                .iter()
                .map(|candidate| candidate.overlap)
                .collect::<Vec<_>>(),
            vec![
                Duration::seconds(LOG_SPAN_SECS),
                Duration::seconds(4),
                Duration::seconds(4),
                Duration::zero()
            ]
        );
        assert_eq!(
            ranked.first().map(|candidate| candidate.fraction_of_log),
            Some(1.0),
            "the recording running the whole log through covers all of it"
        );
        assert_eq!(
            ranked.get(1).map(|candidate| candidate.fraction_of_log),
            Some(4.0 / LOG_SPAN_SECS as f64)
        );
        assert_eq!(
            ranked.get(3).map(AssociationCandidate::overlaps_the_log),
            Some(false),
            "a recording missing the log is listed, but is no candidate"
        );
    }

    #[test]
    fn scoped_ranking_uses_overlap_then_the_supplied_recording_order() {
        let files = test_util::loaded(vec![
            test_util::recording_from(Duration::seconds(5), 10),
            test_util::recording_from(Duration::zero(), 10),
            test_util::recording_from(Duration::seconds(5), 10),
            test_util::recording_from(Duration::zero(), 10),
        ]);
        let scope = [
            test_util::id_of(&files, 2),
            test_util::id_of(&files, 1),
            test_util::id_of(&files, 0),
        ];
        let candidates = test_util::log_of(10)
            .rank_association_candidates(&files.view())
            .rank_with_recording_order(&scope);
        assert_eq!(
            candidates
                .ranked()
                .iter()
                .map(|candidate| candidate.recording)
                .collect::<Vec<_>>(),
            vec![
                test_util::id_of(&files, 1),
                test_util::id_of(&files, 2),
                test_util::id_of(&files, 0)
            ]
        );
    }

    #[test]
    fn a_recording_with_no_track_is_listed_but_is_no_candidate() {
        let files = test_util::loaded(vec![
            test_util::recording_with_no_track(),
            test_util::recording_from(Duration::zero(), 10),
        ]);
        let log = test_util::log_of(10);

        let candidates = log.rank_association_candidates(&files.view());

        assert_eq!(
            candidates
                .ranked()
                .iter()
                .find(|candidate| candidate.recording == test_util::id_of(&files, 0))
                .map(|candidate| candidate.overlaps_the_log()),
            Some(false)
        );
        assert_eq!(
            candidates.unambiguous_target(),
            Some(test_util::id_of(&files, 1))
        );
    }

    #[rstest]
    #[case::gap(&[(-10, 5), (20, 5)], 10, 0, 0.0)]
    #[case::disjoint_partial_coverage(&[(-2, 5), (7, 5)], 10, 4, 4.0 / 9.0)]
    #[case::overlapping_tracks(&[(-2, 8), (3, 10)], 10, 9, 1.0)]
    #[case::nested_tracks(&[(0, 10), (2, 3)], 10, 9, 1.0)]
    #[case::unsorted_overlapping_tracks(&[(6, 4), (0, 5), (3, 5)], 10, 9, 1.0)]
    #[case::touching_tracks(&[(-2, 6), (3, 7)], 10, 9, 1.0)]
    #[case::single_instant_tracks(&[(4, 1), (7, 1)], 10, 0, 0.0)]
    #[case::log_touches_track_endpoint(&[(9, 2)], 10, 0, 0.0)]
    #[case::single_instant_log_in_gap(&[(-3, 2), (3, 2)], 1, 0, 0.0)]
    #[case::single_instant_log_at_track_start(&[(0, 4)], 1, 0, 1.0)]
    #[case::single_instant_log_at_track_end(&[(-3, 4)], 1, 0, 1.0)]
    #[case::single_instant_log_inside_track(&[(-1, 3)], 1, 0, 1.0)]
    #[case::single_instant_log_and_track(&[(0, 1)], 1, 0, 1.0)]
    fn candidate_overlap_uses_the_union_of_track_coverage(
        #[case] tracks: &[(i64, usize)],
        #[case] log_entries: usize,
        #[case] overlap_seconds: i64,
        #[case] fraction_of_log: f64,
    ) {
        let tracks: Vec<_> = tracks
            .iter()
            .map(|(offset, count)| (Duration::seconds(*offset), *count))
            .collect();
        let files = test_util::loaded(vec![test_util::recording_with_track_offsets(&tracks)]);
        let log = test_util::log_of(log_entries);
        let recording = test_util::id_of(&files, 0);

        let candidates = log.rank_association_candidates(&files.view());

        assert_eq!(
            candidates.ranked(),
            &[AssociationCandidate {
                recording,
                overlap: Duration::seconds(overlap_seconds),
                fraction_of_log,
            }]
        );
        assert_eq!(
            candidates
                .ranked()
                .first()
                .map(AssociationCandidate::overlaps_the_log),
            Some(fraction_of_log > 0.0)
        );
        assert_eq!(
            candidates.unambiguous_target(),
            (fraction_of_log > 0.0).then_some(recording)
        );
    }

    proptest! {
        #[test]
        fn candidate_overlap_matches_covered_seconds(
            tracks in proptest::collection::vec((-10i64..20, 1usize..15), 0..8),
            log_entries in 1usize..20,
        ) {
            let log_seconds = log_entries as i64 - 1;
            let overlap_seconds = (0..log_seconds)
                .filter(|second| {
                    tracks.iter().any(|(offset, count)| {
                        *offset <= *second && offset + *count as i64 > second + 1
                    })
                })
                .count() as i64;
            let fraction_of_log = if log_seconds > 0 {
                overlap_seconds as f64 / log_seconds as f64
            } else if tracks.iter().any(|(offset, count)| {
                *offset <= 0 && offset + *count as i64 > 0
            }) {
                1.0
            } else {
                0.0
            };
            let tracks: Vec<_> = tracks
                .iter()
                .map(|(offset, count)| (Duration::seconds(*offset), *count))
                .collect();
            let files = test_util::loaded(vec![test_util::recording_with_track_offsets(&tracks)]);
            let candidates = test_util::log_of(log_entries)
                .rank_association_candidates(&files.view());

            prop_assert_eq!(
                candidates.ranked(),
                &[AssociationCandidate {
                    recording: test_util::id_of(&files, 0),
                    overlap: Duration::seconds(overlap_seconds),
                    fraction_of_log,
                }]
            );
        }
    }

    #[rstest]
    #[case::none_overlapping(&[100], None)]
    #[case::one_overlapping(&[0, 100], Some(0))]
    #[case::several_overlapping(&[0, 2], None)]
    fn a_target_is_preselected_only_when_exactly_one_recording_overlaps(
        #[case] recording_offsets_secs: &[i64],
        #[case] expected: Option<usize>,
    ) {
        let files = test_util::loaded(
            recording_offsets_secs
                .iter()
                .map(|offset| test_util::recording_from(Duration::seconds(*offset), 10))
                .collect(),
        );
        let log = test_util::log_of(10);

        assert_eq!(
            log.rank_association_candidates(&files.view())
                .unambiguous_target(),
            expected.map(|index| test_util::id_of(&files, index))
        );
    }

    /// The fixture log runs from its first to its tenth entry, one per second.
    const LOG_SPAN_SECS: i64 = 9;
}
