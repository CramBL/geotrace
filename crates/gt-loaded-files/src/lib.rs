use std::borrow::Cow;
use std::ops::{Deref, Index, IndexMut};

use gt_history_types::{DatabaseRef, RecordingMeta};
use gt_types::{FileIdx, LoadedFile};

mod recording_names;

pub use recording_names::RecordingNames;

/// Prefix marking an identity that GeoTrace derived automatically (from the
/// recording's title/device/filename) rather than one supplied explicitly via
/// the SDK. Produced by `gt_loader::derive_identity`.
pub const AUTO_IDENTITY_PREFIX: &str = "auto:";

/// Split an identity into its user-facing text and whether it was auto-derived.
///
/// The stored identity keeps the [`AUTO_IDENTITY_PREFIX`] marker, but that
/// marker is internal bookkeeping and must not be shown verbatim. Every place
/// that displays an identity strips it through this one helper.
pub fn display_identity(identity: &str) -> (&str, bool) {
    match identity.strip_prefix(AUTO_IDENTITY_PREFIX) {
        Some(name) => (name, true),
        None => (identity, false),
    }
}

/// App/session-side history metadata for one loaded file.
///
/// This is not persisted history schema. It describes how the currently loaded
/// `LoadedFile` relates to history, if at all.
#[derive(Debug, Clone)]
pub enum FileHistory {
    None,
    Recording {
        identity: String,
        meta: RecordingMeta,
        db_ref: Option<DatabaseRef>,
    },
}

impl FileHistory {
    pub fn recording(identity: String, meta: RecordingMeta, db_ref: Option<DatabaseRef>) -> Self {
        Self::Recording {
            identity,
            meta,
            db_ref,
        }
    }

    pub fn is_stored(&self) -> bool {
        matches!(
            self,
            Self::Recording {
                db_ref: Some(_),
                ..
            }
        )
    }

    pub fn db_ref(&self) -> Option<&DatabaseRef> {
        match self {
            Self::Recording {
                db_ref: Some(db_ref),
                ..
            } => Some(db_ref),
            Self::None | Self::Recording { db_ref: None, .. } => None,
        }
    }

    pub fn meta(&self) -> Option<RecordingMeta> {
        match self {
            Self::Recording { meta, .. } => Some(*meta),
            Self::None => None,
        }
    }

    fn identity_key<'a>(&'a self, file: &'a LoadedFile) -> Cow<'a, str> {
        match self {
            Self::Recording { identity, .. } => Cow::Borrowed(identity.as_str()),
            Self::None => Cow::Borrowed(file.metadata.filename.as_str()),
        }
    }

    /// The recording identity, if this file has one. Unlike [`identity_key`],
    /// returns `None` (rather than the filename) for files with no history
    /// association, so callers can distinguish a real identity from a fallback.
    ///
    /// [`identity_key`]: Self::identity_key
    fn identity(&self) -> Option<&str> {
        match self {
            Self::Recording { identity, .. } => Some(identity.as_str()),
            Self::None => None,
        }
    }
}

/// Read-only view of loaded files and their app/session sidecar metadata.
///
/// `FileIdx(n)` indexes the same logical file in every method on this view:
/// `files()[n]`, `get(n)`, and `entry_for(FileIdx(n))` all refer to the same
/// file. The invariant is enforced by constructing this view only from
/// [`LoadedFiles`], whose file and history sidecar storage is private and is
/// mutated through methods that keep both vectors aligned.
#[derive(Debug, Clone, Copy)]
pub struct LoadedFilesView<'a> {
    loaded_files: &'a LoadedFiles,
}

/// One loaded file paired with its app/session sidecar metadata.
#[derive(Debug, Clone, Copy)]
pub struct LoadedFileEntry<'a> {
    file: &'a LoadedFile,
    history: &'a FileHistory,
}

impl<'a> LoadedFilesView<'a> {
    pub fn files(&self) -> &'a [LoadedFile] {
        self.loaded_files.files()
    }

    pub fn len(&self) -> usize {
        self.loaded_files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.loaded_files.is_empty()
    }

    pub fn get(&self, index: usize) -> Option<LoadedFileEntry<'a>> {
        let file = self.loaded_files.files.get(index)?;
        let history = self.loaded_files.history.get(index)?;
        Some(LoadedFileEntry { file, history })
    }

    pub fn entry_for(&self, file: FileIdx) -> Option<LoadedFileEntry<'a>> {
        self.get(file.as_usize())
    }

    pub fn entries(&self) -> impl ExactSizeIterator<Item = LoadedFileEntry<'a>> + 'a {
        self.loaded_files
            .files
            .iter()
            .zip(&self.loaded_files.history)
            .map(|(file, history)| LoadedFileEntry { file, history })
    }

    pub fn file_stored_in_history(&self, file: FileIdx) -> bool {
        self.entry_for(file)
            .is_some_and(|entry| entry.is_stored_in_history())
    }

    pub fn recording_metas(&self) -> Vec<RecordingMeta> {
        self.entries()
            .filter_map(|entry| entry.history().meta())
            .collect()
    }
}

impl<'a> LoadedFileEntry<'a> {
    pub fn file(&self) -> &'a LoadedFile {
        self.file
    }

    pub fn history(&self) -> &'a FileHistory {
        self.history
    }

    pub fn identity_key(&self) -> Cow<'a, str> {
        self.history.identity_key(self.file)
    }

    /// The recording identity, or `None` for files not associated with history.
    pub fn identity(&self) -> Option<&'a str> {
        self.history.identity()
    }

    pub fn is_stored_in_history(&self) -> bool {
        self.history.is_stored()
    }
}

/// Loaded files plus app/session metadata that must remain index-aligned.
///
/// The backing vectors are private so callers cannot construct mismatched file
/// and history sidecar slices. Use [`LoadedFiles::view`] when read-only
/// consumers need both the files and their sidecar metadata.
#[derive(Debug, Clone, Default)]
pub struct LoadedFiles {
    files: Vec<LoadedFile>,
    history: Vec<FileHistory>,
}

impl LoadedFiles {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn len(&self) -> usize {
        self.files.len()
    }

    pub fn is_empty(&self) -> bool {
        self.files.is_empty()
    }

    pub fn files(&self) -> &[LoadedFile] {
        &self.files
    }

    pub fn view(&self) -> LoadedFilesView<'_> {
        debug_assert_eq!(self.files.len(), self.history.len());
        LoadedFilesView { loaded_files: self }
    }

    pub fn files_mut(&mut self) -> &mut [LoadedFile] {
        &mut self.files
    }

    pub fn iter(&self) -> std::slice::Iter<'_, LoadedFile> {
        self.files.iter()
    }

    pub fn iter_mut(&mut self) -> std::slice::IterMut<'_, LoadedFile> {
        self.files.iter_mut()
    }

    pub fn get(&self, index: usize) -> Option<&LoadedFile> {
        self.files.get(index)
    }

    pub fn get_mut(&mut self, index: usize) -> Option<&mut LoadedFile> {
        self.files.get_mut(index)
    }

    pub fn push(&mut self, file: LoadedFile, history: FileHistory) {
        self.files.push(file);
        self.history.push(history);
        debug_assert_eq!(self.files.len(), self.history.len());
    }

    /// Re-point loaded recordings after their history identity was renamed from
    /// `old` to `new`. Only the identity changes. The recording `group_name`
    /// is stable across a rename, so the [`DatabaseRef`] stays valid.
    pub fn rename_identity(&mut self, old: &str, new: &str) {
        for history in &mut self.history {
            if let FileHistory::Recording {
                identity, db_ref, ..
            } = history
                && identity == old
            {
                identity.clear();
                identity.push_str(new);
                if let Some(db_ref) = db_ref {
                    db_ref.identity = new.to_owned();
                }
            }
        }
    }

    pub fn remove_file(&mut self, index: usize) -> Option<(LoadedFile, FileHistory)> {
        if index >= self.files.len() {
            return None;
        }
        let file = self.files.remove(index);
        let history = self.history.remove(index);
        debug_assert_eq!(self.files.len(), self.history.len());
        Some((file, history))
    }
}

impl Deref for LoadedFiles {
    type Target = [LoadedFile];

    fn deref(&self) -> &Self::Target {
        self.files()
    }
}

impl Index<usize> for LoadedFiles {
    type Output = LoadedFile;

    #[expect(
        clippy::indexing_slicing,
        reason = "Index follows Vec indexing semantics"
    )]
    fn index(&self, index: usize) -> &Self::Output {
        &self.files[index]
    }
}

impl IndexMut<usize> for LoadedFiles {
    #[expect(
        clippy::indexing_slicing,
        reason = "IndexMut follows Vec indexing semantics"
    )]
    fn index_mut(&mut self, index: usize) -> &mut Self::Output {
        &mut self.files[index]
    }
}

impl<'a> IntoIterator for &'a LoadedFiles {
    type Item = &'a LoadedFile;
    type IntoIter = std::slice::Iter<'a, LoadedFile>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

impl<'a> IntoIterator for &'a mut LoadedFiles {
    type Item = &'a mut LoadedFile;
    type IntoIter = std::slice::IterMut<'a, LoadedFile>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter_mut()
    }
}

#[cfg(test)]
mod tests {
    use super::{DatabaseRef, FileHistory, LoadedFiles, RecordingMeta, display_identity};
    use gt_types::LoadedFile;

    fn meta() -> RecordingMeta {
        RecordingMeta {
            start_us: 0,
            end_us: 0,
            nav_point_count: 0,
            sat_report_count: 0,
            marker_count: 0,
            event_marker_count: 0,
            gtd_size_bytes: 0,
        }
    }

    #[test]
    fn identity_is_some_for_recordings_and_none_otherwise() {
        let recording = FileHistory::recording("auto:ride.gtd".to_owned(), meta(), None);
        assert_eq!(recording.identity(), Some("auto:ride.gtd"));
        assert_eq!(FileHistory::None.identity(), None);
    }

    #[test]
    fn display_identity_strips_auto_prefix() {
        assert_eq!(
            display_identity("auto:Morning ride"),
            ("Morning ride", true)
        );
        assert_eq!(display_identity("explicit-id"), ("explicit-id", false));
    }

    fn empty_file() -> LoadedFile {
        LoadedFile {
            metadata: gt_test_utils::empty_file_metadata(),
            tracks: Vec::new(),
            event_marker_styles: std::collections::HashMap::new(),
            orphaned_event_markers: Vec::new(),
            source: gt_types::FileSource::GtdPath(std::path::PathBuf::new()),
            load_warnings: Vec::new(),
        }
    }

    #[test]
    fn rename_identity_repoints_matching_loaded_recordings() {
        let mut files = LoadedFiles::new();
        files.push(
            empty_file(),
            FileHistory::recording(
                "auto:old".to_owned(),
                meta(),
                Some(DatabaseRef {
                    identity: "auto:old".to_owned(),
                    group_name: "rec0".to_owned(),
                }),
            ),
        );
        // A different identity, left untouched.
        files.push(
            empty_file(),
            FileHistory::recording("other".to_owned(), meta(), None),
        );

        files.rename_identity("auto:old", "Trip");

        let renamed = files.view().entry_for(gt_types::FileIdx::new(0)).unwrap();
        assert_eq!(renamed.identity(), Some("Trip"));
        assert_eq!(
            renamed.history().db_ref().map(|r| r.identity.as_str()),
            Some("Trip"),
            "the db_ref identity is patched too, group_name unchanged"
        );

        let untouched = files.view().entry_for(gt_types::FileIdx::new(1)).unwrap();
        assert_eq!(untouched.identity(), Some("other"));
    }
}
