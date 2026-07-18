//! The whole-track sky trails window: a floating window, opened per track
//! from the map context menu and the side panel, showing every satellite's
//! path across the sky with a time scrubber that walks the map alongside it.

use egui::Window;

use gt_sky::{SkyTrails, SkyTrailsPlot, TrailEpoch};
use gt_types::satellites::{Constellation, ConstellationSet};
use gt_types::{LoadedFile, TrackRef};
use gt_ui_types::MapHighlight;

use crate::tpv_renderer::constellation_swatch;

/// Diameter of the trails plot inside the window.
const PLOT_DIAMETER_PX: f32 = 300.0;

/// The whole-track sky trails window. Owned by the app and drawn each frame;
/// opened by [`SkyTrailsWindow::open_track`] from the context menus.
#[derive(Default)]
pub struct SkyTrailsWindow {
    open: bool,
    track: Option<TrackRef>,
    scrub_index: usize,
    shown: ConstellationSet,
    /// The extracted trails for `track`, kept while the window stays on one
    /// track. Tracks are immutable once loaded, so a per-track cache needs no
    /// invalidation; re-extracted only when the shown track changes.
    cache: Option<(TrackRef, SkyTrails)>,
}

impl SkyTrailsWindow {
    /// Drop the window's `TrackRef`-keyed state after track indices shift
    /// (a removal or re-segmentation), so it never shows stale trails or
    /// cross-highlights the wrong point. The window closes rather than
    /// re-resolving a `TrackRef` that now addresses different data.
    pub fn invalidate(&mut self) {
        self.open = false;
        self.track = None;
        self.cache = None;
    }

    /// Open the window on `track`, resetting the scrubber and filter when it
    /// is a different track than before.
    pub fn open_track(&mut self, track: TrackRef) {
        self.open = true;
        if self.track != Some(track) {
            self.track = Some(track);
            self.scrub_index = 0;
            self.shown = ConstellationSet::all();
            self.cache = None;
        }
    }

    /// Draw the window. `highlight` receives the scrubbed point so the map
    /// cross-highlights it (the same `plot_hover_point` channel the plot uses).
    /// `elevation_mask_deg` is the app's configured analysis mask, drawn as a
    /// dashed ring like the point plot.
    pub fn show(
        &mut self,
        ctx: &egui::Context,
        files: &[LoadedFile],
        elevation_mask_deg: f32,
        highlight: &mut MapHighlight,
    ) {
        if !self.open {
            return;
        }
        let Some(track_ref) = self.track else {
            self.open = false;
            return;
        };
        let Some(track) = track_ref.resolve(files) else {
            // The track went away (file removed); close.
            self.open = false;
            return;
        };
        if self.cache.as_ref().map(|(t, _)| *t) != Some(track_ref) {
            self.cache = Some((track_ref, gt_sky::extract_trails(track)));
        }
        let Some((_, trails)) = &self.cache else {
            return;
        };

        let body = WindowBody {
            trails,
            scrub_index: &mut self.scrub_index,
            shown: &mut self.shown,
            track_ref,
            elevation_mask_deg,
            highlight,
        };
        let mut open = self.open;
        // A stable, title-derived id (like the query and history windows) so
        // the floating position persists when the window re-targets a track.
        Window::new("Sky trails")
            .open(&mut open)
            .resizable(false)
            .show(ctx, |ui| body.ui(ui));
        self.open = open;
    }
}

/// The window's per-frame render inputs: the extracted trails, the mutable
/// scrubber/filter state, and where the scrubbed point is written.
struct WindowBody<'a> {
    trails: &'a SkyTrails,
    scrub_index: &'a mut usize,
    shown: &'a mut ConstellationSet,
    track_ref: TrackRef,
    elevation_mask_deg: f32,
    highlight: &'a mut MapHighlight,
}

impl WindowBody<'_> {
    /// The window contents: the constellation filter, the trails plot, and
    /// the scrubber.
    fn ui(self, ui: &mut egui::Ui) {
        let Self {
            trails,
            scrub_index,
            shown,
            track_ref,
            elevation_mask_deg,
            highlight,
        } = self;
        if trails.epochs.is_empty() {
            ui.label("This track has no satellite reports");
            return;
        }
        let last = trails.epochs.len() - 1;
        *scrub_index = (*scrub_index).min(last);
        let Some(scrub_time) = trails.epochs.get(*scrub_index).map(|e| e.time) else {
            return;
        };

        let mut focus = None;
        ui.horizontal_top(|ui| {
            // Filter on the left, computed before the plot so hover focus
            // lands on the same frame.
            ui.vertical(|ui| focus = constellation_filter(ui, trails, shown));
            ui.add_space(8.0);
            ui.vertical(|ui| {
                SkyTrailsPlot::new(trails, PLOT_DIAMETER_PX)
                    .shown(*shown)
                    .focus(focus)
                    .scrub(Some(scrub_time))
                    .with_elevation_mask_deg(elevation_mask_deg)
                    .ui(ui);
                scrubber(ui, trails, scrub_index, track_ref, highlight);
            });
        });
    }
}

/// One checkbox per constellation present in the track, toggling its trails.
/// Returns the constellation whose row is hovered, to focus on the plot.
fn constellation_filter(
    ui: &mut egui::Ui,
    trails: &SkyTrails,
    shown: &mut ConstellationSet,
) -> Option<Constellation> {
    ui.label(egui::RichText::new("Constellations").weak().small());
    let mut focus = None;
    for constellation in trails.constellations() {
        let row = ui
            .horizontal(|ui| {
                let mut on = shown.contains(constellation);
                if ui.checkbox(&mut on, "").changed() {
                    shown.set(constellation, on);
                }
                constellation_swatch(ui, constellation);
                ui.label(constellation.display_name());
            })
            .response;
        if crate::tpv_renderer::hovering_highlight_target(ui, row.rect) {
            focus = Some(constellation);
        }
    }
    focus
}

/// The time slider. Dragging it drops the scrubbed point into the map
/// highlight so the map and the trails move together.
fn scrubber(
    ui: &mut egui::Ui,
    trails: &SkyTrails,
    scrub_index: &mut usize,
    track_ref: TrackRef,
    highlight: &mut MapHighlight,
) {
    let last = trails.epochs.len() - 1;
    if let Some(epoch) = trails.epochs.get(*scrub_index) {
        ui.label(egui::RichText::new(epoch.time.utc().format("%H:%M:%S").to_string()).monospace());
    }
    let response = ui.add(egui::Slider::new(scrub_index, 0..=last).show_value(false));
    if (response.dragged() || response.changed())
        && let Some(epoch) = trails.epochs.get(*scrub_index)
    {
        apply_scrub_highlight(highlight, track_ref, epoch);
        ui.ctx().request_repaint();
    }
}

/// Point the map's plot-hover cross-highlight at the scrubbed epoch's point,
/// so the map and the trails move together (the same channel the time-series
/// plot writes when its cursor moves).
fn apply_scrub_highlight(highlight: &mut MapHighlight, track_ref: TrackRef, epoch: &TrailEpoch) {
    highlight.plot_hover_point = Some((track_ref.fi, track_ref.index, epoch.point_index));
    highlight.plot_hover_snapped = true;
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Duration, Utc};

    use gt_test_utils::TestHarness;
    use gt_types::satellites::{Constellation, Satellite, Satellites};
    use gt_types::{
        FileIdx, GpsTime, Latitude, Longitude, NavPoint, TimePositionVelocity, TrackIdx,
    };

    use super::{
        ConstellationSet, MapHighlight, SkyTrails, SkyTrailsWindow, TrackRef, WindowBody,
        apply_scrub_highlight,
    };

    /// A synthetic track: a few satellites drifting across the sky over eight
    /// report epochs.
    fn demo_trails() -> SkyTrails {
        const EPOCHS: usize = 8;
        let specs = [
            (
                Constellation::Gps,
                5u32,
                (40.0f32, 95.0f32),
                (58.0f32, 71.0f32),
            ),
            (Constellation::Gps, 12, (85.0, 130.0), (20.0, 47.0)),
            (Constellation::Galileo, 3, (60.0, 30.0), (52.0, 40.0)),
            (Constellation::Glonass, 9, (170.0, 205.0), (48.0, 28.0)),
        ];
        let start = DateTime::<Utc>::from_timestamp(1_748_000_000, 0).expect("valid");
        let lerp = |(a, b): (f32, f32), f: f32| a + (b - a) * f;
        let points = (0..EPOCHS)
            .map(|i| {
                let f = i as f32 / (EPOCHS - 1) as f32;
                let sats = specs
                    .iter()
                    .map(|&(c, prn, az, el)| {
                        Satellite::new(
                            c,
                            prn,
                            Some(lerp(el, f)),
                            Some(lerp(az, f)),
                            Some(40.0),
                            true,
                        )
                    })
                    .collect();
                let tpv = TimePositionVelocity::builder()
                    .time(GpsTime::from_utc(start + Duration::seconds(i as i64)))
                    .lat(Latitude::new(55.0))
                    .lon(Longitude::new(12.0))
                    .build();
                NavPoint::new(tpv, Some(Satellites::new(None, None, sats)))
            })
            .collect();
        gt_sky::extract_trails(&gt_test_utils::loaded_track_with_points(points))
    }

    fn track_ref() -> TrackRef {
        TrackRef::new(FileIdx::new(0), TrackIdx::new(0))
    }

    fn body_snapshot(name: &str, trails: SkyTrails) {
        let mut harness = TestHarness::builder()
            .size(egui::vec2(470.0, 400.0))
            .theme(true)
            .ui(move |ui| {
                WindowBody {
                    trails: &trails,
                    scrub_index: &mut 4,
                    shown: &mut ConstellationSet::all(),
                    track_ref: track_ref(),
                    elevation_mask_deg: 10.0,
                    highlight: &mut MapHighlight::default(),
                }
                .ui(ui);
            });
        harness.run();
        harness.snapshot(name);
    }

    /// Snapshot: the whole window body - the constellation filter, the trails
    /// plot, and the scrubber - at a mid-track scrub position.
    #[test]
    fn sky_trails_window_body() {
        body_snapshot("sky_trails_window_body", demo_trails());
    }

    /// Snapshot: a track with no satellite reports shows the fallback line.
    #[test]
    fn sky_trails_window_empty() {
        body_snapshot("sky_trails_window_empty", SkyTrails::default());
    }

    #[test]
    fn open_track_resets_only_when_the_track_changes() {
        let mut window = SkyTrailsWindow::default();
        window.open_track(track_ref());
        window.scrub_index = 5;
        window.shown.remove(Constellation::Gps);

        // Re-opening the same track preserves the scrub position and filter.
        window.open_track(track_ref());
        assert!(window.open);
        assert_eq!(window.scrub_index, 5);
        assert!(!window.shown.contains(Constellation::Gps));

        // A different track resets both.
        let other = TrackRef::new(FileIdx::new(0), TrackIdx::new(1));
        window.open_track(other);
        assert_eq!(window.scrub_index, 0);
        assert!(window.shown.contains(Constellation::Gps));
    }

    #[test]
    fn invalidate_closes_and_drops_the_track() {
        let mut window = SkyTrailsWindow::default();
        window.open_track(track_ref());
        window.invalidate();
        assert!(!window.open);
        assert_eq!(window.track, None);
        assert!(window.cache.is_none());
    }

    #[test]
    fn scrub_highlight_points_at_the_epochs_map_point() {
        let trails = demo_trails();
        let epoch = trails.epochs[3];
        let mut highlight = MapHighlight::default();
        apply_scrub_highlight(&mut highlight, track_ref(), &epoch);
        assert_eq!(
            highlight.plot_hover_point,
            Some((FileIdx::new(0), TrackIdx::new(0), epoch.point_index))
        );
        assert!(highlight.plot_hover_snapped);
    }
}
