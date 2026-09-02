//! The `timestamps` group of `geotrace.h`: the timestamp type and its constructors.

/// Timestamp: UTC Unix epoch in microseconds.
/// Use `gtd_ts_none()` for absent timestamps.
#[repr(C)]
#[derive(Clone, Copy)]
pub struct GtdTimestamp {
    pub unix_micros: i64,
}

/// Sentinel value for an absent timestamp.
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

#[unsafe(no_mangle)]
pub extern "C" fn gtd_ts_from_seconds(secs: u64) -> GtdTimestamp {
    GtdTimestamp {
        unix_micros: (secs as i64).saturating_mul(1_000_000),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gtd_ts_from_millis(ms: u64) -> GtdTimestamp {
    GtdTimestamp {
        unix_micros: (ms as i64).saturating_mul(1_000),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gtd_ts_from_micros(us: u64) -> GtdTimestamp {
    GtdTimestamp {
        unix_micros: us as i64,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gtd_ts_from_nanos(ns: u64) -> GtdTimestamp {
    GtdTimestamp {
        unix_micros: (ns / 1_000) as i64,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gtd_ts_none() -> GtdTimestamp {
    GtdTimestamp {
        unix_micros: TS_NONE_SENTINEL,
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn gtd_ts_is_none(ts: GtdTimestamp) -> u8 {
    u8::from(ts.unix_micros == TS_NONE_SENTINEL)
}
