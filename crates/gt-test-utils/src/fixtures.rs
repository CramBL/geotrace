use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};
use geo::{Bearing, Distance, Haversine};
use geo_types::Point;
use uom::si::angle::degree;
use uom::si::f64::{Angle, Length, Time as UomTime, Velocity};
use uom::si::length::meter;
use uom::si::time::second;
use uom::si::velocity::kilometer_per_hour;

use gt_types::satellites::{Constellation, Satellite, Satellites};
use gt_types::{
    CustomMarker, GpsTime, Latitude, Longitude, MarkerIcon, NavPoint, TimePositionVelocity,
};

struct RouteSegment {
    start: Point<f64>,
    end: Point<f64>,
    distance: Length,
    heading: Angle,
}

#[expect(
    clippy::unwrap_used,
    reason = "Test data generation with hardcoded values"
)]
pub fn nav_test_data() -> Vec<NavPoint> {
    let date = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
    let time = NaiveTime::from_hms_opt(12, 0, 0).unwrap();
    let start_datetime = NaiveDateTime::new(date, time).and_utc();

    let waypoints = [
        Point::new(12.5638, 55.6867),
        Point::new(12.5578, 55.6814),
        Point::new(12.5658, 55.6762),
        Point::new(12.5715, 55.6740),
        Point::new(12.5790, 55.6755),
        Point::new(12.5855, 55.6800),
        Point::new(12.5740, 55.6845),
        Point::new(12.5638, 55.6867),
    ];

    let mut segments = Vec::new();
    let mut total_distance = Length::new::<meter>(0.0);

    for i in 0..(waypoints.len() - 1) {
        let start = waypoints.get(i).copied().unwrap();
        let end = waypoints.get(i + 1).copied().unwrap();

        let dist = Length::new::<meter>(Haversine.distance(start, end));
        let raw_bearing_deg = Haversine.bearing(start, end);
        let heading = Angle::new::<degree>((raw_bearing_deg + 360.0) % 360.0);

        segments.push(RouteSegment {
            start,
            end,
            distance: dist,
            heading,
        });

        total_distance += dist;
    }

    let total_seconds = 1200;
    let mut route = Vec::with_capacity(total_seconds);

    let total_duration = UomTime::new::<second>(total_seconds as f64);
    let avg_velocity: Velocity = total_distance / total_duration;

    for i in 0..total_seconds {
        let current_time = start_datetime + Duration::seconds(i as i64);
        let target_dist = total_distance * (i as f64 / total_seconds as f64);

        let mut dist_accum = Length::new::<meter>(0.0);
        let mut current_segment = segments.first().unwrap();
        let mut segment_progress = 0.0;

        for seg in &segments {
            if dist_accum + seg.distance >= target_dist {
                current_segment = seg;
                if seg.distance > Length::new::<meter>(0.0) {
                    segment_progress = (target_dist.get::<meter>() - dist_accum.get::<meter>())
                        / seg.distance.get::<meter>();
                }
                break;
            }
            dist_accum += seg.distance;
        }

        let lon = current_segment.start.x()
            + (current_segment.end.x() - current_segment.start.x()) * segment_progress;
        let lat = current_segment.start.y()
            + (current_segment.end.y() - current_segment.start.y()) * segment_progress;

        let speed_curve = (segment_progress * std::f64::consts::PI).sin();
        let current_velocity =
            Velocity::new::<kilometer_per_hour>(5.0) + (avg_velocity * 1.5 * speed_curve);

        let tpv = TimePositionVelocity::builder()
            .time(GpsTime::from_utc(current_time))
            .lat(Latitude::new(lat))
            .lon(Longitude::new(lon))
            .heading(current_segment.heading)
            .velocity(current_velocity)
            .build();

        let satellites = if (100..102).contains(&i) || (400..600).contains(&i) {
            None
        } else {
            let num_sats = if i < 300 { 12 } else { 8 };
            let mut sats = Vec::new();
            for prn in 1..=num_sats {
                #[expect(clippy::cast_sign_loss, reason = "prn is always positive")]
                let prn_u32 = prn as u32;
                let constellation = if prn % 2 == 0 {
                    Constellation::Gps
                } else {
                    Constellation::Galileo
                };
                sats.push(Satellite::new(
                    constellation,
                    prn_u32,
                    Some(45.0),
                    Some(prn as f32 * 30.0),
                    Some(30.0 + (prn as f32 % 5.0)),
                    true,
                ));
            }
            Some(Satellites::new(
                Some(GpsTime::from_utc(current_time)),
                None,
                sats,
            ))
        };

        route.push(NavPoint::new(tpv, satellites));
    }

    route
}

/// A single GPS fix at a known location (Copenhagen, 2026-01-01 12:00:00 UTC).
///
/// Useful for edge-case tests that require the minimum valid input.
#[expect(
    clippy::unwrap_used,
    reason = "Test data generation with hardcoded values"
)]
pub fn single_nav_point() -> NavPoint {
    let time = NaiveDateTime::new(
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
    )
    .and_utc();
    let tpv = TimePositionVelocity::builder()
        .time(GpsTime::from_utc(time))
        .lat(Latitude::new(55.6867))
        .lon(Longitude::new(12.5638))
        .heading(Angle::new::<degree>(0.0))
        .velocity(Velocity::new::<kilometer_per_hour>(0.0))
        .build();
    NavPoint::new(tpv, None)
}

/// `count` evenly-spaced GPS fixes, one per second starting at 2026-01-01 12:00:00 UTC.
///
/// All points are at the same location (stationary). No satellite reports.
/// Useful for tests that need a predictable number of fixes without caring about movement.
#[expect(
    clippy::unwrap_used,
    reason = "Test data generation with hardcoded values"
)]
pub fn stationary_nav_data(count: usize) -> Vec<NavPoint> {
    let start = NaiveDateTime::new(
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
    )
    .and_utc();
    (0..count)
        .map(|i| {
            let time = start + Duration::seconds(i as i64);
            let tpv = TimePositionVelocity::builder()
                .time(GpsTime::from_utc(time))
                .lat(Latitude::new(55.6867))
                .lon(Longitude::new(12.5638))
                .heading(Angle::new::<degree>(0.0))
                .velocity(Velocity::new::<kilometer_per_hour>(0.0))
                .build();
            NavPoint::new(tpv, None)
        })
        .collect()
}

/// Two groups of GPS fixes separated by a 10-minute gap, suitable for track segmentation tests.
///
/// Returns `first_count + second_count` points. The gap falls between index `first_count - 1`
/// and `first_count`. Both groups move along the same straight line; no satellite reports.
#[expect(
    clippy::unwrap_used,
    reason = "Test data generation with hardcoded values"
)]
pub fn nav_data_with_gap(first_count: usize, second_count: usize) -> Vec<NavPoint> {
    let start = NaiveDateTime::new(
        NaiveDate::from_ymd_opt(2026, 1, 1).unwrap(),
        NaiveTime::from_hms_opt(12, 0, 0).unwrap(),
    )
    .and_utc();
    let gap = Duration::minutes(10);

    let make_point = |i: usize, time_offset: Duration| -> NavPoint {
        let time = start + time_offset;
        let lat = 55.6867 + (i as f64) * 0.0001;
        let lon = 12.5638 + (i as f64) * 0.0001;
        let tpv = TimePositionVelocity::builder()
            .time(GpsTime::from_utc(time))
            .lat(Latitude::new(lat))
            .lon(Longitude::new(lon))
            .heading(Angle::new::<degree>(45.0))
            .velocity(Velocity::new::<kilometer_per_hour>(15.0))
            .build();
        NavPoint::new(tpv, None)
    };

    let mut points = Vec::with_capacity(first_count + second_count);
    for i in 0..first_count {
        points.push(make_point(i, Duration::seconds(i as i64)));
    }
    let second_start = Duration::seconds(first_count as i64) + gap;
    for i in 0..second_count {
        points.push(make_point(
            first_count + i,
            second_start + Duration::seconds(i as i64),
        ));
    }
    points
}

/// `count` GPS fixes starting at `start`, separated by `step_secs` each.
///
/// Points move slightly north-east (0.001°/step) to avoid zero-distance degenerate tracks.
/// No satellite reports. Useful when tests need full control over timestamps.
pub fn nav_points_from(
    start: chrono::DateTime<chrono::Utc>,
    count: usize,
    step_secs: i64,
) -> Vec<NavPoint> {
    (0..count)
        .map(|i| {
            let time = start + Duration::seconds(i as i64 * step_secs);
            let tpv = TimePositionVelocity::builder()
                .time(GpsTime::from_utc(time))
                .lat(Latitude::new(55.0 + i as f64 * 0.001))
                .lon(Longitude::new(12.0 + i as f64 * 0.001))
                .heading(Angle::new::<degree>(45.0))
                .velocity(Velocity::new::<kilometer_per_hour>(15.0))
                .build();
            NavPoint::new(tpv, None)
        })
        .collect()
}

pub fn marker_test_data() -> Vec<CustomMarker> {
    let nav_points = nav_test_data();
    let mut markers = Vec::new();

    let mut last_fix_index: Option<usize> = None;

    for (i, p) in nav_points.iter().enumerate() {
        let has_fix = p.fix_count() > 0;

        match (last_fix_index, has_fix) {
            (Some(last_idx), false) => {
                if let Some(last_point) = nav_points.get(last_idx) {
                    markers.push(CustomMarker::new(
                        last_point.tpv.time().utc(),
                        "Fix Lost".to_string(),
                        MarkerIcon::Warning,
                        last_point.tpv.lat(),
                        last_point.tpv.lon(),
                        None,
                    ));
                }
                last_fix_index = None;
            }
            (None, true) => {
                let is_fix_regain = i
                    .checked_sub(1)
                    .and_then(|idx| nav_points.get(idx))
                    .is_some_and(|prev| prev.fix_count() == 0);

                if is_fix_regain {
                    let mut fix_lost_time = p.tpv.time();
                    for j in (0..i).rev() {
                        if let Some(np) = nav_points.get(j).filter(|np| np.fix_count() > 0) {
                            fix_lost_time = np.tpv.time();
                            break;
                        }
                    }

                    let duration = p.tpv.time().signed_duration_since(fix_lost_time);
                    let duration_str = format_duration(duration);

                    markers.push(CustomMarker::new(
                        p.tpv.time().utc(),
                        format!("Fix Regained after {duration_str}"),
                        MarkerIcon::Check,
                        p.tpv.lat(),
                        p.tpv.lon(),
                        None,
                    ));
                }
                last_fix_index = Some(i);
            }
            _ => {
                if has_fix {
                    last_fix_index = Some(i);
                }
            }
        }
    }

    markers
}

fn format_duration(duration: Duration) -> String {
    let total_ms = duration.num_milliseconds();
    let h = total_ms / 3_600_000;
    let m = (total_ms % 3_600_000) / 60_000;
    let s = (total_ms % 60_000) as f64 / 1000.0;

    let mut parts = Vec::new();
    if h > 0 {
        parts.push(format!("{h}h"));
    }
    if m > 0 {
        parts.push(format!("{m}m"));
    }
    if s > 0.0 || (h == 0 && m == 0) {
        if s.fract() == 0.0 {
            parts.push(format!("{s:.0}s"));
        } else {
            let s_str = format!("{s:.2}s");
            let trimmed = s_str
                .trim_end_matches('s')
                .trim_end_matches('0')
                .trim_end_matches('.')
                .to_string()
                + "s";
            parts.push(trimmed);
        }
    }
    parts.concat()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_nav_point_is_valid() {
        let p = single_nav_point();
        assert!(p.tpv.lat().as_degrees() > 55.0 && p.tpv.lat().as_degrees() < 56.0);
        assert!(p.satellites.is_none());
    }

    #[test]
    fn stationary_nav_data_has_correct_count() {
        let points = stationary_nav_data(5);
        assert_eq!(points.len(), 5);
        let lats: Vec<f64> = points.iter().map(|p| p.tpv.lat().as_degrees()).collect();
        assert!(lats.windows(2).all(|w| (w[0] - w[1]).abs() < 1e-9));
    }

    #[test]
    fn stationary_nav_data_has_consecutive_timestamps() {
        let points = stationary_nav_data(3);
        let t0 = points[0].tpv.time().utc();
        let t1 = points[1].tpv.time().utc();
        let t2 = points[2].tpv.time().utc();
        assert_eq!((t1 - t0).num_seconds(), 1);
        assert_eq!((t2 - t1).num_seconds(), 1);
    }

    #[test]
    fn nav_data_with_gap_has_correct_total_count() {
        let points = nav_data_with_gap(3, 4);
        assert_eq!(points.len(), 7);
    }

    #[test]
    fn nav_data_with_gap_has_gap_between_groups() {
        let points = nav_data_with_gap(3, 4);
        let gap_start = points[2].tpv.time().utc();
        let gap_end = points[3].tpv.time().utc();
        assert!(
            (gap_end - gap_start).num_seconds() > 300,
            "gap should be >5 minutes"
        );
    }

    #[test]
    fn test_nav_data_generation() {
        let route = nav_test_data();
        assert_eq!(route.len(), 1200);

        assert!(route.get(100).is_some_and(|p| p.satellites.is_none()));
        assert!(route.get(101).is_some_and(|p| p.satellites.is_none()));
        assert!(route.get(99).is_some_and(|p| p.satellites.is_some()));
        assert!(route.get(102).is_some_and(|p| p.satellites.is_some()));

        assert!(route.first().is_some_and(|p| p.fix_count() >= 10));
        assert!(route.get(400).is_some_and(|p| p.fix_count() == 0));
        assert!(route.get(601).is_some_and(|p| p.fix_count() == 8));
    }

    #[test]
    fn test_marker_data_generation() {
        let markers = marker_test_data();

        let warning_count = markers
            .iter()
            .filter(|m| m.icon == MarkerIcon::Warning)
            .count();
        let check_count = markers
            .iter()
            .filter(|m| m.icon == MarkerIcon::Check)
            .count();

        assert!(warning_count >= 2);
        assert!(check_count >= 2);

        let regain_marker = markers.iter().find(|m| m.icon == MarkerIcon::Check);
        assert!(
            regain_marker
                .is_some_and(|m| m.label.contains("2s") || m.label.contains("Fix Regained"))
        );
    }

    #[test]
    fn test_format_duration() {
        assert_eq!(format_duration(Duration::seconds(5)), "5s");
        assert_eq!(format_duration(Duration::seconds(65)), "1m5s");
        assert_eq!(format_duration(Duration::seconds(3665)), "1h1m5s");
        assert_eq!(format_duration(Duration::milliseconds(40210)), "40.21s");
        assert_eq!(format_duration(Duration::milliseconds(0)), "0s");
    }
}
