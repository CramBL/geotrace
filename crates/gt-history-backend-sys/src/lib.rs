use crate::copy::{list_recordings, load_recording_bytes};
use gt_types::DatabaseRef;
use gt_types::history::{
    CURRENT_SCHEMA_VERSION, DbError, HistoryDatabase, RecordingEntry, RecordingMeta,
    SCHEMA_VERSION_ATTR,
};

use parking_lot::Mutex;
use std::path::{Path, PathBuf};

static DB_LOCK: Mutex<()> = Mutex::new(());

pub mod copy;

/// Extract recording metadata from raw GTD file bytes.
///
/// libhdf5 reads from a path rather than a byte slice, so the bytes are staged
/// in a temporary file. Counts come from the relevant datasets' shapes; the
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

    // Count rows in an optional data group's index dataset; absent groups
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
            Self::validate_existing(path)?;
        } else {
            Self::create_new(path)?;
        }
        Ok(Self {
            path: path.to_owned(),
        })
    }

    fn insert(
        &mut self,
        identity: &str,
        meta: &RecordingMeta,
        gtd_bytes: &[u8],
    ) -> Result<DatabaseRef, DbError> {
        let _guard = DB_LOCK.lock();
        crate::copy::insert_recording(&self.path, identity, meta, gtd_bytes)
            .map(|rec_name| DatabaseRef {
                identity: identity.to_owned(),
                group_name: rec_name,
            })
            .map_err(Into::into)
    }

    fn delete(&mut self, db_ref: &DatabaseRef) -> Result<(), DbError> {
        let _guard = DB_LOCK.lock();
        let file = hdf5::File::open_rw(&self.path).map_err(|e| DbError::Backend(e.to_string()))?;
        let path = format!("by_identity/{}/{}", db_ref.identity, db_ref.group_name);
        if file.link_exists(&path) {
            file.unlink(&path)
                .map_err(|e| DbError::Backend(e.to_string()))?;
        }
        Ok(())
    }

    fn load_bytes(&self, db_ref: &DatabaseRef) -> Result<Vec<u8>, DbError> {
        let _guard = DB_LOCK.lock();
        load_recording_bytes(&self.path, &db_ref.identity, &db_ref.group_name).map_err(Into::into)
    }

    fn list_recordings(&self) -> Result<Vec<RecordingEntry>, DbError> {
        let _guard = DB_LOCK.lock();
        list_recordings(&self.path).map_err(Into::into)
    }

    fn is_duplicate(&self, identity: &str, meta: &RecordingMeta) -> Result<bool, DbError> {
        let _guard = DB_LOCK.lock();
        crate::copy::is_duplicate(&self.path, identity, meta).map_err(Into::into)
    }

    fn delete_batch(&mut self, refs: &[DatabaseRef]) -> Result<(), DbError> {
        for db_ref in refs {
            self.delete(db_ref)?;
        }
        Ok(())
    }

    fn path(&self) -> &Path {
        &self.path
    }
}
