//! The palette slots the layer chips draw their map colour from.

/// Colours the log-layer palette holds for layer chips, beside the one reserved
/// for the live filter.
pub const LAYER_COLOR_SLOT_COUNT: usize = 5;

/// One slot of the log-layer palette, held by a layer chip for as long as that
/// chip exists.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct LayerColorSlot(usize);

impl LayerColorSlot {
    /// Indexes the palette, below [`LAYER_COLOR_SLOT_COUNT`].
    pub fn index(self) -> usize {
        self.0
    }

    /// The slot a stored filter stack drew in. An index past this build's
    /// palette reads as its last slot, and stays a preference either way:
    /// [`LayerColorSlots::allocate_preferring`] hands out a free slot when the
    /// session already gave this one away.
    pub(crate) fn from_stored_index(index: usize) -> Self {
        Self(index.min(LAYER_COLOR_SLOT_COUNT.saturating_sub(1)))
    }
}

/// The slots every loaded log's layer chips draw from: a colour means one
/// filter across the whole session, whichever log owns it.
#[expect(
    missing_copy_implementations,
    reason = "copying the allocator would hand the same slot out twice"
)]
#[derive(Debug, Default)]
pub struct LayerColorSlots {
    holders: [usize; LAYER_COLOR_SLOT_COUNT],
}

impl LayerColorSlots {
    /// Hands out the lowest-numbered free slot, or once all of them are taken,
    /// the least held one: from the sixth concurrent layer chip on, colours
    /// repeat and [`LayerColorSlots::is_shared`] marks the repeats.
    pub(crate) fn allocate(&mut self) -> LayerColorSlot {
        let slot = self
            .holders
            .iter()
            .enumerate()
            .min_by_key(|(_, holders)| **holders)
            .map_or(0, |(index, _)| index);
        if let Some(holders) = self.holders.get_mut(slot) {
            *holders = holders.saturating_add(1);
        }
        LayerColorSlot(slot)
    }

    /// Hands out `preferred` while no chip holds it, so a log unloaded and
    /// loaded again - or restored from an attachment - draws in the colours it
    /// had. Falls back to [`allocate`](Self::allocate) when it is taken.
    pub(crate) fn allocate_preferring(&mut self, preferred: LayerColorSlot) -> LayerColorSlot {
        let free = self
            .holders
            .get(preferred.index())
            .is_some_and(|holders| *holders == 0);
        if !free {
            return self.allocate();
        }
        if let Some(holders) = self.holders.get_mut(preferred.index()) {
            *holders = holders.saturating_add(1);
        }
        preferred
    }

    pub(crate) fn release(&mut self, slot: LayerColorSlot) {
        if let Some(holders) = self.holders.get_mut(slot.index()) {
            *holders = holders.saturating_sub(1);
        }
    }

    /// Whether more than one layer chip holds `slot`. The map draws a doubled
    /// outline around those hexagons: a repeated colour still reads as two
    /// filters.
    pub fn is_shared(&self, slot: LayerColorSlot) -> bool {
        self.holders.get(slot.index()).is_some_and(|held| *held > 1)
    }

    #[cfg(test)]
    pub(crate) fn holders_of(&self, slot: LayerColorSlot) -> usize {
        self.holders.get(slot.index()).copied().unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn allocated(slots: &mut LayerColorSlots, count: usize) -> Vec<usize> {
        (0..count)
            .map(|_| slots.allocate().index())
            .collect::<Vec<_>>()
    }

    #[test]
    fn slots_are_handed_out_lowest_first() {
        let mut slots = LayerColorSlots::default();
        assert_eq!(
            allocated(&mut slots, LAYER_COLOR_SLOT_COUNT),
            [0, 1, 2, 3, 4]
        );
    }

    #[test]
    fn a_preferred_slot_is_handed_out_while_it_is_free_and_skipped_once_it_is_taken() {
        let mut slots = LayerColorSlots::default();

        assert_eq!(
            slots.allocate_preferring(LayerColorSlot(3)),
            LayerColorSlot(3)
        );
        assert_eq!(
            slots.allocate_preferring(LayerColorSlot(3)),
            LayerColorSlot(0),
            "a slot another chip holds falls back to the lowest free one"
        );
    }

    #[test]
    fn a_released_slot_is_the_lowest_free_one_again() {
        let mut slots = LayerColorSlots::default();
        allocated(&mut slots, LAYER_COLOR_SLOT_COUNT);

        slots.release(LayerColorSlot(2));

        assert_eq!(slots.allocate(), LayerColorSlot(2));
    }

    /// The sixth chip onwards cycles the five colours, each repeat marked for
    /// the renderer.
    #[test]
    fn a_sixth_chip_repeats_the_first_colour_and_marks_it_shared() {
        let mut slots = LayerColorSlots::default();
        allocated(&mut slots, LAYER_COLOR_SLOT_COUNT);
        assert!(
            (0..LAYER_COLOR_SLOT_COUNT).all(|slot| !slots.is_shared(LayerColorSlot(slot))),
            "five chips each have a colour to themselves"
        );

        assert_eq!(allocated(&mut slots, 6), [0, 1, 2, 3, 4, 0]);
        assert!(slots.is_shared(LayerColorSlot(0)));
    }

    #[test]
    fn releasing_the_repeat_leaves_the_colour_to_the_chip_that_took_it_first() {
        let mut slots = LayerColorSlots::default();
        allocated(&mut slots, LAYER_COLOR_SLOT_COUNT + 1);
        assert!(slots.is_shared(LayerColorSlot(0)));

        slots.release(LayerColorSlot(0));

        assert!(!slots.is_shared(LayerColorSlot(0)));
        assert_eq!(slots.holders_of(LayerColorSlot(0)), 1);
    }
}
