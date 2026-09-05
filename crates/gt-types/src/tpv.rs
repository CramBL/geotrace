use crate::coordinates::{Latitude, Longitude, RecordedLatitude, RecordedLongitude};
use crate::time_types::{FixTimestamp, GpsTime, SysTime};
use chrono::Duration;
use geo_types::{Coord, Point};
use uom::si::f64::{Angle, Velocity};
use uom::si::velocity::kilometer_per_hour;

pub fn to_linestring(tpvs: &[TimePositionVelocity]) -> geo_types::LineString<f64> {
    tpvs.iter().map(Coord::from).collect()
}

/// A GPS fix: time, position, optional heading and speed.
///
/// `heading` is `None` for ghost/synthetic fixes that carry satellite reports
/// but have no known direction. The renderer draws those as circles.
///
/// `sys_time` is the host system-clock timestamp at the moment of the fix, when
/// available. Use [`TimePositionVelocity::gps_system_clock_offset`] to compute
/// the GPS/system-clock offset - direct arithmetic between the two fields is a
/// compile-time error.
#[derive(bon::Builder, Debug, Clone, Copy)]
pub struct TimePositionVelocity {
    /// Pass a [`GpsTime`] to record that the receiver's own clock stamped this
    /// fix.
    #[builder(into)]
    pub(crate) time: FixTimestamp,
    /// Pass a [`Latitude`] for a coordinate known to be in range.
    #[builder(into)]
    pub(crate) lat: RecordedLatitude,
    /// Pass a [`Longitude`] for a coordinate known to be in range.
    #[builder(into)]
    pub(crate) lon: RecordedLongitude,
    pub(crate) velocity: Option<Velocity>,
    /// Compass heading in \[0°, 360°). `None` = direction unknown (ghost fix).
    pub(crate) heading: Option<Angle>,
    /// Host system-clock timestamp at the time of the fix.
    /// `None` when the host did not record a system timestamp.
    pub(crate) sys_time: Option<SysTime>,
    pub(crate) eph_m: Option<f32>,
}

impl TimePositionVelocity {
    /// Where the fix sits on the recording's time axis: the receiver's own
    /// timestamp when it had a lock, the host timestamp the fix was stamped
    /// with when it did not.
    ///
    /// Read [`Self::gps_time`] to tell those apart.
    pub fn time(&self) -> GpsTime {
        match self.time {
            FixTimestamp::FromGpsReceiver(gps) => gps,
            FixTimestamp::FromHostClock(host) => GpsTime::from_utc(host.utc()),
        }
    }

    /// The receiver's own timestamp for this fix, `None` when it had no lock.
    pub fn gps_time(&self) -> Option<GpsTime> {
        match self.time {
            FixTimestamp::FromGpsReceiver(gps) => Some(gps),
            FixTimestamp::FromHostClock(_) => None,
        }
    }

    /// GPS−system clock offset at this fix, positive when the GPS clock is
    /// ahead of the host clock.
    ///
    /// `None` unless both clocks stamped the fix.
    pub fn gps_system_clock_offset(&self) -> Option<Duration> {
        Some(self.gps_time()?.offset_from_sys(self.sys_time?))
    }
    pub fn lat(&self) -> RecordedLatitude {
        self.lat
    }
    pub fn lon(&self) -> RecordedLongitude {
        self.lon
    }

    /// Where the receiver placed this fix, `None` unless both of its
    /// coordinates are in range.
    pub fn position(&self) -> Option<(Latitude, Longitude)> {
        Some((self.lat.valid()?, self.lon.valid()?))
    }

    /// How many of the fix's two coordinate axes the receiver wrote outside
    /// their range, NaN counting as outside.
    pub fn invalid_coordinate_count(&self) -> u32 {
        u32::from(self.lat.valid().is_none()) + u32::from(self.lon.valid().is_none())
    }

    pub fn velocity(&self) -> Option<Velocity> {
        self.velocity
    }

    pub fn velocity_kmh(&self) -> Option<f64> {
        self.velocity.map(|v| v.get::<kilometer_per_hour>())
    }
    pub fn heading(&self) -> Option<Angle> {
        self.heading
    }
    /// Host system-clock timestamp, if recorded alongside the GPS fix.
    ///
    /// Use [`Self::gps_system_clock_offset`] to compute the GPS/system-clock
    /// offset.
    pub fn sys_time(&self) -> Option<SysTime> {
        self.sys_time
    }

    /// Estimated horizontal position accuracy in metres, as reported by the GPS receiver.
    ///
    /// `None` when the receiver did not report an accuracy estimate.
    pub fn eph_m(&self) -> Option<f32> {
        self.eph_m
    }
}

impl From<&TimePositionVelocity> for Coord<f64> {
    fn from(tpv: &TimePositionVelocity) -> Self {
        Coord {
            x: tpv.lon().as_written(),
            y: tpv.lat().as_written(),
        }
    }
}

impl From<&TimePositionVelocity> for Point<f64> {
    fn from(tpv: &TimePositionVelocity) -> Self {
        Point::from(Coord::from(tpv))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime};
    use rstest::rstest;
    use uom::si::velocity::meter_per_second;

    #[test]
    #[expect(
        clippy::float_cmp,
        reason = "testing exact bit-for-bit round-trip of stored f64"
    )]
    fn test_builder_creates_valid_instance() {
        let dt = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 5, 21).unwrap(),
            NaiveTime::from_hms_opt(11, 5, 0).unwrap(),
        )
        .and_utc();

        let tpv = TimePositionVelocity::builder()
            .time(GpsTime::from_utc(dt))
            .lat(Latitude::new(55.676))
            .lon(Longitude::new(12.565))
            .velocity(Velocity::new::<meter_per_second>(15.0))
            .heading(Angle::new::<uom::si::angle::degree>(270.0))
            .build();

        assert_eq!(tpv.time().utc(), dt);
        assert_eq!(tpv.lat().as_written(), 55.676);
        assert_eq!(
            tpv.heading().map(|h| h.get::<uom::si::angle::degree>()),
            Some(270.0)
        );
    }

    #[test]
    fn test_builder_heading_none_when_omitted() {
        let dt = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 5, 21).unwrap(),
            NaiveTime::from_hms_opt(11, 5, 0).unwrap(),
        )
        .and_utc();

        let tpv = TimePositionVelocity::builder()
            .time(GpsTime::from_utc(dt))
            .lat(Latitude::new(55.0))
            .lon(Longitude::new(12.0))
            .build();

        assert_eq!(tpv.heading(), None);
    }

    #[rstest]
    #[case::both_in_range(55.0, 12.0, true)]
    #[case::latitude_past_the_pole(91.0, 12.0, false)]
    #[case::longitude_past_the_antimeridian(55.0, -181.0, false)]
    #[case::neither_in_range(f64::NAN, f64::NAN, false)]
    fn a_fix_has_a_position_only_when_both_its_coordinates_are_in_range(
        #[case] lat_degrees: f64,
        #[case] lon_degrees: f64,
        #[case] expected_position: bool,
    ) {
        let tpv = TimePositionVelocity::builder()
            .time(GpsTime::from_utc(DateTime::UNIX_EPOCH))
            .lat(RecordedLatitude::from_degrees(lat_degrees))
            .lon(RecordedLongitude::from_degrees(lon_degrees))
            .build();

        assert_eq!(tpv.position().is_some(), expected_position);
    }

    fn fix(time: FixTimestamp, sys_time: Option<SysTime>) -> TimePositionVelocity {
        TimePositionVelocity::builder()
            .time(time)
            .lat(Latitude::new(55.0))
            .lon(Longitude::new(12.0))
            .maybe_sys_time(sys_time)
            .build()
    }

    /// No offset can be measured from a fix the host clock stamped on its own,
    /// however far the two clocks stand apart elsewhere in the recording.
    #[test]
    fn only_a_fix_the_receiver_stamped_has_a_gps_system_clock_offset() {
        const GPS_AHEAD_MS: i64 = 600;
        let stamped_at = DateTime::from_timestamp(1_700_000_000, 0).unwrap();
        let host = SysTime::from_utc(stamped_at);
        let gps = GpsTime::from_utc(stamped_at + Duration::milliseconds(GPS_AHEAD_MS));

        assert_eq!(
            fix(gps.into(), Some(host))
                .gps_system_clock_offset()
                .map(|offset| offset.num_milliseconds()),
            Some(GPS_AHEAD_MS)
        );
        assert_eq!(
            fix(FixTimestamp::FromHostClock(host), Some(host)).gps_system_clock_offset(),
            None
        );
        assert_eq!(fix(gps.into(), None).gps_system_clock_offset(), None);
    }
}
