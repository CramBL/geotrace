//! Strategies generating what the crate's property tests read: an export
//! written in one of the four timestamp formats, the shapes the recogniser
//! reads out of a message, the exporter's summary block, and arbitrary noise.

use chrono::{DateTime, Utc};
use proptest::{prelude::*, prop_oneof, sample};
use strum::IntoEnumIterator as _;

use crate::{
    LogFormat, ServiceCount, SummaryBlock,
    structure::{REBOOT_SEPARATOR_LINE, SUMMARY_BLOCK_HEADER_LINE},
    summary::{
        DEVICE_TYPE_KEY, ENTRY_COUNT_KEY, ERROR_TABLE_HEADER, EXPORTER_TIME_FORMAT, LOGS_BEGIN_KEY,
        LOGS_END_KEY, SERVICE_COUNT_ARROW, WARNING_TABLE_HEADER,
    },
};

/// The span every generated moment lands in: 2020-01-01 to 2030-01-01.
const FIRST_MOMENT_UNIX_SECS: i64 = 1_577_836_800;
const LAST_MOMENT_UNIX_SECS: i64 = 1_893_456_000;

const MICROSECONDS_PER_SECOND: u32 = 1_000_000;
const NANOSECONDS_PER_MICROSECOND: u32 = 1_000;

/// Bytes the month abbreviation of a syslog timestamp takes.
const MONTH_ABBREV_BYTES: usize = 3;

/// Lines one generated log holds, before its summary block.
const MAX_GENERATED_LINES: usize = 24;

/// Rows one generated summary block lists per table.
const MAX_GENERATED_TABLE_ROWS: usize = 3;

/// Five words the level vocabulary knows, and one it does not.
const LEVEL_WORDS: [&str; 6] = ["ERROR", "WARN", "INFO", "DEBUG", "NOTICE", "BLOCK"];

/// The case a month abbreviation is written in: `journalctl` capitalises it
/// under `LC_TIME=C` and lower-cases it under some other locales.
#[derive(Debug, Clone, Copy)]
enum MonthCase {
    Capitalised,
    Lower,
    Upper,
}

impl MonthCase {
    fn applied_to(self, timestamp: &str) -> String {
        let Some((month, rest)) = timestamp.split_at_checked(MONTH_ABBREV_BYTES) else {
            return timestamp.to_owned();
        };
        match self {
            Self::Capitalised => timestamp.to_owned(),
            Self::Lower => format!("{}{rest}", month.to_ascii_lowercase()),
            Self::Upper => format!("{}{rest}", month.to_ascii_uppercase()),
        }
    }
}

/// One of the two per-service tables the exporter writes.
#[derive(Debug, Clone, Copy)]
enum ServiceTable {
    Errors,
    Warnings,
}

impl ServiceTable {
    fn header(self) -> &'static str {
        match self {
            Self::Errors => ERROR_TABLE_HEADER,
            Self::Warnings => WARNING_TABLE_HEADER,
        }
    }

    fn row_noun(self) -> &'static str {
        match self {
            Self::Errors => "Errors",
            Self::Warnings => "Warnings",
        }
    }
}

impl LogFormat {
    /// Writes `moment` the way a log of this format writes it. The syslog
    /// forms write no year, which [`crate::infer_year`] resolves on the way
    /// back.
    pub(crate) fn written_timestamp(self, moment: DateTime<Utc>) -> String {
        match self {
            Self::SyslogShort => moment.format("%b %e %H:%M:%S").to_string(),
            Self::SyslogShortMicro => moment.format("%b %e %H:%M:%S%.6f").to_string(),
            Self::Iso8601Space => moment.format("%Y-%m-%d %H:%M:%S").to_string(),
            Self::Iso8601T => moment.format("%Y-%m-%dT%H:%M:%SZ").to_string(),
        }
    }
}

/// A timestamp and the [`LogFormat`] that wrote it.
#[derive(Debug, Clone)]
pub(crate) struct GeneratedTimestamp {
    pub(crate) format: LogFormat,
    pub(crate) text: String,
}

/// A summary block and the figures its exporter states in it.
#[derive(Debug, Clone)]
pub(crate) struct GeneratedSummaryBlock {
    pub(crate) stated: SummaryBlock,
    pub(crate) text: String,
}

fn any_log_format() -> impl Strategy<Value = LogFormat> {
    sample::select(LogFormat::iter().collect::<Vec<_>>())
}

fn any_moment() -> impl Strategy<Value = DateTime<Utc>> {
    (
        FIRST_MOMENT_UNIX_SECS..LAST_MOMENT_UNIX_SECS,
        0u32..MICROSECONDS_PER_SECOND,
    )
        .prop_map(|(secs, micros)| {
            DateTime::from_timestamp(secs, micros * NANOSECONDS_PER_MICROSECOND)
                .unwrap_or(DateTime::UNIX_EPOCH)
        })
}

fn any_whole_second() -> impl Strategy<Value = DateTime<Utc>> {
    (FIRST_MOMENT_UNIX_SECS..LAST_MOMENT_UNIX_SECS)
        .prop_map(|secs| DateTime::from_timestamp(secs, 0).unwrap_or(DateTime::UNIX_EPOCH))
}

fn any_month_case() -> impl Strategy<Value = MonthCase> {
    prop_oneof![
        Just(MonthCase::Capitalised),
        Just(MonthCase::Lower),
        Just(MonthCase::Upper),
    ]
}

/// A timestamp in `format`, its month in any case where the format writes one.
fn any_timestamp_in(format: LogFormat) -> impl Strategy<Value = String> {
    (any_moment(), any_month_case()).prop_map(move |(moment, case)| {
        let written = format.written_timestamp(moment);
        match format {
            LogFormat::SyslogShort | LogFormat::SyslogShortMicro => case.applied_to(&written),
            LogFormat::Iso8601Space | LogFormat::Iso8601T => written,
        }
    })
}

pub(crate) fn any_timestamp() -> impl Strategy<Value = GeneratedTimestamp> {
    any_log_format().prop_flat_map(|format| {
        any_timestamp_in(format).prop_map(move |text| GeneratedTimestamp { format, text })
    })
}

/// The host `journalctl` writes before the service, or nothing where the log
/// has no hostname column.
fn any_hostname() -> impl Strategy<Value = String> {
    prop_oneof![Just(String::new()), r"[a-z][a-z0-9-]{0,8} "]
}

/// A service in every shape the recogniser reads a name from, and one it reads
/// none from.
fn any_service_token() -> impl Strategy<Value = String> {
    prop_oneof![
        r"[a-z][a-z0-9-]{0,10}:",
        r"[a-z][a-z0-9-]{0,10}\[[0-9]{1,6}\]:",
        r"\([a-z-]{1,12}\):",
        r"[a-z-]{1,10}\.sh:",
        r"[a-z_]{1,12}:",
        r"[a-z]{1,8}/[a-z]{1,8}@[a-z]{1,8}:",
        Just(String::new()),
    ]
}

/// The timestamp a service writes into its own message, before its level.
fn any_inner_timestamp() -> impl Strategy<Value = String> {
    prop_oneof![
        Just(String::new()),
        r"20[0-9]{2}-[01][0-9]-[0-3][0-9]T[0-2][0-9]:[0-5][0-9]:[0-5][0-9](\.[0-9]{1,9})?(Z|\+[01][0-9]:[0-5][0-9])? ",
        r"20[0-9]{2}/[01][0-9]/[0-3][0-9] [0-2][0-9]:[0-5][0-9]:[0-5][0-9] ",
    ]
}

fn any_level_word() -> impl Strategy<Value = String> {
    (sample::select(LEVEL_WORDS.as_slice()), any::<bool>()).prop_map(|(word, lower_case)| {
        match lower_case {
            true => word.to_ascii_lowercase(),
            false => word.to_owned(),
        }
    })
}

/// A level in each of the four forms a message states one in, and nothing.
fn any_level_token() -> impl Strategy<Value = String> {
    prop_oneof![
        (any_level_word(), r"[a-z_]{1,8}::[a-z_]{1,8}")
            .prop_map(|(word, target)| format!("[{word} {target}]")),
        any_level_word().prop_map(|word| format!("[{word}]")),
        any_level_word().prop_map(|word| format!("<{word}>")),
        any_level_word().prop_map(|word| format!("{word}:")),
        any_level_word(),
        Just(String::new()),
    ]
}

/// What a service logged past what the recogniser reads: printable ASCII, text
/// of any script, or a line of a file that is no log at all.
fn any_message_tail() -> impl Strategy<Value = String> {
    prop_oneof![r"[ -~]{0,48}", r"\PC{0,24}", any_noise_line()]
}

/// Any one line of text, which is what a file dropped on the app can hold.
fn any_noise_line() -> impl Strategy<Value = String> {
    r".*"
}

pub(crate) fn any_message() -> impl Strategy<Value = String> {
    prop_oneof![
        4 => (
            any_hostname(),
            any_service_token(),
            any_inner_timestamp(),
            any_level_token(),
            any_message_tail(),
        )
            .prop_map(|(hostname, service, inner_timestamp, level, tail)| format!(
                "{hostname}{service} {inner_timestamp}{level} {tail}"
            )),
        1 => any_noise_line(),
    ]
}

/// One line of a log written in `format`: an entry, a line the exporter wrote
/// without a timestamp, a blank line, or a reboot separator.
fn any_body_line(format: LogFormat) -> impl Strategy<Value = String> {
    prop_oneof![
        16 => (any_timestamp_in(format), any_message())
            .prop_map(|(timestamp, message)| format!("{timestamp} {message}")),
        4 => any_message(),
        1 => Just(String::new()),
        1 => Just(REBOOT_SEPARATOR_LINE.to_owned()),
    ]
}

/// A body line, or the header that opens the exporter's summary block and
/// turns every line after it into structure.
fn any_log_line(format: LogFormat) -> impl Strategy<Value = String> {
    prop_oneof![
        15 => any_body_line(format),
        1 => Just(SUMMARY_BLOCK_HEADER_LINE.to_owned()),
    ]
}

fn any_service_count() -> impl Strategy<Value = ServiceCount> {
    (r"[a-z][a-z0-9-]{0,10}", 0u64..100_000)
        .prop_map(|(service, count)| ServiceCount { service, count })
}

fn any_service_counts() -> impl Strategy<Value = Vec<ServiceCount>> {
    prop::collection::vec(any_service_count(), 0..MAX_GENERATED_TABLE_ROWS)
}

pub(crate) fn any_summary_block() -> impl Strategy<Value = GeneratedSummaryBlock> {
    (
        prop::option::of(r"[a-z0-9][a-z0-9-]{0,14}"),
        prop::option::of(any_whole_second()),
        prop::option::of(any_whole_second()),
        prop::option::of(0u64..1_000_000),
        any_service_counts(),
        any_service_counts(),
    )
        .prop_map(
            |(
                device_type,
                logs_begin_at,
                logs_end_at,
                entry_count,
                service_error_counts,
                service_warning_counts,
            )| {
                let stated = SummaryBlock {
                    device_type,
                    logs_begin_at,
                    logs_end_at,
                    entry_count,
                    service_error_counts,
                    service_warning_counts,
                };
                GeneratedSummaryBlock {
                    text: written_summary_block(&stated),
                    stated,
                }
            },
        )
}

fn written_summary_block(block: &SummaryBlock) -> String {
    let SummaryBlock {
        device_type,
        logs_begin_at,
        logs_end_at,
        entry_count,
        service_error_counts,
        service_warning_counts,
    } = block;
    let mut lines = vec![SUMMARY_BLOCK_HEADER_LINE.to_owned()];
    if let Some(device_type) = device_type {
        lines.push(format!("{DEVICE_TYPE_KEY}: {device_type}"));
    }
    if let Some(logs_begin_at) = logs_begin_at {
        lines.push(format!(
            "{LOGS_BEGIN_KEY}: {}",
            logs_begin_at.format(EXPORTER_TIME_FORMAT)
        ));
    }
    if let Some(logs_end_at) = logs_end_at {
        // The exporter pads its keys to one width, which the parse trims off.
        lines.push(format!(
            "{LOGS_END_KEY}  : {}",
            logs_end_at.format(EXPORTER_TIME_FORMAT)
        ));
    }
    if let Some(entry_count) = entry_count {
        lines.push(format!("{ENTRY_COUNT_KEY}: {entry_count}"));
    }
    write_service_table(&mut lines, ServiceTable::Errors, service_error_counts);
    write_service_table(&mut lines, ServiceTable::Warnings, service_warning_counts);
    lines.join("\n")
}

fn write_service_table(lines: &mut Vec<String>, table: ServiceTable, rows: &[ServiceCount]) {
    if rows.is_empty() {
        return;
    }
    lines.push(table.header().to_owned());
    let noun = table.row_noun();
    for ServiceCount { service, count } in rows {
        lines.push(format!(
            "{service:<20} {SERVICE_COUNT_ARROW} {count} {noun}"
        ));
    }
}

/// A whole log: an export in one timestamp format, or a file that is no log.
pub(crate) fn any_log_text() -> impl Strategy<Value = String> {
    prop_oneof![
        3 => any_exported_log(),
        1 => prop::collection::vec(any_noise_line(), 0..MAX_GENERATED_LINES)
            .prop_map(|lines| lines.join("\n")),
    ]
}

/// A log whose last lines are the exporter's summary block, with no header
/// line before it.
pub(crate) fn any_summarised_log() -> impl Strategy<Value = String> {
    any_log_format().prop_flat_map(|format| {
        (
            prop::collection::vec(any_body_line(format), 0..MAX_GENERATED_LINES),
            any_summary_block(),
        )
            .prop_map(|(lines, summary)| format!("{}\n{}\n", lines.join("\n"), summary.text))
    })
}

fn any_exported_log() -> impl Strategy<Value = String> {
    any_log_format().prop_flat_map(|format| {
        (
            prop::collection::vec(any_log_line(format), 0..MAX_GENERATED_LINES),
            prop::option::weighted(0.25, any_summary_block()),
            any::<bool>(),
        )
            .prop_map(|(lines, summary, trailing_newline)| {
                let mut text = lines.join("\n");
                if let Some(summary) = summary {
                    text.push('\n');
                    text.push_str(&summary.text);
                }
                if trailing_newline {
                    text.push('\n');
                }
                text
            })
    })
}
