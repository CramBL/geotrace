use rstest::rstest;

use crate::parse::HOSTNAME_DETECTION_LINE_LIMIT;
use crate::parse::tests::fixtures;
use crate::recognise::{HostnameColumn, RecognisedService};

/// The layout is the exporter's: the head of the log decides for every
/// line, and a log of both shapes takes the one most of its head has.
#[rstest]
#[case::journalctl(
    "2026-01-01 00:00:00 workstation systemd[1]: a\n2026-01-01 00:00:01 workstation kernel: b\n",
    HostnameColumn::Present
)]
#[case::device_export(
    "2026-01-01 00:00:00 systemd: a\n2026-01-01 00:00:01 kernel: b\n",
    HostnameColumn::Absent
)]
#[case::mostly_hosts(
    "2026-01-01 00:00:00 workstation systemd[1]: a\n2026-01-01 00:00:01 workstation kernel: b\n\
     2026-01-01 00:00:02 kernel: c\n",
    HostnameColumn::Present
)]
#[case::mostly_services(
    "2026-01-01 00:00:00 workstation systemd[1]: a\n2026-01-01 00:00:01 kernel: b\n\
     2026-01-01 00:00:02 kernel: c\n",
    HostnameColumn::Absent
)]
fn the_head_of_the_log_decides_whether_its_lines_name_a_host(
    #[case] text: &str,
    #[case] expected: HostnameColumn,
) {
    assert_eq!(fixtures::parsed_log(text).hostname_column(), expected);
}

/// The lines below the head are read the way the head decided, whatever
/// shape they have themselves.
#[test]
fn a_line_past_the_head_is_read_with_the_layout_the_head_decided() {
    let mut text =
        "2026-01-01 00:00:00 workstation systemd[1]: a\n".repeat(HOSTNAME_DETECTION_LINE_LIMIT);
    text.push_str("2026-01-01 00:00:01 kernel: past the head\n");
    let parsed = fixtures::parsed_log(&text);
    let last = parsed
        .recognised_messages()
        .last()
        .copied()
        .expect("the log has entries");

    assert_eq!(parsed.hostname_column(), HostnameColumn::Present);
    assert_eq!(
        last.hostname(),
        Some(0.."kernel:".len()),
        "the last line's own service is read as the host the layout expects there"
    );
    assert_eq!(last.service(), None);
}

/// A seventh service takes a seventh slot: the palette, not the parse,
/// decides which of them share a colour.
#[test]
fn every_service_of_a_log_takes_a_slot_of_its_own() {
    let text: String = ["a", "b", "c", "d", "e", "f", "g"]
        .iter()
        .map(|service| format!("2026-01-01 00:00:00 {service}: logged\n"))
        .collect();
    let parsed = fixtures::parsed_log(&text);

    assert_eq!(
        parsed.services_by_first_appearance().collect::<Vec<_>>(),
        ["a", "b", "c", "d", "e", "f", "g"]
    );
    assert_eq!(
        parsed
            .recognised_messages()
            .iter()
            .filter_map(|recognised| recognised.service())
            .map(RecognisedService::slot)
            .collect::<Vec<_>>(),
        [0, 1, 2, 3, 4, 5, 6]
    );
}
