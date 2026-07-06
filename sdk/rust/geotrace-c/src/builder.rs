use std::ffi::c_char;

use geotrace_sdk::{
    Angle, Annotation, BuildError, EventMarker, EventMarkerColor, EventMarkerStyle, NavFileBuilder,
    NavRecorder, SatelliteReport, Velocity,
};

use crate::error::{GtdStatus, run_catching_panics, set_last_error};
use crate::{GtdMarkerIcon, GtdNavFile, GtdOptF64, GtdSatellite, GtdTimestamp, ts_to_datetime};

/// Opaque handle for a file-under-construction.
///
/// Created by `gtd_builder_create()`. Freed either by `gtd_builder_destroy()`
/// (on error paths) or consumed by `gtd_builder_finish()` (on success).
pub struct GtdFileBuilder {
    builder: Option<NavFileBuilder>,
    recorder: Option<NavRecorder>,
}

impl GtdFileBuilder {
    fn new() -> Self {
        Self {
            builder: Some(NavFileBuilder::new()),
            recorder: None,
        }
    }

    /// Transition from configuring to open, lazily on first data add.
    fn ensure_open(&mut self) {
        if self.recorder.is_none() {
            let b = self.builder.take().unwrap_or_default();
            self.recorder = Some(b.open());
        }
    }

    #[expect(clippy::panic, reason = "ensure_open guarantees recorder is Some")]
    fn recorder_mut(&mut self) -> &mut NavRecorder {
        self.ensure_open();
        match &mut self.recorder {
            Some(s) => s,
            None => panic!("geotrace-c: recorder is None after ensure_open - this is a bug"),
        }
    }

    pub(crate) fn into_recorder(mut self) -> NavRecorder {
        self.ensure_open();
        match self.recorder {
            Some(s) => s,
            None => NavFileBuilder::new().open(),
        }
    }

    fn set_title(&mut self, title: &str) -> GtdStatus {
        match self.builder.take() {
            Some(b) => {
                self.builder = Some(b.with_title(title));
                GtdStatus::Ok
            }
            None => {
                set_last_error("metadata must be set before adding data");
                GtdStatus::ErrInternal
            }
        }
    }

    fn set_device(&mut self, device: &str) -> GtdStatus {
        match self.builder.take() {
            Some(b) => {
                self.builder = Some(b.with_device(device));
                GtdStatus::Ok
            }
            None => {
                set_last_error("metadata must be set before adding data");
                GtdStatus::ErrInternal
            }
        }
    }

    fn set_notes(&mut self, notes: &str) -> GtdStatus {
        match self.builder.take() {
            Some(b) => {
                self.builder = Some(b.with_notes(notes));
                GtdStatus::Ok
            }
            None => {
                set_last_error("metadata must be set before adding data");
                GtdStatus::ErrInternal
            }
        }
    }

    fn set_identity(&mut self, identity: &str) -> GtdStatus {
        match self.builder.take() {
            Some(b) => {
                self.builder = Some(b.with_identity(identity));
                GtdStatus::Ok
            }
            None => {
                set_last_error("metadata must be set before adding data");
                GtdStatus::ErrInternal
            }
        }
    }

    fn set_lenient(&mut self) {
        match self.builder.take() {
            Some(b) => {
                self.builder = Some(b.with_lenient_errors());
            }
            None => {
                set_last_error("lenient mode must be set before adding data");
            }
        }
    }
}

// ── Lifecycle ───────────────────────────────────────────────────────────────── // [qa-allow-check-em-dash, qa-allow-check-floating-comments, reason = "C API section headers are an established convention in this FFI file"]

#[unsafe(no_mangle)]
pub extern "C" fn gtd_builder_create() -> *mut GtdFileBuilder {
    Box::into_raw(Box::new(GtdFileBuilder::new()))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_destroy(b: *mut GtdFileBuilder) {
    if b.is_null() {
        return;
    }
    // SAFETY: b was allocated by gtd_builder_create via Box::into_raw
    unsafe { drop(Box::from_raw(b)) };
}

// ── Metadata setters ────────────────────────────────────────────────────────── // [qa-allow-check-em-dash, qa-allow-check-floating-comments, reason = "C API section headers are an established convention in this FFI file"]

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_set_title(
    b: *mut GtdFileBuilder,
    title: *const c_char,
) -> GtdStatus {
    run_catching_panics(|| {
        let b = nonnull_mut!(b);
        let s = cstr!(title);
        b.set_title(s)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_set_device(
    b: *mut GtdFileBuilder,
    device: *const c_char,
) -> GtdStatus {
    run_catching_panics(|| {
        let b = nonnull_mut!(b);
        let s = cstr!(device);
        b.set_device(s)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_set_notes(
    b: *mut GtdFileBuilder,
    notes: *const c_char,
) -> GtdStatus {
    run_catching_panics(|| {
        let b = nonnull_mut!(b);
        let s = cstr!(notes);
        b.set_notes(s)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_set_identity(
    b: *mut GtdFileBuilder,
    identity: *const c_char,
) -> GtdStatus {
    run_catching_panics(|| {
        let b = nonnull_mut!(b);
        let s = cstr!(identity);
        b.set_identity(s)
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_set_lenient(b: *mut GtdFileBuilder) {
    if b.is_null() {
        set_last_error("null pointer argument");
        return;
    }
    // SAFETY: b is non-null and valid for the call duration
    unsafe { (*b).set_lenient() };
}

// ── Data ingestion ──────────────────────────────────────────────────────────── // [qa-allow-check-em-dash, qa-allow-check-floating-comments, reason = "C API section headers are an established convention in this FFI file"]

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_add_nav_fix(
    b: *mut GtdFileBuilder,
    gps_time: GtdTimestamp,
    sys_time: GtdTimestamp,
    lat_deg: f64,
    lon_deg: f64,
    heading_deg: GtdOptF64,
    speed_mps: GtdOptF64,
    eph_m: GtdOptF64,
) -> GtdStatus {
    run_catching_panics(|| {
        let b = nonnull_mut!(b);
        b.recorder_mut().add_nav_fix(geotrace_sdk::NavFix {
            gps_time: ts_to_datetime(gps_time),
            sys_time: ts_to_datetime(sys_time),
            lat: Angle::degrees(lat_deg),
            lon: Angle::degrees(lon_deg),
            heading: heading_deg.to_opt().map(Angle::degrees),
            speed: speed_mps.to_opt().map(Velocity::meter_per_second),
            eph_m: eph_m.to_opt(),
        });
        GtdStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_add_satellite_report(
    b: *mut GtdFileBuilder,
    gps_time: GtdTimestamp,
    sys_time: GtdTimestamp,
    sats: *const GtdSatellite,
    n_sats: usize,
) -> GtdStatus {
    run_catching_panics(|| {
        let b = nonnull_mut!(b);
        if n_sats > 0 && sats.is_null() {
            set_last_error("sats is null but n_sats > 0");
            return GtdStatus::ErrNullArgument;
        }
        let tracked: Vec<geotrace_sdk::Satellite> = if n_sats == 0 {
            Vec::new()
        } else {
            // SAFETY: sats is non-null (checked above), n_sats is the element count
            let slice = unsafe { std::slice::from_raw_parts(sats, n_sats) };
            slice.iter().map(|s| s.to_sdk_satellite()).collect()
        };
        b.recorder_mut().add_satellite_report(SatelliteReport {
            gps_time: ts_to_datetime(gps_time),
            sys_time: ts_to_datetime(sys_time),
            tracked,
        });
        GtdStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_add_annotation(
    b: *mut GtdFileBuilder,
    time: GtdTimestamp,
    label: *const c_char,
    icon: GtdMarkerIcon,
) -> GtdStatus {
    run_catching_panics(|| {
        let b = nonnull_mut!(b);
        let Some(ann_time) = ts_to_datetime(time) else {
            set_last_error("annotation time must not be gtd_ts_none()");
            return GtdStatus::ErrNullArgument;
        };
        let label_str = cstr_opt!(label).map(str::to_owned);
        b.recorder_mut().add_annotation(Annotation {
            time: ann_time,
            label: label_str,
            icon: icon.to_marker_icon(),
        });
        GtdStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_add_event_marker(
    b: *mut GtdFileBuilder,
    variant_path: *const c_char,
    sys_time: GtdTimestamp,
    annotation: *const c_char,
) -> GtdStatus {
    run_catching_panics(|| {
        let b = nonnull_mut!(b);
        let path = cstr!(variant_path);
        let ann = cstr_opt!(annotation).map(str::to_owned);
        let Some(dt) = ts_to_datetime(sys_time) else {
            set_last_error("event marker sys_time must not be gtd_ts_none()");
            return GtdStatus::ErrNullArgument;
        };
        let marker = match EventMarker::builder()
            .variant_path(path)
            .sys_time(dt)
            .maybe_annotation(ann)
            .build()
        {
            Ok(m) => m,
            Err(e) => {
                set_last_error(e);
                return GtdStatus::ErrInvalidPath;
            }
        };
        b.recorder_mut().add_event_marker(marker);
        GtdStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_add_event_marker_style(
    b: *mut GtdFileBuilder,
    variant_path: *const c_char,
    icon: GtdMarkerIcon,
    color_hex: *const c_char,
) -> GtdStatus {
    run_catching_panics(|| {
        let b = nonnull_mut!(b);
        let path = cstr!(variant_path);
        let icon_choice = icon.to_icon_choice();
        let color = match cstr_opt!(color_hex) {
            Some(hex) => EventMarkerColor::Hex(hex.to_owned()),
            None => EventMarkerColor::Auto,
        };
        b.recorder_mut().add_event_marker_style(EventMarkerStyle {
            variant_path: path.to_owned(),
            icon: icon_choice,
            color,
        });
        GtdStatus::Ok
    })
}

// ── Finalisation ────────────────────────────────────────────────────────────── // [qa-allow-check-em-dash, qa-allow-check-floating-comments, reason = "C API section headers are an established convention in this FFI file"]

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_builder_finish(
    b: *mut GtdFileBuilder,
    out: *mut *mut GtdNavFile,
) -> GtdStatus {
    run_catching_panics(|| {
        if b.is_null() {
            set_last_error("null pointer argument (b)");
            return GtdStatus::ErrNullArgument;
        }
        if out.is_null() {
            set_last_error("null pointer argument (out)");
            return GtdStatus::ErrNullArgument;
        }
        // SAFETY: b is non-null, was created by gtd_builder_create via Box::into_raw
        let b_box = unsafe { Box::from_raw(b) };
        // SAFETY: out is non-null (checked above)
        let out_ref = unsafe { &mut *out };
        *out_ref = std::ptr::null_mut();

        let recorder = b_box.into_recorder();

        match recorder.finish() {
            Ok(nav_file) => {
                let handle = Box::new(GtdNavFile::from_nav_file(nav_file));
                *out_ref = Box::into_raw(handle);
                GtdStatus::Ok
            }
            Err(BuildError::NoNavFixes) => {
                set_last_error("no nav fixes were added; at least one is required");
                GtdStatus::ErrNoNavFixes
            }
            Err(BuildError::AnnotationsOutsideRange { count }) => {
                set_last_error(format!(
                    "{count} annotation(s) fall outside the nav fix time range"
                ));
                GtdStatus::ErrAnnotationsOob
            }
            // The C API has no way to add channels yet, so this is unreachable
            // through it; handle it exhaustively until the channel parity PR
            // gives it a dedicated status.
            Err(BuildError::DuplicateChannelName { name }) => {
                set_last_error(format!("two channels share the name {name:?}"));
                GtdStatus::ErrInternal
            }
        }
    })
}
