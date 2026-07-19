//! Which parts of the clicked-point window are folded away.

use gt_types::satellites::{Constellation, ConstellationSet};

/// The clicked-point window's fold state: whether the sky plot is folded, and
/// which constellations have their satellite table folded.
///
/// A folded constellation still shows its header - colour, name and fix/seen
/// count - so folding drops the rows without losing the overview.
///
/// Defaults to everything unfolded.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PointWindowFolds {
    /// Whether the sky plot is folded away, leaving the satellite tables the
    /// full window.
    pub plot_folded: bool,
    /// Constellations whose satellite table is folded away.
    pub folded_constellations: ConstellationSet,
}

impl PointWindowFolds {
    /// Whether `constellation`'s satellite table is folded away.
    pub const fn is_folded(self, constellation: Constellation) -> bool {
        self.folded_constellations.contains(constellation)
    }

    /// Fold or unfold `constellation`'s satellite table.
    pub const fn toggle(&mut self, constellation: Constellation) {
        let folded = self.folded_constellations.contains(constellation);
        self.folded_constellations.set(constellation, !folded);
    }
}

#[cfg(test)]
mod tests {
    use super::PointWindowFolds;
    use gt_types::satellites::Constellation;

    #[test]
    fn everything_starts_unfolded() {
        let folds = PointWindowFolds::default();
        assert!(!folds.plot_folded);
        assert!(!folds.is_folded(Constellation::Gps));
    }

    #[test]
    fn toggling_folds_one_constellation_at_a_time() {
        let mut folds = PointWindowFolds::default();
        folds.toggle(Constellation::Gps);
        assert!(folds.is_folded(Constellation::Gps));
        // Its neighbours are untouched.
        assert!(!folds.is_folded(Constellation::Galileo));

        folds.toggle(Constellation::Gps);
        assert!(!folds.is_folded(Constellation::Gps));
    }
}
