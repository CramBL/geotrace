use crate::copy::list_recordings;
use gt_history_types::{
    CURRENT_SCHEMA_VERSION, DatabaseRef, DbError, HistoryDatabase, RecordingEntry, RecordingMeta,
    SCHEMA_VERSION_ATTR, StoredRecording, StoredSegmentation, TrackRange,
};

use parking_lot::Mutex;
use std::path::{Path, PathBuf};

static DB_LOCK: Mutex<()> = Mutex::new(());

pub mod copy;

/// Extract recording metadata from raw GTD file bytes.
///
/// libhdf5 reads from a path rather than a byte slice, so the bytes are staged
/// in a temporary file. Counts come from the relevant datasets' shapes. The
/// time bounds from the first and last `nav_points/time` entries.
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

    let (start_us, end_us) = if nav_point_count > 0 {
        let times: Vec<i64> = time
            .read_raw()
            .map_err(|e| DbError::Backend(e.to_string()))?;
        (
            times.first().copied().unwrap_or(0),
            times.last().copied().unwrap_or(0),
        )
    } else {
        (0, 0)
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
        start_us,
        end_us,
        nav_point_count,
        sat_report_count: count_rows("sat_reports", "nav_point_idx"),
        marker_count: count_rows("markers", "time"),
        event_marker_count: count_rows("event_markers", "sys_time_us"),
        gtd_size_bytes: bytes.len() as u64,
    })
}

/// Map libhdf5's "file is already open for write" / consistency-flags open
/// failure to the recoverable [`DbError::WriteLocked`]. Pass other errors through.
fn into_write_lock(e: DbError) -> DbError {
    if let DbError::Backend(msg) = &e
        && (msg.contains("already open for write")
            || msg.contains("h5clear")
            || msg.contains("consistency flags"))
    {
        return DbError::WriteLocked;
    }
    e
}

pub struct SysDb {
    path: PathBuf,
}

impl SysDb {
    fn create_new(path: &Path) -> Result<(), DbError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Create with a persistent free-space manager so the space vacated by
        // deleting recordings is tracked across sessions and reused by later
        // inserts, rather than left as permanent dead space in the file.
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

    fn validate_existing(path: &Path) -> Result<(), DbError> {
        let file = hdf5::File::open(path).map_err(|e| DbError::Backend(e.to_string()))?;
        let schema_version = file
            .attr(SCHEMA_VERSION_ATTR)
            .and_then(|a| a.read_scalar::<i64>())
            .unwrap_or(0);
        if schema_version > CURRENT_SCHEMA_VERSION {
            return Err(DbError::SchemaTooNew {
                found: schema_version,
                supported: CURRENT_SCHEMA_VERSION,
            });
        }
        Ok(())
    }
}

impl HistoryDatabase for SysDb {
    fn open_or_create(path: &Path) -> Result<Self, DbError> {
        let _guard = DB_LOCK.lock();
        if path.exists() {
            // Surface a stale "open for write" lock as a distinct error so the
            // app can offer to clear it (see `clear_write_lock`).
            Self::validate_existing(path).map_err(into_write_lock)?;
            // A database written by the pure-Rust backend can be read but not
            // extended by libhdf5. Migrate it to a native file once so inserts
            // and deletes work.
            if !crate::copy::is_native_writable(path) {
                log::info!(
                    "Migrating history database at {} to the native HDF5 format",
                    path.display()
                );
                crate::copy::migrate_to_native(path)?;
            }
            crate::copy::repair_unindexed_recordings(path)?;
        } else {
            Self::create_new(path)?;
        }
        Ok(Self {
            path: path.to_owned(),
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

    fn insert(
        &mut self,
        identity: &str,
        meta: &RecordingMeta,
        tracks: &[TrackRange],
        settings: StoredSegmentation,
        gtd_bytes: &[u8],
    ) -> Result<DatabaseRef, DbError> {
        let _guard = DB_LOCK.lock();
        crate::copy::insert_recording(&self.path, identity, meta, tracks, settings, gtd_bytes)
            .map_err(Into::into)
    }

    fn load(&self, db_ref: &DatabaseRef) -> Result<StoredRecording, DbError> {
        let _guard = DB_LOCK.lock();
        crate::copy::load_recording(&self.path, &db_ref.identity, &db_ref.group_name)
            .map_err(Into::into)
    }

    fn set_tracks(
        &mut self,
        db_ref: &DatabaseRef,
        tracks: &[TrackRange],
        settings: StoredSegmentation,
    ) -> Result<(), DbError> {
        let _guard = DB_LOCK.lock();
        crate::copy::set_tracks(
            &self.path,
            &db_ref.identity,
            &db_ref.group_name,
            tracks,
            settings,
        )
        .map_err(Into::into)
    }

    fn set_tracks_hidden(
        &mut self,
        db_ref: &DatabaseRef,
        track_indices: &[usize],
        hidden: bool,
    ) -> Result<(), DbError> {
        let _guard = DB_LOCK.lock();
        crate::copy::set_tracks_hidden(
            &self.path,
            &db_ref.identity,
            &db_ref.group_name,
            track_indices,
            hidden,
        )
        .map_err(Into::into)
    }

    fn list_recordings(&self) -> Result<Vec<RecordingEntry>, DbError> {
        let _guard = DB_LOCK.lock();
        list_recordings(&self.path).map_err(Into::into)
    }

    fn is_duplicate(&self, meta: &RecordingMeta) -> Result<bool, DbError> {
        let _guard = DB_LOCK.lock();
        crate::copy::is_duplicate(&self.path, meta).map_err(Into::into)
    }

    fn delete_batch(&mut self, refs: &[DatabaseRef]) -> Result<(), DbError> {
        let _guard = DB_LOCK.lock();
        let file = hdf5::File::open_rw(&self.path).map_err(|e| DbError::Backend(e.to_string()))?;
        let by_id = file
            .group("by_identity")
            .map_err(|e| DbError::Backend(e.to_string()))?;
        for db_ref in refs {
            let storage_name = gt_history_types::identity_group_name(&db_ref.identity);
            let id_grp = match by_id.group(&storage_name) {
                Ok(group) => Some(group),
                Err(encoded_err) => {
                    if db_ref.identity.contains('/') {
                        log::debug!(
                            "delete_batch could not open encoded identity group for identity={:?}: {encoded_err}",
                            db_ref.identity
                        );
                        None
                    } else {
                        match by_id.group(&db_ref.identity) {
                            Ok(group) => Some(group),
                            Err(raw_err) => {
                                log::debug!(
                                    "delete_batch could not open encoded or legacy identity group for identity={:?}: encoded={encoded_err}; legacy={raw_err}",
                                    db_ref.identity
                                );
                                None
                            }
                        }
                    }
                }
            };
            let Some(id_grp) = id_grp else {
                log::warn!(
                    "delete_batch could not find identity={:?}, group={:?}",
                    db_ref.identity,
                    db_ref.group_name
                );
                continue;
            };
            if id_grp.link_exists(&db_ref.group_name) {
                id_grp
                    .unlink(&db_ref.group_name)
                    .map_err(|e| DbError::Backend(e.to_string()))?;
            }
        }
        Ok(())
    }

    fn path(&self) -> &Path {
        &self.path
    }
}
