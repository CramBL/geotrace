//! Reading the global ionosphere map subset of IONEX 1.0 and 1.1.
//!
//! Every line is a record of up to 80 columns: values in the first 60, a
//! 20-character label after them. The header declares the grid and the
//! scaling exponent, then one `START OF TEC MAP` block per epoch holds a
//! `LAT/LON1/LON2/DLON/H` record per latitude band followed by that band's
//! values, 16 per row and 5 columns each. `9999` marks a node the producer
//! published no value for, and reaches callers as [`None`].
//!
//! RMS and height map blocks are read past without being kept.

use chrono::{DateTime, NaiveDate, TimeDelta, Utc};

use crate::IONOSPHERE_MAPS_TYPE;
use crate::grid::{
    AxisDeclaration, AxisError, DEGREES_TOLERANCE, GridAxis, LatitudeAxis, LongitudeAxis, MapGrid,
};
use crate::maps::{GlobalIonosphereMaps, TecMap};
use crate::tec::{ScalingExponent, TotalElectronContent};

const VERSION_TYPE_LABEL: &str = "IONEX VERSION / TYPE";
const EPOCH_OF_FIRST_MAP_LABEL: &str = "EPOCH OF FIRST MAP";
const EPOCH_OF_LAST_MAP_LABEL: &str = "EPOCH OF LAST MAP";
const INTERVAL_LABEL: &str = "INTERVAL";
const MAP_COUNT_LABEL: &str = "# OF MAPS IN FILE";
const EXPONENT_LABEL: &str = "EXPONENT";
const HEIGHT_AXIS_LABEL: &str = "HGT1 / HGT2 / DHGT";
const LATITUDE_AXIS_LABEL: &str = "LAT1 / LAT2 / DLAT";
const LONGITUDE_AXIS_LABEL: &str = "LON1 / LON2 / DLON";
const END_OF_HEADER_LABEL: &str = "END OF HEADER";
const START_OF_TEC_MAP_LABEL: &str = "START OF TEC MAP";
const END_OF_TEC_MAP_LABEL: &str = "END OF TEC MAP";
const START_OF_RMS_MAP_LABEL: &str = "START OF RMS MAP";
const END_OF_RMS_MAP_LABEL: &str = "END OF RMS MAP";
const START_OF_HEIGHT_MAP_LABEL: &str = "START OF HEIGHT MAP";
const END_OF_HEIGHT_MAP_LABEL: &str = "END OF HEIGHT MAP";
const EPOCH_OF_CURRENT_MAP_LABEL: &str = "EPOCH OF CURRENT MAP";
const LATITUDE_BAND_LABEL: &str = "LAT/LON1/LON2/DLON/H";
const END_OF_FILE_LABEL: &str = "END OF FILE";

/// What a value row is named in an error that expected one.
const VALUE_ROW_EXPECTATION: &str = "a row of TEC values";

/// Column the record label starts at.
const LABEL_COLUMN: usize = 60;

/// Columns the version record writes its number in.
const VERSION_FIELD: FieldSpan = FieldSpan { start: 0, width: 8 };

/// Column the version record writes its file type in.
const FILE_TYPE_COLUMN: usize = 20;

/// Fields of a grid record, `2X,5F6.1`.
const GRID_FIELD_OFFSET: usize = 2;
const GRID_FIELD_WIDTH: usize = 6;

/// Fields of an epoch record, `6I6`.
const EPOCH_FIELD_WIDTH: usize = 6;
const EPOCH_FIELD_COUNT: usize = 6;

/// Fields of a value row, `16I5`.
const VALUE_FIELD_WIDTH: usize = 5;
const VALUES_PER_ROW: usize = 16;

/// The stored integer standing for a node without a published value.
const MISSING_VALUE: i32 = 9999;

/// How far a band's declared height may stand from the shell the header
/// declares.
const HEIGHT_TOLERANCE_KM: f64 = 1e-6;

/// Labels that structure the file, which a value row never carries.
const BLOCK_LABELS: [&str; 7] = [
    START_OF_TEC_MAP_LABEL,
    END_OF_TEC_MAP_LABEL,
    START_OF_RMS_MAP_LABEL,
    END_OF_RMS_MAP_LABEL,
    START_OF_HEIGHT_MAP_LABEL,
    END_OF_HEIGHT_MAP_LABEL,
    END_OF_FILE_LABEL,
];

/// Why a file could not be read as global ionosphere maps.
#[derive(Debug, thiserror::Error)]
pub enum ParseError {
    #[error("the file has no {END_OF_HEADER_LABEL} record")]
    MissingHeaderEnd,

    #[error("the header has no {label} record")]
    MissingHeaderRecord { label: &'static str },

    #[error("line {line_number}: IONEX version {version} is not 1.x")]
    UnsupportedVersion { line_number: usize, version: f64 },

    #[error(
        "line {line_number}: file type {file_type:?} is not the ionosphere maps type {IONOSPHERE_MAPS_TYPE}"
    )]
    UnsupportedFileType {
        line_number: usize,
        file_type: String,
    },

    #[error(
        "line {line_number}: the heights {first_km}km to {last_km}km in steps of {step_km}km are not one shell"
    )]
    UnsupportedHeightAxis {
        line_number: usize,
        first_km: f64,
        last_km: f64,
        step_km: f64,
    },

    #[error("line {line_number}: {label}: {source}")]
    Axis {
        line_number: usize,
        label: &'static str,
        source: AxisError,
    },

    #[error("line {line_number}: {label} field {text:?} is not a number")]
    NumberField {
        line_number: usize,
        label: &'static str,
        text: String,
    },

    #[error("line {line_number}: {label} record {text:?} is not a calendar date and time")]
    Epoch {
        line_number: usize,
        label: &'static str,
        text: String,
    },

    #[error("line {line_number}: an interval of {seconds} seconds is not a time between maps")]
    Interval { line_number: usize, seconds: i64 },

    #[error("line {line_number}: TEC values cannot be scaled by 10^{exponent}")]
    Exponent { line_number: usize, exponent: i64 },

    #[error("line {line_number}: found {found:?} where {expected} was expected")]
    UnexpectedRecord {
        line_number: usize,
        expected: &'static str,
        found: String,
    },

    #[error("the file ends where {expected} was expected")]
    UnexpectedEndOfFile { expected: &'static str },

    #[error("line {line_number}: the block opened here has no {label} record")]
    UnterminatedBlock {
        line_number: usize,
        label: &'static str,
    },

    #[error(
        "line {line_number}: a band at {found_degrees} deg stands where the grid has {expected_degrees} deg"
    )]
    UnexpectedLatitudeBand {
        line_number: usize,
        found_degrees: f64,
        expected_degrees: f64,
    },

    #[error("line {line_number}: the band's longitudes are not the ones the header declares")]
    BandLongitudesDiffer { line_number: usize },

    #[error(
        "line {line_number}: the band's height of {found_km}km is not the shell at {expected_km}km"
    )]
    BandHeightDiffers {
        line_number: usize,
        found_km: f64,
        expected_km: f64,
    },

    #[error("line {line_number}: TEC value {text:?} is not an integer")]
    ValueNotAnInteger { line_number: usize, text: String },

    #[error(
        "line {line_number}: the row holds fewer than the {expected_values} values of its band"
    )]
    TruncatedValueRow {
        line_number: usize,
        expected_values: usize,
    },

    #[error("the header declares {declared} maps and the file holds {found}")]
    MapCountMismatch { declared: usize, found: usize },

    #[error("the map at {epoch} does not follow the one at {previous}")]
    MapEpochsOutOfOrder {
        previous: DateTime<Utc>,
        epoch: DateTime<Utc>,
    },

    #[error("the header declares {label} at {declared} and the file holds {found}")]
    DeclaredEpochMismatch {
        label: &'static str,
        declared: DateTime<Utc>,
        found: DateTime<Utc>,
    },
}

/// Read a decompressed IONEX file.
pub fn global_ionosphere_maps(text: &str) -> Result<GlobalIonosphereMaps, ParseError> {
    let mut records = Records::new(text);
    let header = read_header(&mut records)?;
    let maps = read_maps(&mut records, header)?;
    check_maps_against_header(&maps, header)?;
    Ok(GlobalIonosphereMaps::new(
        header.grid,
        header.interval,
        maps,
    ))
}

/// Columns one fixed-width field occupies.
#[derive(Debug, Clone, Copy)]
struct FieldSpan {
    start: usize,
    width: usize,
}

/// One line of the file.
#[derive(Debug, Clone, Copy)]
struct Record<'a> {
    line_number: usize,
    text: &'a str,
}

impl<'a> Record<'a> {
    /// The record label, empty on the value rows of a map, which carry none.
    fn label(self) -> &'a str {
        self.text
            .split_at_checked(LABEL_COLUMN)
            .map_or("", |(_values, label)| label.trim())
    }

    /// The columns before the label.
    fn values(self) -> &'a str {
        self.text
            .split_at_checked(LABEL_COLUMN)
            .map_or(self.text, |(values, _label)| values)
    }

    fn field(self, span: FieldSpan) -> Option<&'a str> {
        let values = self.values();
        let field = values
            .get(span.start..span.start.checked_add(span.width)?)
            .or_else(|| values.get(span.start..))?
            .trim();
        (!field.is_empty()).then_some(field)
    }

    fn grid_field(self, position: usize) -> Option<&'a str> {
        self.field(FieldSpan {
            start: GRID_FIELD_OFFSET.checked_add(position.checked_mul(GRID_FIELD_WIDTH)?)?,
            width: GRID_FIELD_WIDTH,
        })
    }

    fn epoch_field(self, position: usize) -> Option<&'a str> {
        self.field(FieldSpan {
            start: position.checked_mul(EPOCH_FIELD_WIDTH)?,
            width: EPOCH_FIELD_WIDTH,
        })
    }

    /// A value row runs across the label columns, so its fields are read from
    /// the whole line.
    fn value_field(self, position: usize) -> Option<&'a str> {
        let start = position.checked_mul(VALUE_FIELD_WIDTH)?;
        let field = self
            .text
            .get(start..start.checked_add(VALUE_FIELD_WIDTH)?)
            .or_else(|| self.text.get(start..))?
            .trim();
        (!field.is_empty()).then_some(field)
    }
}

struct Records<'a> {
    lines: std::iter::Enumerate<std::str::Lines<'a>>,
}

impl<'a> Records<'a> {
    fn new(text: &'a str) -> Self {
        Self {
            lines: text.lines().enumerate(),
        }
    }

    fn next_record(&mut self) -> Option<Record<'a>> {
        self.lines.next().map(|(index, text)| Record {
            line_number: index.saturating_add(1),
            text,
        })
    }

    /// The next record, which must carry `label`.
    fn next_labelled(&mut self, label: &'static str) -> Result<Record<'a>, ParseError> {
        let record = self
            .next_record()
            .ok_or(ParseError::UnexpectedEndOfFile { expected: label })?;
        if record.label() == label {
            Ok(record)
        } else {
            Err(ParseError::UnexpectedRecord {
                line_number: record.line_number,
                expected: label,
                found: record.label().to_owned(),
            })
        }
    }
}

/// What the header declares about the file that follows it.
#[derive(Debug, Clone, Copy)]
struct Header {
    grid: MapGrid,
    interval: TimeDelta,
    exponent: ScalingExponent,
    declared_map_count: usize,
    first_epoch: DateTime<Utc>,
    last_epoch: DateTime<Utc>,
}

#[derive(Debug, Default, Clone, Copy)]
struct HeaderFields {
    version: Option<f64>,
    first_epoch: Option<DateTime<Utc>>,
    last_epoch: Option<DateTime<Utc>>,
    interval: Option<TimeDelta>,
    declared_map_count: Option<usize>,
    exponent: Option<ScalingExponent>,
    latitudes: Option<GridAxis>,
    longitudes: Option<GridAxis>,
    shell_height_km: Option<f64>,
}

impl HeaderFields {
    fn into_header(self) -> Result<Header, ParseError> {
        let Self {
            version,
            first_epoch,
            last_epoch,
            interval,
            declared_map_count,
            exponent,
            latitudes,
            longitudes,
            shell_height_km,
        } = self;
        let missing = |label| ParseError::MissingHeaderRecord { label };
        if version.is_none() {
            return Err(missing(VERSION_TYPE_LABEL));
        }
        Ok(Header {
            grid: MapGrid {
                latitudes: LatitudeAxis::new(
                    latitudes.ok_or_else(|| missing(LATITUDE_AXIS_LABEL))?,
                ),
                longitudes: LongitudeAxis::new(
                    longitudes.ok_or_else(|| missing(LONGITUDE_AXIS_LABEL))?,
                ),
                shell_height_km: shell_height_km.ok_or_else(|| missing(HEIGHT_AXIS_LABEL))?,
            },
            interval: interval.ok_or_else(|| missing(INTERVAL_LABEL))?,
            exponent: exponent.unwrap_or_default(),
            declared_map_count: declared_map_count.ok_or_else(|| missing(MAP_COUNT_LABEL))?,
            first_epoch: first_epoch.ok_or_else(|| missing(EPOCH_OF_FIRST_MAP_LABEL))?,
            last_epoch: last_epoch.ok_or_else(|| missing(EPOCH_OF_LAST_MAP_LABEL))?,
        })
    }
}

fn read_header(records: &mut Records<'_>) -> Result<Header, ParseError> {
    let mut fields = HeaderFields::default();
    while let Some(record) = records.next_record() {
        match record.label() {
            VERSION_TYPE_LABEL => fields.version = Some(read_version(record)?),
            EPOCH_OF_FIRST_MAP_LABEL => {
                fields.first_epoch = Some(read_epoch(record, EPOCH_OF_FIRST_MAP_LABEL)?);
            }
            EPOCH_OF_LAST_MAP_LABEL => {
                fields.last_epoch = Some(read_epoch(record, EPOCH_OF_LAST_MAP_LABEL)?);
            }
            INTERVAL_LABEL => fields.interval = Some(read_interval(record)?),
            MAP_COUNT_LABEL => fields.declared_map_count = Some(read_map_count(record)?),
            EXPONENT_LABEL => fields.exponent = Some(read_exponent(record)?),
            HEIGHT_AXIS_LABEL => fields.shell_height_km = Some(read_shell_height_km(record)?),
            LATITUDE_AXIS_LABEL => fields.latitudes = Some(read_axis(record, LATITUDE_AXIS_LABEL)?),
            LONGITUDE_AXIS_LABEL => {
                fields.longitudes = Some(read_axis(record, LONGITUDE_AXIS_LABEL)?);
            }
            END_OF_HEADER_LABEL => return fields.into_header(),
            _unread => {}
        }
    }
    Err(ParseError::MissingHeaderEnd)
}

fn read_version(record: Record<'_>) -> Result<f64, ParseError> {
    let version: f64 = record
        .field(VERSION_FIELD)
        .and_then(|text| text.parse().ok())
        .ok_or_else(|| ParseError::NumberField {
            line_number: record.line_number,
            label: VERSION_TYPE_LABEL,
            text: record.values().trim().to_owned(),
        })?;
    if !(1.0..2.0).contains(&version) {
        return Err(ParseError::UnsupportedVersion {
            line_number: record.line_number,
            version,
        });
    }

    let file_type = record
        .values()
        .get(FILE_TYPE_COLUMN..)
        .map(str::trim)
        .unwrap_or_default();
    if !file_type.starts_with(IONOSPHERE_MAPS_TYPE) {
        return Err(ParseError::UnsupportedFileType {
            line_number: record.line_number,
            file_type: file_type.to_owned(),
        });
    }
    Ok(version)
}

fn read_integer(record: Record<'_>, label: &'static str) -> Result<i64, ParseError> {
    let text = record.values().trim();
    text.parse().map_err(|_err| ParseError::NumberField {
        line_number: record.line_number,
        label,
        text: text.to_owned(),
    })
}

/// A producer that spaces its maps unevenly declares an interval of zero.
fn read_interval(record: Record<'_>) -> Result<TimeDelta, ParseError> {
    let seconds = read_integer(record, INTERVAL_LABEL)?;
    TimeDelta::try_seconds(seconds)
        .filter(|interval| *interval >= TimeDelta::zero())
        .ok_or(ParseError::Interval {
            line_number: record.line_number,
            seconds,
        })
}

fn read_map_count(record: Record<'_>) -> Result<usize, ParseError> {
    let count = read_integer(record, MAP_COUNT_LABEL)?;
    usize::try_from(count).map_err(|_err| ParseError::NumberField {
        line_number: record.line_number,
        label: MAP_COUNT_LABEL,
        text: count.to_string(),
    })
}

fn read_exponent(record: Record<'_>) -> Result<ScalingExponent, ParseError> {
    let exponent = read_integer(record, EXPONENT_LABEL)?;
    i32::try_from(exponent)
        .ok()
        .and_then(ScalingExponent::new)
        .ok_or(ParseError::Exponent {
            line_number: record.line_number,
            exponent,
        })
}

fn read_epoch(record: Record<'_>, label: &'static str) -> Result<DateTime<Utc>, ParseError> {
    let malformed = || ParseError::Epoch {
        line_number: record.line_number,
        label,
        text: record.values().trim().to_owned(),
    };
    let mut fields = [0_i32; EPOCH_FIELD_COUNT];
    for (position, field) in fields.iter_mut().enumerate() {
        *field = record
            .epoch_field(position)
            .and_then(|text| text.parse().ok())
            .ok_or_else(malformed)?;
    }
    epoch_from_fields(fields).ok_or_else(malformed)
}

fn epoch_from_fields(fields: [i32; EPOCH_FIELD_COUNT]) -> Option<DateTime<Utc>> {
    let [year, month, day, hour, minute, second] = fields;
    Some(
        NaiveDate::from_ymd_opt(year, u32::try_from(month).ok()?, u32::try_from(day).ok()?)?
            .and_hms_opt(
                u32::try_from(hour).ok()?,
                u32::try_from(minute).ok()?,
                u32::try_from(second).ok()?,
            )?
            .and_utc(),
    )
}

fn read_grid_field(
    record: Record<'_>,
    label: &'static str,
    position: usize,
) -> Result<f64, ParseError> {
    record
        .grid_field(position)
        .and_then(|text| text.parse().ok())
        .filter(|degrees: &f64| degrees.is_finite())
        .ok_or_else(|| ParseError::NumberField {
            line_number: record.line_number,
            label,
            text: record.grid_field(position).unwrap_or_default().to_owned(),
        })
}

fn read_axis(record: Record<'_>, label: &'static str) -> Result<GridAxis, ParseError> {
    let declaration = AxisDeclaration {
        first_degrees: read_grid_field(record, label, 0)?,
        last_degrees: read_grid_field(record, label, 1)?,
        step_degrees: read_grid_field(record, label, 2)?,
    };
    GridAxis::new(declaration).map_err(|source| ParseError::Axis {
        line_number: record.line_number,
        label,
        source,
    })
}

/// The maps GeoTrace reads model the ionosphere as one shell, so the height
/// axis must hold a single node.
fn read_shell_height_km(record: Record<'_>) -> Result<f64, ParseError> {
    let first_km = read_grid_field(record, HEIGHT_AXIS_LABEL, 0)?;
    let last_km = read_grid_field(record, HEIGHT_AXIS_LABEL, 1)?;
    let step_km = read_grid_field(record, HEIGHT_AXIS_LABEL, 2)?;
    if (first_km - last_km).abs() > HEIGHT_TOLERANCE_KM || step_km.abs() > HEIGHT_TOLERANCE_KM {
        return Err(ParseError::UnsupportedHeightAxis {
            line_number: record.line_number,
            first_km,
            last_km,
            step_km,
        });
    }
    Ok(first_km)
}

fn read_maps(records: &mut Records<'_>, header: Header) -> Result<Vec<TecMap>, ParseError> {
    let mut maps = Vec::new();
    let mut exponent = header.exponent;
    while let Some(record) = records.next_record() {
        match record.label() {
            START_OF_TEC_MAP_LABEL => maps.push(read_tec_map(records, header.grid, exponent)?),
            START_OF_RMS_MAP_LABEL => skip_block(records, record, END_OF_RMS_MAP_LABEL)?,
            START_OF_HEIGHT_MAP_LABEL => skip_block(records, record, END_OF_HEIGHT_MAP_LABEL)?,
            EXPONENT_LABEL => exponent = read_exponent(record)?,
            END_OF_FILE_LABEL => break,
            _unread => {}
        }
    }
    Ok(maps)
}

fn skip_block(
    records: &mut Records<'_>,
    start: Record<'_>,
    end_label: &'static str,
) -> Result<(), ParseError> {
    while let Some(record) = records.next_record() {
        if record.label() == end_label {
            return Ok(());
        }
    }
    Err(ParseError::UnterminatedBlock {
        line_number: start.line_number,
        label: end_label,
    })
}

fn read_tec_map(
    records: &mut Records<'_>,
    grid: MapGrid,
    exponent: ScalingExponent,
) -> Result<TecMap, ParseError> {
    let epoch_record = records.next_labelled(EPOCH_OF_CURRENT_MAP_LABEL)?;
    let epoch = read_epoch(epoch_record, EPOCH_OF_CURRENT_MAP_LABEL)?;
    let mut latitude_bands = Vec::with_capacity(grid.latitudes.node_count());
    for latitude_degrees in grid.latitudes.degrees() {
        latitude_bands.push(read_latitude_band(
            records,
            grid,
            exponent,
            latitude_degrees,
        )?);
    }
    records.next_labelled(END_OF_TEC_MAP_LABEL)?;
    Ok(TecMap::new(epoch, latitude_bands))
}

/// The longitude axis and height a latitude band declares beside its own
/// latitude.
struct BandGrid {
    latitude_degrees: f64,
    longitudes: GridAxis,
    height_km: f64,
}

fn read_latitude_band(
    records: &mut Records<'_>,
    grid: MapGrid,
    exponent: ScalingExponent,
    expected_degrees: f64,
) -> Result<Vec<Option<TotalElectronContent>>, ParseError> {
    let record = records.next_labelled(LATITUDE_BAND_LABEL)?;
    let BandGrid {
        latitude_degrees,
        longitudes,
        height_km,
    } = read_band_grid(record)?;

    if (latitude_degrees - expected_degrees).abs() > DEGREES_TOLERANCE {
        return Err(ParseError::UnexpectedLatitudeBand {
            line_number: record.line_number,
            found_degrees: latitude_degrees,
            expected_degrees,
        });
    }
    if !longitudes.covers_same_nodes(grid.longitudes.axis()) {
        return Err(ParseError::BandLongitudesDiffer {
            line_number: record.line_number,
        });
    }
    if (height_km - grid.shell_height_km).abs() > HEIGHT_TOLERANCE_KM {
        return Err(ParseError::BandHeightDiffers {
            line_number: record.line_number,
            found_km: height_km,
            expected_km: grid.shell_height_km,
        });
    }

    read_band_values(records, longitudes.node_count(), exponent)
}

fn read_band_grid(record: Record<'_>) -> Result<BandGrid, ParseError> {
    let latitude_degrees = read_grid_field(record, LATITUDE_BAND_LABEL, 0)?;
    let declaration = AxisDeclaration {
        first_degrees: read_grid_field(record, LATITUDE_BAND_LABEL, 1)?,
        last_degrees: read_grid_field(record, LATITUDE_BAND_LABEL, 2)?,
        step_degrees: read_grid_field(record, LATITUDE_BAND_LABEL, 3)?,
    };
    Ok(BandGrid {
        latitude_degrees,
        longitudes: GridAxis::new(declaration).map_err(|source| ParseError::Axis {
            line_number: record.line_number,
            label: LATITUDE_BAND_LABEL,
            source,
        })?,
        height_km: read_grid_field(record, LATITUDE_BAND_LABEL, 4)?,
    })
}

fn read_band_values(
    records: &mut Records<'_>,
    node_count: usize,
    exponent: ScalingExponent,
) -> Result<Vec<Option<TotalElectronContent>>, ParseError> {
    let mut values = Vec::with_capacity(node_count);
    while values.len() < node_count {
        let record = records
            .next_record()
            .ok_or(ParseError::UnexpectedEndOfFile {
                expected: VALUE_ROW_EXPECTATION,
            })?;
        if BLOCK_LABELS.contains(&record.label()) || record.label() == LATITUDE_BAND_LABEL {
            return Err(ParseError::UnexpectedRecord {
                line_number: record.line_number,
                expected: VALUE_ROW_EXPECTATION,
                found: record.label().to_owned(),
            });
        }

        let remaining = node_count.saturating_sub(values.len());
        for position in 0..remaining.min(VALUES_PER_ROW) {
            let text = record
                .value_field(position)
                .ok_or(ParseError::TruncatedValueRow {
                    line_number: record.line_number,
                    expected_values: node_count,
                })?;
            let stored: i32 = text.parse().map_err(|_err| ParseError::ValueNotAnInteger {
                line_number: record.line_number,
                text: text.to_owned(),
            })?;
            values.push((stored != MISSING_VALUE).then(|| exponent.scale(stored)));
        }
    }
    Ok(values)
}

fn check_maps_against_header(maps: &[TecMap], header: Header) -> Result<(), ParseError> {
    if maps.len() != header.declared_map_count {
        return Err(ParseError::MapCountMismatch {
            declared: header.declared_map_count,
            found: maps.len(),
        });
    }
    for pair in maps.windows(2) {
        if let [previous, map] = pair
            && map.epoch() <= previous.epoch()
        {
            return Err(ParseError::MapEpochsOutOfOrder {
                previous: previous.epoch(),
                epoch: map.epoch(),
            });
        }
    }
    for (label, declared, found) in [
        (EPOCH_OF_FIRST_MAP_LABEL, header.first_epoch, maps.first()),
        (EPOCH_OF_LAST_MAP_LABEL, header.last_epoch, maps.last()),
    ] {
        if let Some(found) = found
            && found.epoch() != declared
        {
            return Err(ParseError::DeclaredEpochMismatch {
                label,
                declared,
                found: found.epoch(),
            });
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use gt_types::{Latitude, Longitude};

    use crate::grid::GridPoint;

    use super::*;

    /// One TEC map of the hand-written file, in the columns a published file
    /// writes them in.
    #[derive(Debug, Clone, Copy)]
    struct TecMapText {
        number: u32,
        hour: u32,
        bands: [&'static str; 2],
    }

    const FIRST_MAP: TecMapText = TecMapText {
        number: 1,
        hour: 0,
        bands: ["  100  200  100", "  300  400  300"],
    };

    const SECOND_MAP: TecMapText = TecMapText {
        number: 2,
        hour: 2,
        bands: ["  200  300  200", "  400  500  400"],
    };

    /// One record, values padded out to the label columns.
    fn record(values: &str, label: &str) -> String {
        format!("{values:<LABEL_COLUMN$}{label}")
    }

    fn epoch_record(hour: u32, label: &str) -> String {
        record(&format!("  2024     5    10{hour:>6}     0     0"), label)
    }

    /// A header declaring two maps on a grid of two latitudes and three
    /// longitudes, the last of which repeats the first meridian.
    fn header_lines() -> Vec<String> {
        vec![
            record(
                "     1.0            IONOSPHERE MAPS     GPS",
                VERSION_TYPE_LABEL,
            ),
            epoch_record(0, EPOCH_OF_FIRST_MAP_LABEL),
            epoch_record(2, EPOCH_OF_LAST_MAP_LABEL),
            record("  7200", INTERVAL_LABEL),
            record("     2", MAP_COUNT_LABEL),
            record("   450.0 450.0   0.0", HEIGHT_AXIS_LABEL),
            record("    87.5  85.0  -2.5", LATITUDE_AXIS_LABEL),
            record("  -180.0 180.0 180.0", LONGITUDE_AXIS_LABEL),
            record("    -1", EXPONENT_LABEL),
            record("", END_OF_HEADER_LABEL),
        ]
    }

    fn tec_map_lines(
        TecMapText {
            number,
            hour,
            bands,
        }: TecMapText,
    ) -> Vec<String> {
        let number = format!("{number:>6}");
        vec![
            record(&number, START_OF_TEC_MAP_LABEL),
            epoch_record(hour, EPOCH_OF_CURRENT_MAP_LABEL),
            record("    87.5-180.0 180.0 180.0 450.0", LATITUDE_BAND_LABEL),
            record(bands[0], ""),
            record("    85.0-180.0 180.0 180.0 450.0", LATITUDE_BAND_LABEL),
            record(bands[1], ""),
            record(&number, END_OF_TEC_MAP_LABEL),
        ]
    }

    fn published_lines() -> Vec<String> {
        header_lines()
            .into_iter()
            .chain(tec_map_lines(FIRST_MAP))
            .chain(tec_map_lines(SECOND_MAP))
            .chain([record("", END_OF_FILE_LABEL)])
            .collect()
    }

    fn file_text(lines: &[String]) -> String {
        lines.iter().map(|line| format!("{line}\n")).collect()
    }

    fn published_file() -> String {
        file_text(&published_lines())
    }

    fn parsed_published_file() -> GlobalIonosphereMaps {
        global_ionosphere_maps(&published_file()).unwrap()
    }

    /// Which record of the hand-written file a malformed case rewrites, and
    /// what it writes in its value columns.
    #[derive(Debug, Clone, Copy)]
    struct RecordEdit {
        label: &'static str,
        /// Position among the records carrying that label, first being zero.
        occurrence: usize,
        values: &'static str,
    }

    fn label_of(line: &str) -> &str {
        Record {
            line_number: 0,
            text: line,
        }
        .label()
    }

    fn file_with(edit: RecordEdit) -> String {
        let mut lines = published_lines();
        let position = lines
            .iter()
            .enumerate()
            .filter(|(_position, line)| label_of(line) == edit.label)
            .map(|(position, _line)| position)
            .nth(edit.occurrence)
            .unwrap_or_else(|| panic!("the file has no {:?} record to rewrite", edit.label));
        lines[position] = record(edit.values, edit.label);
        file_text(&lines)
    }

    fn without_records(label: &str) -> String {
        let lines: Vec<String> = published_lines()
            .into_iter()
            .filter(|line| label_of(line) != label)
            .collect();
        file_text(&lines)
    }

    fn epoch(hour: u32) -> DateTime<Utc> {
        NaiveDate::from_ymd_opt(2024, 5, 10)
            .and_then(|date| date.and_hms_opt(hour, 0, 0))
            .unwrap()
            .and_utc()
    }

    fn node(maps: &GlobalIonosphereMaps, map_index: usize, point: GridPoint) -> Option<f64> {
        maps.maps()
            .get(map_index)?
            .value_at(point)
            .map(TotalElectronContent::tecu)
    }

    /// The northwestern node of one map, which every band case writes first.
    fn north_west(maps: &GlobalIonosphereMaps, map_index: usize) -> Option<f64> {
        node(
            maps,
            map_index,
            GridPoint {
                latitude_index: 0,
                longitude_index: 0,
            },
        )
    }

    #[test]
    fn parses_the_published_shape() {
        let maps = parsed_published_file();
        let grid = maps.grid();
        assert_eq!(grid.latitudes.node_count(), 2);
        assert_eq!(grid.longitudes.node_count(), 3);
        assert_eq!(grid.latitudes.degrees_at(1), Some(85.0));
        assert_eq!(grid.longitudes.degrees_at(2), Some(180.0));
        assert!(
            (grid.shell_height_km - 450.0).abs() < HEIGHT_TOLERANCE_KM,
            "{} km",
            grid.shell_height_km
        );
        assert_eq!(maps.interval(), TimeDelta::hours(2));
        assert_eq!(maps.maps().len(), 2);
        assert_eq!(maps.epoch_of_first_map(), Some(epoch(0)));
        assert_eq!(maps.epoch_of_last_map(), Some(epoch(2)));
    }

    #[test]
    fn the_exponent_scales_every_stored_value() {
        let maps = parsed_published_file();
        assert_eq!(north_west(&maps, 0), Some(10.0));
        assert_eq!(
            node(
                &maps,
                1,
                GridPoint {
                    latitude_index: 1,
                    longitude_index: 1
                }
            ),
            Some(50.0)
        );
        assert_eq!(
            maps.peak_total_electron_content(),
            Some(TotalElectronContent::from_tecu(50.0))
        );
    }

    /// An exponent record between two maps rescales the ones after it.
    #[test]
    fn an_exponent_after_the_header_rescales_the_maps_that_follow() {
        let lines: Vec<String> = header_lines()
            .into_iter()
            .chain(tec_map_lines(FIRST_MAP))
            .chain([record("     0", EXPONENT_LABEL)])
            .chain(tec_map_lines(SECOND_MAP))
            .chain([record("", END_OF_FILE_LABEL)])
            .collect();
        let maps = global_ionosphere_maps(&file_text(&lines)).unwrap();
        assert_eq!(north_west(&maps, 0), Some(10.0));
        assert_eq!(north_west(&maps, 1), Some(200.0));
    }

    #[test]
    fn a_file_without_an_exponent_record_scales_by_the_ionex_default() {
        let maps = global_ionosphere_maps(&without_records(EXPONENT_LABEL)).unwrap();
        assert_eq!(north_west(&maps, 0), Some(10.0));
    }

    #[test]
    fn a_missing_value_parses_as_a_gap() {
        let maps = global_ionosphere_maps(&file_with(RecordEdit {
            label: "",
            occurrence: 0,
            values: " 9999  200  100",
        }))
        .unwrap();
        assert_eq!(north_west(&maps, 0), None);
        assert_eq!(
            maps.total_electron_content_at(Latitude::new(87.5), Longitude::new(-180.0), epoch(0)),
            None
        );
        assert_eq!(
            maps.total_electron_content_at(Latitude::new(87.5), Longitude::new(0.0), epoch(2)),
            Some(TotalElectronContent::from_tecu(30.0))
        );
    }

    /// The RMS maps a published file holds after its TEC maps.
    #[test]
    fn rms_maps_are_read_past() {
        let rms_map = [
            record("     1", START_OF_RMS_MAP_LABEL),
            epoch_record(0, EPOCH_OF_CURRENT_MAP_LABEL),
            record("    87.5-180.0 180.0 180.0 450.0", LATITUDE_BAND_LABEL),
            record("    1    1    1", ""),
            record("     1", END_OF_RMS_MAP_LABEL),
        ];
        let lines: Vec<String> = header_lines()
            .into_iter()
            .chain(tec_map_lines(FIRST_MAP))
            .chain(tec_map_lines(SECOND_MAP))
            .chain(rms_map)
            .chain([record("", END_OF_FILE_LABEL)])
            .collect();
        let maps = global_ionosphere_maps(&file_text(&lines)).unwrap();
        assert_eq!(maps.maps().len(), 2);
        assert_eq!(north_west(&maps, 1), Some(20.0));
    }

    #[test]
    fn an_rms_block_that_never_ends_is_rejected() {
        let lines: Vec<String> = header_lines()
            .into_iter()
            .chain(tec_map_lines(FIRST_MAP))
            .chain(tec_map_lines(SECOND_MAP))
            .chain([record("     1", START_OF_RMS_MAP_LABEL)])
            .collect();
        assert_eq!(
            global_ionosphere_maps(&file_text(&lines))
                .unwrap_err()
                .to_string(),
            "line 25: the block opened here has no END OF RMS MAP record"
        );
    }

    #[rstest]
    #[case::a_later_version(
        VERSION_TYPE_LABEL,
        0,
        "     2.0            IONOSPHERE MAPS     GPS",
        "line 1: IONEX version 2 is not 1.x"
    )]
    #[case::another_file_type(
        VERSION_TYPE_LABEL,
        0,
        "     1.0            OBSERVATION DATA    GPS",
        "line 1: file type \"OBSERVATION DATA    GPS\" is not the ionosphere maps type I"
    )]
    #[case::a_worded_version(
        VERSION_TYPE_LABEL,
        0,
        "  one point oh      IONOSPHERE MAPS     GPS",
        "line 1: IONEX VERSION / TYPE field \"one point oh      IONOSPHERE MAPS     GPS\" is not a number"
    )]
    #[case::a_worded_interval(
        INTERVAL_LABEL,
        0,
        "  two hours",
        "line 4: INTERVAL field \"two hours\" is not a number"
    )]
    #[case::a_three_dimensional_height_axis(
        HEIGHT_AXIS_LABEL,
        0,
        "   100.0 500.0 100.0",
        "line 6: the heights 100km to 500km in steps of 100km are not one shell"
    )]
    #[case::a_latitude_step_that_does_not_divide_the_span(
        LATITUDE_AXIS_LABEL,
        0,
        "    87.5  85.0  -3.0",
        "line 7: LAT1 / LAT2 / DLAT: 87.5 deg to 85 deg is not a whole number of -3 deg steps"
    )]
    #[case::an_exponent_beyond_the_published_range(
        EXPONENT_LABEL,
        0,
        "   -42",
        "line 9: TEC values cannot be scaled by 10^-42"
    )]
    #[case::an_epoch_that_is_not_a_date(
        EPOCH_OF_CURRENT_MAP_LABEL,
        0,
        "  2024    13    10     0     0     0",
        "line 12: EPOCH OF CURRENT MAP record \"2024    13    10     0     0     0\" is not a calendar date and time"
    )]
    #[case::a_band_on_another_longitude_axis(
        LATITUDE_BAND_LABEL,
        0,
        "    87.5-180.0 180.0  90.0 450.0",
        "line 13: the band's longitudes are not the ones the header declares"
    )]
    #[case::a_band_at_another_height(
        LATITUDE_BAND_LABEL,
        0,
        "    87.5-180.0 180.0 180.0 350.0",
        "line 13: the band's height of 350km is not the shell at 450km"
    )]
    #[case::a_band_at_another_latitude(
        LATITUDE_BAND_LABEL,
        1,
        "    82.5-180.0 180.0 180.0 450.0",
        "line 15: a band at 82.5 deg stands where the grid has 85 deg"
    )]
    #[case::a_worded_value(
        "",
        0,
        "  100  two  100",
        "line 14: TEC value \"two\" is not an integer"
    )]
    #[case::a_row_short_of_values(
        "",
        0,
        "  100  200",
        "line 14: the row holds fewer than the 3 values of its band"
    )]
    #[case::more_maps_declared_than_written(
        MAP_COUNT_LABEL,
        0,
        "     3",
        "the header declares 3 maps and the file holds 2"
    )]
    #[case::an_epoch_the_header_does_not_declare(
        EPOCH_OF_LAST_MAP_LABEL,
        0,
        "  2024     5    10     4     0     0",
        "the header declares EPOCH OF LAST MAP at 2024-05-10 04:00:00 UTC and the file holds 2024-05-10 02:00:00 UTC"
    )]
    #[case::maps_that_do_not_advance(
        EPOCH_OF_CURRENT_MAP_LABEL,
        1,
        "  2024     5    10     0     0     0",
        "the map at 2024-05-10 00:00:00 UTC does not follow the one at 2024-05-10 00:00:00 UTC"
    )]
    fn a_malformed_file_names_what_is_wrong(
        #[case] label: &'static str,
        #[case] occurrence: usize,
        #[case] values: &'static str,
        #[case] expected: &str,
    ) {
        assert_eq!(
            global_ionosphere_maps(&file_with(RecordEdit {
                label,
                occurrence,
                values
            }))
            .unwrap_err()
            .to_string(),
            expected
        );
    }

    #[rstest]
    #[case::an_empty_file("")]
    #[case::one_header_record("     1.0            IONOSPHERE MAPS     GPS")]
    fn a_file_without_a_header_end_is_rejected(#[case] text: &str) {
        assert_eq!(
            global_ionosphere_maps(text).unwrap_err().to_string(),
            "the file has no END OF HEADER record"
        );
    }

    #[rstest]
    #[case::the_latitude_axis(LATITUDE_AXIS_LABEL, "the header has no LAT1 / LAT2 / DLAT record")]
    #[case::the_longitude_axis(LONGITUDE_AXIS_LABEL, "the header has no LON1 / LON2 / DLON record")]
    #[case::the_shell_height(HEIGHT_AXIS_LABEL, "the header has no HGT1 / HGT2 / DHGT record")]
    #[case::the_map_count(MAP_COUNT_LABEL, "the header has no # OF MAPS IN FILE record")]
    #[case::the_interval(INTERVAL_LABEL, "the header has no INTERVAL record")]
    #[case::the_version(VERSION_TYPE_LABEL, "the header has no IONEX VERSION / TYPE record")]
    fn a_header_missing_a_record_names_it(#[case] label: &str, #[case] expected: &str) {
        assert_eq!(
            global_ionosphere_maps(&without_records(label))
                .unwrap_err()
                .to_string(),
            expected
        );
    }

    /// A band's values must follow its own record: a record of the next band
    /// standing there is rejected.
    #[test]
    fn a_map_missing_a_value_row_is_rejected() {
        let mut lines = published_lines();
        lines.remove(13);
        assert_eq!(
            global_ionosphere_maps(&file_text(&lines))
                .unwrap_err()
                .to_string(),
            "line 14: found \"LAT/LON1/LON2/DLON/H\" where a row of TEC values was expected"
        );
    }

    #[test]
    fn a_file_ending_inside_a_map_is_rejected() {
        let lines = published_lines();
        assert_eq!(
            global_ionosphere_maps(&file_text(&lines[..13]))
                .unwrap_err()
                .to_string(),
            "the file ends where a row of TEC values was expected"
        );
    }

    /// A header record the parser does not read leaves the file readable.
    #[test]
    fn unread_header_records_are_ignored() {
        let mut lines = published_lines();
        lines.insert(9, record("   250", "# OF STATIONS"));
        global_ionosphere_maps(&file_text(&lines)).unwrap();
    }
}
