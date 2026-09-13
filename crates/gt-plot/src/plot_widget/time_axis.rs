//! The track plot's x-axis: the grid marks it puts on clock and calendar boundaries, the
//! labels its ticks draw under them, and the band row under the ticks.

use chrono::{DateTime, Datelike as _, NaiveDate, NaiveTime};
use egui_plot::{GridInput, GridMark};

/// The grid marks of one frame, and which of them the axis formatter labels.
///
/// The plot builds this in its x-axis spacer, hands the marks to egui_plot and keeps the
/// labelling for the formatter, which egui_plot calls later in the same frame.
pub(super) struct TimeAxisFrame {
    pub(super) marks: Vec<GridMark>,
    pub(super) labeling: TimeAxisLabeling,
}

/// Which marks of a frame get a tick label, and which calendar unit the band row under them
/// covers.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(super) struct TimeAxisLabeling {
    /// The ladder step the axis labels, `None` until the spacer has run. While it is `None`
    /// the formatter returns an empty label for every mark, and the band row is empty.
    label_step: Option<LadderStep>,
}

#[derive(Debug, Default, PartialEq)]
pub(super) struct TimeAxisBandRow {
    pub(super) labels: Vec<BandLabel>,
    pub(super) divider_positions_secs: Vec<f64>,
}

/// One band's label, centred in the band's visible part.
#[derive(Debug, PartialEq)]
pub(super) struct BandLabel {
    pub(super) text: String,
    pub(super) center_secs: f64,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) enum BandUnit {
    Day,
    Month,
    Year,
}

/// One rung of the ladder the axis picks its steps from.
#[derive(Clone, Copy, Debug, PartialEq)]
enum LadderStep {
    /// A whole number of months, marked at the first of every month whose count from January
    /// of year 0 is a multiple of it. The year rungs are its multiples of twelve.
    Months(i64),
    /// A fixed span in seconds, marked at its multiples from the Unix epoch.
    Seconds(f64),
}

/// The seconds one frame covers and the points it draws them across.
#[derive(Clone, Copy)]
struct MarkGeometry {
    bounds: (f64, f64),
    points_per_second: f64,
}

impl TimeAxisFrame {
    pub(super) fn new(input: &GridInput, plot_width_points: f32) -> Self {
        let Some(geometry) = MarkGeometry::new(input.bounds, plot_width_points) else {
            return Self {
                marks: Vec::new(),
                labeling: TimeAxisLabeling::default(),
            };
        };

        let finest_step = ladder_step_at_least(
            input
                .base_step_size
                .max(geometry.seconds_spanning(MIN_GRID_SPACING_POINTS)),
        );
        let label_step = ladder_step_at_least(
            finest_step
                .secs()
                .max(geometry.seconds_spanning(MIN_LABEL_SPACING_POINTS)),
        );
        let coarse_step = ladder_step_at_least(label_step.secs() * COARSE_STEP_MULTIPLE);
        // Every band edge draws the strongest grid line of the marks around it: the band unit
        // goes on top of the three levels.
        let band_step = label_step.band_unit().step();

        let mut marks = Vec::new();
        for step in [finest_step, label_step, coarse_step, band_step] {
            geometry.extend_marks(&mut marks, step);
        }
        dedup_keeping_the_largest_step(&mut marks);

        Self {
            marks,
            labeling: TimeAxisLabeling {
                label_step: Some(label_step),
            },
        }
    }
}

impl TimeAxisLabeling {
    /// The label under `mark`: the clock time while the label step is under a day, and the
    /// day of the month, the month or the year at a coarser step. A mark finer than the label
    /// step, and a value outside the calendar range, get an empty label.
    pub(super) fn tick_label(self, mark: GridMark) -> String {
        let Some(label_step) = self.label_step else {
            return String::new();
        };
        if mark.step_size < label_step.secs() {
            return String::new();
        }
        let Some(instant) = DateTime::from_timestamp(mark.value as i64, 0) else {
            return String::new();
        };
        instant.format(label_step.tick_format()).to_string()
    }

    pub(super) fn band_unit(self) -> Option<BandUnit> {
        self.label_step.map(LadderStep::band_unit)
    }
}

impl TimeAxisBandRow {
    /// The bands of `unit` over `bounds`, the partial band at either end included.
    ///
    /// `label_width_points` measures a label as the widget lays it out. A band whose visible
    /// part is narrower than its label plus [`BAND_LABEL_MARGIN_POINTS`] on each side keeps
    /// its divider and gets no label.
    pub(super) fn new(
        bounds: (f64, f64),
        plot_width_points: f32,
        unit: BandUnit,
        label_width_points: impl Fn(&str) -> f32,
    ) -> Self {
        let step = unit.step();
        let Some(geometry) = MarkGeometry::new(bounds, plot_width_points) else {
            return Self::default();
        };
        if !geometry.step_reaches_the_minimum_grid_spacing(step) {
            return Self::default();
        }
        let (start, end) = bounds;
        let (Some(first), Some(last)) =
            (step.index_at_or_before(start), step.index_at_or_after(end))
        else {
            return Self::default();
        };

        let mut row = Self::default();
        for index in first..last {
            let (Some(band_start), Some(band_end)) =
                (step.boundary(index), step.boundary(index + 1))
            else {
                continue;
            };
            if start < band_start && band_start < end {
                row.divider_positions_secs.push(band_start);
            }
            let visible_start = band_start.max(start);
            let visible_end = band_end.min(end);
            let Some(instant) = DateTime::from_timestamp(band_start as i64, 0) else {
                continue;
            };
            let text = instant.format(unit.label_format()).to_string();
            let margins = 2.0 * f64::from(BAND_LABEL_MARGIN_POINTS);
            if (visible_end - visible_start) * geometry.points_per_second
                < f64::from(label_width_points(&text)) + margins
            {
                continue;
            }
            row.labels.push(BandLabel {
                text,
                center_secs: 0.5 * (visible_start + visible_end),
            });
        }
        row
    }
}

impl BandUnit {
    /// The ladder step whose marks are this unit's band edges.
    fn step(self) -> LadderStep {
        match self {
            Self::Day => LadderStep::Seconds(DAY_SECS),
            Self::Month => LadderStep::Months(1),
            Self::Year => LadderStep::Months(MONTHS_PER_YEAR),
        }
    }

    fn label_format(self) -> &'static str {
        match self {
            Self::Day => DATE_FORMAT,
            Self::Month => YEAR_AND_MONTH_FORMAT,
            Self::Year => YEAR_FORMAT,
        }
    }
}

impl LadderStep {
    /// How long this step is, a month at its mean Gregorian length. The axis picks its three
    /// levels by this length and egui_plot scales a grid line by it, while the marks of a
    /// calendar step land on the calendar.
    fn secs(self) -> f64 {
        match self {
            Self::Months(months) => months as f64 * MEAN_MONTH_SECS,
            Self::Seconds(step_secs) => step_secs,
        }
    }

    fn band_unit(self) -> BandUnit {
        match self {
            Self::Months(_) => BandUnit::Year,
            Self::Seconds(step_secs) if step_secs < DAY_SECS => BandUnit::Day,
            Self::Seconds(_) => BandUnit::Month,
        }
    }

    fn tick_format(self) -> &'static str {
        match self {
            Self::Months(months) if months < MONTHS_PER_YEAR => MONTH_FORMAT,
            Self::Months(_) => YEAR_FORMAT,
            Self::Seconds(step_secs) if step_secs < MINUTE_SECS => CLOCK_WITH_SECONDS_FORMAT,
            Self::Seconds(step_secs) if step_secs < DAY_SECS => CLOCK_FORMAT,
            Self::Seconds(_) => DAY_OF_MONTH_FORMAT,
        }
    }

    /// The instant of the mark `index` steps from the epoch, `None` where the date falls
    /// outside the calendar range.
    fn boundary(self, index: i64) -> Option<f64> {
        match self {
            Self::Months(months) => {
                let absolute = index.checked_mul(months)?;
                let year = i32::try_from(absolute.div_euclid(MONTHS_PER_YEAR)).ok()?;
                let month = u32::try_from(absolute.rem_euclid(MONTHS_PER_YEAR)).ok()? + 1;
                let date = NaiveDate::from_ymd_opt(year, month, 1)?;
                Some(date.and_time(NaiveTime::MIN).and_utc().timestamp() as f64)
            }
            Self::Seconds(step_secs) => Some(index as f64 * step_secs),
        }
    }

    fn index_at_or_before(self, secs: f64) -> Option<i64> {
        match self {
            Self::Months(months) => {
                if !secs.is_finite() {
                    return None;
                }
                let instant = DateTime::from_timestamp(secs.floor() as i64, 0)?;
                let absolute =
                    i64::from(instant.year()) * MONTHS_PER_YEAR + i64::from(instant.month0());
                Some(absolute.div_euclid(months))
            }
            Self::Seconds(step_secs) => {
                let index = (secs / step_secs).floor();
                index.is_finite().then_some(index as i64)
            }
        }
    }

    fn index_at_or_after(self, secs: f64) -> Option<i64> {
        match self {
            Self::Months(_) => {
                let index = self.index_at_or_before(secs)?;
                if self.boundary(index).is_some_and(|at| at >= secs) {
                    return Some(index);
                }
                index.checked_add(1)
            }
            Self::Seconds(step_secs) => {
                let index = (secs / step_secs).ceil();
                index.is_finite().then_some(index as i64)
            }
        }
    }
}

impl MarkGeometry {
    /// `None` for a view with nothing to mark: an empty, inverted or non-finite span, or a
    /// plot of no width.
    fn new(bounds: (f64, f64), plot_width_points: f32) -> Option<Self> {
        let (start, end) = bounds;
        let span_secs = end - start;
        let width_points = f64::from(plot_width_points);
        if !span_secs.is_finite() || span_secs <= 0.0 || width_points <= 0.0 {
            return None;
        }
        Some(Self {
            bounds,
            points_per_second: width_points / span_secs,
        })
    }

    fn extend_marks(&self, marks: &mut Vec<GridMark>, step: LadderStep) {
        let (start, end) = self.bounds;
        if !self.step_reaches_the_minimum_grid_spacing(step) {
            return;
        }
        let (Some(first), Some(last)) =
            (step.index_at_or_after(start), step.index_at_or_before(end))
        else {
            return;
        };
        for index in first..=last {
            let Some(value) = step.boundary(index) else {
                continue;
            };
            marks.push(GridMark {
                value,
                step_size: step.secs(),
            });
        }
    }

    /// Whether consecutive marks of `step` stand at least [`MIN_GRID_SPACING_POINTS`] apart.
    fn step_reaches_the_minimum_grid_spacing(&self, step: LadderStep) -> bool {
        step.secs() * self.points_per_second >= f64::from(MIN_GRID_SPACING_POINTS)
    }

    fn seconds_spanning(&self, points: f32) -> f64 {
        f64::from(points) / self.points_per_second
    }
}

/// The finest ladder step at or above `minimum_secs`, and the longest one where the ladder
/// ends below it.
fn ladder_step_at_least(minimum_secs: f64) -> LadderStep {
    TIME_AXIS_LADDER
        .into_iter()
        .find(|step| step.secs() >= minimum_secs)
        .unwrap_or(LONGEST_LADDER_STEP)
}

/// Sorts the marks by value and drops every repeat of a value two ladder steps both mark,
/// keeping the largest step size of the repeats, as egui_plot's own spacer does.
fn dedup_keeping_the_largest_step(marks: &mut Vec<GridMark>) {
    marks.sort_by(|a, b| {
        a.value
            .total_cmp(&b.value)
            .then_with(|| b.step_size.total_cmp(&a.step_size))
    });
    marks.dedup_by(|a, b| a.value.to_bits() == b.value.to_bits());
}

/// The on-screen spacing a labelled step reaches, in points. The axis fades a label in up to
/// this spacing, and every mark [`TimeAxisFrame`] labels draws at full strength.
pub(super) const MIN_LABEL_SPACING_POINTS: f32 = 80.0;

/// The spacing egui_plot's own `grid_spacing` floor drops a grid line below, in points. The
/// band row follows the same floor.
const MIN_GRID_SPACING_POINTS: f32 = 8.0;

/// The clear space either side of a band's label, in points.
const BAND_LABEL_MARGIN_POINTS: f32 = 8.0;

/// How much coarser than the label step the strongest of the three levels is.
const COARSE_STEP_MULTIPLE: f64 = 4.0;

const MINUTE_SECS: f64 = 60.0;

const HOUR_SECS: f64 = 60.0 * MINUTE_SECS;

const DAY_SECS: f64 = 24.0 * HOUR_SECS;

const MONTHS_PER_YEAR: i64 = 12;

/// The mean Gregorian month: 365.2425 days over twelve.
const MEAN_MONTH_SECS: f64 = 30.436875 * DAY_SECS;

/// The steps the x-axis marks time at, ascending. A midnight in view always falls on a mark:
/// every step up to 12 hours divides a day. A month start falls on a mark too: every step
/// above 10 days is a whole number of months.
const TIME_AXIS_LADDER: [LadderStep; 29] = [
    LadderStep::Seconds(1.0),
    LadderStep::Seconds(2.0),
    LadderStep::Seconds(5.0),
    LadderStep::Seconds(10.0),
    LadderStep::Seconds(15.0),
    LadderStep::Seconds(30.0),
    LadderStep::Seconds(MINUTE_SECS),
    LadderStep::Seconds(2.0 * MINUTE_SECS),
    LadderStep::Seconds(5.0 * MINUTE_SECS),
    LadderStep::Seconds(10.0 * MINUTE_SECS),
    LadderStep::Seconds(15.0 * MINUTE_SECS),
    LadderStep::Seconds(30.0 * MINUTE_SECS),
    LadderStep::Seconds(HOUR_SECS),
    LadderStep::Seconds(2.0 * HOUR_SECS),
    LadderStep::Seconds(3.0 * HOUR_SECS),
    LadderStep::Seconds(6.0 * HOUR_SECS),
    LadderStep::Seconds(12.0 * HOUR_SECS),
    LadderStep::Seconds(DAY_SECS),
    LadderStep::Seconds(2.0 * DAY_SECS),
    LadderStep::Seconds(5.0 * DAY_SECS),
    LadderStep::Seconds(10.0 * DAY_SECS),
    LadderStep::Months(1),
    LadderStep::Months(2),
    LadderStep::Months(3),
    LadderStep::Months(6),
    LadderStep::Months(MONTHS_PER_YEAR),
    LadderStep::Months(2 * MONTHS_PER_YEAR),
    LadderStep::Months(5 * MONTHS_PER_YEAR),
    LONGEST_LADDER_STEP,
];

const LONGEST_LADDER_STEP: LadderStep = LadderStep::Months(10 * MONTHS_PER_YEAR);

/// A UTC date, the label of a day band.
const DATE_FORMAT: &str = "%Y-%m-%d";

/// A UTC year and month, the label of a month band.
const YEAR_AND_MONTH_FORMAT: &str = "%Y-%m";

/// A UTC year, the label of a year band and the tick label at a year rung.
const YEAR_FORMAT: &str = "%Y";

/// A UTC month of the year, the tick label at a month rung.
const MONTH_FORMAT: &str = "%m";

/// A UTC day of the month, the tick label at a day rung.
const DAY_OF_MONTH_FORMAT: &str = "%d";

/// A UTC clock time to the minute, the tick label while the label step is under a day.
const CLOCK_FORMAT: &str = "%H:%M";

/// A UTC clock time to the second, the tick label while the label step is under a minute.
const CLOCK_WITH_SECONDS_FORMAT: &str = "%H:%M:%S";

#[cfg(test)]
mod tests {
    use chrono::DateTime;
    use egui_plot::{GridInput, GridMark};

    use super::{
        BandLabel, BandUnit, DAY_SECS, HOUR_SECS, LadderStep, MEAN_MONTH_SECS,
        MIN_GRID_SPACING_POINTS, MINUTE_SECS, MONTHS_PER_YEAR, TimeAxisBandRow, TimeAxisFrame,
    };

    /// Plot width in points, the width a track plot has on a desktop window.
    const PLOT_WIDTH_POINTS: f32 = 960.0;

    /// 2024-01-15 12:00:00 UTC, where every view below starts.
    const VIEW_START_SECS: f64 = 1_705_320_000.0;

    /// 2024-01-16 00:00:00 UTC, the one day boundary a view of a day and a half holds.
    const MIDNIGHT_SECS: f64 = VIEW_START_SECS + 12.0 * HOUR_SECS;

    /// 2024-02-01 00:00:00 UTC.
    const FEBRUARY_SECS: f64 = 1_706_745_600.0;

    /// A label of seven points per character, roughly the width of a digit in the plot's body
    /// font.
    fn label_width(text: &str) -> f32 {
        text.chars().count() as f32 * 7.0
    }

    /// The frame a plot [`PLOT_WIDTH_POINTS`] wide draws over `span_secs` from
    /// [`VIEW_START_SECS`], with the base step egui_plot derives from that width.
    fn frame_over(span_secs: f64) -> TimeAxisFrame {
        let input = GridInput {
            bounds: (VIEW_START_SECS, VIEW_START_SECS + span_secs),
            base_step_size: span_secs / f64::from(PLOT_WIDTH_POINTS)
                * f64::from(MIN_GRID_SPACING_POINTS),
        };
        TimeAxisFrame::new(&input, PLOT_WIDTH_POINTS)
    }

    /// The band row a plot [`PLOT_WIDTH_POINTS`] wide draws for `unit` over `bounds`.
    fn band_row_over(bounds: (f64, f64), unit: BandUnit) -> TimeAxisBandRow {
        TimeAxisBandRow::new(bounds, PLOT_WIDTH_POINTS, unit, label_width)
    }

    /// The step sizes the frame's marks take, coarsest first.
    fn step_sizes(frame: &TimeAxisFrame) -> Vec<f64> {
        let mut sizes: Vec<f64> = frame.marks.iter().map(|mark| mark.step_size).collect();
        sizes.sort_by(|a, b| b.total_cmp(a));
        sizes.dedup();
        sizes
    }

    /// The labels the frame draws, in the order of the marks, leaving out the marks it labels
    /// with an empty string.
    fn labels(frame: &TimeAxisFrame) -> Vec<String> {
        frame
            .marks
            .iter()
            .map(|&mark| frame.labeling.tick_label(mark))
            .filter(|label| !label.is_empty())
            .collect()
    }

    /// The texts of a band row's labels, in the order of the bands.
    fn label_texts(row: &TimeAxisBandRow) -> Vec<&str> {
        row.labels.iter().map(|label| label.text.as_str()).collect()
    }

    #[rstest::rstest]
    #[case::two_minutes(2.0 * MINUTE_SECS, &[MINUTE_SECS, 10.0, 1.0])]
    #[case::thirty_hours(30.0 * HOUR_SECS, &[DAY_SECS, 12.0 * HOUR_SECS, 3.0 * HOUR_SECS, 15.0 * MINUTE_SECS])]
    #[case::nine_days(9.0 * DAY_SECS, &[5.0 * DAY_SECS, DAY_SECS, 2.0 * HOUR_SECS])]
    #[case::thirty_days(30.0 * DAY_SECS, &[MEAN_MONTH_SECS, 5.0 * DAY_SECS, 6.0 * HOUR_SECS])]
    fn the_ladder_steps_of_a_span(#[case] span_secs: f64, #[case] expected: &[f64]) {
        assert_eq!(step_sizes(&frame_over(span_secs)), expected);
    }

    #[test]
    fn every_mark_lands_on_a_boundary_of_its_own_step() {
        let frame = frame_over(30.0 * HOUR_SECS);
        let off_boundary: Vec<f64> = frame
            .marks
            .iter()
            .filter(|mark| mark.value.rem_euclid(mark.step_size) != 0.0)
            .map(|mark| mark.value)
            .collect();
        assert_eq!(off_boundary, Vec::<f64>::new());
    }

    #[rstest::rstest]
    #[case::a_midnight_under_hour_ticks(30.0 * HOUR_SECS, MIDNIGHT_SECS, DAY_SECS)]
    #[case::a_month_start_under_day_ticks(30.0 * DAY_SECS, FEBRUARY_SECS, MEAN_MONTH_SECS)]
    fn a_band_edge_is_marked_once_at_the_band_step(
        #[case] span_secs: f64,
        #[case] edge_secs: f64,
        #[case] expected_step: f64,
    ) {
        let frame = frame_over(span_secs);
        let steps: Vec<f64> = frame
            .marks
            .iter()
            .filter(|mark| mark.value.to_bits() == edge_secs.to_bits())
            .map(|mark| mark.step_size)
            .collect();
        assert_eq!(steps, [expected_step]);
    }

    #[rstest::rstest]
    #[case::ten_second_ticks(2.0 * MINUTE_SECS, &["12:00:00", "12:00:10", "12:00:20"])]
    #[case::three_hour_ticks(30.0 * HOUR_SECS, &["12:00", "15:00", "18:00"])]
    #[case::day_ticks(9.0 * DAY_SECS, &["16", "17", "18"])]
    #[case::month_ticks(200.0 * DAY_SECS, &["02", "03", "04"])]
    #[case::year_ticks(3000.0 * DAY_SECS, &["2025", "2026", "2027"])]
    fn the_tick_labels_of_a_span(#[case] span_secs: f64, #[case] expected: &[&str]) {
        let first_three: Vec<String> = labels(&frame_over(span_secs)).into_iter().take(3).collect();
        assert_eq!(first_three, expected);
    }

    #[test]
    fn a_midnight_is_labelled_with_the_clock_alone() {
        let labels = labels(&frame_over(30.0 * HOUR_SECS));
        assert!(
            labels.contains(&"00:00".to_owned()),
            "the tick row draws one line at every step under a day: {labels:?}"
        );
    }

    #[test]
    fn a_mark_finer_than_the_label_step_gets_no_label() {
        let frame = frame_over(30.0 * HOUR_SECS);
        let Some(&mark) = frame.marks.iter().find(|mark| mark.step_size < HOUR_SECS) else {
            panic!("a 30-hour view marks the quarter hours");
        };
        assert_eq!(frame.labeling.tick_label(mark), "");
    }

    #[test]
    fn a_value_outside_the_calendar_range_gets_no_label() {
        let frame = frame_over(30.0 * HOUR_SECS);
        let mark = GridMark {
            value: 1e19,
            step_size: DAY_SECS,
        };
        assert_eq!(frame.labeling.tick_label(mark), "");
    }

    #[rstest::rstest]
    #[case::no_span(0.0, PLOT_WIDTH_POINTS)]
    #[case::an_inverted_span(-HOUR_SECS, PLOT_WIDTH_POINTS)]
    #[case::no_width(HOUR_SECS, 0.0)]
    #[case::a_span_of_no_number(f64::NAN, PLOT_WIDTH_POINTS)]
    fn a_view_with_nothing_to_mark_has_no_marks(
        #[case] span_secs: f64,
        #[case] plot_width_points: f32,
    ) {
        let input = GridInput {
            bounds: (VIEW_START_SECS, VIEW_START_SECS + span_secs),
            base_step_size: 1.0,
        };
        assert_eq!(
            TimeAxisFrame::new(&input, plot_width_points).marks,
            Vec::new()
        );
    }

    #[rstest::rstest]
    #[case::ten_second_ticks(2.0 * MINUTE_SECS, BandUnit::Day)]
    #[case::three_hour_ticks(30.0 * HOUR_SECS, BandUnit::Day)]
    #[case::day_ticks(9.0 * DAY_SECS, BandUnit::Month)]
    #[case::month_ticks(200.0 * DAY_SECS, BandUnit::Year)]
    #[case::year_ticks(3000.0 * DAY_SECS, BandUnit::Year)]
    fn the_band_unit_of_a_span(#[case] span_secs: f64, #[case] expected: BandUnit) {
        assert_eq!(frame_over(span_secs).labeling.band_unit(), Some(expected));
    }

    #[test]
    fn a_partial_band_at_either_end_is_labelled_in_its_visible_part() {
        let row = band_row_over((VIEW_START_SECS, VIEW_START_SECS + DAY_SECS), BandUnit::Day);

        assert_eq!(
            row.labels,
            vec![
                BandLabel {
                    text: "2024-01-15".to_owned(),
                    center_secs: VIEW_START_SECS + 6.0 * HOUR_SECS,
                },
                BandLabel {
                    text: "2024-01-16".to_owned(),
                    center_secs: VIEW_START_SECS + 18.0 * HOUR_SECS,
                },
            ]
        );
        assert_eq!(row.divider_positions_secs, [MIDNIGHT_SECS]);
    }

    #[test]
    fn a_band_narrower_than_its_label_keeps_its_divider_and_loses_its_label() {
        let start = MIDNIGHT_SECS - MINUTE_SECS;
        let row = band_row_over((start, start + 12.0 * HOUR_SECS), BandUnit::Day);

        assert_eq!(label_texts(&row), ["2024-01-16"]);
        assert_eq!(row.divider_positions_secs, [MIDNIGHT_SECS]);
    }

    #[test]
    fn the_month_bands_of_a_view_across_a_year_end() {
        let december = FEBRUARY_SECS - 2.0 * 30.0 * DAY_SECS;
        let row = band_row_over((december, december + 90.0 * DAY_SECS), BandUnit::Month);

        assert_eq!(label_texts(&row), ["2023-12", "2024-01", "2024-02"]);
    }

    #[rstest::rstest]
    #[case::three_months(3, 2024 * MONTHS_PER_YEAR / 3, &["2024-01-01", "2024-04-01", "2024-07-01"])]
    #[case::ten_years(10 * MONTHS_PER_YEAR, 2020 * MONTHS_PER_YEAR / (10 * MONTHS_PER_YEAR), &["2020-01-01", "2030-01-01", "2040-01-01"])]
    fn a_calendar_rung_marks_the_calendar(
        #[case] months: i64,
        #[case] first_index: i64,
        #[case] expected: &[&str],
    ) {
        let step = LadderStep::Months(months);
        let marks: Vec<String> = (0..3)
            .filter_map(|offset| step.boundary(first_index + offset))
            .filter_map(|secs| DateTime::from_timestamp(secs as i64, 0))
            .map(|at| at.format("%Y-%m-%d %H:%M").to_string())
            .collect();

        let expected: Vec<String> = expected.iter().map(|day| format!("{day} 00:00")).collect();
        assert_eq!(marks, expected);
    }

    #[rstest::rstest]
    #[case::no_span(0.0, PLOT_WIDTH_POINTS)]
    #[case::an_inverted_span(-HOUR_SECS, PLOT_WIDTH_POINTS)]
    #[case::no_width(DAY_SECS, 0.0)]
    #[case::a_span_of_no_number(f64::NAN, PLOT_WIDTH_POINTS)]
    fn a_view_with_nothing_to_band_has_no_bands(
        #[case] span_secs: f64,
        #[case] plot_width_points: f32,
    ) {
        let row = TimeAxisBandRow::new(
            (VIEW_START_SECS, VIEW_START_SECS + span_secs),
            plot_width_points,
            BandUnit::Day,
            label_width,
        );

        assert_eq!(row, TimeAxisBandRow::default());
    }
}
