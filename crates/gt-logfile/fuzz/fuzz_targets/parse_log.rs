#![no_main]

use std::num::NonZeroUsize;

use chrono::DateTime;
use gt_logfile::{
    parse_log, parse_log_in_chunks_of, LogText, ParsedLog, RecognisedLevel, RecognisedService,
    TextSlice,
};
use libfuzzer_sys::fuzz_target;

/// A moment past every timestamp an input can state, so the year-less syslog
/// formats resolve the same way from one run to the next.
const NOW_UNIX_SECS: i64 = 4_102_444_800;

/// Bytes up to which the chunked path is run against the one-chunk path, which
/// costs a second parse of the input.
const CHUNKED_COMPARISON_LIMIT_BYTES: usize = 2048;

/// Small enough to cut a fuzz input into several chunks, so the chunk merge is
/// what the comparison drives.
const FUZZED_CHUNK_TARGET_BYTES: NonZeroUsize = match NonZeroUsize::new(64) {
    Some(bytes) => bytes,
    None => NonZeroUsize::MIN,
};

// Feed arbitrary bytes to the log parser. It must return `Ok`/`Err` and index
// what it read consistently, never panic or abort. Mirrors the properties
// gt-logfile's own `parse.rs` asserts over generated logs.
fuzz_target!(|data: &[u8]| {
    let now = DateTime::from_timestamp(NOW_UNIX_SECS, 0).unwrap_or(DateTime::UNIX_EPOCH);
    let text = LogText::decode_lossy(data);
    let Ok(parsed) = parse_log(text.clone(), now) else {
        return;
    };
    check_the_parse_indexes_what_it_read(&parsed);

    if data.len() <= CHUNKED_COMPARISON_LIMIT_BYTES {
        assert_eq!(
            Ok(parsed),
            parse_log_in_chunks_of(text, now, FUZZED_CHUNK_TARGET_BYTES),
            "the chunked parse disagrees with the one-chunk parse"
        );
    }
});

/// The text `slice` addresses, `None` where it reaches outside the log or
/// splits a character of it.
fn sliced(slice: TextSlice, text: &str) -> Option<&str> {
    let start = usize::try_from(slice.offset).ok()?;
    text.get(start..start.checked_add(usize::try_from(slice.len).ok()?)?)
}

fn check_the_parse_indexes_what_it_read(parsed: &ParsedLog) {
    let text = parsed.text();

    let mut previous_line_number = 0;
    for entry in parsed.entries() {
        assert!(
            entry.line_number > previous_line_number,
            "entry on line {} follows line {previous_line_number}",
            entry.line_number
        );
        previous_line_number = entry.line_number;
        let message = sliced(entry.message, text);
        assert_eq!(
            message,
            Some(parsed.message(entry)),
            "the message of line {} lies outside the log text",
            entry.line_number
        );
    }

    let mut previous_line_number = 0;
    for line in parsed.structural_lines() {
        assert!(
            line.line_number > previous_line_number,
            "structural line {} follows line {previous_line_number}",
            line.line_number
        );
        previous_line_number = line.line_number;
        assert!(
            sliced(line.text, text).is_some(),
            "structural line {} lies outside the log text",
            line.line_number
        );
    }

    let mut next_entry = 0;
    for session in parsed.boot_sessions() {
        assert_eq!(
            session.entry_range.start, next_entry,
            "boot {} opens past the entries of the boot before it",
            session.boot_number
        );
        assert!(session.entry_count() > 0, "a boot session without entries");
        next_entry = session.entry_range.end;
    }
    assert_eq!(
        next_entry,
        parsed.entries().len(),
        "the boot sessions leave entries out"
    );

    for (entry, recognised) in parsed.entries().iter().zip(parsed.recognised_messages()) {
        let message = parsed.message(entry);
        let spans = [
            recognised.hostname(),
            recognised.service().map(RecognisedService::span),
            recognised.level().map(RecognisedLevel::span),
        ];
        for span in spans.into_iter().flatten() {
            assert!(
                message.get(span.clone()).is_some(),
                "{span:?} lies outside the message of line {}",
                entry.line_number
            );
        }
    }
}
