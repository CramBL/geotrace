//! The absolute times of a log's entries, or why its years cannot be resolved.

use chrono::{DateTime, Duration, Utc};
use gt_types::TimeRange;

use crate::{
    format::{RawTimestamp, YearlessTimestamp},
    parse::PendingLogEntry,
    session::{self, AnchoredBounds, BootSessionRange, OrderAnomaly},
    summary::SummaryBlock,
    years::{self, UnresolvedYear},
};

#[derive(Debug, Clone, PartialEq)]
pub enum LogChronology {
    Resolved(ResolvedChronology),
    Unresolved(UnresolvedChronology),
}

impl LogChronology {
    /// Resolves the years of `entries` from the reboot separators, the summary block bounds and
    /// `year_reference`, and interpolates the timestamps of the lines without one.
    /// Returns `None` for entries none of which has a timestamp.
    pub(crate) fn resolve(
        text: &str,
        entries: &[PendingLogEntry],
        boot_session_ranges: &[BootSessionRange],
        summary: Option<&SummaryBlock>,
        year_reference: DateTime<Utc>,
    ) -> Option<Self> {
        Some(
            match years::resolve_entry_anchors(
                text,
                entries,
                boot_session_ranges,
                summary,
                year_reference,
            ) {
                Ok(anchors) => Self::Resolved(ResolvedChronology::from_anchors(
                    text,
                    entries,
                    boot_session_ranges,
                    &anchors,
                )?),
                Err(unresolved_year) => Self::Unresolved(UnresolvedChronology {
                    unresolved_year,
                    written_timestamps: entries
                        .iter()
                        .map(|entry| match entry.timestamp {
                            Some(RawTimestamp::Yearless(timestamp)) => Some(timestamp),
                            Some(RawTimestamp::Absolute(_)) | None => None,
                        })
                        .collect(),
                }),
            },
        )
    }

    pub fn resolved(&self) -> Option<&ResolvedChronology> {
        match self {
            Self::Resolved(resolved) => Some(resolved),
            Self::Unresolved(_) => None,
        }
    }

    /// `None` for a log whose years are unresolved.
    pub fn time_range(&self) -> Option<TimeRange> {
        self.resolved().and_then(ResolvedChronology::time_range)
    }

    /// `None` for a log whose years are unresolved.
    pub fn interpolated_entry_count(&self) -> Option<usize> {
        self.resolved()
            .map(ResolvedChronology::interpolated_entry_count)
    }

    /// `None` for a log whose years are unresolved.
    pub fn session_bounds(&self, session_index: usize) -> Option<AnchoredBounds> {
        self.resolved()
            .and_then(|resolved| resolved.session_bounds(session_index))
    }

    /// `None` for a log whose years are unresolved.
    pub fn session_uptime(&self, session_index: usize) -> Option<Duration> {
        self.resolved()
            .and_then(|resolved| resolved.session_uptime(session_index))
    }

    /// Empty for a log whose years are unresolved.
    pub fn order_anomalies(&self) -> &[OrderAnomaly] {
        match self {
            Self::Resolved(resolved) => resolved.order_anomalies(),
            Self::Unresolved(_) => &[],
        }
    }
}

/// One absolute timestamp per entry of [`crate::ParsedLog::entries`], the first anchored
/// timestamp, the interpolated entry count, the anchored bounds of each boot session, and the
/// order anomalies.
#[derive(Debug, Clone, PartialEq)]
pub struct ResolvedChronology {
    timestamps: Vec<DateTime<Utc>>,
    first_anchored_timestamp: DateTime<Utc>,
    interpolated_entry_count: usize,

    /// One per boot session of [`crate::ParsedLog::boot_sessions`], `None` for a session no line
    /// of which has a timestamp.
    session_bounds: Vec<Option<AnchoredBounds>>,

    order_anomalies: Vec<OrderAnomaly>,
}

impl ResolvedChronology {
    /// Returns `None` when no entry has an anchor.
    ///
    /// A session no line of which anchored takes the last anchor before it, starting at the first
    /// anchor of the log: interpolation never crosses a session boundary.
    fn from_anchors(
        text: &str,
        entries: &[PendingLogEntry],
        boot_session_ranges: &[BootSessionRange],
        anchors: &[Option<DateTime<Utc>>],
    ) -> Option<Self> {
        let first_anchored_timestamp = anchors.iter().flatten().next().copied()?;
        let mut timestamps = Vec::with_capacity(anchors.len());
        let mut session_bounds = Vec::with_capacity(boot_session_ranges.len());
        let mut order_anomalies = Vec::new();
        let mut anchor_before_session = first_anchored_timestamp;
        for range in boot_session_ranges {
            let session_entries = entries.get(range.entry_range.clone()).unwrap_or_default();
            let session_anchors = anchors.get(range.entry_range.clone()).unwrap_or_default();
            session::scan_for_order_anomalies(
                text,
                session_entries,
                session_anchors,
                &mut order_anomalies,
            );
            timestamps.extend(session::interpolate_timestamps(
                session_anchors,
                anchor_before_session,
            ));
            let bounds = AnchoredBounds::of_session(session_anchors);
            if let Some(bounds) = bounds {
                anchor_before_session = bounds.last;
            }
            session_bounds.push(bounds);
        }
        debug_assert_eq!(
            timestamps.len(),
            anchors.len(),
            "the boot sessions cover every entry"
        );
        Some(Self {
            timestamps,
            first_anchored_timestamp,
            interpolated_entry_count: anchors.iter().filter(|anchor| anchor.is_none()).count(),
            session_bounds,
            order_anomalies,
        })
    }

    pub fn timestamps(&self) -> &[DateTime<Utc>] {
        &self.timestamps
    }

    pub fn timestamp(&self, entry_index: usize) -> Option<DateTime<Utc>> {
        self.timestamps.get(entry_index).copied()
    }

    /// The earliest to the latest entry timestamp, interpolated ones included.
    ///
    /// A reboot steps the device clock, so the last entry in file order is not
    /// always the latest one.
    pub fn time_range(&self) -> Option<TimeRange> {
        let mut timestamps = self.timestamps.iter().copied();
        let first = timestamps.next()?;
        Some(TimeRange::spanning(first, timestamps))
    }

    /// The timestamp of the first entry with a timestamp of its own.
    pub fn first_anchored_timestamp(&self) -> DateTime<Utc> {
        self.first_anchored_timestamp
    }

    pub fn interpolated_entry_count(&self) -> usize {
        self.interpolated_entry_count
    }

    /// `None` for a session no line of which has a timestamp.
    pub fn session_bounds(&self, session_index: usize) -> Option<AnchoredBounds> {
        self.session_bounds.get(session_index).copied().flatten()
    }

    /// How long the device ran in the session at `session_index`, from its first anchored entry
    /// to its last. `None` for a session no line of which anchored, and for one whose clock was
    /// adjusted back past the moment the session started.
    pub fn session_uptime(&self, session_index: usize) -> Option<Duration> {
        self.session_bounds(session_index)
            .map(AnchoredBounds::uptime)
            .filter(|uptime| *uptime >= Duration::zero())
    }

    /// Backwards timestamp steps no logged clock adjustment explains, in file order.
    pub fn order_anomalies(&self) -> &[OrderAnomaly] {
        &self.order_anomalies
    }
}

/// A log whose years the resolver cannot resolve, with the timestamps its lines write.
#[derive(Debug, Clone, PartialEq)]
pub struct UnresolvedChronology {
    unresolved_year: UnresolvedYear,

    /// One per entry of [`crate::ParsedLog::entries`], `None` for a line without a timestamp.
    /// No written timestamp has a year, because only a format without years leaves its years
    /// unresolved.
    written_timestamps: Vec<Option<YearlessTimestamp>>,
}

impl UnresolvedChronology {
    pub fn unresolved_year(&self) -> &UnresolvedYear {
        &self.unresolved_year
    }

    pub fn written_timestamp(&self, entry_index: usize) -> Option<YearlessTimestamp> {
        self.written_timestamps.get(entry_index).copied().flatten()
    }

    pub fn first_written_timestamp(&self) -> Option<YearlessTimestamp> {
        self.written_timestamps.iter().flatten().next().copied()
    }
}
