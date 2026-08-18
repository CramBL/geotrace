//! Detecting a log's format from its head, indexing every line against its
//! text, and reading the structure the exporter wrote around those lines.

use std::{num::NonZeroUsize, sync::Arc};

use chrono::{DateTime, Utc};
use gt_types::TimeRange;
use rayon::prelude::*;

use crate::{
    format::{self, LogFormat},
    pool,
    session::{self, BootSession, OrderAnomaly},
    structure::{StructuralExtent, StructuralLine, StructuralLineKind},
    summary::{self, EntryCountMismatch, SummaryBlock},
};

/// Non-empty lines the format detector reads before giving up on the log.
const FORMAT_DETECTION_LINE_LIMIT: usize = 10;

/// Characters of the offending line quoted in [`LogParseError::NoRecognisedFormat`].
const ERROR_LINE_EXCERPT_CHARS: NonZeroUsize = match NonZeroUsize::new(120) {
    Some(chars) => chars,
    None => NonZeroUsize::MIN,
};

/// Text one worker indexes, before the chunk's end is aligned forward to the
/// next newline. A log shorter than this is indexed on the calling thread.
const CHUNK_TARGET_BYTES: NonZeroUsize = match NonZeroUsize::new(16 * 1024 * 1024) {
    Some(bytes) => bytes,
    None => NonZeroUsize::MIN,
};

/// A byte range of a [`ParsedLog`]'s text, read with [`TextSlice::in_text`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TextSlice {
    pub offset: u64,
    pub len: u32,
}

impl TextSlice {
    /// `None` for a line longer than the index's length field can address.
    fn new(offset: u64, len: usize) -> Option<Self> {
        Some(Self {
            offset,
            len: u32::try_from(len).ok()?,
        })
    }

    pub fn in_text(self, text: &str) -> &str {
        let start = usize::try_from(self.offset).unwrap_or(usize::MAX);
        let end = start.saturating_add(self.len as usize);
        let slice = text.get(start..end);
        debug_assert!(
            slice.is_some(),
            "{self:?} addresses text outside the log it was indexed from"
        );
        slice.unwrap_or_default()
    }
}

/// Where an entry's timestamp came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimestampKind {
    /// Parsed from the line itself.
    Anchored,

    /// Derived from the anchored entries around it, the line carrying none.
    Interpolated,
}

/// One line of a log kept as an entry: its timestamp and the byte range of its
/// message inside the text of the [`ParsedLog`] it was indexed from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub timestamp_kind: TimestampKind,

    /// 1-based, counting every physical line of the log.
    pub line_number: u32,

    /// The text after the timestamp, or the whole line for an interpolated entry.
    pub message: TextSlice,
}

impl LogEntry {
    pub fn is_anchored(&self) -> bool {
        self.timestamp_kind == TimestampKind::Anchored
    }
}

/// A log read into the text it was parsed from, an index over its lines, and
/// the structure recognized around them.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedLog {
    text: Arc<str>,
    entries: Vec<LogEntry>,
    boot_sessions: Vec<BootSession>,
    structural_lines: Vec<StructuralLine>,
    order_anomalies: Vec<OrderAnomaly>,
    summary_block: Option<SummaryBlock>,
    format: LogFormat,
    anchored_entry_count: usize,
    unindexable_line_count: usize,
}

impl ParsedLog {
    pub fn text(&self) -> &Arc<str> {
        &self.text
    }

    /// Every kept line, anchored and interpolated alike, in file order.
    pub fn entries(&self) -> &[LogEntry] {
        &self.entries
    }

    pub fn boot_sessions(&self) -> &[BootSession] {
        &self.boot_sessions
    }

    pub fn session_entries(&self, session: &BootSession) -> &[LogEntry] {
        self.entries
            .get(session.entry_range.clone())
            .unwrap_or_default()
    }

    /// The recognized non-entry lines, in file order.
    pub fn structural_lines(&self) -> &[StructuralLine] {
        &self.structural_lines
    }

    /// Backwards timestamp steps no logged clock adjustment explains, in file order.
    pub fn order_anomalies(&self) -> &[OrderAnomaly] {
        &self.order_anomalies
    }

    pub fn summary_block(&self) -> Option<&SummaryBlock> {
        self.summary_block.as_ref()
    }

    /// Set when the exporter counted entries and arrived at another number
    /// than this parse did.
    pub fn exporter_entry_count_mismatch(&self) -> Option<EntryCountMismatch> {
        let stated_by_exporter = self.summary_block.as_ref()?.entry_count?;
        let anchored_by_parse = u64::try_from(self.anchored_entry_count).unwrap_or(u64::MAX);
        (stated_by_exporter != anchored_by_parse).then_some(EntryCountMismatch {
            stated_by_exporter,
            anchored_by_parse,
        })
    }

    pub fn format(&self) -> LogFormat {
        self.format
    }

    pub fn anchored_entry_count(&self) -> usize {
        self.anchored_entry_count
    }

    pub fn interpolated_entry_count(&self) -> usize {
        self.entries.len().saturating_sub(self.anchored_entry_count)
    }

    /// Lines dropped because the index cannot address them: a line whose text
    /// exceeds [`u32::MAX`] bytes. Normally zero.
    pub fn unindexable_line_count(&self) -> usize {
        self.unindexable_line_count
    }

    pub fn message(&self, entry: &LogEntry) -> &str {
        entry.message.in_text(&self.text)
    }

    /// The earliest to the latest entry timestamp, interpolated ones included.
    /// `None` for a log without entries.
    ///
    /// A reboot steps the device clock, so the last entry in file order is not
    /// always the latest one.
    pub fn time_range(&self) -> Option<TimeRange> {
        let mut timestamps = self.entries.iter().map(|entry| entry.timestamp);
        let first = timestamps.next()?;
        let (start, end) = timestamps.fold((first, first), |(start, end), timestamp| {
            (start.min(timestamp), end.max(timestamp))
        });
        Some(TimeRange::new(start, end))
    }

    /// The timestamp of the first entry that carried one of its own.
    pub fn first_anchored_timestamp(&self) -> Option<DateTime<Utc>> {
        self.entries
            .iter()
            .find(|entry| entry.is_anchored())
            .map(|entry| entry.timestamp)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LogParseError {
    #[error("Log is empty")]
    Empty,

    #[error("No recognised timestamp format (first line: {first_line:?})")]
    NoRecognisedFormat { first_line: String },
}

/// Reads `text` into entries, taking the format from the head of the log and
/// resolving the year of the year-less syslog formats against `now`.
///
/// Every non-empty line is kept: one that carries no timestamp of that format
/// is either a structural line of a recognized exporter idiom or an entry
/// timestamped from its anchored neighbours. Only a line the index cannot
/// address is dropped.
pub fn parse_log(text: Arc<str>, now: DateTime<Utc>) -> Result<ParsedLog, LogParseError> {
    parse_log_in_chunks_of(text, now, CHUNK_TARGET_BYTES)
}

fn parse_log_in_chunks_of(
    text: Arc<str>,
    now: DateTime<Utc>,
    chunk_target_bytes: NonZeroUsize,
) -> Result<ParsedLog, LogParseError> {
    let format = detect_head_format(&text)?;
    let mut index = index_lines_in_file_order(&text, format, now, chunk_target_bytes);
    let summary_block = index.take_trailing_summary_block(&text);

    let mut anchored_entry_count = 0;
    let mut first_anchor = None;
    for entry in &index.entries {
        if entry.is_anchored() {
            anchored_entry_count += 1;
            first_anchor.get_or_insert(entry.timestamp);
        }
    }
    let Some(first_anchor) = first_anchor else {
        return Err(index.no_anchored_entry_error(&text));
    };

    let boot_sessions =
        session::segment_into_boot_sessions(&index.entries, &index.structural_lines);
    let order_anomalies =
        check_order_and_interpolate(&text, &mut index.entries, &boot_sessions, first_anchor);

    Ok(ParsedLog {
        text,
        entries: index.entries,
        boot_sessions,
        structural_lines: index.structural_lines,
        order_anomalies,
        summary_block,
        format,
        anchored_entry_count,
        unindexable_line_count: index.unindexable_line_count,
    })
}

/// Both per-session passes in one walk: the order check that records anomalies,
/// and the interpolation that timestamps the lines carrying none.
///
/// A session no line of which anchored takes the last anchor before it,
/// starting at `first_anchor`: interpolation never crosses a session boundary.
fn check_order_and_interpolate(
    text: &str,
    entries: &mut [LogEntry],
    boot_sessions: &[BootSession],
    first_anchor: DateTime<Utc>,
) -> Vec<OrderAnomaly> {
    let mut order_anomalies = Vec::new();
    let mut anchor_before_session = first_anchor;
    for boot_session in boot_sessions {
        let Some(session_entries) = entries.get_mut(boot_session.entry_range.clone()) else {
            continue;
        };
        session::scan_for_order_anomalies(text, session_entries, &mut order_anomalies);
        session::interpolate_timestamps(session_entries, anchor_before_session);
        if let Some(anchored) = boot_session.anchored {
            anchor_before_session = anchored.last;
        }
    }
    order_anomalies
}

fn detect_head_format(text: &str) -> Result<LogFormat, LogParseError> {
    let head: Vec<&str> = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .take(FORMAT_DETECTION_LINE_LIMIT)
        .collect();

    if let Some(format) = head.iter().copied().find_map(format::detect_format) {
        return Ok(format);
    }
    match head.first() {
        Some(first) => Err(LogParseError::NoRecognisedFormat {
            first_line: gt_fmt::truncate_with_ellipsis(first, ERROR_LINE_EXCERPT_CHARS)
                .into_owned(),
        }),
        None => Err(LogParseError::Empty),
    }
}

#[derive(Default)]
struct LineIndex {
    entries: Vec<LogEntry>,
    structural_lines: Vec<StructuralLine>,
    line_count: u32,
    unindexable_line_count: usize,
}

impl LineIndex {
    /// Joins per-chunk indices, given in the order their chunks appear in the
    /// log, numbering their lines from the head of the whole log.
    fn concatenated(chunks: &[Self]) -> Self {
        let mut joined = Self {
            entries: Vec::with_capacity(chunks.iter().map(|chunk| chunk.entries.len()).sum()),
            ..Self::default()
        };
        for chunk in chunks {
            let lines_before = joined.line_count;
            joined
                .entries
                .extend(chunk.entries.iter().map(|entry| LogEntry {
                    line_number: entry.line_number.saturating_add(lines_before),
                    ..*entry
                }));
            joined
                .structural_lines
                .extend(chunk.structural_lines.iter().map(|line| StructuralLine {
                    line_number: line.line_number.saturating_add(lines_before),
                    ..*line
                }));
            joined.line_count = lines_before.saturating_add(chunk.line_count);
            joined.unindexable_line_count += chunk.unindexable_line_count;
        }
        joined
    }

    /// The failure of a log whose one line matching a timestamp format turned
    /// out to sit inside a structural block.
    fn no_anchored_entry_error(&self, text: &str) -> LogParseError {
        match self.entries.first() {
            Some(entry) => LogParseError::NoRecognisedFormat {
                first_line: gt_fmt::truncate_with_ellipsis(
                    entry.message.in_text(text),
                    ERROR_LINE_EXCERPT_CHARS,
                )
                .into_owned(),
            },
            None => LogParseError::Empty,
        }
    }

    fn push_classified_line(
        &mut self,
        line: PositionedLine<'_>,
        format: LogFormat,
        now: DateTime<Utc>,
    ) {
        if let Some((timestamp, message)) = format::parse_line(line.trimmed, format, now) {
            self.push_anchored_entry(line, timestamp, message);
            return;
        }
        let Some(text) = TextSlice::new(line.offset_of_trimmed, line.trimmed.len()) else {
            self.unindexable_line_count += 1;
            return;
        };
        match StructuralLineKind::matching_line(line.trimmed) {
            Some(kind) => self.structural_lines.push(StructuralLine {
                kind,
                line_number: line.line_number,
                text,
            }),
            None => self.entries.push(LogEntry {
                // Replaced by the interpolation pass, which reaches every
                // entry this branch pushes.
                timestamp: DateTime::UNIX_EPOCH,
                timestamp_kind: TimestampKind::Interpolated,
                line_number: line.line_number,
                message: text,
            }),
        }
    }

    fn push_anchored_entry(
        &mut self,
        line: PositionedLine<'_>,
        timestamp: DateTime<Utc>,
        message: &str,
    ) {
        // Every format returns the message as a trailing slice of the line.
        let message_start = line.trimmed.len().checked_sub(message.len());
        debug_assert_eq!(
            message_start.and_then(|start| line.trimmed.get(start..)),
            Some(message),
            "the message is a trailing slice of the line it was read from"
        );
        let slice = message_start.and_then(|start| {
            TextSlice::new(
                line.offset_of_trimmed.saturating_add(start as u64),
                message.len(),
            )
        });
        match slice {
            Some(message) => self.entries.push(LogEntry {
                timestamp,
                timestamp_kind: TimestampKind::Anchored,
                line_number: line.line_number,
                message,
            }),
            None => self.unindexable_line_count += 1,
        }
    }

    /// Reclassifies the exporter's summary block - its header line and
    /// everything after it - as structural, whatever the chunk parse read those
    /// lines as, and reads what the block states.
    fn take_trailing_summary_block(&mut self, text: &str) -> Option<SummaryBlock> {
        let header = self
            .structural_lines
            .iter()
            .copied()
            .find(|line| line.kind.extent() == StructuralExtent::ToEndOfLog)?;

        self.entries.truncate(
            self.entries
                .partition_point(|entry| entry.line_number < header.line_number),
        );
        self.structural_lines.truncate(
            self.structural_lines
                .partition_point(|line| line.line_number < header.line_number),
        );

        let block_start = usize::try_from(header.text.offset).unwrap_or(usize::MAX);
        let block_text = text.get(block_start..).unwrap_or_default();
        let mut block_lines: Vec<&str> = Vec::new();
        for line in positioned_lines(block_text, header.text.offset, header.line_number) {
            if line.trimmed.is_empty() {
                continue;
            }
            match TextSlice::new(line.offset_of_trimmed, line.trimmed.len()) {
                Some(text) => {
                    self.structural_lines.push(StructuralLine {
                        kind: header.kind,
                        line_number: line.line_number,
                        text,
                    });
                    block_lines.push(line.trimmed);
                }
                None => self.unindexable_line_count += 1,
            }
        }

        Some(summary::parse_summary_block(block_lines))
    }
}

/// Indexes every line of `text`, spreading a log longer than
/// `chunk_target_bytes` over [`pool::log_worker_pool`].
///
/// The index comes out in file order and needs no sort to put it there: chunks
/// concatenate in the order they appear in the log.
fn index_lines_in_file_order(
    text: &str,
    format: LogFormat,
    now: DateTime<Utc>,
    chunk_target_bytes: NonZeroUsize,
) -> LineIndex {
    let chunks = newline_aligned_chunks(text, chunk_target_bytes);
    match chunks.as_slice() {
        [] => LineIndex::default(),
        [only] => only.parse(format, now),
        many => match pool::log_worker_pool() {
            Some(pool) => pool.install(|| {
                let per_chunk: Vec<LineIndex> = many
                    .par_iter()
                    .map(|chunk| chunk.parse(format, now))
                    .collect();
                LineIndex::concatenated(&per_chunk)
            }),
            None => LineIndex::concatenated(
                &many
                    .iter()
                    .map(|chunk| chunk.parse(format, now))
                    .collect::<Vec<_>>(),
            ),
        },
    }
}

/// One newline-aligned slice of a log's text, and where it starts in that text.
struct LogChunk<'text> {
    offset_in_text: u64,
    text: &'text str,
}

/// Splits `text` into slices of at least `chunk_target_bytes` that each end
/// after a newline, so no line spans two chunks.
fn newline_aligned_chunks(text: &str, chunk_target_bytes: NonZeroUsize) -> Vec<LogChunk<'_>> {
    let mut chunks = Vec::new();
    let mut start = 0;

    while start < text.len() {
        let unaligned_end = start
            .saturating_add(chunk_target_bytes.get())
            .min(text.len());
        let past_target = text.as_bytes().get(unaligned_end..).unwrap_or_default();
        let end = memchr::memchr(b'\n', past_target)
            .map_or(text.len(), |newline| unaligned_end + newline + 1);
        // Every bound is a character boundary: a newline is never part of a
        // multi-byte character.
        let chunk_text = text.get(start..end);
        debug_assert!(
            chunk_text.is_some(),
            "chunk {start}..{end} splits the log text mid-character"
        );
        chunks.push(LogChunk {
            offset_in_text: start as u64,
            text: chunk_text.unwrap_or_default(),
        });
        start = end;
    }

    chunks
}

impl LogChunk<'_> {
    /// Indexes every line of this chunk against the offsets of the whole log
    /// text, numbering lines from the head of the chunk.
    fn parse(&self, format: LogFormat, now: DateTime<Utc>) -> LineIndex {
        let mut index = LineIndex::default();
        for line in positioned_lines(self.text, self.offset_in_text, 1) {
            index.line_count = line.line_number;
            if line.trimmed.is_empty() {
                continue;
            }
            index.push_classified_line(line, format, now);
        }
        index
    }
}

/// One physical line of a log, trimmed of its indent and line ending.
#[derive(Debug, Clone, Copy)]
struct PositionedLine<'text> {
    line_number: u32,
    trimmed: &'text str,
    offset_of_trimmed: u64,
}

/// Walks every physical line of `text`, empty ones included, numbering from
/// `first_line_number` and offsetting from `first_offset`.
fn positioned_lines(
    text: &str,
    first_offset: u64,
    first_line_number: u32,
) -> impl Iterator<Item = PositionedLine<'_>> {
    let mut offset = first_offset;
    let mut line_number = first_line_number;
    text.split_inclusive('\n').map(move |line| {
        let offset_of_line = offset;
        offset = offset.saturating_add(line.len() as u64);
        let this_line_number = line_number;
        line_number = line_number.saturating_add(1);
        let indent = line.len() - line.trim_start().len();
        PositionedLine {
            line_number: this_line_number,
            trimmed: line.trim(),
            offset_of_trimmed: offset_of_line.saturating_add(indent as u64),
        }
    })
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone as _};
    use gt_test_utils::log_fixtures::{self, SyntheticLogSpec};
    use proptest::{prelude::*, prop_oneof, proptest};
    use rstest::rstest;

    use super::*;
    use crate::summary::ServiceCount;

    const REBOOT: &str = "--- Device reboot ---\n";

    fn utc(y: i32, mo: u32, d: u32, h: u32, m: u32, s: u32) -> DateTime<Utc> {
        Utc.with_ymd_and_hms(y, mo, d, h, m, s)
            .single()
            .expect("valid")
    }

    fn now() -> DateTime<Utc> {
        utc(2026, 5, 23, 0, 0, 0)
    }

    fn parse(text: &str) -> ParsedLog {
        parse_log(Arc::from(text), now()).expect("parses")
    }

    fn messages(parsed: &ParsedLog) -> Vec<&str> {
        parsed
            .entries()
            .iter()
            .map(|entry| parsed.message(entry))
            .collect()
    }

    fn timestamps(parsed: &ParsedLog) -> Vec<DateTime<Utc>> {
        parsed
            .entries()
            .iter()
            .map(|entry| entry.timestamp)
            .collect()
    }

    fn timestamp_kinds(parsed: &ParsedLog) -> Vec<TimestampKind> {
        parsed
            .entries()
            .iter()
            .map(|entry| entry.timestamp_kind)
            .collect()
    }

    fn chunk_bytes(bytes: usize) -> NonZeroUsize {
        NonZeroUsize::new(bytes).expect("positive chunk size")
    }

    /// The clock steps back over the reboot, so the log's span ends at an
    /// entry the file wrote before its last one.
    #[test]
    fn the_time_range_spans_the_earliest_and_latest_entry_across_a_reboot() {
        let parsed = parse(&format!(
            "2026-05-23 10:00:00 first\n2026-05-23 12:00:00 last\n{REBOOT}2026-05-23 11:00:00 after the reboot\n"
        ));
        assert_eq!(
            parsed.time_range(),
            Some(TimeRange::new(
                utc(2026, 5, 23, 10, 0, 0),
                utc(2026, 5, 23, 12, 0, 0)
            ))
        );
    }

    /// A run of untimestamped lines opening a log is timestamped from the
    /// first anchor after it, which is the anchor a pasted log is named after.
    #[test]
    fn the_first_anchored_timestamp_skips_the_interpolated_entries_before_it() {
        let parsed = parse("2026-05-23 10:00:00 first\nStack trace follows:\n");
        assert_eq!(
            parsed.first_anchored_timestamp(),
            Some(utc(2026, 5, 23, 10, 0, 0))
        );

        let leading_run = parse("Stack trace follows:\n2026-05-23 10:00:00 first\n");
        assert_eq!(
            leading_run.first_anchored_timestamp(),
            Some(utc(2026, 5, 23, 10, 0, 0)),
            "the interpolated entry before the anchor carries its timestamp, but does not anchor it"
        );
    }

    #[test]
    fn a_log_without_any_recognised_timestamp_names_its_first_line() {
        let error =
            parse_log(Arc::from("nothing here\nnor here\n"), now()).expect_err("fails to parse");
        assert_eq!(
            error.to_string(),
            "No recognised timestamp format (first line: \"nothing here\")"
        );
    }

    #[test]
    fn a_very_long_first_line_is_quoted_up_to_an_excerpt() {
        let text = "x".repeat(ERROR_LINE_EXCERPT_CHARS.get() + 50);
        let error = parse_log(Arc::from(text.as_str()), now()).expect_err("fails to parse");
        assert_eq!(
            error,
            LogParseError::NoRecognisedFormat {
                first_line: format!(
                    "{}{}",
                    "x".repeat(ERROR_LINE_EXCERPT_CHARS.get()),
                    gt_fmt::ELLIPSIS
                ),
            }
        );
    }

    #[rstest]
    #[case::no_bytes("")]
    #[case::only_blank_lines("\n\n   \n")]
    fn a_log_without_any_line_is_empty(#[case] text: &str) {
        assert_eq!(parse_log(Arc::from(text), now()), Err(LogParseError::Empty));
    }

    /// The three kinds a non-empty line can be read as, in one log.
    #[test]
    fn a_line_is_anchored_structural_or_interpolated() {
        let parsed = parse("2026-01-01 00:00:00 anchored\n--- Device reboot ---\nno timestamp\n");

        assert_eq!(messages(&parsed), ["anchored", "no timestamp"]);
        assert_eq!(
            timestamp_kinds(&parsed),
            [TimestampKind::Anchored, TimestampKind::Interpolated]
        );
        assert_eq!(parsed.anchored_entry_count(), 1);
        assert_eq!(parsed.interpolated_entry_count(), 1);
        assert_eq!(
            parsed
                .structural_lines()
                .iter()
                .map(|line| (line.kind, line.line_number))
                .collect::<Vec<_>>(),
            [(StructuralLineKind::RebootSeparator, 2)]
        );
        assert_eq!(parsed.unindexable_line_count(), 0);
    }

    #[test]
    fn blank_lines_are_neither_entries_nor_structure() {
        let parsed = parse("\n\n2026-01-01 00:00:00 only\n\n   \n");
        assert_eq!(messages(&parsed), ["only"]);
        assert!(parsed.structural_lines().is_empty());
        assert_eq!(parsed.unindexable_line_count(), 0);
    }

    /// A separator idiom no registry pattern knows is not an error: it loads as
    /// an entry like any other untimestamped line.
    #[test]
    fn an_unknown_separator_is_read_as_an_entry() {
        let parsed = parse("2026-01-01 00:00:00 a\n=== Power cycle ===\n");
        assert_eq!(messages(&parsed), ["a", "=== Power cycle ==="]);
        assert!(parsed.structural_lines().is_empty());
        assert_eq!(parsed.boot_sessions().len(), 1);
    }

    /// A log of one format keeps the lines written in another: they are entries
    /// timestamped from their neighbours, not a second format.
    #[test]
    fn only_the_detected_format_is_parsed() {
        let parsed = parse("2026-01-01 00:00:00 iso\nMay 29 18:48:24 syslog\n");
        assert_eq!(parsed.format(), LogFormat::Iso8601Space);
        assert_eq!(messages(&parsed), ["iso", "May 29 18:48:24 syslog"]);
        assert_eq!(parsed.interpolated_entry_count(), 1);
    }

    /// A banner longer than the detector's sample hides the format that follows.
    #[test]
    fn the_format_is_detected_within_the_head_of_the_log() {
        let mut within_head = "banner\n".repeat(FORMAT_DETECTION_LINE_LIMIT - 1);
        within_head.push_str("2026-01-01 00:00:00 body\n");
        let parsed = parse(&within_head);
        assert_eq!(parsed.anchored_entry_count(), 1);
        assert_eq!(
            parsed.interpolated_entry_count(),
            FORMAT_DETECTION_LINE_LIMIT - 1
        );

        let past_head = "banner\n".repeat(FORMAT_DETECTION_LINE_LIMIT) + "2026-01-01 00:00:00 body";
        assert_eq!(
            parse_log(Arc::from(past_head.as_str()), now()),
            Err(LogParseError::NoRecognisedFormat {
                first_line: "banner".to_owned(),
            })
        );
    }

    /// A file whose lines a later timestamp reordered stays in the order it was
    /// written in.
    #[test]
    fn entries_stay_in_the_order_the_file_wrote_them() {
        let parsed = parse(
            "2026-01-01 00:00:05 late\n2026-01-01 00:00:00 alpha\n\
             2026-01-01 00:00:00 beta\n2026-01-01 00:00:02 middle\n",
        );
        assert_eq!(messages(&parsed), ["late", "alpha", "beta", "middle"]);
        assert_eq!(
            parsed
                .entries()
                .iter()
                .map(|entry| entry.line_number)
                .collect::<Vec<_>>(),
            [1, 2, 3, 4]
        );
    }

    /// A message is a slice of the log text: indented and CRLF lines shift where
    /// it starts and ends within its line.
    #[test]
    fn an_entry_indexes_its_message_within_the_log_text() {
        let text = "2026-01-01 00:00:00 alpha\r\n   2026-01-01 00:00:01 beta gamma\n";
        let parsed = parse(text);
        assert_eq!(parsed.text().as_ref(), text);
        assert_eq!(messages(&parsed), ["alpha", "beta gamma"]);

        let entry = parsed.entries().get(1).copied().expect("two entries");
        assert_eq!(entry.message.in_text(text), "beta gamma");
    }

    #[rstest]
    #[case::syslog_short("May 20 18:48:24 msg", utc(2026, 5, 20, 18, 48, 24), 0)]
    #[case::syslog_short_micro("May 20 18:48:24.500000 msg", utc(2026, 5, 20, 18, 48, 24), 500_000)]
    #[case::iso_8601_space("2026-05-20 18:48:24 msg", utc(2026, 5, 20, 18, 48, 24), 0)]
    #[case::iso_8601_t("2026-05-20T18:48:24Z msg", utc(2026, 5, 20, 18, 48, 24), 0)]
    fn every_format_yields_its_timestamp_and_message(
        #[case] line: &str,
        #[case] expected_second: DateTime<Utc>,
        #[case] expected_micros: i64,
    ) {
        let parsed = parse(line);
        let entry = parsed.entries().first().copied().expect("one entry");
        assert_eq!(
            entry.timestamp,
            expected_second + Duration::microseconds(expected_micros)
        );
        assert_eq!(parsed.message(&entry), "msg");
    }

    #[test]
    fn a_year_less_format_resolves_against_now() {
        let parsed = parse_log(
            Arc::from("Dec 31 23:59:59 rollover\n"),
            utc(2026, 1, 1, 0, 0, 0),
        )
        .expect("parses");
        assert_eq!(
            parsed.entries().first().map(|entry| entry.timestamp),
            Some(utc(2025, 12, 31, 23, 59, 59))
        );
    }

    #[test]
    fn a_run_of_untimestamped_lines_is_spread_between_its_anchors() {
        let parsed = parse(
            "2026-01-01 00:00:00 first\nStack trace follows:\n  at 0x0\n  ... omitted ...\n\
             2026-01-01 00:00:04 last\n",
        );
        assert_eq!(
            timestamps(&parsed),
            [
                utc(2026, 1, 1, 0, 0, 0),
                utc(2026, 1, 1, 0, 0, 1),
                utc(2026, 1, 1, 0, 0, 2),
                utc(2026, 1, 1, 0, 0, 3),
                utc(2026, 1, 1, 0, 0, 4),
            ]
        );
    }

    /// One line between anchors a second apart lands half a second in.
    #[test]
    fn a_run_shorter_than_the_span_it_covers_keeps_sub_second_places() {
        let parsed = parse("2026-01-01 00:00:00 a\nmid\n2026-01-01 00:00:01 b\n");
        assert_eq!(
            timestamps(&parsed).get(1),
            Some(&(utc(2026, 1, 1, 0, 0, 0) + Duration::milliseconds(500)))
        );
    }

    #[test]
    fn a_run_at_the_edge_of_a_session_takes_the_one_anchor_it_has() {
        let parsed = parse(
            "before any anchor\n2026-01-01 00:00:10 first\n2026-01-01 00:00:20 last\n\
             after the last anchor\n",
        );
        assert_eq!(
            timestamps(&parsed),
            [
                utc(2026, 1, 1, 0, 0, 10),
                utc(2026, 1, 1, 0, 0, 10),
                utc(2026, 1, 1, 0, 0, 20),
                utc(2026, 1, 1, 0, 0, 20),
            ]
        );
    }

    /// No run is ever spread across a reboot: the device clock restarts there.
    #[test]
    fn interpolation_never_crosses_a_boot_boundary() {
        let parsed = parse(&format!(
            "2026-01-01 00:00:00 before\nlast line of boot 1\n{REBOOT}\
             first line of boot 2\n2026-01-01 00:10:00 after\n"
        ));
        assert_eq!(
            timestamps(&parsed),
            [
                utc(2026, 1, 1, 0, 0, 0),
                utc(2026, 1, 1, 0, 0, 0),
                utc(2026, 1, 1, 0, 10, 0),
                utc(2026, 1, 1, 0, 10, 0),
            ]
        );
    }

    #[test]
    fn a_session_no_line_of_which_anchored_takes_the_anchor_before_it() {
        let parsed = parse(&format!(
            "2026-01-01 00:00:00 boot one\n{REBOOT}nothing timestamped here\n{REBOOT}\
             2026-01-01 00:10:00 boot three\n"
        ));
        assert_eq!(
            timestamps(&parsed),
            [
                utc(2026, 1, 1, 0, 0, 0),
                utc(2026, 1, 1, 0, 0, 0),
                utc(2026, 1, 1, 0, 10, 0),
            ]
        );
        assert_eq!(
            parsed
                .boot_sessions()
                .iter()
                .map(|session| session.anchored.is_some())
                .collect::<Vec<_>>(),
            [true, false, true]
        );
    }

    #[rstest]
    #[case::no_separator("2026-01-01 00:00:00 a\n2026-01-01 00:00:01 b\n", &[2])]
    #[case::one_separator("2026-01-01 00:00:00 a\n--- Device reboot ---\n2026-01-01 00:00:01 b\n", &[1, 1])]
    #[case::three_separators(
        "2026-01-01 00:00:00 a\n--- Device reboot ---\n2026-01-01 00:00:01 b\n\
         --- Device reboot ---\n2026-01-01 00:00:02 c\n--- Device reboot ---\n\
         2026-01-01 00:00:03 d\n",
        &[1, 1, 1, 1]
    )]
    #[case::separator_before_the_first_entry("--- Device reboot ---\n2026-01-01 00:00:00 a\n", &[1])]
    #[case::two_separators_in_a_row(
        "2026-01-01 00:00:00 a\n--- Device reboot ---\n--- Device reboot ---\n2026-01-01 00:00:01 b\n",
        &[1, 1]
    )]
    fn reboot_separators_cut_the_log_into_boot_sessions(
        #[case] text: &str,
        #[case] expected_entry_counts: &[usize],
    ) {
        let parsed = parse(text);
        assert_eq!(
            parsed
                .boot_sessions()
                .iter()
                .map(|session| (session.boot_number, session.entry_count()))
                .collect::<Vec<_>>(),
            expected_entry_counts
                .iter()
                .enumerate()
                .map(|(index, count)| (index as u32 + 1, *count))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn a_boot_session_spans_its_own_anchors() {
        let parsed = parse(&format!(
            "2026-01-01 00:00:00 a\n2026-01-01 02:30:00 b\n{REBOOT}2026-01-01 00:05:00 c\n"
        ));
        let uptimes: Vec<Option<Duration>> = parsed
            .boot_sessions()
            .iter()
            .map(|session| session.anchored.map(|anchored| anchored.uptime()))
            .collect();
        assert_eq!(
            uptimes,
            [Some(Duration::minutes(150)), Some(Duration::zero())]
        );
    }

    #[rstest]
    #[case::unexplained("2026-01-01 00:00:10 a\n2026-01-01 00:00:05 b\n", 1)]
    #[case::explained_by_the_stepping_line_itself(
        "2026-01-01 00:00:10 a\n2026-01-01 00:00:05 systemd-timedated: Time has been changed\n",
        0
    )]
    #[case::explained_by_a_later_line(
        "2026-01-01 00:00:10 a\n2026-01-01 00:00:05 b\n2026-01-01 00:00:05 systemd-journald: Time jumped backwards, rotating.\n",
        0
    )]
    #[case::explanation_too_far_away(
        "2026-01-01 00:00:10 a\n2026-01-01 00:00:05 b\n2026-01-01 00:00:06 c\n\
         2026-01-01 00:00:07 d\n2026-01-01 00:00:08 e\n\
         2026-01-01 00:00:09 systemd-journald: Time jumped backwards, rotating.\n",
        1
    )]
    #[case::across_a_reboot(
        "2026-01-01 00:00:10 a\n--- Device reboot ---\n2026-01-01 00:00:05 b\n",
        0
    )]
    fn a_backwards_step_is_an_anomaly_only_when_nothing_nearby_reports_a_clock_change(
        #[case] text: &str,
        #[case] expected_anomalies: usize,
    ) {
        assert_eq!(parse(text).order_anomalies().len(), expected_anomalies);
    }

    #[test]
    fn an_order_anomaly_names_the_line_it_steps_back_on_and_how_far() {
        let parsed = parse("2026-01-01 03:12:00 a\nfiller\n2026-01-01 00:00:00 b\n");
        assert_eq!(
            parsed.order_anomalies(),
            [OrderAnomaly {
                line_number: 3,
                timestamp_step: -Duration::minutes(192),
            }]
        );
    }

    #[test]
    fn the_summary_block_ends_the_entries_and_is_read_as_structure() {
        let parsed = parse(
            "2026-01-01 00:00:00 a\n2026-01-01 00:00:01 b\n\
             ----------- Journal summary -----------\nDevice type: nav-devkit-mk2\n\
             Log entries: 2\n--- Service error count ---\nhal-powerd   -> 7 Errors\n\
             2026-01-01 00:00:02 not an entry any more\n",
        );

        assert_eq!(messages(&parsed), ["a", "b"]);
        assert_eq!(parsed.structural_lines().len(), 6);
        assert!(
            parsed
                .structural_lines()
                .iter()
                .all(|line| line.kind == StructuralLineKind::SummaryBlock)
        );
        let summary = parsed.summary_block().expect("the block is recognized");
        assert_eq!(summary.device_type.as_deref(), Some("nav-devkit-mk2"));
        assert_eq!(
            summary.service_error_counts,
            [ServiceCount {
                service: "hal-powerd".to_owned(),
                count: 7,
            }]
        );
        assert_eq!(parsed.exporter_entry_count_mismatch(), None);
    }

    #[test]
    fn an_exporter_count_the_parse_disagrees_with_is_reported() {
        let parsed = parse(
            "2026-01-01 00:00:00 a\n----------- Journal summary -----------\nLog entries: 9\n",
        );
        assert_eq!(
            parsed.exporter_entry_count_mismatch(),
            Some(EntryCountMismatch {
                stated_by_exporter: 9,
                anchored_by_parse: 1,
            })
        );
    }

    /// A summary block swallowing the one line that anchored leaves nothing to
    /// interpolate from.
    #[test]
    fn a_log_whose_only_anchor_the_summary_block_swallowed_fails_to_load() {
        let error = parse_log(
            Arc::from(
                "kernel output\n----------- Journal summary -----------\n2026-01-01 00:00:00 a\n",
            ),
            now(),
        )
        .expect_err("fails to parse");
        assert_eq!(
            error,
            LogParseError::NoRecognisedFormat {
                first_line: "kernel output".to_owned(),
            }
        );
    }

    /// Covers the structure passes across chunk bounds: thousands of entries,
    /// reboots, untimestamped runs and a summary block, enough for the chunked
    /// path to split all of them.
    #[test]
    fn a_chunked_parse_of_a_journald_sized_log_matches_the_one_chunk_parse() {
        let text = log_fixtures::synthetic_journald_log(SyntheticLogSpec {
            approx_bytes: 200 * 1024,
            seed: 7,
        });
        let chunk_target_bytes = chunk_bytes(4 * 1024);
        assert!(
            newline_aligned_chunks(&text, chunk_target_bytes).len() > 8,
            "the fixture has to span several chunks for this to compare the two paths"
        );

        let one_chunk = parse_log_in_chunks_of(
            Arc::from(text.as_str()),
            now(),
            chunk_bytes(text.len().saturating_add(1)),
        );
        let chunked = parse_log_in_chunks_of(Arc::from(text.as_str()), now(), chunk_target_bytes);
        assert_eq!(one_chunk, chunked);

        assert!(
            text.contains("Time jumped backwards"),
            "the order scan has a step to explain: the fixture corrects its clock mid-session"
        );

        let parsed = chunked.expect("the fixture parses");
        assert!(parsed.entries().len() > 1_000, "the fixture has entries");
        assert!(
            parsed.interpolated_entry_count() > 0,
            "the fixture has lines without a timestamp"
        );
        assert!(
            parsed.boot_sessions().len() > 1,
            "the fixture reboots at least once"
        );
        assert!(parsed.summary_block().is_some(), "the fixture is exported");
        assert_eq!(parsed.exporter_entry_count_mismatch(), None);
        assert_eq!(
            parsed.order_anomalies(),
            [],
            "every backwards step in the fixture is a logged clock change"
        );
    }

    fn any_line() -> impl Strategy<Value = String> {
        prop_oneof![
            6 => r"\PC*",
            6 => (0u32..24, 0u32..60, r"\PC*").prop_map(|(hour, minute, rest)| format!(
                "2026-01-01 {hour:02}:{minute:02}:00 {rest}"
            )),
            6 => (1u32..29, r"\PC*").prop_map(|(day, rest)| format!("Jan {day:2} 00:00:00 {rest}")),
            1 => Just("--- Device reboot ---".to_owned()),
            1 => Just("----------- Journal summary -----------".to_owned()),
        ]
    }

    proptest! {
        /// Whatever text a user drops on the app, every entry the parser emits
        /// slices its own message back out of the log text.
        #[test]
        fn entries_of_any_text_index_it_consistently(
            lines in prop::collection::vec(any_line(), 0..20),
        ) {
            let text = lines.join("\n");
            let Ok(parsed) = parse_log(Arc::from(text.as_str()), now()) else {
                return Ok(());
            };

            prop_assert_eq!(parsed.text().as_ref(), text.as_str());
            prop_assert!(parsed.entries().is_sorted_by_key(|entry| entry.line_number));

            for entry in parsed.entries() {
                let message = parsed.message(entry);
                prop_assert_eq!(entry.message.in_text(&text), message);
                prop_assert_eq!(message, message.trim());
            }
        }

        /// Every non-empty line ends up in exactly one of the parse's counts,
        /// and every entry in exactly one boot session.
        #[test]
        fn every_non_empty_line_is_counted_exactly_once(
            lines in prop::collection::vec(any_line(), 0..20),
        ) {
            let text = lines.join("\n");
            let Ok(parsed) = parse_log(Arc::from(text.as_str()), now()) else {
                return Ok(());
            };

            let non_empty_lines = text.lines().filter(|line| !line.trim().is_empty()).count();
            prop_assert_eq!(
                parsed.entries().len()
                    + parsed.structural_lines().len()
                    + parsed.unindexable_line_count(),
                non_empty_lines
            );
            prop_assert_eq!(
                parsed.anchored_entry_count() + parsed.interpolated_entry_count(),
                parsed.entries().len()
            );

            let mut next_entry = 0;
            for session in parsed.boot_sessions() {
                prop_assert_eq!(session.entry_range.start, next_entry);
                prop_assert!(session.entry_count() > 0);
                next_entry = session.entry_range.end;
            }
            prop_assert_eq!(next_entry, parsed.entries().len());
        }

        /// However badly a log is formed and wherever the chunk bounds fall
        /// in it, the chunked path returns what the one-chunk path returns.
        #[test]
        fn a_chunked_parse_of_any_text_matches_the_one_chunk_parse(
            lines in prop::collection::vec(any_line(), 0..40),
            chunk_target_bytes in 1usize..64,
        ) {
            let text = lines.join("\n");
            let one_chunk = parse_log_in_chunks_of(
                Arc::from(text.as_str()),
                now(),
                chunk_bytes(text.len().saturating_add(1)),
            );
            let chunked = parse_log_in_chunks_of(
                Arc::from(text.as_str()),
                now(),
                chunk_bytes(chunk_target_bytes),
            );
            prop_assert_eq!(one_chunk, chunked);
        }

        #[test]
        fn chunks_tile_the_log_text_and_break_only_after_a_newline(
            text in r"(\PC|\n){0,300}",
            chunk_target_bytes in 1usize..64,
        ) {
            let chunks = newline_aligned_chunks(&text, chunk_bytes(chunk_target_bytes));

            let mut expected_offset = 0;
            for (i, chunk) in chunks.iter().enumerate() {
                prop_assert_eq!(chunk.offset_in_text, expected_offset);
                prop_assert!(!chunk.text.is_empty());
                let last_chunk = i + 1 == chunks.len();
                prop_assert!(
                    last_chunk || chunk.text.ends_with('\n'),
                    "chunk {} of {} ends mid-line: {:?}", i, chunks.len(), chunk.text
                );
                expected_offset += chunk.text.len() as u64;
            }
            prop_assert_eq!(expected_offset, text.len() as u64);

            let rejoined: String = chunks.iter().map(|chunk| chunk.text).collect();
            prop_assert_eq!(rejoined, text);
        }
    }

    #[test]
    fn a_line_without_a_message_yields_an_empty_one() {
        let parsed = parse("2026-01-01 00:00:00\n");
        let entry = parsed.entries().first().copied().expect("one entry");
        assert_eq!(entry.message.len, 0);
        assert_eq!(parsed.message(&entry), "");
    }
}
