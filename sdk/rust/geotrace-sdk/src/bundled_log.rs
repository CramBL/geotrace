use std::ffi::OsStr;
use std::fs;
use std::path::Path;
use std::str;

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime};

use crate::error::BundledLogError;

/// A validated raw-text log bundled inside a `.gtd` file under `logs/<name>/text`.
///
/// Constructors reject invalid names, invalid UTF-8, empty content, and logs
/// whose head does not match a recognised timestamp format.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BundledLog {
    name: String,
    text: String,
}

impl BundledLog {
    /// Read a raw-text log file from `path`, deriving the log name from its file name.
    pub fn from_file(path: impl AsRef<Path>) -> Result<Self, BundledLogError> {
        let path = path.as_ref();
        let Some(name) = path.file_name().and_then(OsStr::to_str) else {
            return Err(BundledLogError::InvalidFileName {
                path: path.to_path_buf(),
            });
        };
        let bytes = fs::read(path).map_err(|source| BundledLogError::Io {
            path: path.to_path_buf(),
            source,
        })?;
        Self::from_utf8_bytes(name, &bytes)
    }

    /// Create a validated [`BundledLog`] from a log name and UTF-8 text.
    pub fn new(name: impl Into<String>, text: impl Into<String>) -> Result<Self, BundledLogError> {
        let name = name.into();
        let text = text.into();
        validate_name(&name)?;
        validate_text(&name, &text)?;
        Ok(Self { name, text })
    }

    /// Create a validated [`BundledLog`] from a log name and raw UTF-8 bytes.
    pub fn from_utf8_bytes(name: impl Into<String>, bytes: &[u8]) -> Result<Self, BundledLogError> {
        let name = name.into();
        validate_name(&name)?;
        let text = str::from_utf8(bytes).map_err(|source| BundledLogError::InvalidUtf8 {
            name: name.clone(),
            source,
        })?;
        validate_text(&name, text)?;
        Ok(Self {
            name,
            text: text.to_owned(),
        })
    }

    /// The HDF5-safe log name stored under `logs/<name>`.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// The raw UTF-8 log text.
    pub fn text(&self) -> &str {
        &self.text
    }
}

fn validate_name(name: &str) -> Result<(), BundledLogError> {
    if name.is_empty() || name == "." || name == ".." || name.contains('/') || name.contains('\0') {
        return Err(BundledLogError::InvalidName {
            name: name.to_owned(),
        });
    }
    Ok(())
}

fn validate_text(name: &str, text: &str) -> Result<(), BundledLogError> {
    let mut non_empty_lines = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(FORMAT_DETECTION_LINE_LIMIT);
    let Some(first_line) = non_empty_lines.next() else {
        return Err(BundledLogError::Empty {
            name: name.to_owned(),
        });
    };
    if has_recognised_timestamp(first_line) || non_empty_lines.any(has_recognised_timestamp) {
        return Ok(());
    }
    Err(BundledLogError::UnrecognisedFormat {
        name: name.to_owned(),
        first_line: truncate_excerpt(first_line),
    })
}

fn truncate_excerpt(line: &str) -> String {
    let mut chars = line.chars();
    let excerpt: String = chars.by_ref().take(ERROR_LINE_EXCERPT_CHARS).collect();
    if chars.next().is_none() {
        return excerpt;
    }
    let prefix: String = line.chars().take(ERROR_LINE_EXCERPT_CHARS - 1).collect();
    format!("{prefix}{ELLIPSIS}")
}

fn has_recognised_timestamp(line: &str) -> bool {
    is_syslog(line) || is_iso_space(line) || is_iso_t(line)
}

fn is_syslog(line: &str) -> bool {
    let bytes = line.as_bytes();
    if bytes.len() < 15 {
        return false;
    }
    let Some(month) = line.get(..3).and_then(parse_month_abbrev) else {
        return false;
    };
    if bytes.get(3).copied() != Some(b' ') {
        return false;
    }
    let Some(day) = parse_day(bytes, 4) else {
        return false;
    };
    if bytes.get(6).copied() != Some(b' ') {
        return false;
    }
    let Some((hour, minute, second)) = line.get(7..).and_then(parse_hms) else {
        return false;
    };
    NaiveDate::from_ymd_opt(YEARLESS_DATE_VALIDATION_LEAP_YEAR, month, day).is_some()
        && NaiveTime::from_hms_opt(hour, minute, second).is_some()
}

fn is_iso_space(line: &str) -> bool {
    let Some(timestamp) = line.get(..19) else {
        return false;
    };
    NaiveDateTime::parse_from_str(timestamp, "%Y-%m-%d %H:%M:%S").is_ok()
}

fn is_iso_t(line: &str) -> bool {
    let end = line.find(' ').unwrap_or(line.len());
    line.get(..end)
        .is_some_and(|timestamp| DateTime::parse_from_rfc3339(timestamp).is_ok())
}

fn parse_month_abbrev(text: &str) -> Option<u32> {
    let index = MONTH_ABBREVS
        .iter()
        .position(|month| month.eq_ignore_ascii_case(text))?;
    u32::try_from(index.saturating_add(1)).ok()
}

fn parse_day(bytes: &[u8], pos: usize) -> Option<u32> {
    let text = str::from_utf8(bytes.get(pos..pos + 2)?).ok()?;
    text.trim().parse().ok()
}

fn parse_hms(text: &str) -> Option<(u32, u32, u32)> {
    let bytes = text.as_bytes();
    if bytes.get(2).copied() != Some(b':') || bytes.get(5).copied() != Some(b':') {
        return None;
    }
    let hour = text.get(..2)?.parse().ok()?;
    let minute = text.get(3..5)?.parse().ok()?;
    let second = text.get(6..8)?.parse().ok()?;
    Some((hour, minute, second))
}

const MONTH_ABBREVS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

const YEARLESS_DATE_VALIDATION_LEAP_YEAR: i32 = 2000;

const FORMAT_DETECTION_LINE_LIMIT: usize = 10;

const ERROR_LINE_EXCERPT_CHARS: usize = 120;

const ELLIPSIS: &str = "…";
