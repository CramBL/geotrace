use crate::matches_attrs;
use gt_types::history::{
    ATTR_END_US, ATTR_EVENT_MARKER_COUNT, ATTR_GTD_SIZE_BYTES, ATTR_IDENTITY, ATTR_MARKER_COUNT,
    ATTR_NAV_POINT_COUNT, ATTR_SAT_REPORT_COUNT, ATTR_SEG_CLOCK_SIGMAS, ATTR_SEG_DETECT_CLOCK,
    ATTR_SEG_GAP_US, ATTR_START_US, CURRENT_SCHEMA_VERSION, DbError, GTD_VERSION_ATTR,
    GTD_VERSION_FALLBACK, RecordingMeta, SCHEMA_VERSION_ATTR, StoredRecording, StoredSegmentation,
    TRACK_END_DATASET, TRACK_HIDDEN_DATASET, TRACK_START_DATASET, TRACKS_GROUP, TrackRange,
    is_db_internal_group, is_db_recording_attr, make_group_name,
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

/// Recursively read an HDF5 group (its attrs, datasets, and subgroups) into an
/// owned `GroupNode`.
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

/// Build the DB-internal `__geotrace_tracks__` subgroup node holding the track
/// ranges as parallel `start`/`end`/`hidden` u64 datasets.
fn track_table_node(tracks: &[TrackRange]) -> GroupNode {
    let n = tracks.len() as u64;
    let (starts, ends, hidden) = gt_types::history::track_columns(tracks);
    GroupNode {
        name: TRACKS_GROUP.to_owned(),
        attrs: Vec::new(),
        datasets: vec![
            DatasetNode {
                name: TRACK_START_DATASET.to_owned(),
                shape: vec![n],
                data: DatasetData::U64(starts),
                attrs: Vec::new(),
            },
            DatasetNode {
                name: TRACK_END_DATASET.to_owned(),
                shape: vec![n],
                data: DatasetData::U64(ends),
                attrs: Vec::new(),
            },
            DatasetNode {
                name: TRACK_HIDDEN_DATASET.to_owned(),
                shape: vec![n],
                data: DatasetData::U64(hidden),
                attrs: Vec::new(),
            },
        ],
        groups: Vec::new(),
    }
}

fn build_new_recording(
    gtd_file: &hdf5_pure::File,
    rec_name: &str,
    meta: &RecordingMeta,
    identity: &str,
    tracks: &[TrackRange],
    settings: StoredSegmentation,
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
            (
                ATTR_SEG_GAP_US.to_owned(),
                AttrValue::I64(settings.track_split_gap_us),
            ),
            (
                ATTR_SEG_DETECT_CLOCK.to_owned(),
                AttrValue::U64(u64::from(settings.detect_clock_discontinuities)),
            ),
            (
                ATTR_SEG_CLOCK_SIGMAS.to_owned(),
                AttrValue::F64(settings.clock_discontinuity_sigmas),
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

    // Store the track ranges in the DB-internal subgroup.
    rec.groups.push(track_table_node(tracks));

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
    tracks: &[TrackRange],
    settings: StoredSegmentation,
    gtd_bytes: &[u8],
) -> Result<String, InternalError> {
    let existing_db = hdf5_pure::File::open(db_path)?;

    // Duplicate check: re-storing a recording already present returns its
    // existing group unchanged (keeping its current track table).
    let duplicate = {
        let root = existing_db.root();
        let mut found = None;
        if let Ok(by_id) = root.group("by_identity") {
            'search: for id_name in by_id.groups()? {
                let Ok(id_grp) = by_id.group(&id_name) else {
                    continue;
                };
                for rec_name in id_grp.groups()? {
                    if let Ok(rec_grp) = id_grp.group(&rec_name)
                        && let Ok(attrs) = rec_grp.attrs()
                        && matches_attrs(meta, &attrs)
                    {
                        found = Some((id_name.clone(), rec_name));
                        break 'search;
                    }
                }
            }
        }
        found
    };

    if let Some((id_name, rec_name)) = duplicate {
        log::debug!("Recording '{id_name}/{rec_name}' already in history");
        return Ok(rec_name);
    }

    // Read all existing identity data into memory.
    let mut identity_nodes = snapshot_by_identity(&existing_db)?;

    // Determine the new recording name. A UUID makes the name collision-free even
    // for recordings that start within the same second.
    let rec_name = make_group_name(meta.start_us, &uuid::Uuid::new_v4().to_string());

    // Build the new recording node from the GTD file.
    let gtd_file = hdf5_pure::File::from_bytes(gtd_bytes.to_vec())?;
    let new_recording =
        build_new_recording(&gtd_file, &rec_name, meta, identity, tracks, settings)?;

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

/// Set or clear the hidden flag on the given tracks (by index) of a recording,
/// via a read-modify-write cycle.
pub(crate) fn set_tracks_hidden(
    db_path: &std::path::Path,
    identity: &str,
    group_name: &str,
    track_indices: &[usize],
    hidden: bool,
) -> Result<(), InternalError> {
    let existing_db = hdf5_pure::File::open(db_path)?;
    let mut identity_nodes = snapshot_by_identity(&existing_db)?;
    drop(existing_db);

    let value = u64::from(hidden);
    let mut found_table = false;
    if let Some(id_node) = identity_nodes.iter_mut().find(|n| n.name == identity)
        && let Some(rec) = id_node.groups.iter_mut().find(|r| r.name == group_name)
        && let Some(tracks_grp) = rec.groups.iter_mut().find(|g| g.name == TRACKS_GROUP)
        && let Some(ds) = tracks_grp
            .datasets
            .iter_mut()
            .find(|d| d.name == TRACK_HIDDEN_DATASET)
        && let DatasetData::U64(flags) = &mut ds.data
    {
        found_table = true;
        for &i in track_indices {
            match flags.get_mut(i) {
                Some(slot) => *slot = value,
                None => log::warn!("track index {i} out of range for {identity}/{group_name}"),
            }
        }
    }
    if !found_table {
        log::warn!("set_tracks_hidden on {identity}/{group_name} which has no track table");
    }

    write_db(&identity_nodes, db_path)?;
    Ok(())
}

/// Replace a recording's track table and segmentation settings.
pub(crate) fn set_tracks(
    db_path: &std::path::Path,
    identity: &str,
    group_name: &str,
    tracks: &[TrackRange],
    settings: StoredSegmentation,
) -> Result<(), InternalError> {
    let existing_db = hdf5_pure::File::open(db_path)?;
    let mut identity_nodes = snapshot_by_identity(&existing_db)?;
    drop(existing_db);

    if let Some(id_node) = identity_nodes.iter_mut().find(|n| n.name == identity)
        && let Some(rec) = id_node.groups.iter_mut().find(|r| r.name == group_name)
    {
        rec.groups.retain(|g| g.name != TRACKS_GROUP);
        rec.groups.push(track_table_node(tracks));
        rec.attrs.retain(|(k, _)| {
            k != ATTR_SEG_GAP_US && k != ATTR_SEG_DETECT_CLOCK && k != ATTR_SEG_CLOCK_SIGMAS
        });
        rec.attrs.push((
            ATTR_SEG_GAP_US.to_owned(),
            AttrValue::I64(settings.track_split_gap_us),
        ));
        rec.attrs.push((
            ATTR_SEG_DETECT_CLOCK.to_owned(),
            AttrValue::U64(u64::from(settings.detect_clock_discontinuities)),
        ));
        rec.attrs.push((
            ATTR_SEG_CLOCK_SIGMAS.to_owned(),
            AttrValue::F64(settings.clock_discontinuity_sigmas),
        ));
    }

    write_db(&identity_nodes, db_path)?;
    Ok(())
}

/// Read the stored track ranges from a recording group (empty if absent).
pub(crate) fn read_track_table(rec_grp: &hdf5_pure::Group<'_>) -> Vec<TrackRange> {
    let Ok(grp) = rec_grp.group(TRACKS_GROUP) else {
        return Vec::new();
    };
    let read = |name: &str| -> Vec<u64> {
        grp.dataset(name)
            .and_then(|d| d.read_u64())
            .unwrap_or_default()
    };
    let starts = read(TRACK_START_DATASET);
    let ends = read(TRACK_END_DATASET);
    let hidden = read(TRACK_HIDDEN_DATASET);
    gt_types::history::track_ranges_from_columns(&starts, &ends, &hidden).unwrap_or_else(|| {
        log::warn!("Inconsistent track table; ignoring it (tracks will be recomputed)");
        Vec::new()
    })
}

/// Read the stored segmentation settings from a recording's attrs, if present.
fn read_segmentation(
    attrs: &std::collections::HashMap<String, AttrValue>,
) -> Option<StoredSegmentation> {
    let gap = match attrs.get(ATTR_SEG_GAP_US)? {
        AttrValue::I64(v) => *v,
        _ => return None,
    };
    let detect = match attrs.get(ATTR_SEG_DETECT_CLOCK)? {
        AttrValue::U64(v) => *v != 0,
        _ => return None,
    };
    let sigmas = match attrs.get(ATTR_SEG_CLOCK_SIGMAS)? {
        AttrValue::F64(v) => *v,
        _ => return None,
    };
    Some(StoredSegmentation {
        track_split_gap_us: gap,
        detect_clock_discontinuities: detect,
        clock_discontinuity_sigmas: sigmas,
    })
}

/// Read a recording back: reconstructed GTD bytes plus its stored tracks/settings.
pub(crate) fn load_recording(
    db_path: &std::path::Path,
    identity: &str,
    group_name: &str,
) -> Result<StoredRecording, InternalError> {
    let db = hdf5_pure::File::open(db_path)?;
    let by_id = db.root().group("by_identity")?;
    let id_grp = by_id.group(identity)?;
    let rec_grp = id_grp.group(group_name)?;

    let tracks = read_track_table(&rec_grp);

    // Snapshot all child data groups (nav_points, sat_reports, etc.) and
    // write them as a fresh GTD-format HDF5 file.
    let mut fb = FileBuilder::new();

    // Restore GTD root attributes.  Every attr on the recording group that is
    // not a DB-internal field is an GTD root attr and belongs on the file root.
    // Using a denylist (rather than an allowlist) means new GTD attrs are
    // restored automatically.  Fall back to geotrace_version="1" for recordings
    // stored by older code that predates attr preservation.
    let rec_attrs = rec_grp.attrs()?;
    let segmentation = read_segmentation(&rec_attrs);
    let mut has_version = false;
    for (k, v) in &rec_attrs {
        if !is_db_recording_attr(k) {
            fb.set_attr(k, v.clone());
            if k == GTD_VERSION_ATTR {
                has_version = true;
            }
        }
    }
    if !has_version {
        fb.set_attr(
            GTD_VERSION_ATTR,
            AttrValue::String(GTD_VERSION_FALLBACK.to_owned()),
        );
    }

    for child_name in rec_grp.groups()? {
        // Skip the DB-internal track table; it is not part of the GTD file.
        if is_db_internal_group(&child_name) {
            continue;
        }
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
    Ok(StoredRecording {
        bytes,
        tracks,
        segmentation,
    })
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
