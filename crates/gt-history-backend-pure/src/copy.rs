//! Internal read-modify-write machinery for the history database.
//!
//! This module reads existing data into an intermediate tree of owned Rust
//! types, manipulates that tree, then writes the whole thing to a new
//! `FileBuilder` in one pass.

use std::collections::HashMap;

use crate::matches_attrs;
use gt_history_types::{
    ATTR_END_US, ATTR_EVENT_MARKER_COUNT, ATTR_GTD_SIZE_BYTES, ATTR_IDENTITY, ATTR_MARKER_COUNT,
    ATTR_NAV_POINT_COUNT, ATTR_SAT_REPORT_COUNT, ATTR_SEG_CLOCK_SIGMAS, ATTR_SEG_DETECT_CLOCK,
    ATTR_SEG_GAP_US, ATTR_SEG_PLACEMENT_RULE, ATTR_SEG_SPLIT_RULE, ATTR_START_US,
    CURRENT_SCHEMA_VERSION, ChannelSummary, DatabaseRef, DbError, GTD_CHANNEL_COMPONENTS_ATTR,
    GTD_CHANNEL_DESCRIPTION_ATTR, GTD_CHANNEL_TIME_DATASET, GTD_CHANNEL_UNIT_ATTR,
    GTD_CHANNELS_GROUP, GTD_VERSION_ATTR, GTD_VERSION_FALLBACK, LogAttachment, LogAttachmentEntry,
    LogAttachmentId, RecordingMeta, SCHEMA_VERSION_ATTR, SNAP_BLOB_DATASET, SNAP_GROUP,
    StoredFixPlacementRule, StoredRecording, StoredSegmentation, StoredTrackSplitRule,
    TRACK_END_DATASET, TRACK_START_DATASET, TRACK_STATE_DATASET, TRACKS_GROUP, TrackRange,
    TrackState, identity_from_group_name, identity_group_name, is_db_internal_group,
    is_db_recording_attr, log_attachment, make_group_name,
};
use hdf5_pure::{AttrValue, DType, FileBuilder, Group, GroupBuilder};
use thiserror::Error;

#[derive(Debug, Error)]
pub(crate) enum InternalError {
    #[error(transparent)]
    Hdf5(#[from] hdf5_pure::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    /// A rename could not proceed without losing or overwriting data.
    #[error("{0}")]
    Conflict(String),
    #[error("no recording {identity:?}/{group_name:?} in the history database")]
    NoSuchRecording {
        identity: String,
        group_name: String,
    },
}

impl From<InternalError> for DbError {
    fn from(e: InternalError) -> Self {
        match e {
            InternalError::Hdf5(e) => crate::classify_hdf5_error(e),
            InternalError::Io(e) => DbError::Io(e),
            InternalError::Conflict(msg) => DbError::Backend(msg),
            missing @ InternalError::NoSuchRecording { .. } => {
                DbError::Backend(missing.to_string())
            }
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

fn attr_string(attrs: &[(String, AttrValue)], name: &str) -> Option<String> {
    attrs
        .iter()
        .find(|entry| entry.0 == name)
        .and_then(|entry| entry.1.as_str())
        .map(str::to_owned)
}

fn ensure_identity_node<'a>(
    identity_nodes: &'a mut Vec<GroupNode>,
    identity: &str,
) -> &'a mut GroupNode {
    let storage_name = identity_group_name(identity);
    let found = identity_nodes
        .iter()
        .position(|n| n.name == storage_name)
        .or_else(|| {
            (!identity.contains('/'))
                .then(|| identity_nodes.iter().position(|n| n.name == identity))?
        });
    match found {
        Some(index) => {
            let node = &mut identity_nodes[index];
            if attr_string(&node.attrs, ATTR_IDENTITY).is_none() {
                node.attrs.push((
                    ATTR_IDENTITY.to_owned(),
                    AttrValue::String(identity.to_owned()),
                ));
            }
            node
        }
        None => {
            identity_nodes.push(GroupNode {
                name: storage_name,
                attrs: vec![(
                    ATTR_IDENTITY.to_owned(),
                    AttrValue::String(identity.to_owned()),
                )],
                datasets: Vec::new(),
                groups: Vec::new(),
            });
            let index = identity_nodes.len() - 1;
            &mut identity_nodes[index]
        }
    }
}

fn find_identity_node_mut<'a>(
    identity_nodes: &'a mut [GroupNode],
    identity: &str,
) -> Option<&'a mut GroupNode> {
    let storage_name = identity_group_name(identity);
    identity_nodes
        .iter_mut()
        .find(|n| n.name == storage_name || (!identity.contains('/') && n.name == identity))
}

fn find_recording_node_mut<'a>(
    identity_nodes: &'a mut [GroupNode],
    identity: &str,
    group_name: &str,
) -> Option<&'a mut GroupNode> {
    find_identity_node_mut(identity_nodes, identity)?
        .groups
        .iter_mut()
        .find(|rec| rec.name == group_name)
}

fn find_identity_group(by_id: &Group, identity: &str) -> Result<Group, hdf5_pure::Error> {
    let storage_name = identity_group_name(identity);
    match by_id.group(&storage_name) {
        Ok(group) => Ok(group),
        Err(encoded_err) => {
            if identity.contains('/') {
                Err(encoded_err)
            } else {
                by_id.group(identity)
            }
        }
    }
}

/// Write a single [`DatasetNode`] into an open group builder.
fn write_dataset_into(gb: &mut GroupBuilder, ds: &DatasetNode) {
    let chunk = chunk_for_shape(&ds.shape);
    let db = gb.create_dataset(&ds.name);
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
}

/// Write a [`GroupNode`] and everything below it into an open group builder.
///
/// The recursion matters: GTD nests to arbitrary depth (a recording's ad-hoc
/// sensor channels live at `channels/{name}/{time,value}`, two levels below the
/// recording group), and this backend rewrites the whole database on every
/// insert, delete, and rename. A writer that stopped at a fixed depth would
/// silently drop the deeper groups on the next rewrite - which is what
/// `gt_history`'s `channels_survive_a_database_rewrite` test guards against.
fn write_group_into(parent: &mut GroupBuilder, node: &GroupNode) {
    let mut gb = parent.create_group(&node.name);
    for (k, v) in &node.attrs {
        gb.set_attr(k, v.clone());
    }
    for ds in &node.datasets {
        write_dataset_into(&mut gb, ds);
    }
    for child in &node.groups {
        write_group_into(&mut gb, child);
    }
    parent.add_group(gb.finish());
}

/// Recursively read an HDF5 group (its attributes, datasets, and subgroups) into an
/// owned `GroupNode`.
fn snapshot_group(src: &Group, name: &str) -> Result<GroupNode, InternalError> {
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
            attrs: id_grp.attrs()?.into_iter().collect(),
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
/// ranges as parallel `start`/`end`/`state` u64 datasets.
fn track_table_node(tracks: &[TrackRange]) -> GroupNode {
    let n = tracks.len() as u64;
    let (starts, ends, states) = gt_history_types::track_columns(tracks);
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
                name: TRACK_STATE_DATASET.to_owned(),
                shape: vec![n],
                data: DatasetData::U64(states),
                attrs: Vec::new(),
            },
        ],
        groups: Vec::new(),
    }
}

/// The `__geotrace_snap__` subgroup node holding one opaque byte dataset.
fn snap_blob_node(blob: &[u8]) -> GroupNode {
    GroupNode {
        name: SNAP_GROUP.to_owned(),
        attrs: Vec::new(),
        datasets: vec![DatasetNode {
            name: SNAP_BLOB_DATASET.to_owned(),
            shape: vec![blob.len() as u64],
            data: DatasetData::U8(blob.to_vec()),
            attrs: Vec::new(),
        }],
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
            (
                ATTR_START_US.to_owned(),
                AttrValue::I64(meta.stored_start_us()),
            ),
            (ATTR_END_US.to_owned(), AttrValue::I64(meta.stored_end_us())),
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
                ATTR_SEG_SPLIT_RULE.to_owned(),
                AttrValue::I64(settings.track_split_rule.attribute_value()),
            ),
            (
                ATTR_SEG_PLACEMENT_RULE.to_owned(),
                AttrValue::I64(settings.fix_placement_rule.attribute_value()),
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
        for (k, v) in &id_node.attrs {
            id_gb.set_attr(k, v.clone());
        }
        for rec_node in &id_node.groups {
            write_group_into(&mut id_gb, rec_node);
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
) -> Result<DatabaseRef, InternalError> {
    let existing_db = hdf5_pure::File::open(db_path)?;

    // Duplicate check: re-storing a recording already present returns its
    // existing group unchanged (keeping its current track table).
    let duplicate = {
        let root = existing_db.root();
        let mut found = None;
        if let Ok(by_id) = root.group("by_identity") {
            'search: for storage_name in by_id.groups()? {
                let Ok(id_grp) = by_id.group(&storage_name) else {
                    continue;
                };
                let id_attrs = id_grp.attrs()?;
                let existing_identity = id_attrs
                    .get(ATTR_IDENTITY)
                    .and_then(AttrValue::as_str)
                    .map(str::to_owned)
                    .or_else(|| identity_from_group_name(&storage_name))
                    .unwrap_or_else(|| storage_name.clone());
                for rec_name in id_grp.groups()? {
                    if let Ok(rec_grp) = id_grp.group(&rec_name)
                        && let Ok(attrs) = rec_grp.attrs()
                        && matches_attrs(meta, &attrs)
                    {
                        found = Some(DatabaseRef {
                            identity: existing_identity,
                            group_name: rec_name,
                        });
                        break 'search;
                    }
                }
            }
        }
        found
    };

    if let Some(db_ref) = duplicate {
        log::debug!(
            "Recording already in history as identity={:?}, group={:?}",
            db_ref.identity,
            db_ref.group_name
        );
        return Ok(db_ref);
    }

    let mut identity_nodes = snapshot_by_identity(&existing_db)?;

    let rec_name = make_group_name(meta.stored_start_us(), &uuid::Uuid::new_v4().to_string());

    let gtd_file = hdf5_pure::File::from_bytes(gtd_bytes.to_vec())?;
    let new_recording =
        build_new_recording(&gtd_file, &rec_name, meta, identity, tracks, settings)?;

    ensure_identity_node(&mut identity_nodes, identity)
        .groups
        .push(new_recording);

    // Release the in-memory snapshot of the old file before writing the new one,
    // matching the other mutators (`delete_batch`, `set_tracks`, `set_tracks_shelved`).
    drop(existing_db);
    write_db(&identity_nodes, db_path)?;
    log::info!("Stored recording identity={identity:?}, group={rec_name:?} in history database");
    Ok(DatabaseRef {
        identity: identity.to_owned(),
        group_name: rec_name,
    })
}

/// Rewrite one recording's GTD data, metadata attributes, track table and
/// segmentation settings under the group name it already has.
///
/// The recording's log attachment attributes are carried over. Everything else
/// under the group, its snap run included, is replaced by what `gtd_bytes`,
/// `meta`, `tracks` and `settings` describe.
pub(crate) fn replace_recording(
    db_path: &std::path::Path,
    db_ref: &DatabaseRef,
    meta: &RecordingMeta,
    tracks: &[TrackRange],
    settings: StoredSegmentation,
    gtd_bytes: &[u8],
) -> Result<(), InternalError> {
    let existing_db = hdf5_pure::File::open(db_path)?;
    let mut identity_nodes = snapshot_by_identity(&existing_db)?;
    drop(existing_db);

    let Some(stored) =
        find_recording_node_mut(&mut identity_nodes, &db_ref.identity, &db_ref.group_name)
    else {
        return Err(InternalError::NoSuchRecording {
            identity: db_ref.identity.clone(),
            group_name: db_ref.group_name.clone(),
        });
    };

    let gtd_file = hdf5_pure::File::from_bytes(gtd_bytes.to_vec())?;
    let mut replacement = build_new_recording(
        &gtd_file,
        &db_ref.group_name,
        meta,
        &db_ref.identity,
        tracks,
        settings,
    )?;
    replacement.attrs.extend(
        stored
            .attrs
            .iter()
            .filter(|(key, _)| LogAttachmentId::from_attr_key(key).is_some())
            .cloned(),
    );
    *stored = replacement;

    write_db(&identity_nodes, db_path)?;
    log::info!(
        "Replaced recording identity={:?}, group={:?} in history database",
        db_ref.identity,
        db_ref.group_name
    );
    Ok(())
}

/// Remove multiple recordings in a single read-modify-write cycle.
pub(crate) fn delete_batch(
    db_path: &std::path::Path,
    refs: &[gt_history_types::DatabaseRef],
) -> Result<(), InternalError> {
    let existing_db = hdf5_pure::File::open(db_path)?;
    let mut identity_nodes = snapshot_by_identity(&existing_db)?;
    drop(existing_db);

    let mut attachments = Vec::new();
    for db_ref in refs {
        if let Some(id_node) = find_identity_node_mut(&mut identity_nodes, &db_ref.identity) {
            if let Some(rec) = id_node.groups.iter().find(|r| r.name == db_ref.group_name) {
                attachments.extend(
                    rec.attrs
                        .iter()
                        .filter_map(|(key, _)| LogAttachmentId::from_attr_key(key)),
                );
            }
            id_node.groups.retain(|r| r.name != db_ref.group_name);
        }
    }
    identity_nodes.retain(|n| !n.groups.is_empty());

    write_db(&identity_nodes, db_path)?;
    log_attachment::delete_files(
        &log_attachment::logs_directory_for_database(db_path),
        &attachments,
    );
    log::info!("Deleted {} recording(s) in batch prune", refs.len());
    Ok(())
}

/// Overwrite (or add) the `identity` string attribute on a node.
fn set_identity_attr(node: &mut GroupNode, identity: &str) {
    node.attrs.retain(|(k, _)| k != ATTR_IDENTITY);
    node.attrs.push((
        ATTR_IDENTITY.to_owned(),
        AttrValue::String(identity.to_owned()),
    ));
}

/// Rename an identity: move all its recordings under the `new` identity group,
/// rewriting the `identity` attribute on the identity group and every recording.
///
/// If `new` already exists the recordings merge into it. A no-op when `old` is
/// absent or equal to `new`. The whole database is rewritten (as with every pure
/// mutation), so no stale space is left behind.
pub(crate) fn rename_identity(
    db_path: &std::path::Path,
    old: &str,
    new: &str,
) -> Result<(), InternalError> {
    if old == new {
        return Ok(());
    }
    if new.trim().is_empty() {
        log::warn!("rename_identity: not renaming {old:?} to an empty identity");
        return Ok(());
    }

    let existing_db = hdf5_pure::File::open(db_path)?;
    let mut identity_nodes = snapshot_by_identity(&existing_db)?;
    drop(existing_db);

    let old_storage = identity_group_name(old);
    let Some(old_idx) = identity_nodes
        .iter()
        .position(|n| n.name == old_storage || (!old.contains('/') && n.name == old))
    else {
        log::warn!("rename_identity: identity {old:?} not found");
        return Ok(());
    };
    let mut old_node = identity_nodes.remove(old_idx);
    for rec in &mut old_node.groups {
        set_identity_attr(rec, new);
    }

    let new_storage = identity_group_name(new);
    match identity_nodes
        .iter_mut()
        .find(|n| n.name == new_storage || (!new.contains('/') && n.name == new))
    {
        Some(target) => {
            // Merge into the existing target identity group. A recording-group
            // name collision would overwrite one recording with another, so it
            // is rejected. Group names are UUID-suffixed, so this is unreachable
            // in practice, and the whole file is untouched because `write_db`
            // has not run yet.
            if let Some(dup) = old_node
                .groups
                .iter()
                .find(|rec| target.groups.iter().any(|t| t.name == rec.name))
            {
                return Err(InternalError::Conflict(format!(
                    "cannot merge identity {old:?} into {new:?}: recording {:?} exists in both",
                    dup.name
                )));
            }
            target.groups.append(&mut old_node.groups);
        }
        None => {
            old_node.name = new_storage;
            set_identity_attr(&mut old_node, new);
            identity_nodes.push(old_node);
        }
    }

    identity_nodes.retain(|n| !n.groups.is_empty());
    write_db(&identity_nodes, db_path)?;
    log::info!("Renamed history identity {old:?} to {new:?}");
    Ok(())
}

/// Shelve or unshelve the tracks in the stored table rows `rows`, via a
/// read-modify-write cycle.
///
/// A recording whose table predates [`TRACK_STATE_DATASET`] comes out of this
/// with a state column: the whole table is rewritten, tombstones and all.
pub(crate) fn set_tracks_shelved(
    db_path: &std::path::Path,
    identity: &str,
    group_name: &str,
    rows: &[usize],
    shelved: bool,
) -> Result<(), InternalError> {
    let existing_db = hdf5_pure::File::open(db_path)?;
    let stored_tracks = existing_db
        .root()
        .group("by_identity")
        .and_then(|by_id| find_identity_group(&by_id, identity))
        .and_then(|id_grp| id_grp.group(group_name))
        .ok()
        .and_then(|rec_grp| stored_track_table(&rec_grp));
    let Some(mut tracks) = stored_tracks else {
        log::warn!("Shelving tracks of {identity}/{group_name}, which has no track table");
        return Ok(());
    };
    let mut identity_nodes = snapshot_by_identity(&existing_db)?;
    drop(existing_db);

    let state = if shelved {
        TrackState::Shelved
    } else {
        TrackState::Live
    };
    for row in gt_history_types::set_state_of_stored_rows(&mut tracks, rows, state) {
        log::warn!(
            "Track row {row} of {identity}/{group_name} holds no live or shelved track: it is past the end of the stored track table, or it holds a permanently deleted one"
        );
    }
    if let Some(rec) = find_recording_node_mut(&mut identity_nodes, identity, group_name) {
        rec.groups.retain(|g| g.name != TRACKS_GROUP);
        rec.groups.push(track_table_node(&tracks));
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

    if let Some(id_node) = find_identity_node_mut(&mut identity_nodes, identity)
        && let Some(rec) = id_node.groups.iter_mut().find(|r| r.name == group_name)
    {
        rec.groups.retain(|g| g.name != TRACKS_GROUP);
        rec.groups.push(track_table_node(tracks));
        rec.attrs.retain(|(k, _)| {
            k != ATTR_SEG_GAP_US
                && k != ATTR_SEG_SPLIT_RULE
                && k != ATTR_SEG_PLACEMENT_RULE
                && k != ATTR_SEG_DETECT_CLOCK
                && k != ATTR_SEG_CLOCK_SIGMAS
        });
        rec.attrs.push((
            ATTR_SEG_GAP_US.to_owned(),
            AttrValue::I64(settings.track_split_gap_us),
        ));
        rec.attrs.push((
            ATTR_SEG_SPLIT_RULE.to_owned(),
            AttrValue::I64(settings.track_split_rule.attribute_value()),
        ));
        rec.attrs.push((
            ATTR_SEG_PLACEMENT_RULE.to_owned(),
            AttrValue::I64(settings.fix_placement_rule.attribute_value()),
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

/// Replace a recording's stored snap run with the opaque bytes in `blob`.
/// A no-op when the recording is absent, like [`set_tracks`].
pub(crate) fn set_snap_blob(
    db_path: &std::path::Path,
    identity: &str,
    group_name: &str,
    blob: &[u8],
) -> Result<(), InternalError> {
    let existing_db = hdf5_pure::File::open(db_path)?;
    let mut identity_nodes = snapshot_by_identity(&existing_db)?;
    drop(existing_db);

    if let Some(id_node) = find_identity_node_mut(&mut identity_nodes, identity)
        && let Some(rec) = id_node.groups.iter_mut().find(|r| r.name == group_name)
    {
        rec.groups.retain(|g| g.name != SNAP_GROUP);
        rec.groups.push(snap_blob_node(blob));
    }

    write_db(&identity_nodes, db_path)?;
    Ok(())
}

/// The stored snap run bytes of a recording, `None` when it carries none.
pub(crate) fn snap_blob(
    db_path: &std::path::Path,
    identity: &str,
    group_name: &str,
) -> Result<Option<Vec<u8>>, InternalError> {
    let file = hdf5_pure::File::open(db_path)?;
    let root = file.root();
    let by_id = root.group("by_identity")?;
    let id_grp = find_identity_group(&by_id, identity)?;
    let rec_grp = id_grp.group(group_name)?;
    let Ok(grp) = rec_grp.group(SNAP_GROUP) else {
        return Ok(None);
    };
    let Ok(dataset) = grp.dataset(SNAP_BLOB_DATASET) else {
        return Ok(None);
    };
    Ok(dataset.read_u8().ok())
}

/// Every row of a recording's stored track table, tombstones and all. Empty
/// for a recording stored before per-track storage existed, and for one whose
/// columns are inconsistent.
pub(crate) fn stored_track_table_of_recording(
    db_path: &std::path::Path,
    identity: &str,
    group_name: &str,
) -> Result<Vec<TrackRange>, InternalError> {
    let file = hdf5_pure::File::open(db_path)?;
    let root = file.root();
    let by_id = root.group("by_identity")?;
    let id_grp = find_identity_group(&by_id, identity)?;
    let rec_grp = id_grp.group(group_name)?;
    Ok(stored_track_table(&rec_grp).unwrap_or_default())
}

/// Every log attached to a recording, in the order
/// [`LogAttachmentEntry::sort_by_name_then_id`] puts them.
pub(crate) fn log_attachments(
    db_path: &std::path::Path,
    identity: &str,
    group_name: &str,
) -> Result<Vec<LogAttachmentEntry>, InternalError> {
    let file = hdf5_pure::File::open(db_path)?;
    let root = file.root();
    let by_id = root.group("by_identity")?;
    let id_grp = find_identity_group(&by_id, identity)?;
    let rec_grp = id_grp.group(group_name)?;
    Ok(log_attachments_in_attrs(&rec_grp.attrs()?))
}

/// The attachments a recording's attributes hold, for a caller that has read
/// those attributes already.
pub(crate) fn log_attachments_in_attrs(
    attrs: &HashMap<String, AttrValue>,
) -> Vec<LogAttachmentEntry> {
    let mut entries = Vec::new();
    for (key, value) in attrs {
        let Some(id) = LogAttachmentId::from_attr_key(key) else {
            continue;
        };
        let Some(json) = value.as_str() else {
            log::warn!("Ignoring the log attachment attribute {key:?}, which is not a string");
            continue;
        };
        if let Some(attachment) = LogAttachment::from_attribute_json(json) {
            entries.push(LogAttachmentEntry { id, attachment });
        }
    }
    LogAttachmentEntry::sort_by_name_then_id(&mut entries);
    entries
}

/// Store one attachment's attribute JSON on a recording, replacing whatever
/// was stored under the same id.
pub(crate) fn set_log_attachment_attribute(
    db_path: &std::path::Path,
    identity: &str,
    group_name: &str,
    id: LogAttachmentId,
    attribute_json: &str,
) -> Result<(), InternalError> {
    let existing_db = hdf5_pure::File::open(db_path)?;
    let mut identity_nodes = snapshot_by_identity(&existing_db)?;
    drop(existing_db);

    let key = id.attr_key();
    let Some(rec) = find_recording_node_mut(&mut identity_nodes, identity, group_name) else {
        return Err(InternalError::NoSuchRecording {
            identity: identity.to_owned(),
            group_name: group_name.to_owned(),
        });
    };
    rec.attrs.retain(|(existing, _)| *existing != key);
    rec.attrs
        .push((key, AttrValue::String(attribute_json.to_owned())));

    write_db(&identity_nodes, db_path)
}

/// Remove one attachment's attribute. A recording that carries no such
/// attachment, or that is gone entirely, leaves nothing to remove.
pub(crate) fn delete_log_attachment_attribute(
    db_path: &std::path::Path,
    identity: &str,
    group_name: &str,
    id: LogAttachmentId,
) -> Result<(), InternalError> {
    let existing_db = hdf5_pure::File::open(db_path)?;
    let mut identity_nodes = snapshot_by_identity(&existing_db)?;
    drop(existing_db);

    let key = id.attr_key();
    match find_recording_node_mut(&mut identity_nodes, identity, group_name) {
        Some(rec) => rec.attrs.retain(|(existing, _)| *existing != key),
        None => log::warn!(
            "Removing log attachment {id} from {identity}/{group_name}, which is not in the database"
        ),
    }

    write_db(&identity_nodes, db_path)
}

/// Every row of a recording's stored track table, tombstones and all.
fn stored_track_table(rec_grp: &Group) -> Option<Vec<TrackRange>> {
    let grp = rec_grp.group(TRACKS_GROUP).ok()?;
    gt_history_types::stored_track_table(|name| grp.dataset(name).and_then(|d| d.read_u64()).ok())
}

/// The tracks a recording has: its [`stored_track_table`] without the
/// [`TrackState::Deleted`] tombstones a permanent delete leaves.
pub(crate) fn read_track_table(rec_grp: &Group) -> Vec<TrackRange> {
    stored_track_table(rec_grp)
        .unwrap_or_default()
        .into_iter()
        .filter(|track| track.state != TrackState::Deleted)
        .collect()
}

/// Summarize the recording's ad-hoc sensor channels for the History listing.
///
/// Reads only each channel's metadata attributes and its `time` dataset shape -
/// never its samples - so listing a database of channel-rich recordings stays
/// cheap. Recordings without channels have no [`GTD_CHANNELS_GROUP`] and
/// summarize to nothing. Sorted by name, matching how the SDK's reader orders
/// them.
pub(crate) fn read_channel_summaries(rec_grp: &Group) -> Vec<ChannelSummary> {
    let Ok(root) = rec_grp.group(GTD_CHANNELS_GROUP) else {
        return Vec::new();
    };
    let Ok(names) = root.groups() else {
        return Vec::new();
    };

    let mut summaries: Vec<ChannelSummary> = names
        .into_iter()
        .filter_map(|name| {
            let grp = root.group(&name).ok()?;
            let attrs = grp.attrs().unwrap_or_default();
            let sample_count = grp
                .dataset(GTD_CHANNEL_TIME_DATASET)
                .and_then(|d| d.shape())
                .ok()
                .and_then(|shape| shape.first().copied())
                .unwrap_or(0);
            Some(ChannelSummary {
                name,
                unit: crate::string_attr(&attrs, GTD_CHANNEL_UNIT_ATTR),
                description: crate::string_attr(&attrs, GTD_CHANNEL_DESCRIPTION_ATTR),
                components: attr_string_array_value(&attrs, GTD_CHANNEL_COMPONENTS_ATTR),
                sample_count,
            })
        })
        .collect();
    ChannelSummary::sort_by_name(&mut summaries);
    summaries
}

/// An array-of-strings attribute's values, or empty when it is absent or
/// another type.
fn attr_string_array_value(
    attrs: &std::collections::HashMap<String, AttrValue>,
    name: &str,
) -> Vec<String> {
    attrs
        .get(name)
        .and_then(AttrValue::as_strings)
        .map(<[String]>::to_vec)
        .unwrap_or_default()
}

/// Read the stored segmentation settings from a recording's attributes, if present.
fn read_segmentation(
    attrs: &std::collections::HashMap<String, AttrValue>,
) -> Option<StoredSegmentation> {
    Some(StoredSegmentation {
        track_split_gap_us: attrs.get(ATTR_SEG_GAP_US).and_then(AttrValue::as_i64)?,
        track_split_rule: StoredTrackSplitRule::from_attribute_value(
            attrs.get(ATTR_SEG_SPLIT_RULE).and_then(AttrValue::as_i64),
        ),
        fix_placement_rule: StoredFixPlacementRule::from_attribute_value(
            attrs
                .get(ATTR_SEG_PLACEMENT_RULE)
                .and_then(AttrValue::as_i64),
        ),
        detect_clock_discontinuities: attrs
            .get(ATTR_SEG_DETECT_CLOCK)
            .and_then(AttrValue::as_u64)?
            != 0,
        clock_discontinuity_sigmas: attrs
            .get(ATTR_SEG_CLOCK_SIGMAS)
            .and_then(AttrValue::as_f64)?,
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
    let id_grp = find_identity_group(&by_id, identity)?;
    let rec_grp = id_grp.group(group_name)?;

    let tracks = stored_track_table(&rec_grp).unwrap_or_default();

    // Snapshot all child data groups (`nav_points`, `sat_reports`, etc.) and
    // write them as a fresh GTD-format HDF5 file.
    let mut fb = FileBuilder::new();

    // Restore GTD root attributes.  Every attribute on the recording group that
    // is not a DB-internal field is a GTD root attribute and belongs on the file
    // root.  The denylist restores newly added GTD attributes automatically.
    // Fall back to geotrace_version="1" for recordings stored by older code that
    // predates attribute preservation.
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
        // Skip the DB-internal groups. They are not part of the GTD file.
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
            write_dataset_into(&mut gb, ds);
        }
        for sg in &node.groups {
            write_group_into(&mut gb, sg);
        }
        fb.add_group(gb.finish());
    }

    // hdf5-pure can only write to a path. Write a sibling temp file then read
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
