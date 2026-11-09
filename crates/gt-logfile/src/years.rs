use std::fmt;
use std::ops::Range;

use chrono::{DateTime, Datelike as _, Utc};

use crate::{
    format::{self, RawTimestamp, YearlessTimestamp},
    parse::{self, LogEntry, LogParseError, PendingLogEntry, TextSlice, TimestampKind},
    session::BootSessionRange,
    summary::SummaryBlock,
};

/// Why the resolver rejects the candidate year for an entry whose timestamp omits the year.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum YearConflict {
    FollowLogsEndAt {
        candidate: DateTime<Utc>,
        logs_end_at: DateTime<Utc>,
    },

    /// Compared only for the last entry when no exporter bound applies, including when the only
    /// bound is a "Logs begin at" dropped for a first entry without a synchronised clock.
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
    boot_session_ranges: &[BootSessionRange],
    summary: Option<&SummaryBlock>,
    year_reference: DateTime<Utc>,
) -> Result<Vec<LogEntry>, LogParseError> {
    let unsynchronised_ranges: Vec<Range<usize>> = boot_session_ranges
        .iter()
        .map(BootSessionRange::unsynchronised_entries)
        .filter(|range| !range.is_empty())
        .collect();
    let is_synchronised = |index: usize| {
        unsynchronised_ranges
            .get(unsynchronised_ranges.partition_point(|range| range.end <= index))
            .is_none_or(|range| !range.contains(&index))
    };
    let yearless: Vec<_> = entries
        .iter()
        .enumerate()
        .filter_map(|(index, entry)| match entry.timestamp {
            Some(RawTimestamp::Yearless(timestamp)) => Some(YearlessAnchor {
                line_number: entry.line_number,
                timestamp,
                message: entry.message,
                synchronised: is_synchronised(index),
            }),
            _ => None,
        })
        .collect();
    let resolved = resolve_anchors(&yearless, summary, year_reference)
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
                        "resolve_anchors returns one timestamp per yearless entry"
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

/// Resolves the synchronised anchors as one chronology.
/// An anchor without a synchronised clock takes the latest year at or before the next
/// synchronised anchor.
fn resolve_anchors(
    anchors: &[YearlessAnchor],
    summary: Option<&SummaryBlock>,
    year_reference: DateTime<Utc>,
) -> Result<Vec<DateTime<Utc>>, YearResolutionFailure> {
    let synchronised: Vec<YearlessAnchor> = anchors
        .iter()
        .copied()
        .filter(|anchor| anchor.synchronised)
        .collect();
    // The resolver drops "Logs begin at" for a first entry without a synchronised clock: the bound
    // is the time of its reset clock.
    let bounds = ExporterBounds {
        begin: summary
            .and_then(|summary| summary.logs_begin_at)
            .filter(|_| anchors.first().is_some_and(|anchor| anchor.synchronised)),
        end: summary.and_then(|summary| summary.logs_end_at),
    };
    let mut synchronised_resolved = bounds
        .resolve_sequence(&synchronised, year_reference)?
        .into_iter()
        .rev();
    let mut resolved = Vec::with_capacity(anchors.len());
    let mut next_synchronised = None;
    for anchor in anchors.iter().rev() {
        if anchor.synchronised {
            let timestamp = synchronised_resolved.next();
            debug_assert!(
                timestamp.is_some(),
                "resolve_sequence returns one timestamp per synchronised entry"
            );
            next_synchronised = timestamp.or(next_synchronised);
            resolved.extend(timestamp);
        } else {
            let ceiling = next_synchronised.unwrap_or(year_reference);
            resolved.push(anchor.place_at_or_before(ceiling, resolved.len())?);
        }
    }
    resolved.reverse();
    Ok(resolved)
}

#[derive(Clone, Copy)]
struct YearlessAnchor {
    line_number: u32,
    timestamp: YearlessTimestamp,
    message: TextSlice,
    synchronised: bool,
}

impl YearlessAnchor {
    fn place_at_or_before(
        self,
        ceiling: DateTime<Utc>,
        resolved_entry_count: usize,
    ) -> Result<DateTime<Utc>, YearResolutionFailure> {
        self.timestamp
            .latest_at_or_before(ceiling)
            .ok_or(YearResolutionFailure {
                anchor: self,
                conflict: YearConflict::LackDate {
                    year: ceiling.year(),
                },
                resolved_entry_count,
            })
    }

    fn written_timestamp(self, text: &str) -> String {
        gt_fmt::truncate_with_ellipsis(
            self.message.trimmed_line_text_before_in(text),
            parse::ERROR_LINE_EXCERPT_CHARS,
        )
        .into_owned()
    }
}

/// The entry at which year resolution stopped.
struct YearResolutionFailure {
    anchor: YearlessAnchor,
    conflict: YearConflict,
    resolved_entry_count: usize,
}

impl YearResolutionFailure {
    fn into_parse_error(self, text: &str) -> LogParseError {
        LogParseError::UnresolvedYear {
            line_number: self.anchor.line_number,
            written_timestamp: self.anchor.written_timestamp(text),
            conflict: self.conflict,
        }
    }
}

/// "Logs begin at" and "Logs end at" state the times of the first and last entries.
/// Both bound the years of those two entries, and of a middle entry when one of its candidate
/// years is within them.
struct ExporterBounds {
    begin: Option<DateTime<Utc>>,
    end: Option<DateTime<Utc>>,
}

impl ExporterBounds {
    fn date_in_year(
        timestamp: YearlessTimestamp,
        year: i32,
    ) -> Result<DateTime<Utc>, YearConflict> {
        timestamp
            .in_year(year)
            .ok_or(YearConflict::LackDate { year })
    }

    fn resolve_sequence(
        &self,
        anchors: &[YearlessAnchor],
        year_reference: DateTime<Utc>,
    ) -> Result<Vec<DateTime<Utc>>, YearResolutionFailure> {
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
    ) -> Result<Vec<DateTime<Utc>>, YearResolutionFailure> {
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
            .map_err(|conflict| YearResolutionFailure {
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
                FileDirection::Forward => calendar_step < -format::YEAR_ROLLOVER_THRESHOLD,
                FileDirection::Backward => calendar_step > format::YEAR_ROLLOVER_THRESHOLD,
            };
            let is_last_entry = anchors
                .last()
                .is_some_and(|last| last.line_number == entry.line_number);
            let is_first_entry = anchors
                .first()
                .is_some_and(|first| first.line_number == entry.line_number);
            let exact_logs_end_at = self
                .in_the_second_of_logs_end_at(entry.timestamp)
                .filter(|_| matches!(direction, FileDirection::Forward) && is_last_entry);
            let bounds_constraint = if is_first_entry || is_last_entry {
                BoundsConstraint::Require
            } else {
                BoundsConstraint::Prefer
            };
            let placement = match exact_logs_end_at {
                Some(exact_logs_end_at) => self.within_bounds(exact_logs_end_at),
                None => self.place_chronologically(
                    entry.timestamp,
                    previous.year(),
                    direction,
                    rollover,
                    bounds_constraint,
                ),
            };
            previous = placement.map_err(|conflict| YearResolutionFailure {
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
        self.within_bounds(Self::date_in_year(timestamp, year)?)
    }

    /// After a rollover the next year is tried first, otherwise the same year.
    /// Returns the conflict of the first year tried, or under [`BoundsConstraint::Prefer`] the
    /// first year tried with the date.
    /// Skips the next year when the same year lacks the date.
    fn place_chronologically(
        &self,
        timestamp: YearlessTimestamp,
        previous_year: i32,
        direction: FileDirection,
        rollover: bool,
        bounds_constraint: BoundsConstraint,
    ) -> Result<DateTime<Utc>, YearConflict> {
        let next_year = previous_year.saturating_add(direction.year_step());
        let (first_year, second_year) = if rollover {
            (next_year, previous_year)
        } else {
            (previous_year, next_year)
        };
        let first_candidate = Self::date_in_year(timestamp, first_year);
        if !rollover && first_candidate.is_err() {
            return first_candidate;
        }
        let within_bounds = |candidate| self.within_bounds(candidate);
        let first_conflict = match first_candidate.and_then(within_bounds) {
            Ok(placed) => return Ok(placed),
            Err(conflict) => conflict,
        };
        let second_candidate = Self::date_in_year(timestamp, second_year);
        if let Ok(placed) = second_candidate.and_then(within_bounds) {
            return Ok(placed);
        }
        match bounds_constraint {
            BoundsConstraint::Prefer => {
                first_candidate.or(second_candidate).or(Err(first_conflict))
            }
            BoundsConstraint::Require => Err(first_conflict),
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

#[derive(Clone, Copy)]
enum BoundsConstraint {
    /// Takes the first year tried with the date when no year tried is within the bounds.
    Prefer,

    /// Fails when no year tried is within the bounds.
    Require,
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

const LOGS_BEGIN_AT: &str = "Logs begin at";

const LOGS_END_AT: &str = "Logs end at";

const REFERENCE_DATE: &str = "Reference date";
