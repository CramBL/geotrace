//! A headless harness for the seam between the query language and map point
//! state.
//!
//! A scenario builds a synthetic dataset, drives a script of steps, and asserts
//! on what the map would show - with no egui, no worker thread, and no GPU:
//!
//! ```ignore
//! let mut scenario = MapScenario::new(Dataset::single_track(
//!     TrackSpec::from_speeds_kmh(&[5.0, 40.0, 40.0, 5.0]),
//! ));
//! scenario.run("points | where velocity > 30 km/h | draw");
//! insta::assert_snapshot!(scenario.picture(), @r"
//! track.gtd#0  .00.
//! counts: shown 4, halos 1
//! ");
//! ```
//!
//! The classification behind that picture asks real code every question it can:
//! [`gt_map::scope::point_visibility`] for whether a point is on the map and why
//! not (the same predicate the map's own hit-testing uses),
//! [`gt_ui_types::QueryMatches::draw_mask`] for its halo layers (the same call
//! the renderer makes per point), [`gt_ui_types::MatchHighlight::contains`] for
//! the hovered match, and [`gt_map::display_counts::DisplayCounts`] for the
//! aggregate the map derives independently - which
//! [`MapScenario::picture`] asserts against the per-point view every time, so a
//! divergence between the two fails every scenario.
//!
//! Not routed through real map code, because it is screen geometry rather than
//! point state: viewport culling, LOD decimation, polyline span splitting, and
//! the hover/click input gating that turns a pointer position into a candidate.
//!
//! What this crate owns is the scenario script and the rendering, and nothing
//! else: it builds no shared domain type by hand. Files and tracks come from
//! [`gt_track_builder::build_loaded_file`], points from
//! [`gt_types::tpv::TimePositionVelocity`]'s builder, the run from
//! [`gt_query_run::QuerySession`], the per-point verdict from [`gt_map::scope`].
//! A field added to a loaded recording therefore never reaches here.

mod classify;
mod dataset;
mod panel;
mod picture;
mod scenario;

pub use classify::PointClass;
pub use dataset::{Dataset, EPOCH_SECS, FileSpec, PointSpec, TrackSpec, epoch, track};
pub use gt_map::scope::PointVisibility;
pub use panel::{PanelView, RunAttempt};
pub use picture::{MapPicture, TrackPicture};
pub use scenario::MapScenario;
