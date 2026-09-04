//! Reading one environment archive's day index again after a read failed
//! because another process had the file open.
//!
//! One failed read leaves a scheduler with no days for the whole run: it reads
//! the index once, when it adopts its store. That read can come back
//! [`DayArchiveError::is_held_by_another_process`] in a read-only session,
//! which reads the index beside the instance that owns the data directory. The
//! archives open their file for the duration of each operation, and libhdf5
//! takes an OS lock over that open, readers included.

use std::ops::Deref;
use std::time::{Duration, Instant};

use egui::Context;
use gt_store::{ArchiveHandle, DayArchiveError, EnvironmentArchive};

/// Interval before the first re-read, and the interval every further one
/// doubles from.
const FIRST_DAY_INDEX_REREAD_INTERVAL: Duration = Duration::from_millis(250);

/// Longest interval the doubling reaches, which bounds how long a session
/// goes without its days once the other instance lets go of the file.
const SLOWEST_DAY_INDEX_REREAD_INTERVAL: Duration = Duration::from_secs(10);

/// Re-reads that run at [`FIRST_DAY_INDEX_REREAD_INTERVAL`] before the
/// interval starts doubling, which is the first two seconds.
const REREADS_AT_THE_FIRST_INTERVAL: u32 = 8;

/// The re-read of one archive's day index that a failed read left due.
pub struct DayIndexReadRetry {
    archive: EnvironmentArchive,
    due: Option<DueReread>,
}

struct DueReread {
    last_read: Instant,
    failed_reads: u32,
}

impl DueReread {
    fn interval(&self) -> Duration {
        let doublings = self
            .failed_reads
            .saturating_sub(REREADS_AT_THE_FIRST_INTERVAL);
        FIRST_DAY_INDEX_REREAD_INTERVAL
            .saturating_mul(2u32.saturating_pow(doublings))
            .min(SLOWEST_DAY_INDEX_REREAD_INTERVAL)
    }
}

impl DayIndexReadRetry {
    pub fn for_archive(archive: EnvironmentArchive) -> Self {
        Self { archive, due: None }
    }

    /// Takes the outcome of one read of the day index, handing back the days
    /// it read.
    ///
    /// [`DayArchiveError::is_held_by_another_process`] leaves a re-read due,
    /// and requests the repaint that runs it: a session doing nothing else
    /// would never poll again. Every other failure is a problem in the file
    /// itself, and no re-read is due.
    pub fn record_read<Days, E: DayArchiveError>(
        &mut self,
        ctx: &Context,
        read: Result<Days, E>,
    ) -> Option<Days> {
        match read {
            Ok(days) => {
                self.due = None;
                Some(days)
            }
            Err(err) if err.is_held_by_another_process() => {
                let due = DueReread {
                    last_read: Instant::now(),
                    failed_reads: self.failed_reads().saturating_add(1),
                };
                let interval = due.interval();
                self.due = Some(due);
                log::info!(
                    "Another process has the {} archive open: reading its day index again in {:.1}s",
                    self.archive.label_in_sentence(),
                    interval.as_secs_f32()
                );
                ctx.request_repaint_after(interval);
                None
            }
            Err(err) => {
                self.due = None;
                log::error!(
                    "Reading the {} archive's day index: {err}",
                    self.archive.label_in_sentence()
                );
                None
            }
        }
    }

    /// Reads `store`'s day index with `read_index`, returning the days it read.
    ///
    /// [`None`] for `store` is a session with no archive open, and no re-read
    /// is due. A failed read returns [`None`] per [`Self::record_read`].
    pub fn read_the_day_index_of<W: Deref<Target = R>, R, Days, E: DayArchiveError>(
        &mut self,
        ctx: &Context,
        store: Option<&ArchiveHandle<W, R>>,
        read_index: impl FnOnce(&R) -> Result<Days, E>,
    ) -> Option<Days> {
        let Some(store) = store else {
            self.forget_the_due_reread();
            return None;
        };
        self.record_read(ctx, read_index(store.read()))
    }

    pub fn forget_the_due_reread(&mut self) {
        self.due = None;
    }

    pub fn is_due(&self, now: Instant) -> bool {
        self.due
            .as_ref()
            .is_some_and(|due| now.saturating_duration_since(due.last_read) >= due.interval())
    }

    fn failed_reads(&self) -> u32 {
        self.due.as_ref().map_or(0, |due| due.failed_reads)
    }
}

#[cfg(test)]
mod tests {
    use gt_store::SolarStoreError;

    use super::*;

    fn retry() -> DayIndexReadRetry {
        DayIndexReadRetry::for_archive(EnvironmentArchive::GeomagneticIndices)
    }

    fn held() -> Result<u8, SolarStoreError> {
        Err(SolarStoreError::HeldByAnotherProcess)
    }

    fn interval_after(failed_reads: u32) -> Duration {
        DueReread {
            last_read: Instant::now(),
            failed_reads,
        }
        .interval()
    }

    #[test]
    fn a_read_that_succeeded_leaves_no_reread_due() {
        let ctx = Context::default();
        let mut retry = retry();

        assert_eq!(
            retry.record_read(&ctx, Ok::<_, SolarStoreError>(7)),
            Some(7)
        );

        assert!(!retry.is_due(Instant::now() + SLOWEST_DAY_INDEX_REREAD_INTERVAL));
    }

    #[test]
    fn a_read_that_failed_on_another_process_comes_due_after_the_first_interval() {
        let ctx = Context::default();
        let mut retry = retry();

        assert_eq!(retry.record_read(&ctx, held()), None);

        let failed_at = Instant::now();
        assert!(!retry.is_due(failed_at));
        assert!(retry.is_due(failed_at + FIRST_DAY_INDEX_REREAD_INTERVAL));
    }

    #[test]
    fn a_read_the_archive_itself_failed_leaves_no_reread_due() {
        let ctx = Context::default();
        let mut retry = retry();

        let read = retry.record_read(
            &ctx,
            Err::<u8, _>(SolarStoreError::Corrupt("no day column".to_owned())),
        );

        assert_eq!(read, None);
        assert!(!retry.is_due(Instant::now() + SLOWEST_DAY_INDEX_REREAD_INTERVAL));
    }

    #[test]
    fn a_read_that_succeeded_puts_the_next_failed_one_back_on_the_first_interval() {
        let ctx = Context::default();
        let mut retry = retry();
        for _ in 0..REREADS_AT_THE_FIRST_INTERVAL + 4 {
            retry.record_read(&ctx, held());
        }
        retry.record_read(&ctx, Ok::<_, SolarStoreError>(7));

        retry.record_read(&ctx, held());

        assert!(retry.is_due(Instant::now() + FIRST_DAY_INDEX_REREAD_INTERVAL));
    }

    #[test]
    fn the_interval_doubles_past_the_first_rereads_and_stops_at_the_slowest() {
        let intervals: Vec<Duration> = (1..=REREADS_AT_THE_FIRST_INTERVAL + 12)
            .map(interval_after)
            .collect();

        assert!(
            intervals[..REREADS_AT_THE_FIRST_INTERVAL as usize]
                .iter()
                .all(|interval| *interval == FIRST_DAY_INDEX_REREAD_INTERVAL)
        );
        assert_eq!(
            intervals[REREADS_AT_THE_FIRST_INTERVAL as usize],
            FIRST_DAY_INDEX_REREAD_INTERVAL * 2
        );
        assert_eq!(
            intervals.last().copied(),
            Some(SLOWEST_DAY_INDEX_REREAD_INTERVAL)
        );
    }
}
