use std::ops::Range;

use chrono::{DateTime, Duration, Utc};
use gt_types::{LoadedTrack, MarkerRequirement, NavPoint, TimeRange};
use uom::si::f64::Length;
use uom::si::length::meter;

/// A validated closed time window.
///
/// The endpoints are ordered when this value is constructed, so a bounded
/// window cannot accidentally encode an empty interval.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BoundedTimeWindow(TimeRange);

impl BoundedTimeWindow {
    /// Construct a closed window, rejecting an inverted pair of endpoints.
    pub fn try_new(start: DateTime<Utc>, end: DateTime<Utc>) -> Option<Self> {
        (start <= end).then_some(Self(TimeRange::new(start, end)))
    }

    pub fn range(self) -> TimeRange {
        self.0
    }
}

/// Optional endpoints of a [`TimeWindow`].
pub type TimeBounds = (Option<DateTime<Utc>>, Option<DateTime<Utc>>);

/// The time constraint of a [`GlobalFilter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TimeWindow {
    /// No time constraint.
    #[default]
    All,
    /// Keep instants at or after this bound.
    From(DateTime<Utc>),
    /// Keep instants at or before this bound.
    Until(DateTime<Utc>),
    /// Keep instants inside this validated closed interval.
    Between(BoundedTimeWindow),
    /// Contains zero instants.
    Empty,
}

impl TimeWindow {
    /// Construct a window from optional endpoints.
    ///
    /// An inverted bounded selection becomes the explicit [`Self::Empty`] state.
    pub fn from_bounds(start: Option<DateTime<Utc>>, end: Option<DateTime<Utc>>) -> Self {
        match (start, end) {
            (None, None) => Self::All,
            (Some(start), None) => Self::From(start),
            (None, Some(end)) => Self::Until(end),
            (Some(start), Some(end)) => {
                BoundedTimeWindow::try_new(start, end).map_or(Self::Empty, Self::Between)
            }
        }
    }

    /// Whether `instant` is inside this window, including a bounded window's
    /// endpoints.
    pub fn contains(self, instant: DateTime<Utc>) -> bool {
        match self {
            Self::All => true,
            Self::From(start) => instant >= start,
            Self::Until(end) => instant <= end,
            Self::Between(window) => window.range().contains(instant),
            Self::Empty => false,
        }
    }

    /// Whether any instant of `range` is inside this window.
    pub fn overlaps(self, range: TimeRange) -> bool {
        match self {
            Self::All => true,
            Self::From(start) => range.end >= start,
            Self::Until(end) => range.start <= end,
            Self::Between(window) => range.intersection(window.range()).is_some(),
            Self::Empty => false,
        }
    }

    /// Whether every instant of `range` is inside this window.
    pub fn covers(self, range: TimeRange) -> bool {
        match self {
            Self::All => true,
            Self::From(start) => start <= range.start,
            Self::Until(end) => range.end <= end,
            Self::Between(window) => {
                let window = window.range();
                window.start <= range.start && range.end <= window.end
            }
            Self::Empty => false,
        }
    }

    /// The part of `range` inside this window.
    pub fn intersection(self, range: TimeRange) -> Option<TimeRange> {
        match self {
            Self::All => Some(range),
            Self::From(start) => range.intersection(TimeRange::new(start, range.end)),
            Self::Until(end) => range.intersection(TimeRange::new(range.start, end)),
            Self::Between(window) => range.intersection(window.range()),
            Self::Empty => None,
        }
    }

    /// Optional endpoints for consumers that need to edit the two bounds.
    ///
    /// [`None`] means the explicit empty window, while `Some((None, None))`
    /// means an unbounded window.
    pub fn bounds(self) -> Option<TimeBounds> {
        match self {
            Self::All => Some((None, None)),
            Self::From(start) => Some((Some(start), None)),
            Self::Until(end) => Some((None, Some(end))),
            Self::Between(window) => {
                let range = window.range();
                Some((Some(range.start), Some(range.end)))
            }
            Self::Empty => None,
        }
    }
}

fn valid_positive_length(value: Length) -> bool {
    let meters = value.get::<meter>();
    meters.is_finite() && meters > 0.0
}

/// A finite, positive minimum track distance.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MinimumDistance(Length);

impl MinimumDistance {
    pub fn try_new(value: Length) -> Option<Self> {
        valid_positive_length(value).then_some(Self(value))
    }

    pub fn get(self) -> Length {
        self.0
    }
}

/// A positive minimum track duration.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MinimumDuration(Duration);

impl MinimumDuration {
    pub fn try_new(value: Duration) -> Option<Self> {
        (value > Duration::zero()).then_some(Self(value))
    }

    pub fn get(self) -> Duration {
        self.0
    }
}

/// A finite, positive minimum track spread.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MinimumSpread(Length);

impl MinimumSpread {
    pub fn try_new(value: Length) -> Option<Self> {
        valid_positive_length(value).then_some(Self(value))
    }

    pub fn get(self) -> Length {
        self.0
    }
}

/// Why a track was rejected by [`GlobalFilter::track_result`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackFilterReason {
    OutsideTimeWindow,
    BelowMinimumDistance,
    BelowMinimumDuration,
    BelowMinimumSpread,
    MissingAnyMarker,
    MissingCustomMarker,
}

/// The track-level result of applying a [`GlobalFilter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TrackFilterResult {
    Pass,
    Reject(TrackFilterReason),
}

impl TrackFilterResult {
    pub fn passes(self) -> bool {
        matches!(self, Self::Pass)
    }
}

/// Returns `true` when the timestamp falls within the filter's active time window.
pub fn point_passes_time_filter(time: DateTime<Utc>, filter: &GlobalFilter) -> bool {
    filter.contains_time(time)
}

/// The smallest contiguous range of `points` covering every point the filter's
/// time window keeps.
///
/// Nothing sorts a track's fixes by time, so on a track whose timestamps step
/// backwards the range also covers points the window rejects. A consumer
/// evaluating over the range applies [`point_passes_time_filter`] to each
/// point it reads, which excludes those.
pub fn time_filtered_range(points: &[NavPoint], filter: &GlobalFilter) -> Range<usize> {
    let inside_window = |point: &NavPoint| point_passes_time_filter(point.tpv.time().utc(), filter);
    match (
        points.iter().position(inside_window),
        points.iter().rposition(inside_window),
    ) {
        (Some(first), Some(last)) => first..last + 1,
        _ => 0..0,
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct GlobalFilter {
    time_window: TimeWindow,
    minimum_distance: Option<MinimumDistance>,
    minimum_duration: Option<MinimumDuration>,
    minimum_spread: Option<MinimumSpread>,
    marker_requirement: MarkerRequirement,
}

impl GlobalFilter {
    pub fn time_window(&self) -> TimeWindow {
        self.time_window
    }

    pub fn set_time_window(&mut self, time_window: TimeWindow) {
        self.time_window = time_window;
    }

    pub fn set_time_bounds(&mut self, start: Option<DateTime<Utc>>, end: Option<DateTime<Utc>>) {
        self.set_time_window(TimeWindow::from_bounds(start, end));
    }

    #[must_use]
    pub fn with_time_window(mut self, time_window: TimeWindow) -> Self {
        self.set_time_window(time_window);
        self
    }

    #[must_use]
    pub fn with_time_bounds(
        mut self,
        start: Option<DateTime<Utc>>,
        end: Option<DateTime<Utc>>,
    ) -> Self {
        self.set_time_bounds(start, end);
        self
    }

    pub fn minimum_distance(&self) -> Option<Length> {
        self.minimum_distance.map(MinimumDistance::get)
    }

    pub fn set_minimum_distance(&mut self, minimum_distance: Option<Length>) {
        self.minimum_distance = minimum_distance.and_then(MinimumDistance::try_new);
    }

    #[must_use]
    pub fn with_minimum_distance(mut self, minimum_distance: Option<Length>) -> Self {
        self.set_minimum_distance(minimum_distance);
        self
    }

    pub fn minimum_duration(&self) -> Option<Duration> {
        self.minimum_duration.map(MinimumDuration::get)
    }

    pub fn set_minimum_duration(&mut self, minimum_duration: Option<Duration>) {
        self.minimum_duration = minimum_duration.and_then(MinimumDuration::try_new);
    }

    #[must_use]
    pub fn with_minimum_duration(mut self, minimum_duration: Option<Duration>) -> Self {
        self.set_minimum_duration(minimum_duration);
        self
    }

    pub fn minimum_spread(&self) -> Option<Length> {
        self.minimum_spread.map(MinimumSpread::get)
    }

    pub fn set_minimum_spread(&mut self, minimum_spread: Option<Length>) {
        self.minimum_spread = minimum_spread.and_then(MinimumSpread::try_new);
    }

    #[must_use]
    pub fn with_minimum_spread(mut self, minimum_spread: Option<Length>) -> Self {
        self.set_minimum_spread(minimum_spread);
        self
    }

    pub fn marker_requirement(&self) -> MarkerRequirement {
        self.marker_requirement
    }

    pub fn set_marker_requirement(&mut self, marker_requirement: MarkerRequirement) {
        self.marker_requirement = marker_requirement;
    }

    #[must_use]
    pub fn with_marker_requirement(mut self, marker_requirement: MarkerRequirement) -> Self {
        self.set_marker_requirement(marker_requirement);
        self
    }

    pub fn contains_time(&self, time: DateTime<Utc>) -> bool {
        self.time_window.contains(time)
    }

    /// Returns `true` when no filter conditions are active.
    pub fn is_empty(&self) -> bool {
        let Self {
            time_window,
            minimum_distance,
            minimum_duration,
            minimum_spread,
            marker_requirement,
        } = *self;
        time_window == TimeWindow::All
            && minimum_distance.is_none()
            && minimum_duration.is_none()
            && minimum_spread.is_none()
            && marker_requirement == MarkerRequirement::None
    }

    /// Evaluate all track-level filter conditions, preserving the first reason
    /// a track was rejected.
    pub fn track_result(&self, track: &LoadedTrack) -> TrackFilterResult {
        let Self {
            time_window,
            minimum_distance,
            minimum_duration,
            minimum_spread,
            marker_requirement,
        } = *self;
        let meta = &track.metadata;
        let geometry = track.geometry.measured();

        if !time_window.overlaps(meta.time_range) {
            return TrackFilterResult::Reject(TrackFilterReason::OutsideTimeWindow);
        }
        if let Some(minimum_distance) = minimum_distance
            && geometry.is_some_and(|geometry| geometry.distance_km < minimum_distance.get())
        {
            return TrackFilterResult::Reject(TrackFilterReason::BelowMinimumDistance);
        }
        if let Some(minimum_duration) = minimum_duration
            && meta.duration < minimum_duration.get()
        {
            return TrackFilterResult::Reject(TrackFilterReason::BelowMinimumDuration);
        }
        if let Some(minimum_spread) = minimum_spread
            && geometry.is_some_and(|geometry| geometry.point_set_diameter_m < minimum_spread.get())
        {
            return TrackFilterResult::Reject(TrackFilterReason::BelowMinimumSpread);
        }
        match marker_requirement {
            MarkerRequirement::AnyMarker if !meta.has_any_marker() => {
                TrackFilterResult::Reject(TrackFilterReason::MissingAnyMarker)
            }
            MarkerRequirement::CustomMarker
                if !meta.has_custom_markers && meta.event_marker_count == 0 =>
            {
                TrackFilterResult::Reject(TrackFilterReason::MissingCustomMarker)
            }
            MarkerRequirement::AnyMarker
            | MarkerRequirement::CustomMarker
            | MarkerRequirement::None => TrackFilterResult::Pass,
        }
    }
}

/// Returns `true` when the track satisfies all active filter conditions.
///
/// A track no fix of which has a valid position measures neither a distance
/// nor a spread, so a minimum on either has nothing to compare and keeps it.
pub fn track_passes_filter(track: &LoadedTrack, filter: &GlobalFilter) -> bool {
    filter.track_result(track).passes()
}

#[cfg(test)]
mod tests {
    use chrono::{Duration, TimeZone, Utc};
    use gt_types::coordinates::{Latitude, Longitude};
    use gt_types::{
        GeoBounds, MeasuredTrackGeometry, MercBounds, TimeRange, TrackGeometry, TrackMetadata,
    };
    use rstest::rstest;
    use uom::si::length::{kilometer, meter};

    use super::*;

    /// A case states only the fields its own condition reads. The defaults
    /// pass every filter below.
    struct TrackFilterInputs {
        distance_km: f64,
        duration_secs: i64,
        spread_m: f64,
        has_custom_markers: bool,
        event_marker_count: usize,
        /// The track's time range, in seconds from the Unix epoch.
        time_range_secs: Range<i64>,
    }

    impl Default for TrackFilterInputs {
        fn default() -> Self {
            Self {
                distance_km: 1.0,
                duration_secs: 60,
                spread_m: 100.0,
                has_custom_markers: false,
                event_marker_count: 0,
                time_range_secs: 0..60,
            }
        }
    }

    impl TrackFilterInputs {
        /// A track built from this geometry and metadata, without fixes. The
        /// track-level clause reads neither the points nor the markers
        /// themselves.
        fn track(self) -> LoadedTrack {
            let epoch = Utc.timestamp_opt(0, 0).single().expect("valid");
            let bounding_box = GeoBounds::from_positions([
                (Latitude::new(0.0), Longitude::new(0.0)),
                (Latitude::new(1.0), Longitude::new(1.0)),
            ])
            .expect("two positions");
            LoadedTrack {
                metadata: TrackMetadata {
                    index: 1,
                    duration: Duration::seconds(self.duration_secs),
                    time_range: TimeRange::new(
                        epoch + Duration::seconds(self.time_range_secs.start),
                        epoch + Duration::seconds(self.time_range_secs.end),
                    ),
                    has_custom_markers: self.has_custom_markers,
                    event_marker_count: self.event_marker_count,
                    ..gt_test_utils::empty_track_metadata()
                },
                geometry: TrackGeometry::Measured(MeasuredTrackGeometry {
                    resolved_positions: Vec::new(),
                    bounding_box,
                    merc_bounds: MercBounds::from(bounding_box),
                    distance_km: Length::new::<kilometer>(self.distance_km),
                    point_set_diameter_m: Length::new::<meter>(self.spread_m),
                    segment_length_range: None,
                }),
                points: Vec::new(),
                lod: gt_types::TrackLod::default(),
                sat_label_anchors: Vec::new(),
                custom_markers: Vec::new(),
                generated_markers: Vec::new(),
                event_markers: Vec::new(),
                channels: Vec::new(),
            }
        }
    }

    fn at_second(secs: i64) -> DateTime<Utc> {
        Utc.timestamp_opt(secs, 0)
            .single()
            .expect("valid timestamp")
    }

    /// A filter whose only active condition is the start of the time window.
    fn window_from(secs: i64) -> GlobalFilter {
        GlobalFilter::default().with_time_window(TimeWindow::From(at_second(secs)))
    }

    /// A filter whose only active condition is the end of the time window.
    fn window_until(secs: i64) -> GlobalFilter {
        GlobalFilter::default().with_time_window(TimeWindow::Until(at_second(secs)))
    }

    #[rstest]
    #[case::no_condition_is_active(TrackFilterInputs::default(), GlobalFilter::default(), true)]
    #[case::the_window_starts_after_the_track_ends(
        TrackFilterInputs::default(),
        window_from(120),
        false
    )]
    #[case::the_window_starts_inside_the_track(
        TrackFilterInputs { time_range_secs: 0..200, ..TrackFilterInputs::default() },
        window_from(100),
        true
    )]
    #[case::the_window_ends_before_the_track_starts(
        TrackFilterInputs { time_range_secs: 200..260, ..TrackFilterInputs::default() },
        window_until(100),
        false
    )]
    #[case::the_window_ends_inside_the_track(
        TrackFilterInputs { time_range_secs: 50..150, ..TrackFilterInputs::default() },
        window_until(100),
        true
    )]
    #[case::the_track_runs_further_than_the_minimum_distance(
        TrackFilterInputs { distance_km: 10.0, ..TrackFilterInputs::default() },
        GlobalFilter::default().with_minimum_distance(Some(Length::new::<kilometer>(5.0))),
        true
    )]
    #[case::the_track_runs_shorter_than_the_minimum_distance(
        TrackFilterInputs { distance_km: 3.0, ..TrackFilterInputs::default() },
        GlobalFilter::default().with_minimum_distance(Some(Length::new::<kilometer>(5.0))),
        false
    )]
    #[case::the_track_lasts_longer_than_the_minimum_duration(
        TrackFilterInputs { duration_secs: 600, ..TrackFilterInputs::default() },
        GlobalFilter::default().with_minimum_duration(Some(Duration::seconds(300))),
        true
    )]
    #[case::the_track_lasts_shorter_than_the_minimum_duration(
        TrackFilterInputs::default(),
        GlobalFilter::default().with_minimum_duration(Some(Duration::seconds(300))),
        false
    )]
    #[case::the_track_spreads_wider_than_the_minimum(
        TrackFilterInputs { spread_m: 500.0, ..TrackFilterInputs::default() },
        GlobalFilter::default().with_minimum_spread(Some(Length::new::<meter>(200.0))),
        true
    )]
    #[case::the_track_spreads_narrower_than_the_minimum(
        TrackFilterInputs { spread_m: 50.0, ..TrackFilterInputs::default() },
        GlobalFilter::default().with_minimum_spread(Some(Length::new::<meter>(200.0))),
        false
    )]
    #[case::the_track_has_a_custom_marker(
        TrackFilterInputs { has_custom_markers: true, ..TrackFilterInputs::default() },
        GlobalFilter::default().with_marker_requirement(MarkerRequirement::CustomMarker),
        true
    )]
    #[case::the_track_has_no_marker_of_any_kind(
        TrackFilterInputs::default(),
        GlobalFilter::default().with_marker_requirement(MarkerRequirement::CustomMarker),
        false
    )]
    #[case::an_event_marker_satisfies_the_custom_marker_requirement(
        TrackFilterInputs { event_marker_count: 3, ..TrackFilterInputs::default() },
        GlobalFilter::default().with_marker_requirement(MarkerRequirement::CustomMarker),
        true
    )]
    #[case::an_event_marker_satisfies_the_any_marker_requirement(
        TrackFilterInputs { event_marker_count: 1, ..TrackFilterInputs::default() },
        GlobalFilter::default().with_marker_requirement(MarkerRequirement::AnyMarker),
        true
    )]
    fn a_track_passes_the_filter_only_when_every_active_condition_holds(
        #[case] track: TrackFilterInputs,
        #[case] filter: GlobalFilter,
        #[case] passes: bool,
    ) {
        assert_eq!(track_passes_filter(&track.track(), &filter), passes);
    }

    #[rstest]
    #[case::time(window_from(120), TrackFilterReason::OutsideTimeWindow)]
    #[case::distance(
        GlobalFilter::default().with_minimum_distance(Some(Length::new::<kilometer>(5.0))),
        TrackFilterReason::BelowMinimumDistance,
    )]
    #[case::duration(
        GlobalFilter::default().with_minimum_duration(Some(Duration::seconds(300))),
        TrackFilterReason::BelowMinimumDuration,
    )]
    #[case::spread(
        GlobalFilter::default().with_minimum_spread(Some(Length::new::<meter>(200.0))),
        TrackFilterReason::BelowMinimumSpread,
    )]
    #[case::any_marker(
        GlobalFilter::default().with_marker_requirement(MarkerRequirement::AnyMarker),
        TrackFilterReason::MissingAnyMarker,
    )]
    #[case::custom_marker(
        GlobalFilter::default().with_marker_requirement(MarkerRequirement::CustomMarker),
        TrackFilterReason::MissingCustomMarker,
    )]
    fn track_result_reports_the_rejection_reason(
        #[case] filter: GlobalFilter,
        #[case] reason: TrackFilterReason,
    ) {
        assert_eq!(
            filter.track_result(&TrackFilterInputs::default().track()),
            TrackFilterResult::Reject(reason),
        );
    }

    #[test]
    fn inverted_bounds_become_an_explicit_empty_window() {
        assert_eq!(
            TimeWindow::from_bounds(Some(at_second(20)), Some(at_second(10))),
            TimeWindow::Empty,
        );
    }

    #[test]
    fn invalid_minimum_values_are_rejected() {
        for value in [f64::NEG_INFINITY, -1.0, 0.0, f64::INFINITY, f64::NAN] {
            assert_eq!(MinimumDistance::try_new(Length::new::<meter>(value)), None);
            assert_eq!(MinimumSpread::try_new(Length::new::<meter>(value)), None);
        }
        assert_eq!(MinimumDuration::try_new(Duration::zero()), None);
        assert_eq!(MinimumDuration::try_new(Duration::seconds(-1)), None);

        let filter = GlobalFilter::default()
            .with_minimum_distance(Some(Length::new::<meter>(f64::NAN)))
            .with_minimum_duration(Some(Duration::zero()))
            .with_minimum_spread(Some(Length::new::<meter>(f64::INFINITY)));
        assert_eq!(filter.minimum_distance(), None);
        assert_eq!(filter.minimum_duration(), None);
        assert_eq!(filter.minimum_spread(), None);
    }

    /// A track no fix of which has a valid position has neither a distance nor
    /// a spread to compare against a minimum, so the filter keeps it: the same
    /// reading as a track without segments, whose icons stay visible at every
    /// zoom.
    #[test]
    fn a_track_without_geometry_passes_the_distance_and_spread_filters() {
        let track = gt_test_utils::loaded_track_with_points(
            gt_test_utils::fixtures::nav_points_without_a_valid_position(3),
        );
        assert_eq!(track.geometry, gt_types::TrackGeometry::NoValidPosition);

        let filter = GlobalFilter::default()
            .with_minimum_distance(Some(Length::new::<kilometer>(5.0)))
            .with_minimum_spread(Some(Length::new::<meter>(200.0)));

        assert!(track_passes_filter(&track, &filter));
    }
}
