//! The days an explicit history download is still waiting on, shared by the
//! schedulers that run one.

use std::collections::HashSet;

use chrono::NaiveDate;

/// A backfill's progress, for the panel's bar and count.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BackfillProgress {
    /// Days that have reported back, whatever they reported.
    pub done: usize,
    /// Days the backfill queued. Days already archived or already requested
    /// this session are not among them.
    pub total: usize,
}

impl BackfillProgress {
    pub fn fraction(self) -> f32 {
        if self.total == 0 {
            return 1.0;
        }
        #[expect(
            clippy::cast_precision_loss,
            reason = "a backfill spans at most a few thousand days"
        )]
        {
            self.done as f32 / self.total as f32
        }
    }
}

/// The queued days of a running backfill.
pub struct PendingBackfill {
    pending: HashSet<NaiveDate>,
    total: usize,
}

impl PendingBackfill {
    pub fn new(pending: HashSet<NaiveDate>) -> Self {
        Self {
            total: pending.len(),
            pending,
        }
    }

    pub fn progress(&self) -> BackfillProgress {
        BackfillProgress {
            done: self.total.saturating_sub(self.pending.len()),
            total: self.total,
        }
    }

    /// Whether the backfill queued `day`, which distinguishes it from a day
    /// a track load queued.
    pub fn queued(&self, day: NaiveDate) -> bool {
        self.pending.contains(&day)
    }

    /// Drop `day` from what the backfill waits on.
    pub fn retire(&mut self, day: NaiveDate) {
        self.pending.remove(&day);
    }

    pub fn is_finished(&self) -> bool {
        self.pending.is_empty()
    }

    /// The days that never went out, for a cancelling scheduler to release.
    pub fn into_pending_days(self) -> impl Iterator<Item = NaiveDate> {
        self.pending.into_iter()
    }
}
