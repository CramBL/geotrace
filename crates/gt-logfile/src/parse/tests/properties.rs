use std::collections::HashMap;

use proptest::prelude::*;
use proptest::proptest;

use crate::format;
use crate::parse::tests::fixtures;
use crate::parse::{self, FORMAT_DETECTION_LINE_LIMIT, LogEntry, TextSlice};
use crate::recognise::{RecognisedLevel, RecognisedService};
use crate::structure::StructuralLineKind;
use crate::summary::EntryCountMismatch;
use crate::test_util::strategies;

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
        text in strategies::any_log_text(),
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
                .is_some_and(|format| format::parse_line(line, format, fixtures::now()).is_some())
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

        prop_assert_eq!(parse::parse_log(text.as_str().into(), fixtures::now()).is_ok(), anchored);
    }

    /// Whatever text a user drops on the app, every span the parse hands
    /// out slices the text or the message it was read from.
    #[test]
    fn every_span_of_a_parsed_log_slices_the_text_it_was_indexed_from(
        text in strategies::any_log_text(),
    ) {
        let Ok(parsed) = parse::parse_log(text.as_str().into(), fixtures::now()) else {
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
        text in strategies::any_log_text(),
    ) {
        let Ok(parsed) = parse::parse_log(text.as_str().into(), fixtures::now()) else {
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
        text in strategies::any_log_text(),
    ) {
        let Ok(parsed) = parse::parse_log(text.as_str().into(), fixtures::now()) else {
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
        text in strategies::any_log_text(),
    ) {
        let Ok(parsed) = parse::parse_log(text.as_str().into(), fixtures::now()) else {
            return Ok(());
        };

        for entry in parsed.entries() {
            let line = line_of(&text, entry.line_number);
            prop_assert_eq!(
                entry.is_anchored(),
                format::parse_line(line, parsed.format(), fixtures::now()).is_some(),
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
        text in strategies::any_log_text(),
    ) {
        let Ok(parsed) = parse::parse_log(text.as_str().into(), fixtures::now()) else {
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

    /// An anomaly identifies an anchored entry of a boot session with an
    /// anchored entry before it in that session: the step is measured
    /// between the two.
    #[test]
    fn every_order_anomaly_names_an_anchored_entry_of_a_boot_session(
        text in strategies::any_log_text(),
    ) {
        let Ok(parsed) = parse::parse_log(text.as_str().into(), fixtures::now()) else {
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
        text in strategies::any_log_text(),
    ) {
        let Ok(parsed) = parse::parse_log(text.as_str().into(), fixtures::now()) else {
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
        text in strategies::any_summarised_log(),
    ) {
        let Ok(parsed) = parse::parse_log(text.as_str().into(), fixtures::now()) else {
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
        text in strategies::any_log_text(),
        chunk_target_bytes in 1usize..64,
    ) {
        let one_chunk = parse::parse_log_in_chunks_of(
            text.as_str().into(),
            fixtures::now(),
            fixtures::chunk_bytes(text.len().saturating_add(1)),
        );
        let chunked = parse::parse_log_in_chunks_of(
            text.as_str().into(),
            fixtures::now(),
            fixtures::chunk_bytes(chunk_target_bytes),
        );
        prop_assert_eq!(one_chunk, chunked);
    }

    #[test]
    fn chunks_tile_the_log_text_and_break_only_after_a_newline(
        text in r"(\PC|\n){0,300}",
        chunk_target_bytes in 1usize..64,
    ) {
        let chunks = parse::newline_aligned_chunks(&text, fixtures::chunk_bytes(chunk_target_bytes));

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
