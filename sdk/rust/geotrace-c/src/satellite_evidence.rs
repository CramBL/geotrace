use geotrace_sdk::{
    ConstellationSet, NavFixTime, SatelliteCoverage, SatelliteIdentity, SatelliteMembership,
    SatelliteObservation, SatelliteReportPlacement, SatelliteScope, SatelliteUsage,
};

use crate::error::GtdStatus;
use crate::{GtdConstellation, GtdOptF32, GtdOptU32, GtdTimestamp};
use crate::{navigation, optf32, timestamp};

/// Membership is independent of satellite observations and navigation state.
/// cbindgen:rename-all=QualifiedScreamingSnakeCase
#[repr(C)]
#[derive(Clone, Copy)]
pub enum GtdSatelliteMembership {
    /// The source did not establish membership.
    Unknown = 0,
    /// The source reported that this identity was unused.
    Unused = 1,
    /// The source reported that this identity contributed to the solution.
    Used = 2,
}

/// Coverage applies to the identity list, independently of the reported aggregate.
/// cbindgen:rename-all=QualifiedScreamingSnakeCase
#[repr(C)]
#[derive(Clone, Copy)]
pub enum GtdSatelliteCoverage {
    /// Complete list within scope. Omitted usage identities are known-unused.
    /// Explicit unknown membership remains unknown.
    Complete = 0,
    /// Omitted identities have unreported evidence.
    Partial = 1,
}

/// Global scope remains distinct from an explicit set of every supported constellation.
/// cbindgen:rename-all=QualifiedScreamingSnakeCase
#[repr(C)]
#[derive(Clone, Copy)]
pub enum GtdSatelliteScopeKind {
    /// Explicit nonempty set of constellations.
    Constellations = 1,
    /// All constellations, including identities omitted from complete lists.
    Global = 0,
}

/// Global requires a zero mask. Explicit scope requires a nonzero mask.
/// Bit `1 << GtdConstellation` selects that constellation. Unsupported bits are invalid.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdSatelliteScope {
    /// A `GtdSatelliteScopeKind` tag.
    pub kind: u32,
    /// Selected constellation bits, or zero for global scope.
    pub constellation_mask: u32,
}

impl GtdSatelliteScope {
    pub(crate) fn to_sdk(self) -> Result<SatelliteScope, GtdStatus> {
        match self.kind {
            kind if kind == GtdSatelliteScopeKind::Global as u32
                && self.constellation_mask == 0 =>
            {
                Ok(SatelliteScope::Global)
            }
            kind if kind == GtdSatelliteScopeKind::Constellations as u32 => {
                let mask = self.constellation_mask;
                let mut constellations = Vec::new();
                for bit in 0..u32::BITS {
                    let flag = 1_u32 << bit;
                    if mask & flag != 0 {
                        let constellation = GtdConstellation::from_abi_value(bit)
                            .ok_or_else(|| navigation::invalid_field("scope.constellation_mask"))?;
                        constellations.push(constellation.into());
                    }
                }
                let mut constellations = constellations.into_iter();
                let first = constellations
                    .next()
                    .ok_or_else(|| navigation::invalid_field("scope.constellation_mask"))?;
                Ok(SatelliteScope::Constellations(ConstellationSet::new(
                    first,
                    constellations,
                )))
            }
            _ => Err(navigation::invalid_field(
                "scope.kind or scope.constellation_mask",
            )),
        }
    }
}

impl TryFrom<&SatelliteScope> for GtdSatelliteScope {
    type Error = GtdStatus;
    fn try_from(scope: &SatelliteScope) -> Result<Self, Self::Error> {
        match scope {
            SatelliteScope::Global => Ok(Self {
                kind: GtdSatelliteScopeKind::Global as u32,
                constellation_mask: 0,
            }),
            SatelliteScope::Constellations(set) => Ok(Self {
                kind: GtdSatelliteScopeKind::Constellations as u32,
                constellation_mask: set.as_slice().iter().fold(0, |mask, constellation| {
                    mask | (1 << GtdConstellation::from(*constellation) as u32)
                }),
            }),
            _ => Err(navigation::invalid_field("scope")),
        }
    }
}

/// Shared input metadata. Original clocks remain independent of recorder placement.
/// Coverage uses `GtdSatelliteCoverage` constants. Null rows with zero length are valid.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdSatelliteReportInput {
    /// Original receiver timestamp, or `gtd_ts_none()` when unreported.
    pub gps_time: GtdTimestamp,
    /// Original host timestamp, or `gtd_ts_none()` when unreported.
    pub sys_time: GtdTimestamp,
    /// Scope of the aggregate and identity coverage.
    pub scope: GtdSatelliteScope,
    /// A `GtdSatelliteCoverage` tag for the identity list.
    pub coverage: u32,
    /// Exact for the declared scope, independently of list coverage and row count.
    pub reported_count: GtdOptU32,
}

impl GtdSatelliteReportInput {
    pub(crate) fn coverage(self) -> Result<SatelliteCoverage, GtdStatus> {
        match self.coverage {
            value if value == GtdSatelliteCoverage::Complete as u32 => {
                Ok(SatelliteCoverage::Complete)
            }
            value if value == GtdSatelliteCoverage::Partial as u32 => {
                Ok(SatelliteCoverage::Partial)
            }
            _ => Err(navigation::invalid_field("coverage")),
        }
    }
}

/// Raw constellation and membership tags are checked before recording any rows.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdSatelliteUsage {
    /// A `GtdConstellation` tag.
    pub constellation: u32,
    /// Satellite identifier within the constellation.
    pub prn: u32,
    /// A `GtdSatelliteMembership` tag.
    pub membership: u32,
}

impl GtdSatelliteUsage {
    pub(crate) fn to_sdk(self, index: usize) -> Result<SatelliteUsage, GtdStatus> {
        let constellation =
            GtdConstellation::from_abi_value(self.constellation).ok_or_else(|| {
                navigation::invalid_field(format_args!("rows[{index}].constellation"))
            })?;
        let membership = match self.membership {
            value if value == GtdSatelliteMembership::Unknown as u32 => {
                SatelliteMembership::Unknown
            }
            value if value == GtdSatelliteMembership::Unused as u32 => SatelliteMembership::Unused,
            value if value == GtdSatelliteMembership::Used as u32 => SatelliteMembership::Used,
            _ => {
                return Err(navigation::invalid_field(format_args!(
                    "rows[{index}].membership"
                )));
            }
        };
        Ok(SatelliteUsage::new(
            SatelliteIdentity::new(constellation.into(), self.prn),
            membership,
        ))
    }
}

impl TryFrom<&SatelliteUsage> for GtdSatelliteUsage {
    type Error = GtdStatus;
    fn try_from(row: &SatelliteUsage) -> Result<Self, Self::Error> {
        Ok(Self {
            constellation: GtdConstellation::from(row.identity.constellation) as u32,
            prn: row.identity.prn,
            membership: match row.membership {
                SatelliteMembership::Unknown => GtdSatelliteMembership::Unknown as u32,
                SatelliteMembership::Unused => GtdSatelliteMembership::Unused as u32,
                SatelliteMembership::Used => GtdSatelliteMembership::Used as u32,
                _ => return Err(navigation::invalid_field("membership")),
            },
        })
    }
}

/// Measurements are optional independently. Measured zero remains present.
/// Present NaN measurements decode as absent after serialization.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdSatelliteObservation {
    /// A `GtdConstellation` tag.
    pub constellation: u32,
    /// Satellite identifier within the constellation.
    pub prn: u32,
    /// Optional degrees above the horizon.
    pub elevation_deg: GtdOptF32,
    /// Optional degrees clockwise from true north.
    pub azimuth_deg: GtdOptF32,
    /// Optional measured signal in dB-Hz, including zero.
    pub snr_dbhz: GtdOptF32,
}

impl GtdSatelliteObservation {
    pub(crate) fn to_sdk(self, index: usize) -> Result<SatelliteObservation, GtdStatus> {
        let constellation =
            GtdConstellation::from_abi_value(self.constellation).ok_or_else(|| {
                navigation::invalid_field(format_args!("rows[{index}].constellation"))
            })?;
        Ok(SatelliteObservation::builder()
            .identity(SatelliteIdentity::new(constellation.into(), self.prn))
            .maybe_elevation(self.elevation_deg.to_opt())
            .maybe_azimuth(self.azimuth_deg.to_opt())
            .maybe_snr(self.snr_dbhz.to_opt())
            .build())
    }
}

impl From<&SatelliteObservation> for GtdSatelliteObservation {
    fn from(row: &SatelliteObservation) -> Self {
        Self {
            constellation: GtdConstellation::from(row.identity.constellation) as u32,
            prn: row.identity.prn,
            elevation_deg: row
                .elevation
                .map_or(optf32::opt_f32_none(), optf32::opt_f32_some),
            azimuth_deg: row
                .azimuth
                .map_or(optf32::opt_f32_none(), optf32::opt_f32_some),
            snr_dbhz: row.snr.map_or(optf32::opt_f32_none(), optf32::opt_f32_some),
        }
    }
}

/// All fields are caller-owned.
/// Absent placement has `normalized_time == gtd_ts_none()`.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdSatelliteReportInfo {
    /// Original clocks and recorded scope, coverage and aggregate.
    pub source: GtdSatelliteReportInput,
    /// Number of ordered source rows, including repeated identities.
    pub row_count: usize,
    /// Optional report placement time on the common navigation clock.
    pub normalized_time: GtdTimestamp,
    /// Zero-based insertion order across both report kinds when placement is present.
    pub input_order: u64,
    /// Zero for absent recorder placement, nonzero for present.
    pub placement_present: u8,
}

pub(crate) struct RecordedSatelliteReportMetadata<'a> {
    pub(crate) time: NavFixTime,
    pub(crate) scope: &'a SatelliteScope,
    pub(crate) coverage: SatelliteCoverage,
    pub(crate) reported_count: Option<u32>,
    pub(crate) row_count: usize,
    pub(crate) placement: Option<SatelliteReportPlacement>,
}

impl TryFrom<RecordedSatelliteReportMetadata<'_>> for GtdSatelliteReportInfo {
    type Error = GtdStatus;

    fn try_from(metadata: RecordedSatelliteReportMetadata<'_>) -> Result<Self, Self::Error> {
        let coverage = match metadata.coverage {
            SatelliteCoverage::Complete => GtdSatelliteCoverage::Complete as u32,
            SatelliteCoverage::Partial => GtdSatelliteCoverage::Partial as u32,
            _ => return Err(navigation::invalid_field("coverage")),
        };
        Ok(Self {
            source: GtdSatelliteReportInput {
                gps_time: metadata
                    .time
                    .gps_time()
                    .map_or(timestamp::gtd_ts_none(), timestamp::ts_from_datetime),
                sys_time: metadata
                    .time
                    .sys_time()
                    .map_or(timestamp::gtd_ts_none(), timestamp::ts_from_datetime),
                scope: GtdSatelliteScope::try_from(metadata.scope)?,
                coverage,
                reported_count: metadata.reported_count.into(),
            },
            row_count: metadata.row_count,
            normalized_time: metadata
                .placement
                .map_or(timestamp::gtd_ts_none(), |placement| {
                    timestamp::ts_from_datetime(placement.normalized_time)
                }),
            input_order: metadata
                .placement
                .map_or(0, |placement| placement.input_order),
            placement_present: u8::from(metadata.placement.is_some()),
        })
    }
}
