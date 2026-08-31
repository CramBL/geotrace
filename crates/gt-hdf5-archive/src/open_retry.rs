//! Opening an archive again after libhdf5 rejected the open because another
//! process had the file open.

use std::thread;
use std::time::{Duration, Instant};

use crate::ArchiveError;

/// Interval between two attempts at an open that failed with
/// [`ArchiveError::HeldByAnotherProcess`].
const RETRY_INTERVAL: Duration = Duration::from_millis(8);

/// How long one open waits in total before it returns
/// [`ArchiveError::HeldByAnotherProcess`]. The wait blocks the calling thread,
/// which is the GUI thread for the archive lookups a UI action makes.
const TOTAL_RETRY_BUDGET: Duration = Duration::from_millis(40);

/// How often one [`OpenRetry`] spends the whole budget. A backfill opens the
/// archive once per day of its range: at one budget per open, a year-long
/// range over an archive another process holds blocks the calling thread for
/// 14 seconds.
const SHORTEST_INTERVAL_BETWEEN_RETRY_BUDGETS: Duration = Duration::from_secs(1);

/// The retrying of one archive's read-only opens, over the lifetime of the
/// [`crate::ArchiveFile`] that holds it.
#[derive(Debug, Default)]
pub struct OpenRetry {
    last_lock_conflict_at: Option<Instant>,
}

impl OpenRetry {
    /// Calls `open` again on [`ArchiveError::HeldByAnotherProcess`], sleeping
    /// [`RETRY_INTERVAL`] on the calling thread between attempts.
    pub fn open<T>(
        &mut self,
        open: impl FnMut() -> Result<T, ArchiveError>,
    ) -> Result<T, ArchiveError> {
        self.open_with_clock_and_wait(Instant::now(), thread::sleep, open)
    }

    /// The tests pass a `wait` that records the interval and a `now` they
    /// step themselves, which runs the schedule without sleeping.
    fn open_with_clock_and_wait<T>(
        &mut self,
        now: Instant,
        mut wait: impl FnMut(Duration),
        mut open: impl FnMut() -> Result<T, ArchiveError>,
    ) -> Result<T, ArchiveError> {
        let mut budget = self.budget_at(now);
        loop {
            match open() {
                Ok(opened) => {
                    self.last_lock_conflict_at = None;
                    return Ok(opened);
                }
                Err(ArchiveError::HeldByAnotherProcess) if budget >= RETRY_INTERVAL => {
                    wait(RETRY_INTERVAL);
                    budget = budget.saturating_sub(RETRY_INTERVAL);
                }
                Err(ArchiveError::HeldByAnotherProcess) => {
                    self.last_lock_conflict_at = Some(now);
                    return Err(ArchiveError::HeldByAnotherProcess);
                }
                Err(err) => return Err(err),
            }
        }
    }

    fn budget_at(&self, now: Instant) -> Duration {
        match self.last_lock_conflict_at {
            Some(conflict)
                if now.saturating_duration_since(conflict)
                    < SHORTEST_INTERVAL_BETWEEN_RETRY_BUDGETS =>
            {
                Duration::ZERO
            }
            _ => TOTAL_RETRY_BUDGET,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// One open of an archive another process holds throughout, run without
    /// sleeping.
    struct HeldThroughout {
        outcome: Result<(), ArchiveError>,
        attempts: u32,
        waited: Duration,
    }

    fn open_an_archive_held_throughout(retry: &mut OpenRetry, now: Instant) -> HeldThroughout {
        let mut attempts: u32 = 0;
        let mut waited = Duration::ZERO;
        let outcome = retry.open_with_clock_and_wait(
            now,
            |interval| waited = waited.saturating_add(interval),
            || {
                attempts = attempts.saturating_add(1);
                Err(ArchiveError::HeldByAnotherProcess)
            },
        );
        HeldThroughout {
            outcome,
            attempts,
            waited,
        }
    }

    #[test]
    fn a_lock_conflict_that_clears_within_the_budget_opens() {
        let mut retry = OpenRetry::default();
        let mut attempts = 0;
        let mut waits = Vec::new();

        let opened = retry.open_with_clock_and_wait(
            Instant::now(),
            |interval| waits.push(interval),
            || {
                attempts += 1;
                if attempts < 3 {
                    Err(ArchiveError::HeldByAnotherProcess)
                } else {
                    Ok(attempts)
                }
            },
        );

        assert_eq!(opened.ok(), Some(3));
        assert_eq!(waits, [RETRY_INTERVAL; 2]);
    }

    #[test]
    fn a_lock_conflict_that_never_clears_fails_after_the_whole_budget() {
        let held = open_an_archive_held_throughout(&mut OpenRetry::default(), Instant::now());

        assert!(
            matches!(held.outcome, Err(ArchiveError::HeldByAnotherProcess)),
            "{:?}",
            held.outcome
        );
        assert_eq!(held.attempts, 6);
        assert_eq!(held.waited, TOTAL_RETRY_BUDGET);
    }

    #[test]
    fn an_error_that_is_not_a_lock_conflict_is_returned_without_a_wait() {
        let mut retry = OpenRetry::default();
        let mut attempts = 0;
        let mut waits = 0;

        let opened = retry.open_with_clock_and_wait(
            Instant::now(),
            |_| waits += 1,
            || {
                attempts += 1;
                Err::<(), _>(ArchiveError::SchemaTooNew {
                    found: 2,
                    supported: 1,
                })
            },
        );

        assert!(
            matches!(opened, Err(ArchiveError::SchemaTooNew { .. })),
            "{opened:?}"
        );
        assert_eq!(attempts, 1);
        assert_eq!(waits, 0);
    }

    #[test]
    fn a_second_open_within_the_shortest_interval_makes_one_attempt() {
        let mut retry = OpenRetry::default();
        let first = Instant::now();
        open_an_archive_held_throughout(&mut retry, first);

        let second = open_an_archive_held_throughout(
            &mut retry,
            first + SHORTEST_INTERVAL_BETWEEN_RETRY_BUDGETS / 2,
        );

        assert_eq!(second.attempts, 1);
        assert_eq!(second.waited, Duration::ZERO);
    }

    #[test]
    fn an_open_past_the_shortest_interval_spends_the_budget_again() {
        let mut retry = OpenRetry::default();
        let first = Instant::now();
        open_an_archive_held_throughout(&mut retry, first);

        let later = open_an_archive_held_throughout(
            &mut retry,
            first + SHORTEST_INTERVAL_BETWEEN_RETRY_BUDGETS,
        );

        assert_eq!(later.waited, TOTAL_RETRY_BUDGET);
    }

    #[test]
    fn an_open_that_succeeded_puts_the_whole_budget_back() {
        let mut retry = OpenRetry::default();
        let now = Instant::now();
        open_an_archive_held_throughout(&mut retry, now);
        retry
            .open_with_clock_and_wait(now, |_| {}, || Ok::<_, ArchiveError>(()))
            .ok();

        let held = open_an_archive_held_throughout(&mut retry, now);

        assert_eq!(held.waited, TOTAL_RETRY_BUDGET);
    }
}
