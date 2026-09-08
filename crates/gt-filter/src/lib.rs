use std::ops::Range;

use chrono::{DateTime, Duration, Utc};
use gt_types::{LoadedTrack, MarkerRequirement, NavPoint, TimeRange};
use uom::si::f64::Length;

/// Returns `true` when the timestamp falls within the filter's active time window.
pub fn point_passes_time_filter(time: DateTime<Utc>, filter: &GlobalFilter) -> bool {
    TimeRange::new(time, time).overlaps_window(filter.time_start, filter.time_end)
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
    pub time_start: Option<DateTime<Utc>>,
    pub time_end: Option<DateTime<Utc>>,
    pub min_distance_km: Option<Length>,
    pub min_duration: Option<Duration>,
    pub min_spread_m: Option<Length>,
    /// Whether tracks must carry markers of a particular kind to pass.
    pub marker_requirement: MarkerRequirement,
}

impl GlobalFilter {
    /// Returns `true` when no filter conditions are active.
    pub fn is_empty(&self) -> bool {
        self.time_start.is_none()
            && self.time_end.is_none()
            && self.min_distance_km.is_none()
            && self.min_duration.is_none()
            && self.min_spread_m.is_none()
            && self.marker_requirement == MarkerRequirement::None
    }
}

/// Returns `true` when the track satisfies all active filter conditions.
///
/// A track no fix of which has a valid position measures neither a distance
/// nor a spread, so a minimum on either has nothing to compare and keeps it.
///
/// A time window whose start is after its end rejects every track.
pub fn track_passes_filter(track: &LoadedTrack, filter: &GlobalFilter) -> bool {
    let meta = &track.metadata;
    let geometry = track.geometry.measured();
    if !meta
        .time_range
        .overlaps_window(filter.time_start, filter.time_end)
    {
        return false;
    }
    if let Some(min_dist) = filter.min_distance_km
        && geometry.is_some_and(|geometry| geometry.distance_km < min_dist)
    {
        return false;
    }
    if let Some(min_duration) = filter.min_duration
        && meta.duration < min_duration
    {
        return false;
    }
    if let Some(min_spread) = filter.min_spread_m
        && geometry.is_some_and(|geometry| geometry.point_set_diameter_m < min_spread)
    {
        return false;
    }
    match filter.marker_requirement {
        MarkerRequirement::AnyMarker if !meta.has_any_marker() => return false,
        MarkerRequirement::CustomMarker
            if !meta.has_custom_markers && meta.event_marker_count == 0 =>
        {
            return false;
        }
        _ => {}
    }
    true
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

    /// A filter whose only active condition is the start of the time window.
    fn window_from(secs: i64) -> GlobalFilter {
        GlobalFilter {
            time_start: Utc.timestamp_opt(secs, 0).single(),
            ..GlobalFilter::default()
        }
    }

    /// A filter whose only active condition is the end of the time window.
    fn window_until(secs: i64) -> GlobalFilter {
        GlobalFilter {
            time_end: Utc.timestamp_opt(secs, 0).single(),
            ..GlobalFilter::default()
        }
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
        GlobalFilter {
            min_distance_km: Some(Length::new::<kilometer>(5.0)),
            ..GlobalFilter::default()
        },
        true
    )]
    #[case::the_track_runs_shorter_than_the_minimum_distance(
        TrackFilterInputs { distance_km: 3.0, ..TrackFilterInputs::default() },
        GlobalFilter {
            min_distance_km: Some(Length::new::<kilometer>(5.0)),
            ..GlobalFilter::default()
        },
        false
    )]
    #[case::the_track_lasts_longer_than_the_minimum_duration(
        TrackFilterInputs { duration_secs: 600, ..TrackFilterInputs::default() },
        GlobalFilter {
            min_duration: Some(Duration::seconds(300)),
            ..GlobalFilter::default()
        },
        true
    )]
    #[case::the_track_lasts_shorter_than_the_minimum_duration(
        TrackFilterInputs::default(),
        GlobalFilter {
            min_duration: Some(Duration::seconds(300)),
            ..GlobalFilter::default()
        },
        false
    )]
    #[case::the_track_spreads_wider_than_the_minimum(
        TrackFilterInputs { spread_m: 500.0, ..TrackFilterInputs::default() },
        GlobalFilter {
            min_spread_m: Some(Length::new::<meter>(200.0)),
            ..GlobalFilter::default()
        },
        true
    )]
    #[case::the_track_spreads_narrower_than_the_minimum(
        TrackFilterInputs { spread_m: 50.0, ..TrackFilterInputs::default() },
        GlobalFilter {
            min_spread_m: Some(Length::new::<meter>(200.0)),
            ..GlobalFilter::default()
        },
        false
    )]
    #[case::the_track_has_a_custom_marker(
        TrackFilterInputs { has_custom_markers: true, ..TrackFilterInputs::default() },
        GlobalFilter {
            marker_requirement: MarkerRequirement::CustomMarker,
            ..GlobalFilter::default()
        },
        true
    )]
    #[case::the_track_has_no_marker_of_any_kind(
        TrackFilterInputs::default(),
        GlobalFilter {
            marker_requirement: MarkerRequirement::CustomMarker,
            ..GlobalFilter::default()
        },
        false
    )]
    #[case::an_event_marker_satisfies_the_custom_marker_requirement(
        TrackFilterInputs { event_marker_count: 3, ..TrackFilterInputs::default() },
        GlobalFilter {
            marker_requirement: MarkerRequirement::CustomMarker,
            ..GlobalFilter::default()
        },
        true
    )]
    #[case::an_event_marker_satisfies_the_any_marker_requirement(
        TrackFilterInputs { event_marker_count: 1, ..TrackFilterInputs::default() },
        GlobalFilter {
            marker_requirement: MarkerRequirement::AnyMarker,
            ..GlobalFilter::default()
        },
        true
    )]
    fn a_track_passes_the_filter_only_when_every_active_condition_holds(
        #[case] track: TrackFilterInputs,
        #[case] filter: GlobalFilter,
        #[case] passes: bool,
    ) {
        assert_eq!(track_passes_filter(&track.track(), &filter), passes);
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

        let filter = GlobalFilter {
            min_distance_km: Some(Length::new::<kilometer>(5.0)),
            min_spread_m: Some(Length::new::<meter>(200.0)),
            ..Default::default()
        };

        assert!(track_passes_filter(&track, &filter));
    }
}
