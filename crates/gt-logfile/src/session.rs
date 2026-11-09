//! Splitting a log into boot sessions, checking each one's timestamps against
//! the order its lines are written in, and timestamping the lines that carry
//! none.

use std::ops::Range;

use chrono::{DateTime, Duration, Utc};

use crate::{
    format::{self, RawTimestamp},
    parse::{PendingLogEntry, TextSlice},
    structure::{self, StructuralLine, StructuralLineKind},
};

/// One run of the device: the entries between two reboot separators.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BootSession {
    /// 1-based, counting the sessions that hold at least one entry.
    pub boot_number: u32,

    /// Indexes [`crate::ParsedLog::entries`].
    pub entry_range: Range<usize>,

    /// Index in [`crate::ParsedLog::entries`] of the first entry after the clock jump that follows
    /// the session's reboot separator, or `entry_range.start` for a session without one.
    pub first_synchronised_entry: usize,
}

impl BootSession {
    pub fn entry_count(&self) -> usize {
        self.entry_range.len()
    }

    pub fn unsynchronised_entries(&self) -> Range<usize> {
        self.entry_range.start..self.first_synchronised_entry
    }
}

/// The first and the last anchored timestamp of a boot session, in file order.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnchoredBounds {
    pub first: DateTime<Utc>,
    pub last: DateTime<Utc>,
}

impl AnchoredBounds {
    /// `None` for a session no line of which carried a timestamp.
    pub(crate) fn of_session(anchors: &[Option<DateTime<Utc>>]) -> Option<Self> {
        let mut anchored = anchors.iter().flatten().copied();
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

/// The entries of one boot session, cut before their years resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BootSessionRange {
    pub entry_range: Range<usize>,

    /// Index of the first entry with a synchronised clock. The entries from `entry_range.start`
    /// up to it follow a reboot separator and precede a clock jump.
    pub first_synchronised_entry: usize,
}

impl BootSessionRange {
    pub fn unsynchronised_entries(&self) -> Range<usize> {
        self.entry_range.start..self.first_synchronised_entry
    }
}

/// Cuts `entries` at every reboot separator. Each session holds at least one
/// entry: a separator with no entry before the next one is skipped, so a log
/// opening on a separator reports as many boots as the device was rebooted.
pub(crate) fn segment_into_boot_session_ranges(
    text: &str,
    entries: &[PendingLogEntry],
    structural_lines: &[StructuralLine],
) -> Vec<BootSessionRange> {
    let cuts = structural_lines
        .iter()
        .filter(|line| line.kind == StructuralLineKind::RebootSeparator)
        .map(|line| entries.partition_point(|entry| entry.line_number < line.line_number))
        .chain(std::iter::once(entries.len()));

    let mut sessions = Vec::new();
    let mut start = 0;
    let mut opened_by_reboot_separator = false;
    for end in cuts {
        let end = end.max(start);
        if end > start {
            let session_entries = entries.get(start..end).unwrap_or_default();
            let first_synchronised_entry = if opened_by_reboot_separator {
                start + first_entry_after_a_clock_jump(text, session_entries).unwrap_or(0)
            } else {
                start
            };
            sessions.push(BootSessionRange {
                entry_range: start..end,
                first_synchronised_entry,
            });
        }
        start = end;
        opened_by_reboot_separator = true;
    }
    sessions
}

pub(crate) fn number_boot_sessions(ranges: &[BootSessionRange]) -> Vec<BootSession> {
    ranges
        .iter()
        .zip(1..)
        .map(|(range, boot_number)| BootSession {
            boot_number,
            entry_range: range.entry_range.clone(),
            first_synchronised_entry: range.first_synchronised_entry,
        })
        .collect()
}

/// Records every backwards step between the anchored entries of one session
/// that no entry within [`CLOCK_ADJUSTMENT_NEIGHBOUR_ENTRIES`] of it explains.
/// `anchors` has one element per entry of `entries`, `None` for a line without a timestamp.
pub(crate) fn scan_for_order_anomalies(
    text: &str,
    entries: &[PendingLogEntry],
    anchors: &[Option<DateTime<Utc>>],
    anomalies: &mut Vec<OrderAnomaly>,
) {
    let mut previous: Option<DateTime<Utc>> = None;
    for (index, (entry, anchor)) in entries.iter().zip(anchors).enumerate() {
        let Some(timestamp) = *anchor else {
            continue;
        };
        if let Some(previous) = previous
            && timestamp < previous
            && !any_reports_a_clock_adjustment(
                text,
                clock_adjustment_neighbours(entries, index)
                    .iter()
                    .map(|entry| entry.message),
            )
        {
            anomalies.push(OrderAnomaly {
                line_number: entry.line_number,
                timestamp_step: timestamp - previous,
            });
        }
        previous = Some(timestamp);
    }
}

/// Offset in `session_entries` of the first entry after a clock jump: a forward calendar step
/// past [`format::YEAR_ROLLOVER_THRESHOLD`] within [`CLOCK_ADJUSTMENT_NEIGHBOUR_ENTRIES`] of a
/// clock adjustment. A backward calendar step can be a New Year, and stays in the year transition
/// chain.
fn first_entry_after_a_clock_jump(
    text: &str,
    session_entries: &[PendingLogEntry],
) -> Option<usize> {
    let mut previous = None;
    for (offset, entry) in session_entries.iter().enumerate() {
        let Some(RawTimestamp::Yearless(timestamp)) = entry.timestamp else {
            continue;
        };
        if let Some(previous) = previous
            && timestamp.calendar_step_from(previous) > format::YEAR_ROLLOVER_THRESHOLD
            && any_reports_a_clock_adjustment(
                text,
                clock_adjustment_neighbours(session_entries, offset)
                    .iter()
                    .map(|entry| entry.message),
            )
        {
            return Some(offset);
        }
        previous = Some(timestamp);
    }
    None
}

fn clock_adjustment_neighbours<Entry>(entries: &[Entry], index: usize) -> &[Entry] {
    let first = index.saturating_sub(CLOCK_ADJUSTMENT_NEIGHBOUR_ENTRIES);
    let past_last = index
        .saturating_add(CLOCK_ADJUSTMENT_NEIGHBOUR_ENTRIES)
        .saturating_add(1)
        .min(entries.len());
    entries.get(first..past_last).unwrap_or_default()
}

fn any_reports_a_clock_adjustment(
    text: &str,
    messages: impl IntoIterator<Item = TextSlice>,
) -> bool {
    messages
        .into_iter()
        .any(|message| structure::reports_a_clock_adjustment(message.in_text(text)))
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

impl RunTimestamps {
    fn timestamps_of_run(self, run_length: usize) -> impl Iterator<Item = DateTime<Utc>> {
        let steps = i128::try_from(run_length)
            .unwrap_or(i128::MAX)
            .saturating_add(1);
        (1..=run_length).map(move |position| match self {
            Self::Copied(anchor) => anchor,
            Self::Between { previous, next } => {
                let span_micros = i128::from((next - previous).num_microseconds().unwrap_or(0));
                let step = i128::try_from(position).unwrap_or(i128::MAX);
                let micros = i64::try_from(span_micros.saturating_mul(step) / steps).unwrap_or(0);
                duration_of_micros(micros)
                    .and_then(|elapsed| previous.checked_add_signed(elapsed))
                    .unwrap_or(previous)
            }
        })
    }
}

/// One timestamp per entry of one session: the anchor of an anchored entry, and for every other
/// entry one derived from the anchors around it. `no_anchor_fallback` timestamps a session
/// whose lines all arrived without one.
pub(crate) fn interpolate_timestamps(
    anchors: &[Option<DateTime<Utc>>],
    no_anchor_fallback: DateTime<Utc>,
) -> Vec<DateTime<Utc>> {
    let mut timestamps = Vec::with_capacity(anchors.len());
    let mut previous_anchor: Option<DateTime<Utc>> = None;
    let mut run_length = 0;
    for anchor in anchors {
        let Some(anchor) = *anchor else {
            run_length += 1;
            continue;
        };
        let run = match previous_anchor {
            Some(previous) => RunTimestamps::Between {
                previous,
                next: anchor,
            },
            None => RunTimestamps::Copied(anchor),
        };
        timestamps.extend(run.timestamps_of_run(run_length));
        timestamps.push(anchor);
        run_length = 0;
        previous_anchor = Some(anchor);
    }
    timestamps.extend(
        RunTimestamps::Copied(previous_anchor.unwrap_or(no_anchor_fallback))
            .timestamps_of_run(run_length),
    );
    timestamps
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

/// Entries either side of a backwards timestamp step that are read for a clock
/// adjustment explaining it. A 622,286-line journald export logs an adjustment
/// within one entry of every one of its 31 backwards steps. Three leaves room
/// for an exporter that interleaves more between the two.
const CLOCK_ADJUSTMENT_NEIGHBOUR_ENTRIES: usize = 3;

const MICROSECONDS_PER_SECOND: i64 = 1_000_000;
const NANOSECONDS_PER_MICROSECOND: i64 = 1_000;
