//! Which day the overlay shows, and what to say when it has nothing to draw.

use chrono::{Days, NaiveDate};

use crate::calendar::{self, COVERAGE_START, DayOutlook};

/// Why the overlay is drawing nothing, and what the legend says about it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, strum::EnumCount, strum::EnumIter, strum::IntoStaticStr,
)]
#[strum(serialize_all = "snake_case")]
pub enum EmptyReason {
    /// No track is loaded, so no day was picked.
    NoTrack,
    /// Earlier than [`COVERAGE_START`].
    BeforeCoverage,
    /// Later than today, UTC.
    InFuture,
    /// Inside the coverage window, recent enough that the host is not
    /// expected to have published it.
    AwaitingPublication,
    /// Inside the coverage window and old enough to have been published,
    /// but the archive has nothing.
    NotFetched,
    /// The host answered that it has no dataset for the day.
    NotPublished,
}

impl EmptyReason {
    /// Two or three words for the display-toggle row, where the full
    /// message does not fit beside the layer's name. [`Self::message`] is
    /// the row's hover text.
    pub const fn badge(self) -> &'static str {
        match self {
            Self::NoTrack => "no day",
            Self::BeforeCoverage => "before coverage",
            Self::InFuture => "future day",
            Self::AwaitingPublication => "not published yet",
            Self::NotFetched => "not downloaded",
            Self::NotPublished => "none published",
        }
    }

    /// The full sentence, for the row's hover text.
    pub fn message(self) -> String {
        match self {
            Self::NoTrack => "Load a recording to choose a day".to_owned(),
            Self::BeforeCoverage => format!("No data was published before {COVERAGE_START}"),
            Self::InFuture => "This day has not happened yet".to_owned(),
            Self::AwaitingPublication => {
                "Not published yet - the source runs a few days behind".to_owned()
            }
            Self::NotFetched => "Not downloaded yet".to_owned(),
            Self::NotPublished => "The source published nothing for this day".to_owned(),
        }
    }
}

/// The day the overlay shows and how it can be stepped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DaySelection {
    /// The day being shown. [`None`] before any track is loaded.
    day: Option<NaiveDate>,
    /// Today, UTC, as the clamp's upper bound.
    today: NaiveDate,
    /// Whether the user has moved the selection themselves.
    stepped: bool,
}

impl DaySelection {
    pub const fn new(day: Option<NaiveDate>, today: NaiveDate) -> Self {
        Self {
            day,
            today,
            stepped: false,
        }
    }

    pub const fn day(&self) -> Option<NaiveDate> {
        self.day
    }

    /// Adopt `day` if it is earlier than the day already shown.
    ///
    /// Once the user steps, later calls are ignored. Taking the minimum
    /// keeps the day the same whichever order concurrent loads finish in.
    pub fn adopt_default(&mut self, day: NaiveDate) {
        if self.stepped {
            return;
        }
        self.day = Some(self.day.map_or(day, |current| current.min(day)));
    }

    /// The day one step back, or [`None`] at the start of coverage.
    pub fn previous(&self) -> Option<NaiveDate> {
        let previous = self.day?.checked_sub_days(Days::new(1))?;
        (previous >= COVERAGE_START).then_some(previous)
    }

    /// The day one step forward, or [`None`] at today.
    pub fn next(&self) -> Option<NaiveDate> {
        let next = self.day?.checked_add_days(Days::new(1))?;
        (next <= self.today).then_some(next)
    }

    /// Step back one day, if there is one.
    pub fn step_back(&mut self) {
        if let Some(previous) = self.previous() {
            self.day = Some(previous);
            self.stepped = true;
        }
    }

    /// Step forward one day, if there is one.
    pub fn step_forward(&mut self) {
        if let Some(next) = self.next() {
            self.day = Some(next);
            self.stepped = true;
        }
    }

    /// Why the overlay has nothing to draw, given whether the archive holds
    /// the day and whether the host refused it.
    ///
    /// [`None`] when the day is archived and has cells.
    pub fn empty_reason(&self, archived_cells: usize, host_refused: bool) -> Option<EmptyReason> {
        let Some(day) = self.day else {
            return Some(EmptyReason::NoTrack);
        };
        if archived_cells > 0 {
            return None;
        }
        Some(match calendar::day_outlook(day, self.today) {
            DayOutlook::BeforeCoverage => EmptyReason::BeforeCoverage,
            DayOutlook::InFuture => EmptyReason::InFuture,
            DayOutlook::Fetchable if host_refused => {
                if calendar::awaiting_publication(day, self.today) {
                    EmptyReason::AwaitingPublication
                } else {
                    EmptyReason::NotPublished
                }
            }
            DayOutlook::Fetchable => EmptyReason::NotFetched,
        })
    }

    /// Hover text for a stepper button that cannot move further back.
    pub fn earliest_day_text() -> String {
        EmptyReason::BeforeCoverage.message()
    }

    /// Hover text for a stepper button that cannot move further forward.
    pub fn latest_day_text() -> String {
        "Later days have not happened yet".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use rstest::rstest;
    use strum::{EnumCount as _, IntoEnumIterator as _};

    use super::*;

    /// Longest badge the display-toggle row has space for.
    const MAX_BADGE_CHARS: usize = 18;

    fn date(year: i32, month: u32, day: u32) -> NaiveDate {
        NaiveDate::from_ymd_opt(year, month, day).unwrap()
    }

    fn today() -> NaiveDate {
        date(2026, 7, 31)
    }

    fn selection(day: NaiveDate) -> DaySelection {
        DaySelection::new(Some(day), today())
    }

    #[test]
    fn the_first_loaded_track_picks_the_day() {
        let mut selection = DaySelection::new(None, today());
        assert_eq!(selection.day(), None);
        selection.adopt_default(date(2026, 7, 20));
        assert_eq!(selection.day(), Some(date(2026, 7, 20)));
    }

    /// A second track must not move the overlay off the day the user chose.
    #[test]
    fn a_later_track_does_not_move_a_chosen_day() {
        let mut selection = DaySelection::new(None, today());
        selection.adopt_default(date(2026, 7, 20));
        selection.step_back();
        assert_eq!(selection.day(), Some(date(2026, 7, 19)));

        selection.adopt_default(date(2026, 7, 25));
        assert_eq!(selection.day(), Some(date(2026, 7, 19)));
    }

    /// Files load on their own threads and finish in any order, so the day
    /// must not depend on which arrived first.
    #[rstest]
    #[case::ascending([date(2026, 7, 20), date(2026, 7, 25)])]
    #[case::descending([date(2026, 7, 25), date(2026, 7, 20)])]
    fn concurrent_loads_settle_on_the_earliest_day(#[case] order: [NaiveDate; 2]) {
        let mut selection = DaySelection::new(None, today());
        for day in order {
            selection.adopt_default(day);
        }
        assert_eq!(selection.day(), Some(date(2026, 7, 20)));
    }

    /// Stepping to the coverage edge still counts as stepping.
    #[test]
    fn a_step_that_cannot_move_does_not_lock_the_default() {
        let mut selection = DaySelection::new(Some(COVERAGE_START), today());
        selection.step_back();
        selection.adopt_default(date(2026, 7, 20));
        assert_eq!(
            selection.day(),
            Some(COVERAGE_START),
            "a refused step leaves the day, and an earlier default still wins"
        );
    }

    #[test]
    fn stepping_moves_one_day_at_a_time() {
        let mut selection = selection(date(2026, 7, 20));
        selection.step_forward();
        assert_eq!(selection.day(), Some(date(2026, 7, 21)));
        selection.step_back();
        selection.step_back();
        assert_eq!(selection.day(), Some(date(2026, 7, 19)));
    }

    #[test]
    fn stepping_stops_at_the_start_of_coverage() {
        let mut selection = selection(COVERAGE_START);
        assert_eq!(selection.previous(), None);
        selection.step_back();
        assert_eq!(selection.day(), Some(COVERAGE_START));
    }

    #[test]
    fn stepping_stops_at_today() {
        let mut selection = selection(today());
        assert_eq!(selection.next(), None);
        selection.step_forward();
        assert_eq!(selection.day(), Some(today()));
    }

    #[rstest]
    #[case::no_track(None, 0, false, Some(EmptyReason::NoTrack))]
    #[case::archived(Some(date(2026, 7, 20)), 44_546, false, None)]
    #[case::before_coverage(Some(date(2020, 1, 1)), 0, false, Some(EmptyReason::BeforeCoverage))]
    #[case::in_future(Some(date(2027, 1, 1)), 0, false, Some(EmptyReason::InFuture))]
    #[case::not_fetched(Some(date(2026, 7, 20)), 0, false, Some(EmptyReason::NotFetched))]
    #[case::awaiting(
        Some(date(2026, 7, 30)),
        0,
        true,
        Some(EmptyReason::AwaitingPublication)
    )]
    #[case::not_published(Some(date(2026, 7, 20)), 0, true, Some(EmptyReason::NotPublished))]
    fn every_empty_state_has_its_own_reason(
        #[case] day: Option<NaiveDate>,
        #[case] archived_cells: usize,
        #[case] host_refused: bool,
        #[case] expected: Option<EmptyReason>,
    ) {
        let selection = DaySelection::new(day, today());
        assert_eq!(
            selection.empty_reason(archived_cells, host_refused),
            expected
        );
    }

    /// A variant cannot be added without a case that reaches it.
    #[test]
    fn every_reason_is_reachable() {
        let reached: HashSet<&'static str> = [
            DaySelection::new(None, today()).empty_reason(0, false),
            DaySelection::new(Some(date(2020, 1, 1)), today()).empty_reason(0, false),
            DaySelection::new(Some(date(2027, 1, 1)), today()).empty_reason(0, false),
            DaySelection::new(Some(date(2026, 7, 30)), today()).empty_reason(0, true),
            DaySelection::new(Some(date(2026, 7, 20)), today()).empty_reason(0, false),
            DaySelection::new(Some(date(2026, 7, 20)), today()).empty_reason(0, true),
        ]
        .into_iter()
        .flatten()
        .map(<&'static str>::from)
        .collect();
        assert_eq!(reached.len(), EmptyReason::COUNT);
    }

    /// Every message is distinct: an empty map must say which empty it is.
    #[test]
    fn every_message_is_distinct() {
        let messages: HashSet<String> = EmptyReason::iter().map(EmptyReason::message).collect();
        assert_eq!(messages.len(), EmptyReason::COUNT);
    }

    /// A badge has to fit beside the layer's name and still say which empty
    /// state it is.
    #[test]
    fn every_badge_is_distinct_and_short() {
        let badges: HashSet<&str> = EmptyReason::iter().map(EmptyReason::badge).collect();
        assert_eq!(badges.len(), EmptyReason::COUNT);
        for badge in badges {
            assert!(
                badge.len() <= MAX_BADGE_CHARS,
                "{badge:?} is too long for the row"
            );
        }
    }

    #[test]
    fn empty_messages() {
        let wording: Vec<String> = EmptyReason::iter()
            .map(|reason| {
                format!(
                    "{}: [{}] {}",
                    <&'static str>::from(reason),
                    reason.badge(),
                    reason.message()
                )
            })
            .collect();
        insta::assert_debug_snapshot!("empty_messages", wording);
    }
}
