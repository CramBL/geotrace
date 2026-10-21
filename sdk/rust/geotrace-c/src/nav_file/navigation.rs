use geotrace_sdk::{NavPoint, SatelliteObservationReport, SatelliteUsageReport};

use super::GtdNavFile;
use crate::error::{self, GtdStatus};
use crate::satellite_evidence::RecordedSatelliteReportMetadata;
use crate::{GtdNavFixV2, GtdSatelliteObservation, GtdSatelliteReportInfo, GtdSatelliteUsage};

/// Caller-owned navigation facts and canonical report counts.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdNavPointInfoV2 {
    /// Recorded navigation facts and coordinate origin.
    pub fix: GtdNavFixV2,
    /// Number of canonical usage samples associated with this point.
    pub usage_report_count: usize,
    /// Number of canonical observation samples associated with this point.
    pub observation_report_count: usize,
}

#[derive(Clone, Copy)]
struct SatelliteReportIndex {
    nav_point_index: usize,
    report_index: usize,
}

impl GtdNavFile {
    fn nav_point(&self, index: usize) -> Result<&NavPoint, GtdStatus> {
        self.file.nav_points().get(index).ok_or_else(|| {
            error::set_last_error(format!("nav point index {index} is out of range"));
            GtdStatus::GTD_ERR_OUT_OF_RANGE
        })
    }

    fn usage_report(
        &self,
        index: SatelliteReportIndex,
    ) -> Result<&SatelliteUsageReport, GtdStatus> {
        self.nav_point(index.nav_point_index)?
            .satellite_evidence()
            .usage
            .get(index.report_index)
            .ok_or_else(|| {
                error::set_last_error(format!(
                    "usage report index {} is out of range",
                    index.report_index
                ));
                GtdStatus::GTD_ERR_OUT_OF_RANGE
            })
    }

    fn observation_report(
        &self,
        index: SatelliteReportIndex,
    ) -> Result<&SatelliteObservationReport, GtdStatus> {
        self.nav_point(index.nav_point_index)?
            .satellite_evidence()
            .observations
            .get(index.report_index)
            .ok_or_else(|| {
                error::set_last_error(format!(
                    "observation report index {} is out of range",
                    index.report_index
                ));
                GtdStatus::GTD_ERR_OUT_OF_RANGE
            })
    }
}

/// Fill caller-owned @p out on success. Errors leave @p out unchanged.
/// Canonical counts include every independent sample even without a legacy projection.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_nav_point_v2(
    file: *const GtdNavFile,
    index: usize,
    out: *mut GtdNavPointInfoV2,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let file = nonnull_ref!(file);
        let out = nonnull_mut!(out);
        match file.nav_point(index) {
            Ok(point) => {
                *out = GtdNavPointInfoV2 {
                    fix: point.fix.into(),
                    usage_report_count: point.satellite_evidence().usage.len(),
                    observation_report_count: point.satellite_evidence().observations.len(),
                };
                GtdStatus::GTD_OK
            }
            Err(status) => status,
        }
    })
}

/// Return one canonical usage sample's metadata in caller-owned @p out.
/// Errors leave @p out unchanged and identify the invalid navigation or report index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_satellite_usage_report(
    file: *const GtdNavFile,
    nav_point_index: usize,
    report_index: usize,
    out: *mut GtdSatelliteReportInfo,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let file = nonnull_ref!(file);
        let out = nonnull_mut!(out);
        let result = (|| {
            let report = file.usage_report(SatelliteReportIndex {
                nav_point_index,
                report_index,
            })?;
            GtdSatelliteReportInfo::try_from(RecordedSatelliteReportMetadata {
                time: report.time,
                scope: &report.scope,
                coverage: report.coverage,
                reported_count: report.reported_count,
                row_count: report.satellites.len(),
                placement: report.placement,
            })
        })();
        match result {
            Ok(info) => {
                *out = info;
                GtdStatus::GTD_OK
            }
            Err(status) => status,
        }
    })
}

/// Copy one ordered source row into caller-owned @p out.
/// Errors leave @p out unchanged and identify the invalid navigation, report or row index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_satellite_usage(
    file: *const GtdNavFile,
    nav_point_index: usize,
    report_index: usize,
    row_index: usize,
    out: *mut GtdSatelliteUsage,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let file = nonnull_ref!(file);
        let out = nonnull_mut!(out);
        let result = (|| {
            let report = file.usage_report(SatelliteReportIndex {
                nav_point_index,
                report_index,
            })?;
            let row = report.satellites.get(row_index).ok_or_else(|| {
                error::set_last_error(format!("usage row index {row_index} is out of range"));
                GtdStatus::GTD_ERR_OUT_OF_RANGE
            })?;
            GtdSatelliteUsage::try_from(row)
        })();
        match result {
            Ok(row) => {
                *out = row;
                GtdStatus::GTD_OK
            }
            Err(status) => status,
        }
    })
}

/// Return one canonical observation sample's metadata in caller-owned @p out.
/// Errors leave @p out unchanged and identify the invalid navigation or report index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_satellite_observation_report(
    file: *const GtdNavFile,
    nav_point_index: usize,
    report_index: usize,
    out: *mut GtdSatelliteReportInfo,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let file = nonnull_ref!(file);
        let out = nonnull_mut!(out);
        let result = (|| {
            let report = file.observation_report(SatelliteReportIndex {
                nav_point_index,
                report_index,
            })?;
            GtdSatelliteReportInfo::try_from(RecordedSatelliteReportMetadata {
                time: report.time,
                scope: &report.scope,
                coverage: report.coverage,
                reported_count: report.reported_count,
                row_count: report.satellites.len(),
                placement: report.placement,
            })
        })();
        match result {
            Ok(info) => {
                *out = info;
                GtdStatus::GTD_OK
            }
            Err(status) => status,
        }
    })
}

/// Copy one ordered source row into caller-owned @p out.
/// Errors leave @p out unchanged and identify the invalid navigation, report or row index.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_satellite_observation(
    file: *const GtdNavFile,
    nav_point_index: usize,
    report_index: usize,
    row_index: usize,
    out: *mut GtdSatelliteObservation,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let file = nonnull_ref!(file);
        let out = nonnull_mut!(out);
        let result = (|| {
            let report = file.observation_report(SatelliteReportIndex {
                nav_point_index,
                report_index,
            })?;
            let row = report.satellites.get(row_index).ok_or_else(|| {
                error::set_last_error(format!("observation row index {row_index} is out of range"));
                GtdStatus::GTD_ERR_OUT_OF_RANGE
            })?;
            Ok::<_, GtdStatus>(GtdSatelliteObservation::from(row))
        })();
        match result {
            Ok(row) => {
                *out = row;
                GtdStatus::GTD_OK
            }
            Err(status) => status,
        }
    })
}

#[cfg(test)]
mod tests;
