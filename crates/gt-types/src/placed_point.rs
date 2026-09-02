use std::ops::Range;

use crate::coordinates::{Latitude, Longitude};
use crate::highlight::FixRef;
use crate::mercator::MercPoint;
use crate::nav_point::{NavPoint, ResolvedPosition};

/// A recorded fix together with the position the track builder placed it at.
#[derive(Debug, Clone, Copy)]
pub struct PlacedPoint<'a> {
    pub fix: &'a NavPoint,
    resolved: ResolvedPosition,
}

impl PlacedPoint<'_> {
    pub fn resolved(self) -> ResolvedPosition {
        self.resolved
    }

    pub fn resolved_position(self) -> (Latitude, Longitude) {
        self.resolved.coordinates()
    }

    /// The resolved position in normalized Web Mercator coordinates, see
    /// [`crate::mercator`].
    pub fn merc(self) -> MercPoint {
        self.resolved.merc()
    }
}

/// A [`PlacedPoint`] under the address of the fix it holds, for callers
/// walking the fixes of several tracks at once.
#[derive(Debug, Clone, Copy)]
pub struct AddressedFix<'a> {
    pub fix: FixRef,
    pub placed: PlacedPoint<'a>,
}

/// The fixes of a track that has a geometry, each paired with where it is
/// drawn. Reached through [`crate::track::LoadedTrack::placed_points`], which
/// yields nothing for a track no fix of which has a valid position.
#[derive(Debug, Clone, Copy, Default)]
pub struct PlacedPoints<'a> {
    fixes: &'a [NavPoint],
    resolved: &'a [ResolvedPosition],
}

impl<'a> PlacedPoints<'a> {
    /// `None` unless `resolved` holds one position per fix, in fix order.
    pub fn new(fixes: &'a [NavPoint], resolved: &'a [ResolvedPosition]) -> Option<Self> {
        (fixes.len() == resolved.len()).then_some(Self { fixes, resolved })
    }

    pub fn len(self) -> usize {
        self.fixes.len()
    }

    pub fn is_empty(self) -> bool {
        self.fixes.is_empty()
    }

    pub fn fixes(self) -> &'a [NavPoint] {
        self.fixes
    }

    pub fn get(self, index: usize) -> Option<PlacedPoint<'a>> {
        Some(PlacedPoint {
            fix: self.fixes.get(index)?,
            resolved: *self.resolved.get(index)?,
        })
    }

    /// The stretch of points `range` covers, `None` when it reaches past them.
    pub fn range(self, range: Range<usize>) -> Option<Self> {
        Some(Self {
            fixes: self.fixes.get(range.clone())?,
            resolved: self.resolved.get(range)?,
        })
    }

    pub fn iter(self) -> impl ExactSizeIterator<Item = PlacedPoint<'a>> + Clone {
        self.fixes
            .iter()
            .zip(self.resolved)
            .map(|(fix, &resolved)| PlacedPoint { fix, resolved })
    }

    pub fn positions(self) -> impl ExactSizeIterator<Item = (Latitude, Longitude)> + Clone {
        self.resolved.iter().map(|resolved| resolved.coordinates())
    }
}
