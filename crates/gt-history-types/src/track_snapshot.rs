use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Weak};

use parking_lot::{Mutex, ReentrantMutex, ReentrantMutexGuard};
use same_file::Handle;

use crate::{
    DatabaseRef, DbError, HistoryDatabase, ReadOnlyHistoryDatabase, StoredRecording,
    StoredSegmentation, TrackRange, TrackState,
};

pub struct HistoryAccess {
    _guard: ReentrantMutexGuard<'static, ()>,
}

impl HistoryAccess {
    pub fn lock() -> Self {
        Self {
            _guard: HISTORY_LOCK.lock(),
        }
    }

    pub fn read_track_snapshot(
        &self,
        database: &impl ReadOnlyHistoryDatabase,
        recording: &DatabaseRef,
    ) -> Result<StoredTrackSnapshot, DbError> {
        require_recording_group(recording)?;
        let identity = Handle::from_path(database.path())?;
        let rows = database.stored_track_table(recording)?;
        Ok(self.snapshot(identity, recording.clone(), rows))
    }

    pub fn load_recording_with_snapshot(
        &self,
        database: &impl ReadOnlyHistoryDatabase,
        recording: &DatabaseRef,
    ) -> Result<(StoredRecording, StoredTrackSnapshot), DbError> {
        require_recording_group(recording)?;
        let identity = Handle::from_path(database.path())?;
        let stored = database.load(recording)?;
        let snapshot = self.snapshot(identity, recording.clone(), stored.tracks.clone());
        Ok((stored, snapshot))
    }

    pub fn replace_track_layout(
        &self,
        database: &mut impl HistoryDatabase,
        expected: &StoredTrackSnapshot,
        rows: Vec<TrackRange>,
        settings: StoredSegmentation,
    ) -> Result<StoredTrackSnapshot, DbError> {
        let identity = Handle::from_path(database.path())?;
        let recording = expected.recording();
        let current = REVISIONS
            .lock()
            .get(&identity)
            .and_then(|database| database.get(recording))
            .and_then(|revision| revision.payload.upgrade());
        if !current.is_some_and(|current| Arc::ptr_eq(&current, &expected.payload_revision)) {
            return Err(DbError::StoredTrackLayoutChanged);
        }
        database.set_tracks(recording, &rows, settings)?;
        let identity = Handle::from_path(database.path())?;
        Ok(self.snapshot(identity, recording.clone(), rows))
    }

    pub fn invalidate_track_snapshots(
        &self,
        database: &impl ReadOnlyHistoryDatabase,
        recording: &DatabaseRef,
    ) -> Result<(), DbError> {
        require_recording_group(recording)?;
        let identity = Handle::from_path(database.path())?;
        if let Some(revision) = REVISIONS
            .lock()
            .get_mut(&identity)
            .and_then(|database| database.get_mut(recording))
        {
            revision.invalidate();
            revision.layout = Weak::new();
        }
        Ok(())
    }

    pub fn invalidate_recording_snapshots(
        &self,
        database: &impl ReadOnlyHistoryDatabase,
        recordings: &[DatabaseRef],
    ) -> Result<(), DbError> {
        for recording in recordings {
            require_recording_group(recording)?;
        }
        let identity = Handle::from_path(database.path())?;
        if let Some(revisions) = REVISIONS.lock().get_mut(&identity) {
            for recording in recordings {
                if let Some(revision) = revisions.remove(recording) {
                    revision.invalidate();
                }
            }
        }
        Ok(())
    }

    pub fn invalidate_identity_snapshots(
        &self,
        database: &impl ReadOnlyHistoryDatabase,
        identities: [&str; 2],
    ) -> Result<(), DbError> {
        let identity = Handle::from_path(database.path())?;
        if let Some(revisions) = REVISIONS.lock().get_mut(&identity) {
            revisions.retain(|recording, revision| {
                if identities.contains(&recording.identity.as_str()) {
                    revision.invalidate();
                    false
                } else {
                    true
                }
            });
        }
        Ok(())
    }

    pub fn invalidate_database_snapshots(&self, path: &Path) -> io::Result<()> {
        let identity = Handle::from_path(path)?;
        if let Some(revisions) = REVISIONS.lock().remove(&identity) {
            for revision in revisions.values() {
                revision.invalidate();
            }
        }
        Ok(())
    }

    pub fn verify_track_snapshot(
        &self,
        expected: &StoredTrackSnapshot,
        actual: &StoredTrackSnapshot,
    ) -> Result<(), DbError> {
        if expected.same_observation(actual)
            && expected.is_current()
            && expected.rows.len() == actual.rows.len()
            && expected
                .rows
                .iter()
                .zip(actual.rows())
                .all(|(expected, actual)| {
                    expected.start == actual.start
                        && expected.end == actual.end
                        && (expected.state == TrackState::Deleted)
                            == (actual.state == TrackState::Deleted)
                })
        {
            Ok(())
        } else {
            Err(DbError::StoredTrackLayoutChanged)
        }
    }

    fn snapshot(
        &self,
        identity: Handle,
        recording: DatabaseRef,
        rows: Vec<TrackRange>,
    ) -> StoredTrackSnapshot {
        let mut revisions = REVISIONS.lock();
        revisions.retain(|_, database| {
            database.retain(|_, revision| revision.payload.strong_count() > 0);
            !database.is_empty()
        });
        let revision = revisions
            .entry(identity)
            .or_default()
            .entry(recording.clone())
            .or_default();
        StoredTrackSnapshot {
            recording,
            rows: rows.into(),
            layout_revision: current_revision(&mut revision.layout, || AtomicBool::new(true)),
            payload_revision: current_revision(&mut revision.payload, || ()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct StoredTrackSnapshot {
    recording: DatabaseRef,
    rows: Arc<[TrackRange]>,
    layout_revision: Arc<AtomicBool>,
    payload_revision: Arc<()>,
}

impl StoredTrackSnapshot {
    pub fn recording(&self) -> &DatabaseRef {
        &self.recording
    }

    pub fn rows(&self) -> &[TrackRange] {
        &self.rows
    }

    /// Returns false after this process invalidates the snapshot.
    /// Check a fresh observation under the same [HistoryAccess] lock as the write.
    pub fn is_current(&self) -> bool {
        self.layout_revision.load(Ordering::Acquire)
    }

    pub fn same_observation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.layout_revision, &other.layout_revision)
    }
}

#[derive(Default)]
struct TrackRevisions {
    layout: Weak<AtomicBool>,
    payload: Weak<()>,
}

impl TrackRevisions {
    fn invalidate(&self) {
        if let Some(layout) = self.layout.upgrade() {
            layout.store(false, Ordering::Release);
        }
    }
}

fn current_revision<T>(revision: &mut Weak<T>, new: impl FnOnce() -> T) -> Arc<T> {
    if let Some(current) = revision.upgrade() {
        return current;
    }
    let current = Arc::new(new());
    *revision = Arc::downgrade(&current);
    current
}

fn require_recording_group(recording: &DatabaseRef) -> Result<(), DbError> {
    if crate::is_history_link_name(&recording.group_name) {
        Ok(())
    } else {
        Err(DbError::Backend(format!(
            "invalid recording group {:?}",
            recording.group_name
        )))
    }
}

pub static HISTORY_LOCK: ReentrantMutex<()> = ReentrantMutex::new(());

// The open file handle prevents file identity reuse while revisions are registered.
static REVISIONS: LazyLock<Mutex<HashMap<Handle, HashMap<DatabaseRef, TrackRevisions>>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));
