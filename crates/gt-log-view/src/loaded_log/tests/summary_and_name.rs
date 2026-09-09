use crate::loaded_log::LoadedLog;
use crate::test_util;

#[test]
fn the_summary_names_the_format_the_counts_and_what_took_no_position() {
    let files = test_util::loaded(vec![test_util::recording_at(55.0, 10)]);
    let mut log = test_util::log_of(10);

    assert_eq!(
        log.parse_summary_line(),
        "ISO 8601 · 10 entries · 1 boot · 10 unassociated"
    );

    test_util::anchor_to(&mut log, &files, 0);

    assert_eq!(
        log.parse_summary_line(),
        "ISO 8601 · 10 entries · 1 boot · 0 unassociated"
    );
}

/// A log whose lines do not all carry a timestamp, across two boots.
#[test]
fn the_summary_states_how_many_entries_were_timestamped_from_their_neighbours() {
    let text = "\
2026-01-01 14:02:11 navsyncd: starting
  at 0x0000c3f4 in gnss_task+0x54
2026-01-01 14:02:13 navsyncd: fix acquired
--- Device reboot ---
2026-01-01 14:02:20 navsyncd: starting
";
    let parsed = gt_logfile::parse_log(text.into(), test_util::start()).expect("the log parses");
    let log = LoadedLog::new(
        Some("navsyncd.log".to_owned()),
        parsed,
        test_util::association_window(),
    );

    assert_eq!(
        log.parse_summary_line(),
        "ISO 8601 · 4 entries (1 interpolated) · 2 boots · 4 unassociated"
    );
}

#[test]
fn a_log_that_arrived_without_a_filename_is_named_after_its_first_entry() {
    let log = LoadedLog::new(
        None,
        test_util::parsed_log(3),
        test_util::association_window(),
    );
    assert_eq!(log.name(), "pasted 14:02:11");
}
