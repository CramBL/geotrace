use std::collections::{BTreeMap, BTreeSet};

use gt_types::{DataCategory, FileIdx, GeneratedMarkerKindTag, LoadedFile, TrackIdx, TrackRef};
use gt_ui_types::{
    EventMarkerVisibility, FileVisibility, GeneratedMarkerVisibility, TrackDataVisibility,
    TrackVisibility,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CheckState {
    On,
    Off,
    Mixed,
}

/// A tree node that can be selected (for shift/ctrl-click).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum NodeKey {
    File(FileIdx),
    Track(TrackRef),
}

pub struct DeleteConfirmState {
    pub items: Vec<NodeKey>,
    /// When set, removal also permanently deletes the affected recordings from
    /// the history database instead of only hiding them.
    pub delete_permanently: bool,
}

/// Tracks which data categories are currently expanded in the track row.
///
/// Replaces `BTreeSet<DataCategory>` to avoid per-interaction heap allocations.
#[derive(Default, Clone, Copy, Debug)]
pub struct CategoriesExpanded {
    tpv: bool,
    satellite_report: bool,
    custom_marker: bool,
    generated_marker: bool,
    event_marker: bool,
}

impl CategoriesExpanded {
    pub fn contains(&self, cat: &DataCategory) -> bool {
        match cat {
            DataCategory::Track => false,
            DataCategory::Tpv => self.tpv,
            DataCategory::SatelliteReport => self.satellite_report,
            DataCategory::CustomMarker => self.custom_marker,
            DataCategory::GeneratedMarker => self.generated_marker,
            DataCategory::EventMarker => self.event_marker,
        }
    }

    pub fn insert(&mut self, cat: DataCategory) {
        match cat {
            DataCategory::Track => {}
            DataCategory::Tpv => self.tpv = true,
            DataCategory::SatelliteReport => self.satellite_report = true,
            DataCategory::CustomMarker => self.custom_marker = true,
            DataCategory::GeneratedMarker => self.generated_marker = true,
            DataCategory::EventMarker => self.event_marker = true,
        }
    }

    pub fn remove(&mut self, cat: &DataCategory) {
        match cat {
            DataCategory::Track => {}
            DataCategory::Tpv => self.tpv = false,
            DataCategory::SatelliteReport => self.satellite_report = false,
            DataCategory::CustomMarker => self.custom_marker = false,
            DataCategory::GeneratedMarker => self.generated_marker = false,
            DataCategory::EventMarker => self.event_marker = false,
        }
    }

    pub fn toggle(&mut self, cat: DataCategory) {
        if self.contains(&cat) {
            self.remove(&cat);
        } else {
            self.insert(cat);
        }
    }
}

/// Per-track tree of event variant paths with tri-state visibility.
///
/// Stores all unique prefix segments derived from the track's marker
/// variant paths.  Leaves are paths that exist as actual marker variants;
/// internal nodes are shared prefix segments.  `CheckState` is derived for
/// internal nodes from their children. Only leaves carry "true" state.
#[derive(Default)]
pub struct EventPathTree {
    /// All paths (leaves and internal nodes) → visibility state.
    pub nodes: BTreeMap<String, CheckState>,
}

impl EventPathTree {
    /// Sync the tree from the current set of marker variant paths.
    ///
    /// New nodes are added as `On`; existing nodes preserve their state;
    /// nodes that no longer appear are removed.
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
        let new_state = match current {
            CheckState::On => CheckState::Off,
            CheckState::Off | CheckState::Mixed => CheckState::On,
        };
        set_subtree(&mut self.nodes, path, new_state);
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

pub struct TrackNode {
    pub expanded: bool,
    pub check: CheckState,
    pub categories_expanded: CategoriesExpanded,
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
    /// visibility yet, so they get a dedicated flag rather than a
    /// [`DataCategory`] entry.
    pub channels_expanded: bool,
}

impl TrackNode {
    fn new() -> Self {
        Self {
            expanded: false,
            check: CheckState::On,
            categories_expanded: CategoriesExpanded::default(),
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
}

pub struct FileNode {
    pub expanded: bool,
    pub check: CheckState,
    pub tracks: Vec<TrackNode>,
}

impl FileNode {
    fn new() -> Self {
        Self {
            expanded: false,
            check: CheckState::On,
            tracks: Vec::new(),
        }
    }

    fn recompute_check(&mut self) {
        self.check =
            aggregate_check_states(self.tracks.iter().map(|t| t.check)).unwrap_or(CheckState::On);
    }
}

pub struct TreeState {
    pub files: Vec<FileNode>,
    pub selection: BTreeSet<NodeKey>,
    pub selection_anchor: Option<NodeKey>,
    pub delete_confirm: Option<DeleteConfirmState>,
    /// Items the user asked to unload from the view (non-destructive, the
    /// recordings stay in history). Consumed by the app each frame.
    pub pending_unload: Option<Vec<NodeKey>>,
    pub detached: bool,
    /// Derived from tree state, kept in sync.  Passed to gt-map renderers.
    visibility: TrackDataVisibility,
    /// Derived from tree state, kept in sync.  Passed to gt-map renderers.
    event_marker_visibility: EventMarkerVisibility,
    /// Derived from tree state, kept in sync.  Passed to gt-map renderers.
    generated_marker_visibility: GeneratedMarkerVisibility,
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
            delete_confirm: None,
            pending_unload: None,
            detached: false,
            visibility: TrackDataVisibility { files: Vec::new() },
            event_marker_visibility: EventMarkerVisibility::new(),
            generated_marker_visibility: GeneratedMarkerVisibility::new(),
        }
    }

    /// Integrate newly loaded files while preserving state for existing nodes.
    ///
    /// Appends `FileNode`/`TripNode` entries for any new data and rebuilds
    /// the event path trees.  Does not reset any check or expand state.
    pub fn sync_from_loaded_files(&mut self, files: &[LoadedFile]) {
        while self.files.len() < files.len() {
            self.files.push(FileNode::new());
        }
        self.files.truncate(files.len());

        for (file_node, loaded_file) in self.files.iter_mut().zip(files.iter()) {
            while file_node.tracks.len() < loaded_file.tracks.len() {
                file_node.tracks.push(TrackNode::new());
            }
            file_node.tracks.truncate(loaded_file.tracks.len());

            for (track_node, loaded_track) in
                file_node.tracks.iter_mut().zip(loaded_file.tracks.iter())
            {
                track_node.event_paths.sync_from_paths(
                    loaded_track
                        .event_markers
                        .iter()
                        .map(|m| m.variant_path.as_str()),
                );
            }

            file_node.recompute_check();
        }

        self.rebuild_visibility();
        self.rebuild_event_marker_visibility();
        self.rebuild_generated_marker_visibility();
    }

    /// Full reset: rebuild from scratch with all nodes On and collapsed.
    /// Used after deletion when indices shift.
    pub fn reset_for_files(&mut self, files: &[LoadedFile]) {
        self.files = files
            .iter()
            .map(|loaded_file| {
                let mut file_node = FileNode::new();
                for loaded_track in &loaded_file.tracks {
                    let mut track_node = TrackNode::new();
                    track_node.event_paths.sync_from_paths(
                        loaded_track
                            .event_markers
                            .iter()
                            .map(|m| m.variant_path.as_str()),
                    );
                    file_node.tracks.push(track_node);
                }
                file_node
            })
            .collect();
        self.selection.clear();
        self.selection_anchor = None;
        self.delete_confirm = None;
        self.rebuild_visibility();
        self.rebuild_event_marker_visibility();
        self.rebuild_generated_marker_visibility();
    }

    /// Toggle a file's check state with cascade to all child tracks.
    pub fn toggle_file_check(&mut self, fi: FileIdx) {
        let Some(file_node) = self.file_node_mut(fi) else {
            return;
        };
        let new_state = match file_node.check {
            CheckState::On => CheckState::Off,
            CheckState::Off | CheckState::Mixed => CheckState::On,
        };
        file_node.check = new_state;
        for track in &mut file_node.tracks {
            track.check = new_state;
        }
        self.rebuild_visibility();
    }

    /// Toggle a track's check state and recompute the parent file's aggregate.
    pub fn toggle_track_check(&mut self, track: TrackRef) {
        let Some(file_node) = self.file_node_mut(track.fi) else {
            return;
        };
        let Some(track_node) = track.index.get_mut(&mut file_node.tracks) else {
            return;
        };
        track_node.check = match track_node.check {
            CheckState::On => CheckState::Off,
            CheckState::Off | CheckState::Mixed => CheckState::On,
        };
        file_node.recompute_check();
        self.rebuild_visibility();
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
        let new_state = match agg {
            CheckState::On => CheckState::Off,
            CheckState::Off | CheckState::Mixed => CheckState::On,
        };
        self.set_all_event_paths(track, new_state);
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

    pub fn toggle_expand_track(&mut self, track: TrackRef) {
        if let Some(track_node) = self.track_node_mut(track) {
            track_node.expanded = !track_node.expanded;
        }
    }

    pub fn toggle_category_expanded(&mut self, track: TrackRef, cat: DataCategory) {
        if let Some(track_node) = self.track_node_mut(track) {
            track_node.categories_expanded.toggle(cat);
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
                track_node.check = state;
            }
        }
        self.rebuild_visibility();
    }

    pub fn show_only_file(&mut self, fi: FileIdx) {
        for (i, file_node) in self.files.iter_mut().enumerate() {
            if FileIdx::new(i) == fi {
                file_node.check = CheckState::On;
                for track_node in &mut file_node.tracks {
                    track_node.check = CheckState::On;
                }
            } else {
                file_node.check = CheckState::Off;
                for track_node in &mut file_node.tracks {
                    track_node.check = CheckState::Off;
                }
            }
        }
        self.rebuild_visibility();
    }

    /// Show only the given tracks. Hide everything else. All listed tracks must
    /// belong to the same file as each other (any file structure is fine, but
    /// each `TrackRef` identifies a file+track pair). Tracks from files not
    /// mentioned in `tracks` are hidden. Within a mentioned file, only the
    /// listed tracks are shown.
    pub fn show_only_tracks(&mut self, tracks: &[TrackRef]) {
        for (i, file_node) in self.files.iter_mut().enumerate() {
            let fi = FileIdx::new(i);
            let any_shown = tracks.iter().any(|t| t.fi == fi);
            if !any_shown {
                file_node.check = CheckState::Off;
                for track_node in &mut file_node.tracks {
                    track_node.check = CheckState::Off;
                }
            } else {
                for (j, track_node) in file_node.tracks.iter_mut().enumerate() {
                    let ti = TrackIdx::new(j);
                    track_node.check = if tracks.iter().any(|t| t.fi == fi && t.index == ti) {
                        CheckState::On
                    } else {
                        CheckState::Off
                    };
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
                    track_node.check = if TrackIdx::new(j) == track.index {
                        CheckState::On
                    } else {
                        CheckState::Off
                    };
                }
                file_node.recompute_check();
            } else {
                file_node.check = CheckState::Off;
                for track_node in &mut file_node.tracks {
                    track_node.check = CheckState::Off;
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
        &self.visibility
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
            .visibility
            .files
            .iter()
            .any(|f| f.enabled && f.tracks.iter().any(|t| t.enabled))
    }

    fn rebuild_visibility(&mut self) {
        let files = &self.files;
        let vis = &mut self.visibility;

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
                tv.track_visible = track_node.track_visible;
                tv.tpv_visible = track_node.tpv_visible;
                tv.satellites_visible = track_node.satellites_visible;
                tv.custom_markers_visible = track_node.custom_markers_visible;
                tv.generated_markers_visible = track_node.generated_markers_visible;
                tv.event_markers_visible =
                    !matches!(track_node.event_paths.aggregate(), CheckState::Off);
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

        if let Some(file_vis) = track.fi.get_mut(&mut self.visibility.files)
            && let Some(tv) = track.index.get_mut(&mut file_vis.tracks)
        {
            tv.enabled = enabled;
            tv.track_visible = track_visible;
            tv.tpv_visible = tpv_visible;
            tv.satellites_visible = satellites_visible;
            tv.custom_markers_visible = custom_markers_visible;
            tv.generated_markers_visible = generated_markers_visible;
            tv.event_markers_visible = event_markers_visible;
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
