//! Shared fixture construction for the gt-plot integration test binaries: the
//! recordings they load, the sources `show_track_plot` reads besides them, the
//! harness that draws it once, and the pointer and tooltip helpers a hover case
//! drives it with.

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
use gt_flare::{MarkedFlare, SolarFlare};
use gt_loaded_files::RecordingNames;
use gt_plot::{ArchiveOverlays, PlotState};
use gt_test_utils::{By, HarnessInteraction as _, TestHarness};
use gt_types::{Channel, FileIdx, LoadedFile, NavPoint, TrackIdx, TrackRef};
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

/// `count` fixes `step_secs` apart from the first fix.
pub fn fixes(count: usize, step_secs: i64) -> Vec<NavPoint> {
    gt_test_utils::fixtures::nav_points_from(at_second(0), count, step_secs)
}

/// A recording of one track over `points`, carrying `channels`. Its metadata
/// has the duration of the span its fixes cover, which is what the plot's
/// x-fit reads.
pub fn recording(points: Vec<NavPoint>, channels: Vec<Channel>) -> LoadedFile {
    let mut track = gt_test_utils::loaded_track_with_points(points);
    track.metadata.duration = track.metadata.time_range.duration();
    track.channels = channels;
    gt_test_utils::loaded_file_with_tracks(vec![track])
}

/// The one track of the one recording most scenes load.
pub fn track0() -> TrackRef {
    TrackRef::new(FileIdx::new(0), TrackIdx::new(0))
}

/// One archived X2.2 flare peaking `offset_secs` after the first fix, its
/// published begin 28 minutes before the peak and its end 23 minutes after.
pub fn flare_peaking_at(offset_secs: i64) -> MarkedFlare {
    let peak = at_second(offset_secs);
    MarkedFlare {
        flare: SolarFlare {
            id: format!("{peak}-FLR-001"),
            begin: peak - TimeDelta::minutes(28),
            peak,
            end: Some(peak + TimeDelta::minutes(23)),
            classification: "X2.2".parse().expect("a published class"),
            source_location: None,
            active_region: None,
        },
        receiver_side: None,
    }
}

/// Everything `show_track_plot` reads besides the recordings and the plot's
/// own state. The default is a plot no filter narrows, over recordings no
/// archive covers and none of which was snapped, with its x bounds free to
/// re-fit to the data.
#[derive(Default)]
pub struct PlotSources {
    pub filter: GlobalFilter,
    pub snap_error: SnapErrorSeries,
    pub jamming: JammingSeries,
    pub geomagnetic: GeomagneticSeries,
    pub tec: TecSeries,
    pub context_lines: ContextLines,
    pub solar_flares: Vec<MarkedFlare>,
    /// The x bounds map-to-plot sync pins the view to, in seconds since the
    /// Unix epoch. A pinned view no longer re-fits to the data.
    pub map_sync_x_range: Option<(f64, f64)>,
}

impl PlotSources {
    /// The x bounds of a view pinned to `view`, in seconds from the first fix.
    pub fn pinned_to_map_view(mut self, view: std::ops::RangeInclusive<i64>) -> Self {
        let seconds = |offset: i64| at_second(offset).timestamp() as f64;
        self.map_sync_x_range = Some((seconds(*view.start()), seconds(*view.end())));
        self
    }
}

/// A point of the plot: `offset_secs` after the first fix, at `y` on the
/// shared value axis.
#[derive(Clone, Copy)]
pub struct PlotPosition {
    pub offset_secs: f64,
    pub y: f64,
}

/// The plot's own state and the sources it draws under, so a test can move
/// either between two frames the way the app does.
pub struct DrawnPlotState {
    pub plot: PlotState,
    pub sources: PlotSources,
}

/// A harness that has drawn the plot, and the id the plot stored the frame's
/// transform under.
pub struct DrawnPlot {
    pub harness: TestHarness<'static, DrawnPlotState>,
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
        move |ui, state: &mut DrawnPlotState| {
            written_plot_id.set(Some(
                ui.make_persistent_id(egui::Id::new(gt_plot::TRACK_PLOT_ID_SALT)),
            ));
            gt_plot::show_track_plot(
                ui,
                &files,
                &names,
                &visibility,
                &state.sources.filter,
                None,
                None,
                None,
                state.sources.map_sync_x_range,
                &state.sources.snap_error,
                &state.sources.jamming,
                &state.sources.geomagnetic,
                &state.sources.tec,
                ArchiveOverlays {
                    context_lines: &state.sources.context_lines,
                    solar_flares: &state.sources.solar_flares,
                },
                &mut state.plot,
            );
        },
        DrawnPlotState { plot, sources },
    );
    harness.run();
    DrawnPlot { harness, plot_id }
}

impl DrawnPlot {
    pub fn state(&self) -> &PlotState {
        &self.harness.state().plot
    }

    pub fn state_mut(&mut self) -> &mut PlotState {
        &mut self.harness.state_mut().plot
    }

    pub fn sources_mut(&mut self) -> &mut PlotSources {
        &mut self.harness.state_mut().sources
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
        self.harness
            .inner
            .label_texts_top_to_bottom(By::new().include_labels().role(Role::Label))
            .join("\n")
    }

    pub fn snapshot(&mut self, name: &str) {
        self.harness.snapshot_loose(name);
    }
}
