use crate::coordinates::{Latitude, Longitude};
use crate::mercator::{self, MercPoint};
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

/// One nav fix with its satellite report, carrying two positions that can
/// differ.
///
/// `tpv` holds the position the receiver recorded for this epoch. The
/// resolved position is where the point actually is, and is what every
/// renderer draws. The two are equal until the track builder resolves a ghost
/// fix, which it places by interpolating between the surrounding measured
/// fixes: a receiver that reports no heading often writes coordinates it did
/// not measure, down to (0, 0).
///
/// So read [`NavPoint::resolved_position`] (or [`NavPoint::merc`], its
/// projection) for anything geometric - a distance, a bounding box, a
/// bearing, a label placement. Read `tpv` only for what the receiver itself
/// reported at this epoch.
#[derive(Debug, Clone)]
pub struct NavPoint {
    pub tpv: TimePositionVelocity,
    pub satellites: Option<Satellites>,
    resolved_position: (Latitude, Longitude),
    merc: MercPoint,
}

impl NavPoint {
    /// `None` when the fix has no position to resolve to, which is when
    /// either of its recorded coordinates is out of range.
    pub fn new(tpv: TimePositionVelocity, satellites: Option<Satellites>) -> Option<Self> {
        let (latitude, longitude) = tpv.position()?;
        Some(Self {
            tpv,
            satellites,
            resolved_position: (latitude, longitude),
            merc: mercator::normalize(latitude, longitude),
        })
    }

    pub fn resolved_position(&self) -> (Latitude, Longitude) {
        self.resolved_position
    }

    /// The resolved position in normalized Web Mercator coordinates, see
    /// [`crate::mercator`]. Kept pre-computed because the renderers project
    /// every visible point each frame.
    pub fn merc(&self) -> MercPoint {
        self.merc
    }

    pub fn set_resolved_position(&mut self, (latitude, longitude): (Latitude, Longitude)) {
        self.resolved_position = (latitude, longitude);
        self.merc = mercator::normalize(latitude, longitude);
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
    use crate::coordinates::{Latitude, Longitude, RecordedLatitude, RecordedLongitude};
    use crate::satellites::{Constellation, Satellite, Satellites};
    use crate::time_types::GpsTime;
    use crate::tpv::TimePositionVelocity;
    use chrono::Utc;
    use uom::si::angle::degree;
    use uom::si::f64::Angle;

    fn point_at(latitude: Latitude, longitude: Longitude) -> NavPoint {
        let tpv = TimePositionVelocity::builder()
            .time(GpsTime::from_utc(Utc::now()))
            .lat(latitude)
            .lon(longitude)
            .build();
        NavPoint::new(tpv, None).expect("coordinates in range")
    }

    #[test]
    fn a_new_point_resolves_to_the_recorded_position() {
        let point = point_at(Latitude::new(55.0), Longitude::new(12.0));

        assert_eq!(
            point.resolved_position(),
            (Latitude::new(55.0), Longitude::new(12.0))
        );
        assert_eq!(
            point.merc(),
            mercator::normalize(Latitude::new(55.0), Longitude::new(12.0))
        );
    }

    #[test]
    fn resolving_a_point_elsewhere_reprojects_it_and_keeps_the_recorded_position() {
        let mut point = point_at(Latitude::new(55.0), Longitude::new(12.0));

        point.set_resolved_position((Latitude::new(-33.0), Longitude::new(151.0)));

        assert_eq!(
            point.resolved_position(),
            (Latitude::new(-33.0), Longitude::new(151.0))
        );
        assert_eq!(
            point.merc(),
            mercator::normalize(Latitude::new(-33.0), Longitude::new(151.0))
        );
        assert_eq!(
            point.tpv.position(),
            Some((Latitude::new(55.0), Longitude::new(12.0)))
        );
    }

    #[test]
    fn a_fix_with_a_coordinate_out_of_range_has_no_point_to_resolve_to() {
        let tpv = TimePositionVelocity::builder()
            .time(GpsTime::from_utc(Utc::now()))
            .lat(RecordedLatitude::from_degrees(91.0))
            .lon(RecordedLongitude::from_degrees(12.0))
            .build();

        assert!(NavPoint::new(tpv, None).is_none());
    }

    #[test]
    fn test_nav_point_fix_counts() {
        let tpv = TimePositionVelocity::builder()
            .time(GpsTime::from_utc(Utc::now()))
            .lat(Latitude::new(0.0))
            .lon(Longitude::new(0.0))
            .heading(Angle::new::<degree>(0.0))
            .build();

        let mut np = NavPoint::new(tpv, None).expect("coordinates in range");
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
