//! The whole-track sky trails window: a floating window, opened per track
//! from the map context menu and the side panel, showing every satellite's
//! path across the sky with a time scrubber that walks the map alongside it.

use egui::{Align, Layout, RichText, Window};

use gt_sky::{EpochCount, SkyTrails, SkyTrailsPlot, TrailEpoch};
use gt_types::satellites::{Constellation, ConstellationSet};
use gt_types::{GpsTime, LoadedFile, TrackRef};
use gt_ui_types::MapHighlight;

use crate::tpv_renderer::{constellation_swatch, fix_count_color, seen_count_color};

/// Width of the left stats/filter column.
const STATS_COL_WIDTH_PX: f32 = 168.0;

/// Gap between the stats column and the plot.
const COLUMN_GAP_PX: f32 = 12.0;

/// Smallest the trails plot shrinks to as the window resizes, so it stays
/// legible even in a small window.
const MIN_PLOT_DIAMETER_PX: f32 = 240.0;

/// Vertical space kept below the plot for the transport (time labels, slider).
/// The plot is sized to fit above it, so resizing grows the plot, not a gap.
const TRANSPORT_RESERVE_PX: f32 = 78.0;

/// Fixed width of each stats count column, so the fix/seen numbers line up
/// across rows regardless of constellation name length.
const STATS_COUNT_WIDTH_PX: f32 = 34.0;

/// Default and minimum window size, chosen so the plot opens comfortably
/// above [`MIN_PLOT_DIAMETER_PX`] with the stats column beside it.
const DEFAULT_WINDOW_SIZE: [f32; 2] = [560.0, 420.0];
const MIN_WINDOW_WIDTH_PX: f32 = 460.0;
const MIN_WINDOW_HEIGHT_PX: f32 = 360.0;

/// The whole-track sky trails window. Owned by the app and drawn each frame;
/// opened by [`SkyTrailsWindow::open_track`] from the context menus.
#[derive(Default)]
pub struct SkyTrailsWindow {
    open: bool,
    track: Option<TrackRef>,
    scrub_index: usize,
    shown: ConstellationSet,
    /// Whether trails for satellites never in the fix are drawn. Set on
    /// [`SkyTrailsWindow::open_track`] (the `Default` bool is the wrong value),
    /// so it is always initialized before the window shows.
    show_not_in_fix: bool,
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
            self.show_not_in_fix = true;
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
            show_not_in_fix: &mut self.show_not_in_fix,
            track_ref,
            elevation_mask_deg,
            highlight,
        };
        let mut open = self.open;
        // A stable, title-derived id (like the query and history windows) so
        // the floating position persists when the window re-targets a track.
        Window::new("Sky trails")
            .open(&mut open)
            .resizable(true)
            .default_size(DEFAULT_WINDOW_SIZE)
            .min_width(MIN_WINDOW_WIDTH_PX)
            .min_height(MIN_WINDOW_HEIGHT_PX)
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
    show_not_in_fix: &'a mut bool,
    track_ref: TrackRef,
    elevation_mask_deg: f32,
    highlight: &'a mut MapHighlight,
}

impl WindowBody<'_> {
    /// The window contents: the constellation stats/filter column beside the
    /// trails plot, with the transport (time labels and slider) below.
    fn ui(self, ui: &mut egui::Ui) {
        let Self {
            trails,
            scrub_index,
            shown,
            show_not_in_fix,
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

        // Size the plot to the space left after the stats column and the
        // transport, so resizing the window grows the plot rather than a gap.
        // The transport height is measured from the previous frame and cached:
        // egui's window only ever grows to fit content, so sizing the plot
        // against a guessed reserve that undershoots the real transport would
        // make the window creep taller every frame.
        let reserve_id = ui.id().with("transport_height");
        let reserved = ui
            .data(|d| d.get_temp::<f32>(reserve_id))
            .unwrap_or(TRANSPORT_RESERVE_PX);
        let diameter = plot_diameter(ui.available_size(), reserved);

        let mut focus = None;
        ui.horizontal_top(|ui| {
            // Stats/filter on the left, computed before the plot so hover
            // focus lands on the same frame.
            ui.allocate_ui_with_layout(
                egui::vec2(STATS_COL_WIDTH_PX, diameter),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.set_width(STATS_COL_WIDTH_PX);
                    focus = constellation_stats(ui, trails, shown, scrub_time, *show_not_in_fix);
                    ui.add_space(6.0);
                    ui.checkbox(show_not_in_fix, "Show not in fix").on_hover_text(
                        "Show satellites that were tracked but never contributed to a fix over this track",
                    );
                },
            );
            ui.add_space(COLUMN_GAP_PX);
            SkyTrailsPlot::new(trails, diameter)
                .shown(*shown)
                .focus(focus)
                .scrub(Some(scrub_time))
                .show_not_in_fix(*show_not_in_fix)
                .with_elevation_mask_deg(elevation_mask_deg)
                .ui(ui);
        });
        let transport_rect = ui
            .scope(|ui| transport(ui, trails, scrub_index, track_ref, highlight))
            .response
            .rect;
        ui.data_mut(|d| d.insert_temp(reserve_id, transport_rect.height()));
    }
}

/// The plot diameter for the given available space, leaving `reserved` pixels
/// below for the transport and the stats column beside it, clamped so the plot
/// stays legible in a small window.
fn plot_diameter(avail: egui::Vec2, reserved: f32) -> f32 {
    (avail.x - STATS_COL_WIDTH_PX - COLUMN_GAP_PX)
        .min(avail.y - reserved)
        .max(MIN_PLOT_DIAMETER_PX)
}

/// The stats/filter column: one row per constellation showing its live fix and
/// seen counts at the scrubbed instant, a total row, and doubling as the
/// show/hide toggle (checkbox) and hover-to-focus target. Returns the
/// constellation whose row is hovered, to focus on the plot.
fn constellation_stats(
    ui: &mut egui::Ui,
    trails: &SkyTrails,
    shown: &mut ConstellationSet,
    scrub_time: GpsTime,
    show_not_in_fix: bool,
) -> Option<Constellation> {
    stats_header(ui);
    let dark_mode = ui.visuals().dark_mode;
    let counts = trails.counts_at(scrub_time, show_not_in_fix);
    let mut focus = None;
    for count in &counts {
        if stats_row(ui, shown, count, dark_mode) {
            focus = Some(count.constellation);
        }
    }
    stats_total_row(ui, &counts, dark_mode);
    focus
}

/// The stats column header: a label plus the right-aligned Fix / Seen columns.
fn stats_header(ui: &mut egui::Ui) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Satellites").weak().small());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            stats_number(ui, RichText::new("Seen").weak().small());
            stats_number(ui, RichText::new("Fix").weak().small());
        });
    });
}

/// One constellation row: its swatch and name, a checkbox toggle, and the live
/// fix/seen counts, colored the same way as the sticky point popup. Returns
/// whether the row is hovered (to focus the plot).
fn stats_row(
    ui: &mut egui::Ui,
    shown: &mut ConstellationSet,
    count: &EpochCount,
    dark_mode: bool,
) -> bool {
    let on = shown.contains(count.constellation);
    let row = ui
        .horizontal(|ui| {
            let mut checked = on;
            if ui.checkbox(&mut checked, "").changed() {
                shown.set(count.constellation, checked);
            }
            constellation_swatch(ui, count.constellation);
            ui.label(dimmed_if_off(count.constellation.display_name().into(), on));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                stats_number(ui, count_text(count.seen, seen_count_color, on, dark_mode));
                stats_number(ui, count_text(count.fix, fix_count_color, on, dark_mode));
            });
        })
        .response;
    crate::hover_labels::hover_affordance(ui, row.rect)
}

/// The total row: summed fix and seen across every constellation, set off by a
/// separator above it.
fn stats_total_row(ui: &mut egui::Ui, counts: &[EpochCount], dark_mode: bool) {
    let (seen, fix) = counts
        .iter()
        .fold((0, 0), |(seen, fix), c| (seen + c.seen, fix + c.fix));
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Total").strong());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            stats_number(
                ui,
                count_text(seen, seen_count_color, true, dark_mode).strong(),
            );
            stats_number(
                ui,
                count_text(fix, fix_count_color, true, dark_mode).strong(),
            );
        });
    });
}

/// A right-aligned, fixed-width numeric cell, so the columns line up.
fn stats_number(ui: &mut egui::Ui, text: RichText) {
    ui.add_sized(
        egui::vec2(STATS_COUNT_WIDTH_PX, ui.spacing().interact_size.y),
        egui::Label::new(text.monospace()),
    );
}

/// A count cell colored by the given tier function (the same coloring the
/// sticky point popup uses for fix/seen), or grayed when the row is hidden.
fn count_text(
    count: usize,
    color: fn(u32, bool) -> egui::Color32,
    on: bool,
    dark_mode: bool,
) -> RichText {
    let text = RichText::new(count.to_string());
    if on {
        text.color(color(count as u32, dark_mode))
    } else {
        text.weak()
    }
}

/// Weakens `text` when the constellation is hidden, so off rows recede.
fn dimmed_if_off(text: String, on: bool) -> RichText {
    let text = RichText::new(text);
    if on { text } else { text.weak() }
}

/// The transport below the plot: the current time and offset above a
/// full-width time slider, with the start, total duration, and end beneath it.
/// Moving the slider drops the scrubbed point into the map highlight so the
/// map and the trails move together.
fn transport(
    ui: &mut egui::Ui,
    trails: &SkyTrails,
    scrub_index: &mut usize,
    track_ref: TrackRef,
    highlight: &mut MapHighlight,
) {
    let last = trails.epochs.len() - 1;
    let (Some(first), Some(end)) = (
        trails.epochs.first().map(|e| e.time),
        trails.epochs.last().map(|e| e.time),
    ) else {
        return;
    };
    let current = trails
        .epochs
        .get(*scrub_index)
        .map_or(first, |epoch| epoch.time);
    let total = end.signed_duration_since(first);
    let offset = current.signed_duration_since(first);

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!("+{}", gt_fmt::format_timeline_offset(offset)))
                .monospace()
                .strong(),
        );
        ui.label(
            RichText::new(current.utc().format("%H:%M:%S").to_string())
                .monospace()
                .weak(),
        );
    });

    ui.spacing_mut().slider_width = ui.available_width();
    let response = ui.add(egui::Slider::new(scrub_index, 0..=last).show_value(false));

    // Start clock, total duration, end clock: the fixed span the slider runs
    // over, with the running position shown above it.
    ui.columns(3, |cols| {
        let [start_col, total_col, end_col] = cols else {
            return;
        };
        start_col.with_layout(Layout::left_to_right(Align::Center), |ui| {
            ui.label(
                RichText::new(first.utc().format("%H:%M:%S").to_string())
                    .monospace()
                    .weak(),
            );
        });
        total_col.with_layout(Layout::top_down(Align::Center), |ui| {
            ui.label(
                RichText::new(gt_fmt::format_human_terse_duration(total))
                    .weak()
                    .small(),
            );
        });
        end_col.with_layout(Layout::right_to_left(Align::Center), |ui| {
            ui.label(
                RichText::new(end.utc().format("%H:%M:%S").to_string())
                    .monospace()
                    .weak(),
            );
        });
    });

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
            .size(egui::vec2(560.0, 440.0))
            .theme(true)
            .ui(move |ui| {
                WindowBody {
                    trails: &trails,
                    scrub_index: &mut 4,
                    shown: &mut ConstellationSet::all(),
                    show_not_in_fix: &mut true,
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

    #[rstest::rstest]
    // Ample space: the plot fills the height left above the transport.
    #[case::height_bound(egui::vec2(800.0, 500.0), 80.0, 420.0)]
    // Wide but short: the plot is bounded by the leftover height, not width.
    #[case::width_is_slack(egui::vec2(1200.0, 400.0), 80.0, 320.0)]
    // Tiny window: the plot holds its legibility floor rather than shrinking.
    #[case::clamped_to_floor(egui::vec2(300.0, 300.0), 80.0, 240.0)]
    fn plot_diameter_fills_the_space_above_the_transport(
        #[case] avail: egui::Vec2,
        #[case] reserved: f32,
        #[case] expected: f32,
    ) {
        let diameter = super::plot_diameter(avail, reserved);
        assert!(
            (diameter - expected).abs() < 0.5,
            "diameter {diameter} != expected {expected}"
        );
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
