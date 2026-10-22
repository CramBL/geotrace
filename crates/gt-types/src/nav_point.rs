use crate::coordinates::{Latitude, Longitude};
use crate::mercator::{self, MercPoint};
use crate::navigation::{NavigationMetadata, PositionProvenance, SolutionState};
use crate::satellite_evidence::SatelliteEvidence;
use crate::satellites::Satellites;
use crate::tpv::TimePositionVelocity;

/// Quality classification for legacy reports and resolved navigation evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FixQuality {
    /// Resolved NoFix state or a legacy report with zero used satellites.
    Lost,
    /// 1-9 satellites in fix.
    Marginal,
    /// 10 or more satellites in fix.
    Strong,
    Unknown,
}

impl FixQuality {
    pub(crate) const STRONG_USED_COUNT: u32 = 10;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MapFixKind {
    CoordinateOutOfRange,
    Estimated,
    NavigationLoss,
    Recorded,
    Synthesized,
    UnreportedPosition,
}

/// Coordinates with their Web Mercator projection, held together so the two
/// can never disagree. The projection is kept because the renderers project
/// every visible point each frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ProjectedPosition {
    latitude: Latitude,
    longitude: Longitude,
    merc: MercPoint,
}

impl ProjectedPosition {
    pub fn new(latitude: Latitude, longitude: Longitude) -> Self {
        Self {
            latitude,
            longitude,
            merc: mercator::normalize(latitude, longitude),
        }
    }

    pub fn coordinates(self) -> (Latitude, Longitude) {
        (self.latitude, self.longitude)
    }

    pub fn merc(self) -> MercPoint {
        self.merc
    }
}

/// Where the track builder placed a fix or an event marker, and how it got
/// there.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ResolvedPosition {
    /// A position the track builder derived, in time between the fixes around
    /// it: a fix the receiver dead-reckoned, one whose recorded coordinates
    /// are out of range, or an event marker between two fixes when the builder
    /// placed either of them away from its recorded coordinates.
    Interpolated(ProjectedPosition),
    /// The coordinates the recording holds, both inside their axis' range.
    Measured(ProjectedPosition),
}

impl ResolvedPosition {
    pub fn measured(latitude: Latitude, longitude: Longitude) -> Self {
        Self::Measured(ProjectedPosition::new(latitude, longitude))
    }

    pub fn interpolated(latitude: Latitude, longitude: Longitude) -> Self {
        Self::Interpolated(ProjectedPosition::new(latitude, longitude))
    }

    pub fn projected(self) -> ProjectedPosition {
        match self {
            Self::Measured(projected) | Self::Interpolated(projected) => projected,
        }
    }

    pub fn coordinates(self) -> (Latitude, Longitude) {
        self.projected().coordinates()
    }

    pub fn merc(self) -> MercPoint {
        self.projected().merc()
    }
}

/// Recorded coordinates and navigation facts, with independent satellite samples.
/// Geometry uses [`crate::track::LoadedTrack::placed_points`].
#[derive(Debug, Clone)]
pub struct NavPoint {
    pub tpv: TimePositionVelocity,
    pub satellites: Option<Satellites>,
    pub navigation: NavigationMetadata,
    pub satellite_evidence: SatelliteEvidence,
}

impl NavPoint {
    pub fn new(tpv: TimePositionVelocity, satellites: Option<Satellites>) -> Self {
        let satellite_evidence = satellites
            .as_ref()
            .map(|report| SatelliteEvidence::from_legacy(report, &tpv))
            .unwrap_or_default();
        Self {
            tpv,
            satellites,
            navigation: NavigationMetadata::default(),
            satellite_evidence,
        }
    }

    pub fn fix_count(&self) -> u32 {
        self.satellites.as_ref().map_or(0, |s| s.fix_count())
    }

    pub fn total_satellites(&self) -> u32 {
        self.satellites.as_ref().map_or(0, |s| s.satellite_count())
    }

    /// Quality from the legacy combined report.
    pub fn fix_quality(&self) -> FixQuality {
        match &self.satellites {
            None => FixQuality::Unknown,
            Some(sats) => match sats.fix_count() {
                n if n >= FixQuality::STRONG_USED_COUNT => FixQuality::Strong,
                n if n > 0 => FixQuality::Marginal,
                _ => FixQuality::Lost,
            },
        }
    }

    /// Two NaN coordinates represent an unreported position when an explicit state is present.
    pub fn has_unreported_position(&self) -> bool {
        self.tpv.lat().as_written().is_nan()
            && self.tpv.lon().as_written().is_nan()
            && self.navigation.state.is_some()
    }

    pub fn map_fix_kind(&self) -> MapFixKind {
        if self.tpv.position().is_none() && !self.has_unreported_position() {
            MapFixKind::CoordinateOutOfRange
        } else if self.navigation.position_provenance == Some(PositionProvenance::SdkSynthesized) {
            MapFixKind::Synthesized
        } else {
            match self.resolved_navigation_state().state {
                SolutionState::NoFix => MapFixKind::NavigationLoss,
                SolutionState::EstimatedOnly => MapFixKind::Estimated,
                SolutionState::Fix | SolutionState::Unknown if self.has_unreported_position() => {
                    MapFixKind::UnreportedPosition
                }
                SolutionState::Fix | SolutionState::Unknown => MapFixKind::Recorded,
            }
        }
    }

    pub fn is_navigation_estimate(&self) -> bool {
        matches!(
            self.map_fix_kind(),
            MapFixKind::NavigationLoss
                | MapFixKind::Estimated
                | MapFixKind::Synthesized
                | MapFixKind::UnreportedPosition
        )
    }

    pub fn render_class(&self) -> (MapFixKind, FixQuality) {
        (self.map_fix_kind(), self.resolved_fix_quality())
    }
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Utc};
    use rstest::rstest;
    use uom::si::angle::degree;
    use uom::si::f64::Angle;

    use super::*;
    use crate::coordinates::{Latitude, Longitude, RecordedLatitude, RecordedLongitude};
    use crate::satellites::{Constellation, Satellite, Satellites};
    use crate::time_types::GpsTime;
    use crate::tpv::TimePositionVelocity;

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

    #[rstest]
    #[case::fix(Some(SolutionState::Fix), f64::NAN, f64::NAN, true)]
    #[case::loss(Some(SolutionState::NoFix), f64::NAN, f64::NAN, true)]
    #[case::estimated(Some(SolutionState::EstimatedOnly), f64::NAN, f64::NAN, true)]
    #[case::unknown(Some(SolutionState::Unknown), f64::NAN, f64::NAN, true)]
    #[case::unclassified(None, f64::NAN, f64::NAN, false)]
    #[case::missing_latitude(Some(SolutionState::NoFix), f64::NAN, 12.0, false)]
    #[case::missing_longitude(Some(SolutionState::NoFix), 55.0, f64::NAN, false)]
    #[case::out_of_range(Some(SolutionState::NoFix), 91.0, 181.0, false)]
    #[case::positioned(Some(SolutionState::NoFix), 55.0, 12.0, false)]
    fn an_unreported_position_requires_two_nan_coordinates_and_an_explicit_state(
        #[case] state: Option<SolutionState>,
        #[case] latitude: f64,
        #[case] longitude: f64,
        #[case] expected: bool,
    ) {
        let tpv = TimePositionVelocity::builder()
            .time(GpsTime::from_utc(DateTime::UNIX_EPOCH))
            .lat(RecordedLatitude::from_degrees(latitude))
            .lon(RecordedLongitude::from_degrees(longitude))
            .build();
        let mut point = NavPoint::new(tpv, None);
        point.navigation.state = state;
        assert_eq!(point.has_unreported_position(), expected);
    }
}
