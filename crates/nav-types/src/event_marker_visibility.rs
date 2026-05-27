use std::collections::{BTreeSet, HashMap};

/// Per-trip visibility state for event marker variant paths.
///
/// A marker at variant path `p` is hidden when any prefix of `p` (including `p`
/// itself) appears in the hidden set for that trip.
/// This lets a single toggle on a parent node hide all its descendants.
#[derive(Debug, Clone, Default)]
pub struct EventMarkerVisibility {
    hidden: HashMap<(usize, usize), BTreeSet<String>>,
}

impl EventMarkerVisibility {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` when the variant should be rendered (not hidden).
    pub fn is_visible(&self, fi: usize, ti: usize, variant_path: &str) -> bool {
        let Some(hidden) = self.hidden.get(&(fi, ti)) else {
            return true;
        };
        let mut path = variant_path;
        loop {
            if hidden.contains(path) {
                return false;
            }
            match path.rfind('/') {
                Some(pos) => match path.get(..pos) {
                    Some(prefix) => path = prefix,
                    None => return true,
                },
                None => return true,
            }
        }
    }

    /// Returns `true` when this exact path (not a descendant) is explicitly hidden.
    pub fn is_explicitly_hidden(&self, fi: usize, ti: usize, path: &str) -> bool {
        self.hidden.get(&(fi, ti)).is_some_and(|h| h.contains(path))
    }

    /// Toggle the explicit hidden state of `path` for the given trip.
    pub fn toggle(&mut self, fi: usize, ti: usize, path: &str) {
        let hidden = self.hidden.entry((fi, ti)).or_default();
        if !hidden.remove(path) {
            hidden.insert(path.to_owned());
        }
    }
}
