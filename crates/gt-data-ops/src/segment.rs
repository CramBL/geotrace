use chrono::{DateTime, Duration, Utc};
use geo_types::{Coord, Rect};
use gt_geo_math::{path_distance_km, point_set_diameter_m};
use gt_types::coordinates::{Latitude, Longitude};
use gt_types::markers::{
    CustomMarker, EventMarker, EventMarkerStyle, GeneratedMarker, GeneratedMarkerKind,
};
use gt_types::nav_point::NavPoint;
use gt_types::time_types::GpsTime;
use gt_types::track::{FileMetadata, LoadedFile, LoadedTrack, TimeRange, TrackMetadata};
use std::ops::Range;

/// Configuration for the track-segmentation algorithm.
#[derive(Debug, Clone, Copy)]
pub struct SegmentationConfig {
    /// Timestamp gap between consecutive points that triggers a new track split.
    pub track_split_gap: Duration,
}

impl Default for SegmentationConfig {
    fn default() -> Self {
        Self {
            track_split_gap: Duration::seconds(300),
        }
    }
}

/// Partitions `points` into contiguous trip ranges. A new trip begins when the
/// timestamp gap between consecutive points reaches `config.track_split_gap`.
/// Returns an empty vec for empty input.
pub fn segment_trips(points: &[NavPoint], config: &SegmentationConfig) -> Vec<Range<usize>> {
    if points.is_empty() {
        return Vec::new();
    }

    let mut ranges: Vec<Range<usize>> = Vec::new();
    let mut start = 0usize;

    for (i, pair) in points.windows(2).enumerate() {
        if let [a, b] = pair {
            let gap = b.tpv.time() - a.tpv.time();
            if gap >= config.track_split_gap {
                ranges.push(start..i + 1);
                start = i + 1;
            }
        }
    }
    ranges.push(start..points.len());
    ranges
}

/// State machine for tracking GPS fix transitions within a trip.
///
/// Three states:
/// - `Waiting` — no satellite report seen yet.
/// - `HasFix` — the most recent satellite report had `fix_count > 0`.
/// - `LostFix` — most recent report had `fix_count == 0`; records when the fix
///   was last seen so the regained-duration can be computed.
enum GpsFixState {
    Waiting,
    HasFix {
        last_time: GpsTime,
        last_lat: Latitude,
        last_lon: Longitude,
    },
    LostFix {
        lost_at: GpsTime,
    },
}

struct GpsFixTracker {
    state: GpsFixState,
}

impl GpsFixTracker {
    fn new() -> Self {
        Self {
            state: GpsFixState::Waiting,
        }
    }

    /// Advance the state machine by one satellite report.
    ///
    /// Returns `Some(marker)` when a transition emits a generated marker
    /// (`GpsFixLost` or `GpsFixRegained`), or `None` for silent transitions.
    fn update(&mut self, point: &NavPoint, fix_count: u32) -> Option<GeneratedMarker> {
        let result;
        self.state = match self.state {
            GpsFixState::Waiting => {
                result = None;
                if fix_count > 0 {
                    GpsFixState::HasFix {
                        last_time: point.tpv.time(),
                        last_lat: point.tpv.lat(),
                        last_lon: point.tpv.lon(),
                    }
                } else {
                    GpsFixState::Waiting
                }
            }
            GpsFixState::HasFix {
                last_time,
                last_lat,
                last_lon,
            } => {
                if fix_count == 0 {
                    // Fix just dropped — emit GpsFixLost at the last known fix position.
                    result = Some(GeneratedMarker::new(
                        last_time.utc(),
                        GeneratedMarkerKind::GpsFixLost,
                        last_lat,
                        last_lon,
                        None,
                    ));
                    GpsFixState::LostFix { lost_at: last_time }
                } else {
                    result = None;
                    GpsFixState::HasFix {
                        last_time: point.tpv.time(),
                        last_lat: point.tpv.lat(),
                        last_lon: point.tpv.lon(),
                    }
                }
            }
            GpsFixState::LostFix { lost_at } => {
                if fix_count > 0 {
                    // Fix regained — emit GpsFixRegained with gap duration.
                    let duration = point.tpv.time().signed_duration_since(lost_at);
                    result = Some(GeneratedMarker::new(
                        point.tpv.time().utc(),
                        GeneratedMarkerKind::GpsFixRegained,
                        point.tpv.lat(),
                        point.tpv.lon(),
                        Some(duration),
                    ));
                    GpsFixState::HasFix {
                        last_time: point.tpv.time(),
                        last_lat: point.tpv.lat(),
                        last_lon: point.tpv.lon(),
                    }
                } else {
                    result = None;
                    GpsFixState::LostFix { lost_at }
                }
            }
        };
        result
    }
}

fn detect_generated_markers(points: &[NavPoint]) -> Vec<GeneratedMarker> {
    let mut tracker = GpsFixTracker::new();
    let mut markers = Vec::new();
    for point in points {
        if let Some(sats) = &point.satellites
            && let Some(marker) = tracker.update(point, sats.fix_count())
        {
            markers.push(marker);
        }
    }
    markers
}

/// Computes `TrackMetadata` from a non-empty slice of points.
pub fn compute_trip_metadata(
    index: usize,
    points: &vec1::Vec1<NavPoint>,
    custom_markers: &[CustomMarker],
    generated_markers: &[GeneratedMarker],
) -> TrackMetadata {
    let first = points.first();
    let last = points.last();

    let first_lat = first.tpv.lat().as_degrees();
    let first_lon = first.tpv.lon().as_degrees();

    let (min_lat, max_lat, min_lon, max_lon) = points.iter().fold(
        (first_lat, first_lat, first_lon, first_lon),
        |(min_lat, max_lat, min_lon, max_lon), p| {
            let lat = p.tpv.lat().as_degrees();
            let lon = p.tpv.lon().as_degrees();
            (
                min_lat.min(lat),
                max_lat.max(lat),
                min_lon.min(lon),
                max_lon.max(lon),
            )
        },
    );

    let bounding_box = Rect::new(
        Coord {
            x: min_lon,
            y: min_lat,
        },
        Coord {
            x: max_lon,
            y: max_lat,
        },
    );
    let merc_bounds = gt_types::merc_bounds_for_rect(bounding_box);

    let coords: Vec<(f64, f64)> = points
        .iter()
        .map(|p| (p.tpv.lat().as_degrees(), p.tpv.lon().as_degrees()))
        .collect();

    let distance_km = path_distance_km(&coords);
    let diameter_m = point_set_diameter_m(&coords);

    let time_range = TimeRange::new(first.tpv.time().utc(), last.tpv.time().utc());
    let duration = if points.len() >= 2 {
        last.tpv.time() - first.tpv.time()
    } else {
        Duration::zero()
    };

    TrackMetadata {
        index,
        distance_km,
        duration,
        time_range,
        bounding_box,
        merc_bounds,
        point_set_diameter_m: diameter_m,
        has_custom_markers: !custom_markers.is_empty(),
        tpv_count: points.len(),
        satellite_report_count: points.iter().filter(|p| p.satellites.is_some()).count(),
        custom_marker_count: custom_markers.len(),
        generated_marker_count: generated_markers.len(),
        event_marker_count: 0, // filled in by build_loaded_file after event marker assignment
    }
}

/// Segments `points` into trips and builds a fully-populated `LoadedFile`.
#[expect(
    clippy::expect_used,
    reason = "ranges from segment_trips are always in-bounds and non-empty"
)]
pub fn build_loaded_file(
    filename: String,
    points: &[NavPoint],
    custom_markers: &[CustomMarker],
    event_markers: Vec<EventMarker>,
    event_marker_styles: Vec<EventMarkerStyle>,
    config: &SegmentationConfig,
) -> LoadedFile {
    let ranges = segment_trips(points, config);

    let mut loaded_tracks: Vec<LoadedTrack> = ranges
        .into_iter()
        .enumerate()
        .map(|(trip_idx, range)| {
            let trip_points_slice = points
                .get(range)
                .expect("ranges from segment_trips are in bounds");

            let trip_points: vec1::Vec1<NavPoint> =
                vec1::Vec1::try_from_vec(trip_points_slice.to_vec())
                    .expect("segment_trips produces only non-empty ranges");

            let trip_start = trip_points.first().tpv.time().utc();
            let trip_end = trip_points.last().tpv.time().utc();

            let trip_custom: Vec<CustomMarker> = custom_markers
                .iter()
                .filter(|m| m.time >= trip_start && m.time <= trip_end)
                .cloned()
                .collect();

            let trip_generated = detect_generated_markers(&trip_points);

            let metadata =
                compute_trip_metadata(trip_idx + 1, &trip_points, &trip_custom, &trip_generated);

            LoadedTrack {
                metadata,
                points: trip_points.into_vec(),
                custom_markers: trip_custom,
                generated_markers: trip_generated,
                event_markers: Vec::new(),
            }
        })
        .collect();

    // Assign event markers to trips by timestamp; orphans go into LoadedFile.
    let mut orphaned_event_markers = Vec::new();
    for em in event_markers {
        let mut em = Some(em);
        for track in &mut loaded_tracks {
            let start = track.metadata.time_range.start;
            let end = track.metadata.time_range.end;
            if em
                .as_ref()
                .is_some_and(|e| e.time >= start && e.time <= end)
            {
                track.event_markers.push(
                    #[expect(clippy::expect_used, reason = "just checked is_some")]
                    em.take().expect("checked above"),
                );
                break;
            }
        }
        if let Some(unassigned) = em {
            orphaned_event_markers.push(unassigned);
        }
    }
    // Back-fill event_marker_count now that assignment is done.
    for track in &mut loaded_tracks {
        track.metadata.event_marker_count = track.event_markers.len();
    }

    let total_distance_km = loaded_tracks
        .iter()
        .map(|t| t.metadata.distance_km)
        .sum::<f64>();
    let total_duration = loaded_tracks
        .iter()
        .fold(Duration::zero(), |acc, t| acc + t.metadata.duration);

    let fallback = DateTime::<Utc>::UNIX_EPOCH;
    let file_time_range = match (loaded_tracks.first(), loaded_tracks.last()) {
        (Some(first), Some(last)) => TimeRange::new(
            first.metadata.time_range.start,
            last.metadata.time_range.end,
        ),
        _ => TimeRange::new(fallback, fallback),
    };

    LoadedFile {
        metadata: FileMetadata {
            filename,
            total_distance_km,
            total_duration,
            time_range: file_time_range,
        },
        tracks: loaded_tracks,
        event_marker_styles: event_marker_styles
            .into_iter()
            .map(|s| (s.variant_path.clone(), s))
            .collect(),
        orphaned_event_markers,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use gt_types::coordinates::Latitude;
    use gt_types::time_types::GpsTime;
    use gt_types::tpv::TimePositionVelocity;
    use uom::si::angle::degree;
    use uom::si::f64::Angle;

    fn make_point_at(t: i64) -> NavPoint {
        #[expect(
            clippy::expect_used,
            reason = "fixed timestamp is always valid in tests"
        )]
        let time = GpsTime::from_utc(Utc.timestamp_opt(t, 0).single().expect("valid timestamp"));
        let tpv = TimePositionVelocity::builder()
            .time(time)
            .lat(Latitude::new(55.0))
            .lon(gt_types::coordinates::Longitude::new(12.0))
            .heading(Angle::new::<degree>(0.0))
            .build();
        NavPoint::new(tpv, None)
    }

    fn make_point_at_pos(t: i64, lat: f64, lon: f64) -> NavPoint {
        #[expect(
            clippy::expect_used,
            reason = "fixed timestamp is always valid in tests"
        )]
        let time = GpsTime::from_utc(Utc.timestamp_opt(t, 0).single().expect("valid timestamp"));
        let tpv = TimePositionVelocity::builder()
            .time(time)
            .lat(Latitude::new(lat))
            .lon(gt_types::coordinates::Longitude::new(lon))
            .heading(Angle::new::<degree>(0.0))
            .build();
        NavPoint::new(tpv, None)
    }

    // --- segment_trips ---

    #[test]
    fn segment_trips_empty_input() {
        assert!(segment_trips(&[], &SegmentationConfig::default()).is_empty());
    }

    #[test]
    fn segment_trips_single_point() {
        let pts = vec![make_point_at(0)];
        let ranges = segment_trips(&pts, &SegmentationConfig::default());
        assert_eq!(ranges, vec![0..1]);
    }

    #[test]
    fn segment_trips_all_within_five_minutes() {
        let pts: Vec<NavPoint> = (0..5).map(|i| make_point_at(i * 60)).collect();
        let ranges = segment_trips(&pts, &SegmentationConfig::default());
        assert_eq!(ranges, vec![0..5]);
    }

    #[test]
    fn segment_trips_gap_exactly_300s_starts_new_trip() {
        // [0s, 300s] → gap of exactly 300 s triggers a new trip
        let pts = vec![make_point_at(0), make_point_at(300), make_point_at(360)];
        let ranges = segment_trips(&pts, &SegmentationConfig::default());
        assert_eq!(ranges, vec![0..1, 1..3]);
    }

    #[test]
    fn segment_trips_one_gap_gives_two_trips() {
        let pts = vec![
            make_point_at(0),
            make_point_at(60),
            make_point_at(3600), // +1 h gap
            make_point_at(3660),
        ];
        let ranges = segment_trips(&pts, &SegmentationConfig::default());
        assert_eq!(ranges, vec![0..2, 2..4]);
    }

    #[test]
    fn segment_trips_multiple_gaps() {
        let pts = vec![
            make_point_at(0),
            make_point_at(3600), // gap
            make_point_at(7200), // gap
        ];
        let ranges = segment_trips(&pts, &SegmentationConfig::default());
        assert_eq!(ranges, vec![0..1, 1..2, 2..3]);
    }

    // --- compute_trip_metadata ---

    #[test]
    fn compute_trip_metadata_basic() {
        let pts = vec1::vec1![
            make_point_at_pos(0, 55.0, 12.0),
            make_point_at_pos(3600, 55.1, 12.1), // 1 h later, ~13 km away
        ];
        let meta = compute_trip_metadata(1, &pts, &[], &[]);
        assert_eq!(meta.index, 1);
        assert_eq!(meta.tpv_count, 2);
        assert_eq!(meta.duration.num_seconds(), 3600);
        assert!(
            meta.distance_km > 5.0,
            "expected > 5 km, got {}",
            meta.distance_km
        );
        assert!(!meta.has_custom_markers);
        assert_eq!(meta.satellite_report_count, 0);
    }

    #[test]
    fn compute_trip_metadata_single_point_has_zero_duration() {
        let pts = vec1::vec1![make_point_at_pos(0, 55.0, 12.0)];
        let meta = compute_trip_metadata(1, &pts, &[], &[]);
        assert_eq!(meta.duration.num_seconds(), 0);
        assert_eq!(meta.distance_km, 0.0);
    }

    // --- build_loaded_file ---

    #[test]
    fn build_loaded_file_empty_points() {
        let f = build_loaded_file(
            "test.nvd".to_owned(),
            &[],
            &[],
            vec![],
            vec![],
            &SegmentationConfig::default(),
        );
        assert!(f.tracks.is_empty());
        assert_eq!(f.metadata.filename, "test.nvd");
    }

    #[test]
    fn build_loaded_file_two_trips() {
        let pts = vec![
            make_point_at(0),
            make_point_at(60),
            make_point_at(3600), // gap → new trip
            make_point_at(3660),
        ];
        let f = build_loaded_file(
            "ride.nvd".to_owned(),
            &pts,
            &[],
            vec![],
            vec![],
            &SegmentationConfig::default(),
        );
        assert_eq!(f.tracks.len(), 2);
        assert_eq!(f.tracks[0].points.len(), 2);
        assert_eq!(f.tracks[1].points.len(), 2);
        assert_eq!(f.tracks[0].metadata.index, 1);
        assert_eq!(f.tracks[1].metadata.index, 2);
    }
}
