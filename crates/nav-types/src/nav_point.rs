use crate::satellites::Satellites;
use crate::tpv::TimePositionVelocity;

#[derive(Debug, Clone)]
pub struct NavPoint {
    pub tpv: TimePositionVelocity,
    pub satellites: Option<Satellites>,
}

impl NavPoint {
    pub fn new(tpv: TimePositionVelocity, satellites: Option<Satellites>) -> Self {
        Self { tpv, satellites }
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
    use crate::satellites::{Constellation, Satellite, Satellites};
    use crate::tpv::TimePositionVelocity;
    use chrono::Utc;
    use uom::si::angle::degree;
    use uom::si::f64::Angle;

    #[test]
    fn test_nav_point_fix_counts() {
        let tpv = TimePositionVelocity::build()
            .with_time(Utc::now())
            .with_lat(Angle::new::<degree>(0.0))
            .with_lon(Angle::new::<degree>(0.0))
            .with_heading(Angle::new::<degree>(0.0))
            .build();

        let mut np = NavPoint::new(tpv, None);
        assert_eq!(np.fix_count(), 0);
        assert_eq!(np.total_satellites(), 0);

        let sats = Satellites::new(
            Utc::now(),
            vec![
                Satellite::new(Constellation::Gps, 1, None, None, None),
                Satellite::new(Constellation::Gps, 2, None, None, None),
            ],
            vec![(Some(Constellation::Gps), 1)],
        );

        np.satellites = Some(sats);
        assert_eq!(np.fix_count(), 1);
        assert_eq!(np.total_satellites(), 2);
    }
}
