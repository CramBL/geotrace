//! The day queue every day-keyed fetch scheduler runs on: what is waiting,
//! what is in flight, what failed, and how far the archive covers the loaded
//! recordings.

use std::collections::{HashSet, VecDeque};
use std::fmt::Display;

use chrono::NaiveDate;

use super::backfill::{BackfillProgress, PendingBackfill};
use super::day_failures::DayFailure;
use super::day_fetch_status::{
    ArchivedDayCount, DayArchiveCoverage, DayArchiveState, DayFetchStatus,
};

/// The days one fetch worker has queued, is fetching, or could not archive,
/// and how far the archive covers the loaded recordings.
///
/// A day is requested at most once a session: every queued day stays in
/// `requested`, so a day that failed or came back revisable does not go out
/// again until the scheduler is pointed at another host.
#[derive(Default)]
pub struct DayFetchQueue {
    queue: VecDeque<NaiveDate>,
    requested: HashSet<NaiveDate>,
    in_flight: Option<NaiveDate>,
    failures: Vec<DayFailure>,
    recording_days: DayArchiveCoverage,
    /// Days fetched only to place the recording days against the archive
    /// around them, counted apart from the recording days themselves.
    background_days: DayArchiveCoverage,
    backfill: Option<PendingBackfill>,
}

impl DayFetchQueue {
    /// Queue `day` for a loaded recording, and record what the archive holds
    /// for it.
    pub fn request_recording_day<E: Display>(
        &mut self,
        day: NaiveDate,
        needs_fetch: Result<bool, E>,
    ) {
        let state = self.queue_counted_day(day, needs_fetch);
        self.background_days.forget(day);
        self.recording_days.record(day, state);
    }

    /// Queue `day` to place a recording day against the archive around it.
    ///
    /// A day a loaded recording spans keeps its recording-day count and is
    /// left alone here.
    pub fn request_background_day<E: Display>(
        &mut self,
        day: NaiveDate,
        needs_fetch: Result<bool, E>,
    ) {
        if self.recording_days.holds(day) {
            return;
        }
        let state = self.queue_counted_day(day, needs_fetch);
        self.background_days.record(day, state);
    }

    /// Queue `day` under the scheduler's refresh rule and report what the
    /// archive holds for it. An archive it could not be read from leaves the
    /// day awaited and reports the read as a failure.
    fn queue_counted_day<E: Display>(
        &mut self,
        day: NaiveDate,
        needs_fetch: Result<bool, E>,
    ) -> DayArchiveState {
        match needs_fetch {
            Ok(false) => DayArchiveState::Archived,
            Ok(true) => {
                self.queue_day(day);
                DayArchiveState::Awaited
            }
            Err(err) => {
                if self.requested.insert(day) {
                    self.report_unreadable_archive(day, &err);
                }
                DayArchiveState::Awaited
            }
        }
    }

    /// Queue every day of `days` the refresh rule wants, as one backfill.
    ///
    /// Days already requested this session are skipped, so re-running a
    /// backfill over the same range costs nothing. Replaces a backfill already
    /// running. Returns how many days were queued.
    pub fn start_backfill<E: Display>(
        &mut self,
        days: impl IntoIterator<Item = NaiveDate>,
        mut needs_fetch: impl FnMut(NaiveDate) -> Result<bool, E>,
    ) -> usize {
        self.cancel_backfill();
        let mut pending = HashSet::new();
        for day in days {
            if !self.requested.insert(day) {
                continue;
            }
            match needs_fetch(day) {
                Ok(true) => {
                    self.queue.push_back(day);
                    pending.insert(day);
                }
                Ok(false) => {}
                Err(err) => self.report_unreadable_archive(day, &err),
            }
        }
        let total = pending.len();
        if total > 0 {
            self.backfill = Some(PendingBackfill::new(pending));
        }
        total
    }

    /// Drop a running backfill's queued days.
    ///
    /// A later backfill over the same range queues the cancelled days again:
    /// they leave `requested`. The day in flight is not one of them, and stays
    /// until a response for it arrives: releasing it would let a second request
    /// go out for a day already being fetched.
    pub fn cancel_backfill(&mut self) {
        let Some(backfill) = self.backfill.take() else {
            return;
        };
        self.queue.retain(|day| !backfill.queued(*day));
        for day in backfill.into_pending_days() {
            if Some(day) != self.in_flight {
                self.requested.remove(&day);
            }
        }
    }

    /// Progress of the running backfill, or [`None`] when none is running.
    pub fn backfill_progress(&self) -> Option<BackfillProgress> {
        self.backfill.as_ref().map(PendingBackfill::progress)
    }

    /// What the settings page reports about the queue and the archive's
    /// coverage of the loaded recordings.
    pub fn fetch_status(&self) -> DayFetchStatus {
        DayFetchStatus {
            fetching: self.in_flight,
            queued: self.queue.len(),
            recording_days: self.recording_days.counts(),
        }
    }

    /// How far the archive covers the days fetched around the recording days.
    pub fn background_day_coverage(&self) -> ArchivedDayCount {
        self.background_days.counts()
    }

    /// Days that could not be archived, in the order they were reported.
    pub fn failures(&self) -> &[DayFailure] {
        &self.failures
    }

    pub fn report_failure(&mut self, day: NaiveDate, detail: String) {
        self.failures.push(DayFailure { day, detail });
    }

    /// Report `day` as holding everything the source publishes for it.
    pub fn mark_archived(&mut self, day: NaiveDate) {
        self.recording_days.mark_archived(day);
        self.background_days.mark_archived(day);
    }

    /// The next day to dispatch, or [`None`] while one is already in flight or
    /// nothing waits. The day it returns counts as in flight.
    pub fn take_next_day(&mut self) -> Option<NaiveDate> {
        if self.in_flight.is_some() {
            return None;
        }
        let day = self.queue.pop_front()?;
        self.in_flight = Some(day);
        Some(day)
    }

    /// Retire the day a worker reported on, ending the backfill with its last
    /// day.
    pub fn finish_day(&mut self, day: NaiveDate) {
        self.in_flight = None;
        let Some(backfill) = self.backfill.as_mut() else {
            return;
        };
        backfill.retire(day);
        if backfill.is_finished() {
            self.backfill = None;
        }
    }

    /// Drop what belongs to the host that was being fetched from: the queue,
    /// the days requested of it, its failures, and the backfill running over
    /// them. What the archive already holds is untouched.
    pub fn forget_host(&mut self) {
        self.queue.clear();
        self.requested.clear();
        self.failures.clear();
        self.backfill = None;
    }

    fn queue_day(&mut self, day: NaiveDate) {
        if self.requested.insert(day) {
            self.queue.push_back(day);
        }
    }

    fn report_unreadable_archive(&mut self, day: NaiveDate, err: &impl Display) {
        let detail = format!("reading the archive: {err}");
        log::error!("Cannot determine whether {day} is archived: {detail}");
        self.report_failure(day, detail);
    }

    #[cfg(test)]
    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    #[cfg(test)]
    pub fn is_fetching(&self) -> bool {
        self.in_flight.is_some()
    }

    /// The days requested this session, whatever they reported back.
    #[cfg(test)]
    pub fn requested_days(&self) -> &HashSet<NaiveDate> {
        &self.requested
    }

    /// Queue `days` as a running backfill without dispatching any of them, so
    /// a test does not depend on whether a transport can be built.
    #[cfg(test)]
    pub fn queue_backfill_of(&mut self, days: &[NaiveDate]) {
        for day in days {
            self.queue_day(*day);
        }
        self.backfill = Some(PendingBackfill::new(days.iter().copied().collect()));
    }

    /// Queue `day` the way a track load does.
    #[cfg(test)]
    pub fn queue_track_day(&mut self, day: NaiveDate) {
        self.queue_day(day);
    }

    /// Record `day` as a loaded recording's day the archive lacks.
    #[cfg(test)]
    pub fn await_recording_day(&mut self, day: NaiveDate) {
        self.recording_days.record(day, DayArchiveState::Awaited);
    }
}
