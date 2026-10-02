use chrono::{DateTime, Datelike as _, Duration, Utc};

use crate::{
    format::{self, RawTimestamp, YearlessTimestamp},
    parse::{LogEntry, LogParseError, PendingLogEntry, TimestampKind},
    summary::SummaryBlock,
};

pub(crate) fn resolve_entries(
    entries: &[PendingLogEntry],
    summary: Option<&SummaryBlock>,
    year_reference: DateTime<Utc>,
) -> Result<Vec<LogEntry>, LogParseError> {
    let yearless: Vec<_> = entries
        .iter()
        .filter_map(|entry| match entry.timestamp {
            Some(RawTimestamp::Yearless(timestamp)) => Some(YearlessAnchor {
                line_number: entry.line_number,
                timestamp,
            }),
            _ => None,
        })
        .collect();
    let bounds = ExporterBounds {
        begin: summary.and_then(|summary| summary.logs_begin_at),
        end: summary.and_then(|summary| summary.logs_end_at),
    };
    let resolved = bounds.resolve_sequence(&yearless, year_reference)?;
    let first_anchor = entries.iter().find_map(|entry| match entry.timestamp {
        Some(RawTimestamp::Absolute(timestamp)) => Some(timestamp),
        Some(RawTimestamp::Yearless(_)) => resolved.first().copied(),
        None => None,
    });
    let Some(first_anchor) = first_anchor else {
        return Ok(Vec::new());
    };
    let mut anchored = resolved.into_iter();
    entries
        .iter()
        .map(|entry| {
            let (timestamp, timestamp_kind) = match entry.timestamp {
                Some(RawTimestamp::Absolute(timestamp)) => (timestamp, TimestampKind::Anchored),
                Some(RawTimestamp::Yearless(_)) => (
                    anchored.next().ok_or(LogParseError::UnresolvedYear {
                        line_number: entry.line_number,
                    })?,
                    TimestampKind::Anchored,
                ),
                None => (first_anchor, TimestampKind::Interpolated),
            };
            Ok(LogEntry {
                timestamp,
                timestamp_kind,
                line_number: entry.line_number,
                message: entry.message,
            })
        })
        .collect()
}

#[derive(Clone, Copy)]
struct YearlessAnchor {
    line_number: u32,
    timestamp: YearlessTimestamp,
}

impl YearlessAnchor {
    fn in_year(self, year: i32) -> Result<DateTime<Utc>, LogParseError> {
        self.timestamp.in_year(year).ok_or(self.unresolved_year())
    }

    fn unresolved_year(self) -> LogParseError {
        LogParseError::UnresolvedYear {
            line_number: self.line_number,
        }
    }
}

struct ExporterBounds {
    begin: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
}

impl ExporterBounds {
    fn resolve_sequence(
        &self,
        anchors: &[YearlessAnchor],
        year_reference: DateTime<Utc>,
    ) -> Result<Vec<DateTime<Utc>>, LogParseError> {
        let Some(first) = anchors.first().copied() else {
            return Ok(Vec::new());
        };
        let direction = if self.begin.is_some() {
            FileDirection::Forward
        } else {
            FileDirection::Backward
        };
        let reference = self.begin.or(self.end).unwrap_or(year_reference);
        let latest = if self.end.is_some() {
            reference
        } else {
            reference
                .checked_add_signed(format::YEAR_REFERENCE_FUTURE_TOLERANCE)
                .unwrap_or(reference)
        };
        let mut failure = first.unresolved_year();
        // Retry the entire sequence so a leap day changes the adjacent entries' years too.
        // The search includes the eight-year leap-day interval around a Gregorian century exception.
        for offset in 0..=format::LEAP_YEAR_SEARCH_YEARS {
            let Some(year) = reference.year().checked_add(direction.year_step() * offset) else {
                continue;
            };
            match self.resolve_in_pivot_year(anchors, year, direction, latest) {
                Ok(resolved) => return Ok(resolved),
                Err(error) => failure = error,
            }
        }
        Err(failure)
    }

    fn resolve_in_pivot_year(
        &self,
        anchors: &[YearlessAnchor],
        year: i32,
        direction: FileDirection,
        latest: DateTime<Utc>,
    ) -> Result<Vec<DateTime<Utc>>, LogParseError> {
        let mut ordered = (0..anchors.len()).filter_map(|index| {
            anchors.get(match direction {
                FileDirection::Forward => index,
                FileDirection::Backward => anchors.len().saturating_sub(index + 1),
            })
        });
        let Some(pivot) = ordered.next().copied() else {
            return Ok(Vec::new());
        };
        let mut adjacent = pivot;
        let mut previous = pivot.in_year(year)?;
        if !self.contains(previous)
            || (matches!(direction, FileDirection::Backward) && previous > latest)
        {
            return Err(pivot.unresolved_year());
        }
        let mut resolved = Vec::with_capacity(anchors.len());
        resolved.push(previous);
        for entry in ordered.copied() {
            let same_year = entry.timestamp.in_year(previous.year());
            let calendar_step = same_year.map_or_else(
                || entry.timestamp.calendar_step_from(adjacent.timestamp),
                |candidate| candidate - previous,
            );
            let rollover = match direction {
                FileDirection::Forward => calendar_step < -YEAR_ROLLOVER_THRESHOLD,
                FileDirection::Backward => calendar_step > YEAR_ROLLOVER_THRESHOLD,
            };
            let next_year = || {
                previous
                    .year()
                    .checked_add(direction.year_step())
                    .and_then(|year| entry.timestamp.in_year(year))
            };
            let candidate =
                if rollover || same_year.is_some_and(|candidate| !self.contains(candidate)) {
                    next_year()
                        .filter(|candidate| self.contains(*candidate))
                        .or_else(|| same_year.filter(|candidate| self.contains(*candidate)))
                } else {
                    same_year.filter(|candidate| self.contains(*candidate))
                };
            let exact_end = if matches!(direction, FileDirection::Forward)
                && anchors
                    .last()
                    .is_some_and(|last| last.line_number == entry.line_number)
            {
                self.end.and_then(|end| {
                    entry
                        .timestamp
                        .in_year(end.year())
                        .filter(|timestamp| timestamp.timestamp() == end.timestamp())
                })
            } else {
                None
            };
            previous = exact_end
                .or(candidate)
                .filter(|candidate| self.contains(*candidate))
                .ok_or(entry.unresolved_year())?;
            resolved.push(previous);
            adjacent = entry;
        }
        if matches!(direction, FileDirection::Backward) {
            resolved.reverse();
        }
        Ok(resolved)
    }

    fn contains(&self, timestamp: DateTime<Utc>) -> bool {
        self.begin.is_none_or(|begin| timestamp >= begin)
            && self
                .end
                .is_none_or(|end| timestamp <= end || timestamp.timestamp() == end.timestamp())
    }
}

#[derive(Clone, Copy)]
enum FileDirection {
    Backward,
    Forward,
}

impl FileDirection {
    fn year_step(self) -> i32 {
        match self {
            Self::Forward => 1,
            Self::Backward => -1,
        }
    }
}

// Backward calendar steps over 183 days indicate year transitions.
// Smaller clock corrections retain their year for the order-anomaly scan.
const YEAR_ROLLOVER_THRESHOLD: Duration = Duration::days(183);
