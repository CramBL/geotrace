//! Journald-shaped log text, generated deterministically for log-parser tests
//! and benchmarks.

use std::fmt::{self, Write as _};

use chrono::{DateTime, Datelike as _, Duration, Timelike as _, Utc};

/// How much text [`synthetic_journald_log`] writes, the seed deciding what it
/// writes, and the timestamp form it writes the lines in.
#[derive(Debug, Clone, Copy)]
pub struct SyntheticLogSpec {
    pub approx_bytes: usize,
    pub seed: u64,
    pub timestamps: SyntheticLogTimestamps,
}

/// The timestamp form the generated lines carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SyntheticLogTimestamps {
    /// The year-less short form journald's own export writes, whose year the
    /// parser infers from the clock.
    SyslogShort,

    /// ISO 8601 with a space separator, carrying the year: the form a test
    /// whose expectations name absolute times needs.
    Iso8601Space,
}

/// 2026-05-29 18:48:25 UTC, the moment the first generated line is logged at.
const FIRST_LINE_UNIX_SECS: i64 = 1_780_080_505;

/// The moment [`synthetic_journald_log`] logs its first line at, for a test
/// building a recording that runs alongside the generated log.
pub fn synthetic_log_start() -> DateTime<Utc> {
    DateTime::from_timestamp(FIRST_LINE_UNIX_SECS, 0).unwrap_or(DateTime::UNIX_EPOCH)
}

/// Milliseconds one line can advance the clock, so several lines share the
/// second that the syslog-short format records them at.
const MAX_LINE_INTERVAL_MILLIS: usize = 400;

const MONTH_ABBREVS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

const UNITS: [&str; 8] = [
    "kernel",
    "systemd[1]",
    "gpsd[412]",
    "NetworkManager[588]",
    "connmand[431]",
    "navsyncd[770]",
    "sshd[1032]",
    "dbus-daemon[401]",
];

const MESSAGES: [&str; 10] = [
    "Booting Linux on physical CPU 0x0",
    "Memory policy: Data cache writealloc",
    "gnss: fix acquired, 9 satellites in view",
    "can0: bus-off state entered, restarting",
    "Started Network Time Synchronization.",
    "wlan0: authenticate with 02:00:5e:2f:aa:01",
    "navsyncd: uploaded 2 recordings, queue empty",
    "Failed to start Modem Manager, retrying in 30s",
    "systemd-journald: file /var/log/journal rotated",
    "usb 1-1: new high-speed USB device number 4",
];

/// Tails that stretch a line out to the widths a real journal shows.
const MESSAGE_TAILS: [&str; 6] = [
    " (state=0x1f flags=0x00c0)",
    " [ 0x0000c3f4 0x0000c410 0x0000c44c ]",
    " uid=0 pid=770 comm=navsyncd exe=/usr/bin/navsyncd",
    " rc=-110 retries=3 backoff=250ms",
    " ttyS1 115200n8 rts/cts off dtr on",
    "",
];

/// Lines carrying no timestamp: blank lines and the continuation lines of
/// anything that logs more than one line at a time.
const UNTIMED_LINES: [&str; 4] = [
    "",
    "Stack trace follows:",
    "  at 0x0000c3f4 in gnss_task+0x54",
    "  ... 3 frames omitted ...",
];

/// One in this many lines opens a run of lines carrying no timestamp.
const UNTIMED_LINE_ONE_IN: usize = 50;

/// Lines one such run can be long, covering both runs and lone lines.
const MAX_UNTIMED_RUN_LINES: usize = 3;

/// One in this many lines is a reboot marker, after which the clock restarts
/// behind where it left off, as it does on a device that has not set its clock
/// yet.
const REBOOT_ONE_IN: usize = 512;

const REBOOT_MARKER: &str = "--- Device reboot ---";

/// Seconds a reboot can set the clock back by.
const MAX_REBOOT_CLOCK_STEP_BACK_SECS: usize = 60;

/// One in this many lines is preceded by a clock correction: the device syncs
/// against a time server mid-session and journald says so on the next line.
const CLOCK_ADJUSTMENT_ONE_IN: usize = 1024;

/// Seconds a mid-session clock correction can set the clock back by, always
/// larger than [`MAX_LINE_INTERVAL_MILLIS`]: every correction steps the clock
/// backwards.
const MAX_ADJUSTMENT_CLOCK_STEP_BACK_SECS: usize = 30;

const CLOCK_ADJUSTMENT_UNIT: &str = "systemd-journald";

const CLOCK_ADJUSTMENT_MESSAGE: &str = "Time jumped backwards, rotating.";

const SUMMARY_BLOCK_HEADER: &str = "----------- Journal summary -----------";

const SUMMARY_DEVICE_TYPE: &str = "nav-devkit-mk2";

/// `Fri 29-May-2026 18:48:25 UTC`, how the exporter writes the log's span.
const SUMMARY_TIME_FORMAT: &str = "%a %d-%b-%Y %H:%M:%S UTC";

/// `2026-05-29 18:48:25`, the ISO form carrying the year.
const ISO_8601_SPACE_FORMAT: &str = "%Y-%m-%d %H:%M:%S";

/// The head of the per-service tables of a real export.
const SUMMARY_ERROR_ROWS: [(&str, u32); 3] =
    [("hal-powerd", 56429), ("ofonod", 1092), ("hal-gnssd", 1027)];

const SUMMARY_WARNING_ROWS: [(&str, u32); 2] = [("core-appd", 29562), ("kernel", 315)];

/// Syslog-short text of about [`SyntheticLogSpec::approx_bytes`], shaped like a
/// journald export from an embedded device: several lines per second, message
/// widths from a few characters to a few hundred, runs of lines without a
/// timestamp, reboots that send the clock backwards, mid-session clock
/// corrections that say so, and the exporter's trailing summary block.
pub fn synthetic_journald_log(spec: SyntheticLogSpec) -> String {
    let SyntheticLogSpec {
        approx_bytes,
        seed,
        timestamps,
    } = spec;
    let mut rng = DeterministicRng::new(seed);
    let mut text = String::with_capacity(approx_bytes);
    let mut time = synthetic_log_start();
    let mut first_entry_time = None;
    let mut last_entry_time = time;
    let mut entry_count: u64 = 0;

    while text.len() < approx_bytes {
        if rng.below(REBOOT_ONE_IN) == 0 {
            text.push_str(REBOOT_MARKER);
            text.push('\n');
            time -= rng.seconds_below(MAX_REBOOT_CLOCK_STEP_BACK_SECS);
            continue;
        }
        if rng.below(UNTIMED_LINE_ONE_IN) == 0 {
            for _ in 0..=rng.below(MAX_UNTIMED_RUN_LINES) {
                text.push_str(rng.pick_one_of(&UNTIMED_LINES));
                text.push('\n');
            }
            continue;
        }

        let corrected_clock = rng.below(CLOCK_ADJUSTMENT_ONE_IN) == 0;
        if corrected_clock {
            time -= rng.seconds_below(MAX_ADJUSTMENT_CLOCK_STEP_BACK_SECS) + Duration::seconds(1);
        } else {
            time += Duration::milliseconds(
                i64::try_from(rng.below(MAX_LINE_INTERVAL_MILLIS)).unwrap_or(0),
            );
        }

        let unit = rng.pick_one_of(&UNITS);
        let message = rng.pick_one_of(&MESSAGES);
        let tail = rng.pick_one_of(&MESSAGE_TAILS);
        write_entry(
            &mut text,
            timestamps,
            time,
            format_args!("{unit}: {message}{tail}"),
        );
        if corrected_clock {
            write_entry(
                &mut text,
                timestamps,
                time,
                format_args!("{CLOCK_ADJUSTMENT_UNIT}: {CLOCK_ADJUSTMENT_MESSAGE}"),
            );
            entry_count += 1;
        }
        first_entry_time.get_or_insert(time);
        last_entry_time = time;
        entry_count += 1;
    }

    if let Some(logs_begin_at) = first_entry_time {
        write_summary_block(
            &mut text,
            &SummaryFigures {
                logs_begin_at,
                logs_end_at: last_entry_time,
                entry_count,
            },
        );
    }
    text
}

/// Writes one line: the moment `time` names in the form `timestamps` selects,
/// then `message`.
fn write_entry(
    text: &mut String,
    timestamps: SyntheticLogTimestamps,
    time: DateTime<Utc>,
    message: fmt::Arguments<'_>,
) {
    match timestamps {
        SyntheticLogTimestamps::SyslogShort => {
            let month = MONTH_ABBREVS
                .get(time.month0() as usize)
                .copied()
                .unwrap_or("Jan");
            writeln!(
                text,
                "{month} {day:2} {hour:02}:{minute:02}:{second:02} {message}",
                day = time.day(),
                hour = time.hour(),
                minute = time.minute(),
                second = time.second(),
            )
            .ok();
        }
        SyntheticLogTimestamps::Iso8601Space => {
            writeln!(text, "{} {message}", time.format(ISO_8601_SPACE_FORMAT)).ok();
        }
    }
}

/// What the exporter states about the log it wrote.
struct SummaryFigures {
    logs_begin_at: DateTime<Utc>,
    logs_end_at: DateTime<Utc>,

    /// Timestamped lines only, as the exporter counts journal entries.
    entry_count: u64,
}

fn write_summary_block(
    text: &mut String,
    &SummaryFigures {
        logs_begin_at,
        logs_end_at,
        entry_count,
    }: &SummaryFigures,
) {
    writeln!(text, "{SUMMARY_BLOCK_HEADER}").ok();
    writeln!(text, "Device type: {SUMMARY_DEVICE_TYPE}").ok();
    writeln!(
        text,
        "Logs begin at: {}",
        logs_begin_at.format(SUMMARY_TIME_FORMAT)
    )
    .ok();
    writeln!(
        text,
        "Logs end at  : {}",
        logs_end_at.format(SUMMARY_TIME_FORMAT)
    )
    .ok();
    writeln!(text, "Log entries: {entry_count}").ok();
    writeln!(text, "--- Service error count ---").ok();
    for (service, count) in SUMMARY_ERROR_ROWS {
        writeln!(text, "{service:20} -> {count} Errors").ok();
    }
    writeln!(text, "--- Service warning count ---").ok();
    for (service, count) in SUMMARY_WARNING_ROWS {
        writeln!(text, "{service:20} -> {count} Warnings").ok();
    }
}

/// A fixture is the same text on every machine and every run: SplitMix64 is a
/// fixed, portable generator.
struct DeterministicRng {
    state: u64,
}

impl DeterministicRng {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn next_u64(&mut self) -> u64 {
        self.state = self.state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut mixed = self.state;
        mixed = (mixed ^ (mixed >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        mixed = (mixed ^ (mixed >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        mixed ^ (mixed >> 31)
    }

    fn below(&mut self, exclusive_max: usize) -> usize {
        let drawn = self.next_u64();
        match u64::try_from(exclusive_max) {
            Ok(0) | Err(_) => 0,
            Ok(max) => usize::try_from(drawn % max).unwrap_or(0),
        }
    }

    fn seconds_below(&mut self, exclusive_max_secs: usize) -> Duration {
        Duration::seconds(i64::try_from(self.below(exclusive_max_secs)).unwrap_or(0))
    }

    fn pick_one_of<'options>(&mut self, options: &[&'options str]) -> &'options str {
        let index = self.below(options.len());
        options.get(index).copied().unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_generated_log_is_the_same_text_for_the_same_seed() {
        let spec = SyntheticLogSpec {
            approx_bytes: 8 * 1024,
            seed: 3,
            timestamps: SyntheticLogTimestamps::SyslogShort,
        };
        assert_eq!(synthetic_journald_log(spec), synthetic_journald_log(spec));
        assert_ne!(
            synthetic_journald_log(spec),
            synthetic_journald_log(SyntheticLogSpec { seed: 4, ..spec })
        );
    }

    /// The ISO form carries the year, so a test naming absolute times reads the
    /// same log whenever it runs.
    #[test]
    fn the_iso_form_dates_every_entry_it_writes() {
        let text = synthetic_journald_log(SyntheticLogSpec {
            approx_bytes: 8 * 1024,
            seed: 1,
            timestamps: SyntheticLogTimestamps::Iso8601Space,
        });
        assert_eq!(
            text.lines().next(),
            Some(
                "2026-05-29 18:48:25 systemd[1]: systemd-journald: file /var/log/journal rotated \
                 rc=-110 retries=3 backoff=250ms"
            )
        );
    }

    #[test]
    fn a_generated_log_fills_the_requested_size_with_journald_shaped_lines() {
        let approx_bytes = 256 * 1024;
        let text = synthetic_journald_log(SyntheticLogSpec {
            approx_bytes,
            seed: 1,
            timestamps: SyntheticLogTimestamps::SyslogShort,
        });
        assert!(text.len() >= approx_bytes);

        let lines: Vec<&str> = text.lines().collect();
        let timestamped = lines
            .iter()
            .filter(|line| line.starts_with("May ") || line.starts_with("Apr "))
            .count();
        assert!(
            timestamped > lines.len() * 9 / 10,
            "{timestamped} of {} lines carry a timestamp",
            lines.len()
        );
        assert!(lines.iter().any(|line| line.starts_with("  at 0x")));
        assert!(lines.contains(&REBOOT_MARKER));
        assert!(lines.contains(&SUMMARY_BLOCK_HEADER));
        assert!(
            lines
                .iter()
                .any(|line| line.ends_with(CLOCK_ADJUSTMENT_MESSAGE)),
            "the clock is corrected mid-session at least once"
        );
        assert!(
            lines
                .iter()
                .any(|line| line.starts_with("  ") || line.is_empty()),
            "some lines carry no timestamp at all"
        );
        assert!(
            lines
                .windows(2)
                .any(|pair| pair.iter().all(|line| line.starts_with("  "))),
            "untimestamped lines come in runs, not only one at a time"
        );
    }
}
