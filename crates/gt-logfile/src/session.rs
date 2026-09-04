//! Splitting a log into boot sessions, checking each one's timestamps against
//! the order its lines are written in, and timestamping the lines that carry
//! none.

use std::ops::Range;

use chrono::{DateTime, Duration, Utc};

use crate::{
    parse::LogEntry,
    structure::{self, StructuralLine, StructuralLineKind},
};

/// Entries either side of a backwards timestamp step that are read for a clock
/// adjustment explaining it. A 622,286-line journald export logs an adjustment
/// within one entry of every one of its 31 backwards steps. Three leaves room
/// for an exporter that interleaves more between the two.
const CLOCK_ADJUSTMENT_NEIGHBOUR_ENTRIES: usize = 3;

const MICROSECONDS_PER_SECOND: i64 = 1_000_000;
const NANOSECONDS_PER_MICROSECOND: i64 = 1_000;

/// One run of the device: the entries between two reboot separators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootSession {
    /// 1-based, counting the sessions that hold at least one entry.
    pub boot_number: u32,

    /// Indexes [`crate::ParsedLog::entries`].
    pub entry_range: Range<usize>,

    /// Unset for a session no line of which carried a timestamp.
    pub anchored: Option<AnchoredBounds>,
}

impl BootSession {
    pub fn entry_count(&self) -> usize {
        self.entry_range.len()
    }

    /// How long the device ran, from the session's first anchored entry to its
    /// last. Unset for a session no line of which anchored, and for one whose
    /// clock was adjusted back past the moment the session started.
    pub fn uptime(&self) -> Option<Duration> {
        self.anchored
            .map(AnchoredBounds::uptime)
            .filter(|uptime| *uptime >= Duration::zero())
    }
}

/// The first and the last anchored timestamp of a boot session, in file order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnchoredBounds {
    pub first: DateTime<Utc>,
    pub last: DateTime<Utc>,
}

impl AnchoredBounds {
    fn from_entries(entries: &[LogEntry]) -> Option<Self> {
        let mut anchored = entries
            .iter()
            .filter(|entry| entry.is_anchored())
            .map(|entry| entry.timestamp);
        let first = anchored.next()?;
        Some(Self {
            first,
            last: anchored.next_back().unwrap_or(first),
        })
    }

    pub fn uptime(self) -> Duration {
        self.last - self.first
    }
}

/// A backwards timestamp step inside a boot session that no nearby entry
/// reports as a clock adjustment: the signature of a spliced or hand-edited log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrderAnomaly {
    pub line_number: u32,

    /// Step from the previous anchored entry, negative for a step backwards.
    pub timestamp_step: Duration,
}

/// Cuts `entries` at every reboot separator. Each session holds at least one
/// entry: a separator with no entry before the next one is skipped, so a log
/// opening on a separator reports as many boots as the device was rebooted.
pub(crate) fn segment_into_boot_sessions(
    entries: &[LogEntry],
    structural_lines: &[StructuralLine],
) -> Vec<BootSession> {
    let cuts = structural_lines
        .iter()
        .filter(|line| line.kind == StructuralLineKind::RebootSeparator)
        .map(|line| entries.partition_point(|entry| entry.line_number < line.line_number))
        .chain(std::iter::once(entries.len()));

    let mut sessions: Vec<BootSession> = Vec::new();
    let mut boot_number: u32 = 0;
    let mut start = 0;
    for end in cuts {
        let end = end.max(start);
        if end > start {
            boot_number = boot_number.saturating_add(1);
            sessions.push(BootSession {
                boot_number,
                entry_range: start..end,
                anchored: AnchoredBounds::from_entries(entries.get(start..end).unwrap_or_default()),
            });
        }
        start = end;
    }
    sessions
}

/// Records every backwards step between the anchored entries of one session
/// that no entry within [`CLOCK_ADJUSTMENT_NEIGHBOUR_ENTRIES`] of it explains.
pub(crate) fn scan_for_order_anomalies(
    text: &str,
    entries: &[LogEntry],
    anomalies: &mut Vec<OrderAnomaly>,
) {
    let mut previous: Option<DateTime<Utc>> = None;
    for (index, entry) in entries.iter().enumerate() {
        if !entry.is_anchored() {
            continue;
        }
        if let Some(previous) = previous
            && entry.timestamp < previous
            && !has_clock_adjustment_nearby(text, entries, index)
        {
            anomalies.push(OrderAnomaly {
                line_number: entry.line_number,
                timestamp_step: entry.timestamp - previous,
            });
        }
        previous = Some(entry.timestamp);
    }
}

fn has_clock_adjustment_nearby(text: &str, entries: &[LogEntry], index: usize) -> bool {
    let first = index.saturating_sub(CLOCK_ADJUSTMENT_NEIGHBOUR_ENTRIES);
    let past_last = index
        .saturating_add(CLOCK_ADJUSTMENT_NEIGHBOUR_ENTRIES)
        .saturating_add(1)
        .min(entries.len());
    entries
        .get(first..past_last)
        .unwrap_or_default()
        .iter()
        .any(|entry| structure::reports_a_clock_adjustment(entry.message.in_text(text)))
}

/// The anchors a run of untimestamped lines takes its timestamps from.
#[derive(Debug, Clone, Copy)]
enum RunTimestamps {
    /// Spread over the open span between the anchors either side of the run.
    Between {
        previous: DateTime<Utc>,
        next: DateTime<Utc>,
    },

    /// Taken whole from the one anchor the run has, at a session's start or end.
    Copied(DateTime<Utc>),
}

/// Timestamps every entry of one session that carried none, from the anchored
/// entries around it. `no_anchor_fallback` timestamps a session whose lines all
/// arrived without one.
pub(crate) fn interpolate_timestamps(entries: &mut [LogEntry], no_anchor_fallback: DateTime<Utc>) {
    let mut run_start: Option<usize> = None;
    let mut previous_anchor: Option<DateTime<Utc>> = None;

    for index in 0..entries.len() {
        let Some(anchor) = entries
            .get(index)
            .filter(|entry| entry.is_anchored())
            .map(|entry| entry.timestamp)
        else {
            run_start.get_or_insert(index);
            continue;
        };
        if let Some(start) = run_start.take()
            && let Some(run) = entries.get_mut(start..index)
        {
            match previous_anchor {
                Some(previous) => assign_run(
                    run,
                    RunTimestamps::Between {
                        previous,
                        next: anchor,
                    },
                ),
                None => assign_run(run, RunTimestamps::Copied(anchor)),
            }
        }
        previous_anchor = Some(anchor);
    }

    if let Some(start) = run_start
        && let Some(run) = entries.get_mut(start..)
    {
        let anchor = previous_anchor.unwrap_or(no_anchor_fallback);
        assign_run(run, RunTimestamps::Copied(anchor));
    }
}

fn assign_run(run: &mut [LogEntry], timestamps: RunTimestamps) {
    match timestamps {
        RunTimestamps::Copied(anchor) => {
            for entry in run {
                entry.timestamp = anchor;
            }
        }
        RunTimestamps::Between { previous, next } => {
            let steps = i128::try_from(run.len())
                .unwrap_or(i128::MAX)
                .saturating_add(1);
            let span_micros = i128::from((next - previous).num_microseconds().unwrap_or(0));
            for (offset, entry) in run.iter_mut().enumerate() {
                let step = i128::try_from(offset)
                    .unwrap_or(i128::MAX)
                    .saturating_add(1);
                let micros = i64::try_from(span_micros.saturating_mul(step) / steps).unwrap_or(0);
                entry.timestamp = duration_of_micros(micros)
                    .and_then(|elapsed| previous.checked_add_signed(elapsed))
                    .unwrap_or(previous);
            }
        }
    }
}

/// A signed microsecond count as a [`Duration`]. [`Duration::microseconds`]
/// panics on a count it cannot represent.
fn duration_of_micros(micros: i64) -> Option<Duration> {
    let nanos = micros
        .rem_euclid(MICROSECONDS_PER_SECOND)
        .saturating_mul(NANOSECONDS_PER_MICROSECOND);
    Duration::new(
        micros.div_euclid(MICROSECONDS_PER_SECOND),
        u32::try_from(nanos).ok()?,
    )
}
