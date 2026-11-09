use std::fmt;

use chrono::{DateTime, Datelike as _, Duration, Utc};

use crate::{
    format::{self, RawTimestamp, YearlessTimestamp},
    parse::{self, LogEntry, LogParseError, PendingLogEntry, TextSlice, TimestampKind},
    summary::SummaryBlock,
};

/// Why the resolver rejects the candidate year for an entry whose timestamp omits the year.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum YearConflict {
    FollowLogsEndAt {
        candidate: DateTime<Utc>,
        logs_end_at: DateTime<Utc>,
    },

    /// Compared only for the last entry of a log without exporter bounds.
    FollowYearReference {
        candidate: DateTime<Utc>,
        year_reference: DateTime<Utc>,
    },

    /// February 29 in a non-leap year, or a year outside the range of [`DateTime`].
    LackDate { year: i32 },

    PrecedeLogsBeginAt {
        candidate: DateTime<Utc>,
        logs_begin_at: DateTime<Utc>,
    },
}

impl YearConflict {
    /// Sentence case, without the instants of [`YearConflict::bound_comparison`].
    /// `Display` writes it with a lowercase first character.
    pub fn reason(self) -> String {
        match self {
            Self::FollowLogsEndAt { .. } => format!("After the summary block's \"{LOGS_END_AT}\""),
            Self::FollowYearReference { .. } => format!(
                "More than {} after the {}",
                gt_fmt::format_human_terse_duration(format::YEAR_REFERENCE_FUTURE_TOLERANCE),
                with_lowercase_first_char(REFERENCE_DATE)
            ),
            Self::LackDate { year } => format!("The date does not exist in {year}"),
            Self::PrecedeLogsBeginAt { .. } => {
                format!("Before the summary block's \"{LOGS_BEGIN_AT}\"")
            }
        }
    }

    /// Returns `None` for [`YearConflict::LackDate`], which has no candidate instant.
    pub fn bound_comparison(self) -> Option<BoundComparison> {
        match self {
            Self::FollowLogsEndAt {
                candidate,
                logs_end_at,
            } => Some(BoundComparison {
                candidate,
                bound_name: LOGS_END_AT,
                bound: logs_end_at,
            }),
            Self::FollowYearReference {
                candidate,
                year_reference,
            } => Some(BoundComparison {
                candidate,
                bound_name: REFERENCE_DATE,
                bound: year_reference,
            }),
            Self::LackDate { .. } => None,
            Self::PrecedeLogsBeginAt {
                candidate,
                logs_begin_at,
            } => Some(BoundComparison {
                candidate,
                bound_name: LOGS_BEGIN_AT,
                bound: logs_begin_at,
            }),
        }
    }
}

impl fmt::Display for YearConflict {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let lowercase_reason = with_lowercase_first_char(&self.reason());
        match self.bound_comparison() {
            Some(BoundComparison {
                candidate, bound, ..
            }) => write!(
                f,
                "{} UTC is {lowercase_reason} {} UTC",
                candidate.format(gt_fmt::UTC_SUBSECOND_FORMAT),
                bound.format(gt_fmt::UTC_SUBSECOND_FORMAT)
            ),
            None => f.write_str(&lowercase_reason),
        }
    }
}

/// The candidate instant of a [`YearConflict`] and the instant it conflicts with.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct BoundComparison {
    pub candidate: DateTime<Utc>,
    pub bound_name: &'static str,
    pub bound: DateTime<Utc>,
}

fn with_lowercase_first_char(phrase: &str) -> String {
    match phrase.split_at_checked(1) {
        Some((first, rest)) => format!("{}{rest}", first.to_ascii_lowercase()),
        None => phrase.to_owned(),
    }
}

pub(crate) fn resolve_entries(
    text: &str,
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
                message: entry.message,
            }),
            _ => None,
        })
        .collect();
    let bounds = ExporterBounds {
        begin: summary.and_then(|summary| summary.logs_begin_at),
        end: summary.and_then(|summary| summary.logs_end_at),
    };
    let resolved = bounds
        .resolve_sequence(&yearless, year_reference)
        .map_err(|failure| failure.into_parse_error(text))?;
    let first_anchor = entries.iter().find_map(|entry| match entry.timestamp {
        Some(RawTimestamp::Absolute(timestamp)) => Some(timestamp),
        Some(RawTimestamp::Yearless(_)) => resolved.first().copied(),
        None => None,
    });
    let Some(first_anchor) = first_anchor else {
        return Ok(Vec::new());
    };
    let mut anchored = resolved.into_iter();
    Ok(entries
        .iter()
        .map(|entry| {
            let (timestamp, timestamp_kind) = match entry.timestamp {
                Some(RawTimestamp::Absolute(timestamp)) => (timestamp, TimestampKind::Anchored),
                Some(RawTimestamp::Yearless(_)) => {
                    let resolved = anchored.next();
                    debug_assert!(
                        resolved.is_some(),
                        "resolve_sequence returns one timestamp per yearless entry"
                    );
                    resolved.map_or((first_anchor, TimestampKind::Interpolated), |timestamp| {
                        (timestamp, TimestampKind::Anchored)
                    })
                }
                None => (first_anchor, TimestampKind::Interpolated),
            };
            LogEntry {
                timestamp,
                timestamp_kind,
                line_number: entry.line_number,
                message: entry.message,
            }
        })
        .collect())
}

#[derive(Clone, Copy)]
struct YearlessAnchor {
    line_number: u32,
    timestamp: YearlessTimestamp,
    message: TextSlice,
}

impl YearlessAnchor {
    fn written_timestamp(self, text: &str) -> String {
        gt_fmt::truncate_with_ellipsis(
            self.message.trimmed_line_text_before_in(text),
            parse::ERROR_LINE_EXCERPT_CHARS,
        )
        .into_owned()
    }
}

/// The entry at which one pivot year's attempt stopped.
struct PivotYearFailure {
    anchor: YearlessAnchor,
    conflict: YearConflict,
    resolved_entry_count: usize,
}

impl PivotYearFailure {
    fn into_parse_error(self, text: &str) -> LogParseError {
        LogParseError::UnresolvedYear {
            line_number: self.anchor.line_number,
            written_timestamp: self.anchor.written_timestamp(text),
            conflict: self.conflict,
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
    ) -> Result<Vec<DateTime<Utc>>, PivotYearFailure> {
        let direction = if self.begin.is_some() {
            FileDirection::Forward
        } else {
            FileDirection::Backward
        };
        let reference_year = self.begin.or(self.end).unwrap_or(year_reference).year();
        let pivot_ceiling = match self.end {
            Some(logs_end_at) => PivotCeiling::CapAtLogsEndAt(logs_end_at),
            None => PivotCeiling::CapAtYearReference(year_reference),
        };
        // Keeps the failure with the most resolved entries.
        // A tie keeps the pivot nearest the reference.
        let mut most_resolved_failure =
            match self.resolve_in_pivot_year(anchors, reference_year, direction, pivot_ceiling) {
                Ok(resolved) => return Ok(resolved),
                Err(failure) => failure,
            };
        // Retry the entire sequence so a leap day changes the adjacent entries' years too.
        // The search includes the eight-year leap-day interval around a Gregorian century exception.
        for offset in 1..=format::LEAP_YEAR_SEARCH_YEARS {
            let Some(year) = reference_year.checked_add(direction.year_step() * offset) else {
                continue;
            };
            match self.resolve_in_pivot_year(anchors, year, direction, pivot_ceiling) {
                Ok(resolved) => return Ok(resolved),
                Err(failure)
                    if failure.resolved_entry_count
                        > most_resolved_failure.resolved_entry_count =>
                {
                    most_resolved_failure = failure;
                }
                Err(_) => {}
            }
        }
        Err(most_resolved_failure)
    }

    fn resolve_in_pivot_year(
        &self,
        anchors: &[YearlessAnchor],
        year: i32,
        direction: FileDirection,
        pivot_ceiling: PivotCeiling,
    ) -> Result<Vec<DateTime<Utc>>, PivotYearFailure> {
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
        let mut previous = self
            .place_in_year(pivot.timestamp, year)
            .and_then(|candidate| match direction {
                FileDirection::Forward => Ok(candidate),
                FileDirection::Backward => pivot_ceiling.within_ceiling(candidate),
            })
            .map_err(|conflict| PivotYearFailure {
                anchor: pivot,
                conflict,
                resolved_entry_count: 0,
            })?;
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
            let is_last_entry_read_forward = matches!(direction, FileDirection::Forward)
                && anchors
                    .last()
                    .is_some_and(|last| last.line_number == entry.line_number);
            let exact_logs_end_at = self
                .in_the_second_of_logs_end_at(entry.timestamp)
                .filter(|_| is_last_entry_read_forward);
            let placement = match exact_logs_end_at {
                Some(exact_logs_end_at) => self.within_bounds(exact_logs_end_at),
                None => self.place_chronologically(
                    entry.timestamp,
                    previous.year(),
                    direction,
                    rollover,
                ),
            };
            previous = placement.map_err(|conflict| PivotYearFailure {
                anchor: entry,
                conflict,
                resolved_entry_count: resolved.len(),
            })?;
            resolved.push(previous);
            adjacent = entry;
        }
        if matches!(direction, FileDirection::Backward) {
            resolved.reverse();
        }
        Ok(resolved)
    }

    fn place_in_year(
        &self,
        timestamp: YearlessTimestamp,
        year: i32,
    ) -> Result<DateTime<Utc>, YearConflict> {
        self.within_bounds(
            timestamp
                .in_year(year)
                .ok_or(YearConflict::LackDate { year })?,
        )
    }

    /// After a rollover the next year is tried first, otherwise the same year.
    /// Returns the conflict of the first year tried.
    /// Skips the next year when the same year lacks the date.
    fn place_chronologically(
        &self,
        timestamp: YearlessTimestamp,
        previous_year: i32,
        direction: FileDirection,
        rollover: bool,
    ) -> Result<DateTime<Utc>, YearConflict> {
        let next_year = previous_year.saturating_add(direction.year_step());
        if rollover {
            return match self.place_in_year(timestamp, next_year) {
                Ok(candidate) => Ok(candidate),
                Err(next_year_conflict) => self
                    .place_in_year(timestamp, previous_year)
                    .or(Err(next_year_conflict)),
            };
        }
        match self.place_in_year(timestamp, previous_year) {
            Ok(candidate) => Ok(candidate),
            Err(missing_date @ YearConflict::LackDate { .. }) => Err(missing_date),
            Err(same_year_conflict) => self
                .place_in_year(timestamp, next_year)
                .or(Err(same_year_conflict)),
        }
    }

    fn in_the_second_of_logs_end_at(&self, timestamp: YearlessTimestamp) -> Option<DateTime<Utc>> {
        let end = self.end?;
        timestamp
            .in_year(end.year())
            .filter(|candidate| candidate.timestamp() == end.timestamp())
    }

    /// "Logs end at" admits every timestamp inside its own second.
    fn within_bounds(&self, candidate: DateTime<Utc>) -> Result<DateTime<Utc>, YearConflict> {
        if let Some(logs_begin_at) = self.begin.filter(|begin| candidate < *begin) {
            return Err(YearConflict::PrecedeLogsBeginAt {
                candidate,
                logs_begin_at,
            });
        }
        match self.end {
            Some(logs_end_at)
                if candidate > logs_end_at && candidate.timestamp() != logs_end_at.timestamp() =>
            {
                Err(YearConflict::FollowLogsEndAt {
                    candidate,
                    logs_end_at,
                })
            }
            _ => Ok(candidate),
        }
    }
}

/// The latest timestamp of the pivot when the resolver reads the log backward from its last
/// entry.
#[derive(Clone, Copy)]
enum PivotCeiling {
    CapAtLogsEndAt(DateTime<Utc>),

    /// Allows [`format::YEAR_REFERENCE_FUTURE_TOLERANCE`] after the year reference.
    CapAtYearReference(DateTime<Utc>),
}

impl PivotCeiling {
    fn within_ceiling(self, candidate: DateTime<Utc>) -> Result<DateTime<Utc>, YearConflict> {
        match self {
            Self::CapAtLogsEndAt(logs_end_at) if candidate > logs_end_at => {
                Err(YearConflict::FollowLogsEndAt {
                    candidate,
                    logs_end_at,
                })
            }
            Self::CapAtYearReference(year_reference)
                if candidate
                    > year_reference
                        .checked_add_signed(format::YEAR_REFERENCE_FUTURE_TOLERANCE)
                        .unwrap_or(year_reference) =>
            {
                Err(YearConflict::FollowYearReference {
                    candidate,
                    year_reference,
                })
            }
            Self::CapAtLogsEndAt(_) | Self::CapAtYearReference(_) => Ok(candidate),
        }
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

const LOGS_BEGIN_AT: &str = "Logs begin at";

const LOGS_END_AT: &str = "Logs end at";

const REFERENCE_DATE: &str = "Reference date";
