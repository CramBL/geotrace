//! The GNSS constellation identifier.

/// GNSS constellation identifier.
/// cbindgen:rename-all=QualifiedScreamingSnakeCase
#[repr(C)]
#[derive(Clone, Copy)]
pub enum GtdConstellation {
    /// GPS (USA).
    Gps = 0,
    /// GLONASS (Russia).
    Glonass = 1,
    /// Galileo (EU).
    Galileo = 2,
    /// BeiDou (China).
    Beidou = 3,
    /// NavIC / IRNSS (India).
    Navic = 4,
    /// QZSS (Japan).
    Qzss = 5,
}

impl From<GtdConstellation> for geotrace_sdk::Constellation {
    fn from(c: GtdConstellation) -> Self {
        match c {
            GtdConstellation::Gps => geotrace_sdk::Constellation::Gps,
            GtdConstellation::Glonass => geotrace_sdk::Constellation::Glonass,
            GtdConstellation::Galileo => geotrace_sdk::Constellation::Galileo,
            GtdConstellation::Beidou => geotrace_sdk::Constellation::Beidou,
            GtdConstellation::Navic => geotrace_sdk::Constellation::Navic,
            GtdConstellation::Qzss => geotrace_sdk::Constellation::Qzss,
        }
    }
}

impl From<geotrace_sdk::Constellation> for GtdConstellation {
    fn from(c: geotrace_sdk::Constellation) -> Self {
        match c {
            geotrace_sdk::Constellation::Gps => GtdConstellation::Gps,
            geotrace_sdk::Constellation::Glonass => GtdConstellation::Glonass,
            geotrace_sdk::Constellation::Galileo => GtdConstellation::Galileo,
            geotrace_sdk::Constellation::Beidou => GtdConstellation::Beidou,
            geotrace_sdk::Constellation::Navic => GtdConstellation::Navic,
            geotrace_sdk::Constellation::Qzss => GtdConstellation::Qzss,
        }
    }
}
