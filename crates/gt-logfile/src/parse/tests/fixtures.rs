use std::num::NonZeroUsize;

use chrono::{DateTime, Utc};

use crate::parse::{self, ParsedLog, TimestampKind};
use crate::test_util;

pub const REBOOT: &str = "--- Device reboot ---\n";

pub fn now() -> DateTime<Utc> {
    test_util::utc(2026, 5, 23, 0, 0, 0)
}

pub fn parsed_log(text: &str) -> ParsedLog {
    parse::parse_log(text.into(), now()).expect("parses")
}

pub fn messages(parsed: &ParsedLog) -> Vec<&str> {
    parsed
        .entries()
        .iter()
        .map(|entry| parsed.message(entry))
        .collect()
}

pub fn timestamps(parsed: &ParsedLog) -> Vec<DateTime<Utc>> {
    parsed
        .entries()
        .iter()
        .map(|entry| entry.timestamp)
        .collect()
}

pub fn timestamp_kinds(parsed: &ParsedLog) -> Vec<TimestampKind> {
    parsed
        .entries()
        .iter()
        .map(|entry| entry.timestamp_kind)
        .collect()
}

pub fn chunk_bytes(bytes: usize) -> NonZeroUsize {
    NonZeroUsize::new(bytes).expect("positive chunk size")
}
