//! Sky plot rendering for the polar satellite view: north up, azimuth
//! clockwise, horizon at the rim, zenith at the center.
//!
//! Shared by the hover badge, the sticky popup, and the map's sky glyphs so a
//! satellite always projects to the same spot regardless of surface.

mod grid;
mod plot_common;
mod projection;
mod sky_plot;
pub mod style;
mod trails;
mod trails_plot;

pub use projection::{mark_position, unit_disc_position, unit_disc_radius};
pub use sky_plot::{SkyHighlight, SkyPlot, SkyPlotSize};
pub use trails::{
    EpochCount, SkyTrail, SkyTrails, SlipMark, TrailEpoch, TrailSample, extract_trails,
};
pub use trails_plot::SkyTrailsPlot;
