//! Journald-shaped log text, generated deterministically for log-parser tests
//! and benchmarks.

use std::fmt::Write as _;

use chrono::{DateTime, Datelike as _, Duration, Timelike as _};

/// How much text [`synthetic_journald_log`] writes, and the seed deciding what
/// it writes.
#[derive(Debug, Clone, Copy)]
pub struct SyntheticLogSpec {
    pub approx_bytes: usize,
    pub seed: u64,
}

/// 2026-05-29 18:48:25 UTC, the moment the first generated line is logged at.
const FIRST_LINE_UNIX_SECS: i64 = 1_780_080_505;

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
    "device-plus[770]",
    "sshd[1032]",
    "dbus-daemon[401]",
];

const MESSAGES: [&str; 10] = [
    "Booting Linux on physical CPU 0x0",
    "Memory policy: Data cache writealloc",
    "gnss: fix acquired, 9 satellites in view",
    "can0: bus-off state entered, restarting",
    "Started Network Time Synchronization.",
    "wlan0: authenticate with 7c:10:c9:2f:aa:01",
    "device-plus: uploaded 2 recordings, queue empty",
    "Failed to start Modem Manager, retrying in 30s",
    "systemd-journald: file /var/log/journal rotated",
    "usb 1-1: new high-speed USB device number 4",
];

/// Tails that stretch a line out to the widths a real journal shows.
const MESSAGE_TAILS: [&str; 6] = [
    " (state=0x1f flags=0x00c0)",
    " [ 0x0000c3f4 0x0000c410 0x0000c44c ]",
    " uid=0 pid=770 comm=device-plus exe=/usr/bin/device-plus",
    " rc=-110 retries=3 backoff=250ms",
    " ttyS1 115200n8 rts/cts off dtr on",
    "",
];

/// Lines carrying no timestamp: the journal export's own separators and the
/// continuation lines of anything that logs more than one line at a time.
const UNTIMED_LINES: [&str; 4] = [
    "",
    "Stack trace follows:",
    "  at 0x0000c3f4 in gnss_task+0x54",
    "  ... 3 frames omitted ...",
];

/// One in this many lines carries no timestamp.
const UNTIMED_LINE_ONE_IN: usize = 50;

/// One in this many lines is a reboot marker, after which the clock restarts
/// behind where it left off, as an unsynchronised device's does.
const REBOOT_ONE_IN: usize = 512;

const REBOOT_MARKER: &str = "--- Device reboot ---";

/// Seconds a reboot can set the clock back by.
const MAX_REBOOT_CLOCK_STEP_BACK_SECS: usize = 60;

/// Syslog-short text of about [`SyntheticLogSpec::approx_bytes`], shaped like a
/// journald export from an embedded device: several lines per second, message
/// widths from a few characters to a few hundred, lines without a timestamp,
/// and reboots that send the clock backwards.
pub fn synthetic_journald_log(spec: SyntheticLogSpec) -> String {
    let SyntheticLogSpec { approx_bytes, seed } = spec;
    let mut rng = DeterministicRng::new(seed);
    let mut text = String::with_capacity(approx_bytes);
    let mut time =
        DateTime::from_timestamp(FIRST_LINE_UNIX_SECS, 0).unwrap_or(DateTime::UNIX_EPOCH);

    while text.len() < approx_bytes {
        if rng.below(REBOOT_ONE_IN) == 0 {
            text.push_str(REBOOT_MARKER);
            text.push('\n');
            let step_back = rng.below(MAX_REBOOT_CLOCK_STEP_BACK_SECS);
            time -= Duration::seconds(i64::try_from(step_back).unwrap_or(0));
            continue;
        }
        if rng.below(UNTIMED_LINE_ONE_IN) == 0 {
            text.push_str(rng.pick_one_of(&UNTIMED_LINES));
            text.push('\n');
            continue;
        }

        let step = rng.below(MAX_LINE_INTERVAL_MILLIS);
        time += Duration::milliseconds(i64::try_from(step).unwrap_or(0));
        let month = MONTH_ABBREVS
            .get(time.month0() as usize)
            .copied()
            .unwrap_or("Jan");
        write!(
            text,
            "{month} {day:2} {hour:02}:{minute:02}:{second:02} {unit}: {message}{tail}",
            day = time.day(),
            hour = time.hour(),
            minute = time.minute(),
            second = time.second(),
            unit = rng.pick_one_of(&UNITS),
            message = rng.pick_one_of(&MESSAGES),
            tail = rng.pick_one_of(&MESSAGE_TAILS),
        )
        .ok();
        text.push('\n');
    }

    text
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
        };
        assert_eq!(synthetic_journald_log(spec), synthetic_journald_log(spec));
        assert_ne!(
            synthetic_journald_log(spec),
            synthetic_journald_log(SyntheticLogSpec { seed: 4, ..spec })
        );
    }

    #[test]
    fn a_generated_log_fills_the_requested_size_with_journald_shaped_lines() {
        let approx_bytes = 256 * 1024;
        let text = synthetic_journald_log(SyntheticLogSpec {
            approx_bytes,
            seed: 1,
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
    }
}
