//! Which parts of the clicked-point window are folded away.

use gt_types::satellites::{Constellation, ConstellationSet};
use strum::IntoEnumIterator as _;

/// The clicked-point window's fold state: whether the sky plot is folded, and
/// which constellations have their satellite table folded.
///
/// A folded constellation still shows its header - colour, name and fix/seen
/// count - so folding drops the rows without losing the overview.
///
/// Defaults to everything unfolded, which is also what settings written
/// before this key existed deserialize to.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(from = "FoldedSections", into = "FoldedSections")]
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

/// The wire form of [`PointWindowFolds`]: the folded sections listed by name.
///
/// An absent or empty list means nothing is folded. A constellation added later
/// defaults to unfolded on an existing settings file.
#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct FoldedSections {
    plot: bool,
    constellations: Vec<Constellation>,
}

impl From<FoldedSections> for PointWindowFolds {
    fn from(folded: FoldedSections) -> Self {
        let mut folds = Self {
            plot_folded: folded.plot,
            ..Self::default()
        };
        for constellation in folded.constellations {
            folds.folded_constellations.insert(constellation);
        }
        folds
    }
}

impl From<PointWindowFolds> for FoldedSections {
    fn from(folds: PointWindowFolds) -> Self {
        Self {
            plot: folds.plot_folded,
            constellations: Constellation::iter()
                .filter(|&c| folds.is_folded(c))
                .collect(),
        }
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

    /// The persisted form lists folded constellations by name in declaration
    /// order, whatever order they were folded in, and round-trips back.
    #[test]
    fn wire_form_lists_folded_sections_by_name() {
        use super::FoldedSections;

        let mut folds = PointWindowFolds {
            plot_folded: true,
            ..PointWindowFolds::default()
        };
        folds.toggle(Constellation::Galileo);
        folds.toggle(Constellation::Gps);

        let wire = FoldedSections::from(folds);
        assert!(wire.plot);
        assert_eq!(
            wire.constellations,
            vec![Constellation::Gps, Constellation::Galileo]
        );
        assert_eq!(PointWindowFolds::from(wire), folds);
    }

    /// Settings written before this key existed - and a settings file that
    /// simply folds nothing - both read back as everything unfolded.
    #[test]
    fn an_absent_wire_form_means_nothing_folded() {
        assert_eq!(
            PointWindowFolds::from(super::FoldedSections::default()),
            PointWindowFolds::default()
        );
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
