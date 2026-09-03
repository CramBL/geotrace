//! How strongly the line table draws each visible row's timestamp, and where a
//! new UTC day opens.
//!
//! Both are derived once per rebuild of the visible set: a row that is drawn
//! looks its tick up, and never walks the log for it.

use chrono::{DateTime, Utc};
use gt_logfile::{BootSession, ParsedLog};

use crate::filter::stack::VisibleEntries;

const SECONDS_PER_MINUTE: i64 = 60;

const SECONDS_PER_HOUR: i64 = 60 * SECONDS_PER_MINUTE;

const SECONDS_PER_DAY: i64 = 24 * SECONDS_PER_HOUR;

/// How strongly the line table draws one row's timestamp: the largest UTC
/// wall-clock field that differs from the previous shown row picks it.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord)]
pub enum TimestampTick {
    /// Only the seconds differ, or nothing does.
    #[default]
    Weak,

    /// The minute differs, the hour and the day do not.
    Plain,

    /// The hour or the day differs, or the row opens the table or a boot
    /// session.
    Strong,
}

/// The UTC day, hour and minute a timestamp falls in, each counted from the
/// epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClockFields {
    day: i64,
    hour: i64,
    minute: i64,
}

impl ClockFields {
    fn of(timestamp: DateTime<Utc>) -> Self {
        let seconds = timestamp.timestamp();
        Self {
            day: seconds.div_euclid(SECONDS_PER_DAY),
            hour: seconds.div_euclid(SECONDS_PER_HOUR),
            minute: seconds.div_euclid(SECONDS_PER_MINUTE),
        }
    }

    fn tick_after(self, previous: Self) -> TimestampTick {
        if self.hour != previous.hour {
            return TimestampTick::Strong;
        }
        match self.minute == previous.minute {
            true => TimestampTick::Weak,
            false => TimestampTick::Plain,
        }
    }
}

/// The divider row the line table opens a new UTC day with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DayDivider {
    /// The visible row of the first entry of the new day, which the divider is
    /// drawn above.
    pub visible_row: usize,

    /// Where the divider is drawn once every day divider above it has taken a
    /// row of its own. The line table adds the boot dividers on top of this.
    pub row_with_day_dividers: usize,
}

impl DayDivider {
    /// The divider above `visible_row`, following the `dividers_above` the
    /// same table already holds.
    pub fn following(dividers_above: &[Self], visible_row: usize) -> Self {
        Self {
            visible_row,
            row_with_day_dividers: visible_row.saturating_add(dividers_above.len()),
        }
    }
}

/// What the clock did along the visible rows of one log: the tick each row's
/// timestamp draws at, and the rows a day divider precedes.
///
/// The filters that pick the visible set also pick what "the previous row"
/// means: both are indexed by visible row.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClockTicks {
    row_ticks: Vec<TimestampTick>,
    day_dividers: Vec<DayDivider>,
}

impl ClockTicks {
    pub(crate) fn of(log: &ParsedLog, visible: &VisibleEntries) -> Self {
        let entries = log.entries();
        let mut sessions = log.boot_sessions().iter();
        let mut session = sessions.next();
        let mut row_ticks = Vec::with_capacity(visible.len());
        let mut day_dividers = Vec::new();
        let mut previous: Option<ClockFields> = None;
        for (visible_row, entry_index) in visible.entry_indices().enumerate() {
            let mut opens_a_session = previous.is_none();
            while session.is_some_and(|open: &BootSession| open.entry_range.end <= entry_index) {
                session = sessions.next();
                opens_a_session = true;
            }
            let Some(entry) = entries.get(entry_index) else {
                row_ticks.push(TimestampTick::default());
                continue;
            };
            let fields = ClockFields::of(entry.timestamp);
            let tick = match (previous, opens_a_session) {
                (Some(previous), false) => fields.tick_after(previous),
                _ => TimestampTick::Strong,
            };
            if previous.is_some_and(|previous| previous.day != fields.day) {
                day_dividers.push(DayDivider::following(&day_dividers, visible_row));
            }
            row_ticks.push(tick);
            previous = Some(fields);
        }
        Self {
            row_ticks,
            day_dividers,
        }
    }

    pub fn tick(&self, visible_row: usize) -> TimestampTick {
        self.row_ticks.get(visible_row).copied().unwrap_or_default()
    }

    /// The day dividers of the table, by ascending visible row.
    pub fn day_dividers(&self) -> &[DayDivider] {
        &self.day_dividers
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use chrono::NaiveDateTime;
    use rstest::rstest;

    use super::*;
    use crate::{FilterStack, test_fixtures};

    /// A minute boundary, an hour boundary and a day boundary, each reached
    /// from the second before it.
    const START: &str = "2026-01-01 14:02:11";

    fn at(text: &str) -> DateTime<Utc> {
        NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S")
            .expect("the fixture timestamp parses")
            .and_utc()
    }

    #[rstest]
    #[case::the_same_second(START, START, TimestampTick::Weak)]
    #[case::another_second(START, "2026-01-01 14:02:12", TimestampTick::Weak)]
    #[case::another_minute(START, "2026-01-01 14:03:00", TimestampTick::Plain)]
    #[case::another_hour(START, "2026-01-01 15:00:00", TimestampTick::Strong)]
    #[case::another_day(START, "2026-01-02 14:02:11", TimestampTick::Strong)]
    #[case::the_clock_stepping_back_a_minute(START, "2026-01-01 14:01:59", TimestampTick::Plain)]
    #[case::the_clock_stepping_back_a_day(START, "2025-12-31 14:02:11", TimestampTick::Strong)]
    fn the_largest_wall_clock_field_that_differs_picks_the_tick(
        #[case] previous: &str,
        #[case] shown: &str,
        #[case] expected: TimestampTick,
    ) {
        assert_eq!(
            ClockFields::of(at(shown)).tick_after(ClockFields::of(at(previous))),
            expected
        );
    }

    fn ticks_of(text: &str) -> ClockTicks {
        let log = Arc::new(test_fixtures::parsed_log_of_text(text));
        let mut stack = FilterStack::new(log);
        stack.wait_for_queries();
        stack.clock_ticks().clone()
    }

    fn ticks_by_row(ticks: &ClockTicks) -> Vec<TimestampTick> {
        ticks.row_ticks.clone()
    }

    fn day_divider_rows(ticks: &ClockTicks) -> Vec<usize> {
        ticks
            .day_dividers()
            .iter()
            .map(|separator| separator.visible_row)
            .collect()
    }

    /// One log read row by row: the table opens strong, dims where only the
    /// seconds move, and lights up again at the minute and the hour.
    #[test]
    fn every_row_takes_the_tick_of_its_step_from_the_row_above() {
        let ticks = ticks_of(
            "\
2026-01-01 14:02:11 navsyncd: gnss fix acquired
2026-01-01 14:02:12 navsyncd: gnss fix lost
2026-01-01 14:03:12 navsyncd: gnss fix acquired
2026-01-01 15:03:12 navsyncd: gnss fix lost
",
        );

        assert_eq!(
            ticks_by_row(&ticks),
            [
                TimestampTick::Strong,
                TimestampTick::Weak,
                TimestampTick::Plain,
                TimestampTick::Strong,
            ]
        );
        assert_eq!(day_divider_rows(&ticks), Vec::<usize>::new());
    }

    /// The row above is the row the table draws above: a filter that hides the
    /// line in between makes the step across it the one that counts.
    #[test]
    fn a_filtered_table_steps_from_the_row_the_filter_left_above() {
        let text = "\
2026-01-01 14:02:11 navsyncd: gnss fix acquired
2026-01-01 14:03:11 hal-powerd: battery low
2026-01-01 14:03:12 navsyncd: gnss fix lost
";
        assert_eq!(
            ticks_by_row(&ticks_of(text)),
            [
                TimestampTick::Strong,
                TimestampTick::Plain,
                TimestampTick::Weak,
            ]
        );

        let mut stack = FilterStack::new(Arc::new(test_fixtures::parsed_log_of_text(text)));
        stack.set_live_filter_text("gnss");
        stack.wait_for_queries();

        assert_eq!(
            ticks_by_row(stack.clock_ticks()),
            [TimestampTick::Strong, TimestampTick::Plain],
            "the two fix lines sit a minute apart once the battery line is filtered out"
        );
    }

    #[test]
    fn a_log_crossing_midnight_opens_a_day_divider_above_the_first_line_of_the_new_day() {
        let ticks = ticks_of(
            "\
2026-01-01 23:59:58 navsyncd: gnss fix acquired
2026-01-01 23:59:59 navsyncd: gnss fix lost
2026-01-02 00:00:01 navsyncd: gnss fix acquired
2026-01-02 00:00:02 navsyncd: gnss fix lost
",
        );

        assert_eq!(day_divider_rows(&ticks), [2]);
        assert_eq!(
            ticks.tick(2),
            TimestampTick::Strong,
            "the first line of a new day differs in its day"
        );
        assert_eq!(ticks.tick(3), TimestampTick::Weak);
    }

    /// A clock corrected back past midnight is compared like any other step:
    /// the line it lands on opens the day it names.
    #[test]
    fn a_backwards_step_across_midnight_opens_a_day_divider_too() {
        let ticks = ticks_of(
            "\
2026-01-02 00:00:01 navsyncd: gnss fix acquired
2026-01-01 23:59:58 navsyncd: gnss fix lost
",
        );

        assert_eq!(day_divider_rows(&ticks), [1]);
    }

    /// The divider opens the day at the first line of it the table draws: a
    /// filter that hides that line moves the divider down to the next one, and
    /// one that hides the whole day leaves no divider at all.
    #[test]
    fn a_filter_moves_the_day_divider_to_the_first_line_of_the_day_it_leaves() {
        let text = "\
2026-01-01 23:59:58 navsyncd: gnss fix acquired
2026-01-02 00:00:01 hal-powerd: battery low
2026-01-02 00:00:02 navsyncd: gnss fix lost
";
        assert_eq!(day_divider_rows(&ticks_of(text)), [1]);

        let log = Arc::new(test_fixtures::parsed_log_of_text(text));
        let mut stack = FilterStack::new(Arc::clone(&log));
        stack.set_live_filter_text("navsyncd");
        stack.wait_for_queries();

        assert_eq!(
            day_divider_rows(stack.clock_ticks()),
            [1],
            "the fix lost line is the first of the new day the filter left"
        );

        let mut stack = FilterStack::new(log);
        stack.set_live_filter_text("acquired");
        stack.wait_for_queries();

        assert_eq!(
            day_divider_rows(stack.clock_ticks()),
            Vec::<usize>::new(),
            "the filter left no line of the second day"
        );
    }

    /// A boot session opening on a new day takes both dividers: the day
    /// divider names the date, the boot divider the run.
    #[test]
    fn the_first_line_of_a_boot_session_is_strong_and_opens_its_day() {
        let ticks = ticks_of(
            "\
2026-01-01 23:59:58 navsyncd: gnss fix acquired
--- Device reboot ---
2026-01-02 00:00:01 navsyncd: starting
2026-01-02 00:00:02 navsyncd: gnss fix acquired
",
        );

        assert_eq!(day_divider_rows(&ticks), [1]);
        assert_eq!(
            ticks_by_row(&ticks),
            [
                TimestampTick::Strong,
                TimestampTick::Strong,
                TimestampTick::Weak,
            ]
        );
    }

    /// A boot session inside the same minute still opens strong: the reader
    /// looks for where each run starts.
    #[test]
    fn a_boot_session_starting_in_the_same_minute_opens_strong() {
        let ticks = ticks_of(
            "\
2026-01-01 14:02:11 navsyncd: gnss fix acquired
--- Device reboot ---
2026-01-01 14:02:12 navsyncd: starting
",
        );

        assert_eq!(
            ticks_by_row(&ticks),
            [TimestampTick::Strong, TimestampTick::Strong]
        );
        assert_eq!(day_divider_rows(&ticks), Vec::<usize>::new());
    }

    /// Every divider states where it is drawn among the visible rows and the
    /// dividers above it, which is what the table resolves its rows with.
    #[test]
    fn each_day_divider_states_the_row_it_is_drawn_at() {
        let days: String = (1..=3)
            .map(|day| format!("2026-01-0{day} 14:02:11 navsyncd: gnss fix acquired\n"))
            .collect();

        let ticks = ticks_of(&days);

        assert_eq!(
            ticks.day_dividers(),
            [
                DayDivider {
                    visible_row: 1,
                    row_with_day_dividers: 1,
                },
                DayDivider {
                    visible_row: 2,
                    row_with_day_dividers: 3,
                },
            ]
        );
    }

    #[test]
    fn a_log_of_one_line_draws_it_strong_and_opens_no_day() {
        let ticks = ticks_of("2026-01-01 14:02:11 navsyncd: gnss fix acquired\n");

        assert_eq!(ticks_by_row(&ticks), [TimestampTick::Strong]);
        assert_eq!(day_divider_rows(&ticks), Vec::<usize>::new());
    }

    #[test]
    fn a_table_the_filters_emptied_has_no_row_to_tick() {
        let log = Arc::new(test_fixtures::parsed_log_of_text(
            "2026-01-01 14:02:11 navsyncd: gnss fix acquired\n",
        ));
        let mut stack = FilterStack::new(log);

        stack.set_live_filter_text("hal-powerd");
        stack.wait_for_queries();

        assert_eq!(
            ticks_by_row(stack.clock_ticks()),
            Vec::<TimestampTick>::new()
        );
        assert_eq!(day_divider_rows(stack.clock_ticks()), Vec::<usize>::new());
    }

    /// The tick of a row the table does not draw: the lookup stays inside the
    /// visible set.
    #[test]
    fn a_row_past_the_visible_set_is_weak() {
        let ticks = ticks_of("2026-01-01 14:02:11 navsyncd: gnss fix acquired\n");

        assert_eq!(ticks.tick(9), TimestampTick::Weak);
    }

    /// The unfiltered stack ticks its rows the moment the log is loaded.
    #[test]
    fn a_freshly_loaded_log_has_a_tick_for_every_line() {
        let log = Arc::new(test_fixtures::parsed_log(3));

        let stack = FilterStack::new(log);

        assert_eq!(
            ticks_by_row(stack.clock_ticks()),
            [
                TimestampTick::Strong,
                TimestampTick::Weak,
                TimestampTick::Weak,
            ],
            "the fixture writes one entry a second"
        );
    }
}
