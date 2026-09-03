//! The conditions every environment scheduler checks before it hands a day to
//! a worker thread, and the thread it hands it to.

use std::sync::Arc;
use std::sync::mpsc;

use chrono::NaiveDate;
use egui::Context;
use gt_fetch::Connection;
use gt_pending_writes::PendingWrites;
use gt_store::WritableArchive;

use super::background_thread;
use super::day_fetch_queue::DayFetchQueue;
use super::day_fetch_transport::DayFetchTransport;

/// One day taken off a [`DayFetchQueue`], with the archive to insert it into
/// and the connection to request it on.
pub struct DayFetch<W> {
    pub archive: WritableArchive<W>,
    pub day: NaiveDate,
    pub transport: Arc<Connection>,
}

impl<W> DayFetch<W> {
    /// The next day to fetch, or [`None`] when `archive` is [`None`],
    /// [`PendingWrites::rejection`] holds a rejection, or `days` has no day
    /// ready.
    ///
    /// A scheduler with further conditions of its own checks those before
    /// calling this: a returned day stays in flight in `days` until
    /// [`DayFetchQueue::finish_day`].
    pub fn take_next(
        archive: Option<WritableArchive<W>>,
        pending_writes: &PendingWrites,
        days: &mut DayFetchQueue,
        transport: &mut DayFetchTransport,
    ) -> Option<Self> {
        let archive = archive?;
        if pending_writes.rejection().is_some() {
            return None;
        }
        let day = days.take_next_day()?;
        Some(Self {
            archive,
            day,
            transport: transport.connect_or_offline(),
        })
    }
}

/// Run `fetch` on a thread named `{thread_name_prefix}-{day}`, send what it
/// returns on `tx`, and repaint.
pub fn spawn_fetch_thread<M: Send + 'static>(
    ctx: &Context,
    tx: &mpsc::Sender<M>,
    thread_name_prefix: &str,
    day: NaiveDate,
    fetch: impl FnOnce() -> M + Send + 'static,
) {
    let ctx = ctx.clone();
    let tx = tx.clone();
    background_thread::spawn_or_panic(format!("{thread_name_prefix}-{day}"), move || {
        let message = fetch();
        tx.send(message).ok();
        ctx.request_repaint();
    });
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use gt_fetch::TransportSource;
    use gt_pending_writes::WriteAccess;
    use gt_store::EnvironmentArchive;

    use super::*;

    /// A test double for one of the day archives, which nothing here calls into.
    struct TestArchive;

    fn day() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 7, 20).expect("a valid date")
    }

    fn queue_with_one_day() -> DayFetchQueue {
        let mut days = DayFetchQueue::default();
        days.queue_track_day(day());
        days
    }

    fn transport() -> DayFetchTransport {
        DayFetchTransport::paced(
            TransportSource::Offline,
            Duration::from_secs(1),
            EnvironmentArchive::AircraftInterference,
        )
    }

    fn writable(pending_writes: &PendingWrites) -> Option<WritableArchive<TestArchive>> {
        Some(WritableArchive::new(
            Arc::new(TestArchive),
            pending_writes.clone(),
        ))
    }

    #[test]
    fn a_queued_day_is_taken_with_the_archive_and_a_connection() {
        let pending_writes = PendingWrites::new(WriteAccess::Owner);
        let mut days = queue_with_one_day();

        let fetch = DayFetch::take_next(
            writable(&pending_writes),
            &pending_writes,
            &mut days,
            &mut transport(),
        )
        .expect("every condition holds");

        assert_eq!(fetch.day, day());
        assert!(matches!(fetch.transport.as_ref(), Connection::Offline(_)));
        assert!(days.is_fetching());
    }

    #[test]
    fn a_read_only_session_leaves_its_queued_day_alone() {
        let pending_writes = PendingWrites::new(WriteAccess::ReadOnly);
        let mut days = queue_with_one_day();

        let fetch =
            DayFetch::<TestArchive>::take_next(None, &pending_writes, &mut days, &mut transport());

        assert!(fetch.is_none());
        assert_eq!(days.queued(), 1);
        assert!(!days.is_fetching());
    }

    #[test]
    fn shutdown_leaves_the_queued_day_alone() {
        let pending_writes = PendingWrites::new(WriteAccess::Owner);
        pending_writes.begin_shutdown();
        let mut days = queue_with_one_day();

        let fetch = DayFetch::take_next(
            writable(&pending_writes),
            &pending_writes,
            &mut days,
            &mut transport(),
        );

        assert!(fetch.is_none());
        assert_eq!(days.queued(), 1);
    }

    #[test]
    fn an_empty_queue_yields_no_fetch() {
        let pending_writes = PendingWrites::new(WriteAccess::Owner);

        let fetch = DayFetch::take_next(
            writable(&pending_writes),
            &pending_writes,
            &mut DayFetchQueue::default(),
            &mut transport(),
        );

        assert!(fetch.is_none());
    }

    #[test]
    fn the_thread_reports_what_the_fetch_returned() {
        let (tx, rx) = mpsc::channel();

        spawn_fetch_thread(&Context::default(), &tx, "test", day(), || "fetched");

        assert_eq!(
            rx.recv_timeout(Duration::from_secs(5)),
            Ok("fetched"),
            "the worker sends its message on the channel"
        );
    }
}
