use gt_query::{CheckedQuery, QueryMetric};

use crate::check::QueryChunk;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct RunDependencies {
    snap_error: bool,
    jamming: bool,
    pub(crate) hp30: bool,
    pub(crate) kp: bool,
    tec: bool,
    pub(crate) util: bool,
    pub(crate) slip: bool,
}

impl RunDependencies {
    pub(crate) fn of(chunks: &[QueryChunk]) -> Self {
        let mut dependencies = Self::default();
        let metrics = chunks
            .iter()
            .filter_map(|chunk| chunk.result.as_ref().ok())
            .flat_map(CheckedQuery::referenced_metrics);
        for &metric in metrics {
            match metric {
                QueryMetric::SnapError => dependencies.snap_error = true,
                QueryMetric::Jamming => dependencies.jamming = true,
                QueryMetric::Hp30 => dependencies.hp30 = true,
                QueryMetric::Kp => dependencies.kp = true,
                QueryMetric::Tec => dependencies.tec = true,
                QueryMetric::UtilAll
                | QueryMetric::UtilGps
                | QueryMetric::UtilGlonass
                | QueryMetric::UtilGalileo
                | QueryMetric::UtilBeidou
                | QueryMetric::UtilNavic
                | QueryMetric::UtilQzss => dependencies.util = true,
                QueryMetric::SlipAll
                | QueryMetric::SlipGps
                | QueryMetric::SlipGlonass
                | QueryMetric::SlipGalileo
                | QueryMetric::SlipBeidou
                | QueryMetric::SlipNavic
                | QueryMetric::SlipQzss => dependencies.slip = true,
                QueryMetric::Time
                | QueryMetric::SysTime
                | QueryMetric::Lat
                | QueryMetric::Lon
                | QueryMetric::InvalidCoordinates
                | QueryMetric::Velocity
                | QueryMetric::Heading
                | QueryMetric::Accel
                | QueryMetric::Eph
                | QueryMetric::Hdop
                | QueryMetric::Vdop
                | QueryMetric::Pdop
                | QueryMetric::ClockDelta
                | QueryMetric::SystemBootDelta
                | QueryMetric::SatsSeen
                | QueryMetric::SatsFix
                | QueryMetric::GpsSeen
                | QueryMetric::GpsFix
                | QueryMetric::GlonassSeen
                | QueryMetric::GlonassFix
                | QueryMetric::GalileoSeen
                | QueryMetric::GalileoFix
                | QueryMetric::BeidouSeen
                | QueryMetric::BeidouFix
                | QueryMetric::NavicSeen
                | QueryMetric::NavicFix
                | QueryMetric::QzssSeen
                | QueryMetric::QzssFix => {}
            }
        }
        dependencies
    }

    pub fn uses_snap_error(self) -> bool {
        self.snap_error
    }

    pub fn uses_jamming(self) -> bool {
        self.jamming
    }

    pub fn uses_geomagnetic(self) -> bool {
        self.hp30 || self.kp
    }

    pub fn uses_tec(self) -> bool {
        self.tec
    }

    pub(crate) fn external_values(self) -> bool {
        self.uses_snap_error() || self.uses_jamming() || self.uses_geomagnetic() || self.uses_tec()
    }
}
