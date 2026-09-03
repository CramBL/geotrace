//! Reading the parts of an entry's message a reader scans for: the service
//! that logged the line, the level it logged at, and the host it came from.
//!
//! The two known sources are a device's own journald export, whose messages
//! open with the service, and a workstation's `journalctl`, whose messages open
//! with the host.

use std::{
    num::{NonZeroU8, NonZeroU16, NonZeroUsize},
    ops::Range,
};

/// Bytes a service name may take from where it starts. A message whose first
/// word runs past this carries no service.
const SERVICE_LIMIT_BYTES: usize = 64;

/// Bytes a hostname may take, past which the message opens with no host.
const HOSTNAME_LIMIT_BYTES: usize = 64;

/// Bytes after the service a level token is looked for in.
const LEVEL_HEAD_BYTES: usize = 40;

/// Bytes a level token may take, which is what its length field addresses.
const LEVEL_LIMIT_BYTES: usize = u8::MAX as usize;

/// The level words a message states its severity with, whatever their case.
const LEVEL_VOCABULARY: &[(&str, LogLevelKind)] = &[
    ("ERROR", LogLevelKind::Error),
    ("ERR", LogLevelKind::Error),
    ("CRIT", LogLevelKind::Error),
    ("CRITICAL", LogLevelKind::Error),
    ("FATAL", LogLevelKind::Error),
    ("PANIC", LogLevelKind::Error),
    ("EMERG", LogLevelKind::Error),
    ("ALERT", LogLevelKind::Error),
    ("WARN", LogLevelKind::Warning),
    ("WARNING", LogLevelKind::Warning),
    ("INFO", LogLevelKind::Info),
    ("NOTICE", LogLevelKind::Info),
    ("DEBUG", LogLevelKind::Debug),
    ("TRACE", LogLevelKind::Debug),
    ("VERBOSE", LogLevelKind::Debug),
];

/// Whether every line of a log names the host it came from. `journalctl` writes
/// the host before the service. A device's own export leaves it out.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostnameColumn {
    Present,
    Absent,
}

/// The severity a level token states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogLevelKind {
    Error,
    Warning,
    Info,
    Debug,
}

/// The parts of one entry's message that were recognised, as byte ranges
/// relative to the message.
///
/// Twelve bytes per entry, beside the forty an entry itself takes: a log of a
/// million lines carries its recognition in twelve megabytes, and a drawn row
/// looks its colouring up here.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecognisedMessage {
    hostname_len: Option<NonZeroU16>,
    service: Option<RecognisedService>,
    level: Option<RecognisedLevel>,
}

impl RecognisedMessage {
    /// The host the line came from, which opens the message where the log has
    /// a hostname column.
    pub fn hostname(self) -> Option<Range<usize>> {
        Some(0..NonZeroUsize::from(self.hostname_len?).get())
    }

    pub fn service(self) -> Option<RecognisedService> {
        self.service
    }

    pub fn level(self) -> Option<RecognisedLevel> {
        self.level
    }

    /// Called once per entry, once the services are known in the order they
    /// first appear.
    pub(crate) fn set_service_slot(&mut self, slot: u16) {
        if let Some(service) = &mut self.service {
            service.slot = slot;
        }
    }
}

/// The service a message names, up to and including its colon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecognisedService {
    start: u16,
    slot: u16,
    len: NonZeroU8,
}

impl RecognisedService {
    fn at(span: Range<usize>) -> Option<Self> {
        Some(Self {
            start: u16::try_from(span.start).ok()?,
            len: NonZeroU8::new(u8::try_from(span.len()).ok()?)?,
            slot: 0,
        })
    }

    pub fn span(self) -> Range<usize> {
        let start = usize::from(self.start);
        start..start.saturating_add(NonZeroUsize::from(self.len).get())
    }

    /// Counted up from zero over the services of the log in the order they
    /// first appear, saturating past 65535. The viewer's palette cycles it
    /// again over the colours it has.
    pub fn slot(self) -> u16 {
        self.slot
    }
}

/// The level token of a message: a bracketed `[WARN can::session_pool]` to its
/// closing bracket, or a bare `INFO:` to its colon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecognisedLevel {
    start: u16,
    len: NonZeroU8,
    kind: LogLevelKind,
}

impl RecognisedLevel {
    fn at(span: Range<usize>, kind: LogLevelKind) -> Option<Self> {
        Some(Self {
            start: u16::try_from(span.start).ok()?,
            len: NonZeroU8::new(u8::try_from(span.len()).ok()?)?,
            kind,
        })
    }

    pub fn span(self) -> Range<usize> {
        let start = usize::from(self.start);
        start..start.saturating_add(NonZeroUsize::from(self.len).get())
    }

    pub fn kind(self) -> LogLevelKind {
        self.kind
    }
}

/// Reads one message, given what the head of the log decided about its
/// hostname column. The same message always reads the same way: nothing here
/// depends on the entries around it.
pub(crate) fn recognise_message(
    message: &str,
    hostname_column: HostnameColumn,
) -> RecognisedMessage {
    let bytes = message.as_bytes();
    let hostname_end = match hostname_column {
        HostnameColumn::Present => leading_word_end(bytes),
        HostnameColumn::Absent => None,
    };
    let service_start = hostname_end.map_or(0, |end| past_spaces(bytes, end));
    let service = service_span(bytes, service_start);
    let head_start = service.as_ref().map_or(service_start, |span| span.end);
    RecognisedMessage {
        hostname_len: hostname_end
            .and_then(|end| u16::try_from(end).ok())
            .and_then(NonZeroU16::new),
        service: service.and_then(RecognisedService::at),
        level: level_token(bytes, head_start),
    }
}

/// Whether a message opens the way a hostname column looks: a word with no
/// colon, then a word ending in one.
pub(crate) fn opens_with_a_hostname(message: &str) -> bool {
    let mut words = message.split_ascii_whitespace();
    let (Some(first), Some(second)) = (words.next(), words.next()) else {
        return false;
    };
    !first.contains(':') && second.ends_with(':')
}

/// The characters a service name is made of, beside the `[pid]` it may carry
/// before its colon.
fn is_service_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b'.' | b'/' | b'@' | b'-' | b'(' | b')')
}

/// The service starting at `start`, `None` where the bytes there are not a
/// name closed by a colon within [`SERVICE_LIMIT_BYTES`].
fn service_span(bytes: &[u8], start: usize) -> Option<Range<usize>> {
    let limit = start.saturating_add(SERVICE_LIMIT_BYTES).min(bytes.len());
    let scanned = bytes.get(..limit)?;
    let mut at = start;
    while at < scanned.len() {
        match scanned.get(at).copied()? {
            b':' if at > start => return Some(start..at.saturating_add(1)),
            b'[' if at > start => at = process_id_end(scanned, at)?,
            byte if is_service_byte(byte) => at = at.saturating_add(1),
            _ => return None,
        }
    }
    None
}

/// The byte after the `]` closing the `[1223]` a service name carries at
/// `open`, `None` where those bytes are not digits in brackets.
fn process_id_end(bytes: &[u8], open: usize) -> Option<usize> {
    let digits_start = open.saturating_add(1);
    let mut at = digits_start;
    while bytes.get(at).is_some_and(u8::is_ascii_digit) {
        at = at.saturating_add(1);
    }
    (at > digits_start && bytes.get(at) == Some(&b']')).then(|| at.saturating_add(1))
}

/// The level token in the head of the message, which begins at `head_start`.
/// The bare form is read first: it can only stand at the head, while a bracket
/// stating a level may follow one that does not.
fn level_token(bytes: &[u8], head_start: usize) -> Option<RecognisedLevel> {
    let from = past_spaces(bytes, head_start);
    let head_end = from.saturating_add(LEVEL_HEAD_BYTES).min(bytes.len());
    let head = bytes.get(from..head_end)?;
    if let Some(level) = bare_level(head, from) {
        return Some(level);
    }
    // A bracket opening in the head is read to its closing bracket, which may
    // stand past the head.
    memchr::memchr_iter(b'[', head)
        .find_map(|open| bracketed_level(bytes, from.saturating_add(open)))
}

/// The `INFO:` a shell script writes, taken to its colon. `head` is the head
/// of the message, which starts at `head_start` in it.
fn bare_level(head: &[u8], head_start: usize) -> Option<RecognisedLevel> {
    let colon = memchr::memchr(b':', head)?;
    let kind = level_of(head.get(..colon)?)?;
    let end = head_start.saturating_add(colon).saturating_add(1);
    RecognisedLevel::at(head_start..end, kind)
}

/// The `[WARN can::session_pool]` a Rust service writes, taken to its closing
/// bracket. A bracket whose first word states no level, or that never closes
/// within [`LEVEL_LIMIT_BYTES`], is no level token.
fn bracketed_level(bytes: &[u8], open: usize) -> Option<RecognisedLevel> {
    let limit = open.saturating_add(LEVEL_LIMIT_BYTES).min(bytes.len());
    let bracketed = bytes.get(open.saturating_add(1)..limit)?;
    let word_len = memchr::memchr2(b' ', b']', bracketed)?;
    let kind = level_of(bracketed.get(..word_len)?)?;
    let close = memchr::memchr(b']', bracketed)?;
    let end = open.saturating_add(close).saturating_add(2);
    RecognisedLevel::at(open..end, kind)
}

fn level_of(word: &[u8]) -> Option<LogLevelKind> {
    LEVEL_VOCABULARY
        .iter()
        .find(|(name, _)| name.as_bytes().eq_ignore_ascii_case(word))
        .map(|&(_, kind)| kind)
}

/// The index of the space closing the word the message opens with, `None` for
/// a word that is empty or that no space closes within
/// [`HOSTNAME_LIMIT_BYTES`].
fn leading_word_end(bytes: &[u8]) -> Option<usize> {
    let end = bytes
        .iter()
        .take(HOSTNAME_LIMIT_BYTES)
        .position(|byte| *byte == b' ')?;
    (end > 0).then_some(end)
}

fn past_spaces(bytes: &[u8], from: usize) -> usize {
    let spaces = bytes
        .iter()
        .skip(from)
        .take_while(|byte| **byte == b' ')
        .count();
    from.saturating_add(spaces)
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    /// What the recogniser found in a message, as the cases below write it:
    /// the hostname, the service and the level, each as the text it covers.
    fn recognised(message: &str, hostname_column: HostnameColumn) -> (&str, &str, &str) {
        let read = recognise_message(message, hostname_column);
        let part = |span: Option<Range<usize>>| {
            span.and_then(|span| message.get(span)).unwrap_or_default()
        };
        (
            part(read.hostname()),
            part(read.service().map(RecognisedService::span)),
            part(read.level().map(RecognisedLevel::span)),
        )
    }

    #[rstest]
    #[case::plain("kernel: Booting Linux", "kernel:")]
    #[case::hyphenated("hal-gnss: opening serial device", "hal-gnss:")]
    #[case::parenthesised("(udev-worker): ci_hdrc.0: failed", "(udev-worker):")]
    #[case::capitalised("No-service-id: src/command/install.c", "No-service-id:")]
    #[case::dotted("save-time-state.sh: writing state", "save-time-state.sh:")]
    #[case::with_a_process_id("systemd[1223]: Started chronyd", "systemd[1223]:")]
    #[case::underscored("sshd_check_keys: rotating", "sshd_check_keys:")]
    #[case::second_colon_ignored("kernel: usbcore: registered", "kernel:")]
    #[case::no_colon("ScanElement {", "")]
    #[case::space_before_the_colon("hal accel: reading", "")]
    #[case::colon_first(": nothing before it", "")]
    #[case::empty("", "")]
    #[case::only_spaces("    ", "")]
    #[case::process_id_without_digits("systemd[]: Started", "")]
    fn a_service_is_the_first_word_of_the_message_ending_in_a_colon(
        #[case] message: &str,
        #[case] expected: &str,
    ) {
        let (_, service, _) = recognised(message, HostnameColumn::Absent);
        assert_eq!(service, expected);
    }

    /// A name longer than the scan reads is no service, so no line costs more
    /// than a bounded look at its head.
    #[test]
    fn a_service_past_the_scan_limit_is_not_recognised() {
        let longest = format!("{}: still read", "s".repeat(SERVICE_LIMIT_BYTES - 1));
        let (_, service, _) = recognised(&longest, HostnameColumn::Absent);
        assert_eq!(service.len(), SERVICE_LIMIT_BYTES);

        let past_the_limit = format!("{}: not read", "s".repeat(SERVICE_LIMIT_BYTES));
        let (_, service, _) = recognised(&past_the_limit, HostnameColumn::Absent);
        assert_eq!(service, "");
    }

    #[rstest]
    #[case::bracketed_with_a_target(
        "hal-pm: [INFO common::feature_flags] watching",
        "[INFO common::feature_flags]",
        Some(LogLevelKind::Info)
    )]
    #[case::bracketed_alone(
        "app:  [WARNING] no previous state",
        "[WARNING]",
        Some(LogLevelKind::Warning)
    )]
    #[case::bracketed_error(
        "hal-modem: [ERROR modem::manager::modem] timed out",
        "[ERROR modem::manager::modem]",
        Some(LogLevelKind::Error)
    )]
    #[case::bare(
        "save-time-state.sh: INFO: writing state",
        "INFO:",
        Some(LogLevelKind::Info)
    )]
    #[case::bare_debug(
        "save-time-state.sh: DEBUG: waitsync output",
        "DEBUG:",
        Some(LogLevelKind::Debug)
    )]
    #[case::lower_case("app: [warn] gnss", "[warn]", Some(LogLevelKind::Warning))]
    #[case::without_a_service("[TRACE] no service here", "[TRACE]", Some(LogLevelKind::Debug))]
    #[case::outside_the_vocabulary("kernel: [UFW BLOCK] IN=enp5s0", "", None)]
    #[case::a_date_in_brackets("podman: [2026-09-03 21:11:29] container init", "", None)]
    #[case::a_subsystem_prefix("kernel: usbcore: registered", "", None)]
    #[case::a_bracket_that_never_closes("app: [WARN gnss", "", None)]
    #[case::an_open_bracket_alone("app: [", "", None)]
    #[case::past_the_head(
        "kernel: 0123456789012345678901234567890123456789 [WARN] late",
        "",
        None
    )]
    fn a_level_is_a_vocabulary_word_in_brackets_or_before_a_colon(
        #[case] message: &str,
        #[case] expected_token: &str,
        #[case] expected_kind: Option<LogLevelKind>,
    ) {
        let (_, _, level) = recognised(message, HostnameColumn::Absent);
        assert_eq!(level, expected_token);
        assert_eq!(
            recognise_message(message, HostnameColumn::Absent)
                .level()
                .map(RecognisedLevel::kind),
            expected_kind
        );
    }

    /// The bracket stating the level is the first one that does: `[UFW BLOCK]`
    /// does not stop the scan.
    #[test]
    fn a_bracket_outside_the_vocabulary_is_read_past() {
        let (_, _, level) =
            recognised("kernel: [UFW BLOCK] [WARN] dropped", HostnameColumn::Absent);
        assert_eq!(level, "[WARN]");
    }

    #[test]
    fn a_hostname_column_moves_the_service_to_the_second_word() {
        assert_eq!(
            recognised(
                "workstation chronyd[1042]: [WARN] no selectable sources",
                HostnameColumn::Present
            ),
            ("workstation", "chronyd[1042]:", "[WARN]")
        );
        assert_eq!(
            recognised(
                "workstation chronyd[1042]: [WARN] no selectable sources",
                HostnameColumn::Absent
            ),
            ("", "", "[WARN]"),
            "read without a hostname column, the host is where the service would be"
        );
    }

    #[rstest]
    #[case::journalctl("workstation systemd[1]: Started chronyd.service", true)]
    #[case::device_export("hal-gnss: [WARN] no stored time", false)]
    #[case::one_word_only("kernel:", false)]
    #[case::no_colon_at_all("Stack trace follows", false)]
    fn a_message_opens_with_a_hostname_when_a_word_without_a_colon_precedes_one(
        #[case] message: &str,
        #[case] expected: bool,
    ) {
        assert_eq!(opens_with_a_hostname(message), expected);
    }

    /// Every span lands on a character boundary, so the viewer can slice the
    /// message it was read from.
    #[test]
    fn a_span_never_splits_a_multi_byte_character() {
        let message = "sérvice: [WARN gnss::aidingÅ] no stored time";
        let read = recognise_message(message, HostnameColumn::Absent);

        assert_eq!(read.service(), None, "é is not a service character");
        let level = read.level().expect("the bracket states a level");
        assert_eq!(message.get(level.span()), Some("[WARN gnss::aidingÅ]"));
    }

    /// The colouring costs twelve bytes per entry, which is what keeps a
    /// million-line log affordable.
    #[test]
    fn a_recognised_message_takes_twelve_bytes() {
        assert_eq!(size_of::<RecognisedMessage>(), 12);
    }
}
