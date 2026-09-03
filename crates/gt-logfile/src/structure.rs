//! The lines a log carries that are not entries of their own, and the entries
//! that report a clock adjustment.

use crate::parse::TextSlice;

/// How a registry pattern is matched against one trimmed line.
#[derive(Debug, Clone, Copy)]
enum LineMatcher {
    Exact(&'static str),
    Contains(&'static str),
}

impl LineMatcher {
    fn matches(self, line: &str) -> bool {
        match self {
            Self::Exact(pattern) => line == pattern,
            Self::Contains(pattern) => line.contains(pattern),
        }
    }
}

/// The line a device's exporter writes where the device rebooted.
pub(crate) const REBOOT_SEPARATOR_LINE: &str = "--- Device reboot ---";

/// The line the exporter opens its trailing summary block with.
pub(crate) const SUMMARY_BLOCK_HEADER_LINE: &str = "----------- Journal summary -----------";

/// What a recognized non-entry line marks. Structural lines are kept out of the
/// entries: they carry the log's structure instead of its content.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StructuralLineKind {
    /// The boundary between two boot sessions.
    RebootSeparator,

    /// A line of the exporter's trailing summary block.
    SummaryBlock,
}

/// How far a structural pattern reaches past the line that matched it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StructuralExtent {
    OwnLine,
    ToEndOfLog,
}

struct StructuralPattern {
    matcher: LineMatcher,
    kind: StructuralLineKind,
    extent: StructuralExtent,
}

/// The exporter idioms the parser recognizes. Teaching it another one is one
/// row here. A line matching no row is read as an entry.
const STRUCTURAL_LINE_REGISTRY: &[StructuralPattern] = &[
    StructuralPattern {
        matcher: LineMatcher::Exact(REBOOT_SEPARATOR_LINE),
        kind: StructuralLineKind::RebootSeparator,
        extent: StructuralExtent::OwnLine,
    },
    StructuralPattern {
        matcher: LineMatcher::Exact(SUMMARY_BLOCK_HEADER_LINE),
        kind: StructuralLineKind::SummaryBlock,
        extent: StructuralExtent::ToEndOfLog,
    },
];

impl StructuralLineKind {
    pub(crate) fn matching_line(line: &str) -> Option<Self> {
        STRUCTURAL_LINE_REGISTRY
            .iter()
            .find(|pattern| pattern.matcher.matches(line))
            .map(|pattern| pattern.kind)
    }

    pub(crate) fn extent(self) -> StructuralExtent {
        STRUCTURAL_LINE_REGISTRY
            .iter()
            .find(|pattern| pattern.kind == self)
            .map_or(StructuralExtent::OwnLine, |pattern| pattern.extent)
    }
}

/// One recognized non-entry line and where its text sits in the log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StructuralLine {
    pub kind: StructuralLineKind,

    /// 1-based, counting every physical line of the log.
    pub line_number: u32,

    /// The whole line, without its indent and line ending.
    pub text: TextSlice,
}

/// What the system logs when it moves the clock. A backwards timestamp step
/// beside one of these is an intentional clock change, not an order anomaly.
/// These lines stay ordinary anchored entries.
const TIME_CHANGE_REGISTRY: &[LineMatcher] = &[
    LineMatcher::Contains("jumped backwards"), // systemd-journald, systemd-timesyncd
    LineMatcher::Contains("Time has been changed"), // systemd-timedated
    LineMatcher::Contains("Clock change detected"), // systemd-resolved
    LineMatcher::Contains("Synchronized to time server"), // systemd-timesyncd
    LineMatcher::Contains("Initial clock synchronization"), // systemd-timesyncd
    LineMatcher::Contains("System time before build time"), // systemd
    LineMatcher::Contains("setting system clock to"), // kernel RTC drivers
    LineMatcher::Contains("System clock wrong by"), // chronyd
    LineMatcher::Contains("System clock was stepped by"), // chronyd makestep
    LineMatcher::Contains("Backward time jump detected"), // chronyd
    LineMatcher::Contains("time reset "),      // ntpd clock step
    LineMatcher::Contains("adjusting local clock by"), // openntpd
    LineMatcher::Contains("setting time to"),  // busybox ntpd
];

pub(crate) fn reports_a_clock_adjustment(message: &str) -> bool {
    TIME_CHANGE_REGISTRY
        .iter()
        .any(|matcher| matcher.matches(message))
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;

    #[rstest]
    #[case::reboot("--- Device reboot ---", Some(StructuralLineKind::RebootSeparator))]
    #[case::summary_header(
        "----------- Journal summary -----------",
        Some(StructuralLineKind::SummaryBlock)
    )]
    #[case::unknown_separator("=== something else ===", None)]
    #[case::reboot_with_a_suffix("--- Device reboot --- again", None)]
    #[case::entry("May 29 18:48:25 kernel: Booting Linux", None)]
    fn a_line_is_structural_only_when_a_registry_pattern_matches_it_whole(
        #[case] line: &str,
        #[case] expected: Option<StructuralLineKind>,
    ) {
        assert_eq!(StructuralLineKind::matching_line(line), expected);
    }

    #[rstest]
    #[case::reboot(StructuralLineKind::RebootSeparator, StructuralExtent::OwnLine)]
    #[case::summary(StructuralLineKind::SummaryBlock, StructuralExtent::ToEndOfLog)]
    fn a_kind_reaches_as_far_as_its_registry_row_says(
        #[case] kind: StructuralLineKind,
        #[case] expected: StructuralExtent,
    ) {
        assert_eq!(kind.extent(), expected);
    }

    #[rstest]
    #[case::journald("systemd-journald: Time jumped backwards, rotating.", true)]
    #[case::timesyncd_restore(
        "systemd-timesyncd: System clock time unset or jumped backwards, restored from recorded timestamp: Fri 2026-05-29 14:16:23 UTC.",
        true
    )]
    #[case::timesyncd(
        "systemd-timesyncd: Initial clock synchronization to Fri 2026-05-29 13:58:58 UTC.",
        true
    )]
    #[case::kernel_rtc(
        "kernel: snvs_rtc 20cc000.snvs:snvs-rtc-lp: setting system clock to 2026-05-29T14:00:29 UTC (1780063229)",
        true
    )]
    #[case::chronyd_adjustment(
        "chronyd[610]: System clock wrong by 1.234567 seconds, adjustment started",
        true
    )]
    #[case::chronyd_step("chronyd[610]: System clock was stepped by -3600.000000 seconds", true)]
    #[case::chronyd_backward_jump("chronyd[610]: Backward time jump detected!", true)]
    #[case::ntpd_reset("ntpd[842]: time reset -0.812345 s", true)]
    #[case::openntpd("ntpd[842]: adjusting local clock by -1.204345s", true)]
    #[case::busybox_ntpd(
        "ntpd: setting time to 2026-05-29 14:00:29.123456 (offset +3600.000000s)",
        true
    )]
    #[case::ordinary_entry("gpsd[412]: gnss: fix acquired, 9 satellites in view", false)]
    fn a_clock_adjustment_is_recognized_by_the_message_it_logs(
        #[case] message: &str,
        #[case] expected: bool,
    ) {
        assert_eq!(reports_a_clock_adjustment(message), expected);
    }
}
