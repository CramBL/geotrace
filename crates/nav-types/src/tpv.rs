use chrono::{DateTime, Utc};
use geo_types::{Coord, Point};
use uom::si::angle::degree;
use uom::si::f64::{Angle, Velocity};

pub fn to_linestring(tpvs: &[TimePositionVelocity]) -> geo_types::LineString<f64> {
    tpvs.iter().map(Coord::from).collect()
}

/// A GPS fix: time, position, optional heading and speed.
///
/// `heading` is `None` for ghost/synthetic fixes that carry satellite reports
/// but have no known direction. The renderer draws those as circles.
#[derive(bon::Builder, Debug, Clone, Copy)]
pub struct TimePositionVelocity {
    pub(crate) time: DateTime<Utc>,
    pub(crate) lat: Angle,
    pub(crate) lon: Angle,
    pub(crate) velocity: Option<Velocity>,
    /// Compass heading in \[0°, 360°). `None` = direction unknown (ghost fix).
    pub(crate) heading: Option<Angle>,
}

impl TimePositionVelocity {
    pub fn time(&self) -> DateTime<Utc> {
        self.time
    }
    pub fn lat(&self) -> Angle {
        self.lat
    }
    pub fn lon(&self) -> Angle {
        self.lon
    }
    pub fn velocity(&self) -> Option<Velocity> {
        self.velocity
    }
    pub fn heading(&self) -> Option<Angle> {
        self.heading
    }
}

impl From<&TimePositionVelocity> for Coord<f64> {
    fn from(tpv: &TimePositionVelocity) -> Self {
        Coord {
            x: tpv.lon().get::<degree>(), // X is Longitude
            y: tpv.lat().get::<degree>(), // Y is Latitude
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
    use chrono::{NaiveDate, NaiveDateTime, NaiveTime};
    use uom::si::velocity::meter_per_second;

    #[test]
    fn test_builder_creates_valid_instance() {
        let dt = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 5, 21).unwrap(),
            NaiveTime::from_hms_opt(11, 5, 0).unwrap(),
        )
        .and_utc();

        let tpv = TimePositionVelocity::builder()
            .time(dt)
            .lat(Angle::new::<degree>(55.676))
            .lon(Angle::new::<degree>(12.565))
            .velocity(Velocity::new::<meter_per_second>(15.0))
            .heading(Angle::new::<degree>(270.0))
            .build();

        assert_eq!(tpv.time(), dt);
        assert_eq!(tpv.lat().get::<degree>(), 55.676);
        assert_eq!(tpv.heading().map(|h| h.get::<degree>()), Some(270.0));
    }

    #[test]
    fn test_builder_heading_none_when_omitted() {
        let dt = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 5, 21).unwrap(),
            NaiveTime::from_hms_opt(11, 5, 0).unwrap(),
        )
        .and_utc();

        let tpv = TimePositionVelocity::builder()
            .time(dt)
            .lat(Angle::new::<degree>(55.0))
            .lon(Angle::new::<degree>(12.0))
            .build();

        assert_eq!(tpv.heading(), None);
    }
}
