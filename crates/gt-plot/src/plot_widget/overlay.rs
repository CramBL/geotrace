//! What the plot's annotation overlays share: the plot item they paint
//! themselves with, and the placement of a marker drawn at the view's edge.

use egui::epaint::Shape;
use egui::{Color32, Ui};
use egui_plot::{PlotBounds, PlotGeometry, PlotItem, PlotItemBase, PlotTransform};

/// How far inside the plot's edge a marker drawn at that edge sits, as a
/// fraction of the visible y range.  Keeps the whole glyph on screen.
pub(super) const EDGE_INSET: f64 = 0.03;

/// Length of the line running from a marker at the edge back into the plot, in
/// points: the clock excursion marker's tail, and the backward time step
/// mark's leader.  Short on purpose: a full-height line would read as a cursor,
/// and the plot already has two of those.
pub(super) const TAIL_LENGTH: f32 = 22.0;

/// What one overlay paints over the plot.
pub(super) trait OverlayPainter {
    /// What the plot's legend entry for the overlay is drawn in, where the
    /// markers take a colour each.
    fn legend_color(&self) -> Color32;

    fn paint(&self, transform: &PlotTransform, shapes: &mut Vec<Shape>);
}

/// An [`OverlayPainter`] as a plot item that contributes nothing to the plot's
/// auto-bounds.
///
/// An overlay marker sits at the current view's edge, or comes from an archive
/// reaching past every loaded recording.  Feeding either position into the
/// auto-bounds pushes that edge further out every frame, and the view creeps
/// outward for as long as the plot stays open.  `bounds()` returns
/// [`PlotBounds::NOTHING`], as egui_plot's own `Span` does, which breaks the
/// loop at the source.
pub(super) struct OverlayItem<P> {
    base: PlotItemBase,
    painter: P,
}

impl<P> OverlayItem<P> {
    pub(super) fn new(name: &str, painter: P) -> Self {
        Self {
            base: PlotItemBase::new(name.to_owned()),
            painter,
        }
    }
}

impl<P: OverlayPainter> PlotItem for OverlayItem<P> {
    fn shapes(&self, _ui: &Ui, transform: &PlotTransform, shapes: &mut Vec<Shape>) {
        self.painter.paint(transform, shapes);
    }

    fn initialize(&mut self, _x_range: std::ops::RangeInclusive<f64>) {}

    fn color(&self) -> Color32 {
        self.painter.legend_color()
    }

    /// No geometry: each overlay hit-tests its own markers and shows a hover
    /// text egui_plot's own label cannot carry.
    fn geometry(&self) -> PlotGeometry<'_> {
        PlotGeometry::None
    }

    fn bounds(&self) -> PlotBounds {
        PlotBounds::NOTHING
    }

    fn base(&self) -> &PlotItemBase {
        &self.base
    }

    fn base_mut(&mut self) -> &mut PlotItemBase {
        &mut self.base
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct PaintsNothing;

    impl OverlayPainter for PaintsNothing {
        fn legend_color(&self) -> Color32 {
            Color32::WHITE
        }

        fn paint(&self, _transform: &PlotTransform, _shapes: &mut Vec<Shape>) {}
    }

    /// The plot must not read an overlay marker's position back into the
    /// auto-bounds: the marker is placed against the current view's edge.
    #[test]
    fn an_overlay_contributes_no_bounds() {
        let item = OverlayItem::new("Overlay", PaintsNothing);

        assert_eq!(item.bounds(), PlotBounds::NOTHING);
    }
}
