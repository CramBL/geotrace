use std::collections::HashMap;
use std::io;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, LazyLock, Weak};

use parking_lot::{Mutex, ReentrantMutex, ReentrantMutexGuard};
use same_file::Handle;

use crate::{
    DatabaseRef, DbError, HistoryDatabase, ReadOnlyHistoryDatabase, RecordingMeta, StoredRecording,
    StoredSegmentation, TrackRange, TrackState,
};

pub struct HistoryAccess {
    _guard: ReentrantMutexGuard<'static, ()>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TrackLayoutVisibility {
    Clear,
    Preserve,
}

pub struct RecordingReplacement<'a> {
    pub meta: &'a RecordingMeta,
    pub tracks: &'a [TrackRange],
    pub settings: StoredSegmentation,
    pub bytes: &'a [u8],
    visibility: TrackLayoutVisibility,
}

impl<'a> RecordingReplacement<'a> {
    pub(crate) fn new(
        meta: &'a RecordingMeta,
        tracks: &'a [TrackRange],
        settings: StoredSegmentation,
        bytes: &'a [u8],
    ) -> Self {
        Self {
            meta,
            tracks,
            settings,
            bytes,
            visibility: TrackLayoutVisibility::Clear,
        }
    }

    pub fn visibility(&self) -> TrackLayoutVisibility {
        self.visibility
    }
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
        let snapshot = self.snapshot(identity, recording.clone(), stored.track_rows().to_vec());
        Ok((stored, snapshot))
    }

    pub fn replace_track_layout(
        &self,
        database: &mut impl HistoryDatabase,
        expected: &StoredTrackSnapshot,
        rows: Vec<TrackRange>,
        settings: StoredSegmentation,
        visibility: TrackLayoutVisibility,
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
        database.set_track_layout(recording, &rows, settings, visibility)?;
        let identity = Handle::from_path(database.path())?;
        Ok(self.snapshot(identity, recording.clone(), rows))
    }

    /// Retains the stored UI subtree when surviving rows keep their track numbers.
    pub fn replace_recording_after_track_purge(
        &self,
        database: &mut impl HistoryDatabase,
        before: StoredTrackSnapshot,
        meta: &RecordingMeta,
        tracks: &[TrackRange],
        settings: StoredSegmentation,
        bytes: &[u8],
    ) -> Result<Option<StoredTrackSnapshotUpdate>, DbError> {
        self.verify_layout_revision(database, before.layout())?;
        if !before.is_current_with_membership() || !before.is_partial_purge_successor(tracks) {
            return Err(DbError::StoredTrackLayoutChanged);
        }

        let recording = before.recording();
        database.replace_recording_payload(
            recording,
            RecordingReplacement {
                visibility: TrackLayoutVisibility::Preserve,
                ..RecordingReplacement::new(meta, tracks, settings, bytes)
            },
        )?;
        match self.read_track_snapshot(database, recording) {
            Ok(after) if after.rows() == tracks => {
                Ok(Some(StoredTrackSnapshotUpdate { before, after }))
            }
            result => {
                let error = result.err().unwrap_or(DbError::StoredTrackLayoutChanged);
                log::warn!("Could not verify the remaining track table of {recording}: {error}");
                Ok(None)
            }
        }
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

    pub fn invalidate_track_membership(
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
            if let Some(membership) = revision.membership.upgrade() {
                membership.store(false, Ordering::Release);
            }
            revision.membership = Weak::new();
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

    pub fn verify_layout_revision(
        &self,
        database: &impl ReadOnlyHistoryDatabase,
        expected: &StoredLayoutRevision,
    ) -> Result<(), DbError> {
        let identity = Handle::from_path(database.path())?;
        let current = REVISIONS
            .lock()
            .get(&identity)
            .and_then(|revisions| revisions.get(expected.recording()))
            .and_then(|revision| revision.layout.upgrade());
        if expected.is_current()
            && current.is_some_and(|current| Arc::ptr_eq(&current, &expected.revision))
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
            database.retain(|_, revision| {
                revision.payload.strong_count() > 0 || revision.layout.strong_count() > 0
            });
            !database.is_empty()
        });
        let revision = revisions
            .entry(identity)
            .or_default()
            .entry(recording.clone())
            .or_default();
        StoredTrackSnapshot {
            layout: StoredLayoutRevision {
                recording,
                revision: current_revision(&mut revision.layout, || AtomicBool::new(true)),
            },
            rows: rows.into(),
            membership_revision: current_revision(&mut revision.membership, || {
                AtomicBool::new(true)
            }),
            payload_revision: current_revision(&mut revision.payload, || ()),
        }
    }
}

#[derive(Clone, Debug)]
pub struct StoredLayoutRevision {
    recording: DatabaseRef,
    revision: Arc<AtomicBool>,
}

impl StoredLayoutRevision {
    pub fn recording(&self) -> &DatabaseRef {
        &self.recording
    }

    pub fn is_current(&self) -> bool {
        self.revision.load(Ordering::Acquire)
    }

    pub fn same_observation(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.revision, &other.revision)
    }
}

#[derive(Clone, Debug)]
pub struct StoredTrackSnapshot {
    layout: StoredLayoutRevision,
    rows: Arc<[TrackRange]>,
    membership_revision: Arc<AtomicBool>,
    payload_revision: Arc<()>,
}

impl StoredTrackSnapshot {
    pub fn recording(&self) -> &DatabaseRef {
        self.layout.recording()
    }

    pub fn layout(&self) -> &StoredLayoutRevision {
        &self.layout
    }

    pub fn rows(&self) -> &[TrackRange] {
        &self.rows
    }

    /// Returns false after this process invalidates the snapshot.
    /// Check a fresh observation under the same [HistoryAccess] lock as the write.
    pub fn is_current(&self) -> bool {
        self.layout.is_current()
    }

    /// Shelving invalidates the membership revision and preserves the layout revision.
    pub fn is_current_with_membership(&self) -> bool {
        self.is_current() && self.membership_revision.load(Ordering::Acquire)
    }

    pub fn same_observation(&self, other: &Self) -> bool {
        self.layout.same_observation(&other.layout)
    }

    pub fn same_payload_revision(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.payload_revision, &other.payload_revision)
    }

    fn is_partial_purge_successor(&self, rows: &[TrackRange]) -> bool {
        if self.rows.len() != rows.len() {
            return false;
        }
        let mut cursor = 0;
        let mut removed_track = false;
        let mut retained_track = false;
        for (old, new) in self.rows.iter().zip(rows) {
            if new.start != cursor {
                return false;
            }
            if new.state == TrackState::Deleted {
                if new.end != new.start {
                    return false;
                }
                removed_track |= old.state != TrackState::Deleted;
            } else {
                let same_length = old
                    .end
                    .checked_sub(old.start)
                    .is_some_and(|length| new.end.checked_sub(new.start) == Some(length));
                if old.state != new.state || !same_length {
                    return false;
                }
                retained_track = true;
            }
            cursor = new.end;
        }
        removed_track && retained_track
    }
}

pub struct StoredTrackSnapshotUpdate {
    before: StoredTrackSnapshot,
    after: StoredTrackSnapshot,
}

impl StoredTrackSnapshotUpdate {
    pub fn before(&self) -> &StoredTrackSnapshot {
        &self.before
    }

    pub fn after(&self) -> &StoredTrackSnapshot {
        &self.after
    }
}

#[derive(Default)]
struct TrackRevisions {
    layout: Weak<AtomicBool>,
    membership: Weak<AtomicBool>,
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
