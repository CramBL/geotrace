use crate::matches_attrs;
use gt_types::history::{
    ATTR_END_US, ATTR_EVENT_MARKER_COUNT, ATTR_GTD_SIZE_BYTES, ATTR_IDENTITY, ATTR_MARKER_COUNT,
    ATTR_NAV_POINT_COUNT, ATTR_SAT_REPORT_COUNT, ATTR_START_US, CURRENT_SCHEMA_VERSION, DbError,
    RecordingMeta, SCHEMA_VERSION_ATTR, is_db_recording_attr, make_group_name,
};
/// Internal read-modify-write machinery for the history database.
///
/// `hdf5_pure::GroupBuilder` is not publicly exported by name. This module
/// avoids the problem by reading existing data into an intermediate tree of
/// owned Rust types, manipulating that tree, then writing the whole thing to a
/// new `FileBuilder` in one pass.
use hdf5_pure::{AttrValue, DType, FileBuilder};
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum InternalError {
    #[error(transparent)]
    Hdf5(#[from] hdf5_pure::Error),
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

const CHUNK_SIZE: u64 = 8_192;

enum DatasetData {
    F64(Vec<f64>),
    F32(Vec<f32>),
    I64(Vec<i64>),
    U64(Vec<u64>),
    U32(Vec<u32>),
    U8(Vec<u8>),
}

struct DatasetNode {
    name: String,
    shape: Vec<u64>,
    data: DatasetData,
    attrs: Vec<(String, AttrValue)>,
}

struct GroupNode {
    name: String,
    attrs: Vec<(String, AttrValue)>,
    datasets: Vec<DatasetNode>,
    groups: Vec<GroupNode>,
}

/// Write a single `DatasetNode` into a builder identified by `$gb`.
/// Using a macro avoids naming the private `GroupBuilder`/`DatasetBuilder` types.
macro_rules! write_dataset_into {
    ($gb:ident, $ds:expr) => {{
        let ds = $ds;
        let chunk = chunk_for_shape(&ds.shape);
        let db = $gb.create_dataset(&ds.name);
        db.with_shape(&ds.shape);
        match &ds.data {
            DatasetData::F64(v) => {
                db.with_f64_data(v);
            }
            DatasetData::F32(v) => {
                db.with_f32_data(v);
            }
            DatasetData::I64(v) => {
                db.with_i64_data(v);
            }
            DatasetData::U64(v) => {
                db.with_u64_data(v);
            }
            DatasetData::U32(v) => {
                db.with_u32_data(v);
            }
            DatasetData::U8(v) => {
                db.with_u8_data(v);
            }
        }
        for (k, v) in &ds.attrs {
            db.set_attr(k, v.clone());
        }
        db.with_chunks(&chunk);
        db.with_shuffle();
        db.with_deflate(6);
    }};
}

/// Write `identity_nodes` (the full `by_identity` tree) to a new database file at `db_path`.
fn snapshot_group(src: &hdf5_pure::Group<'_>, name: &str) -> Result<GroupNode, InternalError> {
    let mut node = GroupNode {
        name: name.to_owned(),
        attrs: src.attrs()?.into_iter().collect(),
        datasets: Vec::new(),
        groups: Vec::new(),
    };

    for ds_name in src.datasets()? {
        let ds = src.dataset(&ds_name)?;
        let shape = ds.shape()?;
        let dtype = ds.dtype()?;
        let attrs: Vec<(String, AttrValue)> = ds.attrs()?.into_iter().collect();

        let data = match dtype {
            DType::F64 => DatasetData::F64(ds.read_f64()?),
            DType::F32 => DatasetData::F32(ds.read_f32()?),
            DType::I64 => DatasetData::I64(ds.read_i64()?),
            DType::U64 => DatasetData::U64(ds.read_u64()?),
            DType::U32 => DatasetData::U32(ds.read_u32()?),
            DType::U8 => DatasetData::U8(ds.read_u8()?),
            _ => {
                log::warn!("Skipping dataset '{ds_name}' with unsupported dtype {dtype}");
                continue;
            }
        };

        node.datasets.push(DatasetNode {
            name: ds_name,
            shape,
            data,
            attrs,
        });
    }

    for sg_name in src.groups()? {
        let sg = src.group(&sg_name)?;
        node.groups.push(snapshot_group(&sg, &sg_name)?);
    }

    Ok(node)
}

fn snapshot_by_identity(file: &hdf5_pure::File) -> Result<Vec<GroupNode>, InternalError> {
    let by_id = file.root().group("by_identity")?;
    let mut identity_nodes = Vec::new();
    for id_name in by_id.groups()? {
        let id_grp = by_id.group(&id_name)?;
        let mut id_node = GroupNode {
            name: id_name.clone(),
            attrs: Vec::new(),
            datasets: Vec::new(),
            groups: Vec::new(),
        };
        for rec_name in id_grp.groups()? {
            let rec_grp = id_grp.group(&rec_name)?;
            id_node.groups.push(snapshot_group(&rec_grp, &rec_name)?);
        }
        identity_nodes.push(id_node);
    }
    Ok(identity_nodes)
}

fn build_new_recording(
    gtd_file: &hdf5_pure::File,
    rec_name: &str,
    meta: &RecordingMeta,
    identity: &str,
) -> Result<GroupNode, InternalError> {
    let gtd_root = gtd_file.root();
    let mut rec = GroupNode {
        name: rec_name.to_owned(),
        attrs: vec![
            (
                ATTR_IDENTITY.to_owned(),
                AttrValue::String(identity.to_owned()),
            ),
            (ATTR_START_US.to_owned(), AttrValue::I64(meta.start_us)),
            (ATTR_END_US.to_owned(), AttrValue::I64(meta.end_us)),
            (
                ATTR_NAV_POINT_COUNT.to_owned(),
                AttrValue::U64(meta.nav_point_count),
            ),
            (
                ATTR_SAT_REPORT_COUNT.to_owned(),
                AttrValue::U64(meta.sat_report_count),
            ),
            (
                ATTR_MARKER_COUNT.to_owned(),
                AttrValue::U64(meta.marker_count),
            ),
            (
                ATTR_EVENT_MARKER_COUNT.to_owned(),
                AttrValue::U64(meta.event_marker_count),
            ),
            (
                ATTR_GTD_SIZE_BYTES.to_owned(),
                AttrValue::U64(meta.gtd_size_bytes),
            ),
        ],
        datasets: Vec::new(),
        groups: Vec::new(),
    };

    for grp_name in gtd_root.groups()? {
        let data_src = gtd_root.group(&grp_name)?;
        rec.groups.push(snapshot_group(&data_src, &grp_name)?);
    }

    // Preserve all GTD root attributes so they are restored when the recording
    // is loaded back.  Copying unconditionally means new attributes added to the
    // GTD format are carried through without any change to this function.
    for (k, v) in gtd_root.attrs()? {
        rec.attrs.push((k, v));
    }

    Ok(rec)
}

/// Write `identity_nodes` (the full `by_identity` tree) to a new database file at `db_path`.
fn write_db(identity_nodes: &[GroupNode], db_path: &std::path::Path) -> Result<(), InternalError> {
    let mut fb = FileBuilder::new();
    fb.set_attr(SCHEMA_VERSION_ATTR, AttrValue::I64(CURRENT_SCHEMA_VERSION));

    let meta_gb = fb.create_group("meta");
    fb.add_group(meta_gb.finish());

    let mut by_id_gb = fb.create_group("by_identity");
    for id_node in identity_nodes {
        let mut id_gb = by_id_gb.create_group(&id_node.name);
        for rec_node in &id_node.groups {
            let mut rec_gb = id_gb.create_group(&rec_node.name);
            for (k, v) in &rec_node.attrs {
                rec_gb.set_attr(k, v.clone());
            }
            for ds in &rec_node.datasets {
                write_dataset_into!(rec_gb, ds);
            }
            for child in &rec_node.groups {
                let mut child_gb = rec_gb.create_group(&child.name);
                for (k, v) in &child.attrs {
                    child_gb.set_attr(k, v.clone());
                }
                for ds in &child.datasets {
                    write_dataset_into!(child_gb, ds);
                }
                rec_gb.add_group(child_gb.finish());
            }
            id_gb.add_group(rec_gb.finish());
        }
        by_id_gb.add_group(id_gb.finish());
    }
    fb.add_group(by_id_gb.finish());

    fb.write(db_path)?;
    Ok(())
}

pub(crate) fn insert_recording(
    db_path: &std::path::Path,
    identity: &str,
    meta: &RecordingMeta,
    gtd_bytes: &[u8],
) -> Result<String, InternalError> {
    let existing_db = hdf5_pure::File::open(db_path)?;

    // Check for duplicate; return existing name if found.
    {
        let root = existing_db.root();
        if let Ok(by_id) = root.group("by_identity") {
            for id_name in by_id.groups()? {
                if let Ok(id_grp) = by_id.group(&id_name) {
                    for rec_name in id_grp.groups()? {
                        if let Ok(rec_grp) = id_grp.group(&rec_name)
                            && let Ok(attrs) = rec_grp.attrs()
                            && matches_attrs(meta, &attrs)
                        {
                            log::debug!("Skipping duplicate recording '{id_name}/{rec_name}'");
                            return Ok(rec_name);
                        }
                    }
                }
            }
        }
    }

    // Read all existing identity data into memory.
    let mut identity_nodes = snapshot_by_identity(&existing_db)?;

    // Determine the new recording name.
    let existing_for_identity: Vec<String> = identity_nodes
        .iter()
        .find(|n| n.name == identity)
        .map(|n| n.groups.iter().map(|r| r.name.clone()).collect())
        .unwrap_or_default();
    let rec_name = make_group_name(meta.start_us, meta.total_count(), &existing_for_identity);

    // Build the new recording node from the GTD file.
    let gtd_file = hdf5_pure::File::from_bytes(gtd_bytes.to_vec())?;
    let new_recording = build_new_recording(&gtd_file, &rec_name, meta, identity)?;

    // Insert or create the identity group.
    match identity_nodes.iter_mut().find(|n| n.name == identity) {
        Some(id_node) => id_node.groups.push(new_recording),
        None => identity_nodes.push(GroupNode {
            name: identity.to_owned(),
            attrs: Vec::new(),
            datasets: Vec::new(),
            groups: vec![new_recording],
        }),
    }

    write_db(&identity_nodes, db_path)?;
    log::info!("Stored recording '{identity}/{rec_name}' in history database");
    Ok(rec_name)
}

/// Remove multiple recordings in a single read-modify-write cycle.
pub(crate) fn delete_batch(
    db_path: &std::path::Path,
    refs: &[gt_types::DatabaseRef],
) -> Result<(), InternalError> {
    let existing_db = hdf5_pure::File::open(db_path)?;
    let mut identity_nodes = snapshot_by_identity(&existing_db)?;
    drop(existing_db);

    for db_ref in refs {
        if let Some(id_node) = identity_nodes
            .iter_mut()
            .find(|n| n.name == db_ref.identity)
        {
            id_node.groups.retain(|r| r.name != db_ref.group_name);
        }
    }
    identity_nodes.retain(|n| !n.groups.is_empty());

    write_db(&identity_nodes, db_path)?;
    log::info!("Deleted {} recording(s) in batch prune", refs.len());
    Ok(())
}

/// Remove one recording group from the database using a read-modify-write cycle.
pub(crate) fn delete_recording(
    db_path: &std::path::Path,
    identity: &str,
    group_name: &str,
) -> Result<(), InternalError> {
    let existing_db = hdf5_pure::File::open(db_path)?;
    let mut identity_nodes = snapshot_by_identity(&existing_db)?;

    if let Some(id_node) = identity_nodes.iter_mut().find(|n| n.name == identity) {
        id_node.groups.retain(|r| r.name != group_name);
    }
    // Drop empty identity groups.
    identity_nodes.retain(|n| !n.groups.is_empty());

    write_db(&identity_nodes, db_path)?;
    log::info!("Deleted recording '{identity}/{group_name}' from history database");
    Ok(())
}

/// Read a recording back from the database and return it as GTD-format bytes.
pub(crate) fn load_recording_bytes(
    db_path: &std::path::Path,
    identity: &str,
    group_name: &str,
) -> Result<Vec<u8>, InternalError> {
    let db = hdf5_pure::File::open(db_path)?;
    let by_id = db.root().group("by_identity")?;
    let id_grp = by_id.group(identity)?;
    let rec_grp = id_grp.group(group_name)?;

    // Snapshot all child data groups (nav_points, sat_reports, etc.) and
    // write them as a fresh GTD-format HDF5 file.
    let mut fb = FileBuilder::new();

    // Restore GTD root attributes.  Every attr on the recording group that is
    // not a DB-internal field is an GTD root attr and belongs on the file root.
    // Using a denylist (rather than an allowlist) means new GTD attrs are
    // restored automatically.  Fall back to geotrace_version="1" for recordings
    // stored by older code that predates attr preservation.
    let rec_attrs = rec_grp.attrs()?;
    let mut has_version = false;
    for (k, v) in &rec_attrs {
        if !is_db_recording_attr(k) {
            fb.set_attr(k, v.clone());
            if k == "geotrace_version" {
                has_version = true;
            }
        }
    }
    if !has_version {
        fb.set_attr("geotrace_version", AttrValue::String("1".into()));
    }

    for child_name in rec_grp.groups()? {
        let child = rec_grp.group(&child_name)?;
        let node = snapshot_group(&child, &child_name)?;
        let mut gb = fb.create_group(&node.name);
        for (k, v) in &node.attrs {
            gb.set_attr(k, v.clone());
        }
        for ds in &node.datasets {
            write_dataset_into!(gb, ds);
        }
        for sg in &node.groups {
            let mut sgb = gb.create_group(&sg.name);
            for (k, v) in &sg.attrs {
                sgb.set_attr(k, v.clone());
            }
            for ds in &sg.datasets {
                write_dataset_into!(sgb, ds);
            }
            gb.add_group(sgb.finish());
        }
        fb.add_group(gb.finish());
    }

    // hdf5-pure can only write to a path; write a sibling temp file then read
    // it back into memory before removing it.
    let tmp_path = db_path.with_extension("load_tmp.h5");
    fb.write(&tmp_path)?;
    let bytes = std::fs::read(&tmp_path)?;
    std::fs::remove_file(&tmp_path).ok();
    Ok(bytes)
}

/// Rewrite the database preserving all data but updating the `schema_version`
/// root attribute to `CURRENT_SCHEMA_VERSION`.
///
/// Called after a successful migration to stamp the new version.
pub(crate) fn write_schema_version(db_path: &std::path::Path) -> Result<(), InternalError> {
    let existing_db = hdf5_pure::File::open(db_path)?;
    let identity_nodes = snapshot_by_identity(&existing_db)?;
    drop(existing_db);
    write_db(&identity_nodes, db_path)
}

fn chunk_for_shape(shape: &[u64]) -> Vec<u64> {
    match shape {
        [] => vec![CHUNK_SIZE],
        [n] => vec![CHUNK_SIZE.min(*n).max(1)],
        [_rows, cols] => vec![(CHUNK_SIZE / cols).max(1), *cols],
        _ => shape.to_vec(),
    }
}
