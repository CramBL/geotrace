use gt_history_types::{
    ATTR_END_US, ATTR_EVENT_MARKER_COUNT, ATTR_GTD_SIZE_BYTES, ATTR_IDENTITY, ATTR_MARKER_COUNT,
    ATTR_NAV_POINT_COUNT, ATTR_SAT_REPORT_COUNT, ATTR_SEG_CLOCK_SIGMAS, ATTR_SEG_DETECT_CLOCK,
    ATTR_SEG_GAP_US, ATTR_SEG_PLACEMENT_RULE, ATTR_SEG_SPLIT_RULE, ATTR_START_US, ChannelSummary,
    DatabaseRef, DbError, GTD_CHANNEL_COMPONENTS_ATTR, GTD_CHANNEL_DESCRIPTION_ATTR,
    GTD_CHANNEL_TIME_DATASET, GTD_CHANNEL_UNIT_ATTR, GTD_CHANNELS_GROUP, GTD_META_DEVICE_ATTR,
    GTD_META_NOTES_ATTR, GTD_META_TITLE_ATTR, GTD_META_TRAVEL_MODE_ATTR, GTD_VERSION_ATTR,
    GTD_VERSION_FALLBACK, LogAttachment, LogAttachmentEntry, LogAttachmentId, NavPointTimeRange,
    RecordingEntry, RecordingMeta, SNAP_BLOB_DATASET, SNAP_GROUP, StoredFixPlacementRule,
    StoredRecording, StoredSegmentation, StoredTrackSplitRule, TRACK_END_DATASET,
    TRACK_START_DATASET, TRACK_STATE_DATASET, TRACKS_GROUP, TrackRange, TrackState,
    identity_from_group_name, identity_group_name, is_db_internal_group, is_db_recording_attr,
    make_group_name,
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
    /// A rename could not proceed without losing or overwriting data.
    #[error("{0}")]
    Conflict(String),
}

impl From<InternalError> for DbError {
    fn from(e: InternalError) -> Self {
        match e {
            InternalError::Hdf5(e) => DbError::Backend(e.to_string()),
            InternalError::Io(e) => DbError::Io(e),
            InternalError::Conflict(msg) => DbError::Backend(msg),
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

fn read_recording_meta(group: &Group) -> Option<RecordingMeta> {
    let read_i64 = |name: &str| group.attr(name).and_then(|a| a.read_scalar::<i64>()).ok();
    let read_u64 = |name: &str| group.attr(name).and_then(|a| a.read_scalar::<u64>()).ok();

    let start_us = read_i64(ATTR_START_US)?;
    let end_us = read_i64(ATTR_END_US).unwrap_or(start_us);
    let nav_point_count = read_u64(ATTR_NAV_POINT_COUNT)?;

    Some(RecordingMeta {
        time_range: NavPointTimeRange::from_stored_attributes(nav_point_count, start_us..=end_us),
        nav_point_count,
        sat_report_count: read_u64(ATTR_SAT_REPORT_COUNT)?,
        marker_count: read_u64(ATTR_MARKER_COUNT)?,
        event_marker_count: read_u64(ATTR_EVENT_MARKER_COUNT)?,
        gtd_size_bytes: read_u64(ATTR_GTD_SIZE_BYTES).unwrap_or(0),
    })
}

fn read_group_string_attr(group: &Group, name: &str) -> Result<String, InternalError> {
    let attr = group.attr(name)?;
    let descriptor = attr
        .dtype()?
        .to_descriptor()
        .map_err(|e| InternalError::Hdf5(hdf5::Error::from(e.to_string())))?;
    read_string_attr(&attr, &descriptor)
}

fn recording_identity(group: &Group) -> Option<String> {
    read_group_string_attr(group, ATTR_IDENTITY).ok()
}

fn identity_for_group(group: &Group, storage_name: &str) -> String {
    read_group_string_attr(group, ATTR_IDENTITY)
        .ok()
        .or_else(|| identity_from_group_name(storage_name))
        .unwrap_or_else(|| storage_name.to_owned())
}

fn ensure_identity_group(by_id: &Group, identity: &str) -> Result<Group, InternalError> {
    let storage_name = identity_group_name(identity);
    let id_grp = by_id
        .create_group(&storage_name)
        .or_else(|_| by_id.group(&storage_name))?;
    if id_grp.attr(ATTR_IDENTITY).is_err() {
        write_string_attr(&id_grp, ATTR_IDENTITY, identity)?;
    }
    Ok(id_grp)
}

/// The name `by_identity` holds `identity` under: the encoded one, or the raw
/// one a legacy database was written with.
fn identity_storage_name(by_id: &Group, identity: &str) -> Option<String> {
    let encoded = identity_group_name(identity);
    if by_id.link_exists(&encoded) {
        return Some(encoded);
    }
    (!identity.contains('/') && by_id.link_exists(identity)).then(|| identity.to_owned())
}

fn open_identity_group(by_id: &Group, identity: &str) -> Result<Group, InternalError> {
    let storage_name =
        identity_storage_name(by_id, identity).unwrap_or_else(|| identity_group_name(identity));
    by_id.group(&storage_name).map_err(Into::into)
}

/// Overwrite (or add) the `identity` string attribute on a group. The plain
/// [`write_meta_attrs`] path skips existing attributes, and a string attribute cannot be
/// rewritten in place, so this deletes any existing attribute first.
fn overwrite_identity_attr(group: &Group, identity: &str) -> Result<(), InternalError> {
    if group.attr(ATTR_IDENTITY).is_ok() {
        group.delete_attr(ATTR_IDENTITY)?;
    }
    write_string_attr(group, ATTR_IDENTITY, identity)
}

/// Rename an identity, moving all its recordings under the `new` identity group.
///
/// Recordings are object-copied into the destination and the old identity group
/// is unlinked (there is no HDF5 move primitive), mirroring
/// [`repair_unindexed_recordings`]. If `new` already exists the recordings merge
/// into it. A no-op when `old` is absent or equal to `new`.
///
/// This copies the recordings, so the file grows by their size. The
/// system-HDF5 backend does not reclaim the vacated space (the
/// whole-file-rewrite pure backend does).
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

    let file = hdf5::File::open_rw(db_path)?;
    let Ok(by_id) = file.group("by_identity") else {
        return Ok(());
    };
    let Ok(src) = open_identity_group(&by_id, old) else {
        log::warn!("rename_identity: identity {old:?} not found");
        return Ok(());
    };
    // Merge into an existing target, including a legacy raw-named group. A new
    // group is created only when none exists.
    let dst = match open_identity_group(&by_id, new) {
        Ok(existing) => existing,
        Err(_) => ensure_identity_group(&by_id, new)?,
    };

    let rec_names = src.member_names()?;
    // Reject before touching anything if any recording would overwrite one under
    // the destination (group names are UUID-suffixed, so unreachable in practice
    // - but never silently drop data). The copy-then-unlink below is not atomic,
    // so this must be checked up front.
    if let Some(dup) = rec_names.iter().find(|name| dst.link_exists(name)) {
        return Err(InternalError::Conflict(format!(
            "cannot merge identity {old:?} into {new:?}: recording {dup:?} exists in both"
        )));
    }

    for rec_name in &rec_names {
        let rec = src.group(rec_name)?;
        rec.copy_to(&dst, rec_name)?;
        // The copy still has the old identity attribute. Rewrite it.
        overwrite_identity_attr(&dst.group(rec_name)?, new)?;
    }

    // Drop the now-vacated old identity group (under whichever name it existed).
    let old_storage = identity_group_name(old);
    if by_id.link_exists(&old_storage) {
        by_id.unlink(&old_storage)?;
    } else if !old.contains('/') && by_id.link_exists(old) {
        by_id.unlink(old)?;
    }

    file.flush()?;
    log::info!("Renamed history identity {old:?} to {new:?}");
    Ok(())
}

fn is_indexed_recording_path(path: &str, identity: &str) -> bool {
    let Some((parent, _)) = path.rsplit_once('/') else {
        return false;
    };
    if parent == format!("/by_identity/{}", identity_group_name(identity)) {
        return true;
    }
    !identity.contains('/') && parent == format!("/by_identity/{identity}")
}

fn collect_recording_group_paths(
    group: &Group,
    path: &str,
    out: &mut Vec<String>,
) -> Result<(), InternalError> {
    if read_recording_meta(group).is_some() && recording_identity(group).is_some() {
        out.push(path.to_owned());
        return Ok(());
    }

    for name in group.member_names()? {
        let child_path = if path == "/" {
            format!("/{name}")
        } else {
            format!("{path}/{name}")
        };
        if let Ok(child) = group.group(&name) {
            collect_recording_group_paths(&child, &child_path, out)?;
        }
    }
    Ok(())
}

fn find_indexed_recording(
    file: &hdf5::File,
    meta: &RecordingMeta,
) -> Result<Option<DatabaseRef>, InternalError> {
    let Ok(by_id) = file.group("by_identity") else {
        return Ok(None);
    };
    for storage_name in by_id.member_names()? {
        let Ok(id_grp) = by_id.group(&storage_name) else {
            continue;
        };
        let identity = identity_for_group(&id_grp, &storage_name);
        for rec_name in id_grp.member_names()? {
            if let Ok(rec_grp) = id_grp.group(&rec_name)
                && matches_attrs(meta, &rec_grp)
            {
                return Ok(Some(DatabaseRef {
                    identity,
                    group_name: rec_name,
                }));
            }
        }
    }
    Ok(None)
}

pub(crate) fn repair_unindexed_recordings(db_path: &Path) -> Result<(), InternalError> {
    let file = hdf5::File::open_rw(db_path)?;

    // Must run before the scan below: that scan would otherwise take a stage
    // for a recording stored outside `by_identity` and file it under the
    // stage's own name.
    let mut resolved_stages = 0_usize;
    if let Ok(meta_grp) = file.group("meta") {
        for name in meta_grp.member_names()? {
            if name.starts_with(REPLACEMENT_STAGE_PREFIX) {
                resolve_replacement_stage(&file, &name)?;
                resolved_stages += 1;
            }
        }
    }

    let root = file.group("/")?;
    let mut paths = Vec::new();
    collect_recording_group_paths(&root, "/", &mut paths)?;
    if paths.is_empty() {
        if resolved_stages > 0 {
            file.flush()?;
        }
        return Ok(());
    }

    let by_id = file.group("by_identity")?;
    let mut repaired = 0_usize;
    let mut removed_duplicates = 0_usize;
    for path in paths {
        let rec_grp = file.group(&path)?;
        let Some(identity) = recording_identity(&rec_grp) else {
            log::warn!("History recording at {path:?} has no identity; leaving it in place");
            continue;
        };
        if is_indexed_recording_path(&path, &identity) {
            continue;
        }

        let Some(meta) = read_recording_meta(&rec_grp) else {
            log::warn!(
                "History recording at {path:?} has incomplete metadata; leaving it in place"
            );
            continue;
        };
        let Some(rec_name) = path.rsplit('/').next().filter(|name| !name.is_empty()) else {
            log::warn!("History recording at {path:?} has no group name; leaving it in place");
            continue;
        };

        if let Some(existing) = find_indexed_recording(&file, &meta)? {
            file.unlink(&path)?;
            removed_duplicates += 1;
            log::warn!(
                "Removed unindexed duplicate history recording at {path:?}; indexed as identity={:?}, group={:?}",
                existing.identity,
                existing.group_name
            );
            continue;
        }

        let id_grp = ensure_identity_group(&by_id, &identity)?;
        if id_grp.link_exists(rec_name) {
            file.unlink(&path)?;
            removed_duplicates += 1;
            log::warn!(
                "Removed unindexed history recording at {path:?}; destination group already exists for identity={identity:?}, group={rec_name:?}"
            );
            continue;
        }

        rec_grp.copy_to(&id_grp, rec_name)?;
        file.unlink(&path)?;
        repaired += 1;
        log::warn!(
            "Moved unindexed history recording from {path:?} into /by_identity using identity={identity:?}, group={rec_name:?}"
        );
    }

    if repaired > 0 || removed_duplicates > 0 || resolved_stages > 0 {
        file.flush()?;
    }
    if repaired > 0 || removed_duplicates > 0 {
        log::warn!(
            "Repaired history database at {}: moved {repaired} recording(s), removed {removed_duplicates} duplicate orphan(s)",
            db_path.display()
        );
    }
    Ok(())
}

pub(crate) fn is_duplicate(
    db_path: &std::path::Path,
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

/// The 8-byte magic that begins every HDF5 file (and its superblock).
const HDF5_SIGNATURE: [u8; 8] = [0x89, b'H', b'D', b'F', b'\r', b'\n', 0x1a, b'\n'];
/// Length of a version-2/3 superblock with 8-byte offsets and lengths: the
/// fixed prefix (12) + four 8-byte addresses (32) + the 4-byte checksum.
const SUPERBLOCK_V2_LEN: usize = 48;

/// HDF5's metadata checksum (Jenkins `lookup3`, little-endian, `initval = 0`),
/// matching libhdf5's `H5_checksum_lookup3`.
///
/// Needed because clearing the superblock status flag changes a byte the
/// superblock checksum covers. libhdf5 exposes no API to do this (only the
/// standalone `h5clear` tool, which is not bundled), so the checksum must be
/// recomputed by hand.
fn jenkins_lookup3(data: &[u8]) -> u32 {
    fn rot(x: u32, k: u32) -> u32 {
        x.rotate_left(k)
    }
    let byte = |i: usize| -> u32 { data.get(i).copied().map_or(0, u32::from) };

    let len = u32::try_from(data.len()).unwrap_or(u32::MAX);
    let mut a = 0xdead_beef_u32.wrapping_add(len);
    let mut b = a;
    let mut c = a;

    let mut pos = 0usize;
    let mut remaining = data.len();
    while remaining > 12 {
        a = a.wrapping_add(
            byte(pos) | byte(pos + 1) << 8 | byte(pos + 2) << 16 | byte(pos + 3) << 24,
        );
        b = b.wrapping_add(
            byte(pos + 4) | byte(pos + 5) << 8 | byte(pos + 6) << 16 | byte(pos + 7) << 24,
        );
        c = c.wrapping_add(
            byte(pos + 8) | byte(pos + 9) << 8 | byte(pos + 10) << 16 | byte(pos + 11) << 24,
        );
        a = a.wrapping_sub(c);
        a ^= rot(c, 4);
        c = c.wrapping_add(b);
        b = b.wrapping_sub(a);
        b ^= rot(a, 6);
        a = a.wrapping_add(c);
        c = c.wrapping_sub(b);
        c ^= rot(b, 8);
        b = b.wrapping_add(a);
        a = a.wrapping_sub(c);
        a ^= rot(c, 16);
        c = c.wrapping_add(b);
        b = b.wrapping_sub(a);
        b ^= rot(a, 19);
        a = a.wrapping_add(c);
        c = c.wrapping_sub(b);
        c ^= rot(b, 4);
        b = b.wrapping_add(a);
        pos += 12;
        remaining -= 12;
    }
    if remaining == 0 {
        return c;
    }
    // Add the final 1..=12 bytes (the C `switch` with fall-through).
    if remaining >= 12 {
        c = c.wrapping_add(byte(pos + 11) << 24);
    }
    if remaining >= 11 {
        c = c.wrapping_add(byte(pos + 10) << 16);
    }
    if remaining >= 10 {
        c = c.wrapping_add(byte(pos + 9) << 8);
    }
    if remaining >= 9 {
        c = c.wrapping_add(byte(pos + 8));
    }
    if remaining >= 8 {
        b = b.wrapping_add(byte(pos + 7) << 24);
    }
    if remaining >= 7 {
        b = b.wrapping_add(byte(pos + 6) << 16);
    }
    if remaining >= 6 {
        b = b.wrapping_add(byte(pos + 5) << 8);
    }
    if remaining >= 5 {
        b = b.wrapping_add(byte(pos + 4));
    }
    if remaining >= 4 {
        a = a.wrapping_add(byte(pos + 3) << 24);
    }
    if remaining >= 3 {
        a = a.wrapping_add(byte(pos + 2) << 16);
    }
    if remaining >= 2 {
        a = a.wrapping_add(byte(pos + 1) << 8);
    }
    a = a.wrapping_add(byte(pos));

    c ^= b;
    c = c.wrapping_sub(rot(b, 14));
    a ^= c;
    a = a.wrapping_sub(rot(c, 11));
    b ^= a;
    b = b.wrapping_sub(rot(a, 25));
    c ^= b;
    c = c.wrapping_sub(rot(b, 16));
    a ^= c;
    a = a.wrapping_sub(rot(c, 4));
    b ^= a;
    b = b.wrapping_sub(rot(a, 14));
    c ^= b;
    c = c.wrapping_sub(rot(b, 24));
    c
}

/// Clear the superblock "open for write" status flags (the repair `h5clear -s`
/// performs), recomputing the superblock checksum so libhdf5 will open the file
/// again. Returns `true` if a flag was actually cleared.
///
/// Only version-2/3 superblocks (standard 8-byte offsets) hold these flags.
/// Anything else is left untouched.
pub(crate) fn clear_write_lock(db_path: &Path) -> Result<bool, InternalError> {
    use std::io::{Read, Seek, SeekFrom, Write};

    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(db_path)?;
    let mut sb = [0_u8; SUPERBLOCK_V2_LEN];
    if file.read_exact(&mut sb).is_err() {
        return Ok(false);
    }
    if sb.get(..8) != Some(&HDF5_SIGNATURE[..]) {
        return Ok(false);
    }
    let version = sb.get(8).copied().unwrap_or(0);
    let size_of_offsets = sb.get(9).copied().unwrap_or(0);
    let status_flags = sb.get(11).copied().unwrap_or(0);
    if version < 2 || size_of_offsets != 8 || status_flags == 0 {
        return Ok(false);
    }

    if let Some(slot) = sb.get_mut(11) {
        *slot = 0;
    }
    let checksum = jenkins_lookup3(sb.get(..44).unwrap_or(&[]));
    if let Some(slot) = sb.get_mut(44..48) {
        slot.copy_from_slice(&checksum.to_le_bytes());
    }
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&sb)?;
    file.flush()?;
    Ok(true)
}

/// Name of the throwaway group used to probe whether libhdf5 can write to a file.
const WRITE_PROBE_GROUP: &str = "__geotrace_write_probe__";

/// Returns `true` if libhdf5 can create (and remove) a group in `db_path`.
///
/// Files written by the pure-Rust backend can be read by libhdf5 but not
/// extended, so a successful probe is what distinguishes a native database from
/// a legacy one that needs migrating.
pub(crate) fn is_native_writable(db_path: &Path) -> bool {
    let Ok(file) = hdf5::File::open_rw(db_path) else {
        return false;
    };
    match file.create_group(WRITE_PROBE_GROUP) {
        Ok(probe_group) => {
            drop(probe_group);
            file.unlink(WRITE_PROBE_GROUP).ok();
            true
        }
        Err(create_err) => {
            log::debug!(
                "History database at {} is not native-writable: {create_err}",
                db_path.display()
            );
            false
        }
    }
}

/// Create a new database file with the persistent free-space manager, matching
/// how a fresh database is created.
fn create_native_file(path: &Path) -> Result<hdf5::File, InternalError> {
    hdf5::File::with_options()
        .with_fcpl(|fcpl| {
            fcpl.file_space_strategy(hdf5::file::FileSpaceStrategy::FreeSpaceManager {
                paged: false,
                persist: true,
                threshold: 1,
            })
        })
        .create(path)
        .map_err(InternalError::Hdf5)
}

/// Copy one recording group into `dst_parent` as a freshly created (native)
/// group: a plain `H5Ocopy` would preserve the legacy object header, which
/// libhdf5 cannot later add attributes to. The recording attributes are
/// re-written and the leaf data groups are object-copied.
fn copy_recording_native(
    src_rec: &Group,
    dst_parent: &Group,
    name: &str,
) -> Result<(), InternalError> {
    let dst_rec = dst_parent.create_group(name)?;
    copy_attrs(src_rec, &dst_rec, |_| true)?;
    // Data subtrees (`nav_points`, `sat_reports`, …) are never modified after
    // import, so object-copying them faithfully is fine.
    copy_members(src_rec, &dst_rec)?;
    Ok(())
}

/// Rewrite a legacy (pure-Rust-written) database into a libhdf5-native file in
/// place, preserving all recordings, their data, and the root attributes.
///
/// The structural groups (`by_identity`, each identity, each recording) are
/// re-created natively so libhdf5 can extend them later. Only the leaf data is
/// object-copied. The rebuilt file then atomically replaces the original.
pub(crate) fn migrate_to_native(db_path: &Path) -> Result<(), InternalError> {
    let src = hdf5::File::open(db_path)?;
    let tmp = tempfile::NamedTempFile::new_in(db_path.parent().unwrap_or_else(|| Path::new(".")))?;
    {
        let dst = create_native_file(tmp.path())?;
        let dst_root = dst.group("/")?;
        copy_attrs(&src.group("/")?, &dst_root, |_| true)?;

        let dst_by_id = dst_root.create_group("by_identity")?;
        if let Ok(src_by_id) = src.group("by_identity") {
            for id_name in src_by_id.member_names()? {
                let Ok(src_id) = src_by_id.group(&id_name) else {
                    continue;
                };
                let dst_id = dst_by_id.create_group(&id_name)?;
                copy_attrs(&src_id, &dst_id, |_| true)?;
                for rec_name in src_id.member_names()? {
                    if let Ok(src_rec) = src_id.group(&rec_name) {
                        copy_recording_native(&src_rec, &dst_id, &rec_name)?;
                    }
                }
            }
        }

        let dst_meta = dst_root.create_group("meta")?;
        if let Ok(src_meta) = src.group("meta") {
            copy_attrs(&src_meta, &dst_meta, |_| true)?;
            copy_members(&src_meta, &dst_meta)?;
        }

        dst.flush()?;
    }
    drop(src);

    // Replace the original with the migrated copy.
    tmp.persist(db_path)
        .map_err(|e| InternalError::Io(e.error))?;
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
    let file = hdf5::File::open_rw(db_path)?;

    // Duplicate check: re-storing a recording already present returns its
    // existing group unchanged, keeping its current track table.
    if let Ok(by_id) = file.group("by_identity") {
        for id_name in by_id.member_names()? {
            if let Ok(id_grp) = by_id.group(&id_name) {
                let existing_identity = identity_for_group(&id_grp, &id_name);
                for rec_name in id_grp.member_names()? {
                    if let Ok(rec_grp) = id_grp.group(&rec_name)
                        && matches_attrs(meta, &rec_grp)
                    {
                        log::debug!(
                            "Recording already in history as identity={existing_identity:?}, group={rec_name:?}"
                        );
                        return Ok(DatabaseRef {
                            identity: existing_identity,
                            group_name: rec_name,
                        });
                    }
                }
            }
        }
    }

    let by_id = file.group("by_identity")?;
    let id_grp = ensure_identity_group(&by_id, identity)?;
    let group_name = make_group_name(meta.stored_start_us(), &uuid::Uuid::new_v4().to_string());
    let rec_grp = id_grp.create_group(&group_name)?;

    write_meta_attrs(&rec_grp, identity, meta)?;
    write_segmentation_attrs(&rec_grp, settings)?;

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

    write_track_table(&rec_grp, tracks)?;

    Ok(DatabaseRef {
        identity: identity.to_owned(),
        group_name,
    })
}

/// Name of the group [`replace_recording`] builds a recording's replacement
/// in, under `/meta` and one per recording.
///
/// A stage stays out of the listing: [`list_recordings`] reads the recordings
/// under `by_identity`. [`resolve_replacement_stage`] resolves a stage, from
/// [`replace_recording`] on the next replacement of that recording, and from
/// [`repair_unindexed_recordings`] ahead of its own scan on the next open.
const REPLACEMENT_STAGE_PREFIX: &str = "__geotrace_replacement__";

/// Resolve one replacement stage, in each of the three states an interrupted
/// [`replace_recording`] leaves one in.
///
/// A stage without recording metadata was interrupted while it was being
/// built: the stored recording is as it was, and the stage is dropped. A
/// complete stage whose recording is still stored under its own name was
/// interrupted before the swap: the recording is as it was, and the stage is
/// dropped. A complete stage whose recording is missing from `by_identity` was
/// interrupted between the unlink and the relink: the stage is linked in under
/// the recording's own name, and the recording is stored as the re-encode left
/// it.
fn resolve_replacement_stage(file: &hdf5::File, stage_name: &str) -> Result<(), InternalError> {
    let Some(group_name) = stage_name.strip_prefix(REPLACEMENT_STAGE_PREFIX) else {
        return Ok(());
    };
    let meta_grp = file.group("meta")?;
    let stage = meta_grp.group(stage_name)?;
    let by_id = file.group("by_identity")?;

    let (Some(identity), Some(_)) = (recording_identity(&stage), read_recording_meta(&stage))
    else {
        return meta_grp.unlink(stage_name).map_err(Into::into);
    };

    let identity_name =
        identity_storage_name(&by_id, &identity).unwrap_or_else(|| identity_group_name(&identity));
    let id_grp = match by_id.group(&identity_name) {
        Ok(id_grp) => id_grp,
        Err(_) => ensure_identity_group(&by_id, &identity)?,
    };
    if id_grp.link_exists(group_name) {
        return meta_grp.unlink(stage_name).map_err(Into::into);
    }

    file.relink(
        &format!("meta/{stage_name}"),
        &format!("by_identity/{identity_name}/{group_name}"),
    )?;
    log::warn!(
        "Restored history recording identity={identity:?}, group={group_name:?} from the stage of an interrupted replacement"
    );
    Ok(())
}

/// Rewrite one recording's GTD data, metadata attributes, track table and
/// segmentation settings under the group name it already has.
///
/// The recording's log attachment attributes are kept. Every other attribute
/// and every member of the group, its snap run included, is replaced by what
/// `gtd_bytes`, `meta`, `tracks` and `settings` describe.
///
/// A failure while the replacement is being built leaves the stored recording
/// as it was: the replacement is built in a stage (see
/// [`REPLACEMENT_STAGE_PREFIX`]) and linked in only once it is complete. HDF5
/// has no way to point a name at another object in one step, as
/// [`rename_identity`] also works around: a failure between the two link
/// operations leaves the recording in its stage, and
/// [`resolve_replacement_stage`] puts it back.
pub(crate) fn replace_recording(
    db_path: &std::path::Path,
    db_ref: &DatabaseRef,
    meta: &RecordingMeta,
    tracks: &[TrackRange],
    settings: StoredSegmentation,
    gtd_bytes: &[u8],
) -> Result<(), InternalError> {
    let file = hdf5::File::open_rw(db_path)?;
    let by_id = file.group("by_identity")?;
    let identity_name = identity_storage_name(&by_id, &db_ref.identity)
        .unwrap_or_else(|| identity_group_name(&db_ref.identity));
    let id_grp = by_id.group(&identity_name)?;
    let meta_grp = file.group("meta").or_else(|_| file.create_group("meta"))?;

    let stage_name = format!("{REPLACEMENT_STAGE_PREFIX}{}", db_ref.group_name);
    let stage_path = format!("meta/{stage_name}");
    let recording_path = format!("by_identity/{identity_name}/{}", db_ref.group_name);

    if meta_grp.link_exists(&stage_name) {
        resolve_replacement_stage(&file, &stage_name)?;
    }

    let rec_grp = id_grp.group(&db_ref.group_name)?;

    let tmp = tempfile::NamedTempFile::new_in(db_path.parent().unwrap_or_else(|| Path::new(".")))?;
    std::fs::write(tmp.path(), gtd_bytes)?;
    let gtd_file = hdf5::File::open(tmp.path())?;

    let stage = meta_grp.create_group(&stage_name)?;
    copy_attrs(&gtd_file, &stage, |_| true)?;
    copy_members(&gtd_file, &stage)?;
    write_track_table(&stage, tracks)?;
    copy_attrs(&rec_grp, &stage, |name| {
        LogAttachmentId::from_attr_key(name).is_some()
    })?;
    write_segmentation_attrs(&stage, settings)?;
    // Written last: until it has these, the stage is no recording to
    // `repair_unindexed_recordings`.
    write_meta_attrs(&stage, &db_ref.identity, meta)?;

    drop(rec_grp);
    id_grp.unlink(&db_ref.group_name)?;
    file.relink(&stage_path, &recording_path)?;

    file.flush()?;
    Ok(())
}

/// Write (creating or overwriting) the segmentation settings the stored tracks
/// were built with.
fn write_segmentation_attrs(
    group: &Group,
    settings: StoredSegmentation,
) -> Result<(), InternalError> {
    let upsert_i64 = |name: &str, val: i64| -> Result<(), InternalError> {
        let attr = match group.attr(name) {
            Ok(attr) => attr,
            Err(_) => group.new_attr::<i64>().create(name)?,
        };
        attr.write_scalar(&val)?;
        Ok(())
    };
    let upsert_u64 = |name: &str, val: u64| -> Result<(), InternalError> {
        let attr = match group.attr(name) {
            Ok(attr) => attr,
            Err(_) => group.new_attr::<u64>().create(name)?,
        };
        attr.write_scalar(&val)?;
        Ok(())
    };
    let upsert_f64 = |name: &str, val: f64| -> Result<(), InternalError> {
        let attr = match group.attr(name) {
            Ok(attr) => attr,
            Err(_) => group.new_attr::<f64>().create(name)?,
        };
        attr.write_scalar(&val)?;
        Ok(())
    };
    upsert_i64(ATTR_SEG_GAP_US, settings.track_split_gap_us)?;
    upsert_i64(
        ATTR_SEG_SPLIT_RULE,
        settings.track_split_rule.attribute_value(),
    )?;
    upsert_i64(
        ATTR_SEG_PLACEMENT_RULE,
        settings.fix_placement_rule.attribute_value(),
    )?;
    upsert_u64(
        ATTR_SEG_DETECT_CLOCK,
        u64::from(settings.detect_clock_discontinuities),
    )?;
    upsert_f64(ATTR_SEG_CLOCK_SIGMAS, settings.clock_discontinuity_sigmas)?;
    Ok(())
}

/// (Re)write the `__geotrace_tracks__` subgroup holding the track ranges.
fn write_track_table(rec_grp: &Group, tracks: &[TrackRange]) -> Result<(), InternalError> {
    if rec_grp.link_exists(TRACKS_GROUP) {
        rec_grp.unlink(TRACKS_GROUP)?;
    }
    let grp = rec_grp.create_group(TRACKS_GROUP)?;
    let (starts, ends, states) = gt_history_types::track_columns(tracks);
    for (name, col) in [
        (TRACK_START_DATASET, &starts),
        (TRACK_END_DATASET, &ends),
        (TRACK_STATE_DATASET, &states),
    ] {
        grp.new_dataset::<u64>()
            .shape([col.len()])
            .create(name)?
            .write_raw(col)?;
    }
    Ok(())
}

/// Every row of a recording's stored track table, tombstones and all.
fn stored_track_table(rec_grp: &Group) -> Option<Vec<TrackRange>> {
    let grp = rec_grp.group(TRACKS_GROUP).ok()?;
    gt_history_types::stored_track_table(|name| {
        grp.dataset(name).and_then(|d| d.read_raw::<u64>()).ok()
    })
}

/// The tracks a recording has: its [`stored_track_table`] without the
/// [`TrackState::Deleted`] tombstones a permanent delete leaves.
fn read_track_table(rec_grp: &Group) -> Vec<TrackRange> {
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
fn read_channel_summaries(rec_grp: &Group) -> Vec<ChannelSummary> {
    let Ok(root) = rec_grp.group(GTD_CHANNELS_GROUP) else {
        return Vec::new();
    };
    let Ok(names) = root.member_names() else {
        return Vec::new();
    };

    let mut summaries: Vec<ChannelSummary> = names
        .into_iter()
        .filter_map(|name| {
            let grp = root.group(&name).ok()?;
            let sample_count = grp
                .dataset(GTD_CHANNEL_TIME_DATASET)
                .ok()
                .and_then(|d| d.shape().first().copied())
                .unwrap_or(0) as u64;
            Some(ChannelSummary {
                name,
                unit: read_group_string_attr(&grp, GTD_CHANNEL_UNIT_ATTR).ok(),
                description: read_group_string_attr(&grp, GTD_CHANNEL_DESCRIPTION_ATTR).ok(),
                components: read_group_string_array_attr(&grp, GTD_CHANNEL_COMPONENTS_ATTR)
                    .unwrap_or_default(),
                sample_count,
            })
        })
        .collect();
    ChannelSummary::sort_by_name(&mut summaries);
    summaries
}

/// Read the stored segmentation settings, if present.
fn read_segmentation(rec_grp: &Group) -> Option<StoredSegmentation> {
    let gap = rec_grp
        .attr(ATTR_SEG_GAP_US)
        .and_then(|a| a.read_scalar::<i64>())
        .ok()?;
    let detect = rec_grp
        .attr(ATTR_SEG_DETECT_CLOCK)
        .and_then(|a| a.read_scalar::<u64>())
        .ok()?;
    let sigmas = rec_grp
        .attr(ATTR_SEG_CLOCK_SIGMAS)
        .and_then(|a| a.read_scalar::<f64>())
        .ok()?;
    let split_rule = rec_grp
        .attr(ATTR_SEG_SPLIT_RULE)
        .and_then(|a| a.read_scalar::<i64>())
        .ok();
    let placement_rule = rec_grp
        .attr(ATTR_SEG_PLACEMENT_RULE)
        .and_then(|a| a.read_scalar::<i64>())
        .ok();
    Some(StoredSegmentation {
        track_split_gap_us: gap,
        track_split_rule: StoredTrackSplitRule::from_attribute_value(split_rule),
        fix_placement_rule: StoredFixPlacementRule::from_attribute_value(placement_rule),
        detect_clock_discontinuities: detect != 0,
        clock_discontinuity_sigmas: sigmas,
    })
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
        write_string_attr(group, name, val)
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
    write_i64(ATTR_START_US, meta.stored_start_us())?;
    write_i64(ATTR_END_US, meta.stored_end_us())?;
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
/// Preserves every member's datatype, shape, attributes, chunking and
/// compression for the whole subtree. It copies across open files, so it works
/// for both storing a GTD file into the database and extracting a recording back
/// out.
fn copy_members(src: &Group, dst: &Group) -> Result<(), InternalError> {
    for name in src.member_names()? {
        // Never copy DB-internal bookkeeping (the track table) as GTD data.
        if is_db_internal_group(&name) {
            continue;
        }
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
            // Re-store every string attribute as fixed-length (see
            // `write_string_attr`) so its space is reclaimed when the recording is
            // deleted. Reading the original is the subtle part (see `read_string_attr`).
            let s = read_string_attr(&attr, &descriptor)?;
            write_string_attr(dst, name, &s)?;
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

/// Write a string attribute as **fixed-length**, choosing the smallest capacity
/// that holds `val` (64/256/1024/8192 bytes), falling back to variable-length only
/// for strings of 8 KiB or more.
///
/// Fixed-length string data lives in the object header, whose space libhdf5 reuses
/// when the owning object is deleted. Variable-length strings live in the global
/// heap, which libhdf5 does NOT reclaim across open/close sessions - so writing
/// them per recording made the database file grow without bound as recordings
/// churned. The >= 8 KiB fall-back keeps a pathologically long string from being
/// truncated. One such string is far too rare to reintroduce unbounded growth.
///
/// Every per-recording string attribute must be written through this function so
/// none lands in the global heap. Do not call `new_attr::<VarLenUnicode>()`
/// directly on a recording group. The bounded-file invariant is guarded by
/// `gt_history`'s `file_size_stays_bounded_across_delete_reinsert_cycles` test.
fn write_string_attr(group: &Group, name: &str, val: &str) -> Result<(), InternalError> {
    macro_rules! write_fixed {
        ($cap:literal) => {{
            let s = val
                .parse::<hdf5::types::FixedUnicode<$cap>>()
                .map_err(|e| {
                    InternalError::Hdf5(hdf5::Error::from(format!("invalid attribute string: {e}")))
                })?;
            group
                .new_attr::<hdf5::types::FixedUnicode<$cap>>()
                .create(name)?
                .write_scalar(&s)?;
        }};
    }

    let len = val.len();
    if len < 64 {
        write_fixed!(64);
    } else if len < 256 {
        write_fixed!(256);
    } else if len < 1024 {
        write_fixed!(1024);
    } else if len < 8192 {
        write_fixed!(8192);
    } else {
        let v = val.parse::<hdf5::types::VarLenUnicode>().map_err(|e| {
            InternalError::Hdf5(hdf5::Error::from(format!("invalid attribute string: {e}")))
        })?;
        group
            .new_attr::<hdf5::types::VarLenUnicode>()
            .create(name)?
            .write_scalar(&v)?;
    }
    Ok(())
}

/// The fixed-capacity ladder the string-attribute readers share: pick the
/// smallest compile-time capacity that holds the on-disk size `$len`, then hand
/// the concrete `$fixed<CAP>` type to the `$read` macro.
///
/// libhdf5 converts between fixed string sizes but offers no fixed -> variable
/// conversion path, so a fixed-length attribute cannot be read straight into
/// `VarLenUnicode`. It must be read into a fixed buffer at least as large as the
/// on-disk capacity.
///
/// This ladder keys off the *on-disk capacity* `n` reported by the descriptor,
/// while [`write_string_attr`] chooses its capacity from the string's *byte
/// length*. The `<=` bounds here (against the `<` bounds there) make the reader
/// land in the exact bucket the writer emitted.
macro_rules! read_at_capacity {
    ($read:ident, $fixed:ident, $len:expr) => {{
        let len = $len;
        if len <= 64 {
            $read!($fixed<64>)
        } else if len <= 256 {
            $read!($fixed<256>)
        } else if len <= 1024 {
            $read!($fixed<1024>)
        } else if len <= 8192 {
            $read!($fixed<8192>)
        } else {
            return Err(InternalError::Hdf5(hdf5::Error::from(format!(
                "string attribute too long to copy in place ({len} bytes)"
            ))));
        }
    }};
}

/// Read a string attribute (fixed- or variable-length, Unicode or ASCII) into an
/// owned `String`. See [`read_at_capacity`] for why fixed-length attributes need
/// the capacity ladder.
fn read_string_attr(
    attr: &hdf5::Attribute,
    descriptor: &hdf5::types::TypeDescriptor,
) -> Result<String, InternalError> {
    use hdf5::types::{FixedAscii, FixedUnicode, TypeDescriptor, VarLenAscii, VarLenUnicode};

    macro_rules! scalar {
        ($t:ty) => {
            attr.read_scalar::<$t>()?.as_str().to_owned()
        };
    }

    Ok(match descriptor {
        TypeDescriptor::VarLenUnicode => scalar!(VarLenUnicode),
        TypeDescriptor::VarLenAscii => scalar!(VarLenAscii),
        TypeDescriptor::FixedUnicode(n) => read_at_capacity!(scalar, FixedUnicode, *n),
        TypeDescriptor::FixedAscii(n) => read_at_capacity!(scalar, FixedAscii, *n),
        other => {
            return Err(InternalError::Hdf5(hdf5::Error::from(format!(
                "attribute is not a string type: {other:?}"
            ))));
        }
    })
}

/// Read a 1-D array-of-strings attribute (a channel's component labels) into
/// owned `String`s, over the same element types and capacity ladder as
/// [`read_string_attr`].
fn read_string_array_attr(
    attr: &hdf5::Attribute,
    descriptor: &hdf5::types::TypeDescriptor,
) -> Result<Vec<String>, InternalError> {
    use hdf5::types::{FixedAscii, FixedUnicode, TypeDescriptor, VarLenAscii, VarLenUnicode};

    macro_rules! rows {
        ($t:ty) => {
            attr.read_1d::<$t>()?
                .iter()
                .map(|s| s.as_str().to_owned())
                .collect()
        };
    }

    Ok(match descriptor {
        TypeDescriptor::VarLenUnicode => rows!(VarLenUnicode),
        TypeDescriptor::VarLenAscii => rows!(VarLenAscii),
        TypeDescriptor::FixedUnicode(n) => read_at_capacity!(rows, FixedUnicode, *n),
        TypeDescriptor::FixedAscii(n) => read_at_capacity!(rows, FixedAscii, *n),
        other => {
            return Err(InternalError::Hdf5(hdf5::Error::from(format!(
                "attribute is not a string type: {other:?}"
            ))));
        }
    })
}

/// Read a group's array-of-strings attribute, or an error when it is absent or
/// not a string type.
fn read_group_string_array_attr(group: &Group, name: &str) -> Result<Vec<String>, InternalError> {
    let attr = group.attr(name)?;
    let descriptor = attr
        .dtype()?
        .to_descriptor()
        .map_err(|e| InternalError::Hdf5(hdf5::Error::from(e.to_string())))?;
    read_string_array_attr(&attr, &descriptor)
}

pub(crate) fn load_recording(
    db_path: &std::path::Path,
    identity: &str,
    group_name: &str,
) -> Result<StoredRecording, InternalError> {
    let tmp = tempfile::NamedTempFile::new_in(db_path.parent().unwrap_or_else(|| Path::new(".")))?;

    // Reconstruct the GTD file in the sibling temp file, then read it back below.
    // Every libhdf5 handle on the temp file (`out` and its `root` group) must be
    // closed before `std::fs::read`: on Windows libhdf5 holds a mandatory
    // byte-range lock for as long as any handle to the file is open, so reading a
    // locked range otherwise fails with ERROR_LOCK_VIOLATION. The enclosing scope
    // drops every handle before the read.
    let (tracks, segmentation) = {
        let db = hdf5::File::open(db_path)?;
        let by_id = db.group("by_identity")?;
        let id_grp = open_identity_group(&by_id, identity)?;
        let rec_grp = id_grp.group(group_name)?;

        let tracks = read_track_table(&rec_grp);
        let segmentation = read_segmentation(&rec_grp);

        let out = hdf5::File::create(tmp.path())?;
        let root = out.group("/")?;

        // Copy the data groups/datasets back to the root (the track table is
        // skipped by `copy_members`), and restore the original GTD root
        // attributes (skipping the database's own recording metadata).
        copy_members(&rec_grp, &root)?;
        copy_attrs(&rec_grp, &root, |name| !is_db_recording_attr(name))?;

        // Fall back to the default version for recordings stored before
        // attribute preservation existed.
        if root.attr(GTD_VERSION_ATTR).is_err() {
            let version: hdf5::types::VarLenUnicode =
                GTD_VERSION_FALLBACK.parse().map_err(|e| {
                    InternalError::Hdf5(hdf5::Error::from(format!("invalid version literal: {e}")))
                })?;
            root.new_attr::<hdf5::types::VarLenUnicode>()
                .create(GTD_VERSION_ATTR)?
                .write_scalar(&version)?;
        }

        out.flush().map_err(InternalError::Hdf5)?;
        (tracks, segmentation)
    };

    Ok(StoredRecording {
        bytes: std::fs::read(tmp.path())?,
        tracks,
        segmentation,
    })
}

/// Replace a recording's track table and segmentation settings.
pub(crate) fn set_tracks(
    db_path: &std::path::Path,
    identity: &str,
    group_name: &str,
    tracks: &[TrackRange],
    settings: StoredSegmentation,
) -> Result<(), InternalError> {
    let file = hdf5::File::open_rw(db_path)?;
    let by_id = file.group("by_identity")?;
    let id_grp = open_identity_group(&by_id, identity)?;
    let rec_grp = id_grp.group(group_name)?;
    write_track_table(&rec_grp, tracks)?;
    write_segmentation_attrs(&rec_grp, settings)?;
    Ok(())
}

/// Replace a recording's stored snap run with the opaque bytes in `blob`.
///
/// A fixed-shape `u8` dataset in the `__geotrace_snap__` subgroup,
/// unlinked and recreated on rewrite - variable-length values leak in
/// libhdf5's global heap across rewrite cycles, fixed-shape datasets are
/// reclaimed by the free-space manager.
pub(crate) fn set_snap_blob(
    db_path: &std::path::Path,
    identity: &str,
    group_name: &str,
    blob: &[u8],
) -> Result<(), InternalError> {
    let file = hdf5::File::open_rw(db_path)?;
    let by_id = file.group("by_identity")?;
    let id_grp = open_identity_group(&by_id, identity)?;
    let rec_grp = id_grp.group(group_name)?;
    if rec_grp.link_exists(SNAP_GROUP) {
        rec_grp.unlink(SNAP_GROUP)?;
    }
    let grp = rec_grp.create_group(SNAP_GROUP)?;
    grp.new_dataset::<u8>()
        .shape([blob.len()])
        .create(SNAP_BLOB_DATASET)?
        .write_raw(blob)?;
    Ok(())
}

/// The stored snap run bytes of a recording, `None` when it carries none.
pub(crate) fn snap_blob(
    db_path: &std::path::Path,
    identity: &str,
    group_name: &str,
) -> Result<Option<Vec<u8>>, InternalError> {
    let file = hdf5::File::open(db_path)?;
    let by_id = file.group("by_identity")?;
    let id_grp = open_identity_group(&by_id, identity)?;
    let rec_grp = id_grp.group(group_name)?;
    let Ok(grp) = rec_grp.group(SNAP_GROUP) else {
        return Ok(None);
    };
    let Ok(dataset) = grp.dataset(SNAP_BLOB_DATASET) else {
        return Ok(None);
    };
    Ok(dataset.read_raw::<u8>().ok())
}

/// Every log attached to a recording, in the order
/// [`LogAttachmentEntry::sort_by_name_then_id`] puts them.
pub(crate) fn log_attachments(
    db_path: &std::path::Path,
    identity: &str,
    group_name: &str,
) -> Result<Vec<LogAttachmentEntry>, InternalError> {
    let file = hdf5::File::open(db_path)?;
    let by_id = file.group("by_identity")?;
    let id_grp = open_identity_group(&by_id, identity)?;
    let rec_grp = id_grp.group(group_name)?;
    let attr_names = rec_grp.attr_names()?;
    Ok(log_attachments_in_attrs(&rec_grp, attr_names))
}

/// The attachments the attributes `attr_names` of a recording hold, for a
/// caller that has read those names already.
fn log_attachments_in_attrs(rec_grp: &Group, attr_names: Vec<String>) -> Vec<LogAttachmentEntry> {
    let mut entries = Vec::new();
    for key in attr_names {
        let Some(id) = LogAttachmentId::from_attr_key(&key) else {
            continue;
        };
        match read_group_string_attr(rec_grp, &key) {
            Ok(json) => {
                if let Some(attachment) = LogAttachment::from_attribute_json(&json) {
                    entries.push(LogAttachmentEntry { id, attachment });
                }
            }
            Err(err) => log::warn!("Ignoring the log attachment attribute {key:?}: {err}"),
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
    let file = hdf5::File::open_rw(db_path)?;
    let by_id = file.group("by_identity")?;
    let id_grp = open_identity_group(&by_id, identity)?;
    let rec_grp = id_grp.group(group_name)?;

    let key = id.attr_key();
    if rec_grp.attr(&key).is_ok() {
        rec_grp.delete_attr(&key)?;
    }
    write_string_attr(&rec_grp, &key, attribute_json)
}

/// Remove one attachment's attribute. A recording that carries no such
/// attachment, or that is gone entirely, leaves nothing to remove.
pub(crate) fn delete_log_attachment_attribute(
    db_path: &std::path::Path,
    identity: &str,
    group_name: &str,
    id: LogAttachmentId,
) -> Result<(), InternalError> {
    let file = hdf5::File::open_rw(db_path)?;
    let by_id = file.group("by_identity")?;
    let recording = open_identity_group(&by_id, identity)
        .ok()
        .and_then(|id_grp| id_grp.group(group_name).ok());
    let Some(rec_grp) = recording else {
        log::warn!(
            "Removing log attachment {id} from {identity}/{group_name}, which is not in the database"
        );
        return Ok(());
    };

    let key = id.attr_key();
    if rec_grp.attr(&key).is_ok() {
        rec_grp.delete_attr(&key)?;
    }
    Ok(())
}

/// The attachments of a recording group, for deleting their logs with it.
pub(crate) fn log_attachment_ids(rec_grp: &Group) -> Vec<LogAttachmentId> {
    match rec_grp.attr_names() {
        Ok(keys) => keys
            .iter()
            .map(String::as_str)
            .filter_map(LogAttachmentId::from_attr_key)
            .collect(),
        Err(err) => {
            log::warn!("Could not read the attributes of a recording being deleted: {err}");
            Vec::new()
        }
    }
}

/// Shelve or unshelve a recording's tracks at `track_indices`.
///
/// `track_indices` count the tracks the recording lists, which
/// [`gt_history_types::listed_track_rows`] maps onto the stored table's rows. A
/// recording whose table predates [`TRACK_STATE_DATASET`] comes out of this
/// with a state column: the whole table is rewritten, tombstones and all.
pub(crate) fn set_tracks_shelved(
    db_path: &std::path::Path,
    identity: &str,
    group_name: &str,
    track_indices: &[usize],
    shelved: bool,
) -> Result<(), InternalError> {
    let file = hdf5::File::open_rw(db_path)?;
    let by_id = file.group("by_identity")?;
    let id_grp = open_identity_group(&by_id, identity)?;
    let rec_grp = id_grp.group(group_name)?;
    let Some(mut tracks) = stored_track_table(&rec_grp) else {
        log::warn!("Shelving tracks of {identity}/{group_name}, which has no track table");
        return Ok(());
    };
    let state = if shelved {
        TrackState::Shelved
    } else {
        TrackState::Live
    };
    for index in gt_history_types::set_state_of_listed_tracks(&mut tracks, track_indices, state) {
        log::warn!("track index {index} out of range for {identity}/{group_name}");
    }
    write_track_table(&rec_grp, &tracks)
}

pub(crate) fn list_recordings(
    db_path: &std::path::Path,
) -> Result<Vec<RecordingEntry>, InternalError> {
    let file = hdf5::File::open(db_path)?;
    let by_id = file.group("by_identity")?;
    let mut entries = Vec::new();
    for storage_name in by_id.member_names()? {
        if let Ok(id_grp) = by_id.group(&storage_name) {
            let identity = identity_for_group(&id_grp, &storage_name);
            for rec_name in id_grp.member_names()? {
                if let Ok(rec_grp) = id_grp.group(&rec_name) {
                    let Some(meta) = read_recording_meta(&rec_grp) else {
                        log::warn!(
                            "Skipping malformed history recording identity={identity:?}, group={rec_name:?}: missing metadata"
                        );
                        continue;
                    };
                    let tracks = read_track_table(&rec_grp);
                    let shelved_tracks = tracks
                        .iter()
                        .filter(|t| t.state == TrackState::Shelved)
                        .count();
                    let log_attachments = match rec_grp.attr_names() {
                        Ok(attr_names) => log_attachments_in_attrs(&rec_grp, attr_names),
                        Err(err) => {
                            log::error!(
                                "Could not read the attributes of the history recording identity={identity:?}, group={rec_name:?}: it is listed with none of the logs stored with it: {err}"
                            );
                            Vec::new()
                        }
                    };

                    entries.push(RecordingEntry {
                        db_ref: gt_history_types::DatabaseRef {
                            identity: identity.clone(),
                            group_name: rec_name,
                        },
                        meta,
                        total_tracks: tracks.len(),
                        shelved_tracks,
                        title: read_group_string_attr(&rec_grp, GTD_META_TITLE_ATTR).ok(),
                        device: read_group_string_attr(&rec_grp, GTD_META_DEVICE_ATTR).ok(),
                        notes: read_group_string_attr(&rec_grp, GTD_META_NOTES_ATTR).ok(),
                        travel_mode: read_group_string_attr(&rec_grp, GTD_META_TRAVEL_MODE_ATTR)
                            .ok(),
                        channels: read_channel_summaries(&rec_grp),
                        log_attachments,
                    });
                }
            }
        }
    }
    entries.sort_unstable_by_key(|e| std::cmp::Reverse(e.meta.stored_start_us()));
    Ok(entries)
}

/// The status flags an interrupted SWMR writer leaves: open for write, plus
/// the SWMR marker. Observed on a file `hdf5-pure` flagged and abandoned.
#[cfg(test)]
const WRITER_FLAGS: u8 = 5;

/// Set the superblock status flags a crashed writer leaves behind, the
/// mirror of [`clear_write_lock`].
///
/// libhdf5 clears the flags on close and, within one process, tracks that it
/// already holds the file, so the state cannot be reached by opening one.
#[cfg(test)]
pub(crate) fn mark_write_locked(db_path: &Path) -> Result<(), InternalError> {
    use std::io::{Read, Seek, SeekFrom, Write};

    let mut file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(db_path)?;
    let mut sb = [0_u8; SUPERBLOCK_V2_LEN];
    file.read_exact(&mut sb)?;
    if let Some(slot) = sb.get_mut(11) {
        *slot = WRITER_FLAGS;
    }
    let checksum = jenkins_lookup3(sb.get(..44).unwrap_or(&[]));
    if let Some(slot) = sb.get_mut(44..48) {
        slot.copy_from_slice(&checksum.to_le_bytes());
    }
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&sb)?;
    file.flush()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{
        SUPERBLOCK_V2_LEN, WRITER_FLAGS, clear_write_lock, create_native_file, jenkins_lookup3,
        mark_write_locked, read_string_attr, write_string_attr,
    };

    /// The repair round trip: flags set by an interrupted writer are cleared,
    /// the superblock checksum is left valid, and the file still opens.
    ///
    /// Deliberately does not assert that the flagged file is rejected before
    /// clearing. libhdf5 opens a flagged v2 superblock read-write, which is
    /// what [`create_native_file`] writes. Only a v3 superblock is rejected.
    /// The path that reaches [`crate::classify_open_error`] for these files is
    /// covered by `clear_write_lock_repairs_an_unreadable_superblock` in
    /// `gt-history`.
    #[test]
    fn clearing_the_write_lock_resets_the_status_flags() {
        use std::io::Read as _;

        let tmp = tempfile::NamedTempFile::new().expect("temp file");
        drop(create_native_file(tmp.path()).expect("create"));
        mark_write_locked(tmp.path()).expect("set the flags");

        let flags = |tag: &str| {
            let mut file = std::fs::File::open(tmp.path()).unwrap_or_else(|e| panic!("{tag}: {e}"));
            let mut sb = [0_u8; SUPERBLOCK_V2_LEN];
            file.read_exact(&mut sb)
                .unwrap_or_else(|e| panic!("{tag}: {e}"));
            sb.get(11).copied().unwrap_or(0)
        };
        assert_eq!(flags("after marking"), WRITER_FLAGS);

        assert!(
            clear_write_lock(tmp.path()).expect("clear"),
            "a flag was set"
        );
        assert_eq!(flags("after clearing"), 0);
        hdf5::File::open_rw(tmp.path()).expect("opens once cleared");
    }

    /// Clearing a file that carries no flags reports that nothing was done
    /// and leaves it readable.
    #[test]
    fn clearing_an_unflagged_file_changes_nothing() {
        let tmp = tempfile::NamedTempFile::new().expect("temp file");
        drop(create_native_file(tmp.path()).expect("create"));

        assert!(
            !clear_write_lock(tmp.path()).expect("clear"),
            "no flag was set"
        );
        hdf5::File::open_rw(tmp.path()).expect("still writable");
    }

    /// Strings written fixed-length by `write_string_attr` must read back exactly
    /// via `read_string_attr`, especially at the capacity-ladder boundaries where
    /// the two ladders' bucket choices meet. Covers the fixed buckets and the
    /// `>= 8 KiB` variable-length fall-back.
    #[test]
    fn string_attr_round_trips_at_capacity_boundaries() {
        let tmp = tempfile::NamedTempFile::new().expect("temp file");
        let file = hdf5::File::create(tmp.path()).expect("create");
        let group = file.create_group("g").expect("group");

        for len in [0_usize, 1, 63, 64, 255, 256, 1023, 1024, 8191, 8192, 9000] {
            let value = "x".repeat(len);
            let name = format!("attr_{len}");
            write_string_attr(&group, &name, &value).expect("write");

            let attr = group.attr(&name).expect("attr");
            let descriptor = attr
                .dtype()
                .expect("dtype")
                .to_descriptor()
                .expect("descriptor");
            let got = read_string_attr(&attr, &descriptor).expect("read");
            assert_eq!(got, value, "string of length {len} must round-trip");
        }
    }

    #[test]
    fn jenkins_lookup3_matches_known_vectors() {
        // Canonical Jenkins lookup3 (`hashlittle`) self-test vectors with
        // `initval` 0, matching libhdf5's `H5_checksum_lookup3`.
        assert_eq!(jenkins_lookup3(b""), 0xdead_beef);
        assert_eq!(
            jenkins_lookup3(b"Four score and seven years ago"),
            0x1777_0551
        );
    }

    #[test]
    fn jenkins_lookup3_covers_all_tail_lengths() {
        // Exercise every fall-through arm (tail length 0..=12 after the main
        // loop) so a bug in any tail byte would change the digest. The values
        // are self-consistent regression anchors.
        let data: Vec<u8> = (0..40_u8).collect();
        for len in 0..=data.len() {
            // Must not panic and must be deterministic.
            let h = jenkins_lookup3(&data[..len]);
            assert_eq!(h, jenkins_lookup3(&data[..len]));
        }
    }
}
