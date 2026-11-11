#![no_main]

use std::num::NonZeroUsize;

use chrono::{DateTime, Datelike as _, Duration, NaiveDate, Utc};
use gt_logfile::{
    AnchoredBounds, ClockEligibility, EligibleLogTime, LogChronology, LogEntry, LogText, ParsedLog,
    RecognisedLevel, RecognisedService, TextSlice,
};
use libfuzzer_sys::fuzz_target;

// Feed arbitrary bytes to the log parser. It must return `Ok`/`Err` and index
// what it read consistently, never panic or abort. Mirrors the properties
// gt-logfile's own `parse.rs` asserts over generated logs.
fuzz_target!(|data: &[u8]| {
    if let [year_low, year_high, fraction_low, fraction_high, ..] = data {
        let reference_year = 1900 + i32::from(u16::from_le_bytes([*year_low, *year_high]) % 300);
        let fraction = u32::from(u16::from_le_bytes([*fraction_low, *fraction_high]));
        let reference = NaiveDate::from_ymd_opt(reference_year, 10, 1)
            .and_then(|date| date.and_hms_opt(0, 0, 0))
            .expect("valid reference date")
            .and_utc();
        let pair =
            format!("Feb 29 23:59:59.{fraction:06} first\nMar  1 00:00:00.{fraction:06} second\n");
        let parsed =
            gt_logfile::parse_log(pair.as_str().into(), reference).expect("valid leap-day pair");
        let timestamps = parsed
            .chronology()
            .resolved()
            .expect("the years of a leap-day pair resolve")
            .timestamps();
        let first = *timestamps.first().expect("first line");
        let last = *timestamps.last().expect("last line");
        assert_eq!(last - first, Duration::seconds(1));
        assert_eq!(first.timestamp_subsec_micros(), fraction);
        assert!(last <= reference);
        assert!(reference_year - first.year() <= 8);
    }
    let now = DateTime::from_timestamp(NOW_UNIX_SECS, 0).unwrap_or(DateTime::UNIX_EPOCH);
    let text = LogText::decode_lossy(data);
    for reference in [DateTime::<Utc>::MIN_UTC, DateTime::<Utc>::MAX_UTC] {
        if let Ok(parsed) = gt_logfile::parse_log(text.clone(), reference) {
            let Some(resolved) = parsed.chronology().resolved() else {
                continue;
            };
            let mut anchored_timestamps = parsed
                .entries()
                .iter()
                .zip(resolved.timestamps())
                .filter(|(entry, _)| entry.is_anchored())
                .map(|(_, timestamp)| *timestamp);
            for timestamp in anchored_timestamps.clone() {
                if let Some(inferred) = gt_logfile::infer_year(timestamp.naive_utc(), reference) {
                    let latest = reference
                        .checked_add_signed(Duration::hours(1))
                        .unwrap_or(reference);
                    assert!(inferred <= latest);
                }
            }
            let without_bounds = parsed.summary_block().is_none_or(|summary| {
                summary.logs_begin_at.is_none() && summary.logs_end_at.is_none()
            });
            if without_bounds && !parsed.format().writes_years() {
                let last = anchored_timestamps
                    .next_back()
                    .expect("a parsed log has an anchor");
                let latest = reference
                    .checked_add_signed(Duration::hours(1))
                    .unwrap_or(reference);
                assert!(last <= latest);
            }
        }
    }
    let Ok(parsed) = gt_logfile::parse_log(text.clone(), now) else {
        return;
    };
    check_the_parse_indexes_what_it_read(&parsed);
    if let LogChronology::Unresolved(_) = parsed.chronology() {
        assert!(
            !parsed.format().writes_years(),
            "a format that writes years left them unresolved"
        );
    }
    assert_eq!(
        gt_logfile::parse_log(text.clone(), parsed.year_reference()),
        Ok(parsed.clone()),
        "reusing the saved year reference changes the parse"
    );
    let syslog_summary = parsed
        .summary_block()
        .filter(|_| !parsed.format().writes_years());
    if let (Some(summary), Some(resolved)) = (syslog_summary, parsed.chronology().resolved()) {
        // Every anchored syslog entry has a timestamp without a year.
        // The summary block bounds the first and last of them, never the ones between.
        let mut anchored = parsed
            .entries()
            .iter()
            .zip(resolved.timestamps())
            .enumerate()
            .filter(|(_, (entry, _))| entry.is_anchored())
            .map(|(index, (_, timestamp))| (index, *timestamp));
        let (first_index, first) = anchored.next().expect("a parsed log has an anchor");
        let last = anchored.next_back().map_or(first, |(_, last)| last);
        let first_entry_has_a_synchronised_clock = !parsed
            .boot_sessions()
            .iter()
            .any(|session| session.unsynchronised_entries().contains(&first_index));
        let logs_begin_at = summary
            .logs_begin_at
            .filter(|_| first_entry_has_a_synchronised_clock);
        for timestamp in [first, last] {
            assert!(logs_begin_at.is_none_or(|begin| timestamp >= begin));
            assert!(summary
                .logs_end_at
                .is_none_or(|end| timestamp.timestamp() <= end.timestamp()));
        }
    }

    if data.len() <= CHUNKED_COMPARISON_LIMIT_BYTES {
        assert_eq!(
            Ok(parsed),
            gt_logfile::parse_log_in_chunks_of(text, now, FUZZED_CHUNK_TARGET_BYTES),
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

    match parsed.chronology() {
        LogChronology::Resolved(resolved) => {
            assert_eq!(
                resolved.timestamps().len(),
                parsed.entries().len(),
                "the resolved chronology has another timestamp count than entry count"
            );
            for (session_index, session) in parsed.boot_sessions().iter().enumerate() {
                for entry_index in session.entry_range.clone() {
                    let before_synchronization =
                        session.unsynchronised_entries().contains(&entry_index);
                    assert_eq!(
                        resolved.clock_eligibility(entry_index),
                        Some(if before_synchronization {
                            ClockEligibility::BeforeSynchronization
                        } else {
                            ClockEligibility::AllowAbsoluteTime
                        })
                    );
                    assert_eq!(
                        resolved
                            .eligible_time(entry_index)
                            .map(EligibleLogTime::timestamp),
                        resolved
                            .timestamp(entry_index)
                            .filter(|_| !before_synchronization),
                    );
                }
                let mut anchored = (session.first_synchronised_entry..session.entry_range.end)
                    .filter(|&index| {
                        parsed
                            .entries()
                            .get(index)
                            .is_some_and(LogEntry::is_anchored)
                    })
                    .filter_map(|index| resolved.timestamp(index));
                let first = anchored.next();
                let expected = first.map(|first| AnchoredBounds {
                    first,
                    last: anchored.next_back().unwrap_or(first),
                });
                assert_eq!(
                    resolved.session_bounds(session_index),
                    expected,
                    "boot {} has other bounds than its anchored entries with a synchronised clock",
                    session.boot_number
                );
            }
        }
        LogChronology::Unresolved(unresolved) => {
            for (index, entry) in parsed.entries().iter().enumerate() {
                assert_eq!(
                    unresolved.written_timestamp(index).is_some(),
                    entry.is_anchored(),
                    "the written timestamp of line {} disagrees with whether the line is anchored",
                    entry.line_number
                );
            }
        }
    }

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
