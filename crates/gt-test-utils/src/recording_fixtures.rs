//! A recording assembled fix by fix: a Copenhagen loop with satellite reports
//! and the markers that its fix losses generate, and the same shape written out
//! as `.gtd` bytes.

use chrono::{DateTime, Duration, NaiveDate, NaiveDateTime, NaiveTime, Utc};
use geo::{Bearing, Distance, Haversine};
use geo_types::Point;
use geotrace_sdk as sdk;
use uom::si::angle::degree;
use uom::si::f64::{Angle, Length, Time as UomTime, Velocity};
use uom::si::length::meter;
use uom::si::time::second;
use uom::si::velocity::kilometer_per_hour;

use gt_types::satellites::{Constellation, Satellite, Satellites};
use gt_types::{
    CustomMarker, GeneratedMarkerKind, GpsTime, Latitude, Longitude, MarkerIcon, NavPoint,
    TimePositionVelocity,
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

pub fn marker_test_data() -> Vec<CustomMarker> {
    let nav_points = nav_test_data();
    let mut markers = Vec::new();

    let mut last_fix_index: Option<usize> = None;

    for (i, p) in nav_points.iter().enumerate() {
        let has_fix = p.fix_count() > 0;

        match (last_fix_index, has_fix) {
            (Some(last_idx), false) => {
                if let Some(last_point) = nav_points.get(last_idx)
                    && let Some((lat, lon)) = last_point.tpv.position()
                {
                    markers.push(CustomMarker::new(
                        last_point.tpv.time().utc(),
                        GeneratedMarkerKind::GnssFixLost.to_string(),
                        MarkerIcon::Warning,
                        lat,
                        lon,
                    ));
                }
                last_fix_index = None;
            }
            (None, true) => {
                let is_fix_regain = i
                    .checked_sub(1)
                    .and_then(|idx| nav_points.get(idx))
                    .is_some_and(|prev| prev.fix_count() == 0);

                if is_fix_regain && let Some((lat, lon)) = p.tpv.position() {
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
                        format!(
                            "{} after {duration_str}",
                            GeneratedMarkerKind::GnssFixRegained {
                                fix_lost_duration: duration
                            }
                        ),
                        MarkerIcon::Check,
                        lat,
                        lon,
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

#[derive(Debug, Clone, Copy)]
pub struct SyntheticGtdSpec {
    pub start: DateTime<Utc>,
    pub point_count: usize,
    pub step_secs: i64,
    pub start_lat_deg: f64,
    pub start_lon_deg: f64,
    pub lat_step_deg: f64,
    pub lon_step_deg: f64,
    pub heading_deg: f64,
    pub speed_kmh: f64,
    pub eph_m: f64,
    pub sats_seen: u32,
    pub sats_in_fix: u32,
}

/// Build synthetic `.gtd` bytes with both nav fixes and satellite reports.
///
/// This is intentionally generic so snapshot and integration tests can create
/// overlapping tracks with controlled timing and metric values.
pub fn synthetic_gtd_bytes(spec: SyntheticGtdSpec) -> Vec<u8> {
    synthetic_gtd_bytes_with_channels(spec, Vec::new())
}

/// [`synthetic_gtd_bytes`] with ad-hoc channels alongside the nav data, for
/// tests driving channel-source queries (`@name | …`) through the real load
/// path.
#[expect(
    clippy::expect_used,
    reason = "Fixture generation should fail loudly when test input is invalid"
)]
pub fn synthetic_gtd_bytes_with_channels(
    spec: SyntheticGtdSpec,
    channels: Vec<sdk::Channel>,
) -> Vec<u8> {
    let mut recorder = sdk::NavFileBuilder::new().open();
    for channel in channels {
        recorder.add_channel(channel);
    }
    for i in 0..spec.point_count {
        let i_i64 = i64::try_from(i).unwrap_or(0);
        let time = spec.start + Duration::seconds(i_i64 * spec.step_secs);

        recorder.add_nav_fix(
            sdk::NavFix::builder()
                .time(sdk::NavFixTime::Receiver(time))
                .lat(sdk::Angle::degrees(
                    spec.start_lat_deg + i as f64 * spec.lat_step_deg,
                ))
                .lon(sdk::Angle::degrees(
                    spec.start_lon_deg + i as f64 * spec.lon_step_deg,
                ))
                .heading(sdk::Angle::degrees(spec.heading_deg))
                .speed(sdk::Velocity::kilometer_per_hour(spec.speed_kmh))
                .eph_m(spec.eph_m)
                .build(),
        );

        recorder.add_satellite_report(
            sdk::SatelliteReport::builder()
                .time(sdk::NavFixTime::Receiver(time))
                .tracked(synthetic_satellites(spec.sats_seen, spec.sats_in_fix))
                .build(),
        );
    }

    let nav_file = recorder
        .finish()
        .expect("synthetic test data must be valid");
    let mut bytes = Vec::new();
    nav_file
        .write(&mut bytes)
        .expect("writing synthetic test file must succeed");
    bytes
}

fn synthetic_satellites(sats_seen: u32, sats_in_fix: u32) -> Vec<sdk::Satellite> {
    (1..=sats_seen)
        .map(|prn| {
            let constellation = match prn % 4 {
                0 => sdk::Constellation::Gps,
                1 => sdk::Constellation::Glonass,
                2 => sdk::Constellation::Galileo,
                _ => sdk::Constellation::Beidou,
            };
            sdk::Satellite::builder()
                .constellation(constellation)
                .prn(prn)
                .in_fix(prn <= sats_in_fix)
                .elevation(30.0 + (prn % 20) as f32)
                .azimuth((prn * 27 % 360) as f32)
                .snr(28.0 + (prn % 12) as f32)
                .build()
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nav_test_data_drops_its_reports_over_two_stretches_and_its_satellites_at_fix_300() {
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
    fn marker_test_data_marks_every_fix_loss_and_regain_of_the_route() {
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
        assert!(regain_marker.is_some_and(|m| {
            m.label.contains(
                &GeneratedMarkerKind::GnssFixRegained {
                    fix_lost_duration: Duration::zero(),
                }
                .to_string(),
            )
        }));
    }
}
