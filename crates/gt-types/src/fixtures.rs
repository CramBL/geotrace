//! Builders for the nav points, satellite reports and channels that tests
//! across the workspace assemble by hand.

use chrono::{DateTime, Duration, NaiveDate, Utc};
use geotrace_sdk_units::ChannelUnit;
use uom::si::angle::degree;
use uom::si::f64::{Angle, Velocity};
use uom::si::velocity::kilometer_per_hour;

use crate::channel::Channel;
use crate::coordinates::{Latitude, Longitude, RecordedLatitude};
use crate::nav_point::NavPoint;
use crate::satellites::{Constellation, Satellite, Satellites, Snr};
use crate::time_types::{GpsTime, SysTime};
use crate::tpv::TimePositionVelocity;

/// The heading of every fixture fix that has one.
const EASTWARD_HEADING_DEGREES: f64 = 90.0;

/// The satellites in the fix of a [`FixKind`] the receiver measured.
const SATELLITES_IN_FIX: u32 = 12;

/// The satellites in view of a [`FixKind`] with nothing in its fix.
const SATELLITES_IN_VIEW_ONLY: u32 = 4;

/// The signal quality every satellite of [`nav_points_with_drifting_satellites`]
/// reports.
const DRIFTING_SATELLITE_SNR_DB: f32 = 40.0;

/// The heading and satellite report a fixture fix is built with.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum FixKind {
    /// Heading 90°, 12 GPS satellites in fix.
    #[default]
    Measured,
    /// No heading, 12 satellites in fix.
    MeasuredWithoutHeading,
    /// No heading and no satellite report.
    GhostWithoutHeading,
    /// Heading 90°, a report of 4 satellites in view and none in fix.
    GhostWithoutSatellitesInFix,
    /// [`FixKind::Measured`] with a latitude of NaN in place of the one the
    /// caller passes.
    WithoutAPosition,
}

impl FixKind {
    fn heading(self) -> Option<Angle> {
        match self {
            Self::Measured | Self::GhostWithoutSatellitesInFix | Self::WithoutAPosition => {
                Some(Angle::new::<degree>(EASTWARD_HEADING_DEGREES))
            }
            Self::MeasuredWithoutHeading | Self::GhostWithoutHeading => None,
        }
    }

    fn satellite_report(self, time: DateTime<Utc>) -> Option<Satellites> {
        let counts = match self {
            Self::Measured | Self::MeasuredWithoutHeading | Self::WithoutAPosition => {
                SatelliteCounts {
                    in_fix: SATELLITES_IN_FIX,
                    in_view_only: 0,
                }
            }
            Self::GhostWithoutSatellitesInFix => SatelliteCounts {
                in_fix: 0,
                in_view_only: SATELLITES_IN_VIEW_ONLY,
            },
            Self::GhostWithoutHeading => return None,
        };
        Some(satellite_report(Some(GpsTime::from_utc(time)), counts))
    }

    fn recorded_latitude(self, lat: Latitude) -> RecordedLatitude {
        match self {
            Self::WithoutAPosition => RecordedLatitude::from_degrees(f64::NAN),
            Self::Measured
            | Self::MeasuredWithoutHeading
            | Self::GhostWithoutHeading
            | Self::GhostWithoutSatellitesInFix => lat.into(),
        }
    }
}

/// A fixture fix beyond its time and position.
#[derive(Debug, Clone, Copy, Default)]
pub struct NavPointSpec {
    pub fix: FixKind,
    pub eph_m: Option<f32>,
}

/// How many satellites [`satellite_report`] marks in the fix, and how many
/// further ones it lists without that mark.
#[derive(Debug, Clone, Copy)]
pub struct SatelliteCounts {
    pub in_fix: u32,
    pub in_view_only: u32,
}

/// A position offset from the origin in metres, east and north positive.
#[derive(Debug, Clone, Copy)]
pub struct MetricOffset {
    pub east_m: f64,
    pub north_m: f64,
}

impl MetricOffset {
    /// The position this far from the origin, for tests that place fixes at
    /// controlled metric spacings.
    pub fn to_latlon(self) -> (Latitude, Longitude) {
        // A metre at the equator, in degrees.
        const DEGREES_PER_METER: f64 = 360.0 / 40_030_173.0;
        (
            Latitude::new(self.north_m * DEGREES_PER_METER),
            Longitude::new(self.east_m * DEGREES_PER_METER),
        )
    }
}

/// How many fixes [`nav_data_with_gap`] puts on each side of its gap.
#[derive(Debug, Clone, Copy)]
pub struct FixCountsAroundAGap {
    pub before: usize,
    pub after: usize,
}

/// One satellite sweeping across the sky over the epochs of
/// [`nav_points_with_drifting_satellites`].
#[derive(Debug, Clone, Copy)]
pub struct SatelliteDrift {
    pub constellation: Constellation,
    pub prn: u32,
    /// Azimuth in degrees at the first and the last epoch.
    pub azimuth_deg: (f32, f32),
    /// Elevation in degrees at the first and the last epoch.
    pub elevation_deg: (f32, f32),
    pub in_fix: bool,
    /// The epochs this satellite is missing from.
    pub absent_at: &'static [usize],
}

/// A [`NavPoint`] at `time`, `lat` and `lon`, with the heading and the
/// satellite report of `kind`.
pub fn nav_point(time: DateTime<Utc>, lat: Latitude, lon: Longitude, kind: FixKind) -> NavPoint {
    nav_point_heading(time, lat, lon, kind.heading(), kind)
}

/// [`nav_point`] with a heading of the caller's choosing in place of the one
/// `kind` gives, for a fixture whose fixes run in more than one direction.
pub fn nav_point_heading(
    time: DateTime<Utc>,
    lat: Latitude,
    lon: Longitude,
    heading: Option<Angle>,
    kind: FixKind,
) -> NavPoint {
    nav_point_from_spec(
        time,
        lat,
        lon,
        heading,
        NavPointSpec {
            fix: kind,
            eph_m: None,
        },
    )
}

/// [`nav_point`] with a host-clock timestamp of `host_ahead` past the
/// receiver's own.
pub fn nav_point_with_host_clock(
    time: DateTime<Utc>,
    host_ahead: Duration,
    lat: Latitude,
    lon: Longitude,
    kind: FixKind,
) -> NavPoint {
    let tpv = TimePositionVelocity::builder()
        .time(GpsTime::from_utc(time))
        .lat(kind.recorded_latitude(lat))
        .lon(lon)
        .maybe_heading(kind.heading())
        .sys_time(SysTime::from_utc(time + host_ahead))
        .build();
    NavPoint::new(tpv, kind.satellite_report(time))
}

/// A measured [`NavPoint`] with the caller's satellite report on it, heading
/// 90°.
pub fn nav_point_with_report(
    time: DateTime<Utc>,
    lat: Latitude,
    lon: Longitude,
    report: Satellites,
) -> NavPoint {
    let tpv = TimePositionVelocity::builder()
        .time(GpsTime::from_utc(time))
        .lat(lat)
        .lon(lon)
        .heading(Angle::new::<degree>(EASTWARD_HEADING_DEGREES))
        .build();
    NavPoint::new(tpv, Some(report))
}

fn nav_point_from_spec(
    time: DateTime<Utc>,
    lat: Latitude,
    lon: Longitude,
    heading: Option<Angle>,
    spec: NavPointSpec,
) -> NavPoint {
    let tpv = TimePositionVelocity::builder()
        .time(GpsTime::from_utc(time))
        .lat(spec.fix.recorded_latitude(lat))
        .lon(lon)
        .maybe_heading(heading)
        .maybe_eph_m(spec.eph_m)
        .build();
    NavPoint::new(tpv, spec.fix.satellite_report(time))
}

/// One satellite of `constellation`, without an azimuth.
pub fn satellite(
    constellation: Constellation,
    prn: u32,
    elevation_deg: Option<f32>,
    snr: Option<Snr>,
    in_fix: bool,
) -> Satellite {
    Satellite::new(
        constellation,
        prn,
        elevation_deg,
        None,
        snr.map(Snr::value),
        in_fix,
    )
}

/// A report of GPS satellites numbered from PRN 1, the first `counts.in_fix`
/// of them in the fix and the next `counts.in_view_only` seen without being
/// used.
pub fn satellite_report(time: Option<GpsTime>, counts: SatelliteCounts) -> Satellites {
    let satellites = (0..counts.in_fix + counts.in_view_only)
        .map(|index| {
            Satellite::new(
                Constellation::Gps,
                index + 1,
                None,
                None,
                None,
                index < counts.in_fix,
            )
        })
        .collect();
    Satellites::new(time, None, satellites)
}

/// A straight walk north-east from one position, one fix per step.
struct Walk {
    start: DateTime<Utc>,
    first_lat: Latitude,
    first_lon: Longitude,
    step: Duration,
    stride_degrees: f64,
    heading: Option<Angle>,
    velocity: Option<Velocity>,
}

impl Walk {
    /// A walk on a heading of 45° at 15 km/h.
    fn north_east(
        start: DateTime<Utc>,
        first_lat: Latitude,
        first_lon: Longitude,
        step_secs: i64,
        stride_degrees: f64,
    ) -> Self {
        Self {
            start,
            first_lat,
            first_lon,
            step: Duration::seconds(step_secs),
            stride_degrees,
            heading: Some(Angle::new::<degree>(45.0)),
            velocity: Some(Velocity::new::<kilometer_per_hour>(15.0)),
        }
    }

    /// The fix `index` steps along the walk, with `time` as its timestamp.
    fn point_at(&self, index: usize, time: DateTime<Utc>) -> NavPoint {
        let walked = index as f64 * self.stride_degrees;
        let tpv = TimePositionVelocity::builder()
            .time(GpsTime::from_utc(time))
            .lat(Latitude::new(self.first_lat.as_degrees() + walked))
            .lon(Longitude::new(self.first_lon.as_degrees() + walked))
            .maybe_heading(self.heading)
            .maybe_velocity(self.velocity)
            .build();
        NavPoint::new(tpv, None)
    }

    fn point(&self, index: usize) -> NavPoint {
        self.point_at(index, self.start + self.step * index as i32)
    }

    fn points(&self, count: usize) -> Vec<NavPoint> {
        (0..count).map(|index| self.point(index)).collect()
    }
}

/// `count` fixes `step_secs` apart from `start`, walking north-east from
/// 55°N 12°E in 0.001° steps at 15 km/h, without a satellite report.
pub fn nav_points_from(start: DateTime<Utc>, count: usize, step_secs: i64) -> Vec<NavPoint> {
    nav_points_walking_from(
        start,
        count,
        step_secs,
        Latitude::new(55.0),
        Longitude::new(12.0),
    )
}

/// [`nav_points_from`] starting at a position of the caller's choosing, for
/// tests that tell two recordings apart by where their fixes are.
pub fn nav_points_walking_from(
    start: DateTime<Utc>,
    count: usize,
    step_secs: i64,
    first_lat: Latitude,
    first_lon: Longitude,
) -> Vec<NavPoint> {
    Walk::north_east(start, first_lat, first_lon, step_secs, 0.001).points(count)
}

/// `count` fixes one second apart from 2026-01-01 12:00:00 UTC, all at 55.6867°N
/// 12.5638°E with a heading of 0° and a velocity of 0 km/h.
pub fn stationary_nav_data(count: usize) -> Vec<NavPoint> {
    Walk {
        start: fixture_start(),
        first_lat: Latitude::new(55.6867),
        first_lon: Longitude::new(12.5638),
        step: Duration::seconds(1),
        stride_degrees: 0.0,
        heading: Some(Angle::new::<degree>(0.0)),
        velocity: Some(Velocity::new::<kilometer_per_hour>(0.0)),
    }
    .points(count)
}

/// `counts.before + counts.after` fixes one second apart from 2026-01-01
/// 12:00:00 UTC, walking north-east from 55.6867°N 12.5638°E in 0.0001° steps,
/// with a 10 minute gap between the two groups. The walk continues across the
/// gap.
pub fn nav_data_with_gap(counts: FixCountsAroundAGap) -> Vec<NavPoint> {
    let gap = Duration::minutes(10);
    let walk = Walk::north_east(
        fixture_start(),
        Latitude::new(55.6867),
        Longitude::new(12.5638),
        1,
        0.0001,
    );
    let mut points = walk.points(counts.before);
    points.extend((0..counts.after).map(|index| {
        let position = counts.before + index;
        walk.point_at(position, walk.start + walk.step * position as i32 + gap)
    }));
    points
}

/// `count` fixes `step_ms` apart from `start`, walking north from 55°N 12°E in
/// 1e-5° steps, each built from the spec of `spec(index)`.
///
/// The millisecond spacing and the fine steps suit tests over fix quality and
/// sampling rate. [`nav_points_from`] is the coarser per-second walk.
pub fn nav_points_from_specs(
    start: DateTime<Utc>,
    count: usize,
    step_ms: i64,
    spec: impl Fn(usize) -> NavPointSpec,
) -> Vec<NavPoint> {
    (0..count)
        .map(|index| {
            let spec = spec(index);
            nav_point_from_spec(
                start + Duration::milliseconds(index as i64 * step_ms),
                Latitude::new(55.0 + index as f64 * 1e-5),
                Longitude::new(12.0),
                spec.fix.heading(),
                spec,
            )
        })
        .collect()
}

/// A [`NavPoint`] at [`MetricOffset::to_latlon`], without a heading and with
/// the Unix epoch as its timestamp, for the placement tests, which are
/// time-independent.
pub fn nav_point_at_meters(offset: MetricOffset, satellites: Option<Satellites>) -> NavPoint {
    let (lat, lon) = offset.to_latlon();
    let tpv = TimePositionVelocity::builder()
        .time(GpsTime::from_utc(DateTime::<Utc>::UNIX_EPOCH))
        .lat(lat)
        .lon(lon)
        .build();
    NavPoint::new(tpv, satellites)
}

/// A [`Channel`] of one value per timestamp.
pub fn scalar_channel(
    name: &str,
    unit: Option<&str>,
    times: Vec<DateTime<Utc>>,
    values: Vec<f64>,
) -> Channel {
    Channel {
        name: name.to_owned(),
        unit: unit.map(ChannelUnit::from_file_label),
        period: None,
        description: None,
        components: Vec::new(),
        times,
        values,
    }
}

/// A [`Channel`] of one labelled column per component, `values` row-major.
pub fn vector_channel(
    name: &str,
    unit: Option<&str>,
    components: &[&str],
    times: Vec<DateTime<Utc>>,
    values: Vec<f64>,
) -> Channel {
    Channel {
        name: name.to_owned(),
        unit: unit.map(ChannelUnit::from_file_label),
        period: None,
        description: None,
        components: components.iter().map(|label| (*label).to_owned()).collect(),
        times,
        values,
    }
}

/// One fix per epoch, a second apart from `start`, at 55°N 12°E. Each
/// satellite of `drifts` sits at the linear interpolation between its first
/// and its last epoch, at an SNR of 40 dB. A single epoch puts every
/// satellite at the start of its drift.
pub fn nav_points_with_drifting_satellites(
    start: DateTime<Utc>,
    epochs: usize,
    drifts: &[SatelliteDrift],
) -> Vec<NavPoint> {
    let interpolate = |(first, last): (f32, f32), elapsed: f32| first + (last - first) * elapsed;
    (0..epochs)
        .map(|epoch| {
            let elapsed = if epochs > 1 {
                epoch as f32 / (epochs - 1) as f32
            } else {
                0.0
            };
            let satellites = drifts
                .iter()
                .filter(|drift| !drift.absent_at.contains(&epoch))
                .map(|drift| {
                    Satellite::new(
                        drift.constellation,
                        drift.prn,
                        Some(interpolate(drift.elevation_deg, elapsed)),
                        Some(interpolate(drift.azimuth_deg, elapsed)),
                        Some(DRIFTING_SATELLITE_SNR_DB),
                        drift.in_fix,
                    )
                })
                .collect();
            let tpv = TimePositionVelocity::builder()
                .time(GpsTime::from_utc(start + Duration::seconds(epoch as i64)))
                .lat(Latitude::new(55.0))
                .lon(Longitude::new(12.0))
                .build();
            NavPoint::new(tpv, Some(Satellites::new(None, None, satellites)))
        })
        .collect()
}

/// The start of [`stationary_nav_data`] and [`nav_data_with_gap`].
#[expect(
    clippy::expect_used,
    reason = "a hardcoded date the fixture builders depend on"
)]
fn fixture_start() -> DateTime<Utc> {
    NaiveDate::from_ymd_opt(2026, 1, 1)
        .and_then(|date| date.and_hms_opt(12, 0, 0))
        .map(|naive| naive.and_utc())
        .expect("2026-01-01 12:00:00 is a date and a time")
}

#[cfg(test)]
mod tests {
    use super::*;
    use rstest::rstest;

    use crate::test_util;

    fn at(secs: i64) -> DateTime<Utc> {
        DateTime::<Utc>::UNIX_EPOCH + Duration::seconds(secs)
    }

    fn heading_degrees(point: &NavPoint) -> Option<f64> {
        point.tpv.heading().map(|angle| angle.get::<degree>())
    }

    #[rstest]
    #[case::measured(FixKind::Measured, Some(90.0), 12, 12, true)]
    #[case::measured_without_heading(FixKind::MeasuredWithoutHeading, None, 12, 12, true)]
    #[case::ghost_without_heading(FixKind::GhostWithoutHeading, None, 0, 0, true)]
    #[case::ghost_without_satellites_in_fix(
        FixKind::GhostWithoutSatellitesInFix,
        Some(90.0),
        0,
        4,
        true
    )]
    #[case::without_a_position(FixKind::WithoutAPosition, Some(90.0), 12, 12, false)]
    fn nav_point_builds_the_heading_and_report_of_its_fix_kind(
        #[case] kind: FixKind,
        #[case] expected_heading_degrees: Option<f64>,
        #[case] expected_fix_count: u32,
        #[case] expected_satellite_count: u32,
        #[case] expected_position: bool,
    ) {
        let point = nav_point(at(0), Latitude::new(55.0), Longitude::new(12.0), kind);

        assert_eq!(heading_degrees(&point), expected_heading_degrees);
        assert_eq!(point.fix_count(), expected_fix_count);
        assert_eq!(point.total_satellites(), expected_satellite_count);
        assert_eq!(point.tpv.position().is_some(), expected_position);
    }

    #[test]
    fn nav_point_heading_replaces_the_heading_of_its_fix_kind() {
        let point = nav_point_heading(
            at(0),
            Latitude::new(55.0),
            Longitude::new(12.0),
            Some(Angle::new::<degree>(270.0)),
            FixKind::Measured,
        );

        assert_eq!(heading_degrees(&point), Some(270.0));
        assert_eq!(point.fix_count(), 12);
    }

    #[test]
    fn nav_point_with_host_clock_sets_the_host_timestamp_past_the_receivers() {
        let point = nav_point_with_host_clock(
            at(100),
            Duration::milliseconds(300),
            Latitude::new(55.0),
            Longitude::new(12.0),
            FixKind::Measured,
        );

        assert_eq!(point.tpv.gps_time(), Some(GpsTime::from_utc(at(100))));
        assert_eq!(
            point.tpv.sys_time().map(SysTime::utc),
            Some(at(100) + Duration::milliseconds(300))
        );
        assert_eq!(
            point.tpv.gps_system_clock_offset(),
            Some(Duration::milliseconds(-300))
        );
    }

    #[test]
    fn satellite_puts_its_elevation_and_signal_quality_on_the_satellite() {
        let satellite = satellite(
            Constellation::Galileo,
            7,
            Some(31.0),
            Some(Snr::new(38.0)),
            true,
        );

        assert_eq!(satellite.constellation(), Constellation::Galileo);
        assert_eq!(satellite.prn().value(), 7);
        assert_eq!(satellite.elevation(), Some(31.0));
        assert_eq!(satellite.azimuth(), None);
        assert_eq!(satellite.snr().map(Snr::value), Some(38.0));
        assert!(satellite.in_fix());
    }

    #[test]
    fn nav_points_from_specs_builds_each_point_from_the_spec_of_its_index() {
        let points = nav_points_from_specs(at(0), 2, 500, |index| {
            if index == 0 {
                NavPointSpec {
                    fix: FixKind::Measured,
                    eph_m: Some(2.5),
                }
            } else {
                NavPointSpec {
                    fix: FixKind::GhostWithoutHeading,
                    eph_m: None,
                }
            }
        });

        let built: Vec<(Option<f32>, u32, i64)> = points
            .iter()
            .map(|point| {
                (
                    point.tpv.eph_m(),
                    point.fix_count(),
                    point.tpv.time().utc().timestamp_millis(),
                )
            })
            .collect();
        assert_eq!(built, vec![(Some(2.5), 12, 0), (None, 0, 500)]);
    }

    #[test]
    fn nav_points_walking_from_starts_at_the_position_the_caller_passes() {
        let points =
            nav_points_walking_from(at(0), 2, 1, Latitude::new(-33.86), Longitude::new(151.21));

        let positions: Vec<(f64, f64)> = points
            .iter()
            .filter_map(|point| point.tpv.position())
            .map(|(lat, lon)| (lat.as_degrees(), lon.as_degrees()))
            .collect();
        assert_eq!(positions.len(), 2);
        for (index, (latitude, longitude)) in positions.into_iter().enumerate() {
            test_util::assert_degrees_close(latitude, -33.86 + index as f64 * 0.001);
            test_util::assert_degrees_close(longitude, 151.21 + index as f64 * 0.001);
        }
    }

    /// [`MetricOffset::to_latlon`] takes the earth for a sphere. A degree of
    /// longitude at the equator and a degree of latitude are both 111 195 m.
    #[test]
    fn to_latlon_converts_111_195_metres_to_one_degree() {
        let (latitude, longitude) = MetricOffset {
            east_m: 111_195.0,
            north_m: 222_390.0,
        }
        .to_latlon();

        assert!((longitude.as_degrees() - 1.0).abs() < 1e-5);
        assert!((latitude.as_degrees() - 2.0).abs() < 1e-5);
    }

    #[test]
    fn nav_point_at_meters_has_no_heading_and_the_unix_epoch_as_its_time() {
        let report = satellite_report(
            None,
            SatelliteCounts {
                in_fix: 4,
                in_view_only: 0,
            },
        );

        let point = nav_point_at_meters(
            MetricOffset {
                east_m: 111_195.0,
                north_m: 0.0,
            },
            Some(report),
        );

        assert_eq!(point.tpv.time().utc(), DateTime::<Utc>::UNIX_EPOCH);
        assert_eq!(heading_degrees(&point), None);
        assert_eq!(point.fix_count(), 4);
        let (latitude, longitude) = point.tpv.position().expect("a position in range");
        assert!((longitude.as_degrees() - 1.0).abs() < 1e-5);
        test_util::assert_degrees_close(latitude.as_degrees(), 0.0);
    }

    #[test]
    fn nav_point_with_report_puts_the_callers_report_on_the_fix() {
        let report = satellite_report(
            Some(GpsTime::from_utc(at(7))),
            SatelliteCounts {
                in_fix: 3,
                in_view_only: 2,
            },
        );

        let point = nav_point_with_report(at(0), Latitude::new(55.0), Longitude::new(12.0), report);

        assert_eq!(point.fix_count(), 3);
        assert_eq!(point.total_satellites(), 5);
        assert_eq!(
            point
                .satellites
                .as_ref()
                .and_then(|report| report.gps_time()),
            Some(GpsTime::from_utc(at(7)))
        );
        assert_eq!(heading_degrees(&point), Some(90.0));
    }

    #[test]
    fn nav_points_from_steps_by_a_thousandth_of_a_degree_per_step() {
        let points = nav_points_from(at(0), 3, 5);

        let positions: Vec<(f64, f64)> = points
            .iter()
            .filter_map(|point| point.tpv.position())
            .map(|(lat, lon)| (lat.as_degrees(), lon.as_degrees()))
            .collect();
        assert_eq!(positions.len(), 3);
        for (index, (latitude, longitude)) in positions.into_iter().enumerate() {
            test_util::assert_degrees_close(latitude, 55.0 + index as f64 * 0.001);
            test_util::assert_degrees_close(longitude, 12.0 + index as f64 * 0.001);
        }
        let times: Vec<i64> = points
            .iter()
            .map(|point| point.tpv.time().utc().timestamp())
            .collect();
        assert_eq!(times, vec![0, 5, 10]);
    }

    #[test]
    fn nav_data_with_gap_puts_ten_minutes_between_the_two_groups() {
        let points = nav_data_with_gap(FixCountsAroundAGap {
            before: 2,
            after: 2,
        });

        let times: Vec<DateTime<Utc>> = points.iter().map(|point| point.tpv.time().utc()).collect();
        assert_eq!(
            times,
            vec![
                fixture_start(),
                fixture_start() + Duration::seconds(1),
                fixture_start() + Duration::seconds(2) + Duration::minutes(10),
                fixture_start() + Duration::seconds(3) + Duration::minutes(10),
            ]
        );
        let latitudes: Vec<f64> = points
            .iter()
            .filter_map(|point| point.tpv.position())
            .map(|(lat, _)| lat.as_degrees())
            .collect();
        assert_eq!(latitudes.len(), 4);
        for (index, latitude) in latitudes.into_iter().enumerate() {
            test_util::assert_degrees_close(latitude, 55.6867 + index as f64 * 0.0001);
        }
    }

    #[test]
    fn a_drifting_satellite_moves_from_its_first_to_its_last_epoch() {
        let drifts = [SatelliteDrift {
            constellation: Constellation::Gps,
            prn: 5,
            azimuth_deg: (40.0, 60.0),
            elevation_deg: (20.0, 40.0),
            in_fix: true,
            absent_at: &[],
        }];

        let points = nav_points_with_drifting_satellites(at(0), 3, &drifts);

        let sky: Vec<(Option<f32>, Option<f32>)> = points
            .iter()
            .filter_map(|point| point.satellites.as_ref())
            .filter_map(|report| report.satellites().next())
            .map(|satellite| (satellite.azimuth(), satellite.elevation()))
            .collect();
        assert_eq!(
            sky,
            vec![
                (Some(40.0), Some(20.0)),
                (Some(50.0), Some(30.0)),
                (Some(60.0), Some(40.0)),
            ]
        );
    }

    #[test]
    fn a_single_epoch_puts_a_drifting_satellite_at_the_start_of_its_drift() {
        let drifts = [SatelliteDrift {
            constellation: Constellation::Gps,
            prn: 5,
            azimuth_deg: (40.0, 60.0),
            elevation_deg: (20.0, 40.0),
            in_fix: true,
            absent_at: &[],
        }];

        let points = nav_points_with_drifting_satellites(at(0), 1, &drifts);

        let satellite = points
            .first()
            .and_then(|point| point.satellites.as_ref())
            .and_then(|report| report.satellites().next());
        assert_eq!(
            satellite.map(|satellite| (satellite.azimuth(), satellite.elevation())),
            Some((Some(40.0), Some(20.0)))
        );
    }

    #[test]
    fn a_drifting_satellite_is_missing_from_the_epochs_of_absent_at() {
        let drifts = [SatelliteDrift {
            constellation: Constellation::Gps,
            prn: 5,
            azimuth_deg: (40.0, 60.0),
            elevation_deg: (20.0, 40.0),
            in_fix: true,
            absent_at: &[1],
        }];

        let points = nav_points_with_drifting_satellites(at(0), 3, &drifts);

        let counts: Vec<u32> = points.iter().map(NavPoint::total_satellites).collect();
        assert_eq!(counts, vec![1, 0, 1]);
    }
}
