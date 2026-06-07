use gt_types::DatabaseRef;
use gt_types::history::{DbError, HistoryDatabase, RecordingEntry, RecordingMeta};
use std::path::{Path, PathBuf};

pub struct SysDb {
    path: PathBuf,
}

impl HistoryDatabase for SysDb {
    fn open_or_create(path: &Path) -> Result<Self, DbError> {
        // Placeholder implementation
        Ok(Self {
            path: path.to_owned(),
        })
    }

    fn insert(
        &mut self,
        _identity: &str,
        _meta: &RecordingMeta,
        _bytes: &[u8],
    ) -> Result<DatabaseRef, DbError> {
        Err(DbError::Backend("SysDb not implemented".to_string()))
    }

    fn delete(&mut self, _db_ref: &DatabaseRef) -> Result<(), DbError> {
        Err(DbError::Backend("SysDb not implemented".to_string()))
    }

    fn load_bytes(&self, _db_ref: &DatabaseRef) -> Result<Vec<u8>, DbError> {
        Err(DbError::Backend("SysDb not implemented".to_string()))
    }

    fn list_recordings(&self) -> Result<Vec<RecordingEntry>, DbError> {
        Ok(vec![])
    }

    fn is_duplicate(&self, _identity: &str, _meta: &RecordingMeta) -> Result<bool, DbError> {
        Ok(false)
    }

    fn delete_batch(&mut self, _refs: &[DatabaseRef]) -> Result<(), DbError> {
        Ok(())
    }

    fn path(&self) -> &Path {
        &self.path
    }
}
