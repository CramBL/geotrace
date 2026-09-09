//! The per-node TEC series captured across the May 2024 storm, as a fixture
//! small enough to commit.
//!
//! A quiet-time median needs the 27 days before the day assessed, and a whole
//! month of published files is 30 MB. This capture keeps, for the few grid
//! nodes [`FIXTURE_NODES`](crate::FIXTURE_NODES) declares, every published
//! epoch's value over [`NODE_SERIES_DAYS`](crate::NODE_SERIES_DAYS), which is
//! what the storm index reads and what the reference illustration draws.
//!
//! Written by `just ionex-node-series` and frozen once committed, like the
//! whole-file captures beside it.

use std::collections::BTreeMap;

use chrono::{DateTime, NaiveDate, NaiveTime, TimeDelta, Utc};
use serde::{Deserialize, Serialize};

use crate::quiet_time;
use crate::tec::TotalElectronContent;

/// One archived day of the capture: what the archive served, and each node's
/// value at every epoch of that day's file.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CapturedNodeDay {
    pub day: NaiveDate,
    pub file_name: String,
    pub url: String,
    pub http_status: u16,
    /// Time between the day's epochs, as its header declares it.
    pub interval_seconds: i64,
    /// The highest value anywhere in the day's maps, which places the node
    /// values against the whole grid.
    pub peak_tecu: Option<f64>,
    /// One entry per node of [`FIXTURE_NODES`](crate::FIXTURE_NODES), keyed by
    /// its name, holding that node's value at each epoch of the day's file in
    /// epoch order. A published file's last epoch is the next day's midnight,
    /// so the last entry of a day repeats the first entry of the day after it.
    pub values_tecu: BTreeMap<String, Vec<Option<f64>>>,
}

impl CapturedNodeDay {
    fn interval(&self) -> TimeDelta {
        TimeDelta::seconds(self.interval_seconds)
    }

    /// The value of `node` at `offset` into the day, [`None`] where the file
    /// has no epoch there or the producer published no value for the node.
    ///
    /// Only offsets the day's own epochs name are read: the capture holds
    /// published values, not interpolations between them.
    fn value_at_offset(&self, node: &str, offset: TimeDelta) -> Option<TotalElectronContent> {
        let interval = self.interval();
        if interval.is_zero() || offset.num_milliseconds() % interval.num_milliseconds() != 0 {
            return None;
        }
        let index =
            usize::try_from(offset.num_milliseconds() / interval.num_milliseconds()).ok()?;
        self.values_tecu
            .get(node)?
            .get(index)
            .copied()
            .flatten()
            .map(TotalElectronContent::from_tecu)
    }

    /// Every offset into the day its file holds an epoch at, oldest first.
    /// Each node carries one value per map, so the count is the day's own.
    pub fn epoch_offsets(&self) -> Vec<TimeDelta> {
        let epochs = self.values_tecu.values().map(Vec::len).max().unwrap_or(0);
        (0..epochs)
            .filter_map(|index| self.interval().checked_mul(i32::try_from(index).ok()?))
            .collect()
    }

    /// Every epoch of the day and the value `node` carries there, oldest
    /// first.
    fn samples(&self, node: &str) -> Vec<NodeSample> {
        let midnight = self.day.and_time(NaiveTime::MIN).and_utc();
        self.values_tecu
            .get(node)
            .into_iter()
            .flatten()
            .enumerate()
            .filter_map(|(index, tecu)| {
                let offset = self.interval().checked_mul(i32::try_from(index).ok()?)?;
                Some(NodeSample {
                    epoch: midnight.checked_add_signed(offset)?,
                    tecu: *tecu,
                })
            })
            .collect()
    }
}

/// One node's value at one published epoch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NodeSample {
    pub epoch: DateTime<Utc>,
    pub tecu: Option<f64>,
}

/// The whole capture, oldest day first.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeSeriesCapture {
    pub captured_at: String,
    pub base_url: String,
    pub days: Vec<CapturedNodeDay>,
}

impl NodeSeriesCapture {
    pub fn day(&self, day: NaiveDate) -> Option<&CapturedNodeDay> {
        self.days.iter().find(|captured| captured.day == day)
    }

    /// The value of `node` at `offset` into `day`, [`None`] for a day missing
    /// from the capture.
    pub fn value_at_offset(
        &self,
        node: &str,
        day: NaiveDate,
        offset: TimeDelta,
    ) -> Option<TotalElectronContent> {
        self.day(day)?.value_at_offset(node, offset)
    }

    /// The quiet-time window one deviation is taken against: the same node and
    /// offset on each of the 27 days before `day` the capture holds.
    pub fn background_window(
        &self,
        node: &str,
        day: NaiveDate,
        offset: TimeDelta,
    ) -> Vec<TotalElectronContent> {
        quiet_time::background_days(day)
            .into_iter()
            .filter_map(|background| self.value_at_offset(node, background, offset))
            .collect()
    }

    /// Every published sample of `node` over the whole capture, oldest first.
    ///
    /// A day's closing epoch is dropped where the day after it is captured:
    /// the two name the same instant, and a series with one point per instant
    /// is what a plot line and a median are read from.
    pub fn samples(&self, node: &str) -> Vec<NodeSample> {
        let mut samples = Vec::new();
        for (position, captured) in self.days.iter().enumerate() {
            let day_samples = captured.samples(node);
            let followed_by_the_next_day = self
                .days
                .get(position + 1)
                .is_some_and(|next| next.day == captured.day.succ_opt().unwrap_or(next.day));
            let kept = day_samples
                .len()
                .saturating_sub(usize::from(followed_by_the_next_day));
            samples.extend(day_samples.into_iter().take(kept));
        }
        samples
    }
}
