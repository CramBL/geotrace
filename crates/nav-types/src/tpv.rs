use chrono::{DateTime, Utc};
use geo_types::{Coord, Point};
use uom::si::angle::degree;
use uom::si::f64::{Angle, Velocity};

pub fn to_linestring(tpvs: &[TimePositionVelocity]) -> geo_types::LineString<f64> {
    tpvs.iter().map(Coord::from).collect()
}

#[derive(Default, Clone, Copy)]
pub struct TimePositionVelocityBuilder {
    time: Option<DateTime<Utc>>,
    lat: Option<Angle>,
    lon: Option<Angle>,
    velocity: Option<Velocity>,
    heading: Option<Angle>,
}

impl TimePositionVelocityBuilder {
    pub fn with_time(mut self, time: DateTime<Utc>) -> Self {
        self.time = Some(time);
        self
    }
    pub fn with_lat(mut self, lat: Angle) -> Self {
        self.lat = Some(lat);
        self
    }
    pub fn with_lon(mut self, lon: Angle) -> Self {
        self.lon = Some(lon);
        self
    }
    pub fn with_velocity(mut self, velocity: Velocity) -> Self {
        self.velocity = Some(velocity);
        self
    }
    pub fn with_heading(mut self, heading: Angle) -> Self {
        self.heading = Some(heading);
        self
    }

    #[expect(clippy::expect_used, reason = "Builder pattern invariants")]
    pub fn build(self) -> TimePositionVelocity {
        let Self {
            time,
            lat,
            lon,
            velocity,
            heading,
        } = self;
        TimePositionVelocity {
            time: time.expect("Time is required"),
            lat: lat.expect("Latitude is required"),
            lon: lon.expect("Longitude is required"),
            velocity,
            heading: heading.expect("Heading is required"),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct TimePositionVelocity {
    time: DateTime<Utc>,
    lat: Angle,
    lon: Angle,
    velocity: Option<Velocity>,
    heading: Angle,
}

impl TimePositionVelocity {
    pub fn build() -> TimePositionVelocityBuilder {
        TimePositionVelocityBuilder::default()
    }
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
    pub fn heading(&self) -> Angle {
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

        let tpv = TimePositionVelocity::build()
            .with_time(dt)
            .with_lat(Angle::new::<degree>(55.676))
            .with_lon(Angle::new::<degree>(12.565))
            .with_velocity(Velocity::new::<meter_per_second>(15.0))
            .with_heading(Angle::new::<degree>(270.0))
            .build();

        assert_eq!(tpv.time(), dt);
        assert_eq!(tpv.lat().get::<degree>(), 55.676);
        assert_eq!(tpv.heading().get::<degree>(), 270.0);
    }

    #[test]
    #[should_panic(expected = "Latitude is required")]
    fn test_builder_panics_on_missing_required_field() {
        let dt = NaiveDateTime::new(
            NaiveDate::from_ymd_opt(2026, 5, 21).unwrap(),
            NaiveTime::from_hms_opt(11, 5, 0).unwrap(),
        )
        .and_utc();

        TimePositionVelocity::build()
            .with_time(dt)
            .with_lon(Angle::new::<degree>(12.565))
            .with_heading(Angle::new::<degree>(270.0))
            .build();
    }
}
