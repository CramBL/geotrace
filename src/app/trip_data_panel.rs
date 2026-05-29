use gt_types::{DataCategory, FileIdx, TripIdx};
use std::collections::{BTreeSet, HashMap};

/// A typed pair of file + trip indices — used as a key wherever both are
/// needed together, replacing bare `(FileIdx, TripIdx)` tuples.
#[derive(Copy, Clone, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub struct TripRef {
    pub file: FileIdx,
    pub trip: TripIdx,
}

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SelectionKey {
    File(FileIdx),
    Trip(TripRef),
}

pub struct DeleteConfirmState {
    pub items: Vec<SelectionKey>,
}

pub struct TripDataPanelState {
    pub expanded_files: BTreeSet<FileIdx>,
    pub expanded_trips: BTreeSet<TripRef>,
    pub expanded_categories: BTreeSet<(TripRef, DataCategory)>,
    pub selection: BTreeSet<SelectionKey>,
    pub selection_anchor: Option<SelectionKey>,
    pub delete_confirm: Option<DeleteConfirmState>,
    pub detached: bool,
    /// Per-trip prefix filter text for the Events section.
    pub event_marker_filter: HashMap<TripRef, String>,
}

impl TripDataPanelState {
    pub fn new() -> Self {
        Self {
            expanded_files: BTreeSet::new(),
            expanded_trips: BTreeSet::new(),
            expanded_categories: BTreeSet::new(),
            selection: BTreeSet::new(),
            selection_anchor: None,
            delete_confirm: None,
            detached: false,
            event_marker_filter: HashMap::new(),
        }
    }
}

pub fn apply_click(
    state: &mut TripDataPanelState,
    key: SelectionKey,
    ctrl: bool,
    shift: bool,
    ordered_keys: &[SelectionKey],
) {
    if shift {
        if let Some(anchor) = &state.selection_anchor {
            let anchor_pos = ordered_keys.iter().position(|k| k == anchor);
            let key_pos = ordered_keys.iter().position(|k| k == &key);
            if let (Some(a), Some(b)) = (anchor_pos, key_pos) {
                let (lo, hi) = if a <= b { (a, b) } else { (b, a) };
                state.selection = ordered_keys
                    .get(lo..=hi)
                    .map(|s| s.iter().cloned().collect())
                    .unwrap_or_default();
            }
        } else {
            state.selection = BTreeSet::from([key.clone()]);
            state.selection_anchor = Some(key);
        }
    } else if ctrl {
        if state.selection.contains(&key) {
            state.selection.remove(&key);
        } else {
            state.selection.insert(key);
        }
    } else {
        state.selection = BTreeSet::from([key.clone()]);
        state.selection_anchor = Some(key);
    }
}
