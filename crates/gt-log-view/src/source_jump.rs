use std::num::NonZeroU32;

use chrono::{DateTime, NaiveDateTime, Utc};
use gt_ui_types::LoadedLogId;
use thiserror::Error;

use crate::LoadedLogs;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LogJumpRequest {
    GoToLine(NonZeroU32),
    GoToTime(DateTime<Utc>),
}

impl LogJumpRequest {
    pub fn parse_line(text: &str) -> Result<Self, InvalidLogJump> {
        text.trim()
            .parse()
            .map(Self::GoToLine)
            .map_err(|_invalid_line_number| InvalidLogJump::RequirePhysicalLine)
    }

    pub fn parse_time(text: &str) -> Result<Self, InvalidLogJump> {
        let text = text.trim();
        if text.split_once('.').is_some_and(|(_, fraction)| {
            fraction
                .bytes()
                .take_while(u8::is_ascii_digit)
                .take(NANOSECOND_FRACTION_DIGITS + 1)
                .count()
                > NANOSECOND_FRACTION_DIGITS
        }) {
            return Err(InvalidLogJump::RequireSupportedPrecision);
        }
        let utc = text.strip_suffix(UTC_INPUT_SUFFIX).and_then(|date| {
            UTC_INPUT_FORMATS.iter().find_map(|format| {
                NaiveDateTime::parse_from_str(date, format)
                    .ok()
                    .map(|time| time.and_utc())
            })
        });
        let time = utc
            .or_else(|| {
                DateTime::parse_from_rfc3339(text)
                    .ok()
                    .map(|time| time.with_timezone(&Utc))
            })
            .or_else(|| {
                OFFSET_INPUT_FORMATS.iter().find_map(|format| {
                    DateTime::parse_from_str(text, format)
                        .ok()
                        .map(|time| time.with_timezone(&Utc))
                })
            });
        time.map(Self::GoToTime)
            .ok_or(InvalidLogJump::RequireZonedDateTime)
    }

    pub fn resolve(
        self,
        logs: &LoadedLogs,
        id: LoadedLogId,
    ) -> Result<LogJumpTarget, UnresolvedLogJump> {
        let log = logs
            .get_by_id(id)
            .ok_or(UnresolvedLogJump::ReportUnavailableLog)?;
        let parsed = log.parsed();
        let line_number = match self {
            Self::GoToLine(line) => {
                parsed
                    .source_line(line.get())
                    .ok_or(UnresolvedLogJump::ReportNoLine)?;
                line.get()
            }
            Self::GoToTime(requested) => {
                let chronology = parsed
                    .chronology()
                    .resolved()
                    .ok_or(UnresolvedLogJump::ReportUnavailableDates)?;
                parsed
                    .entries()
                    .iter()
                    .enumerate()
                    .filter_map(|(index, entry)| {
                        let time = chronology.eligible_time(index)?.timestamp();
                        (time >= requested).then_some((time, entry.line_number))
                    })
                    .min_by_key(|&(time, line)| (time, line))
                    .map(|(_, line)| line)
                    .ok_or(UnresolvedLogJump::ReportNoTime)?
            }
        };
        Ok(LogJumpTarget {
            log: id,
            line_number,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LogJumpTarget {
    log: LoadedLogId,
    line_number: u32,
}

impl LogJumpTarget {
    pub fn log(self) -> LoadedLogId {
        self.log
    }

    pub fn line_number(self) -> u32 {
        self.line_number
    }
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum InvalidLogJump {
    #[error("Enter a physical line number greater than zero")]
    RequirePhysicalLine,
    #[error("Use at most nine fractional second digits")]
    RequireSupportedPrecision,
    #[error("Enter a date, time and UTC or a numeric offset")]
    RequireZonedDateTime,
}

#[derive(Clone, Copy, Debug, Eq, Error, PartialEq)]
pub enum UnresolvedLogJump {
    #[error("Source line unavailable")]
    ReportNoLine,
    #[error("No entry at or after this time")]
    ReportNoTime,
    #[error("Dates unresolved")]
    ReportUnavailableDates,
    #[error("Log unavailable")]
    ReportUnavailableLog,
}

const NANOSECOND_FRACTION_DIGITS: usize = 9;

const UTC_INPUT_FORMATS: [&str; 2] = ["%Y-%m-%d %H:%M:%S%.f", "%Y-%m-%d %H:%M"];
const UTC_INPUT_SUFFIX: &str = " UTC";
const OFFSET_INPUT_FORMATS: [&str; 2] = ["%Y-%m-%d %H:%M:%S%.f %:z", "%Y-%m-%d %H:%M %:z"];

#[cfg(test)]
mod tests;
