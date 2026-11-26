use std::collections::{BTreeMap, BTreeSet};
use std::mem;

use gt_history_types::{DatabaseRef, StoredLayoutRevision, StoredTrackSnapshotUpdate, TrackState};
use gt_loaded_files::{LoadedFileEntry, LoadedFileId, LoadedFilesView};
use gt_types::{
    DataCategory, DataCategorySet, FileIdx, GeneratedMarkerKindTag, Generation, LoadedTrack,
    TrackIdx, TrackRef, Versioned,
};
use gt_ui_types::{
    EventMarkerVisibility, FileVisibility, GeneratedMarkerVisibility, TrackDataVisibility,
    TrackVisibility,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    Mixed,
    Off,
    On,
}

impl CheckState {
    /// The state a click puts the node in: a partly checked node turns fully
    /// on.
    pub fn toggled(self) -> Self {
        match self {
            Self::On => Self::Off,
            Self::Off | Self::Mixed => Self::On,
        }
    }

    fn apply_to_hidden_tracks(self, number: usize, hidden: &mut BTreeSet<usize>) -> bool {
        if self == Self::Off {
            hidden.insert(number)
        } else {
            hidden.remove(&number)
        }
    }
}

/// A tree node that can be selected (for shift/ctrl-click).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NodeKey {
    File(FileIdx),
    Track(TrackRef),
}

impl NodeKey {
    pub fn file(self) -> FileIdx {
        match self {
            Self::File(fi) => fi,
            Self::Track(track_ref) => track_ref.fi,
        }
    }
}

/// What a track's tree state is kept under while the tree is rebuilt: the
/// session id of the recording the track belongs to, and
/// [`gt_types::track::TrackMetadata::index`]. Removing a file or a track
/// shifts the positions [`NodeKey`] addresses a row by, and leaves these two
/// alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct TrackStateKey {
    file: LoadedFileId,
    track_number: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum NodeStateKey {
    File(LoadedFileId),
    Track(TrackStateKey),
}

/// The items of an open shelve confirmation, and the state of its
/// permanent-delete tickbox.
pub struct ShelveConfirmState {
    pub items: Vec<NodeKey>,
    pub delete_permanently: bool,
}

/// Per-track tree of event variant paths with tri-state visibility.
///
/// Stores all unique prefix segments derived from the track's marker
/// variant paths.  Leaves are paths that exist as actual marker variants.
/// Internal nodes are shared prefix segments.  `CheckState` is derived for
/// internal nodes from their children. Only leaves carry "true" state.
#[derive(Default)]
pub struct EventPathTree {
    /// All paths (leaves and internal nodes) → visibility state.
    pub nodes: BTreeMap<String, CheckState>,
}

impl EventPathTree {
    /// Sync the tree from the current set of marker variant paths.
    ///
    /// New nodes are added as `On`. Existing nodes keep their state. Nodes that
    /// no longer appear are removed.
    pub fn sync_from_paths<'a>(&mut self, paths: impl Iterator<Item = &'a str>) {
        let mut all_prefixes: BTreeSet<String> = BTreeSet::new();
        for path in paths {
            let segs: Vec<&str> = path.split('/').collect();
            for depth in 1..=segs.len() {
                if let Some(slice) = segs.get(..depth) {
                    all_prefixes.insert(slice.join("/"));
                }
            }
        }
        for prefix in &all_prefixes {
            self.nodes.entry(prefix.clone()).or_insert(CheckState::On);
        }
        self.nodes.retain(|p, _| all_prefixes.contains(p));
    }

    /// Toggle a path: On → Off, Off/Mixed → On.
    /// Cascades to all descendants, then recomputes all ancestors.
    pub fn toggle(&mut self, path: &str) {
        let current = self.nodes.get(path).copied().unwrap_or(CheckState::On);
        set_subtree(&mut self.nodes, path, current.toggled());
        recompute_ancestors(&mut self.nodes, path);
    }

    /// Aggregate visibility of the entire tree (used for the Events header checkbox).
    pub fn aggregate(&self) -> CheckState {
        if self.nodes.is_empty() {
            return CheckState::On;
        }
        let has_roots = self.nodes.keys().any(|k| !k.contains('/'));
        let iter = self
            .nodes
            .iter()
            .filter(move |(k, _)| !has_roots || !k.contains('/'))
            .map(|(_, &v)| v);
        aggregate_check_states(iter).unwrap_or(CheckState::On)
    }

    /// Iterator over hidden-root paths - paths that are Off but whose parent
    /// is NOT Off.  This gives the minimal representation for
    /// `EventMarkerVisibility`.
    pub fn hidden_roots(&self) -> impl Iterator<Item = &str> {
        self.nodes.iter().filter_map(|(path, &check)| {
            if check != CheckState::Off {
                return None;
            }
            let parent_off = path.rfind('/').is_some_and(|pos| {
                path.get(..pos).is_some_and(|parent| {
                    self.nodes
                        .get(parent)
                        .is_some_and(|&c| c == CheckState::Off)
                })
            });
            if parent_off {
                None
            } else {
                Some(path.as_str())
            }
        })
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum TrackCheckEdit {
    Recorded,
    Unchanged,
    Unrecorded,
}

pub struct TrackNode {
    pub expanded: bool,
    pub check: CheckState,
    check_edit: TrackCheckEdit,
    /// Which of the track's element-category sections are expanded in the
    /// tree. `Track` has no sub-items, so it never appears here.
    pub categories_expanded: DataCategorySet,
    pub track_visible: bool,
    pub tpv_visible: bool,
    pub satellites_visible: bool,
    pub custom_markers_visible: bool,
    pub generated_markers_visible: bool,
    /// Generated-marker event types whose group is expanded in the tree.
    pub generated_kinds_expanded: BTreeSet<GeneratedMarkerKindTag>,
    /// Generated-marker event types hidden on the map (a refinement under the
    /// category-level `generated_markers_visible` master toggle).
    pub generated_kinds_hidden: BTreeSet<GeneratedMarkerKindTag>,
    pub event_paths: EventPathTree,
    /// Per-track text filter for the Events section search box.
    pub event_filter: String,
    /// Whether the read-only Channels section is expanded. Channels have no map
    /// visibility yet, so they get a dedicated flag.
    pub channels_expanded: bool,
    /// [`gt_types::track::TrackMetadata::index`] of the track this node is
    /// built from, one half of the [`TrackStateKey`] its state moves by.
    track_number: usize,
}

impl TrackNode {
    fn new(track_number: usize) -> Self {
        Self {
            track_number,
            expanded: false,
            check: CheckState::On,
            check_edit: TrackCheckEdit::Unchanged,
            categories_expanded: DataCategorySet::default(),
            track_visible: true,
            tpv_visible: true,
            satellites_visible: true,
            custom_markers_visible: true,
            generated_markers_visible: true,
            generated_kinds_expanded: BTreeSet::new(),
            generated_kinds_hidden: BTreeSet::new(),
            event_paths: EventPathTree::default(),
            event_filter: String::new(),
            channels_expanded: false,
        }
    }

    fn edit_check(&mut self, check: CheckState) {
        self.check = check;
        self.check_edit = TrackCheckEdit::Unrecorded;
    }

    fn sync_event_paths_from(&mut self, loaded_track: &LoadedTrack) {
        self.event_paths.sync_from_paths(
            loaded_track
                .event_markers
                .iter()
                .map(|marker| marker.variant_path.as_str()),
        );
    }
}

pub struct FileNode {
    pub expanded: bool,
    pub check: CheckState,
    pub tracks: Vec<TrackNode>,
    /// Session id of the recording this node is built from. A rebuild moves
    /// the node's state, and the state of every track under it, by this id.
    id: LoadedFileId,
    visibility_layout: Option<StoredLayoutRevision>,
}

impl FileNode {
    fn from_loaded(entry: LoadedFileEntry<'_>) -> Self {
        Self {
            id: entry.id(),
            visibility_layout: entry
                .history()
                .track_snapshot()
                .map(|observation| observation.layout().clone()),
            expanded: false,
            check: CheckState::On,
            tracks: Vec::new(),
        }
    }

    fn current_visibility_layout(&self) -> Option<&StoredLayoutRevision> {
        self.visibility_layout
            .as_ref()
            .filter(|layout| layout.is_current())
    }

    fn recompute_check(&mut self) {
        self.check =
            aggregate_check_states(self.tracks.iter().map(|t| t.check)).unwrap_or(CheckState::On);
    }

    fn apply_stored_hidden_tracks(&mut self, track_numbers: &BTreeSet<usize>) {
        for track in &mut self.tracks {
            if track.check_edit == TrackCheckEdit::Unchanged {
                track.check = if track_numbers.contains(&track.track_number) {
                    CheckState::Off
                } else {
                    CheckState::On
                };
            }
        }
        self.recompute_check();
    }
}

struct StoredHiddenTracks {
    layout: StoredLayoutRevision,
    hidden: Option<BTreeSet<usize>>,
    cleared_at: Option<u64>,
    // Retain edits after writes to reconcile both addresses of a checked rename.
    edits: BTreeMap<usize, StoredTrackCheckEdit>,
}

#[derive(Clone, Copy)]
struct StoredTrackCheckEdit {
    check: CheckState,
    sequence: u64,
}

impl StoredHiddenTracks {
    fn awaiting_read(layout: &StoredLayoutRevision) -> Self {
        Self {
            layout: layout.clone(),
            hidden: None,
            cleared_at: None,
            edits: BTreeMap::new(),
        }
    }

    /// The tree initializes checkboxes from an exact cached layout even after that layout expires.
    fn numbers_for_layout(&self, layout: &StoredLayoutRevision) -> Option<&BTreeSet<usize>> {
        self.hidden
            .as_ref()
            .filter(|_| self.layout.same_observation(layout))
    }

    fn adopt_layout_update(&mut self, update: &StoredTrackSnapshotUpdate) {
        self.layout = update.after().layout().clone();
        if self.hidden.is_none() {
            self.edits.retain(|&number, _| {
                number
                    .checked_sub(1)
                    .and_then(|index| update.after().rows().get(index))
                    .is_some_and(|row| row.state != TrackState::Deleted)
            });
        }
    }

    fn reconcile_successor(&mut self, successor: Self) {
        self.cleared_at = self.cleared_at.max(successor.cleared_at);
        for (number, edit) in successor.edits {
            self.edits
                .entry(number)
                .and_modify(|earlier| {
                    if edit.sequence > earlier.sequence {
                        *earlier = edit;
                    }
                })
                .or_insert(edit);
        }
        let baseline = if let Some(cleared_at) = self.cleared_at {
            self.edits.retain(|_, edit| edit.sequence > cleared_at);
            Some(BTreeSet::new())
        } else {
            successor.hidden.or_else(|| self.hidden.take())
        };
        if let Some(mut hidden) = baseline {
            self.apply_edits(&mut hidden);
            self.hidden = Some(hidden);
        }
    }

    fn clear_visibility(&mut self, sequence: u64) -> bool {
        let changed = self
            .hidden
            .replace(BTreeSet::new())
            .is_none_or(|hidden| !hidden.is_empty());
        self.edits.clear();
        self.cleared_at = Some(sequence);
        changed
    }

    fn has_local_change(&self) -> bool {
        self.cleared_at.is_some() || !self.edits.is_empty()
    }

    fn apply_edits(&self, hidden: &mut BTreeSet<usize>) -> bool {
        let mut changed = false;
        for (&number, edit) in &self.edits {
            changed |= edit.check.apply_to_hidden_tracks(number, hidden);
        }
        changed
    }

    fn record_edit(&mut self, number: usize, edit: StoredTrackCheckEdit) -> bool {
        self.edits.insert(number, edit);
        self.hidden
            .as_mut()
            .is_some_and(|hidden| edit.check.apply_to_hidden_tracks(number, hidden))
    }
}

/// The tracks of one recording that are toggled on, as the Visible section
/// lists them.
#[derive(Debug)]
pub struct VisibleTracksInFile {
    pub file: FileIdx,
    pub tracks: Vec<TrackRef>,
}

pub struct TreeState {
    pub files: Vec<FileNode>,
    pub selection: BTreeSet<NodeKey>,
    pub selection_anchor: Option<NodeKey>,
    pub shelve_confirm: Option<ShelveConfirmState>,
    /// Items the user asked to unload from the view (non-destructive, the
    /// recordings stay in history). Consumed by the app each frame.
    pub pending_unload: Option<Vec<NodeKey>>,
    pub detached: bool,
    /// The tree row to scroll into view, set by a click in the Visible section
    /// and cleared once the tree has rendered.
    pub reveal_request: Option<NodeKey>,
    visible_section_fraction: f32,
    /// Derived from tree state, kept in sync.  Passed to gt-map renderers.
    visibility: Versioned<TrackDataVisibility>,
    /// Derived from tree state, kept in sync.  Passed to gt-map renderers.
    event_marker_visibility: EventMarkerVisibility,
    /// Derived from tree state, kept in sync.  Passed to gt-map renderers.
    generated_marker_visibility: GeneratedMarkerVisibility,
    /// Saved visibility and checkbox edits by stored track number and layout.
    hidden_tracks_by_recording: BTreeMap<DatabaseRef, StoredHiddenTracks>,
    next_track_check_edit: u64,
    /// The recordings whose hidden tracks differ from what the history
    /// database holds, until [`TreeState::take_observed_hidden_tracks_to_store`] takes
    /// them.
    recordings_with_hidden_tracks_to_store: BTreeSet<DatabaseRef>,
}

impl Default for TreeState {
    fn default() -> Self {
        Self::new()
    }
}

impl TreeState {
    pub fn new() -> Self {
        Self {
            files: Vec::new(),
            selection: BTreeSet::new(),
            selection_anchor: None,
            shelve_confirm: None,
            pending_unload: None,
            detached: false,
            reveal_request: None,
            visible_section_fraction: crate::VISIBLE_SECTION_DEFAULT_FRACTION,
            visibility: Versioned::new(TrackDataVisibility { files: Vec::new() }),
            event_marker_visibility: EventMarkerVisibility::new(),
            generated_marker_visibility: GeneratedMarkerVisibility::new(),
            hidden_tracks_by_recording: BTreeMap::new(),
            next_track_check_edit: 0,
            recordings_with_hidden_tracks_to_store: BTreeSet::new(),
        }
    }

    /// The numbers of the hidden tracks of `db_ref`, and [`None`] for a
    /// recording whose stored UI state this session has not read.
    #[cfg(any(test, feature = "test-util"))]
    pub fn hidden_track_numbers(&self, db_ref: &DatabaseRef) -> Option<&BTreeSet<usize>> {
        let stored = self.hidden_tracks_by_recording.get(db_ref)?;
        stored.numbers_for_layout(&stored.layout)
    }

    pub fn cancel_shelve_confirmation_for_file(&mut self, file: FileIdx) {
        if self
            .shelve_confirm
            .as_ref()
            .is_some_and(|confirm| confirm.items.iter().any(|item| item.file() == file))
        {
            self.shelve_confirm = None;
        }
    }

    pub fn hidden_track_numbers_for_layout(
        &self,
        layout: &StoredLayoutRevision,
    ) -> Option<&BTreeSet<usize>> {
        self.hidden_tracks_by_recording
            .get(layout.recording())?
            .numbers_for_layout(layout)
    }

    pub fn apply_stored_hidden_tracks(
        &mut self,
        layout: StoredLayoutRevision,
        mut track_numbers: BTreeSet<usize>,
    ) {
        if !layout.is_current() {
            return;
        }
        let recording = layout.recording().clone();
        if self
            .hidden_tracks_by_recording
            .get(&recording)
            .is_some_and(|stored| {
                stored.layout.same_observation(&layout)
                    && stored.hidden.is_some()
                    && stored.has_local_change()
            })
        {
            return;
        }
        let edits = self
            .hidden_tracks_by_recording
            .remove(&recording)
            .filter(|stored| stored.layout.same_observation(&layout))
            .map(|stored| stored.edits)
            .unwrap_or_default();
        let mut stored = StoredHiddenTracks {
            layout,
            hidden: None,
            cleared_at: None,
            edits,
        };
        let changed = stored.apply_edits(&mut track_numbers);
        for file_node in &mut self.files {
            if !file_node
                .visibility_layout
                .as_ref()
                .is_some_and(|source| source.same_observation(&stored.layout))
            {
                continue;
            }
            file_node.apply_stored_hidden_tracks(&track_numbers);
        }
        if changed {
            self.recordings_with_hidden_tracks_to_store
                .insert(recording.clone());
        } else {
            self.recordings_with_hidden_tracks_to_store
                .remove(&recording);
        }
        stored.hidden = Some(track_numbers);
        self.hidden_tracks_by_recording.insert(recording, stored);
        self.rebuild_visibility();
    }

    pub fn take_observed_hidden_tracks_to_store(
        &mut self,
    ) -> Vec<(StoredLayoutRevision, BTreeSet<usize>)> {
        mem::take(&mut self.recordings_with_hidden_tracks_to_store)
            .into_iter()
            .filter_map(|recording| {
                let stored = self.hidden_tracks_by_recording.get(&recording)?;
                if !stored.layout.is_current() {
                    return None;
                }
                let hidden = stored.numbers_for_layout(&stored.layout)?;
                Some((stored.layout.clone(), hidden.clone()))
            })
            .collect()
    }

    /// The Visible section's share of the region it divides with the tree.
    pub fn visible_section_fraction(&self) -> f32 {
        self.visible_section_fraction
    }

    /// Keeps the previous share when `fraction` is outside `0.0..=1.0` or is
    /// not a number.
    pub fn set_visible_section_fraction(&mut self, fraction: f32) {
        if (0.0..=1.0).contains(&fraction) {
            self.visible_section_fraction = fraction;
        }
    }

    /// The tree preserves expansion, event paths and marker toggles by loaded file ID and track number.
    /// Tracks on the same layout keep their local checkbox edits.
    /// Tracks without local edits use the cached visibility for their layout.
    /// New rows start collapsed.
    ///
    /// Each [`NodeKey`] the state holds - the selection, its anchor, the
    /// reveal request, the items of an open shelve confirmation and those of a
    /// pending unload - moves to the position of the row it refers to, and is
    /// dropped once that row is gone.
    pub fn sync_from_loaded_files(&mut self, files: LoadedFilesView<'_>) {
        let key_at_previous_position: BTreeMap<NodeKey, NodeStateKey> =
            self.node_state_keys_by_position().into_iter().collect();

        let mut previous_files = BTreeMap::new();
        let mut previous_tracks: BTreeMap<TrackStateKey, TrackNode> = BTreeMap::new();
        for FileNode {
            id,
            expanded,
            tracks,
            visibility_layout,
            ..
        } in mem::take(&mut self.files)
        {
            previous_files.insert(id, (expanded, visibility_layout));
            for track_node in tracks {
                let key = TrackStateKey {
                    file: id,
                    track_number: track_node.track_number,
                };
                previous_tracks.insert(key, track_node);
            }
        }

        let hidden_tracks_by_recording = &self.hidden_tracks_by_recording;
        self.files = files
            .entries()
            .map(|entry| {
                let mut file_node = FileNode::from_loaded(entry);
                let (expanded, previous_layout) =
                    previous_files.remove(&entry.id()).unwrap_or_default();
                file_node.expanded = expanded;
                let layout = file_node.visibility_layout.as_ref();
                let reset_checks = layout.is_some_and(|layout| {
                    previous_layout
                        .as_ref()
                        .is_none_or(|previous| !previous.same_observation(layout))
                });
                let hidden_tracks = layout.and_then(|layout| {
                    hidden_tracks_by_recording
                        .get(layout.recording())?
                        .numbers_for_layout(layout)
                });
                for loaded_track in &entry.file().tracks {
                    let track_number = loaded_track.metadata.index;
                    let key = TrackStateKey {
                        file: entry.id(),
                        track_number,
                    };
                    let previous = previous_tracks.remove(&key);
                    let new_track = previous.is_none();
                    let mut track_node = previous.unwrap_or_else(|| TrackNode::new(track_number));
                    if reset_checks || new_track {
                        track_node.check_edit = TrackCheckEdit::Unchanged;
                        track_node.check = CheckState::On;
                    }
                    track_node.sync_event_paths_from(loaded_track);
                    file_node.tracks.push(track_node);
                }
                if let Some(hidden_tracks) = hidden_tracks {
                    file_node.apply_stored_hidden_tracks(hidden_tracks);
                } else {
                    file_node.recompute_check();
                }
                file_node
            })
            .collect();

        self.move_held_positions(&key_at_previous_position);
        self.rebuild_visibility();
        self.rebuild_event_marker_visibility();
        self.rebuild_generated_marker_visibility();
    }

    pub fn sync_after_track_snapshot_updates(
        &mut self,
        files: LoadedFilesView<'_>,
        updates: &[StoredTrackSnapshotUpdate],
        adopted: &[(LoadedFileId, &StoredTrackSnapshotUpdate)],
    ) -> Vec<StoredLayoutRevision> {
        let mut updated_visibility = false;
        for update in updates {
            let before = update.before().layout();
            let after = update.after().layout();
            if self
                .hidden_tracks_by_recording
                .get(before.recording())
                .is_none_or(|stored| !stored.layout.same_observation(before))
                || self
                    .hidden_tracks_by_recording
                    .get(after.recording())
                    .is_some_and(|stored| {
                        stored.layout.is_current() && !stored.layout.same_observation(after)
                    })
            {
                continue;
            }
            let Some(mut stored) = self.hidden_tracks_by_recording.remove(before.recording())
            else {
                continue;
            };
            stored.adopt_layout_update(update);
            self.recordings_with_hidden_tracks_to_store
                .remove(before.recording());
            let after_recording = after.recording();
            if let Some(successor) = self.hidden_tracks_by_recording.remove(after_recording)
                && successor.layout.same_observation(after)
            {
                stored.reconcile_successor(successor);
            }
            if stored.hidden.is_some() && stored.has_local_change() {
                self.recordings_with_hidden_tracks_to_store
                    .insert(after_recording.clone());
            } else {
                self.recordings_with_hidden_tracks_to_store
                    .remove(after_recording);
            }
            self.hidden_tracks_by_recording
                .insert(after_recording.clone(), stored);
            updated_visibility = true;
        }
        for &(id, update) in adopted {
            let before = update.before().layout();
            let after = update.after().layout();
            if let Ok(index) = self.files.binary_search_by_key(&id, |node| node.id)
                && let Some(node) = self.files.get_mut(index)
                && node
                    .visibility_layout
                    .as_ref()
                    .is_some_and(|layout| layout.same_observation(before))
            {
                node.visibility_layout = Some(after.clone());
            }
        }
        if updated_visibility || !adopted.is_empty() {
            self.sync_from_loaded_files(files);
        }
        let adopted_layouts = adopted.iter().map(|(_, update)| update.after().layout());
        let pending_reads = updates.iter().filter_map(|update| {
            let layout = update.after().layout();
            let stored = self.hidden_tracks_by_recording.get(layout.recording())?;
            if stored.layout.same_observation(layout)
                && stored.hidden.is_none()
                && !stored.edits.is_empty()
            {
                return Some(layout);
            }
            None
        });
        adopted_layouts
            .chain(pending_reads)
            .filter(|layout| {
                layout.is_current() && self.hidden_track_numbers_for_layout(layout).is_none()
            })
            .map(|layout| (layout.recording(), layout))
            .collect::<BTreeMap<_, _>>()
            .into_values()
            .cloned()
            .collect()
    }

    pub fn sync_after_recording_load(
        &mut self,
        files: LoadedFilesView<'_>,
        reset_visibility: Option<StoredLayoutRevision>,
    ) {
        if let Some(observation) = reset_visibility {
            let recording = observation.recording().clone();
            self.recordings_with_hidden_tracks_to_store
                .remove(&recording);
            self.hidden_tracks_by_recording.insert(
                recording,
                StoredHiddenTracks {
                    layout: observation,
                    hidden: Some(BTreeSet::new()),
                    cleared_at: None,
                    edits: BTreeMap::new(),
                },
            );
        }
        self.sync_from_loaded_files(files);
    }

    /// Rebuild every node at its defaults: checked on, collapsed, and nothing
    /// selected.
    ///
    /// The caller re-segmented the recordings, which cuts their fixes into
    /// different tracks and numbers them over the new boundaries: a track
    /// number from the previous segmentation addresses another stretch of the
    /// recording.
    pub fn reset_for_resegmented_files(&mut self, files: LoadedFilesView<'_>) {
        self.clear_saved_track_visibility();
        self.files = files
            .entries()
            .map(|entry| {
                let mut file_node = FileNode::from_loaded(entry);
                for loaded_track in &entry.file().tracks {
                    let mut track_node = TrackNode::new(loaded_track.metadata.index);
                    track_node.sync_event_paths_from(loaded_track);
                    file_node.tracks.push(track_node);
                }
                file_node
            })
            .collect();
        self.selection.clear();
        self.selection_anchor = None;
        self.shelve_confirm = None;
        self.rebuild_visibility();
        self.rebuild_event_marker_visibility();
        self.rebuild_generated_marker_visibility();
    }

    fn record_hidden_tracks_of_the_loaded_recordings(&mut self) {
        for file_node in &mut self.files {
            let Some(layout) = file_node.visibility_layout.as_ref() else {
                continue;
            };
            let mut edited_tracks = file_node
                .tracks
                .iter_mut()
                .filter(|track| track.check_edit == TrackCheckEdit::Unrecorded)
                .peekable();
            if edited_tracks.peek().is_none() {
                continue;
            }
            let recording = layout.recording();
            let stored = self
                .hidden_tracks_by_recording
                .entry(recording.clone())
                .or_insert_with(|| StoredHiddenTracks::awaiting_read(layout));
            if !stored.layout.same_observation(layout) {
                if !layout.is_current() || stored.layout.is_current() {
                    continue;
                }
                *stored = StoredHiddenTracks::awaiting_read(layout);
            }

            let mut changed = false;
            for track in edited_tracks {
                changed |= stored.record_edit(
                    track.track_number,
                    StoredTrackCheckEdit {
                        check: track.check,
                        sequence: self.next_track_check_edit,
                    },
                );
                self.next_track_check_edit += 1;
                track.check_edit = TrackCheckEdit::Recorded;
            }
            if changed {
                self.recordings_with_hidden_tracks_to_store
                    .insert(recording.clone());
            }
        }
    }

    fn clear_saved_track_visibility(&mut self) {
        for file_node in &self.files {
            let Some(layout) = file_node.current_visibility_layout() else {
                continue;
            };
            let recording = layout.recording();
            let stored = self
                .hidden_tracks_by_recording
                .entry(recording.clone())
                .or_insert_with(|| StoredHiddenTracks::awaiting_read(layout));
            if !stored.layout.same_observation(layout) {
                if stored.layout.is_current() {
                    continue;
                }
                *stored = StoredHiddenTracks::awaiting_read(layout);
            }
            if stored.clear_visibility(self.next_track_check_edit) {
                self.recordings_with_hidden_tracks_to_store
                    .insert(recording.clone());
            }
            self.next_track_check_edit += 1;
        }
    }

    /// Every row of the tree, the position it sits at paired with the key its
    /// state is kept under.
    fn node_state_keys_by_position(&self) -> Vec<(NodeKey, NodeStateKey)> {
        let mut keys = Vec::new();
        for (fi, file_node) in self.files.iter().enumerate() {
            let fi = FileIdx::new(fi);
            keys.push((NodeKey::File(fi), NodeStateKey::File(file_node.id)));
            for (ti, track_node) in file_node.tracks.iter().enumerate() {
                let key = TrackStateKey {
                    file: file_node.id,
                    track_number: track_node.track_number,
                };
                keys.push((
                    NodeKey::Track(TrackRef::new(fi, TrackIdx::new(ti))),
                    NodeStateKey::Track(key),
                ));
            }
        }
        keys
    }

    /// Move every [`NodeKey`] the state holds to the position its row sits at
    /// now, given the key each position stood for before the rebuild.
    fn move_held_positions(&mut self, key_at_previous_position: &BTreeMap<NodeKey, NodeStateKey>) {
        let position_of_key: BTreeMap<NodeStateKey, NodeKey> = self
            .node_state_keys_by_position()
            .into_iter()
            .map(|(position, key)| (key, position))
            .collect();
        let moved = |position: NodeKey| -> Option<NodeKey> {
            position_of_key
                .get(key_at_previous_position.get(&position)?)
                .copied()
        };

        self.selection = mem::take(&mut self.selection)
            .into_iter()
            .filter_map(moved)
            .collect();
        self.selection_anchor = self.selection_anchor.and_then(moved);
        self.reveal_request = self.reveal_request.and_then(moved);
        if let Some(confirm) = &mut self.shelve_confirm {
            keep_moved_positions(&mut confirm.items, moved);
        }
        if self
            .shelve_confirm
            .as_ref()
            .is_some_and(|confirm| confirm.items.is_empty())
        {
            self.shelve_confirm = None;
        }
        if let Some(pending) = &mut self.pending_unload {
            keep_moved_positions(pending, moved);
        }
    }

    /// Set a file's check state with cascade to all child tracks.
    fn set_file_check(&mut self, fi: FileIdx, check: CheckState) {
        let Some(file_node) = self.file_node_mut(fi) else {
            return;
        };
        file_node.check = check;
        for track in &mut file_node.tracks {
            track.edit_check(check);
        }
        self.rebuild_visibility();
    }

    /// Set one track's check state and recompute the parent file's aggregate.
    fn set_track_check(&mut self, track: TrackRef, check: CheckState) {
        let Some(file_node) = self.file_node_mut(track.fi) else {
            return;
        };
        let Some(track_node) = track.index.get_mut(&mut file_node.tracks) else {
            return;
        };
        track_node.edit_check(check);
        file_node.recompute_check();
        self.rebuild_visibility();
    }

    /// Toggle a file's check state with cascade to all child tracks.
    pub fn toggle_file_check(&mut self, fi: FileIdx) {
        let Some(check) = self.file_node(fi).map(|file_node| file_node.check) else {
            return;
        };
        self.set_file_check(fi, check.toggled());
    }

    /// Hide a recording and every track under it.
    pub fn hide_file(&mut self, fi: FileIdx) {
        self.set_file_check(fi, CheckState::Off);
    }

    /// Hide one track and recompute the parent file's aggregate.
    pub fn hide_track(&mut self, track: TrackRef) {
        self.set_track_check(track, CheckState::Off);
    }

    /// The tracks whose check is `On`, grouped by their recording, in tree
    /// order. A recording whose check is `Off`, and one with no such track, is
    /// left out.
    pub fn visible_tracks_by_file(&self) -> Vec<VisibleTracksInFile> {
        self.files
            .iter()
            .enumerate()
            .filter(|(_, file_node)| file_node.check != CheckState::Off)
            .filter_map(|(fi, file_node)| {
                let file = FileIdx::new(fi);
                let tracks: Vec<TrackRef> = file_node
                    .tracks
                    .iter()
                    .enumerate()
                    .filter(|(_, track_node)| track_node.check == CheckState::On)
                    .map(|(ti, _)| TrackRef::new(file, TrackIdx::new(ti)))
                    .collect();
                (!tracks.is_empty()).then_some(VisibleTracksInFile { file, tracks })
            })
            .collect()
    }

    /// Toggle a track's check state and recompute the parent file's aggregate.
    pub fn toggle_track_check(&mut self, track: TrackRef) {
        let Some(check) = self.track_node(track).map(|track_node| track_node.check) else {
            return;
        };
        self.set_track_check(track, check.toggled());
    }

    /// Toggle a single event path node and recompute ancestors.
    pub fn toggle_event_path(&mut self, track: TrackRef, path: &str) {
        let Some(track_node) = self.track_node_mut(track) else {
            return;
        };
        track_node.event_paths.toggle(path);
        self.rebuild_event_marker_visibility_for(track);
        self.rebuild_visibility_for_track(track);
    }

    /// Expand or collapse one generated-marker event-type group in the tree.
    pub fn toggle_generated_kind_expanded(&mut self, track: TrackRef, tag: GeneratedMarkerKindTag) {
        let Some(track_node) = self.track_node_mut(track) else {
            return;
        };
        if !track_node.generated_kinds_expanded.remove(&tag) {
            track_node.generated_kinds_expanded.insert(tag);
        }
    }

    /// Show or hide one generated-marker event type on the map.
    pub fn toggle_generated_kind_hidden(&mut self, track: TrackRef, tag: GeneratedMarkerKindTag) {
        let Some(track_node) = self.track_node_mut(track) else {
            return;
        };
        if !track_node.generated_kinds_hidden.remove(&tag) {
            track_node.generated_kinds_hidden.insert(tag);
        }
        self.rebuild_generated_marker_visibility_for(track);
    }

    /// Whether markers of `tag` are currently shown for `track`.
    pub fn generated_kind_visible(&self, track: TrackRef, tag: GeneratedMarkerKindTag) -> bool {
        self.track_node(track)
            .is_none_or(|t| !t.generated_kinds_hidden.contains(&tag))
    }

    /// Whether the generated-marker event-type group is expanded for `track`.
    pub fn generated_kind_expanded(&self, track: TrackRef, tag: GeneratedMarkerKindTag) -> bool {
        self.track_node(track)
            .is_some_and(|t| t.generated_kinds_expanded.contains(&tag))
    }

    /// Toggle all event paths for a track (the "Events" header checkbox).
    pub fn toggle_all_event_paths(&mut self, track: TrackRef) {
        let agg = self.track_node(track).map(|t| t.event_paths.aggregate());
        let Some(agg) = agg else { return };
        self.set_all_event_paths(track, agg.toggled());
    }

    fn set_all_event_paths(&mut self, track: TrackRef, state: CheckState) {
        let Some(track_node) = self.track_node_mut(track) else {
            return;
        };
        for check in track_node.event_paths.nodes.values_mut() {
            *check = state;
        }
        self.rebuild_event_marker_visibility_for(track);
        self.rebuild_visibility_for_track(track);
    }

    pub fn set_category_visible(&mut self, track: TrackRef, cat: DataCategory, visible: bool) {
        let Some(track_node) = self.track_node_mut(track) else {
            return;
        };
        match cat {
            DataCategory::Track => track_node.track_visible = visible,
            DataCategory::Tpv => track_node.tpv_visible = visible,
            DataCategory::SatelliteReport => track_node.satellites_visible = visible,
            DataCategory::CustomMarker => track_node.custom_markers_visible = visible,
            DataCategory::GeneratedMarker => track_node.generated_markers_visible = visible,
            DataCategory::EventMarker => {
                let state = if visible {
                    CheckState::On
                } else {
                    CheckState::Off
                };
                for check in track_node.event_paths.nodes.values_mut() {
                    *check = state;
                }
                self.rebuild_event_marker_visibility_for(track);
            }
        }
        self.rebuild_visibility_for_track(track);
    }

    pub fn toggle_expand_file(&mut self, fi: FileIdx) {
        if let Some(file_node) = self.file_node_mut(fi) {
            file_node.expanded = !file_node.expanded;
        }
    }

    /// Expand, select and scroll to the row for `key`, which renders further
    /// down this frame.
    pub fn reveal(&mut self, key: NodeKey) {
        self.expand_file(key.file());
        self.apply_click(key, false, false);
        self.reveal_request = Some(key);
    }

    pub fn expand_file(&mut self, fi: FileIdx) {
        if let Some(file_node) = self.file_node_mut(fi) {
            file_node.expanded = true;
        }
    }

    pub fn toggle_expand_track(&mut self, track: TrackRef) {
        if let Some(track_node) = self.track_node_mut(track) {
            track_node.expanded = !track_node.expanded;
        }
    }

    pub fn toggle_category_expanded(&mut self, track: TrackRef, cat: DataCategory) {
        if let Some(track_node) = self.track_node_mut(track) {
            let expanded = track_node.categories_expanded.contains(cat);
            track_node.categories_expanded.set(cat, !expanded);
        }
    }

    pub fn toggle_channels_expanded(&mut self, track: TrackRef) {
        if let Some(track_node) = self.track_node_mut(track) {
            track_node.channels_expanded = !track_node.channels_expanded;
        }
    }

    pub fn set_all_enabled(&mut self, enabled: bool) {
        let state = if enabled {
            CheckState::On
        } else {
            CheckState::Off
        };
        for file_node in &mut self.files {
            file_node.check = state;
            for track_node in &mut file_node.tracks {
                track_node.edit_check(state);
            }
        }
        self.rebuild_visibility();
    }

    pub fn show_only_file(&mut self, fi: FileIdx) {
        for (i, file_node) in self.files.iter_mut().enumerate() {
            if FileIdx::new(i) == fi {
                file_node.check = CheckState::On;
                for track_node in &mut file_node.tracks {
                    track_node.edit_check(CheckState::On);
                }
            } else {
                file_node.check = CheckState::Off;
                for track_node in &mut file_node.tracks {
                    track_node.edit_check(CheckState::Off);
                }
            }
        }
        self.rebuild_visibility();
    }

    /// Show only the tracks in `tracks`, hiding everything else. Tracks from
    /// files not mentioned in `tracks` are hidden. Within a mentioned file, only
    /// the listed tracks are shown.
    pub fn show_only_tracks(&mut self, tracks: &[TrackRef]) {
        for (i, file_node) in self.files.iter_mut().enumerate() {
            let fi = FileIdx::new(i);
            let any_shown = tracks.iter().any(|t| t.fi == fi);
            if !any_shown {
                file_node.check = CheckState::Off;
                for track_node in &mut file_node.tracks {
                    track_node.edit_check(CheckState::Off);
                }
            } else {
                for (j, track_node) in file_node.tracks.iter_mut().enumerate() {
                    let ti = TrackIdx::new(j);
                    track_node.edit_check(if tracks.iter().any(|t| t.fi == fi && t.index == ti) {
                        CheckState::On
                    } else {
                        CheckState::Off
                    });
                }
                file_node.recompute_check();
            }
        }
        self.rebuild_visibility();
    }

    pub fn show_only_track(&mut self, track: TrackRef) {
        for (i, file_node) in self.files.iter_mut().enumerate() {
            if FileIdx::new(i) == track.fi {
                for (j, track_node) in file_node.tracks.iter_mut().enumerate() {
                    track_node.edit_check(if TrackIdx::new(j) == track.index {
                        CheckState::On
                    } else {
                        CheckState::Off
                    });
                }
                file_node.recompute_check();
            } else {
                file_node.check = CheckState::Off;
                for track_node in &mut file_node.tracks {
                    track_node.edit_check(CheckState::Off);
                }
            }
        }
        self.rebuild_visibility();
    }

    pub fn apply_click(&mut self, key: NodeKey, ctrl: bool, shift: bool) {
        let ordered = self.ordered_visible_keys();
        if shift {
            if let Some(anchor) = &self.selection_anchor {
                let anchor_pos = ordered.iter().position(|k| k == anchor);
                let key_pos = ordered.iter().position(|k| k == &key);
                if let (Some(a), Some(b)) = (anchor_pos, key_pos) {
                    let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                    self.selection = ordered
                        .get(lo..=hi)
                        .map(|s| s.iter().cloned().collect())
                        .unwrap_or_default();
                }
            } else {
                self.selection = BTreeSet::from([key]);
                self.selection_anchor = Some(key);
            }
        } else if ctrl {
            if self.selection.contains(&key) {
                self.selection.remove(&key);
            } else {
                self.selection.insert(key);
            }
        } else {
            self.selection = BTreeSet::from([key]);
            self.selection_anchor = Some(key);
        }
    }

    /// Keys in render order, restricted to nodes whose parent is expanded.
    /// Used for shift-click range selection.
    pub fn ordered_visible_keys(&self) -> Vec<NodeKey> {
        let mut keys = Vec::new();
        for (fi, file_node) in self.files.iter().enumerate() {
            let fi = FileIdx::new(fi);
            keys.push(NodeKey::File(fi));
            if file_node.expanded {
                for ti in 0..file_node.tracks.len() {
                    keys.push(NodeKey::Track(TrackRef::new(fi, TrackIdx::new(ti))));
                }
            }
        }
        keys
    }

    pub fn file_node(&self, fi: FileIdx) -> Option<&FileNode> {
        fi.get(&self.files)
    }

    pub fn file_node_mut(&mut self, fi: FileIdx) -> Option<&mut FileNode> {
        fi.get_mut(&mut self.files)
    }

    pub fn track_node(&self, track: TrackRef) -> Option<&TrackNode> {
        track.index.get(&track.fi.get(&self.files)?.tracks)
    }

    pub fn track_node_mut(&mut self, track: TrackRef) -> Option<&mut TrackNode> {
        track
            .index
            .get_mut(&mut track.fi.get_mut(&mut self.files)?.tracks)
    }

    /// Returns the derived visibility state for gt-map renderers.
    pub fn visibility(&self) -> &TrackDataVisibility {
        self.visibility.get()
    }

    pub fn visibility_generation(&self) -> Generation<TrackDataVisibility> {
        self.visibility.generation()
    }

    /// Returns the derived event-marker visibility for gt-map renderers.
    pub fn event_marker_visibility(&self) -> &EventMarkerVisibility {
        &self.event_marker_visibility
    }

    /// Returns the derived generated-marker (per-type) visibility for gt-map renderers.
    pub fn generated_marker_visibility(&self) -> &GeneratedMarkerVisibility {
        &self.generated_marker_visibility
    }

    /// Returns `true` when every track is hidden - used to trigger zoom-to-fit
    /// when the first track becomes visible.
    pub fn all_hidden(&self) -> bool {
        !self
            .visibility()
            .files
            .iter()
            .any(|f| f.enabled && f.tracks.iter().any(|t| t.enabled))
    }

    /// Rebuild what the map renderers read from the tree's checks and
    /// category toggles, and record the hidden tracks of the loaded
    /// recordings. Every method that writes a check calls this.
    fn rebuild_visibility(&mut self) {
        self.record_hidden_tracks_of_the_loaded_recordings();
        let files = &self.files;
        let vis = self.visibility.get_mut();

        vis.files.resize_with(files.len(), || FileVisibility {
            enabled: true,
            tracks: vec![],
        });

        for (fi, file_node) in files.iter().enumerate() {
            let Some(file_vis) = vis.files.get_mut(fi) else {
                continue;
            };
            file_vis.enabled = !matches!(file_node.check, CheckState::Off);
            file_vis
                .tracks
                .resize_with(file_node.tracks.len(), TrackVisibility::all_visible);

            for (ti, track_node) in file_node.tracks.iter().enumerate() {
                let Some(tv) = file_vis.tracks.get_mut(ti) else {
                    continue;
                };
                tv.enabled = matches!(track_node.check, CheckState::On);
                tv.set_category_visible(DataCategory::Track, track_node.track_visible);
                tv.set_category_visible(DataCategory::Tpv, track_node.tpv_visible);
                tv.set_category_visible(
                    DataCategory::SatelliteReport,
                    track_node.satellites_visible,
                );
                tv.set_category_visible(
                    DataCategory::CustomMarker,
                    track_node.custom_markers_visible,
                );
                tv.set_category_visible(
                    DataCategory::GeneratedMarker,
                    track_node.generated_markers_visible,
                );
                tv.set_category_visible(
                    DataCategory::EventMarker,
                    !matches!(track_node.event_paths.aggregate(), CheckState::Off),
                );
            }
        }
    }

    fn rebuild_visibility_for_track(&mut self, track: TrackRef) {
        let Some(track_node) = self.track_node(track) else {
            return;
        };
        let enabled = matches!(track_node.check, CheckState::On);
        let track_visible = track_node.track_visible;
        let tpv_visible = track_node.tpv_visible;
        let satellites_visible = track_node.satellites_visible;
        let custom_markers_visible = track_node.custom_markers_visible;
        let generated_markers_visible = track_node.generated_markers_visible;
        let event_markers_visible = !matches!(track_node.event_paths.aggregate(), CheckState::Off);

        if let Some(file_vis) = track.fi.get_mut(&mut self.visibility.get_mut().files)
            && let Some(tv) = track.index.get_mut(&mut file_vis.tracks)
        {
            tv.enabled = enabled;
            tv.set_category_visible(DataCategory::Track, track_visible);
            tv.set_category_visible(DataCategory::Tpv, tpv_visible);
            tv.set_category_visible(DataCategory::SatelliteReport, satellites_visible);
            tv.set_category_visible(DataCategory::CustomMarker, custom_markers_visible);
            tv.set_category_visible(DataCategory::GeneratedMarker, generated_markers_visible);
            tv.set_category_visible(DataCategory::EventMarker, event_markers_visible);
        }
    }

    fn rebuild_event_marker_visibility(&mut self) {
        let files = &self.files;
        let emv = &mut self.event_marker_visibility;
        emv.clear_all();

        for (fi, file_node) in files.iter().enumerate() {
            for (ti, track_node) in file_node.tracks.iter().enumerate() {
                emv.set_hidden(
                    TrackRef::new(FileIdx::new(fi), TrackIdx::new(ti)),
                    track_node.event_paths.hidden_roots().map(str::to_owned),
                );
            }
        }
    }

    fn rebuild_event_marker_visibility_for(&mut self, track: TrackRef) {
        let files = &self.files;
        let emv = &mut self.event_marker_visibility;

        let hidden = track
            .fi
            .get(files)
            .and_then(|f| track.index.get(&f.tracks))
            .into_iter()
            .flat_map(|t| t.event_paths.hidden_roots())
            .map(str::to_owned);

        emv.set_hidden(track, hidden);
    }

    fn rebuild_generated_marker_visibility(&mut self) {
        let gmv = &mut self.generated_marker_visibility;
        gmv.clear_all();
        for (fi, file_node) in self.files.iter().enumerate() {
            for (ti, track_node) in file_node.tracks.iter().enumerate() {
                gmv.set_hidden(
                    TrackRef::new(FileIdx::new(fi), TrackIdx::new(ti)),
                    track_node.generated_kinds_hidden.iter().copied(),
                );
            }
        }
    }

    fn rebuild_generated_marker_visibility_for(&mut self, track: TrackRef) {
        let hidden: Vec<GeneratedMarkerKindTag> = track
            .fi
            .get(&self.files)
            .and_then(|f| track.index.get(&f.tracks))
            .into_iter()
            .flat_map(|t| t.generated_kinds_hidden.iter().copied())
            .collect();
        self.generated_marker_visibility
            .set_hidden(track, hidden.into_iter());
    }
}

/// Put every key of `positions` at the position it sits at now, dropping the
/// ones whose row is gone.
fn keep_moved_positions(positions: &mut Vec<NodeKey>, moved: impl Fn(NodeKey) -> Option<NodeKey>) {
    positions.retain_mut(|position| match moved(*position) {
        Some(moved_to) => {
            *position = moved_to;
            true
        }
        None => false,
    });
}

fn set_subtree(nodes: &mut BTreeMap<String, CheckState>, prefix: &str, state: CheckState) {
    let child_prefix = format!("{prefix}/");
    for (path, check) in nodes.iter_mut() {
        if path == prefix || path.starts_with(&child_prefix) {
            *check = state;
        }
    }
}

fn recompute_ancestors(nodes: &mut BTreeMap<String, CheckState>, changed_path: &str) {
    let mut current = changed_path.to_owned();
    while let Some(pos) = current.rfind('/') {
        let Some(parent_str) = current.get(..pos) else {
            break;
        };
        let parent = parent_str.to_owned();
        let prefix = format!("{parent}/");
        // Collect first to release the immutable borrow before mutably updating the parent.
        let child_states: Vec<CheckState> = nodes
            .iter()
            .filter(|(k, _)| {
                k.starts_with(&prefix) && k.get(prefix.len()..).is_some_and(|s| !s.contains('/'))
            })
            .map(|(_, &v)| v)
            .collect();
        if let Some(agg) = aggregate_check_states(child_states.into_iter())
            && let Some(node) = nodes.get_mut(&parent)
        {
            *node = agg;
        }
        current = parent;
    }
}

fn aggregate_check_states(mut states: impl Iterator<Item = CheckState>) -> Option<CheckState> {
    let first = states.next()?;
    for s in states {
        if s != first {
            return Some(CheckState::Mixed);
        }
    }
    Some(first)
}
