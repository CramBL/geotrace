use chrono::Duration;

use crate::parse::tests::fixtures::{self, REBOOT};
use crate::test_util;

/// A run of untimestamped lines opening a log is timestamped from the
/// first anchor after it, which is the anchor a pasted log is named after.
#[test]
fn the_first_anchored_timestamp_skips_the_interpolated_entries_before_it() {
    let parsed = fixtures::parsed_log("2026-05-23 10:00:00 first\nStack trace follows:\n");
    assert_eq!(
        parsed.first_anchored_timestamp(),
        Some(test_util::utc(2026, 5, 23, 10, 0, 0))
    );

    let leading_run = fixtures::parsed_log("Stack trace follows:\n2026-05-23 10:00:00 first\n");
    assert_eq!(
        leading_run.first_anchored_timestamp(),
        Some(test_util::utc(2026, 5, 23, 10, 0, 0)),
        "the interpolated entry before the anchor carries its timestamp, but does not anchor it"
    );
}

#[test]
fn a_run_of_untimestamped_lines_is_spread_between_its_anchors() {
    let parsed = fixtures::parsed_log(
        "2026-01-01 00:00:00 first\nStack trace follows:\n  at 0x0\n  ... omitted ...\n\
         2026-01-01 00:00:04 last\n",
    );
    assert_eq!(
        fixtures::timestamps(&parsed),
        [
            test_util::utc(2026, 1, 1, 0, 0, 0),
            test_util::utc(2026, 1, 1, 0, 0, 1),
            test_util::utc(2026, 1, 1, 0, 0, 2),
            test_util::utc(2026, 1, 1, 0, 0, 3),
            test_util::utc(2026, 1, 1, 0, 0, 4),
        ]
    );
}

/// One line between anchors a second apart lands half a second in.
#[test]
fn a_run_shorter_than_the_span_it_covers_keeps_sub_second_places() {
    let parsed = fixtures::parsed_log("2026-01-01 00:00:00 a\nmid\n2026-01-01 00:00:01 b\n");
    assert_eq!(
        fixtures::timestamps(&parsed).get(1),
        Some(&(test_util::utc(2026, 1, 1, 0, 0, 0) + Duration::milliseconds(500)))
    );
}

#[test]
fn a_run_at_the_edge_of_a_session_takes_the_one_anchor_it_has() {
    let parsed = fixtures::parsed_log(
        "before any anchor\n2026-01-01 00:00:10 first\n2026-01-01 00:00:20 last\n\
         after the last anchor\n",
    );
    assert_eq!(
        fixtures::timestamps(&parsed),
        [
            test_util::utc(2026, 1, 1, 0, 0, 10),
            test_util::utc(2026, 1, 1, 0, 0, 10),
            test_util::utc(2026, 1, 1, 0, 0, 20),
            test_util::utc(2026, 1, 1, 0, 0, 20),
        ]
    );
}

/// No run is ever spread across a reboot: the device clock restarts there.
#[test]
fn interpolation_never_crosses_a_boot_boundary() {
    let parsed = fixtures::parsed_log(&format!(
        "2026-01-01 00:00:00 before\nlast line of boot 1\n{REBOOT}\
         first line of boot 2\n2026-01-01 00:10:00 after\n"
    ));
    assert_eq!(
        fixtures::timestamps(&parsed),
        [
            test_util::utc(2026, 1, 1, 0, 0, 0),
            test_util::utc(2026, 1, 1, 0, 0, 0),
            test_util::utc(2026, 1, 1, 0, 10, 0),
            test_util::utc(2026, 1, 1, 0, 10, 0),
        ]
    );
}

#[test]
fn a_session_no_line_of_which_anchored_takes_the_anchor_before_it() {
    let parsed = fixtures::parsed_log(&format!(
        "2026-01-01 00:00:00 boot one\n{REBOOT}nothing timestamped here\n{REBOOT}\
         2026-01-01 00:10:00 boot three\n"
    ));
    assert_eq!(
        fixtures::timestamps(&parsed),
        [
            test_util::utc(2026, 1, 1, 0, 0, 0),
            test_util::utc(2026, 1, 1, 0, 0, 0),
            test_util::utc(2026, 1, 1, 0, 10, 0),
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
