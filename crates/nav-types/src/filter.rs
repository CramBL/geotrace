use crate::trip::TripMetadata;
use chrono::{DateTime, Utc};

/// Returns `true` when the timestamp falls within the filter's active time window.
pub fn point_passes_time_filter(time: DateTime<Utc>, filter: &GlobalFilter) -> bool {
    if let Some(start) = filter.time_start
        && time < start
    {
        return false;
    }
    if let Some(end) = filter.time_end
        && time > end
    {
        return false;
    }
    true
}

#[derive(Debug, Clone, Copy, Default)]
pub struct GlobalFilter {
    pub time_start: Option<DateTime<Utc>>,
    pub time_end: Option<DateTime<Utc>>,
    pub min_distance_km: Option<f64>,
    pub min_duration_secs: Option<i64>,
    pub min_spread_m: Option<f64>,
    /// Show only trips that have at least one marker of any kind (custom or generated).
    pub require_any_marker: bool,
    /// Show only trips that have at least one *custom* marker specifically.
    pub require_custom_marker: bool,
}

impl GlobalFilter {
    /// Returns `true` when no filter conditions are active.
    pub fn is_empty(&self) -> bool {
        self.time_start.is_none()
            && self.time_end.is_none()
            && self.min_distance_km.is_none()
            && self.min_duration_secs.is_none()
            && self.min_spread_m.is_none()
            && !self.require_any_marker
            && !self.require_custom_marker
    }
}

/// Returns `true` when the trip satisfies all active filter conditions.
pub fn trip_passes_filter(meta: &TripMetadata, filter: &GlobalFilter) -> bool {
    if let Some(start) = filter.time_start
        && meta.time_range.1 < start
    {
        return false;
    }
    if let Some(end) = filter.time_end
        && meta.time_range.0 > end
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
    if filter.require_any_marker && !meta.has_custom_markers && meta.generated_marker_count == 0 {
        return false;
    }
    if filter.require_custom_marker && !meta.has_custom_markers {
        return false;
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::trip::TripMetadata;
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
            time_range: (
                epoch + Duration::seconds(start_offset_secs),
                epoch + Duration::seconds(end_offset_secs),
            ),
            bounding_box: Rect::new(
                geo_types::coord! { x: 0.0, y: 0.0 },
                geo_types::coord! { x: 1.0, y: 1.0 },
            ),
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
            require_custom_marker: true,
            ..Default::default()
        };
        assert!(trip_passes_filter(&meta, &filter));
    }

    #[test]
    fn require_custom_marker_fail() {
        let meta = make_meta(1.0, 60, 100.0, false, 0, 60);
        let filter = GlobalFilter {
            require_custom_marker: true,
            ..Default::default()
        };
        assert!(!trip_passes_filter(&meta, &filter));
    }
}
