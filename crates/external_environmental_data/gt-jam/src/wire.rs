//! The published CSV, parsed into [`HexObservation`]s.
//!
//! One dataset is one UTC day of the whole world: a header, then roughly
//! 44 000 rows of one H3 cell and its aircraft tally.
//!
//! ```text
//! hex,count_good_aircraft,count_bad_aircraft
//! 84005c7ffffffff,412,3
//! ```
//!
//! A bad row does not fail the parse: it is skipped and reported as a
//! [`ParseWarning`] with its line number.

use std::collections::HashMap;
use std::str::FromStr as _;

use h3o::CellIndex;
use parking_lot::Mutex;

use crate::H3_RESOLUTION;

/// Name of the cell-index column, as published.
const HEX_COLUMN: &str = "hex";

/// Name of the good-aircraft count column, as published.
const GOOD_COLUMN: &str = "count_good_aircraft";

/// Name of the low-accuracy aircraft count column, as published.
const BAD_COLUMN: &str = "count_bad_aircraft";

/// The published columns, in order. The header must match exactly.
const COLUMNS: [&str; 3] = [HEX_COLUMN, GOOD_COLUMN, BAD_COLUMN];

/// Field separator. The published data has no quoting or escaping.
const FIELD_SEPARATOR: char = ',';

/// The header row as the host writes it.
fn header_line() -> String {
    COLUMNS.join(&FIELD_SEPARATOR.to_string())
}

/// One cell's aircraft tally over one UTC day, as published.
///
/// Counts are aircraft in flight over a whole day and a cell tens of
/// kilometers across - see [`crate::text`] for the wording surfaces showing
/// one of these carry.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HexObservation {
    /// The cell the tally covers, at [`H3_RESOLUTION`].
    pub cell: CellIndex,
    /// Aircraft that reported good navigation accuracy.
    pub good: u32,
    /// Aircraft that reported low navigation accuracy.
    pub bad: u32,
}

impl HexObservation {
    /// Aircraft counted in the cell, good and bad together.
    ///
    /// Saturates: a sum overflowing [`u32`] is corrupt input, and wrapping
    /// would turn it into a plausible small count.
    pub const fn aircraft(&self) -> u32 {
        self.good.saturating_add(self.bad)
    }

    /// The share of aircraft that reported low navigation accuracy, with
    /// the sample size behind it.
    ///
    /// [`None`] for an empty tally, which would otherwise render as 0 % and
    /// read as clear. [`parse_dataset`] rejects such rows, so this is only
    /// reachable for hand-built observations.
    pub fn rate(&self) -> Option<InterferenceRate> {
        let aircraft = self.aircraft();
        if aircraft == 0 {
            return None;
        }
        Some(InterferenceRate {
            bad_fraction: (f64::from(self.bad) / f64::from(aircraft)) as f32,
            aircraft,
        })
    }
}

/// The share of aircraft reporting low navigation accuracy, with its
/// sample size.
///
/// One value so a share cannot be rendered without its confidence: 1 bad of
/// 2 aircraft is 50 % and means nothing.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InterferenceRate {
    /// Bad aircraft over total aircraft, in `0.0..=1.0`.
    pub bad_fraction: f32,
    /// Aircraft the share was computed over.
    pub aircraft: u32,
}

impl InterferenceRate {
    /// The share as a percentage.
    pub fn percent(self) -> f64 {
        f64::from(self.bad_fraction) * 100.0
    }
}

/// A header that is not the published one, which makes every row unreadable.
/// Row-level problems are [`ParseWarning`]s instead.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ParseError {
    /// The dataset had no lines at all.
    #[error("dataset is empty: expected a header row {:?}", header_line())]
    MissingHeader,

    /// The first line was not the published header.
    #[error("unexpected header {found:?}, expected {:?}", header_line())]
    UnexpectedHeader { found: String },
}

/// Which count column a row's bad value came from.
#[derive(Debug, Clone, Copy, PartialEq, Eq, strum::Display, strum::EnumCount)]
#[strum(serialize_all = "snake_case")]
pub enum CountColumn {
    Good,
    Bad,
}

impl CountColumn {
    /// The column's published header name.
    pub const fn header_name(self) -> &'static str {
        match self {
            Self::Good => GOOD_COLUMN,
            Self::Bad => BAD_COLUMN,
        }
    }
}

/// A row the parser could not use, which contributes no observation.
///
/// `line` is 1-based with the header as line 1.
#[derive(Debug, Clone, PartialEq, Eq, strum::EnumCount, strum::IntoStaticStr)]
#[strum(serialize_all = "snake_case")]
pub enum ParseWarning {
    /// The row did not have the published number of fields.
    FieldCount { line: usize, fields: usize },

    /// An empty line inside the dataset. The host serves none.
    BlankLine { line: usize },

    /// The cell index did not parse as an H3 index.
    CellIndex {
        line: usize,
        hex: String,
        detail: String,
    },

    /// A valid cell index, but not at [`H3_RESOLUTION`], so it does not tile
    /// with the rest of the dataset.
    Resolution { line: usize, cell: CellIndex },

    /// A count field was not a non-negative integer.
    Count {
        line: usize,
        column: CountColumn,
        value: String,
        detail: String,
    },

    /// Zero aircraft, so the row has no share. The host omits such cells.
    EmptyTally { line: usize, cell: CellIndex },

    /// A cell already seen earlier in the dataset. The first row is kept.
    DuplicateCell {
        line: usize,
        cell: CellIndex,
        first_seen_line: usize,
    },
}

/// How many warnings a [`ParseWarningReporter`] retains before it only
/// counts them. A malformed response can be bad in all 44 000 rows.
pub const MAX_RETAINED_WARNINGS: usize = 64;

/// Accumulates [`ParseWarning`]s across one dataset.
///
/// Interior mutability so parsing can run on a worker thread while the app
/// holds the reporter (the pattern in `CODE_STYLE.md`).
#[derive(Debug, Default)]
pub struct ParseWarningReporter {
    state: Mutex<ReporterState>,
}

#[derive(Debug, Default)]
struct ReporterState {
    retained: Vec<ParseWarning>,
    suppressed: usize,
}

impl ParseWarningReporter {
    /// Record `warning`, or count it as suppressed once
    /// [`MAX_RETAINED_WARNINGS`] have been retained.
    pub fn report(&self, warning: ParseWarning) {
        let mut state = self.state.lock();
        if state.retained.len() < MAX_RETAINED_WARNINGS {
            state.retained.push(warning);
        } else {
            state.suppressed += 1;
        }
    }

    /// The retained warnings, in the order they were reported.
    pub fn warnings(&self) -> Vec<ParseWarning> {
        self.state.lock().retained.clone()
    }

    /// Warnings reported after the retention ceiling was reached.
    pub fn suppressed(&self) -> usize {
        self.state.lock().suppressed
    }

    /// Whether anything at all was reported, retained or suppressed.
    pub fn is_empty(&self) -> bool {
        let state = self.state.lock();
        state.retained.is_empty() && state.suppressed == 0
    }
}

/// Parse one day's dataset, in published row order.
///
/// Unusable rows go to `reporter` and are skipped. The parser rejects the whole
/// dataset only for an unexpected header.
pub fn parse_dataset(
    csv: &str,
    reporter: &ParseWarningReporter,
) -> Result<Vec<HexObservation>, ParseError> {
    let mut lines = csv.lines().enumerate();
    let Some((_, header)) = lines.next() else {
        return Err(ParseError::MissingHeader);
    };
    let columns: Vec<&str> = header.split(FIELD_SEPARATOR).collect();
    if columns.as_slice() != COLUMNS {
        return Err(ParseError::UnexpectedHeader {
            found: header.to_owned(),
        });
    }

    let mut observations: Vec<HexObservation> = Vec::new();
    // The line each cell was first accepted on, so a duplicate warning can
    // name the row that won.
    let mut first_seen: HashMap<CellIndex, usize> = HashMap::new();
    for (index, line) in lines {
        // The header took index 0 and is line 1.
        let line_number = index + 1;
        match parse_row(line_number, line) {
            Ok(observation) => match first_seen.get(&observation.cell) {
                Some(&first_seen_line) => reporter.report(ParseWarning::DuplicateCell {
                    line: line_number,
                    cell: observation.cell,
                    first_seen_line,
                }),
                None => {
                    first_seen.insert(observation.cell, line_number);
                    observations.push(observation);
                }
            },
            Err(warning) => reporter.report(warning),
        }
    }
    Ok(observations)
}

/// One data row, or the warning explaining why it is unusable.
fn parse_row(line: usize, row: &str) -> Result<HexObservation, ParseWarning> {
    if row.is_empty() {
        return Err(ParseWarning::BlankLine { line });
    }
    let fields: Vec<&str> = row.split(FIELD_SEPARATOR).collect();
    let [hex, good, bad] = fields.as_slice() else {
        return Err(ParseWarning::FieldCount {
            line,
            fields: fields.len(),
        });
    };

    let cell = CellIndex::from_str(hex).map_err(|err| ParseWarning::CellIndex {
        line,
        hex: (*hex).to_owned(),
        detail: err.to_string(),
    })?;
    if cell.resolution() != H3_RESOLUTION {
        return Err(ParseWarning::Resolution { line, cell });
    }

    let observation = HexObservation {
        cell,
        good: parse_count(line, CountColumn::Good, good)?,
        bad: parse_count(line, CountColumn::Bad, bad)?,
    };
    if observation.aircraft() == 0 {
        return Err(ParseWarning::EmptyTally { line, cell });
    }
    Ok(observation)
}

/// One count field, or the warning explaining why it is unusable.
fn parse_count(line: usize, column: CountColumn, value: &str) -> Result<u32, ParseWarning> {
    u32::from_str(value).map_err(|err| ParseWarning::Count {
        line,
        column,
        value: value.to_owned(),
        detail: err.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use rstest::rstest;
    use strum::EnumCount as _;

    use super::*;

    // Cells at the published resolution, copied from the captured fixture
    // day. Each is named for the row it carries in the malformed dataset.
    const GOOD_CELL: &str = "84005c7ffffffff";
    const SECOND_GOOD_CELL: &str = "8401255ffffffff";
    const SHORT_ROW_CELL: &str = "840104bffffffff";
    const LONG_ROW_CELL: &str = "8401221ffffffff";
    const WORDED_COUNT_CELL: &str = "8401227ffffffff";
    const NEGATIVE_COUNT_CELL: &str = "8401233ffffffff";
    const OVERFLOWING_COUNT_CELL: &str = "840124bffffffff";
    const EMPTY_TALLY_CELL: &str = "8401251ffffffff";

    /// A valid H3 index at resolution 0, for the resolution check.
    const CELL_RES_0: &str = "8005fffffffffff";

    /// A string that is not an H3 index at all.
    const NOT_A_CELL: &str = "not-a-cell";

    fn cell(hex: &str) -> CellIndex {
        CellIndex::from_str(hex).unwrap()
    }

    fn parse(csv: &str) -> (Vec<HexObservation>, Vec<ParseWarning>) {
        let reporter = ParseWarningReporter::default();
        let observations = parse_dataset(csv, &reporter).unwrap();
        (observations, reporter.warnings())
    }

    fn dataset(rows: &[&str]) -> String {
        let mut csv = header_line();
        for row in rows {
            csv.push('\n');
            csv.push_str(row);
        }
        csv.push('\n');
        csv
    }

    #[test]
    fn parses_the_published_shape() {
        let (observations, warnings) = parse(&dataset(&[
            &format!("{GOOD_CELL},412,3"),
            &format!("{SECOND_GOOD_CELL},1,0"),
        ]));
        assert!(
            warnings.is_empty(),
            "both rows are as published: {warnings:#?}"
        );
        assert_eq!(
            observations.as_slice(),
            [
                HexObservation {
                    cell: cell(GOOD_CELL),
                    good: 412,
                    bad: 3,
                },
                HexObservation {
                    cell: cell(SECOND_GOOD_CELL),
                    good: 1,
                    bad: 0,
                }
            ]
        );
    }

    #[test]
    fn a_clean_dataset_reports_nothing() {
        let reporter = ParseWarningReporter::default();
        let csv = dataset(&[&format!("{GOOD_CELL},412,3")]);
        parse_dataset(&csv, &reporter).unwrap();
        assert!(reporter.is_empty());
        assert_eq!(reporter.suppressed(), 0);
    }

    #[test]
    fn an_empty_dataset_has_no_header() {
        let reporter = ParseWarningReporter::default();
        assert_eq!(parse_dataset("", &reporter), Err(ParseError::MissingHeader));
        assert!(reporter.is_empty(), "nothing was parsed to warn about");
    }

    /// The host serves a trailing newline.
    #[test]
    fn a_trailing_newline_is_not_a_row() {
        let (observations, warnings) = parse(&format!("{}\n{GOOD_CELL},412,3\n", header_line()));
        assert_eq!(observations.len(), 1);
        assert!(warnings.is_empty(), "{warnings:#?}");
    }

    /// A CRLF line ending must not end up inside the last field.
    #[test]
    fn carriage_returns_are_not_part_of_a_count() {
        let (observations, warnings) =
            parse(&format!("{}\r\n{GOOD_CELL},412,3\r\n", header_line()));
        assert!(warnings.is_empty(), "{warnings:#?}");
        assert_eq!(observations.first().map(|row| row.bad), Some(3));
    }

    #[rstest]
    #[case::renamed_column("hex,good,bad")]
    #[case::reordered_columns("hex,count_bad_aircraft,count_good_aircraft")]
    #[case::extra_column("hex,count_good_aircraft,count_bad_aircraft,count_unknown")]
    #[case::missing_column("hex,count_good_aircraft")]
    #[case::data_row_instead_of_header("84005c7ffffffff,412,3")]
    fn a_header_that_is_not_the_published_one_is_rejected(#[case] header: &str) {
        let reporter = ParseWarningReporter::default();
        let csv = format!("{header}\n{GOOD_CELL},412,3\n");
        assert_eq!(
            parse_dataset(&csv, &reporter),
            Err(ParseError::UnexpectedHeader {
                found: header.to_owned(),
            })
        );
    }

    /// The header error states what arrived and what was expected.
    #[test]
    fn header_error_wording() {
        let error = ParseError::UnexpectedHeader {
            found: "hex,good,bad".to_owned(),
        };
        insta::assert_snapshot!("unexpected_header", error.to_string());
        insta::assert_snapshot!("missing_header", ParseError::MissingHeader.to_string());
    }

    /// Every way a row can be unusable, in one dataset. The first and last
    /// rows are good, so a bad row is shown to cost nothing either side of it.
    fn malformed_dataset() -> String {
        dataset(&[
            &format!("{GOOD_CELL},412,3"),
            "",
            &format!("{SHORT_ROW_CELL},7"),
            &format!("{LONG_ROW_CELL},7,1,extra"),
            &format!("{NOT_A_CELL},7,1"),
            &format!("{CELL_RES_0},7,1"),
            &format!("{WORDED_COUNT_CELL},seven,1"),
            &format!("{NEGATIVE_COUNT_CELL},7,-1"),
            &format!("{OVERFLOWING_COUNT_CELL},4294967296,1"),
            &format!("{EMPTY_TALLY_CELL},0,0"),
            &format!("{GOOD_CELL},99,99"),
            &format!("{SECOND_GOOD_CELL},10,2"),
        ])
    }

    /// Each unusable row is skipped and warns with its line number.
    #[test]
    fn malformed_rows_are_skipped_and_reported() {
        let (observations, warnings) = parse(&malformed_dataset());

        insta::assert_debug_snapshot!("malformed_rows_warnings", warnings);
        insta::assert_debug_snapshot!(
            "malformed_rows_observations",
            observations
                .iter()
                .map(|observation| (
                    observation.cell.to_string(),
                    observation.good,
                    observation.bad
                ))
                .collect::<Vec<_>>()
        );
    }

    /// Ten unusable rows precede the last, good, row.
    #[test]
    fn parsing_continues_past_a_malformed_row() {
        let (observations, _) = parse(&malformed_dataset());
        assert_eq!(
            observations.last().map(|observation| observation.cell),
            Some(cell(SECOND_GOOD_CELL))
        );
    }

    /// A variant cannot be added without a row that reaches it.
    #[test]
    fn every_warning_variant_is_reachable_from_a_dataset() {
        let (_, warnings) = parse(&malformed_dataset());
        let reached: HashSet<&'static str> = warnings.iter().map(<&'static str>::from).collect();
        assert_eq!(
            reached.len(),
            ParseWarning::COUNT,
            "one dataset should reach every declared warning: {warnings:#?}"
        );
    }

    #[test]
    fn a_duplicate_cell_keeps_the_first_row() {
        let (observations, warnings) = parse(&dataset(&[
            &format!("{GOOD_CELL},412,3"),
            &format!("{GOOD_CELL},1,1"),
        ]));
        assert_eq!(
            observations.first().map(|observation| observation.good),
            Some(412)
        );
        assert_eq!(
            warnings.as_slice(),
            [ParseWarning::DuplicateCell {
                line: 3,
                cell: cell(GOOD_CELL),
                first_seen_line: 2,
            }]
        );
    }

    #[rstest]
    #[case::all_bad(0, 4, 1.0, 4)]
    #[case::all_good(4, 0, 0.0, 4)]
    #[case::the_yellow_breakpoint(98, 2, 0.02, 100)]
    #[case::the_red_breakpoint(90, 10, 0.1, 100)]
    fn rate_carries_its_sample_size(
        #[case] good: u32,
        #[case] bad: u32,
        #[case] bad_fraction: f32,
        #[case] aircraft: u32,
    ) {
        let observation = HexObservation {
            cell: cell(GOOD_CELL),
            good,
            bad,
        };
        assert_eq!(
            observation.rate(),
            Some(InterferenceRate {
                bad_fraction,
                aircraft
            })
        );
    }

    #[test]
    fn an_empty_tally_has_no_rate() {
        let observation = HexObservation {
            cell: cell(GOOD_CELL),
            good: 0,
            bad: 0,
        };
        assert_eq!(observation.rate(), None);
        assert_eq!(observation.aircraft(), 0);
    }

    #[test]
    fn a_tally_that_would_overflow_saturates() {
        let observation = HexObservation {
            cell: cell(GOOD_CELL),
            good: u32::MAX,
            bad: 1,
        };
        assert_eq!(observation.aircraft(), u32::MAX);
    }

    /// Retention stops at the ceiling, counting does not.
    #[test]
    fn the_reporter_counts_what_it_stops_retaining() {
        let overshoot = 3;
        let rows: Vec<String> = (0..MAX_RETAINED_WARNINGS + overshoot)
            .map(|row| format!("not-a-cell-{row},1,1"))
            .collect();
        let row_refs: Vec<&str> = rows.iter().map(String::as_str).collect();

        let reporter = ParseWarningReporter::default();
        let observations = parse_dataset(&dataset(&row_refs), &reporter).unwrap();

        assert!(observations.is_empty());
        assert_eq!(reporter.warnings().len(), MAX_RETAINED_WARNINGS);
        assert_eq!(reporter.suppressed(), overshoot);
        assert!(!reporter.is_empty());
    }

    #[test]
    fn count_columns_name_published_headers() {
        assert_eq!(CountColumn::Good.header_name(), GOOD_COLUMN);
        assert_eq!(CountColumn::Bad.header_name(), BAD_COLUMN);
        assert_eq!(CountColumn::COUNT, COLUMNS.len() - 1);
    }

    #[test]
    fn the_header_line_is_the_published_one() {
        assert_eq!(header_line(), "hex,count_good_aircraft,count_bad_aircraft");
    }
}
