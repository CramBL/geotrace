use std::collections::HashMap;

use gt_types::{GeneratedMarkerKindSet, GeneratedMarkerKindTag, TrackRef};

/// Per-track visibility of generated-marker event types.
///
/// A marker of tag `t` is hidden when `t` is in the hidden set for its track.
/// This refines the category-level visibility (the "Generated markers" toggle)
/// so individual event types can be shown or hidden, mirroring how
/// [`crate::EventMarkerVisibility`] refines event markers by variant path.
#[derive(Debug, Clone, Default)]
pub struct GeneratedMarkerVisibility {
    hidden: HashMap<TrackRef, GeneratedMarkerKindSet>,
}

impl GeneratedMarkerVisibility {
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` when markers of `tag` should be rendered for `track`.
    pub fn is_visible(&self, track: TrackRef, tag: GeneratedMarkerKindTag) -> bool {
        !self
            .hidden
            .get(&track)
            .is_some_and(|hidden| hidden.contains(tag))
    }

    /// Replace the hidden set for one track.
    pub fn set_hidden(
        &mut self,
        track: TrackRef,
        hidden: impl Iterator<Item = GeneratedMarkerKindTag>,
    ) {
        let set: GeneratedMarkerKindSet = hidden.collect();
        if set.is_empty() {
            self.hidden.remove(&track);
        } else {
            self.hidden.insert(track, set);
        }
    }

    /// Clear all hidden state.
    pub fn clear_all(&mut self) {
        self.hidden.clear();
    }
}
