use crate::satellites::Satellites;
use crate::tpv::TimePositionVelocity;

#[derive(Debug, Clone)]
pub struct NavPoint {
    pub tpv: TimePositionVelocity,
    pub satellites: Option<Satellites>,
    /// Normalized Web Mercator X coordinate in `[0, 1]`, pre-computed from
    /// `tpv.lon()` at construction time so the renderer only needs an affine
    /// transform per frame instead of a full trigonometric projection.
    pub merc_x: f64,
    /// Normalized Web Mercator Y coordinate in `[0, 1]`, pre-computed from
    /// `tpv.lat()` at construction time.
    pub merc_y: f64,
}

impl NavPoint {
    pub fn new(tpv: TimePositionVelocity, satellites: Option<Satellites>) -> Self {
        let (merc_x, merc_y) =
            crate::mercator::normalize(tpv.lon().as_degrees(), tpv.lat().as_degrees());
        Self {
            tpv,
            satellites,
            merc_x,
            merc_y,
        }
    }

    pub fn fix_count(&self) -> u32 {
        self.satellites.as_ref().map_or(0, |s| s.fix_count())
    }

    pub fn total_satellites(&self) -> u32 {
        self.satellites.as_ref().map_or(0, |s| s.satellite_count())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::coordinates::{Latitude, Longitude};
    use crate::satellites::{Constellation, Satellite, Satellites};
    use crate::time_types::GpsTime;
    use crate::tpv::TimePositionVelocity;
    use chrono::Utc;
    use uom::si::angle::degree;
    use uom::si::f64::Angle;

    #[test]
    fn test_nav_point_fix_counts() {
        let tpv = TimePositionVelocity::builder()
            .time(GpsTime::from_utc(Utc::now()))
            .lat(Latitude::new(0.0))
            .lon(Longitude::new(0.0))
            .heading(Angle::new::<degree>(0.0))
            .build();

        let mut np = NavPoint::new(tpv, None);
        assert_eq!(np.fix_count(), 0);
        assert_eq!(np.total_satellites(), 0);

        let sats = Satellites::new(
            Some(GpsTime::from_utc(Utc::now())),
            None,
            vec![
                Satellite::new(Constellation::Gps, 1, None, None, None, true),
                Satellite::new(Constellation::Gps, 2, None, None, None, false),
            ],
        );

        np.satellites = Some(sats);
        assert_eq!(np.fix_count(), 1);
        assert_eq!(np.total_satellites(), 2);
    }
}
