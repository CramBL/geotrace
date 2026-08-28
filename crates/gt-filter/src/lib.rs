use std::ops::Range;

use chrono::{DateTime, Duration, Utc};
use gt_types::{MarkerRequirement, NavPoint, TimeRange, TrackMetadata};
use uom::si::f64::Length;

/// Returns `true` when the timestamp falls within the filter's active time window.
pub fn point_passes_time_filter(time: DateTime<Utc>, filter: &GlobalFilter) -> bool {
    TimeRange::new(time, time).overlaps_window(filter.time_start, filter.time_end)
}

/// The contiguous sub-range of time-ordered `points` inside the filter's
/// time window, agreeing point-for-point with [`point_passes_time_filter`]
/// (pinned by a test below).
///
/// Because points are time-ordered within a track, the window selects one
/// contiguous slice, so consumers evaluating over it keep true point adjacency.
pub fn time_filtered_range(points: &[NavPoint], filter: &GlobalFilter) -> Range<usize> {
    let start = match filter.time_start {
        Some(t0) => points.partition_point(|p| p.tpv.time().utc() < t0),
        None => 0,
    };
    let end = match filter.time_end {
        Some(t1) => points.partition_point(|p| p.tpv.time().utc() <= t1),
        None => points.len(),
    };
    start..end.max(start)
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
pub fn track_passes_filter(meta: &TrackMetadata, filter: &GlobalFilter) -> bool {
    if !meta
        .time_range
        .overlaps_window(filter.time_start, filter.time_end)
    {
        return false;
    }
    if let Some(min_dist) = filter.min_distance_km
        && meta.distance_km < min_dist
    {
        return false;
    }
    if let Some(min_duration) = filter.min_duration
        && meta.duration < min_duration
    {
        return false;
    }
    if let Some(min_spread) = filter.min_spread_m
        && meta.point_set_diameter_m < min_spread
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
    use super::*;
    use chrono::{Duration, TimeZone, Utc};
    use gt_types::coordinates::{Latitude, Longitude};
    use gt_types::time_types::GpsTime;
    use gt_types::tpv::TimePositionVelocity;
    use gt_types::{GeoBounds, MercBounds, TimeRange, TrackMetadata};
    use uom::si::length::{kilometer, meter};

    /// Points one second apart starting at a fixed epoch.
    fn timed_points(count: usize) -> Vec<NavPoint> {
        (0..count)
            .map(|i| {
                let time = Utc
                    .timestamp_opt(1_700_000_000 + i as i64, 0)
                    .single()
                    .expect("valid timestamp");
                let tpv = TimePositionVelocity::builder()
                    .time(GpsTime::from_utc(time))
                    .lat(Latitude::new(55.0))
                    .lon(Longitude::new(12.0))
                    .build();
                NavPoint::new(tpv, None)
            })
            .collect()
    }

    /// The range form must agree point-for-point with the per-point
    /// predicate - it exists so slicing consumers (the query evaluator)
    /// cannot drift from what the map draws.
    #[test]
    fn time_filtered_range_agrees_with_the_point_predicate() {
        let points = timed_points(8);
        let t = |i: usize| points.get(i).expect("in range").tpv.time().utc();
        let filters = [
            GlobalFilter::default(),
            GlobalFilter {
                time_start: Some(t(2)),
                ..GlobalFilter::default()
            },
            GlobalFilter {
                time_end: Some(t(5)),
                ..GlobalFilter::default()
            },
            GlobalFilter {
                time_start: Some(t(2)),
                time_end: Some(t(5)),
                ..GlobalFilter::default()
            },
            // Windows entirely before and entirely after the data.
            GlobalFilter {
                time_end: Some(t(0) - Duration::hours(1)),
                ..GlobalFilter::default()
            },
            GlobalFilter {
                time_start: Some(t(7) + Duration::hours(1)),
                ..GlobalFilter::default()
            },
            // Inverted window (end before start) selects nothing.
            GlobalFilter {
                time_start: Some(t(5)),
                time_end: Some(t(2)),
                ..GlobalFilter::default()
            },
        ];
        for filter in filters {
            let range = time_filtered_range(&points, &filter);
            for (pi, point) in points.iter().enumerate() {
                assert_eq!(
                    range.contains(&pi),
                    point_passes_time_filter(point.tpv.time().utc(), &filter),
                    "point {pi} under {filter:?}"
                );
            }
        }
    }

    fn make_meta(
        distance_km: f64,
        duration_secs: i64,
        spread_m: f64,
        has_custom: bool,
        start_offset_secs: i64,
        end_offset_secs: i64,
    ) -> TrackMetadata {
        let epoch = Utc.timestamp_opt(0, 0).single().expect("valid");
        TrackMetadata {
            index: 1,
            distance_km: Length::new::<kilometer>(distance_km),
            duration: Duration::seconds(duration_secs),
            time_range: TimeRange::new(
                epoch + Duration::seconds(start_offset_secs),
                epoch + Duration::seconds(end_offset_secs),
            ),
            bounding_box: GeoBounds::from_positions([
                (Latitude::new(0.0), Longitude::new(0.0)),
                (Latitude::new(1.0), Longitude::new(1.0)),
            ])
            .expect("two positions"),
            merc_bounds: MercBounds {
                x_min: 0.0,
                x_max: 0.0,
                y_min: 0.0,
                y_max: 0.0,
            },
            point_set_diameter_m: Length::new::<meter>(spread_m),
            has_custom_markers: has_custom,
            tpv_count: 1,
            invalid_position_count: 0,
            satellite_report_count: 0,
            custom_marker_count: 0,
            generated_marker_count: 0,
            event_marker_count: 0,
            ..gt_test_utils::empty_track_metadata()
        }
    }

    #[test]
    fn empty_filter_passes_all() {
        let meta = make_meta(1.0, 60, 100.0, false, 0, 60);
        assert!(track_passes_filter(&meta, &GlobalFilter::default()));
    }

    #[test]
    fn time_start_track_ends_before() {
        let meta = make_meta(1.0, 60, 100.0, false, 0, 60);
        let filter = GlobalFilter {
            time_start: Some(Utc.timestamp_opt(120, 0).single().expect("valid")),
            ..Default::default()
        };
        assert!(!track_passes_filter(&meta, &filter));
    }

    #[test]
    fn time_start_track_overlaps() {
        let meta = make_meta(1.0, 60, 100.0, false, 0, 200);
        let filter = GlobalFilter {
            time_start: Some(Utc.timestamp_opt(100, 0).single().expect("valid")),
            ..Default::default()
        };
        assert!(track_passes_filter(&meta, &filter));
    }

    #[test]
    fn time_end_track_starts_after() {
        let meta = make_meta(1.0, 60, 100.0, false, 200, 260);
        let filter = GlobalFilter {
            time_end: Some(Utc.timestamp_opt(100, 0).single().expect("valid")),
            ..Default::default()
        };
        assert!(!track_passes_filter(&meta, &filter));
    }

    #[test]
    fn time_end_track_overlaps() {
        let meta = make_meta(1.0, 60, 100.0, false, 50, 150);
        let filter = GlobalFilter {
            time_end: Some(Utc.timestamp_opt(100, 0).single().expect("valid")),
            ..Default::default()
        };
        assert!(track_passes_filter(&meta, &filter));
    }

    #[test]
    fn min_distance_pass() {
        let meta = make_meta(10.0, 60, 100.0, false, 0, 60);
        let filter = GlobalFilter {
            min_distance_km: Some(Length::new::<kilometer>(5.0)),
            ..Default::default()
        };
        assert!(track_passes_filter(&meta, &filter));
    }

    #[test]
    fn min_distance_fail() {
        let meta = make_meta(3.0, 60, 100.0, false, 0, 60);
        let filter = GlobalFilter {
            min_distance_km: Some(Length::new::<kilometer>(5.0)),
            ..Default::default()
        };
        assert!(!track_passes_filter(&meta, &filter));
    }

    #[test]
    fn min_duration_pass() {
        let meta = make_meta(1.0, 600, 100.0, false, 0, 600);
        let filter = GlobalFilter {
            min_duration: Some(Duration::seconds(300)),
            ..Default::default()
        };
        assert!(track_passes_filter(&meta, &filter));
    }

    #[test]
    fn min_duration_fail() {
        let meta = make_meta(1.0, 60, 100.0, false, 0, 60);
        let filter = GlobalFilter {
            min_duration: Some(Duration::seconds(300)),
            ..Default::default()
        };
        assert!(!track_passes_filter(&meta, &filter));
    }

    #[test]
    fn min_spread_pass() {
        let meta = make_meta(1.0, 60, 500.0, false, 0, 60);
        let filter = GlobalFilter {
            min_spread_m: Some(Length::new::<meter>(200.0)),
            ..Default::default()
        };
        assert!(track_passes_filter(&meta, &filter));
    }

    #[test]
    fn min_spread_fail() {
        let meta = make_meta(1.0, 60, 50.0, false, 0, 60);
        let filter = GlobalFilter {
            min_spread_m: Some(Length::new::<meter>(200.0)),
            ..Default::default()
        };
        assert!(!track_passes_filter(&meta, &filter));
    }

    #[test]
    fn require_custom_marker_pass() {
        let meta = make_meta(1.0, 60, 100.0, true, 0, 60);
        let filter = GlobalFilter {
            marker_requirement: MarkerRequirement::CustomMarker,
            ..Default::default()
        };
        assert!(track_passes_filter(&meta, &filter));
    }

    #[test]
    fn require_custom_marker_fail() {
        let meta = make_meta(1.0, 60, 100.0, false, 0, 60);
        let filter = GlobalFilter {
            marker_requirement: MarkerRequirement::CustomMarker,
            ..Default::default()
        };
        assert!(!track_passes_filter(&meta, &filter));
    }

    #[test]
    fn require_custom_marker_passes_for_event_markers() {
        let mut meta = make_meta(1.0, 60, 100.0, false, 0, 60);
        meta.event_marker_count = 3;
        let filter = GlobalFilter {
            marker_requirement: MarkerRequirement::CustomMarker,
            ..Default::default()
        };
        assert!(
            track_passes_filter(&meta, &filter),
            "track with event markers should pass CustomMarker filter"
        );
    }

    #[test]
    fn any_marker_filter_passes_for_event_markers() {
        let mut meta = make_meta(1.0, 60, 100.0, false, 0, 60);
        meta.event_marker_count = 1;
        let filter = GlobalFilter {
            marker_requirement: MarkerRequirement::AnyMarker,
            ..Default::default()
        };
        assert!(
            track_passes_filter(&meta, &filter),
            "track with event markers should pass AnyMarker filter"
        );
    }
}
