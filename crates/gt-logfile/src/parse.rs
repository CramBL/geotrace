//! Detecting a log's format from its head and indexing every line against its text.

use std::{
    num::NonZeroUsize,
    sync::{Arc, OnceLock},
    thread,
};

use chrono::{DateTime, Utc};
use rayon::{ThreadPool, ThreadPoolBuilder, prelude::*};

use crate::format::{self, LogFormat};

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

/// The share of the machine's cores the parse pool may occupy. Indexing a log
/// is a background job behind a desktop the user is still working in, and a log
/// is large enough to saturate every core for long enough to be felt: measured
/// on an 80 MiB journal, half the cores index it as fast as all of them.
const CORES_PER_PARSE_WORKER: usize = 2;

/// One parsed line: its timestamp and the byte range of its message inside the
/// text of the [`ParsedLog`] it was indexed from, read with [`ParsedLog::message`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LogEntry {
    pub timestamp: DateTime<Utc>,
    pub message_offset: u64,
    pub message_len: u32,
}

/// A log read into the text it was parsed from and an index over its lines.
#[derive(Debug, Clone, PartialEq)]
pub struct ParsedLog {
    text: Arc<str>,
    entries: Vec<LogEntry>,
    format: LogFormat,
    skipped_line_count: usize,
}

impl ParsedLog {
    pub fn text(&self) -> &Arc<str> {
        &self.text
    }

    /// Entries in ascending timestamp order, ties broken by original line order.
    pub fn entries(&self) -> &[LogEntry] {
        &self.entries
    }

    pub fn format(&self) -> LogFormat {
        self.format
    }

    /// Non-empty lines that did not carry a timestamp of [`ParsedLog::format`].
    pub fn skipped_line_count(&self) -> usize {
        self.skipped_line_count
    }

    pub fn message(&self, entry: &LogEntry) -> &str {
        let start = usize::try_from(entry.message_offset).unwrap_or(usize::MAX);
        let end = start.saturating_add(entry.message_len as usize);
        let message = self.text.get(start..end);
        debug_assert!(
            message.is_some(),
            "entry {entry:?} addresses text outside its own log"
        );
        message.unwrap_or_default()
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
/// Lines that carry no timestamp of that format are skipped and counted: a
/// stack trace in the middle of a journal costs those lines and nothing else.
/// A line whose message exceeds [`u32::MAX`] bytes is skipped the same way.
pub fn parse_log(text: Arc<str>, now: DateTime<Utc>) -> Result<ParsedLog, LogParseError> {
    parse_log_in_chunks_of(text, now, CHUNK_TARGET_BYTES)
}

fn parse_log_in_chunks_of(
    text: Arc<str>,
    now: DateTime<Utc>,
    chunk_target_bytes: NonZeroUsize,
) -> Result<ParsedLog, LogParseError> {
    let format = detect_head_format(&text)?;
    let LineIndex {
        entries,
        skipped_line_count,
    } = index_lines_in_timestamp_order(&text, format, now, chunk_target_bytes);

    Ok(ParsedLog {
        text,
        entries,
        format,
        skipped_line_count,
    })
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
    skipped_line_count: usize,
}

impl LineIndex {
    /// Joins per-chunk indices, given in the order their chunks appear in the log.
    fn concatenated(chunks: &[Self]) -> Self {
        let mut entries = Vec::with_capacity(chunks.iter().map(|chunk| chunk.entries.len()).sum());
        let mut skipped_line_count = 0;
        for chunk in chunks {
            entries.extend_from_slice(&chunk.entries);
            skipped_line_count += chunk.skipped_line_count;
        }
        Self {
            entries,
            skipped_line_count,
        }
    }
}

/// The pool a log longer than [`CHUNK_TARGET_BYTES`] is indexed on, holding
/// one worker per [`CORES_PER_PARSE_WORKER`] cores.
///
/// A pool dedicated to log parsing. gt-plot renders frames on rayon's global
/// pool, and a parse sharing it would stall frame rendering.
fn parse_pool() -> Option<&'static ThreadPool> {
    static PARSE_POOL: OnceLock<Option<ThreadPool>> = OnceLock::new();

    PARSE_POOL
        .get_or_init(|| {
            let cores = thread::available_parallelism().map_or(1, NonZeroUsize::get);
            let workers = (cores / CORES_PER_PARSE_WORKER).max(1);
            match ThreadPoolBuilder::new()
                .num_threads(workers)
                .thread_name(|index| format!("gt-logfile-parse-{index}"))
                .build()
            {
                Ok(pool) => Some(pool),
                Err(err) => {
                    log::warn!("Indexing logs on the calling thread only: {err:#}");
                    None
                }
            }
        })
        .as_ref()
}

/// Indexes every line of `text`, spreading a log longer than
/// `chunk_target_bytes` over [`parse_pool`].
///
/// Entries sharing a timestamp come out in line order: chunks concatenate in
/// the order they appear in the log, and both sorts below are stable.
fn index_lines_in_timestamp_order(
    text: &str,
    format: LogFormat,
    now: DateTime<Utc>,
    chunk_target_bytes: NonZeroUsize,
) -> LineIndex {
    let chunks = newline_aligned_chunks(text, chunk_target_bytes);
    let mut index = match chunks.as_slice() {
        [] => LineIndex::default(),
        [only] => only.parse(format, now),
        many => match parse_pool() {
            Some(pool) => {
                return pool.install(|| {
                    let per_chunk: Vec<LineIndex> = many
                        .par_iter()
                        .map(|chunk| chunk.parse(format, now))
                        .collect();
                    let mut index = LineIndex::concatenated(&per_chunk);
                    index.entries.par_sort_by_key(|entry| entry.timestamp);
                    index
                });
            }
            None => LineIndex::concatenated(
                &many
                    .iter()
                    .map(|chunk| chunk.parse(format, now))
                    .collect::<Vec<_>>(),
            ),
        },
    };
    index.entries.sort_by_key(|entry| entry.timestamp);
    index
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
    /// Indexes every line of this chunk against the offsets of the whole log text.
    fn parse(&self, format: LogFormat, now: DateTime<Utc>) -> LineIndex {
        let mut entries = Vec::new();
        let mut skipped_line_count = 0;

        let mut line_offset: u64 = 0;

        for line in self.text.split_inclusive('\n') {
            let offset_of_line = line_offset;
            line_offset += line.len() as u64;

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            let Some((timestamp, message)) = format::parse_line(trimmed, format, now) else {
                skipped_line_count += 1;
                continue;
            };
            // Every format returns the message as a trailing slice of the line.
            let message_start = trimmed.len().checked_sub(message.len());
            debug_assert_eq!(
                message_start.and_then(|start| trimmed.get(start..)),
                Some(message),
                "the message is a trailing slice of the line it was read from"
            );
            let (Some(message_start), Ok(message_len)) =
                (message_start, u32::try_from(message.len()))
            else {
                skipped_line_count += 1;
                continue;
            };
            let indent = line.len() - line.trim_start().len();
            entries.push(LogEntry {
                timestamp,
                message_offset: self.offset_in_text
                    + offset_of_line
                    + (indent + message_start) as u64,
                message_len,
            });
        }

        LineIndex {
            entries,
            skipped_line_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;
    use gt_test_utils::log_fixtures::{self, SyntheticLogSpec};
    use proptest::{prelude::*, prop_oneof, proptest};
    use rstest::rstest;

    use super::*;

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

    fn chunk_bytes(bytes: usize) -> NonZeroUsize {
        NonZeroUsize::new(bytes).expect("positive chunk size")
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

    /// Unparsable lines anywhere in the log cost only themselves.
    #[test]
    fn unparsable_lines_are_skipped_and_counted() {
        let parsed =
            parse("2026-01-01 00:00:00 first\nBAD1\nBAD2\nBAD3\n2026-01-01 00:00:01 last\n");
        assert_eq!(messages(&parsed), ["first", "last"]);
        assert_eq!(parsed.skipped_line_count(), 3);
    }

    #[test]
    fn blank_lines_are_neither_entries_nor_skips() {
        let parsed = parse("\n\n2026-01-01 00:00:00 only\n\n   \n");
        assert_eq!(messages(&parsed), ["only"]);
        assert_eq!(parsed.skipped_line_count(), 0);
    }

    /// A log of one format ignores lines written in another: they are the
    /// skipped lines, not a second format.
    #[test]
    fn only_the_detected_format_is_parsed() {
        let parsed = parse("2026-01-01 00:00:00 iso\nMay 29 18:48:24 syslog\n");
        assert_eq!(parsed.format(), LogFormat::Iso8601Space);
        assert_eq!(messages(&parsed), ["iso"]);
        assert_eq!(parsed.skipped_line_count(), 1);
    }

    /// A banner longer than the detector's sample hides the format that follows.
    #[test]
    fn the_format_is_detected_within_the_head_of_the_log() {
        let mut within_head = "banner\n".repeat(FORMAT_DETECTION_LINE_LIMIT - 1);
        within_head.push_str("2026-01-01 00:00:00 body\n");
        let parsed = parse(&within_head);
        assert_eq!(messages(&parsed), ["body"]);
        assert_eq!(parsed.skipped_line_count(), FORMAT_DETECTION_LINE_LIMIT - 1);

        let past_head = "banner\n".repeat(FORMAT_DETECTION_LINE_LIMIT) + "2026-01-01 00:00:00 body";
        assert_eq!(
            parse_log(Arc::from(past_head.as_str()), now()),
            Err(LogParseError::NoRecognisedFormat {
                first_line: "banner".to_owned(),
            })
        );
    }

    #[test]
    fn entries_are_sorted_by_timestamp_keeping_line_order_within_a_second() {
        let parsed = parse(
            "2026-01-01 00:00:05 late\n2026-01-01 00:00:00 alpha\n\
             2026-01-01 00:00:00 beta\n2026-01-01 00:00:02 middle\n",
        );
        assert_eq!(messages(&parsed), ["alpha", "beta", "middle", "late"]);
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
        let start = entry.message_offset as usize;
        let end = start + entry.message_len as usize;
        assert_eq!(text.get(start..end), Some("beta gamma"));
    }

    #[rstest]
    #[case::syslog_short("May 20 18:48:24 msg", utc(2026, 5, 20, 18, 48, 24), 0)]
    #[case::syslog_short_micro("May 20 18:48:24.500000 msg", utc(2026, 5, 20, 18, 48, 24), 500_000)]
    #[case::iso_8601_space("2026-05-20 18:48:24 msg", utc(2026, 5, 20, 18, 48, 24), 0)]
    #[case::iso_8601_t("2026-05-20T18:48:24Z msg", utc(2026, 5, 20, 18, 48, 24), 0)]
    fn every_format_yields_its_timestamp_and_message(
        #[case] line: &str,
        #[case] expected_second: DateTime<Utc>,
        #[case] expected_micros: u32,
    ) {
        let parsed = parse(line);
        let entry = parsed.entries().first().copied().expect("one entry");
        assert_eq!(
            entry.timestamp,
            expected_second + chrono::Duration::microseconds(expected_micros.into())
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
    fn the_parse_pool_leaves_at_least_half_the_cores_to_the_rest_of_the_machine() {
        let cores = thread::available_parallelism().map_or(1, NonZeroUsize::get);
        let pool = parse_pool().expect("the pool builds");
        assert_eq!(
            pool.current_num_threads(),
            (cores / CORES_PER_PARSE_WORKER).max(1),
            "on {cores} cores"
        );
    }

    /// Covers sort stability across a chunk bound: thousands of entries, many
    /// of them sharing a second, enough for the parallel sort to split its
    /// input.
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

        let parsed = chunked.expect("the fixture parses");
        assert!(parsed.skipped_line_count() > 0, "the fixture has bad lines");
        assert!(parsed.entries().len() > 1_000, "the fixture has entries");
    }

    fn any_line() -> impl Strategy<Value = String> {
        prop_oneof![
            r"\PC*",
            (0u32..24, 0u32..60, r"\PC*").prop_map(|(hour, minute, rest)| format!(
                "2026-01-01 {hour:02}:{minute:02}:00 {rest}"
            )),
            (1u32..29, r"\PC*").prop_map(|(day, rest)| format!("Jan {day:2} 00:00:00 {rest}")),
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
            let line_count = text.lines().filter(|line| !line.trim().is_empty()).count();
            prop_assert_eq!(parsed.entries().len() + parsed.skipped_line_count(), line_count);
            let sorted = parsed.entries().is_sorted_by_key(|entry| entry.timestamp);
            prop_assert!(sorted, "entries are sorted by timestamp");

            for entry in parsed.entries() {
                let start = entry.message_offset as usize;
                let end = start + entry.message_len as usize;
                let message = parsed.message(entry);
                prop_assert_eq!(text.get(start..end), Some(message));
                prop_assert_eq!(message, message.trim());
            }
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
        assert_eq!(entry.message_len, 0);
        assert_eq!(parsed.message(&entry), "");
    }
}
