//! Assertions over an archive operation, for the tests of this crate and of
//! the archives built on it.

use crate::prune::PruneProgress;

/// Fails unless the delete `reported` before it rewrote a column, never went
/// backwards or past what it counted, and ended on the last column with a
/// full bar.
#[track_caller]
pub fn assert_progress_ran_to_completion(reported: &[PruneProgress]) {
    assert!(
        matches!(reported, [first, ..] if first.columns_rewritten == 0),
        "a delete reports before it rewrites a column: {reported:?}"
    );
    assert!(
        reported.windows(2).all(|pair| matches!(
            pair,
            [before, after] if after.columns_rewritten >= before.columns_rewritten
        )),
        "progress went backwards: {reported:?}"
    );
    assert!(
        reported
            .iter()
            .all(|progress| progress.columns_rewritten <= progress.columns_total),
        "progress passed the columns it counts: {reported:?}"
    );
    assert!(
        matches!(reported, [.., last] if last.columns_rewritten == last.columns_total),
        "the delete ended short of the columns it counted: {reported:?}"
    );
    assert!(
        matches!(reported, [.., last] if (last.fraction() - 1.0).abs() < f32::EPSILON),
        "the delete ended on a bar short of full: {reported:?}"
    );
}
