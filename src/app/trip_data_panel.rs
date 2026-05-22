use nav_types::DataCategory;
use std::collections::BTreeSet;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub enum SelectionKey {
    File(usize),
    Trip(usize, usize),
}

pub struct DeleteConfirmState {
    pub items: Vec<SelectionKey>,
}

pub struct TripDataPanelState {
    pub expanded_files: BTreeSet<usize>,
    pub expanded_trips: BTreeSet<(usize, usize)>,
    pub expanded_categories: BTreeSet<(usize, usize, DataCategory)>,
    pub selection: BTreeSet<SelectionKey>,
    pub selection_anchor: Option<SelectionKey>,
    pub delete_confirm: Option<DeleteConfirmState>,
    pub detached: bool,
    pub viewport_id: egui::ViewportId,
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
            viewport_id: egui::ViewportId::from_hash_of("trip_data_panel"),
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
