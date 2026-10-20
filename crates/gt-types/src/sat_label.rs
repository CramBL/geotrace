use crate::highlight::PointIdx;

/// Priority tier of a [`SatLabelAnchor`], ordered by diagnostic value:
/// variants are declared highest-priority first, so the derived [`Ord`]
/// makes a smaller tier win when label placement resolves collisions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, strum::EnumCount)]
pub enum SatLabelTier {
    /// Resolved state or used-count quality differs from the previous point
    /// with a known global used count.
    QualityTransition,
    /// Local minimum of the fix count - the worst point of a dip.
    FixCountMinimum,
    /// First or last point with a known global used count, or a recorded Fix
    /// with a known global used count immediately after synthesis or known loss.
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
    /// Index into the track's points with a known global used count.
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
