//! The exporter's trailing summary block, read into the figures it states.

use chrono::{DateTime, NaiveDateTime, Utc};

const DEVICE_TYPE_KEY: &str = "Device type";
const LOGS_BEGIN_KEY: &str = "Logs begin at";
const LOGS_END_KEY: &str = "Logs end at";
const ENTRY_COUNT_KEY: &str = "Log entries";
const ERROR_TABLE_HEADER: &str = "--- Service error count ---";
const WARNING_TABLE_HEADER: &str = "--- Service warning count ---";
const SERVICE_COUNT_ARROW: &str = "->";

/// `Thu 29-May-2025 18:48:25 UTC`, how the exporter writes the log's span.
const EXPORTER_TIME_FORMAT: &str = "%a %d-%b-%Y %H:%M:%S UTC";

/// What the exporter says about the log it wrote. Every field is optional: a
/// truncated block yields whatever it got as far as stating.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SummaryBlock {
    pub device_type: Option<String>,
    pub logs_begin_at: Option<DateTime<Utc>>,
    pub logs_end_at: Option<DateTime<Utc>>,

    /// Entries the exporter counted, to be held against what the parse indexed.
    pub entry_count: Option<u64>,

    /// In the order the exporter listed them.
    pub service_error_counts: Vec<ServiceCount>,

    /// In the order the exporter listed them.
    pub service_warning_counts: Vec<ServiceCount>,
}

/// One row of the summary block's per-service error or warning table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceCount {
    pub service: String,
    pub count: u64,
}

impl ServiceCount {
    /// Reads a `hal-powerd           -> 56429 Errors` table row.
    fn from_table_row(line: &str) -> Option<Self> {
        let (service, tail) = line.split_once(SERVICE_COUNT_ARROW)?;
        let count = tail.split_whitespace().next()?.parse().ok()?;
        Some(Self {
            service: service.trim().to_owned(),
            count,
        })
    }
}

/// The per-service table the rows that follow belong to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ServiceTable {
    Errors,
    Warnings,
}

/// The exporter's entry count set against the number of anchored entries the
/// parse read from the same log. The two count the same lines: every journal
/// entry the exporter counts carries a timestamp of its own.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EntryCountMismatch {
    pub stated_by_exporter: u64,
    pub anchored_by_parse: u64,
}

/// Reads the block's lines, each already trimmed, starting at its header.
pub(crate) fn parse_summary_block<'lines>(
    lines: impl IntoIterator<Item = &'lines str>,
) -> SummaryBlock {
    let mut summary = SummaryBlock::default();
    let mut table = None;

    for line in lines {
        match line {
            ERROR_TABLE_HEADER => {
                table = Some(ServiceTable::Errors);
                continue;
            }
            WARNING_TABLE_HEADER => {
                table = Some(ServiceTable::Warnings);
                continue;
            }
            _ => {}
        }

        if let Some(row) = ServiceCount::from_table_row(line) {
            match table {
                Some(ServiceTable::Errors) => summary.service_error_counts.push(row),
                Some(ServiceTable::Warnings) => summary.service_warning_counts.push(row),
                None => {}
            }
            continue;
        }

        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            DEVICE_TYPE_KEY => summary.device_type = Some(value.to_owned()),
            LOGS_BEGIN_KEY => summary.logs_begin_at = parse_exporter_time(value),
            LOGS_END_KEY => summary.logs_end_at = parse_exporter_time(value),
            ENTRY_COUNT_KEY => summary.entry_count = value.parse().ok(),
            _ => {}
        }
    }

    summary
}

fn parse_exporter_time(value: &str) -> Option<DateTime<Utc>> {
    NaiveDateTime::parse_from_str(value, EXPORTER_TIME_FORMAT)
        .ok()
        .map(|naive| naive.and_utc())
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone as _;

    use super::*;

    /// The block a real journald export ends with, shortened to two rows per table.
    const EXPORTED_BLOCK: &str = "\
----------- Journal summary -----------
Device type: nav-devkit-mk2
Logs begin at: Thu 29-May-2025 18:48:25 UTC
Logs end at  : Fri 26-Jun-2026 07:59:50 UTC
Log entries: 622286
--- Service error count ---
hal-powerd           -> 56429 Errors
ofonod               -> 1092 Errors
--- Service warning count ---
core-appd               -> 29562 Warnings
kernel               -> 315 Warnings";

    fn parse(block: &str) -> SummaryBlock {
        parse_summary_block(block.lines().map(str::trim))
    }

    #[test]
    fn an_exported_block_yields_every_figure_it_states() {
        let summary = parse(EXPORTED_BLOCK);
        assert_eq!(summary.device_type.as_deref(), Some("nav-devkit-mk2"));
        assert_eq!(
            summary.logs_begin_at,
            Utc.with_ymd_and_hms(2025, 5, 29, 18, 48, 25).single()
        );
        assert_eq!(
            summary.logs_end_at,
            Utc.with_ymd_and_hms(2026, 6, 26, 7, 59, 50).single()
        );
        assert_eq!(summary.entry_count, Some(622_286));
        assert_eq!(
            summary.service_error_counts,
            [
                ServiceCount {
                    service: "hal-powerd".to_owned(),
                    count: 56_429
                },
                ServiceCount {
                    service: "ofonod".to_owned(),
                    count: 1_092
                },
            ]
        );
        assert_eq!(
            summary.service_warning_counts,
            [
                ServiceCount {
                    service: "core-appd".to_owned(),
                    count: 29_562
                },
                ServiceCount {
                    service: "kernel".to_owned(),
                    count: 315
                },
            ]
        );
    }

    #[test]
    fn a_block_cut_short_yields_only_what_it_got_to_state() {
        let summary = parse("----------- Journal summary -----------\nDevice type: nav-devkit-mk2");
        assert_eq!(
            summary,
            SummaryBlock {
                device_type: Some("nav-devkit-mk2".to_owned()),
                ..SummaryBlock::default()
            }
        );
    }

    /// A row above both table headers belongs to no table and is dropped.
    #[test]
    fn a_service_row_outside_a_table_is_not_counted() {
        let summary = parse("----------- Journal summary -----------\nstray -> 3 Errors");
        assert_eq!(summary, SummaryBlock::default());
    }

    #[test]
    fn a_figure_the_exporter_wrote_unreadably_is_left_unset() {
        let summary =
            parse("Log entries: many\nLogs begin at: yesterday\nsome-service -> lots of Errors");
        assert_eq!(summary, SummaryBlock::default());
    }
}
