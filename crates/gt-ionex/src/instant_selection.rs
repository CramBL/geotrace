//! Which instant the map heatmap shows, and what to say when it has nothing
//! to draw.

use chrono::{DateTime, NaiveDate, NaiveTime, TimeDelta, Utc};

use crate::calendar::{self, COVERAGE_START, DayOutlook};

/// Time between maps the stepper moves by until an archived day declares its
/// own interval. JPL's final product publishes a map every two hours.
pub const DEFAULT_MAP_INTERVAL: TimeDelta = TimeDelta::hours(2);

/// Why the heatmap is drawing nothing, and what the display toggle says about
/// it.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, strum::EnumCount, strum::EnumIter, strum::IntoStaticStr,
)]
#[strum(serialize_all = "snake_case")]
pub enum TecEmptyReason {
    /// No track is loaded, so no instant was picked.
    NoTrack,
    /// Earlier than [`COVERAGE_START`].
    BeforeCoverage,
    /// Later than today, UTC.
    InFuture,
    /// Inside the coverage window, but the archive has no maps for the day.
    NotArchived,
}

impl TecEmptyReason {
    /// Two or three words for the display-toggle row, where the full message
    /// does not fit beside the layer's name. [`Self::message`] is the row's
    /// hover text.
    pub const fn badge(self) -> &'static str {
        match self {
            Self::NoTrack => "no instant",
            Self::BeforeCoverage => "before coverage",
            Self::InFuture => "future instant",
            Self::NotArchived => "not downloaded",
        }
    }

    /// The full sentence, for the row's hover text.
    pub fn message(self) -> String {
        match self {
            Self::NoTrack => "Load a recording to choose an instant".to_owned(),
            Self::BeforeCoverage => format!("No maps were published before {COVERAGE_START}"),
            Self::InFuture => "This instant has not happened yet".to_owned(),
            Self::NotArchived => "No maps downloaded for this day yet".to_owned(),
        }
    }
}

/// The instant the heatmap draws, and what put it there.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShownInstant {
    /// The time of the fix under the pointer, or of the one clicked, which the
    /// heatmap follows for as long as that fix stays hovered or selected.
    Followed(DateTime<Utc>),
    /// The instant the stepper holds.
    Stepped(DateTime<Utc>),
}

impl ShownInstant {
    pub const fn instant(self) -> DateTime<Utc> {
        match self {
            Self::Followed(instant) | Self::Stepped(instant) => instant,
        }
    }

    pub const fn is_followed(self) -> bool {
        matches!(self, Self::Followed(_))
    }
}

/// The instant the heatmap shows and how it can be stepped.
///
/// A hovered or selected fix wins over the stepper for as long as it lasts, so
/// the heatmap shows the ionosphere the pointed-at fix was recorded under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TecInstantSelection {
    /// Where the stepper stands. [`None`] before any track is loaded.
    stepped: Option<DateTime<Utc>>,
    /// The hovered or selected fix's time, rewritten every frame.
    followed: Option<DateTime<Utc>>,
    /// Time between the maps of the day being shown, which one step covers.
    interval: TimeDelta,
    /// Today, UTC, as the clamp's upper bound.
    today: NaiveDate,
    /// Whether the user has moved the stepper themselves.
    moved: bool,
}

impl TecInstantSelection {
    pub const fn new(instant: Option<DateTime<Utc>>, today: NaiveDate) -> Self {
        Self {
            stepped: instant,
            followed: None,
            interval: DEFAULT_MAP_INTERVAL,
            today,
            moved: false,
        }
    }

    /// The instant to draw, and whether a fix is driving it.
    pub const fn shown(&self) -> Option<ShownInstant> {
        match (self.followed, self.stepped) {
            (Some(instant), _) => Some(ShownInstant::Followed(instant)),
            (None, Some(instant)) => Some(ShownInstant::Stepped(instant)),
            (None, None) => None,
        }
    }

    pub const fn instant(&self) -> Option<DateTime<Utc>> {
        match self.shown() {
            Some(shown) => Some(shown.instant()),
            None => None,
        }
    }

    /// Adopt `instant` if it is earlier than the one the stepper holds.
    ///
    /// Once the user steps, later calls are ignored. Taking the minimum keeps
    /// the instant the same whichever order concurrent loads finish in.
    pub fn adopt_default(&mut self, instant: DateTime<Utc>) {
        if self.moved {
            return;
        }
        self.stepped = Some(self.stepped.map_or(instant, |current| current.min(instant)));
    }

    /// Show `instant` for as long as the fix it belongs to stays hovered or
    /// selected. [`None`] hands the heatmap back to the stepper.
    pub const fn follow(&mut self, instant: Option<DateTime<Utc>>) {
        self.followed = instant;
    }

    /// Move one step by the interval the shown day's maps are published at.
    /// A non-positive interval is rejected, which keeps a corrupt archived day
    /// from stalling the stepper.
    pub fn set_map_interval(&mut self, interval: TimeDelta) {
        if interval > TimeDelta::zero() {
            self.interval = interval;
        }
    }

    /// The map epoch one step back, or [`None`] at the start of coverage.
    ///
    /// An instant between two epochs steps back onto the earlier one: the
    /// stepper lands on the epochs the producer published.
    pub fn previous(&self) -> Option<DateTime<Utc>> {
        let instant = self.stepped?;
        let epoch = self.epoch_at_or_before(instant);
        let previous = if epoch < instant {
            epoch
        } else {
            instant.checked_sub_signed(self.interval)?
        };
        (previous.date_naive() >= COVERAGE_START).then_some(previous)
    }

    /// The map epoch one step forward, or [`None`] at today.
    pub fn next(&self) -> Option<DateTime<Utc>> {
        let instant = self.stepped?;
        let next = self
            .epoch_at_or_before(instant)
            .checked_add_signed(self.interval)?;
        (next.date_naive() <= self.today).then_some(next)
    }

    pub fn step_back(&mut self) {
        if let Some(previous) = self.previous() {
            self.stepped = Some(previous);
            self.moved = true;
        }
    }

    pub fn step_forward(&mut self) {
        if let Some(next) = self.next() {
            self.stepped = Some(next);
            self.moved = true;
        }
    }

    /// The published epoch at or before `instant`: the epochs of a day start
    /// at its midnight, one interval apart.
    fn epoch_at_or_before(&self, instant: DateTime<Utc>) -> DateTime<Utc> {
        let day_start = instant.date_naive().and_time(NaiveTime::MIN).and_utc();
        let interval_seconds = self.interval.num_seconds().max(1);
        let elapsed = instant
            .signed_duration_since(day_start)
            .num_seconds()
            .div_euclid(interval_seconds);
        day_start
            .checked_add_signed(TimeDelta::seconds(elapsed.saturating_mul(interval_seconds)))
            .unwrap_or(day_start)
    }

    /// Why the heatmap has nothing to draw, given how many grid nodes the
    /// archived day holds.
    ///
    /// [`None`] when the shown instant's day is archived.
    pub fn empty_reason(&self, archived_nodes: usize) -> Option<TecEmptyReason> {
        let Some(shown) = self.instant() else {
            return Some(TecEmptyReason::NoTrack);
        };
        if archived_nodes > 0 {
            return None;
        }
        Some(
            match calendar::day_outlook(shown.date_naive(), self.today) {
                DayOutlook::BeforeCoverage => TecEmptyReason::BeforeCoverage,
                DayOutlook::InFuture => TecEmptyReason::InFuture,
                DayOutlook::Fetchable => TecEmptyReason::NotArchived,
            },
        )
    }

    /// Hover text for a stepper button that cannot move further back.
    pub fn earliest_instant_text() -> String {
        TecEmptyReason::BeforeCoverage.message()
    }

    /// Hover text for a stepper button that cannot move further forward.
    pub fn latest_instant_text() -> String {
        "Later instants have not happened yet".to_owned()
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;

    use rstest::rstest;
    use strum::{EnumCount as _, IntoEnumIterator as _};

    use super::*;

    fn today() -> NaiveDate {
        NaiveDate::from_ymd_opt(2026, 8, 17).unwrap()
    }

    fn at(year: i32, month: u32, day: u32, hour: u32, minute: u32) -> DateTime<Utc> {
        NaiveDate::from_ymd_opt(year, month, day)
            .and_then(|date| date.and_hms_opt(hour, minute, 0))
            .unwrap()
            .and_utc()
    }

    fn selection(instant: DateTime<Utc>) -> TecInstantSelection {
        TecInstantSelection::new(Some(instant), today())
    }

    #[test]
    fn the_first_loaded_track_picks_the_instant() {
        let mut selection = TecInstantSelection::new(None, today());
        assert_eq!(selection.instant(), None);
        selection.adopt_default(at(2024, 5, 10, 18, 37));
        assert_eq!(selection.instant(), Some(at(2024, 5, 10, 18, 37)));
    }

    /// Files load on their own threads and finish in any order, so the instant
    /// must not depend on which arrived first.
    #[rstest]
    #[case::ascending([at(2024, 5, 10, 8, 0), at(2024, 5, 12, 9, 0)])]
    #[case::descending([at(2024, 5, 12, 9, 0), at(2024, 5, 10, 8, 0)])]
    fn concurrent_loads_settle_on_the_earliest_instant(#[case] order: [DateTime<Utc>; 2]) {
        let mut selection = TecInstantSelection::new(None, today());
        for instant in order {
            selection.adopt_default(instant);
        }
        assert_eq!(selection.instant(), Some(at(2024, 5, 10, 8, 0)));
    }

    #[test]
    fn a_later_load_does_not_move_a_stepped_instant() {
        let mut selection = selection(at(2024, 5, 10, 18, 0));
        selection.step_back();
        assert_eq!(selection.instant(), Some(at(2024, 5, 10, 16, 0)));

        selection.adopt_default(at(2024, 5, 9, 8, 0));
        assert_eq!(selection.instant(), Some(at(2024, 5, 10, 16, 0)));
    }

    /// An instant between two epochs steps onto the epoch grid.
    #[rstest]
    #[case::back_from_between_epochs(at(2024, 5, 10, 18, 37), true, at(2024, 5, 10, 18, 0))]
    #[case::back_from_an_epoch(at(2024, 5, 10, 18, 0), true, at(2024, 5, 10, 16, 0))]
    #[case::forward_from_between_epochs(at(2024, 5, 10, 18, 37), false, at(2024, 5, 10, 20, 0))]
    #[case::forward_from_an_epoch(at(2024, 5, 10, 18, 0), false, at(2024, 5, 10, 20, 0))]
    #[case::back_across_midnight(at(2024, 5, 10, 0, 0), true, at(2024, 5, 9, 22, 0))]
    #[case::forward_across_midnight(at(2024, 5, 10, 23, 0), false, at(2024, 5, 11, 0, 0))]
    fn stepping_lands_on_the_published_epochs(
        #[case] from: DateTime<Utc>,
        #[case] back: bool,
        #[case] expected: DateTime<Utc>,
    ) {
        let mut selection = selection(from);
        if back {
            selection.step_back();
        } else {
            selection.step_forward();
        }
        assert_eq!(selection.instant(), Some(expected));
    }

    /// A rapid day publishes hourly, so one step covers one hour.
    #[test]
    fn the_step_follows_the_archived_days_interval() {
        let mut selection = selection(at(2024, 5, 10, 18, 0));
        selection.set_map_interval(TimeDelta::hours(1));
        selection.step_back();
        assert_eq!(selection.instant(), Some(at(2024, 5, 10, 17, 0)));
    }

    #[rstest]
    #[case::zero(TimeDelta::zero())]
    #[case::negative(TimeDelta::hours(-2))]
    fn an_interval_that_is_not_a_step_is_rejected(#[case] interval: TimeDelta) {
        let mut selection = selection(at(2024, 5, 10, 18, 0));
        selection.set_map_interval(interval);
        selection.step_back();
        assert_eq!(selection.instant(), Some(at(2024, 5, 10, 16, 0)));
    }

    #[test]
    fn stepping_stops_at_the_start_of_coverage() {
        let coverage_start = COVERAGE_START.and_time(NaiveTime::MIN).and_utc();
        let mut selection = selection(coverage_start);
        assert_eq!(selection.previous(), None);
        selection.step_back();
        assert_eq!(selection.instant(), Some(coverage_start));
    }

    #[test]
    fn stepping_stops_at_the_end_of_today() {
        let mut selection = selection(at(2026, 8, 17, 23, 0));
        assert_eq!(selection.next(), None);
        selection.step_forward();
        assert_eq!(selection.instant(), Some(at(2026, 8, 17, 23, 0)));
    }

    /// A hovered fix wins over the stepper, and letting go hands the heatmap
    /// back to where the stepper stood.
    #[test]
    fn a_followed_fix_wins_over_the_stepper() {
        let mut selection = selection(at(2024, 5, 10, 18, 0));
        selection.follow(Some(at(2024, 5, 10, 6, 30)));
        assert_eq!(
            selection.shown(),
            Some(ShownInstant::Followed(at(2024, 5, 10, 6, 30)))
        );

        selection.follow(None);
        assert_eq!(
            selection.shown(),
            Some(ShownInstant::Stepped(at(2024, 5, 10, 18, 0)))
        );
        assert!(!selection.shown().unwrap().is_followed());
    }

    /// A fix hovered before any recording set the stepper still draws.
    #[test]
    fn a_followed_fix_alone_is_enough_to_draw() {
        let mut selection = TecInstantSelection::new(None, today());
        selection.follow(Some(at(2024, 5, 10, 6, 30)));
        assert_eq!(selection.instant(), Some(at(2024, 5, 10, 6, 30)));
    }

    #[rstest]
    #[case::no_instant(None, 0, Some(TecEmptyReason::NoTrack))]
    #[case::archived(Some(at(2024, 5, 10, 18, 0)), 5183, None)]
    #[case::before_coverage(Some(at(2005, 1, 1, 0, 0)), 0, Some(TecEmptyReason::BeforeCoverage))]
    #[case::in_future(Some(at(2027, 1, 1, 0, 0)), 0, Some(TecEmptyReason::InFuture))]
    #[case::not_archived(Some(at(2024, 5, 10, 18, 0)), 0, Some(TecEmptyReason::NotArchived))]
    fn every_empty_state_has_its_own_reason(
        #[case] instant: Option<DateTime<Utc>>,
        #[case] archived_nodes: usize,
        #[case] expected: Option<TecEmptyReason>,
    ) {
        let selection = TecInstantSelection::new(instant, today());
        assert_eq!(selection.empty_reason(archived_nodes), expected);
    }

    /// A variant cannot be added without a case that reaches it.
    #[test]
    fn every_reason_is_reachable() {
        let reached: HashSet<&'static str> = [
            TecInstantSelection::new(None, today()).empty_reason(0),
            selection(at(2005, 1, 1, 0, 0)).empty_reason(0),
            selection(at(2027, 1, 1, 0, 0)).empty_reason(0),
            selection(at(2024, 5, 10, 18, 0)).empty_reason(0),
        ]
        .into_iter()
        .flatten()
        .map(<&'static str>::from)
        .collect();
        assert_eq!(reached.len(), TecEmptyReason::COUNT);
    }

    /// An empty map must say which empty it is, in a badge that fits the row.
    #[test]
    fn every_badge_and_message_is_distinct() {
        let badges: HashSet<&str> = TecEmptyReason::iter().map(TecEmptyReason::badge).collect();
        let messages: HashSet<String> = TecEmptyReason::iter()
            .map(TecEmptyReason::message)
            .collect();
        assert_eq!(badges.len(), TecEmptyReason::COUNT);
        assert_eq!(messages.len(), TecEmptyReason::COUNT);
    }

    #[test]
    fn empty_messages() {
        let wording: Vec<String> = TecEmptyReason::iter()
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
