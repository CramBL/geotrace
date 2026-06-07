use gt_types::history::{
    ATTR_END_US, ATTR_EVENT_MARKER_COUNT, ATTR_GTD_SIZE_BYTES, ATTR_IDENTITY, ATTR_MARKER_COUNT,
    ATTR_NAV_POINT_COUNT, ATTR_SAT_REPORT_COUNT, ATTR_START_US, DbError, RecordingEntry,
    RecordingMeta, make_group_name,
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

    // Write meta/copy group
    write_meta_attrs(&rec_grp, identity, meta)?;

    // Temporary file for GTD data
    let tmp = tempfile::NamedTempFile::new_in(db_path.parent().unwrap_or(Path::new(".")))?;
    std::fs::write(tmp.path(), gtd_bytes)?;
    let gtd_file = hdf5::File::open(tmp.path())?;

    // Copy content
    copy_group_hdf5(&gtd_file, &rec_grp)?;

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
        let v = val
            .parse::<hdf5::types::VarLenUnicode>()
            .map_err(|_| InternalError::Hdf5(hdf5::Error::from("parse error".to_string())))?;
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

fn copy_group_hdf5(src: &Group, dst: &Group) -> Result<(), InternalError> {
    // Copy attributes
    for attr_name in src.attr_names()? {
        if dst.attr(&attr_name).is_ok() {
            continue;
        }
        copy_attr(src, dst, &attr_name)?;
    }
    // Copy members
    for name in src.member_names()? {
        if let Ok(ds) = src.dataset(&name) {
            let dtype = ds.dtype()?;
            use hdf5::types::TypeDescriptor;
            match dtype
                .to_descriptor()
                .map_err(|e| InternalError::Hdf5(hdf5::Error::from(e.to_string())))?
            {
                TypeDescriptor::Integer(hdf5::types::IntSize::U8) => {
                    let data: Vec<i64> = ds.read_raw().unwrap_or_default();
                    let new_ds = dst
                        .new_dataset::<i64>()
                        .shape(ds.shape())
                        .create(name.as_str())?;
                    new_ds.write_raw(&data)?;
                }
                _ => {
                    let data: Vec<u8> = ds.read_raw().unwrap_or_default();
                    let new_ds = dst
                        .new_dataset::<u8>()
                        .shape(ds.shape())
                        .create(name.as_str())?;
                    new_ds.write_raw(&data)?;
                }
            }
        } else if let Ok(child_src) = src.group(&name) {
            let child_dst = dst.create_group(name.as_str())?;
            copy_group_hdf5(&child_src, &child_dst)?;
        }
    }
    Ok(())
}

fn copy_attr(src: &Group, dst: &Group, name: &str) -> Result<(), InternalError> {
    let attr = src.attr(name)?;
    let dtype = attr.dtype()?;

    use hdf5::types::TypeDescriptor;
    match dtype
        .to_descriptor()
        .map_err(|e| InternalError::Hdf5(hdf5::Error::from(e.to_string())))?
    {
        TypeDescriptor::VarLenUnicode | TypeDescriptor::FixedUnicode(_) => {
            let v: hdf5::types::VarLenUnicode = attr.read_scalar()?;
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
        _ => log::warn!("Skipping attribute '{}' with unsupported type", name),
    }
    Ok(())
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

    let tmp = tempfile::NamedTempFile::new_in(db_path.parent().unwrap_or(Path::new(".")))?;
    let out = hdf5::File::create(tmp.path())?;

    // Copy everything from rec_grp to out root
    let root = out
        .group("/")
        .map_err(|_| hdf5::Error::from("failed to open root"))?;
    copy_group_hdf5(&rec_grp, &root)?;

    // Copy attributes of rec_grp to root
    for attr_name in rec_grp.attr_names()? {
        if root.attr(&attr_name).is_ok() {
            continue;
        }
        copy_attr(&rec_grp, &root, &attr_name)?;
    }

    // Ensure version exists
    if root.attr("geotrace_version").is_err() {
        root.new_attr::<hdf5::types::VarLenUnicode>()
            .create("geotrace_version")?
            .write_scalar(&"1".parse::<hdf5::types::VarLenUnicode>().unwrap())?;
    }

    out.flush().map_err(InternalError::Hdf5)?;
    drop(out);

    Ok(std::fs::read(tmp.path())?)
}

pub(crate) fn list_recordings(
    db_path: &std::path::Path,
) -> Result<Vec<RecordingEntry>, InternalError> {
    let file = hdf5::File::open(db_path)?;
    let root = file
        .group("/")
        .map_err(|_| hdf5::Error::from("failed to open root"))?;
    let by_id = root.group("by_identity")?;
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
