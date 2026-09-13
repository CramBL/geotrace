//! The track plot's x-axis: the grid marks it puts on clock boundaries, and the labels its
//! ticks draw under them.

use chrono::{DateTime, NaiveTime};
use egui_plot::{GridInput, GridMark};

/// The grid marks of one frame, and which of them the axis formatter labels.
///
/// The plot builds this in its x-axis spacer, hands the marks to egui_plot and keeps the
/// labelling for the formatter, which egui_plot calls later in the same frame.
pub(super) struct TimeAxisFrame {
    pub(super) marks: Vec<GridMark>,
    pub(super) labeling: TimeAxisLabeling,
}

/// Which marks of a frame get a tick label, and which one shows the date besides the
/// midnights.
#[derive(Clone, Copy, Debug, PartialEq)]
pub(super) struct TimeAxisLabeling {
    /// A mark at this step size or coarser gets a label, a finer one none.
    label_step_secs: f64,
    /// The value of the first labelled mark of the view. Its label shows the date on a second
    /// line.
    mark_showing_the_date: Option<f64>,
}

/// The seconds one frame covers and the points it draws them across.
#[derive(Clone, Copy)]
struct MarkGeometry {
    bounds: (f64, f64),
    points_per_second: f64,
}

impl TimeAxisFrame {
    pub(super) fn new(input: &GridInput, plot_width_points: f32) -> Self {
        let (start, end) = input.bounds;
        let span_secs = end - start;
        let width_points = f64::from(plot_width_points);
        if !span_secs.is_finite() || span_secs <= 0.0 || width_points <= 0.0 {
            return Self {
                marks: Vec::new(),
                labeling: TimeAxisLabeling::default(),
            };
        }

        let geometry = MarkGeometry {
            bounds: input.bounds,
            points_per_second: width_points / span_secs,
        };
        let finest_step = ladder_step_at_least(
            input
                .base_step_size
                .max(geometry.seconds_spanning(MIN_GRID_SPACING_POINTS)),
        );
        let label_step = ladder_step_at_least(
            finest_step.max(geometry.seconds_spanning(MIN_LABEL_SPACING_POINTS)),
        );
        let coarse_step = ladder_step_at_least(label_step * COARSE_STEP_MULTIPLE);

        let mut marks = Vec::new();
        for step_secs in [finest_step, label_step, coarse_step] {
            geometry.extend_marks(&mut marks, step_secs);
        }
        // The day goes on top of the three levels. A midnight in view then draws the
        // strongest line of the labelled marks.
        if DAY_SECS >= label_step {
            geometry.extend_marks(&mut marks, DAY_SECS);
        }
        dedup_keeping_the_largest_step(&mut marks);

        let mark_showing_the_date = marks
            .iter()
            .find(|mark| mark.step_size >= label_step)
            .map(|mark| mark.value);
        Self {
            marks,
            labeling: TimeAxisLabeling {
                label_step_secs: label_step,
                mark_showing_the_date,
            },
        }
    }
}

impl TimeAxisLabeling {
    /// The label under `mark`: the clock time, with the date on a second line under a midnight
    /// and under the first labelled mark of the view. A mark finer than the label step, and a
    /// value outside the calendar range, get an empty label.
    pub(super) fn tick_label(self, mark: GridMark) -> String {
        let Self {
            label_step_secs,
            mark_showing_the_date,
        } = self;
        if mark.step_size < label_step_secs {
            return String::new();
        }
        let Some(instant) = DateTime::from_timestamp(mark.value as i64, 0) else {
            return String::new();
        };
        let at_midnight = instant.time() == NaiveTime::MIN;
        if at_midnight && label_step_secs >= DAY_SECS {
            return instant.format(DATE_FORMAT).to_string();
        }

        let clock = instant.format(if label_step_secs < MINUTE_SECS {
            CLOCK_WITH_SECONDS_FORMAT
        } else {
            CLOCK_FORMAT
        });
        let shows_the_date = at_midnight
            || mark_showing_the_date.is_some_and(|dated| dated.to_bits() == mark.value.to_bits());
        if shows_the_date {
            return format!("{clock}\n{}", instant.format(DATE_FORMAT));
        }
        clock.to_string()
    }
}

/// The labelling of a frame whose spacer has not run yet: no step size reaches infinity, so no
/// mark gets a label.
impl Default for TimeAxisLabeling {
    fn default() -> Self {
        Self {
            label_step_secs: f64::INFINITY,
            mark_showing_the_date: None,
        }
    }
}

impl MarkGeometry {
    fn extend_marks(&self, marks: &mut Vec<GridMark>, step_secs: f64) {
        let Self {
            bounds: (start, end),
            points_per_second,
        } = *self;
        if step_secs * points_per_second < f64::from(MIN_GRID_SPACING_POINTS) {
            return;
        }
        let first = (start / step_secs).ceil();
        let last = (end / step_secs).floor();
        if !first.is_finite() || !last.is_finite() {
            return;
        }
        for index in (first as i64)..=(last as i64) {
            marks.push(GridMark {
                value: index as f64 * step_secs,
                step_size: step_secs,
            });
        }
    }

    fn seconds_spanning(&self, points: f32) -> f64 {
        f64::from(points) / self.points_per_second
    }
}

/// The finest ladder step at or above `minimum_secs`, and the longest one where the ladder
/// ends below it.
fn ladder_step_at_least(minimum_secs: f64) -> f64 {
    CLOCK_LADDER_SECS
        .into_iter()
        .find(|&step_secs| step_secs >= minimum_secs)
        .unwrap_or(LONGEST_LADDER_STEP_SECS)
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

/// The spacing egui_plot's own `grid_spacing` floor drops a grid line below, in points.
const MIN_GRID_SPACING_POINTS: f32 = 8.0;

/// How much coarser than the label step the strongest of the three levels is.
const COARSE_STEP_MULTIPLE: f64 = 4.0;

const MINUTE_SECS: f64 = 60.0;

const HOUR_SECS: f64 = 60.0 * MINUTE_SECS;

const DAY_SECS: f64 = 24.0 * HOUR_SECS;

/// The steps the x-axis marks the clock at, in seconds, ascending. Every step is a multiple of
/// the ones a reader of a clock counts in. A midnight in view always falls on a mark: every
/// step up to 12 hours divides a day.
const CLOCK_LADDER_SECS: [f64; 23] = [
    1.0,
    2.0,
    5.0,
    10.0,
    15.0,
    30.0,
    MINUTE_SECS,
    2.0 * MINUTE_SECS,
    5.0 * MINUTE_SECS,
    10.0 * MINUTE_SECS,
    15.0 * MINUTE_SECS,
    30.0 * MINUTE_SECS,
    HOUR_SECS,
    2.0 * HOUR_SECS,
    3.0 * HOUR_SECS,
    6.0 * HOUR_SECS,
    12.0 * HOUR_SECS,
    DAY_SECS,
    2.0 * DAY_SECS,
    5.0 * DAY_SECS,
    10.0 * DAY_SECS,
    20.0 * DAY_SECS,
    LONGEST_LADDER_STEP_SECS,
];

const LONGEST_LADDER_STEP_SECS: f64 = 50.0 * DAY_SECS;

/// A UTC date, the second line of a two-line label.
const DATE_FORMAT: &str = "%Y-%m-%d";

/// A UTC clock time to the minute, the first line of a label.
const CLOCK_FORMAT: &str = "%H:%M";

/// A UTC clock time to the second, the first line of a label while the label step is under a
/// minute.
const CLOCK_WITH_SECONDS_FORMAT: &str = "%H:%M:%S";

#[cfg(test)]
mod tests {
    use egui_plot::{GridInput, GridMark};

    use super::{DAY_SECS, HOUR_SECS, MIN_GRID_SPACING_POINTS, MINUTE_SECS, TimeAxisFrame};

    /// Plot width in points, the width a track plot has on a desktop window.
    const PLOT_WIDTH_POINTS: f32 = 960.0;

    /// 2024-01-15 12:00:00 UTC, where every view below starts.
    const VIEW_START_SECS: f64 = 1_705_320_000.0;

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

    #[rstest::rstest]
    #[case::two_minutes(2.0 * MINUTE_SECS, &[MINUTE_SECS, 10.0, 1.0])]
    #[case::thirty_hours(30.0 * HOUR_SECS, &[DAY_SECS, 12.0 * HOUR_SECS, 3.0 * HOUR_SECS, 15.0 * MINUTE_SECS])]
    #[case::nine_days(9.0 * DAY_SECS, &[5.0 * DAY_SECS, DAY_SECS, 2.0 * HOUR_SECS])]
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

    #[test]
    fn a_midnight_is_marked_once_at_the_day_step() {
        let frame = frame_over(30.0 * HOUR_SECS);
        let midnight_steps: Vec<f64> = frame
            .marks
            .iter()
            .filter(|mark| mark.value.rem_euclid(DAY_SECS) == 0.0)
            .map(|mark| mark.step_size)
            .collect();
        assert_eq!(midnight_steps, [DAY_SECS]);
    }

    #[test]
    fn the_first_labelled_mark_of_the_view_shows_the_date() {
        let frame = frame_over(30.0 * HOUR_SECS);
        let labels = labels(&frame);
        assert_eq!(
            labels.first().map(String::as_str),
            Some("12:00\n2024-01-15")
        );
        assert_eq!(labels.get(1).map(String::as_str), Some("15:00"));
    }

    #[test]
    fn a_midnight_shows_its_date_under_the_clock() {
        let frame = frame_over(30.0 * HOUR_SECS);
        assert!(
            labels(&frame).contains(&"00:00\n2024-01-16".to_owned()),
            "the midnight label shows the day it opens"
        );
    }

    #[test]
    fn a_label_step_under_a_minute_shows_the_seconds() {
        let frame = frame_over(2.0 * MINUTE_SECS);
        assert_eq!(labels(&frame).get(1).map(String::as_str), Some("12:00:10"));
    }

    #[test]
    fn a_span_of_days_labels_a_midnight_with_the_date_alone() {
        let frame = frame_over(9.0 * DAY_SECS);
        assert_eq!(
            labels(&frame).first().map(String::as_str),
            Some("2024-01-16")
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
}
