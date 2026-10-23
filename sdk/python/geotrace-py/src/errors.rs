use geotrace_sdk::Error;
use pyo3::PyErr;
use pyo3::exceptions::{PyIOError, PyValueError};

// Filesystem failures use `OSError`. The exhaustive match classifies every content failure.
#[expect(
    clippy::needless_pass_by_value,
    reason = "map_err supplies an owned SDK error"
)]
pub(crate) fn file_error(e: Error) -> PyErr {
    let msg = e.to_string();
    match e {
        Error::Io(_) => PyIOError::new_err(msg),
        Error::Hdf5(_)
        | Error::UnsupportedVersion { .. }
        | Error::UnknownConstellation { .. }
        | Error::ShapeMismatch { .. }
        | Error::InvalidNavigationTag { .. }
        | Error::InvalidNavigationColumnType { .. }
        | Error::InvalidDatasetShape { .. }
        | Error::MissingNavigationColumn { .. }
        | Error::MissingSatelliteColumn { .. }
        | Error::MissingSatelliteGroup { .. }
        | Error::InvalidSatelliteColumnType { .. }
        | Error::InvalidSatelliteTag { .. }
        | Error::SatelliteReportWithoutTimestamp { .. }
        | Error::MixedSatelliteLayouts
        | Error::UnknownConstellationName { .. }
        | Error::UnknownMarkerIcon { .. }
        | Error::ParseError { .. }
        | Error::UnwritableField { .. }
        | Error::UnreadableField { .. }
        | Error::FixWithoutTimestamp { .. }
        | Error::ReportWithoutTimestamp { .. }
        | Error::EventMarkerWithoutTimestamp { .. }
        | Error::EmptyField { .. }
        | Error::IndexPastTable { .. }
        | Error::DebugTimeRepairTimestampOutOfRange { .. }
        | Error::TimestampOutOfRange { .. }
        | Error::TimestampCountOutOfRange { .. }
        | Error::TimestampIsTheAbsentValue { .. }
        | Error::DatasetSizePastFileLength { .. } => PyValueError::new_err(msg),
    }
}
