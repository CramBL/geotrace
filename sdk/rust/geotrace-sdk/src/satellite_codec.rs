use std::fmt::Write as _;

use hdf5_pure::{
    AttrValue, Dataset, Datatype, DatatypeByteOrder, DeflateLevel, FileBuilder, FixedPointLayout,
    FloatingPointLayout, GroupBuilder,
};
use strum::IntoEnumIterator as _;

use crate::builder;
use crate::error::{Error, FieldLocation};
use crate::read;
use crate::size_checked_file::{ColumnContext, SizeCheckedFile, SizeCheckedGroup};
use crate::{
    Constellation, ConstellationSet, NavFile, NavFixTime, NavPoint, RecordedFixTimestamps,
    SatelliteCoverage, SatelliteEvidence, SatelliteIdentity, SatelliteMembership,
    SatelliteObservation, SatelliteObservationReport, SatelliteReportPlacement, SatelliteScope,
    SatelliteUsage, SatelliteUsageReport,
};

#[derive(Clone, Copy)]
enum NumericType {
    U8,
    U32,
    U64,
    F32,
}

impl NumericType {
    fn datatype(self) -> Datatype {
        let (size, signed) = match self {
            Self::U8 => (1, false),
            Self::U32 => (4, false),
            Self::U64 => (8, false),
            Self::F32 => {
                return Datatype::FloatingPoint {
                    size: 4,
                    byte_order: DatatypeByteOrder::LittleEndian,
                    layout: FloatingPointLayout::IEEE754_BINARY32,
                };
            }
        };
        Datatype::FixedPoint {
            size,
            byte_order: DatatypeByteOrder::LittleEndian,
            layout: FixedPointLayout {
                signed,
                bit_offset: 0,
                bit_precision: (size * 8) as u16,
            },
        }
    }
}

struct Columns {
    group: SizeCheckedGroup,
    name: &'static str,
    rows: usize,
}

impl Columns {
    fn open(
        file: &SizeCheckedFile,
        FieldLocation {
            group: name,
            dataset: index,
        }: FieldLocation,
    ) -> Result<Option<Self>, Error> {
        let Some(group) = file.optional_group(name)? else {
            return Ok(None);
        };
        let shape = group
            .optional_dataset(index)?
            .ok_or(Error::MissingSatelliteColumn {
                group: name,
                dataset: index,
            })?
            .shape()?;
        let rows = match shape.as_slice() {
            [rows] => usize::try_from(*rows).map_err(|_error| Error::InvalidDatasetShape {
                group: name,
                dataset: index,
                expected: vec![0],
                actual: shape.clone(),
            })?,
            _ => {
                return Err(Error::InvalidDatasetShape {
                    group: name,
                    dataset: index,
                    expected: vec![shape.first().copied().unwrap_or(0)],
                    actual: shape,
                });
            }
        };
        Ok(Some(Self { group, name, rows }))
    }

    fn column(&self, name: &'static str, kind: NumericType) -> Result<Dataset, Error> {
        self.group.typed_column(
            ColumnContext::Satellite(self.name),
            name,
            self.rows,
            &kind.datatype(),
        )
    }

    fn invalid_tag(&self, dataset: &'static str, record: usize, code: u32) -> Error {
        Error::InvalidSatelliteTag {
            group: self.name,
            dataset,
            record,
            code,
        }
    }
}

#[derive(Default)]
struct ReportColumns {
    nav_point_idx: Vec<u64>,
    gps: Vec<u64>,
    sys: Vec<u64>,
    scope: Vec<u32>,
    coverage: Vec<u8>,
    count: Vec<u32>,
    count_present: Vec<u8>,
    normalized_time: Vec<u64>,
    input_order: Vec<u64>,
}

#[derive(Clone, Copy)]
struct ReportMetadata<'a> {
    time: NavFixTime,
    scope: &'a SatelliteScope,
    coverage: SatelliteCoverage,
    count: Option<u32>,
    placement: Option<SatelliteReportPlacement>,
}

impl ReportColumns {
    fn push(
        &mut self,
        name: &'static str,
        point: usize,
        metadata: ReportMetadata<'_>,
    ) -> Result<(), Error> {
        let record = self.nav_point_idx.len();
        self.nav_point_idx.push(point as u64);
        self.gps.push(builder::opt_datetime_to_u64(
            metadata.time.gps_time(),
            FieldLocation {
                group: name,
                dataset: "gps_time_us",
            },
            record,
        )?);
        self.sys.push(builder::opt_datetime_to_u64(
            metadata.time.sys_time(),
            FieldLocation {
                group: name,
                dataset: "sys_time_us",
            },
            record,
        )?);
        self.scope.push(match metadata.scope {
            SatelliteScope::Global => 0,
            SatelliteScope::Constellations(set) => {
                set.as_slice().iter().fold(0, |mask, constellation| {
                    mask | (1 << constellation.wire_code())
                })
            }
        });
        self.coverage.push(match metadata.coverage {
            SatelliteCoverage::Complete => 0,
            SatelliteCoverage::Partial => 1,
        });
        self.count.push(metadata.count.unwrap_or(0));
        self.count_present.push(u8::from(metadata.count.is_some()));
        self.normalized_time.push(builder::opt_datetime_to_u64(
            metadata
                .placement
                .map(|placement| placement.normalized_time),
            FieldLocation {
                group: name,
                dataset: "normalized_time_us",
            },
            record,
        )?);
        self.input_order.push(
            metadata
                .placement
                .map_or(0, |placement| placement.input_order),
        );
        Ok(())
    }

    fn write(&self, name: &'static str, file: &mut FileBuilder) {
        let mut group = file.create_group(name);
        for (name, data) in [
            ("nav_point_idx", &self.nav_point_idx),
            ("gps_time_us", &self.gps),
            ("sys_time_us", &self.sys),
            ("normalized_time_us", &self.normalized_time),
            ("input_order", &self.input_order),
        ] {
            group
                .create_dataset(name)
                .with_u64_data(data)
                .with_shape(&[data.len() as u64])
                .with_chunks(&[CHUNK_SIZE])
                .with_shuffle()
                .with_deflate(DEFLATE_LEVEL);
        }
        for (name, data) in [("scope", &self.scope), ("reported_count", &self.count)] {
            group
                .create_dataset(name)
                .with_u32_data(data)
                .with_shape(&[data.len() as u64])
                .with_chunks(&[CHUNK_SIZE])
                .with_shuffle()
                .with_deflate(DEFLATE_LEVEL);
        }
        for (name, data) in [
            ("coverage", &self.coverage),
            ("reported_count_present", &self.count_present),
        ] {
            group
                .create_dataset(name)
                .with_u8_data(data)
                .with_shape(&[data.len() as u64])
                .with_chunks(&[CHUNK_SIZE])
                .with_deflate(DEFLATE_LEVEL);
        }
        group.set_attr(
            "scope_encoding",
            AttrValue::String("0=global,nonzero=constellation wire-code bit set".into()),
        );
        group.set_attr(
            "coverage_encoding",
            AttrValue::String("0=complete,1=partial".into()),
        );
        file.add_group(group.finish());
    }
}

#[derive(Default)]
struct IdentityColumns {
    report_idx: Vec<u64>,
    constellation: Vec<u8>,
    prn: Vec<u32>,
}

impl IdentityColumns {
    fn push(&mut self, report: usize, identity: SatelliteIdentity) {
        self.report_idx.push(report as u64);
        self.constellation.push(identity.constellation.wire_code());
        self.prn.push(identity.prn);
    }

    fn write(&self, group: &mut GroupBuilder) {
        group
            .create_dataset("report_idx")
            .with_u64_data(&self.report_idx)
            .with_shape(&[self.report_idx.len() as u64])
            .with_chunks(&[CHUNK_SIZE])
            .with_shuffle()
            .with_deflate(DEFLATE_LEVEL);
        group
            .create_dataset("constellation")
            .with_u8_data(&self.constellation)
            .set_attr(
                "encoding",
                AttrValue::String(Constellation::encoding_attribute()),
            )
            .with_shape(&[self.constellation.len() as u64])
            .with_chunks(&[CHUNK_SIZE])
            .with_deflate(DEFLATE_LEVEL);
        group
            .create_dataset("prn")
            .with_u32_data(&self.prn)
            .with_shape(&[self.prn.len() as u64])
            .with_chunks(&[CHUNK_SIZE])
            .with_shuffle()
            .with_deflate(DEFLATE_LEVEL);
    }
}

pub(crate) fn write(file: &NavFile, output: &mut FileBuilder) -> Result<(), Error> {
    let mut usage_reports = ReportColumns::default();
    let mut observation_reports = ReportColumns::default();
    let mut usage_rows = IdentityColumns::default();
    let mut observation_rows = IdentityColumns::default();
    let mut memberships = Vec::new();
    let mut elevations = Vec::new();
    let mut azimuths = Vec::new();
    let mut signals = Vec::new();
    for (point_index, point) in file.nav_points().iter().enumerate() {
        for report in &point.satellite_evidence().usage {
            let report_index = usage_reports.nav_point_idx.len();
            usage_reports.push(
                USAGE_REPORTS,
                point_index,
                ReportMetadata {
                    time: report.time,
                    scope: &report.scope,
                    coverage: report.coverage,
                    count: report.reported_count,
                    placement: report.placement,
                },
            )?;
            for row in &report.satellites {
                usage_rows.push(report_index, row.identity);
                memberships.push(match row.membership {
                    SatelliteMembership::Unknown => 0,
                    SatelliteMembership::Unused => 1,
                    SatelliteMembership::Used => 2,
                });
            }
        }
        for report in &point.satellite_evidence().observations {
            let report_index = observation_reports.nav_point_idx.len();
            observation_reports.push(
                OBSERVATION_REPORTS,
                point_index,
                ReportMetadata {
                    time: report.time,
                    scope: &report.scope,
                    coverage: report.coverage,
                    count: report.reported_count,
                    placement: report.placement,
                },
            )?;
            for row in &report.satellites {
                observation_rows.push(report_index, row.identity);
                elevations.push(row.elevation.unwrap_or(f32::NAN));
                azimuths.push(row.azimuth.unwrap_or(f32::NAN));
                signals.push(row.snr.unwrap_or(f32::NAN));
            }
        }
    }
    if !usage_reports.nav_point_idx.is_empty() {
        usage_reports.write(USAGE_REPORTS, output);
        let mut group = output.create_group(USAGE_ROWS);
        usage_rows.write(&mut group);
        group
            .create_dataset("membership")
            .with_u8_data(&memberships)
            .with_shape(&[memberships.len() as u64])
            .set_attr(
                "encoding",
                AttrValue::String("0=unknown,1=unused,2=used".into()),
            )
            .with_chunks(&[CHUNK_SIZE])
            .with_deflate(DEFLATE_LEVEL);
        output.add_group(group.finish());
    }
    if !observation_reports.nav_point_idx.is_empty() {
        observation_reports.write(OBSERVATION_REPORTS, output);
        let mut group = output.create_group(OBSERVATION_ROWS);
        observation_rows.write(&mut group);
        for (name, data) in [
            ("elevation", elevations),
            ("azimuth", azimuths),
            ("snr", signals),
        ] {
            group
                .create_dataset(name)
                .with_f32_data(&data)
                .with_shape(&[data.len() as u64])
                .set_attr("nan_means", AttrValue::String("absent".into()))
                .with_chunks(&[CHUNK_SIZE])
                .with_shuffle()
                .with_deflate(DEFLATE_LEVEL);
        }
        output.add_group(group.finish());
    }
    Ok(())
}

struct DecodedReport {
    point: u64,
    time: NavFixTime,
    scope: SatelliteScope,
    coverage: SatelliteCoverage,
    count: Option<u32>,
    placement: Option<SatelliteReportPlacement>,
}

impl Columns {
    fn placements(&self) -> Result<Vec<Option<SatelliteReportPlacement>>, Error> {
        let times = self.group.optional_dataset("normalized_time_us")?;
        let orders = self.group.optional_dataset("input_order")?;
        match (times.is_some(), orders.is_some()) {
            (false, false) => return Ok(vec![None; self.rows]),
            (true, true) => {}
            _ => {
                return Err(Error::MissingSatelliteColumn {
                    group: self.name,
                    dataset: if times.is_none() {
                        "normalized_time_us"
                    } else {
                        "input_order"
                    },
                });
            }
        }
        let times = self
            .column("normalized_time_us", NumericType::U64)?
            .read_u64()?;
        let orders = self.column("input_order", NumericType::U64)?.read_u64()?;
        times
            .into_iter()
            .zip(orders)
            .enumerate()
            .map(|(record, (time, order))| {
                Ok(read::decode_optional_timestamp(
                    FieldLocation {
                        group: self.name,
                        dataset: "normalized_time_us",
                    },
                    record,
                    time,
                )?
                .map(|time| SatelliteReportPlacement::new(time, order)))
            })
            .collect()
    }

    fn reports(&self) -> Result<Vec<DecodedReport>, Error> {
        let placements = self.placements()?;
        let points = self.column("nav_point_idx", NumericType::U64)?.read_u64()?;
        let gps = self.column("gps_time_us", NumericType::U64)?.read_u64()?;
        let sys = self.column("sys_time_us", NumericType::U64)?.read_u64()?;
        let scopes = self.column("scope", NumericType::U32)?.read_u32()?;
        let coverage = self.column("coverage", NumericType::U8)?.read_u8()?;
        let counts = self
            .column("reported_count", NumericType::U32)?
            .read_u32()?;
        let count_presence = self
            .column("reported_count_present", NumericType::U8)?
            .read_u8()?;
        let supported_scope = Constellation::iter().fold(0_u32, |mask, constellation| {
            mask | (1 << constellation.wire_code())
        });
        points
            .into_iter()
            .zip(gps)
            .zip(sys)
            .zip(scopes)
            .zip(coverage)
            .zip(counts)
            .zip(count_presence)
            .enumerate()
            .map(
                |(record, ((((((point, gps), sys), scope), coverage), count), present))| {
                    let recorded = RecordedFixTimestamps {
                        gps: read::decode_optional_timestamp(
                            FieldLocation {
                                group: self.name,
                                dataset: "gps_time_us",
                            },
                            record,
                            gps,
                        )?,
                        sys: read::decode_optional_timestamp(
                            FieldLocation {
                                group: self.name,
                                dataset: "sys_time_us",
                            },
                            record,
                            sys,
                        )?,
                    };
                    let time = NavFixTime::from_recorded(recorded).ok_or(
                        Error::SatelliteReportWithoutTimestamp {
                            group: self.name,
                            report: record,
                        },
                    )?;
                    let scope = if scope == 0 {
                        SatelliteScope::Global
                    } else {
                        if scope & !supported_scope != 0 {
                            return Err(self.invalid_tag("scope", record, scope));
                        }
                        let mut constellations = Constellation::iter()
                            .filter(|constellation| scope & (1 << constellation.wire_code()) != 0);
                        let first = constellations
                            .next()
                            .ok_or_else(|| self.invalid_tag("scope", record, scope))?;
                        SatelliteScope::Constellations(ConstellationSet::new(first, constellations))
                    };
                    let coverage = match coverage {
                        0 => SatelliteCoverage::Complete,
                        1 => SatelliteCoverage::Partial,
                        code => return Err(self.invalid_tag("coverage", record, u32::from(code))),
                    };
                    let count = match present {
                        0 => None,
                        1 => Some(count),
                        code => {
                            return Err(self.invalid_tag(
                                "reported_count_present",
                                record,
                                u32::from(code),
                            ));
                        }
                    };
                    Ok(DecodedReport {
                        point,
                        time,
                        scope,
                        coverage,
                        count,
                        placement: placements.get(record).copied().flatten(),
                    })
                },
            )
            .collect()
    }

    fn identities(&self) -> Result<Vec<(u64, SatelliteIdentity)>, Error> {
        let reports = self.column("report_idx", NumericType::U64)?.read_u64()?;
        let constellations = self.column("constellation", NumericType::U8)?.read_u8()?;
        let prns = self.column("prn", NumericType::U32)?.read_u32()?;
        reports
            .into_iter()
            .zip(constellations)
            .zip(prns)
            .enumerate()
            .map(|(record, ((report, code), prn))| {
                let constellation = Constellation::from_wire_code(code, self.name)
                    .map_err(|_error| self.invalid_tag("constellation", record, u32::from(code)))?;
                Ok((report, SatelliteIdentity::new(constellation, prn)))
            })
            .collect()
    }
}

pub(crate) fn read(file: &SizeCheckedFile, points: &mut [NavPoint]) -> Result<bool, Error> {
    let usage = Columns::open(
        file,
        FieldLocation {
            group: USAGE_REPORTS,
            dataset: "nav_point_idx",
        },
    )?;
    let observations = Columns::open(
        file,
        FieldLocation {
            group: OBSERVATION_REPORTS,
            dataset: "nav_point_idx",
        },
    )?;
    let usage_rows = Columns::open(
        file,
        FieldLocation {
            group: USAGE_ROWS,
            dataset: "report_idx",
        },
    )?;
    let observation_rows = Columns::open(
        file,
        FieldLocation {
            group: OBSERVATION_ROWS,
            dataset: "report_idx",
        },
    )?;
    if usage.is_none()
        && observations.is_none()
        && usage_rows.is_none()
        && observation_rows.is_none()
    {
        return Ok(false);
    }
    if file.optional_group("sat_reports")?.is_some()
        || file.optional_group("tracked_sats")?.is_some()
    {
        return Err(Error::MixedSatelliteLayouts);
    }
    let mut evidence = vec![SatelliteEvidence::default(); points.len()];
    if let Some(columns) = usage {
        let metadata = columns.reports()?;
        let mut reports: Vec<_> = metadata
            .iter()
            .map(|report| SatelliteUsageReport {
                time: report.time,
                placement: report.placement,
                scope: report.scope.clone(),
                coverage: report.coverage,
                reported_count: report.count,
                satellites: Vec::new(),
            })
            .collect();
        let rows = usage_rows.ok_or(Error::MissingSatelliteGroup { group: USAGE_ROWS })?;
        let identities = rows.identities()?;
        let memberships = rows.column("membership", NumericType::U8)?.read_u8()?;
        for (record, ((report, identity), membership)) in
            identities.into_iter().zip(memberships).enumerate()
        {
            let membership = match membership {
                0 => SatelliteMembership::Unknown,
                1 => SatelliteMembership::Unused,
                2 => SatelliteMembership::Used,
                code => return Err(rows.invalid_tag("membership", record, u32::from(code))),
            };
            read::row_addressed_by_index(
                &mut reports,
                USAGE_REPORTS,
                FieldLocation {
                    group: USAGE_ROWS,
                    dataset: "report_idx",
                },
                record,
                report,
            )?
            .satellites
            .push(SatelliteUsage::new(identity, membership));
        }
        for (record, (metadata, report)) in metadata.into_iter().zip(reports).enumerate() {
            read::row_addressed_by_index(
                &mut evidence,
                "nav_points",
                FieldLocation {
                    group: USAGE_REPORTS,
                    dataset: "nav_point_idx",
                },
                record,
                metadata.point,
            )?
            .usage
            .push(report);
        }
    } else if usage_rows.is_some() {
        return Err(Error::MissingSatelliteGroup {
            group: USAGE_REPORTS,
        });
    }
    if let Some(columns) = observations {
        let metadata = columns.reports()?;
        let mut reports: Vec<_> = metadata
            .iter()
            .map(|report| SatelliteObservationReport {
                time: report.time,
                placement: report.placement,
                scope: report.scope.clone(),
                coverage: report.coverage,
                reported_count: report.count,
                satellites: Vec::new(),
            })
            .collect();
        let rows = observation_rows.ok_or(Error::MissingSatelliteGroup {
            group: OBSERVATION_ROWS,
        })?;
        let identities = rows.identities()?;
        let elevations = rows.column("elevation", NumericType::F32)?.read_f32()?;
        let azimuths = rows.column("azimuth", NumericType::F32)?.read_f32()?;
        let signals = rows.column("snr", NumericType::F32)?.read_f32()?;
        for (record, ((((report, identity), elevation), azimuth), snr)) in identities
            .into_iter()
            .zip(elevations)
            .zip(azimuths)
            .zip(signals)
            .enumerate()
        {
            read::row_addressed_by_index(
                &mut reports,
                OBSERVATION_REPORTS,
                FieldLocation {
                    group: OBSERVATION_ROWS,
                    dataset: "report_idx",
                },
                record,
                report,
            )?
            .satellites
            .push(SatelliteObservation {
                identity,
                elevation: (!elevation.is_nan()).then_some(elevation),
                azimuth: (!azimuth.is_nan()).then_some(azimuth),
                snr: (!snr.is_nan()).then_some(snr),
            });
        }
        for (record, (metadata, report)) in metadata.into_iter().zip(reports).enumerate() {
            read::row_addressed_by_index(
                &mut evidence,
                "nav_points",
                FieldLocation {
                    group: OBSERVATION_REPORTS,
                    dataset: "nav_point_idx",
                },
                record,
                metadata.point,
            )?
            .observations
            .push(report);
        }
    } else if observation_rows.is_some() {
        return Err(Error::MissingSatelliteGroup {
            group: OBSERVATION_REPORTS,
        });
    }
    for (point, evidence) in points.iter_mut().zip(evidence) {
        point.set_satellite_evidence(evidence);
    }
    Ok(true)
}

pub(crate) fn inspect(
    file: &SizeCheckedFile,
    points: &mut [NavPoint],
    output: &mut String,
) -> bool {
    match self::read(file, points) {
        Ok(false) => false,
        Err(error) => {
            writeln!(output, "Satellite evidence: {error}").ok();
            true
        }
        Ok(true) => {
            let usage = points.iter().enumerate().flat_map(|(point_index, point)| {
                point
                    .satellite_evidence()
                    .usage
                    .iter()
                    .enumerate()
                    .map(move |(report_index, report)| (point_index, report_index, report))
            });
            writeln!(output, "Satellite usage reports: {}", usage.clone().count()).ok();
            for (point, index, report) in usage.take(read::MAX_LISTED_ENTRIES) {
                writeln!(
                    output,
                    "  point {point}, report {index}: {:?} {:?}, count {:?}, {} rows",
                    report.scope,
                    report.coverage,
                    report.reported_count,
                    report.satellites.len()
                )
                .ok();
                report.time.write_inspection_timestamps(output);
                if let Some(placement) = report.placement {
                    writeln!(
                        output,
                        "    Normalized time: {}, input order: {}",
                        placement.normalized_time, placement.input_order
                    )
                    .ok();
                }
                for row in report.satellites.iter().take(read::MAX_LISTED_ENTRIES) {
                    writeln!(output, "    {row:?}").ok();
                }
            }
            let observations = points.iter().enumerate().flat_map(|(point_index, point)| {
                point
                    .satellite_evidence()
                    .observations
                    .iter()
                    .enumerate()
                    .map(move |(report_index, report)| (point_index, report_index, report))
            });
            writeln!(
                output,
                "Satellite observation reports: {}",
                observations.clone().count()
            )
            .ok();
            for (point, index, report) in observations.take(read::MAX_LISTED_ENTRIES) {
                writeln!(
                    output,
                    "  point {point}, report {index}: {:?} {:?}, count {:?}, {} rows",
                    report.scope,
                    report.coverage,
                    report.reported_count,
                    report.satellites.len()
                )
                .ok();
                report.time.write_inspection_timestamps(output);
                if let Some(placement) = report.placement {
                    writeln!(
                        output,
                        "    Normalized time: {}, input order: {}",
                        placement.normalized_time, placement.input_order
                    )
                    .ok();
                }
                for row in report.satellites.iter().take(read::MAX_LISTED_ENTRIES) {
                    writeln!(output, "    {row:?}").ok();
                }
            }
            true
        }
    }
}

impl NavFixTime {
    fn write_inspection_timestamps(self, output: &mut String) {
        for (label, time) in [
            ("receiver time", self.gps_time()),
            ("host time", self.sys_time()),
        ] {
            if let Some(time) = time {
                writeln!(
                    output,
                    "    {label}: {}",
                    time.format(INSPECTION_TIMESTAMP_FORMAT)
                )
                .ok();
            } else {
                writeln!(output, "    {label}: unreported").ok();
            }
        }
    }
}

const CHUNK_SIZE: u64 = 8_192;

const USAGE_REPORTS: &str = "satellite_usage_reports";
const OBSERVATION_REPORTS: &str = "satellite_observation_reports";
const USAGE_ROWS: &str = "satellite_usage";
const OBSERVATION_ROWS: &str = "satellite_observations";

const DEFLATE_LEVEL: DeflateLevel = DeflateLevel::Six;
const INSPECTION_TIMESTAMP_FORMAT: &str = "%Y-%m-%d %H:%M:%S%.f UTC";
