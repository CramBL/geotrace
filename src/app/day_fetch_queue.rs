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
use super::environment_storage::PrunedDays;

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
    /// day awaited and reports that the archive could not be read.
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

    /// The earliest day this queue counted for a recording loaded this
    /// session, including the days fetched around the recording days.
    ///
    /// An auto-prune keeps such a day until the session ends: it stays counted
    /// after its recording is closed.
    pub fn oldest_needed_day(&self) -> Option<NaiveDate> {
        self.recording_days
            .oldest_day()
            .into_iter()
            .chain(self.background_days.oldest_day())
            .min()
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

    /// Forget what the queue holds about the days a delete removed from the
    /// archive, so the scheduler requests the ones a loaded recording spans
    /// again.
    ///
    /// The day in flight keeps its place: releasing it would let a second
    /// request go out for a day already being fetched.
    pub fn forget_pruned_days(&mut self, pruned: PrunedDays) {
        let in_flight = self.in_flight;
        self.requested
            .retain(|day| !pruned.covers(*day) || Some(*day) == in_flight);
        self.failures.retain(|failure| !pruned.covers(failure.day));
        self.recording_days.mark_pruned_days_awaited(pruned);
        self.background_days.mark_pruned_days_awaited(pruned);
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

#[cfg(test)]
mod tests {
    use crate::app::test_util::day_archive;

    use super::*;

    const NEEDS_FETCH: Result<bool, &str> = Ok(true);
    const ARCHIVED: Result<bool, &str> = Ok(false);
    const UNREADABLE_ARCHIVE: Result<bool, &str> = Err("the archive is locked");

    #[test]
    fn a_recording_day_the_archive_lacks_is_queued_once() {
        let mut queue = DayFetchQueue::default();
        let day = day_archive::day(2026, 7, 20);

        queue.request_recording_day(day, NEEDS_FETCH);
        queue.request_recording_day(day, NEEDS_FETCH);

        assert_eq!(queue.queued(), 1);
        assert_eq!(queue.requested_days().len(), 1);
        assert_eq!(
            queue.fetch_status().recording_days,
            ArchivedDayCount {
                days: 1,
                archived: 0
            }
        );
    }

    /// A delete that removes an archived day lets the next load request it
    /// again: archived days stay out of `requested`.
    #[test]
    fn an_archived_recording_day_is_not_queued_and_counts_as_archived() {
        let mut queue = DayFetchQueue::default();

        queue.request_recording_day(day_archive::day(2026, 7, 20), ARCHIVED);

        assert_eq!(queue.queued(), 0);
        assert!(queue.requested_days().is_empty());
        assert_eq!(
            queue.fetch_status().recording_days,
            ArchivedDayCount {
                days: 1,
                archived: 1
            }
        );
    }

    #[test]
    fn an_unreadable_archive_awaits_the_recording_day_and_reports_one_failure() {
        let mut queue = DayFetchQueue::default();
        let day = day_archive::day(2026, 7, 20);

        queue.request_recording_day(day, UNREADABLE_ARCHIVE);
        queue.request_recording_day(day, UNREADABLE_ARCHIVE);

        assert_eq!(queue.queued(), 0);
        assert_eq!(
            queue.fetch_status().recording_days,
            ArchivedDayCount {
                days: 1,
                archived: 0
            }
        );
        assert_eq!(
            queue.failures(),
            [DayFailure {
                day,
                detail: "reading the archive: the archive is locked".to_owned(),
            }]
        );
    }

    #[test]
    fn a_background_day_a_recording_spans_stays_in_the_recording_count() {
        let mut queue = DayFetchQueue::default();
        let day = day_archive::day(2026, 7, 20);
        queue.request_recording_day(day, NEEDS_FETCH);

        queue.request_background_day(day, NEEDS_FETCH);

        assert_eq!(
            queue.fetch_status().recording_days,
            ArchivedDayCount {
                days: 1,
                archived: 0
            }
        );
        assert_eq!(queue.background_day_coverage(), ArchivedDayCount::default());
    }

    #[test]
    fn a_background_day_a_recording_later_spans_moves_to_the_recording_count() {
        let mut queue = DayFetchQueue::default();
        let day = day_archive::day(2026, 7, 20);
        queue.request_background_day(day, NEEDS_FETCH);

        queue.request_recording_day(day, NEEDS_FETCH);

        assert_eq!(
            queue.fetch_status().recording_days,
            ArchivedDayCount {
                days: 1,
                archived: 0
            }
        );
        assert_eq!(queue.background_day_coverage(), ArchivedDayCount::default());
        assert_eq!(queue.queued(), 1);
    }

    #[test]
    fn a_backfill_queues_the_days_the_archive_lacks_and_reports_their_total() {
        let mut queue = DayFetchQueue::default();
        let archived = day_archive::day(2026, 7, 21);

        let total = queue.start_backfill(
            (20..=22).map(|number| day_archive::day(2026, 7, number)),
            |day| {
                if day == archived {
                    ARCHIVED
                } else {
                    NEEDS_FETCH
                }
            },
        );

        assert_eq!(total, 2);
        assert_eq!(queue.queued(), 2);
        assert_eq!(
            queue.backfill_progress(),
            Some(BackfillProgress { done: 0, total: 2 })
        );
    }

    #[test]
    fn a_backfill_over_a_fully_archived_range_queues_nothing() {
        let mut queue = DayFetchQueue::default();

        let total = queue.start_backfill(
            (20..=22).map(|number| day_archive::day(2026, 7, number)),
            |_| ARCHIVED,
        );

        assert_eq!(total, 0);
        assert_eq!(queue.queued(), 0);
        assert_eq!(queue.backfill_progress(), None);
    }

    #[test]
    fn a_backfill_skips_a_day_a_recording_already_requested() {
        let mut queue = DayFetchQueue::default();
        let recording_day = day_archive::day(2026, 7, 20);
        queue.request_recording_day(recording_day, NEEDS_FETCH);

        let total = queue.start_backfill([recording_day, day_archive::day(2026, 7, 21)], |_| {
            NEEDS_FETCH
        });

        assert_eq!(total, 1);
        assert_eq!(queue.queued(), 2);
        assert_eq!(queue.take_next_day(), Some(recording_day));
    }

    #[test]
    fn an_unreadable_archive_in_a_backfill_queues_nothing_and_reports_a_failure() {
        let mut queue = DayFetchQueue::default();
        let day = day_archive::day(2026, 7, 20);

        let total = queue.start_backfill([day], |_| UNREADABLE_ARCHIVE);

        assert_eq!(total, 0);
        assert_eq!(queue.queued(), 0);
        assert_eq!(queue.backfill_progress(), None);
        assert_eq!(
            queue.failures(),
            [DayFailure {
                day,
                detail: "reading the archive: the archive is locked".to_owned(),
            }]
        );
    }

    #[test]
    fn starting_a_backfill_drops_the_days_the_running_one_queued() {
        let mut queue = DayFetchQueue::default();
        let replaced = day_archive::day(2026, 7, 20);
        let started = day_archive::day(2026, 7, 25);
        queue.start_backfill([replaced], |_| NEEDS_FETCH);

        let total = queue.start_backfill([started], |_| NEEDS_FETCH);

        assert_eq!(total, 1);
        assert_eq!(queue.queued(), 1);
        assert_eq!(queue.take_next_day(), Some(started));
    }

    #[test]
    fn cancelling_a_backfill_releases_the_days_it_queued() {
        let mut queue = DayFetchQueue::default();
        queue.start_backfill(
            [day_archive::day(2026, 7, 20), day_archive::day(2026, 7, 21)],
            |_| NEEDS_FETCH,
        );

        queue.cancel_backfill();

        assert_eq!(queue.queued(), 0);
        assert!(queue.requested_days().is_empty());
        assert_eq!(queue.backfill_progress(), None);
    }

    /// Releasing the day in flight would let a second request go out for a day
    /// already being fetched.
    #[test]
    fn cancelling_a_backfill_keeps_the_day_in_flight_requested() {
        let mut queue = DayFetchQueue::default();
        let in_flight = day_archive::day(2026, 7, 20);
        let queued = day_archive::day(2026, 7, 21);
        queue.start_backfill([in_flight, queued], |_| NEEDS_FETCH);
        assert_eq!(queue.take_next_day(), Some(in_flight));

        queue.cancel_backfill();

        assert!(queue.requested_days().contains(&in_flight));
        assert!(!queue.requested_days().contains(&queued));
    }

    #[test]
    fn cancelling_a_backfill_leaves_a_recording_day_queued() {
        let mut queue = DayFetchQueue::default();
        let recording_day = day_archive::day(2026, 7, 19);
        queue.request_recording_day(recording_day, NEEDS_FETCH);
        queue.start_backfill([day_archive::day(2026, 7, 20)], |_| NEEDS_FETCH);

        queue.cancel_backfill();

        assert_eq!(queue.queued(), 1);
        assert!(queue.requested_days().contains(&recording_day));
        assert_eq!(queue.take_next_day(), Some(recording_day));
    }

    #[test]
    fn take_next_day_dispatches_one_day_at_a_time_in_queue_order() {
        let mut queue = DayFetchQueue::default();
        let first = day_archive::day(2026, 7, 20);
        let second = day_archive::day(2026, 7, 21);
        queue.request_recording_day(first, NEEDS_FETCH);
        queue.request_recording_day(second, NEEDS_FETCH);

        assert_eq!(queue.take_next_day(), Some(first));
        assert_eq!(queue.take_next_day(), None);
        queue.finish_day(first);
        assert_eq!(queue.take_next_day(), Some(second));
        queue.finish_day(second);
        assert_eq!(queue.take_next_day(), None);
    }

    #[test]
    fn the_last_day_of_a_backfill_ends_it() {
        let mut queue = DayFetchQueue::default();
        let first = day_archive::day(2026, 7, 20);
        let last = day_archive::day(2026, 7, 21);
        queue.start_backfill([first, last], |_| NEEDS_FETCH);

        queue.finish_day(first);
        assert_eq!(
            queue.backfill_progress(),
            Some(BackfillProgress { done: 1, total: 2 })
        );

        queue.finish_day(last);
        assert_eq!(queue.backfill_progress(), None);
    }

    #[test]
    fn a_changed_host_drops_the_queue_its_failures_and_the_backfill() {
        let mut queue = DayFetchQueue::default();
        let day = day_archive::day(2026, 7, 20);
        queue.start_backfill([day], |_| NEEDS_FETCH);
        queue.report_failure(day, "HTTP 500 Internal Server Error".to_owned());

        queue.forget_host();

        assert_eq!(queue.queued(), 0);
        assert!(queue.requested_days().is_empty());
        assert!(queue.failures().is_empty());
        assert_eq!(queue.backfill_progress(), None);
    }

    #[test]
    fn a_changed_host_keeps_what_the_archive_holds_for_the_recording_days() {
        let mut queue = DayFetchQueue::default();
        queue.request_recording_day(day_archive::day(2026, 7, 20), ARCHIVED);
        queue.request_recording_day(day_archive::day(2026, 7, 21), NEEDS_FETCH);

        queue.forget_host();

        assert_eq!(
            queue.fetch_status().recording_days,
            ArchivedDayCount {
                days: 2,
                archived: 1
            }
        );
    }

    #[test]
    fn pruned_days_are_requestable_again_and_lose_their_failures() {
        let mut queue = DayFetchQueue::default();
        let pruned = day_archive::day(2026, 7, 20);
        let kept = day_archive::day(2026, 7, 25);
        queue.request_recording_day(pruned, NEEDS_FETCH);
        queue.request_recording_day(kept, NEEDS_FETCH);
        queue.mark_archived(pruned);
        queue.mark_archived(kept);
        queue.report_failure(pruned, "HTTP 500 Internal Server Error".to_owned());

        queue.forget_pruned_days(PrunedDays::Before(day_archive::day(2026, 7, 21)));

        assert!(!queue.requested_days().contains(&pruned));
        assert!(queue.requested_days().contains(&kept));
        assert!(queue.failures().is_empty());
        assert_eq!(
            queue.fetch_status().recording_days,
            ArchivedDayCount {
                days: 2,
                archived: 1
            }
        );
    }

    #[test]
    fn a_prune_keeps_the_day_in_flight_requested() {
        let mut queue = DayFetchQueue::default();
        let day = day_archive::day(2026, 7, 20);
        queue.request_recording_day(day, NEEDS_FETCH);
        assert_eq!(queue.take_next_day(), Some(day));

        queue.forget_pruned_days(PrunedDays::All);

        assert!(queue.requested_days().contains(&day));
    }

    #[test]
    fn the_oldest_needed_day_is_the_earliest_recording_or_background_day() {
        let mut queue = DayFetchQueue::default();
        queue.request_recording_day(day_archive::day(2026, 7, 20), NEEDS_FETCH);
        queue.request_background_day(day_archive::day(2026, 6, 23), NEEDS_FETCH);

        assert_eq!(
            queue.oldest_needed_day(),
            Some(day_archive::day(2026, 6, 23))
        );
    }
}
