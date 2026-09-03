//! Reading the parts of an entry's message a reader scans for: the service
//! that logged the line, the level it logged at, and the host it came from.
//!
//! The known sources are a device's own journald export, whose messages open
//! with the service, and `journalctl`, whose messages open with the host.

use std::{
    num::{NonZeroU8, NonZeroU16, NonZeroUsize},
    ops::Range,
};

/// Bytes a service name may take from where it starts. A message whose first
/// word runs past this carries no service.
const SERVICE_LIMIT_BYTES: usize = 64;

/// Bytes a hostname may take, past which the message opens with no host.
const HOSTNAME_LIMIT_BYTES: usize = 64;

/// Bytes a level token is looked for in, past any inner timestamp.
const LEVEL_TOKEN_HEAD_BYTES: usize = 40;

/// Bytes the longest inner timestamp takes:
/// `2026-09-02T18:33:31.123456789+02:00`.
const INNER_TIMESTAMP_LIMIT_BYTES: usize = 35;

/// Bytes after the service a level token is looked for in: an inner timestamp
/// and the token past it.
const LEVEL_HEAD_BYTES: usize = INNER_TIMESTAMP_LIMIT_BYTES + LEVEL_TOKEN_HEAD_BYTES;

/// The shapes the date and time of an inner timestamp take, `N` standing for a
/// digit and every other byte for itself: the ISO 8601 form a tracing
/// subscriber writes, and the slash form a Go service writes.
const DATE_TIME_SHAPES: [&[u8]; 2] = [b"NNNN-NN-NNTNN:NN:NN", b"NNNN/NN/NN NN:NN:NN"];

/// The digits and colon an inner timestamp writes after its zone sign, as in
/// `+02:00`.
const ZONE_OFFSET_SHAPE: &[u8] = b"NN:NN";

/// Digits of a second an inner timestamp's fraction is read to.
const FRACTION_LIMIT_DIGITS: usize = 9;

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

/// The level token of a message: a delimited `[WARN can::session_pool]` or
/// `<info>` to its closing delimiter, a bare `INFO:` to its colon, or the bare
/// uppercase `INFO` a tracing subscriber writes before its target.
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
pub fn recognise_message(message: &str, hostname_column: HostnameColumn) -> RecognisedMessage {
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
/// A service that timestamps its own messages writes that timestamp before the
/// level: the token is looked for past it.
///
/// The bare forms are read first: they can only stand where the token starts,
/// while a delimiter stating a level may follow one that does not.
fn level_token(bytes: &[u8], head_start: usize) -> Option<RecognisedLevel> {
    let from = past_spaces(bytes, head_start);
    let head_end = from.saturating_add(LEVEL_HEAD_BYTES).min(bytes.len());
    let token_start = past_spaces(bytes, inner_timestamp_end(bytes, from).unwrap_or(from));
    let head = bytes.get(token_start..head_end)?;
    if let Some(level) = bare_level(head, token_start) {
        return Some(level);
    }
    if let Some(level) = uppercase_word_level(head, token_start) {
        return Some(level);
    }
    // A delimiter opening in the head is read to its closing one, which may
    // stand past the head.
    memchr::memchr2_iter(b'[', b'<', head)
        .find_map(|open| delimited_level(bytes, token_start.saturating_add(open)))
}

/// The end of the timestamp a service wrote at `start` of its own message,
/// `None` where the bytes there are no date and time. The fraction and the
/// zone are optional: `2026-09-02T18:33:31.345324Z`, `2026/09/02 18:33:32`.
fn inner_timestamp_end(bytes: &[u8], start: usize) -> Option<usize> {
    let shape = DATE_TIME_SHAPES
        .into_iter()
        .find(|shape| matches_shape(bytes, start, shape))?;
    let after_fraction = fraction_end(bytes, start.saturating_add(shape.len()));
    Some(zone_end(bytes, after_fraction))
}

fn matches_shape(bytes: &[u8], start: usize, shape: &[u8]) -> bool {
    shape.iter().enumerate().all(|(offset, expected)| {
        bytes
            .get(start.saturating_add(offset))
            .is_some_and(|byte| match *expected {
                b'N' => byte.is_ascii_digit(),
                literal => *byte == literal,
            })
    })
}

/// The end of the `.345324` a timestamp writes after its seconds, `from` where
/// it writes none.
fn fraction_end(bytes: &[u8], from: usize) -> usize {
    if bytes.get(from) != Some(&b'.') {
        return from;
    }
    let digits = bytes
        .iter()
        .skip(from.saturating_add(1))
        .take(FRACTION_LIMIT_DIGITS)
        .take_while(|byte| byte.is_ascii_digit())
        .count();
    match digits {
        0 => from,
        digits => from.saturating_add(1).saturating_add(digits),
    }
}

/// The end of the `Z` or `+02:00` a timestamp writes after its seconds, `from`
/// where it writes neither.
fn zone_end(bytes: &[u8], from: usize) -> usize {
    let after_sign = from.saturating_add(1);
    match bytes.get(from) {
        Some(b'Z') => after_sign,
        Some(b'+' | b'-') if matches_shape(bytes, after_sign, ZONE_OFFSET_SHAPE) => {
            after_sign.saturating_add(ZONE_OFFSET_SHAPE.len())
        }
        _ => from,
    }
}

/// The `INFO:` a shell script writes, taken to its colon. `head` is the head
/// of the message, which starts at `token_start` in it.
fn bare_level(head: &[u8], token_start: usize) -> Option<RecognisedLevel> {
    let colon = memchr::memchr(b':', head)?;
    let kind = level_of(head.get(..colon)?)?;
    let end = token_start.saturating_add(colon).saturating_add(1);
    RecognisedLevel::at(token_start..end, kind)
}

/// The bare `INFO` a tracing subscriber writes before its target, taken to the
/// whitespace after it. Only an all-uppercase word states a level this way: a
/// message opening `Error reading /etc/conf` states none. `head` is the head of
/// the message, which starts at `token_start` in it.
fn uppercase_word_level(head: &[u8], token_start: usize) -> Option<RecognisedLevel> {
    let word_len = head.iter().position(u8::is_ascii_whitespace)?;
    let word = head.get(..word_len)?;
    if !word.iter().all(u8::is_ascii_uppercase) {
        return None;
    }
    let kind = level_of(word)?;
    RecognisedLevel::at(token_start..token_start.saturating_add(word_len), kind)
}

/// The `[WARN can::session_pool]` a Rust service writes or the `<info>`
/// NetworkManager writes, taken to its closing delimiter. A delimiter whose
/// first word states no level, or that never closes within
/// [`LEVEL_LIMIT_BYTES`], is no level token.
fn delimited_level(bytes: &[u8], open: usize) -> Option<RecognisedLevel> {
    let closing = match *bytes.get(open)? {
        b'[' => b']',
        b'<' => b'>',
        _ => return None,
    };
    let limit = open.saturating_add(LEVEL_LIMIT_BYTES).min(bytes.len());
    let delimited = bytes.get(open.saturating_add(1)..limit)?;
    let word_len = memchr::memchr2(b' ', closing, delimited)?;
    let kind = level_of(delimited.get(..word_len)?)?;
    let close = memchr::memchr(closing, delimited)?;
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
    use proptest::{prelude::*, prop_oneof, proptest};
    use rstest::rstest;

    use super::*;
    use crate::log_strategies;

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
    #[case::angle_bracketed(
        "NetworkManager[493]: <info>  [1788374011.6111] NetworkManager is starting",
        "<info>",
        Some(LogLevelKind::Info)
    )]
    #[case::angle_bracketed_outside_the_vocabulary(
        "yt-service[618]: <html> was returned",
        "",
        None
    )]
    #[case::upper_case_word("app: INFO fwu_backend: ready", "INFO", Some(LogLevelKind::Info))]
    #[case::a_capitalised_word_is_prose("app: Error reading /etc/conf", "", None)]
    #[case::a_vocabulary_word_mid_message("kernel: RCU Tasks Trace: Setting shift to 2", "", None)]
    #[case::a_word_pair_before_the_colon("yt-service[618]: TCP Error: TcpStreamClosed", "", None)]
    #[case::a_second_word_before_the_colon(
        "deno[532]: libEGL warning: failed to get driver name for fd -1",
        "",
        None
    )]
    #[case::a_kernel_subsystem_in_brackets("kernel: [drm] Initialized v3d 1.0.0", "", None)]
    #[case::a_dmesg_uptime_prefix(
        "kernel[440]: [    0.000000] Booting Linux on physical CPU 0x0",
        "",
        None
    )]
    #[case::a_dotted_word_in_brackets(
        "alsactl[438]: alsa-lib main.c:1804:(snd) [error.ucm] failed to import hw:0",
        "",
        None
    )]
    #[case::inner_iso_timestamp(
        "fwu-backend[446]: 2026-09-02T18:33:31.345324Z  INFO fwu_backend: Starting",
        "INFO",
        Some(LogLevelKind::Info)
    )]
    #[case::inner_iso_timestamp_to_the_second(
        "fwu-backend[446]: 2026-09-02T18:33:31 ERROR fwu_backend: no config",
        "ERROR",
        Some(LogLevelKind::Error)
    )]
    #[case::inner_slash_timestamp(
        "qbee-agent[555]: 2026/09/02 18:33:32 [INFO] Preparing agent directories",
        "[INFO]",
        Some(LogLevelKind::Info)
    )]
    #[case::a_date_that_is_no_timestamp(
        "home-persistent-clock[417]: Wed Sep  2 18:33:30 UTC 2026",
        "",
        None
    )]
    fn a_level_is_a_vocabulary_word_delimited_in_upper_case_or_before_a_colon(
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

    /// A token whose delimiter opens past the head window is no level, so no
    /// line costs more than a bounded look at its head.
    #[test]
    fn a_level_past_the_head_window_is_not_recognised() {
        let opening_on_the_last_read_byte = format!(
            "kernel: {} [WARN] still read",
            "0".repeat(LEVEL_HEAD_BYTES - 2)
        );
        let (_, _, level) = recognised(&opening_on_the_last_read_byte, HostnameColumn::Absent);
        assert_eq!(level, "[WARN]");

        let opening_past_the_window =
            format!("kernel: {} [WARN] not read", "0".repeat(LEVEL_HEAD_BYTES));
        let (_, _, level) = recognised(&opening_past_the_window, HostnameColumn::Absent);
        assert_eq!(level, "");
    }

    /// The window is wide enough for the longest timestamp a service writes
    /// before its level, with its fraction to the nanosecond and its zone.
    #[test]
    fn the_longest_inner_timestamp_is_read_past_to_the_level() {
        let longest = "2026-09-02T18:33:31.123456789+02:00";
        assert_eq!(longest.len(), INNER_TIMESTAMP_LIMIT_BYTES);

        let message = format!("fwu-backend[446]: {longest} WARN fwu_backend: slow");
        let (_, _, level) = recognised(&message, HostnameColumn::Absent);
        assert_eq!(level, "WARN");
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

    proptest! {
        /// Whatever a line of a dropped file holds, every span read from it
        /// slices the message, and each stands where the layout puts it: the
        /// host before the service, the level at or after the end of it.
        #[test]
        fn every_span_of_any_message_slices_it_in_layout_order(
            message in log_strategies::any_message(),
            hostname_column in prop_oneof![
                Just(HostnameColumn::Present),
                Just(HostnameColumn::Absent)
            ],
        ) {
            let read = recognise_message(&message, hostname_column);
            let hostname = read.hostname();
            let service = read.service().map(RecognisedService::span);
            let level = read.level().map(RecognisedLevel::span);

            let spans = [hostname.clone(), service.clone(), level.clone()];
            for span in spans.into_iter().flatten() {
                prop_assert!(message.get(span).is_some());
            }
            if let (Some(hostname), Some(service)) = (&hostname, &service) {
                prop_assert!(hostname.end < service.start);
            }
            if let (Some(service), Some(level)) = (&service, &level) {
                prop_assert!(level.start >= service.end);
            }
            if let Some(name) = service.and_then(|span| message.get(span)) {
                prop_assert!(!name.contains(' '));
                prop_assert!(name.len() <= SERVICE_LIMIT_BYTES);
            }
            if hostname_column == HostnameColumn::Absent {
                prop_assert_eq!(hostname, None);
            }
        }
    }
}
