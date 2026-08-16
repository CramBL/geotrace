use crate::mercator::MercPoint;
use crate::satellites::Satellites;
use crate::tpv::TimePositionVelocity;

/// Fix-quality tier of a nav point, derived from its satellite report.
///
/// This is the canonical classification shared by the map renderers (icon
/// and quality-line colors) and the track LOD builder (decimation must keep
/// every point where the tier changes, so a quality transition can never be
/// erased by downsampling).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixQuality {
    /// No satellite report attached - quality unknown, assume fine.
    Unknown,
    /// 10 or more satellites in fix.
    Strong,
    /// 1-9 satellites in fix.
    Marginal,
    /// Satellite report present but zero satellites in fix.
    Lost,
}

#[derive(Debug, Clone)]
pub struct NavPoint {
    pub tpv: TimePositionVelocity,
    pub satellites: Option<Satellites>,
    /// Pre-computed normalized Web Mercator coordinates, see [`crate::mercator`].
    pub merc: MercPoint,
}

impl NavPoint {
    pub fn new(tpv: TimePositionVelocity, satellites: Option<Satellites>) -> Self {
        let merc = crate::mercator::normalize(tpv.lat(), tpv.lon());
        Self {
            tpv,
            satellites,
            merc,
        }
    }

    pub fn fix_count(&self) -> u32 {
        self.satellites.as_ref().map_or(0, |s| s.fix_count())
    }

    pub fn total_satellites(&self) -> u32 {
        self.satellites.as_ref().map_or(0, |s| s.satellite_count())
    }

    pub fn fix_quality(&self) -> FixQuality {
        match &self.satellites {
            None => FixQuality::Unknown,
            Some(sats) => match sats.fix_count() {
                n if n >= 10 => FixQuality::Strong,
                n if n > 0 => FixQuality::Marginal,
                _ => FixQuality::Lost,
            },
        }
    }

    /// Returns `true` when the point should be rendered as a ghost (hollow
    /// chevron, dashed track edge).
    ///
    /// Two cases qualify:
    /// - No heading from the GPS receiver (position only, direction
    ///   entirely unknown).
    /// - Satellite fix count dropped to zero: the GPS may still output
    ///   position and heading estimates, but those are internal
    ///   dead-reckoning guesses, not real fixes.
    pub fn is_ghost_fix(&self) -> bool {
        self.tpv.heading().is_none() || self.fix_quality() == FixQuality::Lost
    }

    /// Everything about this point that affects how the map styles it:
    /// the trackline's ghost flag and the fix-quality tier. The LOD builder
    /// always keeps points where this changes.
    pub fn render_class(&self) -> (bool, FixQuality) {
        (self.tpv.heading().is_none(), self.fix_quality())
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
