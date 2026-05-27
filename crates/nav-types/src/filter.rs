use crate::trip::{MarkerRequirement, TimeRange, TripMetadata};
use chrono::{DateTime, Utc};

/// Returns `true` when the timestamp falls within the filter's active time window.
pub fn point_passes_time_filter(time: DateTime<Utc>, filter: &GlobalFilter) -> bool {
    TimeRange::new(time, time).overlaps_window(filter.time_start, filter.time_end)
}

#[derive(Debug, Clone, Copy, Default)]
pub struct GlobalFilter {
    pub time_start: Option<DateTime<Utc>>,
    pub time_end: Option<DateTime<Utc>>,
    pub min_distance_km: Option<f64>,
    pub min_duration_secs: Option<i64>,
    pub min_spread_m: Option<f64>,
    /// Whether trips must carry markers of a particular kind to pass.
    pub marker_requirement: MarkerRequirement,
}

impl GlobalFilter {
    /// Returns `true` when no filter conditions are active.
    pub fn is_empty(&self) -> bool {
        self.time_start.is_none()
            && self.time_end.is_none()
            && self.min_distance_km.is_none()
            && self.min_duration_secs.is_none()
            && self.min_spread_m.is_none()
            && self.marker_requirement == MarkerRequirement::None
    }
}

/// Returns `true` when the trip satisfies all active filter conditions.
pub fn trip_passes_filter(meta: &TripMetadata, filter: &GlobalFilter) -> bool {
    if !meta
        .time_range
        .overlaps_window(filter.time_start, filter.time_end)
    {
        return false;
    }
    if let Some(min_km) = filter.min_distance_km
        && meta.distance_km < min_km
    {
        return false;
    }
    if let Some(min_secs) = filter.min_duration_secs
        && meta.duration.num_seconds() < min_secs
    {
        return false;
    }
    if let Some(min_m) = filter.min_spread_m
        && meta.point_set_diameter_m < min_m
    {
        return false;
    }
    match filter.marker_requirement {
        MarkerRequirement::AnyMarker if !meta.has_any_marker() => return false,
        MarkerRequirement::CustomMarker if !meta.has_custom_markers => return false,
        _ => {}
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trip::{MercBounds, TimeRange, TripMetadata};
    use chrono::{Duration, TimeZone, Utc};
    use geo_types::Rect;

    fn make_meta(
        distance_km: f64,
        duration_secs: i64,
        spread_m: f64,
        has_custom: bool,
        start_offset_secs: i64,
        end_offset_secs: i64,
    ) -> TripMetadata {
        let epoch = Utc.timestamp_opt(0, 0).single().expect("valid");
        TripMetadata {
            index: 1,
            distance_km,
            duration: Duration::seconds(duration_secs),
            time_range: TimeRange::new(
                epoch + Duration::seconds(start_offset_secs),
                epoch + Duration::seconds(end_offset_secs),
            ),
            bounding_box: Rect::new(
                geo_types::coord! { x: 0.0, y: 0.0 },
                geo_types::coord! { x: 1.0, y: 1.0 },
            ),
            merc_bounds: MercBounds {
                x_min: 0.0,
                x_max: 0.0,
                y_min: 0.0,
                y_max: 0.0,
            },
            point_set_diameter_m: spread_m,
            has_custom_markers: has_custom,
            tpv_count: 1,
            satellite_report_count: 0,
            custom_marker_count: 0,
            generated_marker_count: 0,
        }
    }

    #[test]
    fn empty_filter_passes_all() {
        let meta = make_meta(1.0, 60, 100.0, false, 0, 60);
        assert!(trip_passes_filter(&meta, &GlobalFilter::default()));
    }

    #[test]
    fn time_start_trip_ends_before() {
        let meta = make_meta(1.0, 60, 100.0, false, 0, 60);
        let filter = GlobalFilter {
            time_start: Some(Utc.timestamp_opt(120, 0).single().expect("valid")),
            ..Default::default()
        };
        assert!(!trip_passes_filter(&meta, &filter));
    }

    #[test]
    fn time_start_trip_overlaps() {
        let meta = make_meta(1.0, 60, 100.0, false, 0, 200);
        let filter = GlobalFilter {
            time_start: Some(Utc.timestamp_opt(100, 0).single().expect("valid")),
            ..Default::default()
        };
        assert!(trip_passes_filter(&meta, &filter));
    }

    #[test]
    fn time_end_trip_starts_after() {
        let meta = make_meta(1.0, 60, 100.0, false, 200, 260);
        let filter = GlobalFilter {
            time_end: Some(Utc.timestamp_opt(100, 0).single().expect("valid")),
            ..Default::default()
        };
        assert!(!trip_passes_filter(&meta, &filter));
    }

    #[test]
    fn time_end_trip_overlaps() {
        let meta = make_meta(1.0, 60, 100.0, false, 50, 150);
        let filter = GlobalFilter {
            time_end: Some(Utc.timestamp_opt(100, 0).single().expect("valid")),
            ..Default::default()
        };
        assert!(trip_passes_filter(&meta, &filter));
    }

    #[test]
    fn min_distance_pass() {
        let meta = make_meta(10.0, 60, 100.0, false, 0, 60);
        let filter = GlobalFilter {
            min_distance_km: Some(5.0),
            ..Default::default()
        };
        assert!(trip_passes_filter(&meta, &filter));
    }

    #[test]
    fn min_distance_fail() {
        let meta = make_meta(3.0, 60, 100.0, false, 0, 60);
        let filter = GlobalFilter {
            min_distance_km: Some(5.0),
            ..Default::default()
        };
        assert!(!trip_passes_filter(&meta, &filter));
    }

    #[test]
    fn min_duration_pass() {
        let meta = make_meta(1.0, 600, 100.0, false, 0, 600);
        let filter = GlobalFilter {
            min_duration_secs: Some(300),
            ..Default::default()
        };
        assert!(trip_passes_filter(&meta, &filter));
    }

    #[test]
    fn min_duration_fail() {
        let meta = make_meta(1.0, 60, 100.0, false, 0, 60);
        let filter = GlobalFilter {
            min_duration_secs: Some(300),
            ..Default::default()
        };
        assert!(!trip_passes_filter(&meta, &filter));
    }

    #[test]
    fn min_spread_pass() {
        let meta = make_meta(1.0, 60, 500.0, false, 0, 60);
        let filter = GlobalFilter {
            min_spread_m: Some(200.0),
            ..Default::default()
        };
        assert!(trip_passes_filter(&meta, &filter));
    }

    #[test]
    fn min_spread_fail() {
        let meta = make_meta(1.0, 60, 50.0, false, 0, 60);
        let filter = GlobalFilter {
            min_spread_m: Some(200.0),
            ..Default::default()
        };
        assert!(!trip_passes_filter(&meta, &filter));
    }

    #[test]
    fn require_custom_marker_pass() {
        let meta = make_meta(1.0, 60, 100.0, true, 0, 60);
        let filter = GlobalFilter {
            marker_requirement: MarkerRequirement::CustomMarker,
            ..Default::default()
        };
        assert!(trip_passes_filter(&meta, &filter));
    }

    #[test]
    fn require_custom_marker_fail() {
        let meta = make_meta(1.0, 60, 100.0, false, 0, 60);
        let filter = GlobalFilter {
            marker_requirement: MarkerRequirement::CustomMarker,
            ..Default::default()
        };
        assert!(!trip_passes_filter(&meta, &filter));
    }
}
