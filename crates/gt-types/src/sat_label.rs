use crate::highlight::PointIdx;

/// Priority tier of a [`SatLabelAnchor`], ordered by diagnostic value:
/// variants are declared highest-priority first, so the derived [`Ord`]
/// makes a smaller tier win when label placement resolves collisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, strum::EnumCount)]
pub enum SatLabelTier {
    /// The point's [`crate::FixQuality`] differs from the previous point
    /// with a satellite report - a degradation or a recovery.
    QualityTransition,
    /// Local minimum of the fix count - the worst point of a dip.
    FixCountMinimum,
    /// Track start or end, or the first real fix after a ghost stretch.
    Endpoint,
    /// Periodic coverage along the track so labels appear at high zoom
    /// even on stretches with stable satellite reception.
    Fill,
}

/// A point of a track whose satellite state is worth labeling on the map.
///
/// Anchors are selected once per track at load time from the recording's
/// data, so the set is independent of the viewport: which anchors actually
/// get a label at a given zoom is the renderer's collision resolution,
/// which keeps the highest-priority anchor per screen region.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SatLabelAnchor {
    /// Index into the owning track's points. Always a point that carries
    /// a satellite report - there is nothing to label otherwise.
    pub point: PointIdx,
    pub tier: SatLabelTier,
}

#[cfg(test)]
mod tests {
    use super::*;
    use strum::EnumCount;

    #[test]
    fn tier_ordering_is_priority_ordering() {
        let mut tiers = [
            SatLabelTier::Fill,
            SatLabelTier::Endpoint,
            SatLabelTier::QualityTransition,
            SatLabelTier::FixCountMinimum,
        ];
        // A variant added to the `enum` but not to this list fails here.
        assert_eq!(tiers.len(), SatLabelTier::COUNT);
        tiers.sort();
        assert_eq!(
            tiers,
            [
                SatLabelTier::QualityTransition,
                SatLabelTier::FixCountMinimum,
                SatLabelTier::Endpoint,
                SatLabelTier::Fill,
            ]
        );
    }
}
