use gt_test_utils::log_fixtures;

use crate::parse;
use crate::parse::tests::fixtures;

/// Covers the structure passes across chunk bounds: thousands of entries,
/// reboots, untimestamped runs and a summary block, enough for the chunked
/// path to split all of them.
#[test]
fn a_chunked_parse_of_a_journald_sized_log_matches_the_one_chunk_parse() {
    let text = log_fixtures::syslog_journald_log(200 * 1024, 7);
    let chunk_target_bytes = fixtures::chunk_bytes(4 * 1024);
    assert!(
        parse::newline_aligned_chunks(&text, chunk_target_bytes).len() > 8,
        "the fixture has to span several chunks for this to compare the two paths"
    );

    let one_chunk = parse::parse_log_in_chunks_of(
        text.as_str().into(),
        fixtures::now(),
        fixtures::chunk_bytes(text.len().saturating_add(1)),
    );
    let chunked =
        parse::parse_log_in_chunks_of(text.as_str().into(), fixtures::now(), chunk_target_bytes);
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
