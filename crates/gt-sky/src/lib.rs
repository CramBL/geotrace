//! Sky plot rendering for the polar satellite view: north up, azimuth
//! clockwise, horizon at the rim, zenith at the center.
//!
//! Shared by the hover badge, the sticky popup, and the map's sky glyphs so a
//! satellite always projects to the same spot regardless of surface.

mod projection;
mod sky_plot;
pub mod style;

pub use projection::{mark_position, unit_disc_position, unit_disc_radius};
pub use sky_plot::{SkyHighlight, SkyPlot, SkyPlotSize};
