//! Ranking the loaded recordings a log could be associated against.

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
                AssociationCandidate::of_shared_span(
                    entry.id(),
                    entry
                        .file()
                        .metadata
                        .time_range
                        .and_then(|recorded| log_range.intersection(recorded)),
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
    /// `shared` is the span the recording and the log ran through together,
    /// `None` when the recording missed the log entirely.
    fn of_shared_span(
        recording: LoadedFileId,
        shared: Option<TimeRange>,
        log_range: TimeRange,
    ) -> Self {
        let overlap = shared.map_or_else(Duration::zero, |shared| shared.duration());
        let log_micros = log_range.duration().num_microseconds().unwrap_or(0);
        let fraction_of_log = if log_micros > 0 {
            overlap.num_microseconds().unwrap_or(0) as f64 / log_micros as f64
        } else if shared.is_some() {
            // Every entry of the log shares one instant, which the recording
            // either covers whole or misses.
            1.0
        } else {
            0.0
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
    use rstest::rstest;

    use crate::test_fixtures::{id_of, loaded, log_of, recording_from, recording_with_no_track};

    use super::*;

    /// The fixture log runs from its first to its tenth entry, one per second.
    const LOG_SPAN_SECS: i64 = 9;

    #[test]
    fn recordings_rank_by_overlap_and_the_ones_missing_the_log_stay_listed() {
        let files = loaded(vec![
            recording_from(Duration::seconds(5), 10),
            recording_from(Duration::zero(), 10),
            recording_from(Duration::seconds(100), 5),
        ]);
        let log = log_of(10);

        let candidates = log.rank_association_candidates(&files.view());
        let ranked = candidates.ranked();

        assert_eq!(
            ranked
                .iter()
                .map(|candidate| candidate.recording)
                .collect::<Vec<_>>(),
            vec![id_of(&files, 1), id_of(&files, 0), id_of(&files, 2)]
        );
        assert_eq!(
            ranked
                .iter()
                .map(|candidate| candidate.overlap)
                .collect::<Vec<_>>(),
            vec![
                Duration::seconds(LOG_SPAN_SECS),
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
            ranked.get(2).map(AssociationCandidate::overlaps_the_log),
            Some(false),
            "a recording missing the log is listed, but is no candidate"
        );
    }

    #[test]
    fn a_recording_with_no_track_is_listed_but_is_no_candidate() {
        let files = loaded(vec![
            recording_with_no_track(),
            recording_from(Duration::zero(), 10),
        ]);
        let log = log_of(10);

        let candidates = log.rank_association_candidates(&files.view());

        assert_eq!(
            candidates
                .ranked()
                .iter()
                .find(|candidate| candidate.recording == id_of(&files, 0))
                .map(|candidate| candidate.overlaps_the_log()),
            Some(false)
        );
        assert_eq!(candidates.unambiguous_target(), Some(id_of(&files, 1)));
    }

    #[rstest]
    #[case::none_overlapping(&[100], None)]
    #[case::one_overlapping(&[0, 100], Some(0))]
    #[case::several_overlapping(&[0, 2], None)]
    fn a_target_is_preselected_only_when_exactly_one_recording_overlaps(
        #[case] recording_offsets_secs: &[i64],
        #[case] expected: Option<usize>,
    ) {
        let files = loaded(
            recording_offsets_secs
                .iter()
                .map(|offset| recording_from(Duration::seconds(*offset), 10))
                .collect(),
        );
        let log = log_of(10);

        assert_eq!(
            log.rank_association_candidates(&files.view())
                .unambiguous_target(),
            expected.map(|index| id_of(&files, index))
        );
    }
}
