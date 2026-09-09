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
mod tests;
