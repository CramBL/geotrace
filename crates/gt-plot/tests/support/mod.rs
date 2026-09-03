//! Shared fixture construction for the gt-plot integration test binaries: the
//! sources `show_track_plot` reads besides the recordings, the harness that
//! draws it once, and the pointer and tooltip helpers a hover case drives it
//! with.

#![allow(dead_code, reason = "shared across binaries with different needs")]
#![expect(
    clippy::expect_used,
    reason = "the helpers beside the tests are not covered by clippy's in-test relaxations"
)]

use std::cell::Cell;
use std::rc::Rc;

use chrono::{DateTime, TimeDelta, Utc};
use egui::accesskit::Role;
use egui_plot::{PlotPoint, PlotTransform};
use gt_filter::GlobalFilter;
use gt_flare::MarkedFlare;
use gt_loaded_files::RecordingNames;
use gt_plot::{ArchiveOverlays, PlotState};
use gt_test_utils::{By, HarnessInteraction as _, NodeT as _, Queryable as _, TestHarness};
use gt_types::LoadedFile;
use gt_ui_types::{
    ContextLines, GeomagneticSeries, JammingSeries, SnapErrorSeries, TecSeries, TrackDataVisibility,
};

/// 2024-01-15 12:00:00 UTC, the first fix of every recording the binaries
/// build.
pub const FIRST_FIX_SECS: i64 = 1_705_320_000;

/// Plot size in points. Wide enough that a chip row and a plot both lay out.
pub const PLOT_SIZE: egui::Vec2 = egui::vec2(700.0, 400.0);

/// Frames the pointer rests still for before the tooltip is read: egui opens a
/// tooltip once the pointer has stopped moving.
const SETTLE_FRAMES: usize = 3;

pub fn at_second(offset: i64) -> DateTime<Utc> {
    DateTime::UNIX_EPOCH + TimeDelta::seconds(FIRST_FIX_SECS + offset)
}

/// The area a rendered frame is compared over: the whole plot, overlays
/// included.
pub fn plot_area() -> egui::Rect {
    egui::Rect::from_min_size(egui::Pos2::ZERO, PLOT_SIZE)
}

/// Everything `show_track_plot` reads besides the recordings and the plot's
/// own state. The default is a plot no filter narrows, over recordings no
/// archive covers and none of which was snapped.
#[derive(Default)]
pub struct PlotSources {
    pub filter: GlobalFilter,
    pub snap_error: SnapErrorSeries,
    pub jamming: JammingSeries,
    pub geomagnetic: GeomagneticSeries,
    pub tec: TecSeries,
    pub context_lines: ContextLines,
    pub solar_flares: Vec<MarkedFlare>,
}

/// A point of the plot: `offset_secs` after the first fix, at `y` on the
/// shared value axis.
#[derive(Clone, Copy)]
pub struct PlotPosition {
    pub offset_secs: f64,
    pub y: f64,
}

/// A harness that has drawn the plot, and the id the plot stored the frame's
/// transform under.
pub struct DrawnPlot {
    pub harness: TestHarness<'static, PlotState>,
    plot_id: Rc<Cell<Option<egui::Id>>>,
}

/// Draw one frame of the plot over `files`, reading `sources`, with `plot` as
/// the plot's own state.
pub fn drawn_plot(files: Vec<LoadedFile>, sources: PlotSources, mut plot: PlotState) -> DrawnPlot {
    let names = RecordingNames::default();
    let visibility = TrackDataVisibility::from_loaded(&files);
    plot.rebuild_all(&files);

    let plot_id = Rc::new(Cell::new(None));
    let written_plot_id = Rc::clone(&plot_id);
    let mut harness = TestHarness::builder().size(PLOT_SIZE).ui_state(
        move |ui, plot: &mut PlotState| {
            written_plot_id.set(Some(
                ui.make_persistent_id(egui::Id::new(gt_plot::TRACK_PLOT_ID_SALT)),
            ));
            gt_plot::show_track_plot(
                ui,
                &files,
                &names,
                &visibility,
                &sources.filter,
                None,
                None,
                None,
                None,
                &sources.snap_error,
                &sources.jamming,
                &sources.geomagnetic,
                &sources.tec,
                ArchiveOverlays {
                    context_lines: &sources.context_lines,
                    solar_flares: &sources.solar_flares,
                },
                plot,
            );
        },
        plot,
    );
    harness.run();
    DrawnPlot { harness, plot_id }
}

impl DrawnPlot {
    pub fn state(&self) -> &PlotState {
        self.harness.state()
    }

    pub fn state_mut(&mut self) -> &mut PlotState {
        self.harness.state_mut()
    }

    pub fn run(&mut self) {
        self.harness.run();
    }

    /// What the plot mapped values to screen positions with on the frame it
    /// last drew.
    pub fn transform(&self) -> PlotTransform {
        let id = self.plot_id.get().expect("the plot drew once");
        egui_plot::PlotMemory::load(&self.harness.inner.ctx, id)
            .expect("the plot stored its transform")
            .transform()
    }

    pub fn screen_position(&self, at: PlotPosition) -> egui::Pos2 {
        self.transform().position_from_point(&PlotPoint::new(
            at_second(0).timestamp() as f64 + at.offset_secs,
            at.y,
        ))
    }

    pub fn hover(&mut self, target: egui::Pos2) {
        self.harness
            .inner
            .hover_at_and_settle(target, SETTLE_FRAMES);
    }

    /// The tooltip under the pointer, its lines joined top to bottom. Empty
    /// while no label is drawn.
    pub fn hover_label(&self) -> String {
        let mut lines: Vec<(f32, String)> = self
            .harness
            .inner
            .query_all(By::new().include_labels().role(Role::Label))
            .map(|node| {
                (
                    node.rect().top(),
                    node.accesskit_node().value().unwrap_or_default(),
                )
            })
            .collect();
        lines.sort_by(|left, right| left.0.total_cmp(&right.0));
        lines
            .into_iter()
            .map(|(_, text)| text)
            .collect::<Vec<_>>()
            .join("\n")
    }

    pub fn snapshot(&mut self, name: &str) {
        self.harness.snapshot_loose(name);
    }
}
