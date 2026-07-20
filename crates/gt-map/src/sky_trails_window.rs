//! The whole-track sky trails window: a floating window, opened per track
//! from the map context menu and the side panel, showing every satellite's
//! path across the sky with a time scrubber that walks the map alongside it.

use egui::{Align, Layout, RichText, Window};
use egui_phosphor::regular::PAUSE as ICON_PAUSE;
use egui_phosphor::regular::PLAY as ICON_PLAY;

use gt_sky::{EpochCount, SkyTrails, SkyTrailsPlot, TrailEpoch};
use gt_types::satellites::{Constellation, ConstellationSet};
use gt_types::{GpsTime, LoadedFile, TrackRef};
use gt_ui_types::{MapHighlight, SkyTrailsRequest};

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

/// Default playback rate: one track-minute per real second.
const DEFAULT_PLAYBACK_SPEED: f32 = 60.0;

/// Playback rates offered in the speed selector, in track-seconds per real
/// second.
const PLAYBACK_SPEEDS: [f32; 7] = [1.0, 5.0, 10.0, 30.0, 60.0, 120.0, 300.0];

/// The per-frame time step is clamped to this many seconds before advancing
/// playback, so a stall (the window occluded, a breakpoint) doesn't jump the
/// scrubber across the whole track on the next frame.
const MAX_PLAYBACK_FRAME_SECS: f32 = 0.1;

/// Glyph size of the play / pause button.
const PLAY_ICON_SIZE_PX: f32 = 16.0;

/// Width of the speed selector, sized to its widest label ("300x") rather than
/// egui's much wider default combo width.
const SPEED_SELECTOR_WIDTH_PX: f32 = 52.0;

/// Alpha of the underline under an explained term, low enough that it marks
/// the term without reading as a link.
const TERM_UNDERLINE_ALPHA: f32 = 0.5;

/// The whole-track sky trails window. Owned by the app and drawn each frame;
/// opened by [`SkyTrailsWindow::open`] from the context menus and the
/// clicked-point window.
#[derive(Default)]
pub struct SkyTrailsWindow {
    open: bool,
    track: Option<TrackRef>,
    /// Scrub position, in track-seconds from the first report epoch. Continuous
    /// so playback animates smoothly between epochs.
    scrub_secs: f64,
    /// Whether playback is advancing the scrubber.
    playing: bool,
    /// Playback rate in track-seconds per real second. Set on
    /// [`SkyTrailsWindow::open`] (the `Default` is zero, not a usable
    /// rate), so it is always initialized before the window shows.
    speed: f32,
    shown: ConstellationSet,
    /// Whether trails for satellites never in the fix are drawn. Set on
    /// [`SkyTrailsWindow::open`] (the `Default` bool is the wrong value),
    /// so it is always initialized before the window shows.
    show_not_in_fix: bool,
    /// An instant the window was asked to open at, applied on the next
    /// `show` once the trails - and so the track's time span - are known.
    /// Requests arrive before the trails are extracted, so the scrub position
    /// cannot be resolved at request time.
    pending_scrub_to: Option<GpsTime>,
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

    /// Open the window per `request`: on its track, and scrubbed to its
    /// instant when it carries one (opened from a clicked track point) rather
    /// than at the start.
    ///
    /// Re-opening the same track keeps the scrubber, filter and playback speed
    /// where they were, so jumping to a new instant does not reset the rest of
    /// the window.
    pub fn open(&mut self, request: SkyTrailsRequest) {
        self.open_track(request.track);
        if let Some(at) = request.at {
            self.pending_scrub_to = Some(at);
            // Landing on a moment is an inspection, not a playthrough.
            self.playing = false;
        }
    }

    /// Open the window on `track`, resetting the scrubber and filter when it
    /// is a different track than before.
    fn open_track(&mut self, track: TrackRef) {
        self.open = true;
        if self.track != Some(track) {
            self.track = Some(track);
            self.scrub_secs = 0.0;
            self.playing = false;
            self.speed = DEFAULT_PLAYBACK_SPEED;
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

        // Resolve a requested instant now that the track's span is known.
        if let Some(at) = self.pending_scrub_to.take()
            && let (Some(first), Some(total_secs)) = (
                trails.epochs.first().map(|e| e.time),
                track_total_secs(trails),
            )
        {
            self.scrub_secs = scrub_offset_of(first, at, total_secs);
        }

        let body = WindowBody {
            trails,
            scrub_secs: &mut self.scrub_secs,
            playing: &mut self.playing,
            speed: &mut self.speed,
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
    scrub_secs: &'a mut f64,
    playing: &'a mut bool,
    speed: &'a mut f32,
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
            scrub_secs,
            playing,
            speed,
            shown,
            show_not_in_fix,
            track_ref,
            elevation_mask_deg,
            highlight,
        } = self;
        let (Some(first), Some(total_secs)) = (
            trails.epochs.first().map(|e| e.time),
            track_total_secs(trails),
        ) else {
            ui.label("This track has no satellite reports");
            return;
        };

        // Spacebar toggles play/pause while the pointer is anywhere over the
        // window (not just the transport), so watching the plot and hitting
        // space works. Tested against the full content rect - `ui`'s min_rect
        // is still empty this early in the layout, so `ui_contains_pointer`
        // would miss.
        let window_hovered = ui.rect_contains_pointer(ui.max_rect());

        // Advance playback, then clamp - so a paused scrubber and a finished
        // playback both settle inside the track's span.
        advance_playback(ui, playing, *speed, scrub_secs, total_secs);
        *scrub_secs = scrub_secs.clamp(0.0, total_secs);
        let scrub_time = offset_time(first, *scrub_secs);

        // The map point for the current instant stays highlighted the whole
        // time the window is open - while playing, dragging, or parked - not
        // only mid-drag. The app writes the time-series plot's own hover into
        // this same channel before `show()` runs, so defer to it when the plot
        // is actively hovering a point: the trails window only fills the
        // channel when the plot left it empty.
        if highlight.plot_hover_point.is_none()
            && let Some(epoch) = floor_epoch(trails, scrub_time)
        {
            apply_scrub_highlight(highlight, track_ref, epoch);
        }
        // Stats and counts reflect the report in effect at this instant.
        let stats_time = floor_epoch(trails, scrub_time).map_or(first, |e| e.time);

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
                    focus = constellation_stats(ui, trails, shown, stats_time, *show_not_in_fix);
                    ui.add_space(6.0);
                    // The help cursor is the real "hover me for an
                    // explanation" cue; the underlined term is the static hint.
                    ui.checkbox(show_not_in_fix, not_in_fix_label(ui))
                        .on_hover_cursor(egui::CursorIcon::Help)
                        .on_hover_text("Satellites seen but never used in a fix over this track");
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
            .scope(|ui| {
                transport(
                    ui,
                    first,
                    total_secs,
                    scrub_secs,
                    playing,
                    speed,
                    window_hovered,
                )
            })
            .response
            .rect;
        ui.data_mut(|d| d.insert_temp(reserve_id, transport_rect.height()));
    }
}

/// The "Show *not in fix*" checkbox label. The GNSS term is italic and carries
/// a faint underline - the "there is a definition here" marker - so it reads
/// as a term of art with an explanation behind it. The underline is kept weak
/// so it does not read as a link, and the caller pairs it with the help cursor,
/// which is what actually signals "hover me".
fn not_in_fix_label(ui: &egui::Ui) -> egui::text::LayoutJob {
    let font = egui::TextStyle::Body.resolve(ui.style());
    let color = ui.visuals().text_color();
    let mut job = egui::text::LayoutJob::default();
    job.append(
        "Show ",
        0.0,
        egui::TextFormat {
            font_id: font.clone(),
            color,
            ..Default::default()
        },
    );
    job.append(
        "not in fix",
        0.0,
        egui::TextFormat {
            font_id: font,
            color,
            italics: true,
            underline: egui::Stroke::new(1.0, color.gamma_multiply(TERM_UNDERLINE_ALPHA)),
            ..Default::default()
        },
    );
    job
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

/// The track's span in seconds, or `None` when it has no report epochs. Zero
/// for a single-report track. Reads the span [`SkyTrails`] already computed,
/// so the window and the plot's time ramp cannot diverge.
fn track_total_secs(trails: &SkyTrails) -> Option<f64> {
    let range = trails.time_range?;
    Some(secs_between(range.start, range.end))
}

/// Seconds from `first` to `later` (may be zero).
fn secs_between(first: GpsTime, later: GpsTime) -> f64 {
    later.signed_duration_since(first).num_milliseconds() as f64 / 1000.0
}

/// `secs` track-seconds as a [`chrono::Duration`], rounded to the millisecond.
fn duration_from_secs(secs: f64) -> chrono::Duration {
    chrono::Duration::milliseconds((secs * 1000.0).round() as i64)
}

/// The time `secs` track-seconds after `first`.
fn offset_time(first: GpsTime, secs: f64) -> GpsTime {
    GpsTime::from_utc(first.utc() + duration_from_secs(secs))
}

/// The last report epoch at or before `time` - the report in effect at the
/// scrubbed instant, driving the stats and the map highlight.
fn floor_epoch(trails: &SkyTrails, time: GpsTime) -> Option<&TrailEpoch> {
    let after = trails.epochs.partition_point(|e| e.time <= time);
    trails.epochs.get(after.saturating_sub(1))
}

/// The scrub position after one frame of playback, and whether playback keeps
/// running. It advances by the frame's elapsed time (clamped to
/// [`MAX_PLAYBACK_FRAME_SECS`]) scaled by `speed`, and stops at the end of the
/// track.
/// Where on the scrubber `at` falls, given the track's `first` epoch and span.
///
/// Clamped, so a point whose fix sits just outside the reported epochs lands on
/// the nearest end of the scrubber rather than off it.
fn scrub_offset_of(first: GpsTime, at: GpsTime, total_secs: f64) -> f64 {
    secs_between(first, at).clamp(0.0, total_secs)
}

fn advanced_scrub(scrub_secs: f64, speed: f32, dt: f32, total_secs: f64) -> (f64, bool) {
    let dt = dt.min(MAX_PLAYBACK_FRAME_SECS);
    let next = scrub_secs + f64::from(speed) * f64::from(dt);
    if next >= total_secs {
        (total_secs, false)
    } else {
        (next, true)
    }
}

/// Advance the scrubber while playing, stopping at the end of the track.
/// Requests a repaint so the animation keeps running.
///
/// The step is the wall-clock time since this ran last, not `stable_dt`, so it
/// stays correct even when a `Window`'s body runs more than once per frame (a
/// layout pass): a second run in the same frame sees a zero delta and does not
/// double-advance.
fn advance_playback(
    ui: &egui::Ui,
    playing: &mut bool,
    speed: f32,
    scrub_secs: &mut f64,
    total_secs: f64,
) {
    let now = ui.input(|i| i.time);
    let last_id = ui.id().with("playback_last_time");
    let last = ui.data(|d| d.get_temp::<f64>(last_id)).unwrap_or(now);
    ui.data_mut(|d| d.insert_temp(last_id, now));
    if !*playing {
        return;
    }
    let dt = (now - last) as f32;
    (*scrub_secs, *playing) = advanced_scrub(*scrub_secs, speed, dt, total_secs);
    // Keep animating next frame.
    ui.ctx().request_repaint();
}

/// The transport below the plot: the current offset and clock above a row of
/// play button, speed selector, and a full-width time slider, with the start,
/// total duration, and end beneath it.
fn transport(
    ui: &mut egui::Ui,
    first: GpsTime,
    total_secs: f64,
    scrub_secs: &mut f64,
    playing: &mut bool,
    speed: &mut f32,
    window_hovered: bool,
) {
    let current = offset_time(first, *scrub_secs);

    ui.horizontal(|ui| {
        ui.label(
            RichText::new(format!(
                "+{}",
                gt_fmt::format_timeline_offset(duration_from_secs(*scrub_secs))
            ))
            .monospace()
            .strong(),
        );
        ui.label(
            RichText::new(current.utc().format("%H:%M:%S").to_string())
                .monospace()
                .weak(),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            speed_selector(ui, speed);
        });
    });

    ui.horizontal(|ui| {
        let play = play_button(ui, *playing);
        // Toggle on click, on Enter/Space while the button is focused (egui
        // already reports those as a click), or on Space while the window is
        // hovered and the button is not focused (so it is never toggled twice).
        let space =
            window_hovered && !play.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Space));
        if play.clicked() || space {
            *playing = !*playing;
            // Starting from the end replays from the top.
            if *playing && *scrub_secs >= total_secs {
                *scrub_secs = 0.0;
            }
        }

        ui.spacing_mut().slider_width = ui.available_width();
        let slider = ui.add(
            egui::Slider::new(scrub_secs, 0.0..=total_secs.max(f64::EPSILON)).show_value(false),
        );
        // Grabbing the slider takes over from playback.
        if slider.dragged() {
            *playing = false;
        }
    });

    // Start clock, total duration, end clock: the fixed span the slider runs
    // over, with the running position shown above it.
    let end = offset_time(first, total_secs);
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
                RichText::new(gt_fmt::format_human_terse_duration(duration_from_secs(
                    total_secs,
                )))
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
}

/// The play / pause button; returns its response for the click and focus
/// checks. Shows a pause glyph while playing, a play glyph while stopped.
fn play_button(ui: &mut egui::Ui, playing: bool) -> egui::Response {
    let glyph = if playing { ICON_PAUSE } else { ICON_PLAY };
    ui.add(egui::Button::new(
        RichText::new(glyph).size(PLAY_ICON_SIZE_PX),
    ))
    .on_hover_text(if playing { "Pause" } else { "Play" })
}

/// The playback-speed selector, cycling the [`PLAYBACK_SPEEDS`] presets. Shown
/// as e.g. "60x" - the multiple of real time the track plays at.
fn speed_selector(ui: &mut egui::Ui, speed: &mut f32) {
    egui::ComboBox::from_id_salt("sky_trails_speed")
        .width(SPEED_SELECTOR_WIDTH_PX)
        .selected_text(format!("{speed:.0}x"))
        .show_ui(ui, |ui| {
            for preset in PLAYBACK_SPEEDS {
                ui.selectable_value(speed, preset, format!("{preset:.0}x"));
            }
        })
        .response
        .on_hover_text("Playback speed - track-time played per real second");
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
        ConstellationSet, MapHighlight, SkyTrails, SkyTrailsRequest, SkyTrailsWindow, TrackRef,
        WindowBody, advanced_scrub, apply_scrub_highlight, floor_epoch, offset_time,
        scrub_offset_of, track_total_secs,
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
                    scrub_secs: &mut 4.0,
                    playing: &mut false,
                    speed: &mut 60.0,
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

    /// Run the body once at a mid-track scrub with the given starting
    /// highlight, and return the highlight afterwards.
    fn run_body_with_highlight(start: &MapHighlight) -> MapHighlight {
        let trails = demo_trails();
        let mut harness = TestHarness::builder()
            .size(egui::vec2(560.0, 440.0))
            .ui_state(
                move |ui, highlight: &mut MapHighlight| {
                    WindowBody {
                        trails: &trails,
                        scrub_secs: &mut 4.0,
                        playing: &mut false,
                        speed: &mut 60.0,
                        shown: &mut ConstellationSet::all(),
                        show_not_in_fix: &mut true,
                        track_ref: track_ref(),
                        elevation_mask_deg: 10.0,
                        highlight,
                    }
                    .ui(ui);
                },
                *start,
            );
        harness.run();
        *harness.state()
    }

    #[test]
    fn scrub_highlight_fills_the_channel_when_the_plot_left_it_empty() {
        // No plot hover this frame: the window points the map at the scrubbed
        // point (epoch 4 of the demo track).
        let highlight = run_body_with_highlight(&MapHighlight::default());
        let expected = demo_trails().epochs[4].point_index;
        assert_eq!(
            highlight.plot_hover_point,
            Some((FileIdx::new(0), TrackIdx::new(0), expected))
        );
    }

    #[test]
    fn scrub_highlight_defers_to_an_active_plot_hover() {
        // The time-series plot already claimed the channel this frame (the app
        // writes it before the window runs); the window must not clobber it.
        let plots_point = (
            FileIdx::new(1),
            TrackIdx::new(2),
            gt_types::PointIdx::new(9),
        );
        let start = MapHighlight {
            plot_hover_point: Some(plots_point),
            plot_hover_snapped: true,
            ..Default::default()
        };
        let highlight = run_body_with_highlight(&start);
        assert_eq!(highlight.plot_hover_point, Some(plots_point));
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
        window.scrub_secs = 5.0;
        window.playing = true;
        window.shown.remove(Constellation::Gps);

        // Re-opening the same track preserves the scrub position and filter.
        window.open_track(track_ref());
        assert!(window.open);
        assert!((window.scrub_secs - 5.0).abs() < f64::EPSILON);
        assert!(window.playing);
        assert!(!window.shown.contains(Constellation::Gps));

        // A different track resets them.
        let other = TrackRef::new(FileIdx::new(0), TrackIdx::new(1));
        window.open_track(other);
        assert!(window.scrub_secs.abs() < f64::EPSILON);
        assert!(!window.playing);
        assert!(window.shown.contains(Constellation::Gps));
    }

    /// Opening at an instant scrubs to it and does not start playing: landing
    /// on a moment is an inspection. Instants outside the track's epochs clamp
    /// to its ends.
    #[test]
    fn opening_at_an_instant_scrubs_to_it() {
        let trails = demo_trails();
        let first = trails.epochs[0].time;
        let total = track_total_secs(&trails).expect("has epochs");

        assert!((scrub_offset_of(first, offset_time(first, 3.0), total) - 3.0).abs() < 1e-9);
        // Before the first epoch and after the last both clamp.
        assert!(scrub_offset_of(first, offset_time(first, -5.0), total).abs() < 1e-9);
        assert!((scrub_offset_of(first, offset_time(first, 99.0), total) - total).abs() < 1e-9);

        let mut window = SkyTrailsWindow {
            playing: true,
            ..SkyTrailsWindow::default()
        };
        window.open(SkyTrailsRequest::at_instant(track_ref(), first));
        assert_eq!(window.pending_scrub_to, Some(first));
        assert!(!window.playing, "a jump to a moment pauses playback");
    }

    /// The whole-track entry points leave the scrubber alone.
    #[test]
    fn opening_the_whole_track_requests_no_scrub() {
        let mut window = SkyTrailsWindow::default();
        window.open(SkyTrailsRequest::whole_track(track_ref()));
        assert!(window.open);
        assert_eq!(window.pending_scrub_to, None);
    }

    #[test]
    fn track_total_secs_spans_first_to_last_epoch() {
        // demo_trails has eight epochs one second apart, so the span is 7s.
        assert!((track_total_secs(&demo_trails()).expect("has epochs") - 7.0).abs() < 1e-9);
        assert_eq!(track_total_secs(&SkyTrails::default()), None);
    }

    #[test]
    fn offset_time_advances_from_the_first_epoch() {
        let first = demo_trails().epochs[0].time;
        let at_three = offset_time(first, 3.0);
        assert_eq!(at_three.utc(), first.utc() + chrono::Duration::seconds(3));
    }

    /// Spacebar over the window toggles play/pause exactly once per press. A
    /// single net toggle also proves the press is not double-counted: were both
    /// the play button's click path and the window's space path to fire in one
    /// frame, the state would flip twice and land back where it started.
    #[test]
    fn spacebar_toggles_play_once_per_press() {
        let trails = demo_trails();
        let mut harness = TestHarness::builder()
            .size(egui::vec2(560.0, 440.0))
            .ui_state(
                move |ui, playing: &mut bool| {
                    WindowBody {
                        trails: &trails,
                        scrub_secs: &mut 2.0,
                        playing,
                        // Slow, so playback cannot reach the end of the short
                        // demo track between presses and pause itself.
                        speed: &mut 1.0,
                        shown: &mut ConstellationSet::all(),
                        show_not_in_fix: &mut true,
                        track_ref: track_ref(),
                        elevation_mask_deg: 10.0,
                        highlight: &mut MapHighlight::default(),
                    }
                    .ui(ui);
                },
                false,
            );
        harness.run();

        // Press space with the pointer over the window, both in the same frame
        // so the space handler is armed (it is gated on the pointer being over
        // the window).
        let press_space = |harness: &mut TestHarness<'_, bool>| {
            let key_event = |pressed| egui::Event::Key {
                key: egui::Key::Space,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            let events = &mut harness.inner.input_mut().events;
            events.push(egui::Event::PointerMoved(egui::pos2(280.0, 220.0)));
            // Down then up in the frame, so the next press registers a fresh
            // edge rather than a still-held key.
            events.push(key_event(true));
            events.push(key_event(false));
            harness.inner.step();
        };

        press_space(&mut harness);
        assert!(*harness.state(), "space should start playback");
        press_space(&mut harness);
        assert!(!*harness.state(), "space again should pause");
    }

    /// End-to-end: with playback on, stepping the harness advances the
    /// scrubber (the whole loop - input dt, `advanced_scrub`, write-back - not
    /// just the arithmetic), and it stops at the end of the track.
    #[test]
    fn playback_runs_the_scrubber_to_the_end() {
        struct State {
            secs: f64,
            playing: bool,
            speed: f32,
        }
        let trails = demo_trails();
        let mut harness = TestHarness::builder()
            .size(egui::vec2(560.0, 440.0))
            .step_dt(0.1)
            .ui_state(
                move |ui, state: &mut State| {
                    WindowBody {
                        trails: &trails,
                        scrub_secs: &mut state.secs,
                        playing: &mut state.playing,
                        speed: &mut state.speed,
                        shown: &mut ConstellationSet::all(),
                        show_not_in_fix: &mut true,
                        track_ref: track_ref(),
                        elevation_mask_deg: 10.0,
                        highlight: &mut MapHighlight::default(),
                    }
                    .ui(ui);
                },
                // 20x at 0.1s/frame is 2 track-seconds per frame over a 7s span.
                State {
                    secs: 0.0,
                    playing: true,
                    speed: 20.0,
                },
            );

        // Playback requests a repaint every frame, so `run()` steps frames
        // until it stops - i.e. runs the whole playback. It should reach the
        // end of the 7s span and pause there.
        harness.run();
        assert!((harness.state().secs - 7.0).abs() < 1e-6);
        assert!(!harness.state().playing);
    }

    #[test]
    fn advanced_scrub_runs_then_stops_at_the_end() {
        // Mid-track: advances by speed x dt (60 x 0.1 = 6s) and keeps playing.
        let (secs, playing) = advanced_scrub(10.0, 60.0, 0.1, 100.0);
        assert!((secs - 16.0).abs() < 1e-4);
        assert!(playing);

        // A long stall is clamped to MAX_PLAYBACK_FRAME_SECS (0.1s), so the
        // scrubber does not leap the whole track on the next frame.
        let (secs, _) = advanced_scrub(10.0, 60.0, 5.0, 100.0);
        assert!((secs - 16.0).abs() < 1e-4);

        // Reaching the end clamps to the total and stops.
        let (secs, playing) = advanced_scrub(98.0, 60.0, 0.1, 100.0);
        assert!((secs - 100.0).abs() < 1e-4);
        assert!(!playing);
    }

    #[test]
    fn floor_epoch_is_the_report_in_effect() {
        let trails = demo_trails();
        let first = trails.epochs[0].time;
        // Between epoch 2 (t=2s) and 3 (t=3s): the report in effect is epoch 2.
        let mid = floor_epoch(&trails, offset_time(first, 2.5)).expect("floor");
        assert_eq!(mid.time, trails.epochs[2].time);
        // Exactly on an epoch returns that epoch.
        let exact = floor_epoch(&trails, trails.epochs[4].time).expect("floor");
        assert_eq!(exact.time, trails.epochs[4].time);
        // Past the end clamps to the last epoch.
        let past = floor_epoch(&trails, offset_time(first, 99.0)).expect("floor");
        assert_eq!(past.time, trails.epochs[7].time);
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
