use chrono::{DateTime, Datelike as _, Duration, Utc};

use crate::{
    format,
    parse::{LogEntry, LogParseError},
    summary::SummaryBlock,
};

pub(crate) fn resolve_years(
    entries: &mut [LogEntry],
    summary: Option<&SummaryBlock>,
    year_reference: DateTime<Utc>,
) -> Result<(), LogParseError> {
    let bounds = ExporterBounds {
        begin: summary.and_then(|summary| summary.logs_begin_at),
        end: summary.and_then(|summary| summary.logs_end_at),
    };
    let last_exporter_timestamp = bounds.end.and_then(|end| {
        let last = entries.iter().rfind(|entry| entry.is_anchored())?;
        let timestamp = last.timestamp.with_year(end.year())?;
        (timestamp.timestamp() == end.timestamp()).then_some((last.line_number, timestamp))
    });
    let mut anchored = entries.iter_mut().filter(|entry| entry.is_anchored());
    if let Some(begin) = bounds.begin {
        let Some(first) = anchored.next() else {
            return Ok(());
        };
        let mut previous = bounds.first_year_at_or_after(first, begin)?;
        first.timestamp = previous;
        for entry in anchored {
            previous = match last_exporter_timestamp {
                Some((line_number, timestamp)) if line_number == entry.line_number => timestamp,
                _ => bounds.resolve_adjacent_year(entry, previous, FileDirection::Forward)?,
            };
            entry.timestamp = previous;
        }
    } else {
        let Some(last) = anchored.next_back() else {
            return Ok(());
        };
        let mut next = match bounds.end {
            Some(end) => bounds.first_year_at_or_before(last, end)?,
            None => format::infer_year(last.timestamp.naive_utc(), year_reference).ok_or(
                LogParseError::UnresolvedYear {
                    line_number: last.line_number,
                },
            )?,
        };
        last.timestamp = next;
        for entry in anchored.rev() {
            next = bounds.resolve_adjacent_year(entry, next, FileDirection::Backward)?;
            entry.timestamp = next;
        }
    }
    Ok(())
}

struct ExporterBounds {
    begin: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
}

impl ExporterBounds {
    fn first_year_at_or_after(
        &self,
        entry: &LogEntry,
        begin: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, LogParseError> {
        (0..=format::LEAP_YEAR_SEARCH_YEARS)
            .filter_map(|offset| entry.timestamp.with_year(begin.year().checked_add(offset)?))
            .find(|candidate| self.contains(*candidate) && *candidate >= begin)
            .ok_or(LogParseError::UnresolvedYear {
                line_number: entry.line_number,
            })
    }

    fn first_year_at_or_before(
        &self,
        entry: &LogEntry,
        end: DateTime<Utc>,
    ) -> Result<DateTime<Utc>, LogParseError> {
        (0..=format::LEAP_YEAR_SEARCH_YEARS)
            .filter_map(|offset| entry.timestamp.with_year(end.year().checked_sub(offset)?))
            .find(|candidate| self.contains(*candidate))
            .ok_or(LogParseError::UnresolvedYear {
                line_number: entry.line_number,
            })
    }

    fn resolve_adjacent_year(
        &self,
        entry: &LogEntry,
        adjacent: DateTime<Utc>,
        direction: FileDirection,
    ) -> Result<DateTime<Utc>, LogParseError> {
        let same_year = entry.timestamp.with_year(adjacent.year());
        let step = same_year.map(|candidate| candidate - adjacent);
        let year_step = match direction {
            FileDirection::Forward => 1,
            FileDirection::Backward => -1,
        };
        let rollover = step.is_none_or(|step| match direction {
            FileDirection::Forward => step < -YEAR_ROLLOVER_THRESHOLD,
            FileDirection::Backward => step > YEAR_ROLLOVER_THRESHOLD,
        });
        let same_year_within_bounds = same_year.filter(|candidate| self.contains(*candidate));
        let resolved = if rollover || same_year_within_bounds.is_none() {
            (1..=format::LEAP_YEAR_SEARCH_YEARS)
                .filter_map(|offset| {
                    entry
                        .timestamp
                        .with_year(adjacent.year().checked_add(year_step * offset)?)
                })
                .find(|candidate| self.contains(*candidate))
                .or(same_year_within_bounds)
        } else {
            same_year_within_bounds
        };
        resolved.ok_or(LogParseError::UnresolvedYear {
            line_number: entry.line_number,
        })
    }

    fn contains(&self, timestamp: DateTime<Utc>) -> bool {
        self.begin.is_none_or(|begin| timestamp >= begin)
            && self
                .end
                .is_none_or(|end| timestamp <= end || (timestamp.timestamp() == end.timestamp()))
    }
}

#[derive(Clone, Copy)]
enum FileDirection {
    Backward,
    Forward,
}

// Year resolution treats backward calendar steps over 183 days as year transitions.
// Smaller clock corrections retain their year for the order-anomaly scan.
const YEAR_ROLLOVER_THRESHOLD: Duration = Duration::days(183);
