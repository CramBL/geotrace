//! Detecting a log's format from its head, indexing every line against its
//! text, and reading the structure the exporter wrote around those lines.

use std::{num::NonZeroUsize, ops::Range, sync::Arc};

use chrono::{DateTime, Utc};
use gt_types::TimeRange;
use rayon::prelude::*;
use rustc_hash::FxHashMap;

use crate::{
    format::{self, LogFormat},
    pool,
    recognise::{self, HostnameColumn, RecognisedMessage},
    session::{self, BootSession, OrderAnomaly},
    structure::{StructuralExtent, StructuralLine, StructuralLineKind},
    summary::{self, EntryCountMismatch, SummaryBlock},
    text::LogText,
};

/// Non-empty lines the format detector reads before giving up on the log.
const FORMAT_DETECTION_LINE_LIMIT: usize = 10;

/// Lines from the head of the log the hostname decision reads. One exporter
/// writes one layout, so the head decides for the whole log, as it decides the
/// timestamp format.
const HOSTNAME_DETECTION_LINE_LIMIT: usize = 200;

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

    /// One per entry of `entries`, in the same order.
    recognised_messages: Vec<RecognisedMessage>,

    /// Each service the entries name, without its process id, in the order
    /// the services first appear. A service's index here is the slot its
    /// entries carry.
    services: Vec<TextSlice>,

    hostname_column: HostnameColumn,
    boot_sessions: Vec<BootSession>,
    structural_lines: Vec<StructuralLine>,
    order_anomalies: Vec<OrderAnomaly>,
    summary_block: Option<SummaryBlock>,
    format: LogFormat,
    anchored_entry_count: usize,
    unindexable_line_count: usize,
    replaced_byte_count: usize,
}

impl ParsedLog {
    pub fn text(&self) -> &Arc<str> {
        &self.text
    }

    /// Every kept line, anchored and interpolated alike, in file order.
    pub fn entries(&self) -> &[LogEntry] {
        &self.entries
    }

    /// What was recognised in each entry's message, indexed as
    /// [`ParsedLog::entries`] is.
    pub fn recognised_messages(&self) -> &[RecognisedMessage] {
        &self.recognised_messages
    }

    /// The services the log's entries name, each without its process id, in
    /// the order they first appear, which is the order slots are handed out
    /// in.
    pub fn services_by_first_appearance(&self) -> impl Iterator<Item = &str> {
        self.services
            .iter()
            .map(|service| service.in_text(&self.text))
    }

    /// Whether the log's messages open with the host that wrote them, decided
    /// from the head of the log.
    pub fn hostname_column(&self) -> HostnameColumn {
        self.hostname_column
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

    /// Bytes [`LogText::decode_lossy`] replaced to read this log as UTF-8.
    pub fn replaced_byte_count(&self) -> usize {
        self.replaced_byte_count
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
        Some(TimeRange::spanning(first, timestamps))
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

    #[error(
        "Not a recognised log: no line has a timestamp in a known format \
         (first line: {first_line:?})"
    )]
    NoRecognisedFormat { first_line: String },
}

/// Reads `text` into entries, taking the format from the head of the log and
/// resolving the year of the year-less syslog formats against `now`.
///
/// Every non-empty line is kept: one that carries no timestamp of that format
/// is either a structural line of a recognized exporter idiom or an entry
/// timestamped from its anchored neighbours. Only a line the index cannot
/// address is dropped.
pub fn parse_log(text: LogText, now: DateTime<Utc>) -> Result<ParsedLog, LogParseError> {
    parse_log_in_chunks_of(text, now, CHUNK_TARGET_BYTES)
}

/// Reads `text` as [`parse_log`] does, over chunks of `chunk_target_bytes`, so
/// a test drives the chunk merge over a log of any length.
pub fn parse_log_in_chunks_of(
    text: LogText,
    now: DateTime<Utc>,
    chunk_target_bytes: NonZeroUsize,
) -> Result<ParsedLog, LogParseError> {
    let (text, replaced_byte_count) = text.into_parts();
    let format = detect_head_format(&text)?;
    let hostname_column = detect_hostname_column(&text, format, now);
    let layout = LogLayout {
        format,
        hostname_column,
    };
    let mut index = index_lines_in_file_order(&text, layout, now, chunk_target_bytes);
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
        recognised_messages: index.recognised_messages,
        services: index.services,
        hostname_column,
        boot_sessions,
        structural_lines: index.structural_lines,
        order_anomalies,
        summary_block,
        format,
        anchored_entry_count,
        unindexable_line_count: index.unindexable_line_count,
        replaced_byte_count,
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

/// What the head of the log decided about every line of it: how a line writes
/// its timestamp, and whether its message opens with the host.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LogLayout {
    format: LogFormat,
    hostname_column: HostnameColumn,
}

/// Reads the head of the log for the `journalctl` shape - a word with no colon
/// before a word ending in one - and takes the shape most of those lines have
/// for the whole log.
fn detect_hostname_column(text: &str, format: LogFormat, now: DateTime<Utc>) -> HostnameColumn {
    let mut read = 0_usize;
    let mut with_a_hostname = 0_usize;
    for line in text.lines().take(HOSTNAME_DETECTION_LINE_LIMIT) {
        let Some((_, message)) = format::parse_line(line.trim(), format, now) else {
            continue;
        };
        read += 1;
        if recognise::opens_with_a_hostname(message) {
            with_a_hostname += 1;
        }
    }
    match with_a_hostname * 2 > read {
        true => HostnameColumn::Present,
        false => HostnameColumn::Absent,
    }
}

/// The services one stretch of a log names, numbered in the order they first
/// appear in it.
///
/// The chunk parse numbers them while the line it read is still in cache:
/// numbering in one pass over the whole text afterwards took 195 ms over a
/// 100 MiB log, against 57 ms here.
#[derive(Default)]
struct ServiceTable<'text> {
    slot_of_service: FxHashMap<&'text str, u16>,

    /// In first-appearance order, so a service's slot is its index here.
    names: Vec<TextSlice>,
}

impl<'text> ServiceTable<'text> {
    /// The slot of the service `span` of `message` names.
    fn slot_of(
        &mut self,
        message: &'text str,
        message_offset: u64,
        span: Range<usize>,
    ) -> Option<u16> {
        let name = message.get(span.clone())?;
        // The process id is no part of the service: `systemd[1]` and
        // `systemd[1223]` are the same service, logging under two of them.
        let named = name.strip_suffix(':').unwrap_or(name);
        let identity = named.split_once('[').map_or(named, |(service, _)| service);
        let slice = TextSlice::new(
            message_offset.saturating_add(span.start as u64),
            identity.len(),
        )?;
        Some(self.slot_of_name(identity, slice))
    }

    /// The slot `name` holds, a new one where this table has not seen it. The
    /// slot saturates at [`u16::MAX`], which a log of that many services
    /// shares between the rest of them.
    fn slot_of_name(&mut self, name: &'text str, slice: TextSlice) -> u16 {
        let next_slot = u16::try_from(self.names.len()).unwrap_or(u16::MAX);
        let names = &mut self.names;
        *self.slot_of_service.entry(name).or_insert_with(|| {
            names.push(slice);
            next_slot
        })
    }
}

/// The services of a whole log, and what the slots of each of its chunks mean
/// in it.
struct MergedServices {
    /// In the order the log first names them.
    names: Vec<TextSlice>,

    /// One list per chunk, in chunk order, indexed by that chunk's own slots.
    log_slot_of_chunk_slot: Vec<Vec<u16>>,
}

/// Numbers the services of the whole log over the tables its chunks built,
/// keeping the order the log first names them.
fn merge_service_tables(text: &str, chunks: &[LineIndex]) -> MergedServices {
    let mut merged = ServiceTable::default();
    let log_slot_of_chunk_slot = chunks
        .iter()
        .map(|chunk| {
            chunk
                .services
                .iter()
                .map(|name| merged.slot_of_name(name.in_text(text), *name))
                .collect()
        })
        .collect();
    MergedServices {
        names: merged.names,
        log_slot_of_chunk_slot,
    }
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

    /// One per entry, pushed and dropped with it.
    recognised_messages: Vec<RecognisedMessage>,

    /// The services these entries name, in the order they first appear here.
    /// A slot in `recognised_messages` indexes this list.
    services: Vec<TextSlice>,

    structural_lines: Vec<StructuralLine>,
    line_count: u32,
    unindexable_line_count: usize,
}

impl LineIndex {
    /// Joins per-chunk indices, given in the order their chunks appear in the
    /// log, numbering their lines from the head of the whole log and their
    /// services in the order the log first names them.
    fn concatenated(text: &str, chunks: &[Self]) -> Self {
        let merged = merge_service_tables(text, chunks);
        let mut joined = Self {
            entries: Vec::with_capacity(chunks.iter().map(|chunk| chunk.entries.len()).sum()),
            services: merged.names,
            ..Self::default()
        };
        for (chunk, log_slot_of_chunk_slot) in chunks.iter().zip(&merged.log_slot_of_chunk_slot) {
            let lines_before = joined.line_count;
            joined
                .entries
                .extend(chunk.entries.iter().map(|entry| LogEntry {
                    line_number: entry.line_number.saturating_add(lines_before),
                    ..*entry
                }));
            joined
                .recognised_messages
                .extend(chunk.recognised_messages.iter().map(|recognised| {
                    let mut recognised = *recognised;
                    if let Some(service) = recognised.service()
                        && let Some(slot) = log_slot_of_chunk_slot.get(usize::from(service.slot()))
                    {
                        recognised.set_service_slot(*slot);
                    }
                    recognised
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

    fn push_classified_line<'text>(
        &mut self,
        line: PositionedLine<'text>,
        layout: LogLayout,
        now: DateTime<Utc>,
        services: &mut ServiceTable<'text>,
    ) {
        if let Some((timestamp, message)) = format::parse_line(line.trimmed, layout.format, now) {
            self.push_anchored_entry(line, timestamp, message, layout.hostname_column, services);
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
            None => self.push_entry(
                LogEntry {
                    // Replaced by the interpolation pass, which reaches every
                    // entry this branch pushes.
                    timestamp: DateTime::UNIX_EPOCH,
                    timestamp_kind: TimestampKind::Interpolated,
                    line_number: line.line_number,
                    message: text,
                },
                line.trimmed,
                layout.hostname_column,
                services,
            ),
        }
    }

    /// The one way an entry enters the index: its recognised message is pushed
    /// with it, so the two stay indexed alike, and its service takes a slot in
    /// `services`.
    fn push_entry<'text>(
        &mut self,
        entry: LogEntry,
        message: &'text str,
        hostname_column: HostnameColumn,
        services: &mut ServiceTable<'text>,
    ) {
        let mut recognised = recognise::recognise_message(message, hostname_column);
        if let Some(service) = recognised.service()
            && let Some(slot) = services.slot_of(message, entry.message.offset, service.span())
        {
            recognised.set_service_slot(slot);
        }
        self.entries.push(entry);
        self.recognised_messages.push(recognised);
    }

    fn push_anchored_entry<'text>(
        &mut self,
        line: PositionedLine<'text>,
        timestamp: DateTime<Utc>,
        message: &'text str,
        hostname_column: HostnameColumn,
        services: &mut ServiceTable<'text>,
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
            Some(slice) => self.push_entry(
                LogEntry {
                    timestamp,
                    timestamp_kind: TimestampKind::Anchored,
                    line_number: line.line_number,
                    message: slice,
                },
                message,
                hostname_column,
                services,
            ),
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
        self.recognised_messages.truncate(self.entries.len());
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
    layout: LogLayout,
    now: DateTime<Utc>,
    chunk_target_bytes: NonZeroUsize,
) -> LineIndex {
    let chunks = newline_aligned_chunks(text, chunk_target_bytes);
    match chunks.as_slice() {
        [] => LineIndex::default(),
        [only] => only.parse(layout, now),
        many => match pool::log_worker_pool() {
            Some(pool) => pool.install(|| {
                let per_chunk: Vec<LineIndex> = many
                    .par_iter()
                    .map(|chunk| chunk.parse(layout, now))
                    .collect();
                LineIndex::concatenated(text, &per_chunk)
            }),
            None => LineIndex::concatenated(
                text,
                &many
                    .iter()
                    .map(|chunk| chunk.parse(layout, now))
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
    fn parse(&self, layout: LogLayout, now: DateTime<Utc>) -> LineIndex {
        let mut index = LineIndex::default();
        let mut services = ServiceTable::default();
        for line in positioned_lines(self.text, self.offset_in_text, 1) {
            index.line_count = line.line_number;
            if line.trimmed.is_empty() {
                continue;
            }
            index.push_classified_line(line, layout, now, &mut services);
        }
        index.services = services.names;
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
    use std::collections::HashMap;

    use chrono::{Duration, TimeZone as _};
    use gt_test_utils::log_fixtures::{self, SyntheticLogSpec, SyntheticLogTimestamps};
    use proptest::{prelude::*, proptest};
    use rstest::rstest;

    use super::*;
    use crate::{
        log_strategies,
        recognise::{RecognisedLevel, RecognisedService},
        summary::ServiceCount,
    };

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
        parse_log(text.into(), now()).expect("parses")
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
            parse_log("nothing here\nnor here\n".into(), now()).expect_err("fails to parse");
        assert_eq!(
            error.to_string(),
            "Not a recognised log: no line has a timestamp in a known format \
             (first line: \"nothing here\")"
        );
    }

    #[test]
    fn a_very_long_first_line_is_quoted_up_to_an_excerpt() {
        let text = "x".repeat(ERROR_LINE_EXCERPT_CHARS.get() + 50);
        let error = parse_log(text.as_str().into(), now()).expect_err("fails to parse");
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
        assert_eq!(parse_log(text.into(), now()), Err(LogParseError::Empty));
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
            parse_log(past_head.as_str().into(), now()),
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
            "Dec 31 23:59:59 rollover\n".into(),
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

    /// A session's uptime spans its own anchors. A clock adjustment that lands
    /// the session's last anchor before its first leaves it without one.
    #[test]
    fn a_boot_session_spans_its_own_anchors() {
        let parsed = parse(&format!(
            "2026-01-01 00:00:00 a\n2026-01-01 02:30:00 b\n{REBOOT}2026-01-01 00:05:00 c\n\
             2026-01-01 00:04:00 systemd-timedated: Time has been changed\n"
        ));
        let uptimes: Vec<Option<Duration>> = parsed
            .boot_sessions()
            .iter()
            .map(BootSession::uptime)
            .collect();
        assert_eq!(uptimes, [Some(Duration::minutes(150)), None]);
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
            "kernel output\n----------- Journal summary -----------\n2026-01-01 00:00:00 a\n"
                .into(),
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
            timestamps: SyntheticLogTimestamps::SyslogShort,
        });
        let chunk_target_bytes = chunk_bytes(4 * 1024);
        assert!(
            newline_aligned_chunks(&text, chunk_target_bytes).len() > 8,
            "the fixture has to span several chunks for this to compare the two paths"
        );

        let one_chunk = parse_log_in_chunks_of(
            text.as_str().into(),
            now(),
            chunk_bytes(text.len().saturating_add(1)),
        );
        let chunked = parse_log_in_chunks_of(text.as_str().into(), now(), chunk_target_bytes);
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

    /// The text a slice addresses, `None` where it addresses text outside the
    /// log or splits a character in it.
    fn sliced(slice: TextSlice, text: &str) -> Option<&str> {
        let start = usize::try_from(slice.offset).ok()?;
        text.get(start..start.checked_add(usize::try_from(slice.len).ok()?)?)
    }

    /// The trimmed line `line_number` names, empty where the log has no such
    /// line.
    fn line_of(text: &str, line_number: u32) -> &str {
        let index = usize::try_from(line_number).unwrap_or(usize::MAX);
        text.lines()
            .nth(index.saturating_sub(1))
            .unwrap_or_default()
            .trim()
    }

    proptest! {
        /// A parse fails on one condition: no line before the summary block
        /// carries a timestamp in the format the head of the log decided.
        #[test]
        fn a_log_parses_exactly_when_a_line_outside_its_summary_block_is_timestamped(
            text in log_strategies::any_log_text(),
        ) {
            let head_format = text
                .lines()
                .map(str::trim)
                .filter(|line| !line.is_empty())
                .take(FORMAT_DETECTION_LINE_LIMIT)
                .find_map(format::detect_format);

            let mut anchored = false;
            for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
                if head_format
                    .is_some_and(|format| format::parse_line(line, format, now()).is_some())
                {
                    anchored = true;
                    break;
                }
                if StructuralLineKind::matching_line(line)
                    == Some(StructuralLineKind::SummaryBlock)
                {
                    break;
                }
            }

            prop_assert_eq!(parse_log(text.as_str().into(), now()).is_ok(), anchored);
        }

        /// Whatever text a user drops on the app, every span the parse hands
        /// out slices the text or the message it was read from.
        #[test]
        fn every_span_of_a_parsed_log_slices_the_text_it_was_indexed_from(
            text in log_strategies::any_log_text(),
        ) {
            let Ok(parsed) = parse_log(text.as_str().into(), now()) else {
                return Ok(());
            };
            prop_assert_eq!(parsed.text().as_ref(), text.as_str());

            for entry in parsed.entries() {
                let message = parsed.message(entry);
                prop_assert_eq!(sliced(entry.message, &text), Some(message));
                prop_assert_eq!(message, message.trim());
            }
            for line in parsed.structural_lines() {
                let structural = sliced(line.text, &text);
                prop_assert_eq!(structural, Some(line.text.in_text(&text)));
                prop_assert_ne!(structural, Some(""));
            }
            for (entry, recognised) in parsed.entries().iter().zip(parsed.recognised_messages()) {
                let message = parsed.message(entry);
                let spans = [
                    recognised.hostname(),
                    recognised.service().map(RecognisedService::span),
                    recognised.level().map(RecognisedLevel::span),
                ];
                for span in spans.into_iter().flatten() {
                    prop_assert!(message.get(span).is_some());
                }
            }
        }

        /// Every non-empty line ends up in exactly one of the parse's counts,
        /// and every entry in exactly one boot session.
        #[test]
        fn every_non_empty_line_is_counted_exactly_once(
            text in log_strategies::any_log_text(),
        ) {
            let Ok(parsed) = parse_log(text.as_str().into(), now()) else {
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

        /// Entries and structural lines each name one line of the log, in the
        /// order the file wrote them.
        #[test]
        fn entries_and_structural_lines_are_in_strict_line_order(
            text in log_strategies::any_log_text(),
        ) {
            let Ok(parsed) = parse_log(text.as_str().into(), now()) else {
                return Ok(());
            };

            prop_assert!(
                parsed
                    .entries()
                    .is_sorted_by(|before, after| before.line_number < after.line_number)
            );
            prop_assert!(
                parsed
                    .structural_lines()
                    .is_sorted_by(|before, after| before.line_number < after.line_number)
            );
            for entry in parsed.entries() {
                prop_assert!(!line_of(&text, entry.line_number).is_empty());
            }
        }

        /// An entry is anchored exactly where its own line carries a timestamp
        /// in the format the log was read in.
        #[test]
        fn an_entry_is_anchored_exactly_when_its_line_carries_a_timestamp(
            text in log_strategies::any_log_text(),
        ) {
            let Ok(parsed) = parse_log(text.as_str().into(), now()) else {
                return Ok(());
            };

            for entry in parsed.entries() {
                let line = line_of(&text, entry.line_number);
                prop_assert_eq!(
                    entry.is_anchored(),
                    format::parse_line(line, parsed.format(), now()).is_some(),
                    "line {}: {:?}", entry.line_number, line
                );
            }
        }

        /// An interpolated entry lands between the anchored entries around it
        /// in its own boot session, whichever way the clock stepped between
        /// them. An entry with an anchored entry on one side only takes the
        /// timestamp of that one.
        #[test]
        fn an_interpolated_entry_lies_between_the_anchors_of_its_session(
            text in log_strategies::any_log_text(),
        ) {
            let Ok(parsed) = parse_log(text.as_str().into(), now()) else {
                return Ok(());
            };

            for session in parsed.boot_sessions() {
                let entries = parsed.session_entries(session);
                for (index, entry) in entries.iter().enumerate() {
                    if entry.is_anchored() {
                        continue;
                    }
                    let before = entries
                        .get(..index)
                        .unwrap_or_default()
                        .iter()
                        .rfind(|entry| entry.is_anchored());
                    let after = entries
                        .get(index.saturating_add(1)..)
                        .unwrap_or_default()
                        .iter()
                        .find(|entry| entry.is_anchored());
                    match (before, after) {
                        (Some(before), Some(after)) => {
                            let earliest = before.timestamp.min(after.timestamp);
                            let latest = before.timestamp.max(after.timestamp);
                            prop_assert!((earliest..=latest).contains(&entry.timestamp));
                        }
                        (Some(anchor), None) | (None, Some(anchor)) => {
                            prop_assert_eq!(entry.timestamp, anchor.timestamp);
                        }
                        (None, None) => {}
                    }
                }
            }
        }

        /// An anomaly names an anchored entry of a boot session with an
        /// anchored entry before it in that session: the step is measured
        /// between the two.
        #[test]
        fn every_order_anomaly_names_an_anchored_entry_of_a_boot_session(
            text in log_strategies::any_log_text(),
        ) {
            let Ok(parsed) = parse_log(text.as_str().into(), now()) else {
                return Ok(());
            };

            for anomaly in parsed.order_anomalies() {
                let index = parsed
                    .entries()
                    .iter()
                    .position(|entry| entry.line_number == anomaly.line_number);
                let Some(index) = index else {
                    return Err(TestCaseError::fail(format!(
                        "anomaly on line {} names no entry", anomaly.line_number
                    )));
                };
                prop_assert!(parsed.entries().get(index).is_some_and(LogEntry::is_anchored));

                let session = parsed
                    .boot_sessions()
                    .iter()
                    .find(|session| session.entry_range.contains(&index));
                let Some(session) = session else {
                    return Err(TestCaseError::fail(format!(
                        "entry {index} lies in no boot session"
                    )));
                };
                prop_assert!(
                    parsed
                        .entries()
                        .get(session.entry_range.start..index)
                        .unwrap_or_default()
                        .iter()
                        .any(LogEntry::is_anchored)
                );
            }
        }

        /// One service takes one slot over the whole log, whichever chunk read
        /// it, and its slot names it in the log's own service list.
        #[test]
        fn every_entry_naming_a_service_carries_that_services_slot(
            text in log_strategies::any_log_text(),
        ) {
            let Ok(parsed) = parse_log(text.as_str().into(), now()) else {
                return Ok(());
            };
            let names: Vec<&str> = parsed.services_by_first_appearance().collect();
            let mut slot_of_token: HashMap<&str, u16> = HashMap::new();

            for (entry, recognised) in parsed.entries().iter().zip(parsed.recognised_messages()) {
                let Some(service) = recognised.service() else {
                    continue;
                };
                let token = parsed.message(entry).get(service.span()).unwrap_or_default();
                let name = names.get(usize::from(service.slot())).copied();
                prop_assert!(
                    name.is_some_and(|name| token.starts_with(name)),
                    "slot {} of {names:?} does not name the service of {token:?}", service.slot()
                );
                let first_slot = *slot_of_token.entry(token).or_insert(service.slot());
                prop_assert_eq!(service.slot(), first_slot, "two slots for {:?}", token);
            }
        }

        /// The exporter's own entry count is held against the anchored entries
        /// the parse read, and reported only where the two differ.
        #[test]
        fn an_entry_count_mismatch_is_reported_exactly_when_the_exporter_disagrees(
            text in log_strategies::any_summarised_log(),
        ) {
            let Ok(parsed) = parse_log(text.as_str().into(), now()) else {
                return Ok(());
            };
            let stated_by_exporter = parsed.summary_block().and_then(|block| block.entry_count);
            let anchored_by_parse =
                u64::try_from(parsed.anchored_entry_count()).unwrap_or(u64::MAX);

            prop_assert_eq!(
                parsed.exporter_entry_count_mismatch(),
                stated_by_exporter
                    .filter(|stated| *stated != anchored_by_parse)
                    .map(|stated_by_exporter| EntryCountMismatch {
                        stated_by_exporter,
                        anchored_by_parse,
                    })
            );
        }

        /// However badly a log is formed and wherever the chunk bounds fall
        /// in it, the chunked path returns what the one-chunk path returns.
        #[test]
        fn a_chunked_parse_of_any_text_matches_the_one_chunk_parse(
            text in log_strategies::any_log_text(),
            chunk_target_bytes in 1usize..64,
        ) {
            let one_chunk = parse_log_in_chunks_of(
                text.as_str().into(),
                now(),
                chunk_bytes(text.len().saturating_add(1)),
            );
            let chunked = parse_log_in_chunks_of(
                text.as_str().into(),
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

    /// The layout is the exporter's: the head of the log decides for every
    /// line, and a log of both shapes takes the one most of its head has.
    #[rstest]
    #[case::journalctl(
        "2026-01-01 00:00:00 workstation systemd[1]: a\n2026-01-01 00:00:01 workstation kernel: b\n",
        HostnameColumn::Present
    )]
    #[case::device_export(
        "2026-01-01 00:00:00 systemd: a\n2026-01-01 00:00:01 kernel: b\n",
        HostnameColumn::Absent
    )]
    #[case::mostly_hosts(
        "2026-01-01 00:00:00 workstation systemd[1]: a\n2026-01-01 00:00:01 workstation kernel: b\n\
         2026-01-01 00:00:02 kernel: c\n",
        HostnameColumn::Present
    )]
    #[case::mostly_services(
        "2026-01-01 00:00:00 workstation systemd[1]: a\n2026-01-01 00:00:01 kernel: b\n\
         2026-01-01 00:00:02 kernel: c\n",
        HostnameColumn::Absent
    )]
    fn the_head_of_the_log_decides_whether_its_lines_name_a_host(
        #[case] text: &str,
        #[case] expected: HostnameColumn,
    ) {
        assert_eq!(parse(text).hostname_column(), expected);
    }

    /// The lines below the head are read the way the head decided, whatever
    /// shape they have themselves.
    #[test]
    fn a_line_past_the_head_is_read_with_the_layout_the_head_decided() {
        let mut text =
            "2026-01-01 00:00:00 workstation systemd[1]: a\n".repeat(HOSTNAME_DETECTION_LINE_LIMIT);
        text.push_str("2026-01-01 00:00:01 kernel: past the head\n");
        let parsed = parse(&text);
        let last = parsed
            .recognised_messages()
            .last()
            .copied()
            .expect("the log has entries");

        assert_eq!(parsed.hostname_column(), HostnameColumn::Present);
        assert_eq!(
            last.hostname(),
            Some(0.."kernel:".len()),
            "the last line's own service is read as the host the layout expects there"
        );
        assert_eq!(last.service(), None);
    }

    /// A seventh service takes a seventh slot: the palette, not the parse,
    /// decides which of them share a colour.
    #[test]
    fn every_service_of_a_log_takes_a_slot_of_its_own() {
        let text: String = ["a", "b", "c", "d", "e", "f", "g"]
            .iter()
            .map(|service| format!("2026-01-01 00:00:00 {service}: logged\n"))
            .collect();
        let parsed = parse(&text);

        assert_eq!(
            parsed.services_by_first_appearance().collect::<Vec<_>>(),
            ["a", "b", "c", "d", "e", "f", "g"]
        );
        assert_eq!(
            parsed
                .recognised_messages()
                .iter()
                .filter_map(|recognised| recognised.service())
                .map(RecognisedService::slot)
                .collect::<Vec<_>>(),
            [0, 1, 2, 3, 4, 5, 6]
        );
    }

    #[test]
    fn a_line_without_a_message_yields_an_empty_one() {
        let parsed = parse("2026-01-01 00:00:00\n");
        let entry = parsed.entries().first().copied().expect("one entry");
        assert_eq!(entry.message.len, 0);
        assert_eq!(parsed.message(&entry), "");
    }
}
