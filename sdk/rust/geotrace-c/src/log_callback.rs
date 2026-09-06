//! Forwarding the SDK's log records to a callback the C caller registers.

use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::OnceLock;

use log::{Level, LevelFilter, Log, Metadata, Record};
use parking_lot::Mutex;

use crate::error::{self, GtdStatus};

/// Severity of a log record, with the values of the Rust `log` crate's levels.
///
/// The SDK reports data it read or was given but could not use as written at
/// `GTD_LOG_WARN`.
#[repr(C)]
#[derive(Clone, Copy)]
pub enum GtdLogLevel {
    GTD_LOG_ERROR = 1,
    GTD_LOG_WARN = 2,
    GTD_LOG_INFO = 3,
    GTD_LOG_DEBUG = 4,
    GTD_LOG_TRACE = 5,
}

// Pin every discriminant at compile time. These are the C ABI numbers, which a
// reordering of the variants must not change.
const _: () = {
    assert!(GtdLogLevel::GTD_LOG_ERROR as u32 == 1);
    assert!(GtdLogLevel::GTD_LOG_WARN as u32 == 2);
    assert!(GtdLogLevel::GTD_LOG_INFO as u32 == 3);
    assert!(GtdLogLevel::GTD_LOG_DEBUG as u32 == 4);
    assert!(GtdLogLevel::GTD_LOG_TRACE as u32 == 5);
};

impl From<Level> for GtdLogLevel {
    fn from(level: Level) -> Self {
        match level {
            Level::Error => Self::GTD_LOG_ERROR,
            Level::Warn => Self::GTD_LOG_WARN,
            Level::Info => Self::GTD_LOG_INFO,
            Level::Debug => Self::GTD_LOG_DEBUG,
            Level::Trace => Self::GTD_LOG_TRACE,
        }
    }
}

impl From<GtdLogLevel> for LevelFilter {
    fn from(level: GtdLogLevel) -> Self {
        match level {
            GtdLogLevel::GTD_LOG_ERROR => Self::Error,
            GtdLogLevel::GTD_LOG_WARN => Self::Warn,
            GtdLogLevel::GTD_LOG_INFO => Self::Info,
            GtdLogLevel::GTD_LOG_DEBUG => Self::Debug,
            GtdLogLevel::GTD_LOG_TRACE => Self::Trace,
        }
    }
}

/// Called once per log record.
///
/// @param level     Severity of the record.
/// @param target    Module path of the code that wrote the record, NUL-terminated UTF-8.
/// @param message   Text of the record, NUL-terminated UTF-8.
/// @param user_data The pointer given to `gtd_set_log_callback()`.
pub type GtdLogCallback = Option<
    unsafe extern "C" fn(
        level: GtdLogLevel,
        target: *const c_char,
        message: *const c_char,
        user_data: *mut c_void,
    ),
>;

#[derive(Clone, Copy)]
struct CallbackSink {
    callback: unsafe extern "C" fn(GtdLogLevel, *const c_char, *const c_char, *mut c_void),
    user_data: *mut c_void,
}

// SAFETY: the callback runs on whichever thread logged the record, and any
// thread may read the caller's `user_data` out of the shared sink. The header
// states both.
unsafe impl Send for CallbackSink {}

static SINK: Mutex<Option<CallbackSink>> = Mutex::new(None);

/// The level a record must reach to be forwarded, kept across a clear of the
/// callback so that a re-registered callback runs at the level last set.
static FORWARDED_LEVEL: Mutex<LevelFilter> = Mutex::new(LevelFilter::Warn);

static LOGGER_INSTALLED: OnceLock<bool> = OnceLock::new();

static FORWARDING_LOGGER: ForwardingLogger = ForwardingLogger;

const NUL_BYTE_PLACEHOLDER: &CStr = c"(contained a null byte)";

struct ForwardingLogger;

impl Log for ForwardingLogger {
    fn enabled(&self, metadata: &Metadata<'_>) -> bool {
        metadata.level() <= *FORWARDED_LEVEL.lock() && SINK.lock().is_some()
    }

    fn log(&self, record: &Record<'_>) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // The lock is free again before the callback runs. A callback that logs
        // would otherwise wait on itself.
        let sink = *SINK.lock();
        let Some(sink) = sink else {
            return;
        };
        let target =
            CString::new(record.target()).unwrap_or_else(|_| NUL_BYTE_PLACEHOLDER.to_owned());
        let message = CString::new(record.args().to_string())
            .unwrap_or_else(|_| NUL_BYTE_PLACEHOLDER.to_owned());
        // SAFETY: both strings live until the end of this call, which is as long
        // as the header promises them to the callback.
        unsafe {
            (sink.callback)(
                GtdLogLevel::from(record.level()),
                target.as_ptr(),
                message.as_ptr(),
                sink.user_data,
            );
        }
    }

    fn flush(&self) {}
}

fn install_forwarding_logger() -> bool {
    *LOGGER_INSTALLED.get_or_init(|| log::set_logger(&FORWARDING_LOGGER).is_ok())
}

/// Set the `log` crate's maximum level, which decides what reaches
/// [`ForwardingLogger`] at all. Only this crate's own logger may set it.
fn apply_max_level(level: LevelFilter) {
    if LOGGER_INSTALLED.get() == Some(&true) {
        log::set_max_level(level);
    }
}

fn clear_sink() {
    *SINK.lock() = None;
    apply_max_level(LevelFilter::Off);
}

/// Register @p callback as the destination for the SDK's log records.
///
/// The SDK reports what it did with data it could not use as written: a
/// satellite SNR of 99 dB-Hz, an unknown travel mode, an unrecognized icon.
/// Without a callback none of those records reach the caller.
///
/// Records of `GTD_LOG_WARN` and above reach the callback until
/// `gtd_set_log_level()` says otherwise. A record below the level costs nothing:
/// the SDK never formats it.
///
/// The callback runs on the thread that wrote the record, and its @p target and
/// @p message pointers are valid only for the duration of the call. Copy what
/// the callback keeps.
///
/// A second call replaces the callback and its user data. A NULL @p callback
/// clears it, as `gtd_clear_log_callback()` does.
///
/// @param callback  Function to call per record, or NULL to stop forwarding.
/// @param user_data Passed to every call. The SDK stores it and never reads it.
///
/// @return `GTD_ERR_INTERNAL` if the SDK could not install its log sink, in
///         which case the callback receives no records.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_set_log_callback(
    callback: GtdLogCallback,
    user_data: *mut c_void,
) -> GtdStatus {
    error::run_catching_panics(|| {
        let Some(callback) = callback else {
            clear_sink();
            return GtdStatus::GTD_OK;
        };
        if !install_forwarding_logger() {
            error::set_last_error("the sdk could not install its log sink");
            return GtdStatus::GTD_ERR_INTERNAL;
        }
        *SINK.lock() = Some(CallbackSink {
            callback,
            user_data,
        });
        apply_max_level(*FORWARDED_LEVEL.lock());
        GtdStatus::GTD_OK
    })
}

/// Forward records of @p level and above, dropping the rest.
///
/// The level holds until the next call, a clear of the callback included, and
/// is `GTD_LOG_WARN` until this is called.
///
/// @param level Lowest severity to forward.
#[unsafe(no_mangle)]
pub extern "C" fn gtd_set_log_level(level: GtdLogLevel) {
    let level = LevelFilter::from(level);
    *FORWARDED_LEVEL.lock() = level;
    if SINK.lock().is_some() {
        apply_max_level(level);
    }
}

/// Stop forwarding log records.
///
/// Keep the user data alive as long as another thread may still write a record:
/// a callback already running when this call clears it runs to completion.
#[unsafe(no_mangle)]
pub extern "C" fn gtd_clear_log_callback() {
    clear_sink();
}
