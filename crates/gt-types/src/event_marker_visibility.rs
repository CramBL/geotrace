use std::collections::{BTreeSet, HashMap};

use crate::highlight::{FileIdx, TrackIdx};

/// Per-trip visibility state for event marker variant paths.
///
/// A marker at variant path `p` is hidden when any prefix of `p` (including `p`
/// itself) appears in the hidden set for that trip.
/// This lets a single toggle on a parent node hide all its descendants.
#[derive(Debug, Clone, Default)]
pub struct EventMarkerVisibility {
    hidden: HashMap<(FileIdx, TrackIdx), BTreeSet<String>>,
}

impl EventMarkerVisibility {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` when the variant should be rendered (not hidden).
    pub fn is_visible(&self, fi: FileIdx, ti: TrackIdx, variant_path: &str) -> bool {
        let Some(hidden) = self.hidden.get(&(fi, ti)) else {
            return true;
        };
        let mut path = variant_path;
        loop {
            if hidden.contains(path) {
                return false;
            }
            let Some(pos) = path.rfind('/') else {
                return true;
            };
            let Some(prefix) = path.get(..pos) else {
                return true;
            };
            path = prefix;
        }
    }

    /// Returns `true` when this exact path (not a descendant) is explicitly hidden.
    pub fn is_explicitly_hidden(&self, fi: FileIdx, ti: TrackIdx, path: &str) -> bool {
        self.hidden.get(&(fi, ti)).is_some_and(|h| h.contains(path))
    }

    /// Toggle the explicit hidden state of `path` for the given trip.
    pub fn toggle(&mut self, fi: FileIdx, ti: TrackIdx, path: &str) {
        let hidden = self.hidden.entry((fi, ti)).or_default();
        if !hidden.remove(path) {
            hidden.insert(path.to_owned());
        }
    }

    /// Hide `path` and remove any explicit hidden entries for its descendants
    /// (they are now redundant — the parent covers them).
    pub fn set_hidden_cascade(&mut self, fi: FileIdx, ti: TrackIdx, path: &str) {
        let hidden = self.hidden.entry((fi, ti)).or_default();
        let child_prefix = format!("{path}/");
        hidden.retain(|p| !p.starts_with(&child_prefix));
        hidden.insert(path.to_owned());
    }

    /// Show `path` by removing its explicit hidden entry and clearing any
    /// explicitly hidden descendants so they inherit visibility.
    pub fn set_visible_cascade(&mut self, fi: FileIdx, ti: TrackIdx, path: &str) {
        if let Some(hidden) = self.hidden.get_mut(&(fi, ti)) {
            let child_prefix = format!("{path}/");
            hidden.retain(|p| p != path && !p.starts_with(&child_prefix));
        }
    }

    /// Replace the hidden set for one trip with the given minimal root paths.
    ///
    /// Each entry in `hidden_roots` should be a path whose parent is NOT hidden —
    /// callers are responsible for ensuring minimality.
    pub fn set_hidden(
        &mut self,
        fi: FileIdx,
        ti: TrackIdx,
        hidden_roots: impl Iterator<Item = String>,
    ) {
        self.hidden.remove(&(fi, ti));
        let paths: BTreeSet<String> = hidden_roots.collect();
        if !paths.is_empty() {
            self.hidden.insert((fi, ti), paths);
        }
    }

    /// Clear all hidden state.
    pub fn clear_all(&mut self) {
        self.hidden.clear();
    }
}
