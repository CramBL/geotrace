use super::*;

#[test]
fn cancellation_stops_the_run_without_partial_results() {
    let provider = TestProvider::new(5).with(
        QueryMetric::Velocity,
        vec![Some(9.0), Some(9.0), Some(9.0), Some(9.0), Some(9.0)],
    );
    let query = test_util::checked("points | where velocity > 0 km/h");
    let inputs = [TrackInput {
        track: test_util::track_ref(),
        provider: &provider,
    }];

    let cancelled = || true;
    assert_eq!(run_cancellable(&query, &inputs, &cancelled), None);

    let live = || false;
    let output = run_cancellable(&query, &inputs, &live).expect("not cancelled");
    assert_eq!(output.summary.match_count, 1);
}

/// The interval-gated check inside a track's scan loop must also stop
/// the run - not only the per-track check at its entry.
#[test]
fn cancellation_fires_mid_scan() {
    let provider = TestProvider::new(6).with(QueryMetric::Velocity, vec![Some(9.0); 6]);
    let query = test_util::checked("points | where velocity > 0 km/h");
    let inputs = [TrackInput {
        track: test_util::track_ref(),
        provider: &provider,
    }];

    // First call is the per-track check. Cancel on a later one so the
    // stop happens inside the point loop.
    let calls = std::cell::Cell::new(0_u32);
    let cancel_after_two = || {
        calls.set(calls.get() + 1);
        calls.get() > 2
    };
    assert_eq!(
        crate::eval::run_with_interval(&query, &inputs, &cancel_after_two, 1),
        None,
        "mid-scan cancellation must not surface partial matches"
    );
    assert!(calls.get() > 2, "the loop reached the mid-scan checks");
}
