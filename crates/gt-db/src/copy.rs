/// Internal read-modify-write machinery for the history database.
///
/// `hdf5_pure::GroupBuilder` is not publicly exported by name. This module
/// avoids the problem by reading existing data into an intermediate tree of
/// owned Rust types, manipulating that tree, then writing the whole thing to a
/// new `FileBuilder` in one pass.
use hdf5_pure::{AttrValue, DType, FileBuilder};

use crate::{CURRENT_SCHEMA_VERSION, DbError, RecordingMeta, SCHEMA_VERSION_ATTR, make_group_name};

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

fn snapshot_group(src: &hdf5_pure::Group<'_>, name: &str) -> Result<GroupNode, DbError> {
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

fn snapshot_by_identity(file: &hdf5_pure::File) -> Result<Vec<GroupNode>, DbError> {
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
    nvd_file: &hdf5_pure::File,
    rec_name: &str,
    meta: &RecordingMeta,
    identity: &str,
) -> Result<GroupNode, DbError> {
    let nvd_root = nvd_file.root();
    let mut rec = GroupNode {
        name: rec_name.to_owned(),
        attrs: vec![
            (
                "identity".to_owned(),
                AttrValue::String(identity.to_owned()),
            ),
            ("start_us".to_owned(), AttrValue::I64(meta.start_us)),
            (
                "nav_point_count".to_owned(),
                AttrValue::U64(meta.nav_point_count),
            ),
            (
                "sat_report_count".to_owned(),
                AttrValue::U64(meta.sat_report_count),
            ),
            ("marker_count".to_owned(), AttrValue::U64(meta.marker_count)),
            (
                "event_marker_count".to_owned(),
                AttrValue::U64(meta.event_marker_count),
            ),
        ],
        datasets: Vec::new(),
        groups: Vec::new(),
    };

    for grp_name in nvd_root.groups()? {
        let data_src = nvd_root.group(&grp_name)?;
        rec.groups.push(snapshot_group(&data_src, &grp_name)?);
    }

    Ok(rec)
}

#[expect(
    clippy::cognitive_complexity,
    reason = "read-modify-write cycle with nested group/dataset replay is inherently linear; splitting would only add indirection"
)]
pub(crate) fn insert_recording(
    db_path: &std::path::Path,
    identity: &str,
    meta: &RecordingMeta,
    nvd_bytes: &[u8],
) -> Result<String, DbError> {
    let existing_db = hdf5_pure::File::open(db_path)?;

    // Check for duplicate; return existing name if found.
    {
        let root = existing_db.root();
        if let Ok(by_id) = root.group("by_identity")
            && let Ok(id_grp) = by_id.group(identity)
        {
            for rec_name in id_grp.groups()? {
                if let Ok(rec_grp) = id_grp.group(&rec_name)
                    && let Ok(attrs) = rec_grp.attrs()
                    && meta.matches_attrs(&attrs)
                {
                    log::debug!("Skipping duplicate recording '{identity}/{rec_name}'");
                    return Ok(rec_name);
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

    // Build the new recording node from the NVD file.
    let nvd_file = hdf5_pure::File::from_bytes(nvd_bytes.to_vec())?;
    let new_recording = build_new_recording(&nvd_file, &rec_name, meta, identity)?;

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

    // Write the complete new database file.
    let mut fb = FileBuilder::new();
    fb.set_attr(SCHEMA_VERSION_ATTR, AttrValue::I64(CURRENT_SCHEMA_VERSION));

    let meta_gb = fb.create_group("meta");
    fb.add_group(meta_gb.finish());

    {
        let mut by_id_gb = fb.create_group("by_identity");
        for id_node in &identity_nodes {
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
    }

    fb.write(db_path)?;
    log::info!("Stored recording '{identity}/{rec_name}' in history database");
    Ok(rec_name)
}

fn chunk_for_shape(shape: &[u64]) -> Vec<u64> {
    match shape {
        [] => vec![CHUNK_SIZE],
        [n] => vec![CHUNK_SIZE.min(*n).max(1)],
        [_rows, cols] => vec![(CHUNK_SIZE / cols).max(1), *cols],
        _ => shape.to_vec(),
    }
}
