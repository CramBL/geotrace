use std::ffi::{CString, c_char};
use std::io::Cursor;

use geotrace_sdk::NavFile;

use crate::error::{GtdStatus, run_catching_panics, set_last_error, status_for_error};
use crate::{
    GtdChannelInfo, GtdConstellation, GtdEventMarkerInfo, GtdNavPointInfo, GtdSatInfo,
    GtdTimestamp, fill_c_str, opt_f64_none, opt_f64_some, ts_from_datetime,
};

/// Opaque handle for a parsed or freshly-built GeoTrace nav file.
pub struct GtdNavFile {
    file: NavFile,
    title: Option<CString>,
    device: Option<CString>,
    notes: Option<CString>,
    identity: Option<CString>,
}

impl GtdNavFile {
    pub(crate) fn from_nav_file(file: NavFile) -> Self {
        let to_cstring = |s: &str| CString::new(s).ok();
        Self {
            title: file.meta().title.as_deref().and_then(to_cstring),
            device: file.meta().device.as_deref().and_then(to_cstring),
            notes: file.meta().notes.as_deref().and_then(to_cstring),
            identity: file.meta().identity.as_deref().and_then(to_cstring),
            file,
        }
    }
}

// ── Write-path output functions ─────────────────────────────────────────────── // [qa-allow-check-em-dash, qa-allow-check-floating-comments, reason = "C API section headers are an established convention in this FFI file"]

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_write_to_path(
    f: *const GtdNavFile,
    path: *const c_char,
) -> GtdStatus {
    run_catching_panics(|| {
        let f = nonnull_ref!(f);
        let path_str = cstr!(path);
        match f.file.write_to_file(path_str) {
            Ok(()) => GtdStatus::Ok,
            Err(e) => {
                set_last_error(&e);
                status_for_error(&e)
            }
        }
    })
}

/// Serialises the file to a heap buffer. The buffer must be freed with
/// `gtd_free_bytes(buf, len)`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_to_bytes(
    f: *const GtdNavFile,
    buf: *mut *mut u8,
    len: *mut usize,
) -> GtdStatus {
    run_catching_panics(|| {
        let f = nonnull_ref!(f);
        let buf_out = nonnull_mut!(buf);
        let len_out = nonnull_mut!(len);

        let mut bytes: Vec<u8> = Vec::new();
        if let Err(e) = f.file.write(&mut bytes) {
            set_last_error(&e);
            return status_for_error(&e);
        }

        let mut boxed = bytes.into_boxed_slice();
        *len_out = boxed.len();
        *buf_out = boxed.as_mut_ptr();
        // Transfer ownership to the C caller. gtd_free_bytes reconstructs the Box.
        #[expect(
            clippy::mem_forget,
            reason = "intentionally leaking Box<[u8]> to transfer ownership to the C caller"
        )]
        std::mem::forget(boxed);
        GtdStatus::Ok
    })
}

/// Frees a buffer returned by `gtd_nav_file_to_bytes`.
/// `buf` and `len` must match the values written by `gtd_nav_file_to_bytes`.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_free_bytes(buf: *mut u8, len: usize) {
    if buf.is_null() {
        return;
    }
    let slice = std::ptr::slice_from_raw_parts_mut(buf, len);
    // SAFETY: slice reconstructs the Box<[u8]> allocated by gtd_nav_file_to_bytes
    unsafe { drop(Box::from_raw(slice)) };
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_destroy(f: *mut GtdNavFile) {
    if f.is_null() {
        return;
    }
    // SAFETY: f was allocated by gtd_builder_finish or gtd_nav_file_open via Box::into_raw
    unsafe { drop(Box::from_raw(f)) };
}

// ── Read-path constructors ──────────────────────────────────────────────────── // [qa-allow-check-em-dash, qa-allow-check-floating-comments, reason = "C API section headers are an established convention in this FFI file"]

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_open(
    path: *const c_char,
    out: *mut *mut GtdNavFile,
) -> GtdStatus {
    run_catching_panics(|| {
        let path_str = cstr!(path);
        let out_ref = nonnull_mut!(out);
        *out_ref = std::ptr::null_mut();

        match NavFile::open(path_str) {
            Ok(file) => {
                *out_ref = Box::into_raw(Box::new(GtdNavFile::from_nav_file(file)));
                GtdStatus::Ok
            }
            Err(e) => {
                set_last_error(&e);
                status_for_error(&e)
            }
        }
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_from_bytes(
    data: *const u8,
    len: usize,
    out: *mut *mut GtdNavFile,
) -> GtdStatus {
    run_catching_panics(|| {
        let out_ref = nonnull_mut!(out);
        *out_ref = std::ptr::null_mut();
        if data.is_null() && len > 0 {
            set_last_error("data is null but len > 0");
            return GtdStatus::ErrNullArgument;
        }

        let slice = if len == 0 {
            &[][..]
        } else {
            // SAFETY: data is non-null (checked above), len is the byte count
            unsafe { std::slice::from_raw_parts(data, len) }
        };

        match NavFile::read(Cursor::new(slice)) {
            Ok(file) => {
                *out_ref = Box::into_raw(Box::new(GtdNavFile::from_nav_file(file)));
                GtdStatus::Ok
            }
            Err(e) => {
                set_last_error(&e);
                status_for_error(&e)
            }
        }
    })
}

// ── Nav point accessors ─────────────────────────────────────────────────────── // [qa-allow-check-em-dash, qa-allow-check-floating-comments, reason = "C API section headers are an established convention in this FFI file"]

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_nav_point_count(f: *const GtdNavFile) -> usize {
    if f.is_null() {
        return 0;
    }
    // SAFETY: f is non-null (checked above)
    unsafe { (*f).file.nav_points().len() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_nav_point(
    f: *const GtdNavFile,
    idx: usize,
    out: *mut GtdNavPointInfo,
) -> GtdStatus {
    run_catching_panics(|| {
        let f = nonnull_ref!(f);
        let out = nonnull_mut!(out);

        let Some(point) = f.file.nav_points().get(idx) else {
            set_last_error(format!("nav point index {idx} is out of range"));
            return GtdStatus::ErrNullArgument;
        };

        out.gps_time = point
            .fix
            .gps_time
            .map_or(crate::gtd_ts_none(), ts_from_datetime);
        out.sys_time = point
            .fix
            .sys_time
            .map_or(crate::gtd_ts_none(), ts_from_datetime);
        out.lat_deg = point.fix.lat.as_degrees();
        out.lon_deg = point.fix.lon.as_degrees();
        out.heading_deg = point
            .fix
            .heading
            .map_or(opt_f64_none(), |h| opt_f64_some(h.as_degrees()));
        out.speed_mps = point
            .fix
            .speed
            .map_or(opt_f64_none(), |s| opt_f64_some(s.as_meters_per_second()));
        out.eph_m = point.fix.eph_m.map_or(opt_f64_none(), opt_f64_some);
        out.sat_count = point.satellites.as_ref().map_or(0, |r| r.tracked.len());

        GtdStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_satellite(
    f: *const GtdNavFile,
    nav_idx: usize,
    sat_idx: usize,
    out: *mut GtdSatInfo,
) -> GtdStatus {
    run_catching_panics(|| {
        let f = nonnull_ref!(f);
        let out = nonnull_mut!(out);

        let Some(point) = f.file.nav_points().get(nav_idx) else {
            set_last_error(format!("nav point index {nav_idx} is out of range"));
            return GtdStatus::ErrNullArgument;
        };

        let Some(report) = &point.satellites else {
            set_last_error(format!("nav point {nav_idx} has no satellite report"));
            return GtdStatus::ErrNullArgument;
        };

        let Some(sat) = report.tracked.get(sat_idx) else {
            set_last_error(format!("satellite index {sat_idx} is out of range"));
            return GtdStatus::ErrNullArgument;
        };

        out.constellation = GtdConstellation::from(sat.constellation);
        out.prn = sat.prn;
        out.in_fix = u8::from(sat.in_fix);
        out.elevation_deg = sat
            .elevation
            .map_or(opt_f64_none(), |v| opt_f64_some(f64::from(v)));
        out.azimuth_deg = sat
            .azimuth
            .map_or(opt_f64_none(), |v| opt_f64_some(f64::from(v)));
        out.snr_dbhz = sat
            .snr
            .map_or(opt_f64_none(), |v| opt_f64_some(f64::from(v)));

        GtdStatus::Ok
    })
}

// ── Metadata accessors ──────────────────────────────────────────────────────── // [qa-allow-check-em-dash, qa-allow-check-floating-comments, reason = "C API section headers are an established convention in this FFI file"]

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_title(f: *const GtdNavFile) -> *const c_char {
    if f.is_null() {
        return std::ptr::null();
    }
    // SAFETY: f is non-null. CString is stored in the handle for its lifetime
    unsafe {
        (*f).title
            .as_ref()
            .map_or(std::ptr::null(), |cs| cs.as_c_str().as_ptr())
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_device(f: *const GtdNavFile) -> *const c_char {
    if f.is_null() {
        return std::ptr::null();
    }
    // SAFETY: same as gtd_nav_file_title
    unsafe {
        (*f).device
            .as_ref()
            .map_or(std::ptr::null(), |cs| cs.as_c_str().as_ptr())
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_notes(f: *const GtdNavFile) -> *const c_char {
    if f.is_null() {
        return std::ptr::null();
    }
    // SAFETY: same as gtd_nav_file_title
    unsafe {
        (*f).notes
            .as_ref()
            .map_or(std::ptr::null(), |cs| cs.as_c_str().as_ptr())
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_identity(f: *const GtdNavFile) -> *const c_char {
    if f.is_null() {
        return std::ptr::null();
    }
    // SAFETY: same as gtd_nav_file_title
    unsafe {
        (*f).identity
            .as_ref()
            .map_or(std::ptr::null(), |cs| cs.as_c_str().as_ptr())
    }
}

// ── Event marker accessors ──────────────────────────────────────────────────── // [qa-allow-check-em-dash, qa-allow-check-floating-comments, reason = "C API section headers are an established convention in this FFI file"]

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_event_marker_count(f: *const GtdNavFile) -> usize {
    if f.is_null() {
        return 0;
    }
    // SAFETY: f is non-null
    unsafe { (*f).file.event_markers().len() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_event_marker(
    f: *const GtdNavFile,
    idx: usize,
    out: *mut GtdEventMarkerInfo,
) -> GtdStatus {
    run_catching_panics(|| {
        let f = nonnull_ref!(f);
        let out = nonnull_mut!(out);

        let Some(marker) = f.file.event_markers().get(idx) else {
            set_last_error(format!("event marker index {idx} is out of range"));
            return GtdStatus::ErrNullArgument;
        };

        // SAFETY: GtdEventMarkerInfo is repr(C). Zeroing it is valid initial state
        *out = unsafe { std::mem::zeroed() };

        let path_bytes = marker.variant_path.as_bytes();
        for (dst, &src) in out.variant_path.iter_mut().zip(path_bytes.iter()) {
            *dst = src as c_char;
        }
        // out.variant_path[path_bytes.len()] is already 0 from the zeroing above

        out.sys_time = ts_from_datetime(marker.sys_time);
        out.lat_deg = marker.lat.as_degrees();
        out.lon_deg = marker.lon.as_degrees();

        if let Some(ann) = &marker.annotation {
            out.has_annotation = 1;
            let ann_bytes = ann.as_bytes();
            for (dst, &src) in out.annotation.iter_mut().zip(ann_bytes.iter()) {
                *dst = src as c_char;
            }
        }

        GtdStatus::Ok
    })
}

// ── Channel accessors ─────────────────────────────────────────────────────────── // [qa-allow-check-em-dash, qa-allow-check-floating-comments, reason = "C API section headers are an established convention in this FFI file"]

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_channel_count(f: *const GtdNavFile) -> usize {
    if f.is_null() {
        return 0;
    }
    // SAFETY: f is non-null
    unsafe { (*f).file.channels().len() }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_channel(
    f: *const GtdNavFile,
    idx: usize,
    out: *mut GtdChannelInfo,
) -> GtdStatus {
    run_catching_panics(|| {
        let f = nonnull_ref!(f);
        let out = nonnull_mut!(out);

        let Some(ch) = f.file.channels().get(idx) else {
            set_last_error(format!("channel index {idx} is out of range"));
            return GtdStatus::ErrNullArgument;
        };

        // SAFETY: GtdChannelInfo is repr(C). Zeroing it is a valid initial state.
        *out = unsafe { std::mem::zeroed() };
        fill_c_str(&mut out.name, ch.name());
        if let Some(unit) = ch.unit() {
            out.has_unit = 1;
            fill_c_str(&mut out.unit, unit);
        }
        out.period_deg = ch
            .period()
            .map_or(opt_f64_none(), |a| opt_f64_some(a.as_degrees()));
        if let Some(description) = ch.description() {
            out.has_description = 1;
            fill_c_str(&mut out.description, description);
        }
        out.component_count = ch.components().len();
        out.sample_count = ch.times().len();
        GtdStatus::Ok
    })
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_get_channel_component(
    f: *const GtdNavFile,
    ch_idx: usize,
    comp_idx: usize,
    out: *mut c_char,
    cap: usize,
) -> GtdStatus {
    run_catching_panics(|| {
        let f = nonnull_ref!(f);
        if out.is_null() {
            set_last_error("out buffer is null");
            return GtdStatus::ErrNullArgument;
        }
        if cap == 0 {
            set_last_error("out buffer capacity is zero");
            return GtdStatus::ErrNullArgument;
        }
        let Some(ch) = f.file.channels().get(ch_idx) else {
            set_last_error(format!("channel index {ch_idx} is out of range"));
            return GtdStatus::ErrNullArgument;
        };
        let Some(label) = ch.components().get(comp_idx) else {
            set_last_error(format!("component index {comp_idx} is out of range"));
            return GtdStatus::ErrNullArgument;
        };
        // SAFETY: out points to `cap` writable bytes (caller contract).
        let buf = unsafe { std::slice::from_raw_parts_mut(out, cap) };
        fill_c_str(buf, label);
        GtdStatus::Ok
    })
}

/// Copy up to `cap` sample timestamps into `out`, returning the channel's total
/// sample count (independent of `cap`). Passing a null `out` or zero `cap`
/// queries the count without copying.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_channel_times(
    f: *const GtdNavFile,
    ch_idx: usize,
    out: *mut GtdTimestamp,
    cap: usize,
) -> usize {
    if f.is_null() {
        return 0;
    }
    // SAFETY: f is non-null
    let Some(ch) = (unsafe { &(*f).file }).channels().get(ch_idx) else {
        return 0;
    };
    let times = ch.times();
    if !out.is_null() && cap > 0 {
        // SAFETY: out points to `cap` writable elements (caller contract).
        let buf = unsafe { std::slice::from_raw_parts_mut(out, cap) };
        for (slot, &dt) in buf.iter_mut().zip(times.iter()) {
            *slot = ts_from_datetime(dt);
        }
    }
    times.len()
}

/// Copy up to `cap` values into `out`, returning the channel's total value count
/// (`sample_count * max(component_count, 1)`, row-major). Passing a null `out`
/// or zero `cap` queries the count without copying.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn gtd_nav_file_channel_values(
    f: *const GtdNavFile,
    ch_idx: usize,
    out: *mut f64,
    cap: usize,
) -> usize {
    if f.is_null() {
        return 0;
    }
    // SAFETY: f is non-null
    let Some(ch) = (unsafe { &(*f).file }).channels().get(ch_idx) else {
        return 0;
    };
    let values = ch.values();
    if !out.is_null() && cap > 0 {
        // SAFETY: out points to `cap` writable elements (caller contract).
        let buf = unsafe { std::slice::from_raw_parts_mut(out, cap) };
        for (slot, &v) in buf.iter_mut().zip(values.iter()) {
            *slot = v;
        }
    }
    values.len()
}
