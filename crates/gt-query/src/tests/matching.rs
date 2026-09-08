use super::*;

/// A track that carries no value at all for a referenced metric is
/// counted per metric in `tracks_without` - whole-track absences (no
/// snap run, an eph-less receiver) get named, not just point skips. A
/// track with even one value stays out. `accel` derives from velocity,
/// so its absence is probed through velocity.
#[test]
fn summary_counts_tracks_without_a_referenced_metric() {
    let with_values =
        TestProvider::new(3).with(QueryMetric::SnapError, vec![Some(2.0), None, Some(4.0)]);
    let without = TestProvider::new(3);
    let output = run(
        &test_util::checked("points | where snap_error > 1 m"),
        &[
            TrackInput {
                track: test_util::track_ref(),
                provider: &with_values,
            },
            TrackInput {
                track: TrackRef::new(FileIdx::new(0), TrackIdx::new(1)),
                provider: &without,
            },
        ],
    );
    assert_eq!(
        output.summary.tracks_without,
        BTreeMap::from([(QueryMetric::SnapError, 1)]),
        "only the run-less track counts"
    );
    // The valued track's single missing point stays a point skip. The
    // absent track contributes its full length.
    assert_eq!(
        output.summary.skipped,
        BTreeMap::from([(QueryMetric::SnapError, 4)])
    );

    // A run where no point produced a value (every point unsnapped)
    // counts too: the summary reports missing values, and its wording
    // deliberately does not claim the track was never snapped.
    let all_unsnapped = TestProvider::new(3).with(QueryMetric::SnapError, vec![None, None, None]);
    let output = run(
        &test_util::checked("points | where snap_error > 1 m"),
        &[TrackInput {
            track: test_util::track_ref(),
            provider: &all_unsnapped,
        }],
    );
    assert_eq!(
        output.summary.tracks_without,
        BTreeMap::from([(QueryMetric::SnapError, 1)])
    );

    let velocity_only =
        TestProvider::new(2).with(QueryMetric::Velocity, vec![Some(5.0), Some(6.0)]);
    let output = run(
        &test_util::checked("points | where accel > 0 m/s2"),
        &[TrackInput {
            track: test_util::track_ref(),
            provider: &velocity_only,
        }],
    );
    assert!(
        output.summary.tracks_without.is_empty(),
        "accel derives from velocity - a velocity-carrying track is not without it"
    );
}

#[test]
fn point_predicate_matches_consecutive_runs() {
    // 30 km/h is 8.33 m/s. Points 1, 2, and 4 exceed it.
    let provider = TestProvider::new(5).with(
        QueryMetric::Velocity,
        vec![Some(5.0), Some(10.0), Some(9.0), Some(3.0), Some(12.0)],
    );
    let output = test_util::run_one("points | where velocity > 30 km/h", &provider);
    assert_eq!(output.matches.len(), 1);
    assert_eq!(output.matches[0].ranges, vec![1..3, 4..5]);
    assert_eq!(output.summary.match_count, 2);
    assert_eq!(output.summary.tracks_with_matches, 1);
    // 3 of the 5 points matched - the counts the keep/hide summary uses.
    assert_eq!(output.summary.matched_points, 3);
    assert_eq!(output.summary.total_points, 5);
    assert!(output.summary.skipped.is_empty());
}

#[test]
fn missing_values_poison_and_are_counted() {
    let provider = TestProvider::new(5).with(
        QueryMetric::Heading,
        vec![Some(10.0), Some(12.0), None, Some(11.0), Some(13.0)],
    );
    let output = test_util::run_one(
        "points | window 2 | where spread(heading) <= 10 deg",
        &provider,
    );
    // Windows [1,2] and [2,3] touch the hole and are skipped.
    assert_eq!(output.summary.skipped.get(&QueryMetric::Heading), Some(&2));
    assert_eq!(output.matches[0].ranges, vec![0..2, 3..5]);
}

#[test]
fn accel_derives_from_velocity_and_time() {
    let provider = TestProvider::new(4)
        .with(
            QueryMetric::Velocity,
            vec![Some(0.0), Some(1.0), Some(2.0), Some(3.0)],
        )
        .indexed_time();
    let output = test_util::run_one("points | where accel >= 0.5 m/s2", &provider);
    // Point 0 has no accel (no predecessor) and counts as skipped.
    assert_eq!(output.matches[0].ranges, vec![1..4]);
    assert_eq!(output.summary.skipped.get(&QueryMetric::Accel), Some(&1));
}
