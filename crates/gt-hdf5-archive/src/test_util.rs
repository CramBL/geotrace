//! Day builders, row values and assertions over an archive operation, for the
//! tests of this crate and of the archives built on it.

use chrono::{NaiveDate, TimeDelta};

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

pub const fn day_at(offset: u32) -> NaiveDate {
    match NaiveDate::from_ymd_opt(2026, 8, 10 + offset) {
        Some(day) => day,
        None => NaiveDate::MIN,
    }
}

pub fn day(offset: i64) -> NaiveDate {
    day_at(0) + TimeDelta::days(offset)
}

/// Values whose file size follows the rows they fill: they do not compress.
pub fn values_of(day: NaiveDate, rows: usize) -> Vec<u64> {
    let seed = u64::try_from(day.to_epoch_days()).unwrap_or_default();
    (0..rows)
        .map(|row| (seed * 1_000 + row as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15))
        .collect()
}
