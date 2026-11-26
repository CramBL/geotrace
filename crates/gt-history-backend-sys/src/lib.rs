use std::ops::Deref;
use std::path::{Path, PathBuf};

use gt_history_types::{
    CURRENT_SCHEMA_VERSION, DatabaseRef, DbError, HISTORY_LOCK as DB_LOCK, HistoryAccess,
    HistoryDatabase, LogAttachment, LogAttachmentId, LogAttachmentListingEntry, NavPointTimeRange,
    ReadOnlyHistoryDatabase, RecordingEntry, RecordingInsert, RecordingMeta, RecordingReplacement,
    RecordingUiState, SCHEMA_VERSION_ATTR, StoredRecording, StoredSegmentation,
    TrackLayoutVisibility, TrackRange, UiStateVersionReporter, log_attachment,
};

pub mod copy;

/// Extract recording metadata from raw GTD file bytes.
///
/// libhdf5 reads from a path, so the bytes are staged
/// in a temporary file. Each count comes from its dataset's shape, the time
/// range from every `nav_points/time` entry.
pub fn extract_meta(bytes: &[u8]) -> Result<RecordingMeta, DbError> {
    let tmp = tempfile::NamedTempFile::new()?;
    std::fs::write(tmp.path(), bytes)?;
    let file = hdf5::File::open(tmp.path()).map_err(|e| DbError::Backend(e.to_string()))?;

    let nav = file
        .group("nav_points")
        .map_err(|e| DbError::Backend(e.to_string()))?;
    let time = nav
        .dataset("time")
        .map_err(|e| DbError::Backend(e.to_string()))?;
    let nav_point_count = time.shape().first().copied().unwrap_or(0) as u64;

    let time_range = if nav_point_count > 0 {
        let times: Vec<i64> = time
            .read_raw()
            .map_err(|e| DbError::Backend(e.to_string()))?;
        NavPointTimeRange::covering(&times)
    } else {
        None
    };

    // Count rows in an optional data group's index dataset. Absent groups
    // contribute zero.
    let count_rows = |group: &str, dataset: &str| -> u64 {
        file.group(group)
            .ok()
            .and_then(|g| g.dataset(dataset).ok())
            .map(|d| d.shape().first().copied().unwrap_or(0) as u64)
            .unwrap_or(0)
    };

    Ok(RecordingMeta {
        time_range,
        nav_point_count,
        sat_report_count: [
            "sat_reports",
            "satellite_usage_reports",
            "satellite_observation_reports",
        ]
        .into_iter()
        .map(|group| count_rows(group, "nav_point_idx"))
        .fold(0, u64::saturating_add),
        marker_count: count_rows("markers", "time"),
        event_marker_count: count_rows("event_markers", "sys_time_us"),
        gtd_size_bytes: bytes.len() as u64,
        debug_tag: None,
    })
}

/// Classify a failure to open an existing database.
///
/// [`hdf5::MinorErrorCode::CantLockFile`] is what libhdf5 sets when another
/// process holds the file. libhdf5 takes an OS lock for the duration of an
/// open, readers included.
///
/// The consistency flags an unclean shutdown leaves carry no code of their
/// own - every frame reports `CantOpenFile` - so that one is read from the
/// message.
fn classify_open_error(err: hdf5::Error) -> DbError {
    if err.contains_minor(hdf5::MinorErrorCode::CantLockFile) {
        return DbError::Busy;
    }
    let msg = err.to_string();
    if msg.contains("already open for write")
        || msg.contains("h5clear")
        || msg.contains("consistency flags")
    {
        return DbError::WriteLocked;
    }
    DbError::Backend(msg)
}

/// The history database with no method that writes to the file, as
/// [`Self::open_existing_read_only`] opens it.
pub struct ReadOnlySysDb {
    path: PathBuf,
}

impl ReadOnlySysDb {
    /// Open the database at `path` without writing to it: it is not created
    /// where it is missing, not migrated to the native HDF5 format, and its
    /// unindexed recordings are not repaired.
    pub fn open_existing_read_only(path: &Path) -> Result<Self, DbError> {
        let _guard = DB_LOCK.lock();
        let file = hdf5::File::open(path).map_err(classify_open_error)?;
        Self::require_supported_schema(Self::schema_version(&file))?;
        Ok(Self {
            path: path.to_owned(),
        })
    }

    fn schema_version(file: &hdf5::File) -> i64 {
        file.attr(SCHEMA_VERSION_ATTR)
            .and_then(|a| a.read_scalar::<i64>())
            .unwrap_or(0)
    }

    fn require_supported_schema(schema_version: i64) -> Result<(), DbError> {
        if schema_version > CURRENT_SCHEMA_VERSION {
            return Err(DbError::SchemaTooNew {
                found: schema_version,
                supported: CURRENT_SCHEMA_VERSION,
            });
        }
        Ok(())
    }
}

impl ReadOnlyHistoryDatabase for ReadOnlySysDb {
    fn path(&self) -> &Path {
        &self.path
    }

    fn load(&self, db_ref: &DatabaseRef) -> Result<StoredRecording, DbError> {
        let _guard = DB_LOCK.lock();
        crate::copy::load_recording(&self.path, &db_ref.identity, &db_ref.group_name)
            .map_err(Into::into)
    }

    fn stored_track_table(&self, db_ref: &DatabaseRef) -> Result<Vec<TrackRange>, DbError> {
        let _guard = DB_LOCK.lock();
        crate::copy::stored_track_table_of_recording(
            &self.path,
            &db_ref.identity,
            &db_ref.group_name,
        )
        .map_err(Into::into)
    }

    fn snap_blob(&self, db_ref: &DatabaseRef) -> Result<Option<Vec<u8>>, DbError> {
        let _guard = DB_LOCK.lock();
        crate::copy::snap_blob(&self.path, &db_ref.identity, &db_ref.group_name).map_err(Into::into)
    }

    fn recording_ui_state(
        &self,
        db_ref: &DatabaseRef,
        reporter: &UiStateVersionReporter,
    ) -> Result<RecordingUiState, DbError> {
        let _guard = DB_LOCK.lock();
        crate::copy::recording_ui_state(&self.path, db_ref, reporter).map_err(Into::into)
    }

    fn log_attachments(
        &self,
        db_ref: &DatabaseRef,
    ) -> Result<Vec<LogAttachmentListingEntry>, DbError> {
        let _guard = DB_LOCK.lock();
        crate::copy::log_attachments(&self.path, &db_ref.identity, &db_ref.group_name)
            .map_err(Into::into)
    }

    fn list_recordings(&self) -> Result<Vec<RecordingEntry>, DbError> {
        let _guard = DB_LOCK.lock();
        copy::list_recordings(&self.path).map_err(Into::into)
    }

    fn contains(&self, db_ref: &DatabaseRef) -> Result<bool, DbError> {
        let _guard = DB_LOCK.lock();
        crate::copy::contains_recording(&self.path, &db_ref.identity, &db_ref.group_name)
            .map_err(Into::into)
    }

    fn is_duplicate(&self, meta: &RecordingMeta) -> Result<bool, DbError> {
        let _guard = DB_LOCK.lock();
        crate::copy::is_duplicate(&self.path, meta).map_err(Into::into)
    }
}

/// The history database with the methods that write to the file, as
/// [`HistoryDatabase::open_or_create`] opens it.
pub struct SysDb {
    read_only: ReadOnlySysDb,
}

impl SysDb {
    fn create_new(path: &Path) -> Result<(), DbError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Create with a persistent free-space manager so the space vacated by
        // deleting recordings is tracked across sessions and reused by later
        // inserts.
        let file = hdf5::File::with_options()
            .with_fcpl(|fcpl| {
                fcpl.file_space_strategy(hdf5::file::FileSpaceStrategy::FreeSpaceManager {
                    paged: false,
                    persist: true,
                    threshold: 1,
                })
            })
            .create(path)
            .map_err(|e| DbError::Backend(e.to_string()))?;
        file.new_attr::<i64>()
            .create(SCHEMA_VERSION_ATTR)
            .map_err(|e| DbError::Backend(e.to_string()))?
            .write_scalar(&CURRENT_SCHEMA_VERSION)
            .map_err(|e| DbError::Backend(e.to_string()))?;
        file.create_group("by_identity")
            .map_err(|e| DbError::Backend(e.to_string()))?;
        file.create_group("meta")
            .map_err(|e| DbError::Backend(e.to_string()))?;
        Ok(())
    }
}

impl Deref for SysDb {
    type Target = ReadOnlySysDb;

    fn deref(&self) -> &Self::Target {
        &self.read_only
    }
}

impl ReadOnlyHistoryDatabase for SysDb {
    fn path(&self) -> &Path {
        self.read_only.path()
    }

    fn load(&self, db_ref: &DatabaseRef) -> Result<StoredRecording, DbError> {
        self.read_only.load(db_ref)
    }

    fn stored_track_table(&self, db_ref: &DatabaseRef) -> Result<Vec<TrackRange>, DbError> {
        self.read_only.stored_track_table(db_ref)
    }

    fn snap_blob(&self, db_ref: &DatabaseRef) -> Result<Option<Vec<u8>>, DbError> {
        self.read_only.snap_blob(db_ref)
    }

    fn recording_ui_state(
        &self,
        db_ref: &DatabaseRef,
        reporter: &UiStateVersionReporter,
    ) -> Result<RecordingUiState, DbError> {
        self.read_only.recording_ui_state(db_ref, reporter)
    }

    fn log_attachments(
        &self,
        db_ref: &DatabaseRef,
    ) -> Result<Vec<LogAttachmentListingEntry>, DbError> {
        self.read_only.log_attachments(db_ref)
    }

    fn list_recordings(&self) -> Result<Vec<RecordingEntry>, DbError> {
        self.read_only.list_recordings()
    }

    fn contains(&self, db_ref: &DatabaseRef) -> Result<bool, DbError> {
        self.read_only.contains(db_ref)
    }

    fn is_duplicate(&self, meta: &RecordingMeta) -> Result<bool, DbError> {
        self.read_only.is_duplicate(meta)
    }
}

impl HistoryDatabase for SysDb {
    fn open_or_create(path: &Path) -> Result<Self, DbError> {
        let _guard = DB_LOCK.lock();
        if path.exists() {
            let (schema_version, pending_rename) = {
                let file = hdf5::File::open(path).map_err(classify_open_error)?;
                let schema_version = ReadOnlySysDb::schema_version(&file);
                if schema_version != crate::copy::RENAME_SCHEMA_VERSION {
                    ReadOnlySysDb::require_supported_schema(schema_version)?;
                }
                let pending_rename = crate::copy::validate_pending_renames(&file, schema_version)?;
                (schema_version, pending_rename)
            };
            // A native rename stage requires in-place recovery to preserve hard links.
            if !pending_rename && !crate::copy::is_native_writable(path) {
                log::info!(
                    "Migrating history database at {} to the native HDF5 format",
                    path.display()
                );
                crate::copy::migrate_to_native(path)?;
            }
            crate::copy::repair_unindexed_recordings(path, schema_version)?;
        } else {
            Self::create_new(path)?;
        }
        Ok(Self {
            read_only: ReadOnlySysDb {
                path: path.to_owned(),
            },
        })
    }

    fn clear_write_lock(path: &Path) -> Result<(), DbError> {
        let _guard = DB_LOCK.lock();
        if crate::copy::clear_write_lock(path)? {
            log::warn!(
                "Cleared a stale write lock on history database at {}",
                path.display()
            );
        }
        Ok(())
    }

    fn insert_with_outcome(
        &mut self,
        identity: &str,
        meta: &RecordingMeta,
        tracks: &[TrackRange],
        settings: StoredSegmentation,
        gtd_bytes: &[u8],
    ) -> Result<RecordingInsert, DbError> {
        let _guard = DB_LOCK.lock();
        crate::copy::insert_recording(&self.path, identity, meta, tracks, settings, gtd_bytes)
            .map_err(Into::into)
    }

    fn replace_recording_payload(
        &mut self,
        db_ref: &DatabaseRef,
        replacement: RecordingReplacement<'_>,
    ) -> Result<(), DbError> {
        let access = HistoryAccess::lock();
        access.invalidate_recording_snapshots(self, std::slice::from_ref(db_ref))?;
        crate::copy::replace_recording(&self.path, db_ref, replacement).map_err(Into::into)
    }

    fn set_track_layout(
        &mut self,
        db_ref: &DatabaseRef,
        tracks: &[TrackRange],
        settings: StoredSegmentation,
        visibility: TrackLayoutVisibility,
    ) -> Result<(), DbError> {
        let access = HistoryAccess::lock();
        access.invalidate_track_snapshots(self, db_ref)?;
        crate::copy::set_tracks(
            &self.path,
            &db_ref.identity,
            &db_ref.group_name,
            tracks,
            settings,
            visibility,
        )
        .map_err(Into::into)
    }

    fn set_tracks_shelved(
        &mut self,
        db_ref: &DatabaseRef,
        rows: &[usize],
        shelved: bool,
    ) -> Result<(), DbError> {
        let access = HistoryAccess::lock();
        access.invalidate_track_membership(self, db_ref)?;
        crate::copy::set_tracks_shelved(
            &self.path,
            &db_ref.identity,
            &db_ref.group_name,
            rows,
            shelved,
        )
        .map_err(Into::into)
    }

    fn set_snap_blob(&mut self, db_ref: &DatabaseRef, blob: &[u8]) -> Result<(), DbError> {
        let _guard = DB_LOCK.lock();
        crate::copy::set_snap_blob(&self.path, &db_ref.identity, &db_ref.group_name, blob)
            .map_err(Into::into)
    }

    fn set_recording_ui_state(
        &mut self,
        db_ref: &DatabaseRef,
        ui_state: &RecordingUiState,
        reporter: &UiStateVersionReporter,
    ) -> Result<(), DbError> {
        let _guard = DB_LOCK.lock();
        crate::copy::set_recording_ui_state(&self.path, db_ref, ui_state, reporter)
            .map_err(Into::into)
    }

    fn write_log_attachment_attribute(
        &mut self,
        db_ref: &DatabaseRef,
        id: LogAttachmentId,
        attachment: &LogAttachment,
    ) -> Result<(), DbError> {
        let attribute_json = attachment.to_attribute_json()?;
        let _guard = DB_LOCK.lock();
        crate::copy::set_log_attachment_attribute(
            &self.path,
            &db_ref.identity,
            &db_ref.group_name,
            id,
            &attribute_json,
        )
        .map_err(Into::into)
    }

    fn delete_log_attachment_attribute(
        &mut self,
        db_ref: &DatabaseRef,
        id: LogAttachmentId,
    ) -> Result<(), DbError> {
        let _guard = DB_LOCK.lock();
        crate::copy::delete_log_attachment_attribute(
            &self.path,
            &db_ref.identity,
            &db_ref.group_name,
            id,
        )
        .map_err(Into::into)
    }

    fn delete_batch(&mut self, refs: &[DatabaseRef]) -> Result<(), DbError> {
        let access = HistoryAccess::lock();
        access.invalidate_recording_snapshots(self, refs)?;
        let file = hdf5::File::open_rw(&self.path).map_err(|e| DbError::Backend(e.to_string()))?;
        let by_id = file
            .group("by_identity")
            .map_err(|e| DbError::Backend(e.to_string()))?;
        let mut attachments = Vec::new();
        let mut groups_to_delete = Vec::new();
        for db_ref in refs {
            if let Some((_, group)) =
                crate::copy::find_identity_group(&by_id, &db_ref.identity, Some(&db_ref.group_name))
                    .map_err(DbError::from)?
            {
                groups_to_delete.push((group, &db_ref.group_name));
            }
        }
        for (id_grp, group_name) in groups_to_delete {
            if id_grp.link_exists(group_name) {
                if let Ok(rec_grp) = id_grp.group(group_name) {
                    attachments.extend(crate::copy::log_attachment_ids(&rec_grp));
                }
                id_grp
                    .unlink(group_name)
                    .map_err(|e| DbError::Backend(e.to_string()))?;
            }
        }
        log_attachment::delete_files(
            &log_attachment::logs_directory_for_database(&self.path),
            &attachments,
        );
        Ok(())
    }

    fn rename_identity(&mut self, old: &str, new: &str) -> Result<(), DbError> {
        let access = HistoryAccess::lock();
        crate::copy::rename_identity(&self.path, old, new, |by_id, recordings| {
            let mut expired = recordings.to_vec();
            expired.extend(
                access
                    .observed_recording_addresses(self)?
                    .into_iter()
                    .filter(|recording| {
                        if recordings.contains(recording) {
                            return false;
                        }
                        match crate::copy::recording_has_independent_links(by_id, recording) {
                            Ok(independent) => !independent,
                            Err(error) => {
                                log::warn!(
                                    "Could not verify recording links for {recording}: {error}"
                                );
                                true
                            }
                        }
                    }),
            );
            access.invalidate_recording_snapshots(self, &expired)
        })
        .map_err(Into::into)
    }
}

#[cfg(test)]
mod tests {
    use super::DbError;

    /// Each wording libhdf5 uses for the flags an unclean shutdown leaves.
    /// Carried on [`hdf5::Error::Internal`], which has no code stack, the way
    /// a message-only failure reaches the classifier.
    #[test]
    fn a_stale_flag_is_a_write_lock() {
        for msg in [
            "file is already open for write",
            "file is already open for write/SWMR write",
            "may use <h5clear file> to clear file consistency flags",
        ] {
            assert!(
                matches!(
                    super::classify_open_error(hdf5::Error::Internal(msg.to_owned())),
                    DbError::WriteLocked
                ),
                "{msg}"
            );
        }
    }

    #[test]
    fn anything_else_keeps_its_message() {
        let err = hdf5::Error::Internal("bad superblock".to_owned());
        assert!(matches!(
            super::classify_open_error(err),
            DbError::Backend(msg) if msg.contains("bad superblock")
        ));
    }
}
