//! The `constellation` group of `geotrace.h`: the GNSS constellation identifier.

/// GNSS constellation identifier.
#[repr(C)]
#[derive(Clone, Copy)]
pub enum GtdConstellation {
    Gps = 0,
    Glonass = 1,
    Galileo = 2,
    Beidou = 3,
    Navic = 4,
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
