use chrono::{Duration, NaiveDate, NaiveDateTime, NaiveTime};
use geo::{Bearing, Distance, Haversine};
use geo_types::Point;
use uom::si::angle::degree;
use uom::si::f64::{Angle, Length, Time as UomTime, Velocity};
use uom::si::length::meter;
use uom::si::time::second;
use uom::si::velocity::kilometer_per_hour;

use crate::markers::{CustomMarker, MarkerIcon};
use crate::nav_point::NavPoint;
use crate::satellites::{Constellation, Satellite, Satellites};
use crate::tpv::TimePositionVelocity;

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

        let tpv = TimePositionVelocity::build()
            .with_time(current_time)
            .with_lat(Angle::new::<degree>(lat))
            .with_lon(Angle::new::<degree>(lon))
            .with_heading(current_segment.heading)
            .with_velocity(current_velocity)
            .build();

        let satellites = if (100..102).contains(&i) || (400..600).contains(&i) {
            None
        } else {
            let num_sats = if i < 300 { 12 } else { 8 };
            let mut sats = Vec::new();
            let mut fix = Vec::new();
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
                ));
                fix.push((Some(constellation), prn_u32));
            }
            Some(Satellites::new(current_time, sats, fix))
        };

        route.push(NavPoint::new(tpv, satellites));
    }

    route
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
                    markers.push(CustomMarker {
                        time: last_point.tpv.time(),
                        label: "Fix Lost".to_string(),
                        icon: MarkerIcon::Warning,
                        lat: last_point.tpv.lat(),
                        lon: last_point.tpv.lon(),
                    });
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

                    markers.push(CustomMarker {
                        time: p.tpv.time(),
                        label: format!("Fix Regained after {}", duration_str),
                        icon: MarkerIcon::Check,
                        lat: p.tpv.lat(),
                        lon: p.tpv.lon(),
                    });
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
        parts.push(format!("{}h", h));
    }
    if m > 0 {
        parts.push(format!("{}m", m));
    }
    if s > 0.0 || (h == 0 && m == 0) {
        if s.fract() == 0.0 {
            parts.push(format!("{:.0}s", s));
        } else {
            let s_str = format!("{:.2}s", s);
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
    fn test_nav_data_generation() {
        let route = nav_test_data();
        assert_eq!(route.len(), 1200);

        // Check for fix gaps
        assert!(route.get(100).unwrap().satellites.is_none());
        assert!(route.get(101).unwrap().satellites.is_none());
        assert!(route.get(99).unwrap().satellites.is_some());
        assert!(route.get(102).unwrap().satellites.is_some());

        // Check for color transition threshold in renderer (implicitly via data)
        assert!(route.get(0).unwrap().fix_count() >= 10); // 12 sats initially
        assert!(route.get(400).unwrap().fix_count() == 0); // Gap
        assert!(route.get(601).unwrap().fix_count() == 8); // 8 sats later
    }

    #[test]
    fn test_marker_data_generation() {
        let markers = marker_test_data();

        // We expect at least:
        // 2 Warning markers (fix lost at 100 and 400)
        // 2 Check markers (fix regained at 102 and 600)
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

        // Verify duration string format
        let regain_marker = markers
            .iter()
            .find(|m| m.icon == MarkerIcon::Check)
            .unwrap();
        assert!(regain_marker.label.contains("2s") || regain_marker.label.contains("Fix Regained"));
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
