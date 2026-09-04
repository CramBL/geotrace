//! What the recogniser reads out of the log sources the viewer is written for:
//! a device's own journald export, a workstation's `journalctl` under
//! `LC_TIME=C`, and the `journalctl` of a Raspberry Pi running a Yocto/BusyBox
//! image under the C locale.
//!
//! Every fixture under `tests/fixtures/` is an excerpt, with every name and
//! address in it replaced.

use chrono::{DateTime, TimeZone as _, Utc};

use gt_logfile::{
    HostnameColumn, LogLevelKind, LogParseError, ParsedLog, RecognisedService, parse_log,
};

const DEVICE_EXPORT: &str = include_str!("fixtures/device_journald.log");

const WORKSTATION_JOURNALCTL: &str = include_str!("fixtures/workstation_journalctl.log");

const PI_JOURNALCTL: &str = include_str!("fixtures/pi_journalctl.log");

/// After the last line of every fixture, so the year-less timestamps resolve
/// to the year the fixtures were written in.
fn now() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 9, 4, 0, 0, 0)
        .single()
        .unwrap_or_default()
}

fn parse(text: &str) -> Result<ParsedLog, LogParseError> {
    parse_log(text.into(), now())
}

/// The service of every entry, in file order, empty where the entry names none.
fn services(parsed: &ParsedLog) -> Vec<&str> {
    parsed
        .entries()
        .iter()
        .zip(parsed.recognised_messages())
        .map(|(entry, recognised)| {
            let message = parsed.message(entry);
            recognised
                .service()
                .map(RecognisedService::span)
                .and_then(|span| message.get(span))
                .unwrap_or_default()
        })
        .collect()
}

/// The level token of every entry that states one, with the severity it
/// states, in file order.
fn levels(parsed: &ParsedLog) -> Vec<(&str, LogLevelKind)> {
    parsed
        .entries()
        .iter()
        .zip(parsed.recognised_messages())
        .filter_map(|(entry, recognised)| {
            let message = parsed.message(entry);
            let level = recognised.level()?;
            Some((message.get(level.span())?, level.kind()))
        })
        .collect()
}

/// The hostname of every entry with one, in file order.
fn hostnames(parsed: &ParsedLog) -> Vec<&str> {
    parsed
        .entries()
        .iter()
        .zip(parsed.recognised_messages())
        .filter_map(|(entry, recognised)| {
            let message = parsed.message(entry);
            message.get(recognised.hostname()?)
        })
        .collect()
}

/// The service of every entry with one, with the palette slot it took,
/// in file order.
fn services_with_slots(parsed: &ParsedLog) -> Vec<(&str, u16)> {
    parsed
        .entries()
        .iter()
        .zip(parsed.recognised_messages())
        .filter_map(|(entry, recognised)| {
            let message = parsed.message(entry);
            let service = recognised.service()?;
            Some((message.get(service.span())?, service.slot()))
        })
        .collect()
}

/// The palette slot of every entry with a service, in file order.
fn slots(parsed: &ParsedLog) -> Vec<u16> {
    parsed
        .recognised_messages()
        .iter()
        .filter_map(|recognised| recognised.service().map(RecognisedService::slot))
        .collect()
}

#[test]
fn the_device_export_names_no_host_and_journalctl_names_one_on_every_line() {
    assert_eq!(
        parse(DEVICE_EXPORT)
            .expect("the device export parses")
            .hostname_column(),
        HostnameColumn::Absent
    );

    let journalctl = parse(WORKSTATION_JOURNALCTL).expect("the journalctl excerpt parses");
    assert_eq!(journalctl.hostname_column(), HostnameColumn::Present);
    assert_eq!(hostnames(&journalctl), ["workstation"; 11]);

    let pi = parse(PI_JOURNALCTL).expect("the pi journal excerpt parses");
    assert_eq!(pi.hostname_column(), HostnameColumn::Present);
    assert_eq!(hostnames(&pi), ["pi"; 68]);
}

/// Every service form the two sources write, from the plain `kernel:` to a
/// parenthesised name and a name carrying a process id.
#[test]
fn every_service_of_the_device_export_is_read_to_its_colon() {
    let parsed = parse(DEVICE_EXPORT).expect("the device export parses");

    assert_eq!(
        services(&parsed),
        [
            "kernel:",
            "kernel:",
            "kernel:",
            "systemd-journald:",
            "systemd:",
            "hal-pm:",
            "hal-update:",
            "hal-gnss:",
            "hal-gnss:",
            "(udev-worker):",
            "(chronyd):",
            "app:",
            "hal-modem:",
            "kernel:",
            "kernel:",
            "ofonod:",
            "save-time-state.sh:",
            "save-time-state.sh:",
            "No-service-id:",
            "hal-accel:",
            "hal-accel:",
            "hal-accel:",
            "hal-can:",
            "sshd:",
        ]
    );
}

#[test]
fn a_journalctl_service_is_the_word_after_the_host_and_keeps_its_process_id() {
    assert_eq!(
        services(&parse(WORKSTATION_JOURNALCTL).expect("the journalctl excerpt parses")),
        [
            "podman[2582462]:",
            "quirky_hopper[2582494]:",
            "systemd[1223]:",
            "podman[2582462]:",
            "kernel:",
            "systemd[1223]:",
            "systemd[1223]:",
            "kernel:",
            "sudo[2583120]:",
            "chronyd[1042]:",
            "systemd[1]:",
        ]
    );
}

/// Every level form the two sources write: bracketed with a target, bracketed
/// alone, and the bare word a shell script writes before a colon.
#[test]
fn every_level_of_the_device_export_is_read_with_its_severity() {
    let parsed = parse(DEVICE_EXPORT).expect("the device export parses");

    assert_eq!(
        levels(&parsed),
        [
            ("[INFO common::feature_flags]", LogLevelKind::Info),
            ("[INFO update::manager]", LogLevelKind::Info),
            ("[INFO gnss::service::run]", LogLevelKind::Info),
            ("[WARN gnss::aiding::time_store]", LogLevelKind::Warning),
            ("[WARNING]", LogLevelKind::Warning),
            ("[ERROR modem::manager::modem]", LogLevelKind::Error),
            ("INFO:", LogLevelKind::Info),
            ("DEBUG:", LogLevelKind::Debug),
            ("[WARN can::session_pool]", LogLevelKind::Warning),
        ]
    );
}

/// A kernel subsystem prefix, a firewall tag and a timestamp in brackets are
/// not levels: only the vocabulary is.
#[test]
fn a_bracket_or_prefix_outside_the_vocabulary_states_no_level() {
    assert_eq!(
        levels(&parse(WORKSTATION_JOURNALCTL).expect("the journalctl excerpt parses")),
        [("[WARN]", LogLevelKind::Warning)]
    );
}

#[test]
fn each_service_takes_a_slot_of_its_own_in_the_order_it_first_appears() {
    let parsed = parse(DEVICE_EXPORT).expect("the device export parses");

    assert_eq!(
        parsed.services_by_first_appearance().collect::<Vec<_>>(),
        [
            "kernel",
            "systemd-journald",
            "systemd",
            "hal-pm",
            "hal-update",
            "hal-gnss",
            "(udev-worker)",
            "(chronyd)",
            "app",
            "hal-modem",
            "ofonod",
            "save-time-state.sh",
            "No-service-id",
            "hal-accel",
            "hal-can",
            "sshd",
        ]
    );
    assert_eq!(
        slots(&parsed),
        [
            0, 0, 0, 1, 2, 3, 4, 5, 5, 6, 7, 8, 9, 0, 0, 10, 11, 11, 12, 13, 13, 13, 14, 15
        ],
        "every line of a service carries that service's slot"
    );
}

/// A service that logged under two process ids is one service: the slot
/// follows the name, so a restart does not recolour it.
#[test]
fn a_service_keeps_one_slot_across_the_process_ids_it_logs_under() {
    let parsed = parse(WORKSTATION_JOURNALCTL).expect("the journalctl excerpt parses");

    assert_eq!(
        parsed.services_by_first_appearance().collect::<Vec<_>>(),
        [
            "podman",
            "quirky_hopper",
            "systemd",
            "kernel",
            "sudo",
            "chronyd"
        ]
    );
    assert_eq!(slots(&parsed), [0, 1, 2, 0, 3, 2, 2, 3, 4, 5, 2]);
}

/// Every service form the pi image writes: parenthesised names, names carrying
/// a process id, a name ending in a digit, and one whose message is only a
/// date.
#[test]
fn every_service_of_the_pi_journal_is_read_to_its_colon() {
    let parsed = parse(PI_JOURNALCTL).expect("the pi journal excerpt parses");

    assert_eq!(
        parsed.services_by_first_appearance().collect::<Vec<_>>(),
        [
            "kernel",
            "systemd",
            "(udev-worker)",
            "home-persistent-clock",
            "(syslogd)",
            "alsactl",
            "avahi-daemon",
            "NetworkManager",
            "(bluetoothd)",
            "rauc",
            "fwu-backend",
            "sshd_check_keys",
            "nginx",
            "qbee-agent",
            "fife",
            "mpd",
            "sh",
            "mpc",
            "deno",
            "yt-service",
        ]
    );
}

/// Both `kernel:` and `kernel[440]:` take one colour: the slot follows the
/// service name, not the process id.
#[test]
fn a_service_keeps_one_slot_with_and_without_a_process_id() {
    let parsed = parse(PI_JOURNALCTL).expect("the pi journal excerpt parses");

    assert_eq!(
        services_with_slots(&parsed)
            .into_iter()
            .filter(|(service, _)| service.starts_with("kernel"))
            .collect::<Vec<_>>(),
        [
            ("kernel:", 0),
            ("kernel:", 0),
            ("kernel[440]:", 0),
            ("kernel[440]:", 0),
            ("kernel[440]:", 0),
        ]
    );
}

/// Every level form the pi image writes: NetworkManager's angle brackets, the
/// bare upper-case word a tracing subscriber writes before its target, and the
/// bracket a Go service writes. The last two follow a timestamp the service
/// wrote itself, which stays plain.
#[test]
fn every_level_of_the_pi_journal_is_read_past_the_timestamp_before_it() {
    let parsed = parse(PI_JOURNALCTL).expect("the pi journal excerpt parses");

    assert_eq!(
        levels(&parsed),
        [
            ("<info>", LogLevelKind::Info),
            ("<info>", LogLevelKind::Info),
            ("<info>", LogLevelKind::Info),
            ("INFO", LogLevelKind::Info),
            ("INFO", LogLevelKind::Info),
            ("INFO", LogLevelKind::Info),
            ("INFO", LogLevelKind::Info),
            ("[INFO]", LogLevelKind::Info),
            ("[INFO]", LogLevelKind::Info),
        ],
        "a dmesg uptime prefix, [drm], [error.ucm], [youtube], TCP Error:, \
         libEGL warning: and RCU Tasks Trace: state no level"
    );
}

/// The excerpt holds a clock jump from March to September inside one boot, at
/// the point the image read its persistent clock.
#[test]
fn the_pi_journal_parses_across_the_clock_jump_inside_its_boot() {
    let parsed = parse(PI_JOURNALCTL).expect("the pi journal excerpt parses");

    assert_eq!(parsed.entries().len(), 68);
    assert_eq!(parsed.anchored_entry_count(), 68);
    assert_eq!(
        parsed.entries().first().map(|entry| entry.timestamp),
        Utc.with_ymd_and_hms(2026, 3, 13, 15, 35, 10).single()
    );
    assert_eq!(
        parsed.entries().last().map(|entry| entry.timestamp),
        Utc.with_ymd_and_hms(2026, 9, 3, 20, 37, 11).single()
    );
}

/// The fixture is a whole export: what its summary block counts is what the
/// parse read.
#[test]
fn the_device_exports_summary_block_agrees_with_the_parse() {
    let parsed = parse(DEVICE_EXPORT).expect("the device export parses");

    assert_eq!(parsed.exporter_entry_count_mismatch(), None);
    assert_eq!(
        parsed
            .summary_block()
            .and_then(|block| block.device_type.clone()),
        Some("nav-devkit-mk2".to_owned())
    );
}
