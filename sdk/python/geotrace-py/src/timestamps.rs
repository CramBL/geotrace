use chrono::{DateTime, FixedOffset};
use geotrace_sdk::{NavFixTime, RecordedFixTimestamps};
use pyo3::exceptions::PyValueError;
use pyo3::prelude::*;

/// The `gps_time` and `sys_time` keyword arguments of one Python constructor.
/// Each clock has its own field, so a call site cannot transpose the two.
pub(crate) struct TimestampArguments {
    pub(crate) gps_time: Option<DateTime<FixedOffset>>,
    pub(crate) sys_time: Option<DateTime<FixedOffset>>,
}

pub(crate) fn nav_fix_time_or_value_error(
    TimestampArguments { gps_time, sys_time }: TimestampArguments,
) -> PyResult<NavFixTime> {
    let recorded = RecordedFixTimestamps {
        gps: gps_time.map(|t| t.to_utc()),
        sys: sys_time.map(|t| t.to_utc()),
    };
    NavFixTime::from_recorded(recorded)
        .ok_or_else(|| PyValueError::new_err("provide gps_time or sys_time"))
}
