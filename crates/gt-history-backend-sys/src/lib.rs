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
pub struct SysDb {
    path: PathBuf,
}

impl SysDb {
    fn create_new(path: &Path) -> Result<(), DbError> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let file = hdf5::File::create(path).map_err(|e| DbError::Backend(e.to_string()))?;
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
        println!("SysDb::insert called");
        println!("Calling insert_recording");
        let res = crate::copy::insert_recording(&self.path, identity, meta, gtd_bytes);
        match &res {
            Ok(_) => println!("insert_recording succeeded"),
            Err(e) => println!("insert_recording failed: {:?}", e),
        }
        res.map(|rec_name| DatabaseRef {
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
