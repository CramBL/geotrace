use std::mem;
use std::slice;

use geotrace_sdk::{
    NavFixTime, SatelliteCoverage, SatelliteObservationReport, SatelliteScope, SatelliteUsageReport,
};

use super::GtdFileBuilder;
use crate::error::{self, GtdStatus};
use crate::navigation::TimestampArguments;
use crate::{GtdNavFixV2, GtdSatelliteObservation, GtdSatelliteReportInput, GtdSatelliteUsage};

/// Copy a navigation fix after checking every timestamp and metadata tag.
/// The input remains caller-owned. Errors leave the builder unchanged.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_add_nav_fix_v2(
    builder: *mut GtdFileBuilder,
    fix: *const GtdNavFixV2,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let builder = nonnull_mut!(builder);
        let fix = nonnull_ref!(fix);
        match fix.to_sdk() {
            Ok(fix) => {
                builder.recorder_mut().add_nav_fix(fix);
                GtdStatus::GTD_OK
            }
            Err(status) => status,
        }
    })
}

/// Copy all source rows in order after checking the complete report.
/// A null @p rows pointer is valid when @p row_count is zero.
/// Errors leave the builder unchanged. Input buffers may be released after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_add_satellite_usage_report(
    builder: *mut GtdFileBuilder,
    report: *const GtdSatelliteReportInput,
    rows: *const GtdSatelliteUsage,
    row_count: usize,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let builder = nonnull_mut!(builder);
        let report = nonnull_ref!(report);
        // SAFETY: the caller supplies `row_count` readable rows for a non-null pointer.
        let parsed = unsafe { report.parse_rows(rows, row_count, GtdSatelliteUsage::to_sdk) };
        match parsed {
            Ok(report) => {
                builder.recorder_mut().add_satellite_usage_report(
                    SatelliteUsageReport::builder()
                        .time(report.time)
                        .scope(report.scope)
                        .coverage(report.coverage)
                        .maybe_reported_count(report.reported_count)
                        .satellites(report.rows)
                        .build(),
                );
                GtdStatus::GTD_OK
            }
            Err(status) => status,
        }
    })
}

/// Copy all source rows in order after checking the complete report.
/// A null @p rows pointer is valid when @p row_count is zero.
/// Errors leave the builder unchanged. Input buffers may be released after this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_add_satellite_observation_report(
    builder: *mut GtdFileBuilder,
    report: *const GtdSatelliteReportInput,
    rows: *const GtdSatelliteObservation,
    row_count: usize,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let builder = nonnull_mut!(builder);
        let report = nonnull_ref!(report);
        // SAFETY: the caller supplies `row_count` readable rows for a non-null pointer.
        let parsed = unsafe { report.parse_rows(rows, row_count, GtdSatelliteObservation::to_sdk) };
        match parsed {
            Ok(report) => {
                builder.recorder_mut().add_satellite_observation_report(
                    SatelliteObservationReport::builder()
                        .time(report.time)
                        .scope(report.scope)
                        .coverage(report.coverage)
                        .maybe_reported_count(report.reported_count)
                        .satellites(report.rows)
                        .build(),
                );
                GtdStatus::GTD_OK
            }
            Err(status) => status,
        }
    })
}

struct ParsedSatelliteReport<Row> {
    time: NavFixTime,
    scope: SatelliteScope,
    coverage: SatelliteCoverage,
    reported_count: Option<u32>,
    rows: Vec<Row>,
}

impl GtdSatelliteReportInput {
    unsafe fn parse_rows<Input: Copy, Row>(
        self,
        rows: *const Input,
        row_count: usize,
        convert: impl Fn(Input, usize) -> Result<Row, GtdStatus>,
    ) -> Result<ParsedSatelliteReport<Row>, GtdStatus> {
        if row_count > 0 && rows.is_null() {
            error::set_last_error("rows is null with positive row_count");
            return Err(GtdStatus::GTD_ERR_NULL_ARGUMENT);
        }
        if row_count > isize::MAX as usize / mem::size_of::<Input>() {
            error::set_last_error("row_count exceeds the addressable array length");
            return Err(GtdStatus::GTD_ERR_OUT_OF_RANGE);
        }
        let time = TimestampArguments {
            gps_time: self.gps_time,
            sys_time: self.sys_time,
        }
        .into_nav_fix_time("a satellite report")?;
        let scope = self.scope.to_sdk()?;
        let coverage = self.coverage()?;
        let rows = if row_count == 0 {
            &[][..]
        } else {
            // SAFETY: size and null checks passed. The caller supplies `row_count` readable elements.
            unsafe { slice::from_raw_parts(rows, row_count) }
        };
        let rows = rows
            .iter()
            .enumerate()
            .map(|(index, row)| convert(*row, index))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(ParsedSatelliteReport {
            time,
            scope,
            coverage,
            reported_count: self.reported_count.to_opt(),
            rows,
        })
    }
}

#[cfg(test)]
mod tests;
