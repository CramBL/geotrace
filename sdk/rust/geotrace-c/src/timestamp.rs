//! The timestamp type and its constructors.

use crate::error::{self, GtdStatus};

/// UTC Unix epoch timestamp in microseconds.
///
/// Use `gtd_ts_none()` to represent an absent timestamp.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdTimestamp {
    pub unix_micros: i64,
}

/// The `unix_micros` value that marks an absent timestamp.
const TS_NONE_SENTINEL: i64 = i64::MIN;

pub(crate) fn ts_from_datetime(dt: chrono::DateTime<chrono::Utc>) -> GtdTimestamp {
    GtdTimestamp {
        unix_micros: dt.timestamp_micros(),
    }
}

pub(crate) fn ts_to_datetime(ts: GtdTimestamp) -> Option<chrono::DateTime<chrono::Utc>> {
    if ts.unix_micros == TS_NONE_SENTINEL {
        None
    } else {
        chrono::DateTime::from_timestamp_micros(ts.unix_micros)
    }
}

fn write_converted_timestamp(
    converted: Result<geotrace_sdk::Timestamp, geotrace_sdk::Error>,
    out: &mut GtdTimestamp,
) -> GtdStatus {
    match converted {
        Ok(timestamp) => {
            *out = ts_from_datetime(timestamp.into());
            GtdStatus::GTD_OK
        }
        Err(e) => {
            let status = error::status_for_error(&e);
            error::set_last_error(e);
            status
        }
    }
}

/// Construct a timestamp from whole seconds since the Unix epoch.
///
/// @param seconds Seconds since the Unix epoch, negative before it.
/// @param out     Caller-allocated result, written on success.
///
/// @return `GTD_ERR_OUT_OF_RANGE` if @p seconds is past the range a timestamp
///         covers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_ts_from_seconds(seconds: i64, out: *mut GtdTimestamp) -> GtdStatus {
    error::run_catching_panics(|| {
        let out = nonnull_mut!(out);
        write_converted_timestamp(geotrace_sdk::Timestamp::try_from_unix_seconds(seconds), out)
    })
}

/// Construct a timestamp from milliseconds since the Unix epoch.
///
/// @param millis Milliseconds since the Unix epoch, negative before it.
/// @param out    Caller-allocated result, written on success.
///
/// @return `GTD_ERR_OUT_OF_RANGE` if @p millis is past the range a timestamp
///         covers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_ts_from_millis(millis: i64, out: *mut GtdTimestamp) -> GtdStatus {
    error::run_catching_panics(|| {
        let out = nonnull_mut!(out);
        write_converted_timestamp(geotrace_sdk::Timestamp::try_from_unix_millis(millis), out)
    })
}

/// Construct a timestamp from microseconds since the Unix epoch.
///
/// @param micros Microseconds since the Unix epoch, negative before it.
/// @param out    Caller-allocated result, written on success.
///
/// @return `GTD_ERR_OUT_OF_RANGE` if @p micros is past the range a timestamp
///         covers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_ts_from_micros(micros: i64, out: *mut GtdTimestamp) -> GtdStatus {
    error::run_catching_panics(|| {
        let out = nonnull_mut!(out);
        write_converted_timestamp(geotrace_sdk::Timestamp::try_from_unix_micros(micros), out)
    })
}

/// Construct a timestamp from nanoseconds since the Unix epoch, truncated
/// towards zero to whole microseconds.
///
/// @param nanos Nanoseconds since the Unix epoch, negative before it.
/// @param out   Caller-allocated result, written on success.
///
/// @return `GTD_ERR_OUT_OF_RANGE` if @p nanos is past the range a timestamp
///         covers.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_ts_from_nanos(nanos: i64, out: *mut GtdTimestamp) -> GtdStatus {
    error::run_catching_panics(|| {
        let out = nonnull_mut!(out);
        write_converted_timestamp(geotrace_sdk::Timestamp::try_from_unix_nanos(nanos), out)
    })
}

/// The timestamp value that represents an absent timestamp.
#[unsafe(no_mangle)]
pub extern "C" fn gtd_ts_none() -> GtdTimestamp {
    GtdTimestamp {
        unix_micros: TS_NONE_SENTINEL,
    }
}

/// Returns non-zero if @p timestamp is the absent timestamp.
#[unsafe(no_mangle)]
pub extern "C" fn gtd_ts_is_none(timestamp: GtdTimestamp) -> u8 {
    u8::from(timestamp.unix_micros == TS_NONE_SENTINEL)
}
