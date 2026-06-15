use gt_types::history::{
    ATTR_END_US, ATTR_EVENT_MARKER_COUNT, ATTR_GTD_SIZE_BYTES, ATTR_IDENTITY, ATTR_MARKER_COUNT,
    ATTR_NAV_POINT_COUNT, ATTR_SAT_REPORT_COUNT, ATTR_START_US, DbError, RecordingEntry,
    RecordingMeta, is_db_recording_attr, make_group_name,
};
use hdf5::Group;
use std::path::Path;
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum InternalError {
    #[error(transparent)]
    Hdf5(#[from] hdf5::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

impl From<InternalError> for DbError {
    fn from(e: InternalError) -> Self {
        match e {
            InternalError::Hdf5(e) => DbError::Backend(e.to_string()),
            InternalError::Io(e) => DbError::Io(e),
        }
    }
}

pub(crate) fn matches_attrs(meta: &RecordingMeta, group: &Group) -> bool {
    let read_i64 = |name: &str| group.attr(name).and_then(|a| a.read_scalar::<i64>()).ok();
    let read_u64 = |name: &str| group.attr(name).and_then(|a| a.read_scalar::<u64>()).ok();

    let Some(start_us) = read_i64(ATTR_START_US) else {
        return false;
    };
    let Some(nav_point_count) = read_u64(ATTR_NAV_POINT_COUNT) else {
        return false;
    };
    let Some(sat_report_count) = read_u64(ATTR_SAT_REPORT_COUNT) else {
        return false;
    };
    let Some(marker_count) = read_u64(ATTR_MARKER_COUNT) else {
        return false;
    };
    let Some(event_marker_count) = read_u64(ATTR_EVENT_MARKER_COUNT) else {
        return false;
    };

    meta.matches(
        start_us,
        nav_point_count,
        sat_report_count,
        marker_count,
        event_marker_count,
    )
}

pub(crate) fn is_duplicate(
    db_path: &std::path::Path,
    _identity: &str,
    meta: &RecordingMeta,
) -> Result<bool, InternalError> {
    let file = hdf5::File::open(db_path)?;
    if let Ok(by_id) = file.group("by_identity") {
        for id_name in by_id.member_names()? {
            if let Ok(id_grp) = by_id.group(&id_name) {
                for rec_name in id_grp.member_names()? {
                    if let Ok(rec_grp) = id_grp.group(&rec_name)
                        && matches_attrs(meta, &rec_grp)
                    {
                        return Ok(true);
                    }
                }
            }
        }
    }
    Ok(false)
}

pub(crate) fn insert_recording(
    db_path: &std::path::Path,
    identity: &str,
    meta: &RecordingMeta,
    gtd_bytes: &[u8],
) -> Result<String, InternalError> {
    let file = hdf5::File::open_rw(db_path)?;

    // Check for duplicate
    if let Ok(by_id) = file.group("by_identity") {
        for id_name in by_id.member_names()? {
            if let Ok(id_grp) = by_id.group(&id_name) {
                for rec_name in id_grp.member_names()? {
                    if let Ok(rec_grp) = id_grp.group(&rec_name)
                        && matches_attrs(meta, &rec_grp)
                    {
                        return Ok(rec_name);
                    }
                }
            }
        }
    }

    // Determine name, create group
    let by_id = file.group("by_identity")?;
    let id_grp = by_id
        .create_group(identity)
        .or_else(|_| by_id.group(identity))?;
    let existing_names = id_grp.member_names()?;
    let group_name = make_group_name(meta.start_us, meta.total_count(), &existing_names);
    let rec_grp = id_grp.create_group(&group_name)?;

    // Record the database metadata as recording-group attributes.
    write_meta_attrs(&rec_grp, identity, meta)?;

    // libhdf5's object copy works between open files, so stage the GTD bytes in
    // a temporary file and copy its objects straight into the recording group.
    let tmp = tempfile::NamedTempFile::new_in(db_path.parent().unwrap_or_else(|| Path::new(".")))?;
    std::fs::write(tmp.path(), gtd_bytes)?;
    let gtd_file = hdf5::File::open(tmp.path())?;

    // Preserve the GTD file's root attributes, then faithfully copy each data
    // group/dataset (datatypes, attributes, chunking, and compression) into the
    // recording group.
    copy_attrs(&gtd_file, &rec_grp, |_| true)?;
    copy_members(&gtd_file, &rec_grp)?;

    Ok(group_name)
}

fn write_meta_attrs(
    group: &Group,
    identity: &str,
    meta: &RecordingMeta,
) -> Result<(), InternalError> {
    let write_str = |name: &str, val: &str| -> Result<(), InternalError> {
        if group.attr(name).is_ok() {
            return Ok(());
        }
        let attr = group
            .new_attr::<hdf5::types::VarLenUnicode>()
            .create(name)?;
        let v = val.parse::<hdf5::types::VarLenUnicode>().map_err(|e| {
            InternalError::Hdf5(hdf5::Error::from(format!("invalid attribute string: {e}")))
        })?;
        attr.write_scalar(&v)?;
        Ok(())
    };
    let write_i64 = |name: &str, val: i64| -> Result<(), InternalError> {
        if group.attr(name).is_ok() {
            return Ok(());
        }
        let attr = group.new_attr::<i64>().create(name)?;
        attr.write_scalar(&val)?;
        Ok(())
    };
    let write_u64 = |name: &str, val: u64| -> Result<(), InternalError> {
        if group.attr(name).is_ok() {
            return Ok(());
        }
        let attr = group.new_attr::<u64>().create(name)?;
        attr.write_scalar(&val)?;
        Ok(())
    };

    write_str(ATTR_IDENTITY, identity)?;
    write_i64(ATTR_START_US, meta.start_us)?;
    write_i64(ATTR_END_US, meta.end_us)?;
    write_u64(ATTR_NAV_POINT_COUNT, meta.nav_point_count)?;
    write_u64(ATTR_SAT_REPORT_COUNT, meta.sat_report_count)?;
    write_u64(ATTR_MARKER_COUNT, meta.marker_count)?;
    write_u64(ATTR_EVENT_MARKER_COUNT, meta.event_marker_count)?;
    write_u64(ATTR_GTD_SIZE_BYTES, meta.gtd_size_bytes)?;

    Ok(())
}

/// Faithfully copy every member object of `src` into `dst` using the HDF5
/// object-copy primitive (`H5Ocopy`).
///
/// Unlike a hand-rolled dataset copy, this preserves every member's datatype,
/// shape, attributes, chunking, and compression for the whole subtree - and it
/// copies across open files, so it works for both storing a GTD file into the
/// database and extracting a recording back out.
fn copy_members(src: &Group, dst: &Group) -> Result<(), InternalError> {
    for name in src.member_names()? {
        if let Ok(grp) = src.group(&name) {
            grp.copy_to(dst, &name)?;
        } else if let Ok(ds) = src.dataset(&name) {
            ds.copy_to(dst, &name)?;
        }
    }
    Ok(())
}

/// Copy the group-level attributes of `src` onto `dst`, skipping any whose name
/// `keep` rejects and any already present on `dst`.
fn copy_attrs(src: &Group, dst: &Group, keep: impl Fn(&str) -> bool) -> Result<(), InternalError> {
    for attr_name in src.attr_names()? {
        if !keep(&attr_name) || dst.attr(&attr_name).is_ok() {
            continue;
        }
        copy_attr(src, dst, &attr_name)?;
    }
    Ok(())
}

fn copy_attr(src: &Group, dst: &Group, name: &str) -> Result<(), InternalError> {
    use hdf5::types::TypeDescriptor;
    let attr = src.attr(name)?;
    let descriptor = attr
        .dtype()?
        .to_descriptor()
        .map_err(|e| InternalError::Hdf5(hdf5::Error::from(e.to_string())))?;

    match descriptor {
        TypeDescriptor::VarLenUnicode
        | TypeDescriptor::VarLenAscii
        | TypeDescriptor::FixedUnicode(_)
        | TypeDescriptor::FixedAscii(_) => {
            // Normalise every string attribute to variable-length unicode on the
            // way in; reading the original is the subtle part (see read_string_attr).
            let s = read_string_attr(&attr, &descriptor)?;
            let v: hdf5::types::VarLenUnicode = s.parse().map_err(|e| {
                InternalError::Hdf5(hdf5::Error::from(format!("invalid attribute string: {e}")))
            })?;
            dst.new_attr::<hdf5::types::VarLenUnicode>()
                .create(name)?
                .write_scalar(&v)?;
        }
        TypeDescriptor::Integer(hdf5::types::IntSize::U8) => {
            let v: i64 = attr.read_scalar()?;
            dst.new_attr::<i64>().create(name)?.write_scalar(&v)?;
        }
        TypeDescriptor::Unsigned(hdf5::types::IntSize::U8) => {
            let v: u64 = attr.read_scalar()?;
            dst.new_attr::<u64>().create(name)?.write_scalar(&v)?;
        }
        _ => log::warn!("Skipping attribute '{name}' with unsupported type"),
    }
    Ok(())
}

/// Read a string attribute (fixed- or variable-length, unicode or ASCII) into an
/// owned `String`.
///
/// libhdf5 converts between fixed string sizes but offers no fixed -> variable
/// conversion path, so a fixed-length attribute (how the SDK writes the GTD root
/// strings) cannot be read straight into `VarLenUnicode` - it must be read into
/// a fixed buffer at least as large as the on-disk length. The ladder picks the
/// smallest compile-time capacity that covers it.
fn read_string_attr(
    attr: &hdf5::Attribute,
    descriptor: &hdf5::types::TypeDescriptor,
) -> Result<String, InternalError> {
    use hdf5::types::{FixedAscii, FixedUnicode, TypeDescriptor, VarLenAscii, VarLenUnicode};

    macro_rules! read_fixed {
        ($fixed:ident, $len:expr) => {{
            let len = $len;
            if len < 64 {
                attr.read_scalar::<$fixed<64>>()?.as_str().to_owned()
            } else if len < 256 {
                attr.read_scalar::<$fixed<256>>()?.as_str().to_owned()
            } else if len < 1024 {
                attr.read_scalar::<$fixed<1024>>()?.as_str().to_owned()
            } else if len < 8192 {
                attr.read_scalar::<$fixed<8192>>()?.as_str().to_owned()
            } else {
                return Err(InternalError::Hdf5(hdf5::Error::from(format!(
                    "string attribute too long to copy in place ({len} bytes)"
                ))));
            }
        }};
    }

    Ok(match descriptor {
        TypeDescriptor::VarLenUnicode => attr.read_scalar::<VarLenUnicode>()?.as_str().to_owned(),
        TypeDescriptor::VarLenAscii => attr.read_scalar::<VarLenAscii>()?.as_str().to_owned(),
        TypeDescriptor::FixedUnicode(n) => read_fixed!(FixedUnicode, *n),
        TypeDescriptor::FixedAscii(n) => read_fixed!(FixedAscii, *n),
        other => {
            return Err(InternalError::Hdf5(hdf5::Error::from(format!(
                "attribute is not a string type: {other:?}"
            ))));
        }
    })
}

pub(crate) fn load_recording_bytes(
    db_path: &std::path::Path,
    identity: &str,
    group_name: &str,
) -> Result<Vec<u8>, InternalError> {
    let db = hdf5::File::open(db_path)?;
    let rec_grp = db
        .group("by_identity")?
        .group(identity)?
        .group(group_name)?;

    let tmp = tempfile::NamedTempFile::new_in(db_path.parent().unwrap_or_else(|| Path::new(".")))?;
    let out = hdf5::File::create(tmp.path())?;
    let root = out.group("/")?;

    // Reconstruct the GTD file: copy the data groups/datasets back to the root,
    // and restore the original GTD root attributes (skipping the database's own
    // recording metadata, which is not part of the GTD format).
    copy_members(&rec_grp, &root)?;
    copy_attrs(&rec_grp, &root, |name| !is_db_recording_attr(name))?;

    // Fall back to geotrace_version="1" for recordings stored before attribute
    // preservation existed.
    if root.attr("geotrace_version").is_err() {
        let version: hdf5::types::VarLenUnicode = "1".parse().map_err(|e| {
            InternalError::Hdf5(hdf5::Error::from(format!("invalid version literal: {e}")))
        })?;
        root.new_attr::<hdf5::types::VarLenUnicode>()
            .create("geotrace_version")?
            .write_scalar(&version)?;
    }

    out.flush().map_err(InternalError::Hdf5)?;
    drop(out);

    Ok(std::fs::read(tmp.path())?)
}

pub(crate) fn list_recordings(
    db_path: &std::path::Path,
) -> Result<Vec<RecordingEntry>, InternalError> {
    let file = hdf5::File::open(db_path)?;
    let by_id = file.group("by_identity")?;
    let mut entries = Vec::new();
    for identity in by_id.member_names()? {
        if let Ok(id_grp) = by_id.group(&identity) {
            for rec_name in id_grp.member_names()? {
                if let Ok(rec_grp) = id_grp.group(&rec_name) {
                    // Extract meta
                    let read_i64 =
                        |name: &str| rec_grp.attr(name).and_then(|a| a.read_scalar::<i64>()).ok();
                    let read_u64 =
                        |name: &str| rec_grp.attr(name).and_then(|a| a.read_scalar::<u64>()).ok();

                    let meta = RecordingMeta {
                        start_us: read_i64(ATTR_START_US).unwrap_or(0),
                        end_us: read_i64(ATTR_END_US).unwrap_or(0),
                        nav_point_count: read_u64(ATTR_NAV_POINT_COUNT).unwrap_or(0),
                        sat_report_count: read_u64(ATTR_SAT_REPORT_COUNT).unwrap_or(0),
                        marker_count: read_u64(ATTR_MARKER_COUNT).unwrap_or(0),
                        event_marker_count: read_u64(ATTR_EVENT_MARKER_COUNT).unwrap_or(0),
                        gtd_size_bytes: read_u64(ATTR_GTD_SIZE_BYTES).unwrap_or(0),
                    };

                    entries.push(RecordingEntry {
                        db_ref: gt_types::DatabaseRef {
                            identity: identity.clone(),
                            group_name: rec_name,
                        },
                        meta,
                    });
                }
            }
        }
    }
    entries.sort_unstable_by_key(|e| std::cmp::Reverse(e.meta.start_us));
    Ok(entries)
}
