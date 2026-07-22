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

/// Width of the left stats/filter column's label area: the checkbox, the
/// colour swatch and the constellation name. The count columns are added to
/// this, so a wider seen column widens the whole column rather than crowding
/// the names.
const STATS_LABEL_WIDTH_PX: f32 = 100.0;

/// Gap between the stats column and the plot.
const COLUMN_GAP_PX: f32 = 12.0;

/// Smallest the trails plot shrinks to as the window resizes, so it stays
/// legible even in a small window.
const MIN_PLOT_DIAMETER_PX: f32 = 240.0;

/// First-frame estimate of the vertical space below the plot (the transport
/// and the gap above it). Only ever used before the real height has been
/// measured; every later frame sizes the plot against the measurement, so an
/// inexact seed costs one frame of settling rather than a permanently wrong
/// layout.
const TRANSPORT_RESERVE_PX: f32 = 78.0;

/// Fixed width of a count column (fix, seen), sized for the column heading and
/// a couple of digits. Every count is right-aligned in one of these, so the
/// digits line up down the column and across rows whatever their width.
const STATS_COUNT_COL_PX: f32 = 38.0;

/// Fixed width of the parenthesised unfiltered-total column, present only while
/// the not-in-fix filter hides satellites. Reserved on every row of a filtered
/// column, so the seen numbers stay in a line whether or not their own row
/// hides anything.
const STATS_PAREN_COL_PX: f32 = 34.0;

/// Right-hand padding inside a count cell, so a digit does not sit flush
/// against the next column.
const STATS_CELL_PAD_PX: f32 = 2.0;

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

/// The per-frame time step is clamped to this many seconds, so a stall (the
/// window occluded, a breakpoint) doesn't jump the scrubber across the whole
/// track on the next frame.
const MAX_PLAYBACK_FRAME_SECS: f32 = 0.1;

/// How far one press of an arrow key seeks. One second is one report on a 1 Hz
/// recording, so a tap steps report by report.
const SEEK_TAP_SECS: f64 = 1.0;

/// How long an arrow key must be held before seeking turns from single steps
/// into a continuous sweep, so a tap stays a tap.
const HOLD_BEFORE_FAST_SEEK_SECS: f64 = 0.35;

/// Held-down seeking crosses this fraction of the track per real second, so a
/// sweep takes about the same time whether the recording ran for a minute or
/// for a day.
const HELD_SEEK_TRACK_FRACTION_PER_SEC: f64 = 0.25;

/// Floor for the held-down seek rate, so seeking a very short track is still
/// quicker than playing it.
const MIN_HELD_SEEK_SECS_PER_SEC: f64 = 10.0;

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
    /// Whether the whole-track trails are drawn, or only the current-instant
    /// snapshot markers. Set on [`SkyTrailsWindow::open`] (the `Default` bool is
    /// the wrong value), so it is always initialized before the window shows.
    show_trails: bool,
    /// Whether the trails are trimmed to the stretches where the satellite was
    /// in the fix, and the current-instant marker of one merely tracked right
    /// now is hidden. See [`gt_sky::SkyTrailsPlot::in_fix_now`].
    in_fix_now: bool,
    /// Whether the current-instant signal heat field is drawn beneath the
    /// trails.
    show_heatmap: bool,
    /// How strongly the trails are drawn, as the opacity field's percentage.
    /// Unlike the other view toggles this is a persisted preference, seeded from
    /// settings via [`SkyTrailsWindow::set_trail_opacity_percent`] and left alone
    /// by `open_track`, so it carries across tracks and restarts rather than
    /// resetting.
    trail_opacity_percent: f32,
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

    /// The trails' opacity percentage, for the app to persist to settings.
    pub fn trail_opacity_percent(&self) -> f32 {
        self.trail_opacity_percent
    }

    /// Seed the trails' opacity percentage from persisted settings, clamped to
    /// the valid range. The app calls this at startup so the window opens at the
    /// last-used strength.
    pub fn set_trail_opacity_percent(&mut self, percent: f32) {
        self.trail_opacity_percent = percent.clamp(
            gt_sky::TRAIL_OPACITY_PERCENT_MIN,
            gt_sky::TRAIL_OPACITY_PERCENT_MAX,
        );
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
            self.show_trails = true;
            self.in_fix_now = false;
            self.show_heatmap = false;
            // `trail_opacity_percent` is a persisted preference, so it is
            // deliberately not reset here.
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
            show_trails: &mut self.show_trails,
            in_fix_now: &mut self.in_fix_now,
            show_heatmap: &mut self.show_heatmap,
            trail_opacity_percent: &mut self.trail_opacity_percent,
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
    show_trails: &'a mut bool,
    in_fix_now: &'a mut bool,
    show_heatmap: &'a mut bool,
    trail_opacity_percent: &'a mut f32,
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
            show_trails,
            in_fix_now,
            show_heatmap,
            trail_opacity_percent,
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

        // The arrow-key transport (seek, speed) only fires while the pointer is
        // over the window, so it never hijacks the arrows from the rest of the
        // app. Spacebar is not gated this way - it toggles play/pause wherever
        // the pointer is (handled in `transport`). Tested against the full
        // content rect - `ui`'s min_rect is still empty this early in the
        // layout, so `ui_contains_pointer` would miss.
        let window_hovered = ui.rect_contains_pointer(ui.max_rect());

        // Advance playback, then clamp - so a paused scrubber and a finished
        // playback both settle inside the track's span.
        // Read once and handed to both: `frame_dt` resets the clock it reads,
        // so calling it twice in a frame would leave the second caller with a
        // zero delta and no movement at all.
        let dt = frame_dt(ui);
        advance_playback(ui, playing, *speed, scrub_secs, total_secs, dt);
        // Before the plot is laid out, so a seek shows on the same frame it is
        // pressed rather than the next one.
        handle_seek_keys(ui, window_hovered, total_secs, scrub_secs, speed, dt);
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
        // Computed before layout: the counts decide how wide the seen column
        // has to be, and that in turn sets the stats column and the plot.
        let counts = trails.counts_at(stats_time, *show_not_in_fix);
        let has_paren = any_row_is_filtered(&counts);
        let stats_width = stats_col_width(has_paren);

        // Size the plot to the space left after the stats column and the
        // transport, so resizing the window grows the plot rather than a gap.
        // The reserve is measured from the previous frame and cached: egui's
        // window only ever grows to fit content, so a reserve that undershoots
        // what is really laid out below the plot makes the window creep taller
        // every frame, and the taller window then grows the plot on the next
        // one. The measurement therefore has to span *everything* below the
        // plot row - the transport and the spacing egui inserts above it - not
        // just the transport's own height.
        let reserve_id = ui.id().with("below_plot_height");
        let reserved = ui
            .data(|d| d.get_temp::<f32>(reserve_id))
            .unwrap_or(TRANSPORT_RESERVE_PX);
        let diameter = plot_diameter(ui.available_size(), stats_width, reserved);

        let mut focus = None;
        ui.horizontal_top(|ui| {
            // Stats/filter on the left, computed before the plot so hover
            // focus lands on the same frame.
            ui.allocate_ui_with_layout(
                egui::vec2(stats_width, diameter),
                Layout::top_down(Align::Min),
                |ui| {
                    ui.set_width(stats_width);
                    focus = constellation_stats(ui, &counts, shown, has_paren);
                    ui.add_space(6.0);
                    view_toggles(
                        ui,
                        ViewToggles {
                            show_trails,
                            trail_opacity_percent,
                            show_heatmap,
                            in_fix_now,
                            show_not_in_fix,
                        },
                    );
                },
            );
            ui.add_space(COLUMN_GAP_PX);
            SkyTrailsPlot::new(trails, diameter)
                .shown(*shown)
                .focus(focus)
                .scrub(Some(scrub_time))
                .show_not_in_fix(*show_not_in_fix)
                .show_trails(*show_trails)
                .in_fix_now(*in_fix_now)
                .show_heatmap(*show_heatmap)
                .trail_opacity(gt_sky::trail_opacity_multiplier(*trail_opacity_percent))
                .with_elevation_mask_deg(elevation_mask_deg)
                .ui(ui);
        });
        // Measured across the gap as well as the transport, so next frame's
        // plot is sized against the space the content actually occupies.
        let plot_bottom = ui.min_rect().bottom();
        transport(ui, first, total_secs, scrub_secs, playing, speed);
        let below_plot = ui.min_rect().bottom() - plot_bottom;
        ui.data_mut(|d| d.insert_temp(reserve_id, below_plot));
    }
}

/// The window's view toggles, borrowed from the [`WindowBody`] state so the
/// checkboxes write straight back. Grouped into a struct to keep them off the
/// helper's parameter list, which would otherwise be a row of bare booleans.
struct ViewToggles<'a> {
    show_trails: &'a mut bool,
    trail_opacity_percent: &'a mut f32,
    show_heatmap: &'a mut bool,
    in_fix_now: &'a mut bool,
    show_not_in_fix: &'a mut bool,
}

/// The view toggles below the stats: what is drawn (the whole-track trails and
/// how strongly, and the current-instant signal heat field) above which
/// satellites are kept (only those in the fix, and the whole-track not-in-fix
/// filter).
fn view_toggles(ui: &mut egui::Ui, toggles: ViewToggles<'_>) {
    let ViewToggles {
        show_trails,
        trail_opacity_percent,
        show_heatmap,
        in_fix_now,
        show_not_in_fix,
    } = toggles;
    ui.checkbox(show_trails, "Trails").on_hover_text(
        "Draw each satellite's whole path across the sky. Off shows only where \
         they are at the current instant.",
    );
    // A compact percentage field rather than a slider, which would eat a row's
    // width. Directly under the checkbox it qualifies, and indented so it reads
    // as belonging to it rather than as a fifth independent control. Grayed out
    // with the trails off - there is nothing for it to act on - rather than
    // disappearing and reflowing the column.
    ui.indent("trail_opacity", |ui| {
        ui.add_enabled_ui(*show_trails, |ui| {
            ui.horizontal(|ui| {
                ui.label("Opacity");
                ui.add(
                    egui::DragValue::new(trail_opacity_percent)
                        .range(
                            gt_sky::TRAIL_OPACITY_PERCENT_MIN..=gt_sky::TRAIL_OPACITY_PERCENT_MAX,
                        )
                        .speed(0.5)
                        .fixed_decimals(0)
                        .suffix("%"),
                )
                .on_hover_text("How strongly the trails are drawn.")
                .on_disabled_hover_text("Turn the trails on to change their opacity.");
            });
        });
    });
    ui.checkbox(show_heatmap, "Signal heatmap").on_hover_text(
        "Glow where the fix satellites are right now, brighter with stronger signal.",
    );
    ui.checkbox(in_fix_now, "In fix only").on_hover_text(
        "Keep only the parts of each trail where the satellite was used in the \
         fix, and hide the current marker of one that is not right now.",
    );
    // The help cursor is the real "hover me for an explanation" cue; the
    // underlined term is the static hint.
    ui.checkbox(show_not_in_fix, not_in_fix_label(ui))
        .on_hover_cursor(egui::CursorIcon::Help)
        .on_hover_text("Satellites seen but never used in a fix over this track");
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
fn plot_diameter(avail: egui::Vec2, stats_width: f32, reserved: f32) -> f32 {
    (avail.x - stats_width - COLUMN_GAP_PX)
        .min(avail.y - reserved)
        .max(MIN_PLOT_DIAMETER_PX)
}

/// The stats column's total width: the label area plus the two count columns,
/// plus the parenthetical column while the not-in-fix filter is hiding
/// satellites.
fn stats_col_width(has_paren: bool) -> f32 {
    STATS_LABEL_WIDTH_PX
        + STATS_COUNT_COL_PX * 2.0
        + if has_paren { STATS_PAREN_COL_PX } else { 0.0 }
}

/// Whether any row's seen count is being cut by the not-in-fix filter, so the
/// whole column reserves the parenthetical slot.
fn any_row_is_filtered(counts: &[EpochCount]) -> bool {
    counts.iter().copied().any(EpochCount::is_filtered)
}

/// The stats/filter column: one row per constellation showing its live fix and
/// seen counts at the scrubbed instant, a total row, and doubling as the
/// show/hide toggle (checkbox) and hover-to-focus target. Returns the
/// constellation whose row is hovered, to focus on the plot.
fn constellation_stats(
    ui: &mut egui::Ui,
    counts: &[EpochCount],
    shown: &mut ConstellationSet,
    has_paren: bool,
) -> Option<Constellation> {
    stats_header(ui, has_paren);
    let mut focus = None;
    for count in counts {
        if stats_row(ui, shown, count, has_paren) {
            focus = Some(count.constellation);
        }
    }
    stats_total_row(ui, counts, has_paren);
    focus
}

/// The stats column header: a label plus the right-aligned Fix / Seen columns.
fn stats_header(ui: &mut egui::Ui, has_paren: bool) {
    let heading = |ui: &mut egui::Ui, width: f32, text: &str| {
        let font = egui::TextStyle::Small.resolve(ui.style());
        stats_cell(ui, width, text, font, ui.visuals().weak_text_color());
    };
    ui.horizontal(|ui| {
        ui.label(RichText::new("Satellites").weak().small());
        // Right to left, matching the rows: seen (with its empty parenthetical
        // slot) then fix, so each heading lands over its own column.
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            if has_paren {
                ui.add_space(STATS_PAREN_COL_PX);
            }
            heading(ui, STATS_COUNT_COL_PX, "Seen");
            heading(ui, STATS_COUNT_COL_PX, "Fix");
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
    has_paren: bool,
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
            // Right to left, so the columns anchor to the row's right edge and
            // line up regardless of the constellation name's width.
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                seen_columns(ui, count, has_paren, on);
                count_cell(ui, count.fix, fix_count_color, on);
            });
        })
        .response;
    crate::hover_labels::hover_affordance(ui, row.rect)
}

/// The seen count and, while the not-in-fix filter is on, the unfiltered total
/// in parentheses beside it.
///
/// Without the total the count silently drops when the filter engages, which
/// reads as satellites leaving the sky rather than the view. The parenthetical
/// gets its own fixed column, reserved on every row of a filtered table (empty
/// on rows that hide nothing), so the seen numbers stay in one line. Added
/// right to left, so the parenthetical sits to the right of the number.
fn seen_columns(ui: &mut egui::Ui, count: &EpochCount, has_paren: bool, on: bool) {
    let hidden = count
        .seen_unfiltered
        .checked_sub(count.seen)
        .filter(|h| *h > 0);
    if has_paren {
        let text = hidden.map(|_| format!("({})", count.seen_unfiltered));
        let font = count_font(ui);
        let cell = stats_cell(
            ui,
            STATS_PAREN_COL_PX,
            text.as_deref().unwrap_or(""),
            font,
            ui.visuals().weak_text_color(),
        );
        if let Some(hidden) = hidden {
            let plural = if hidden == 1 { "" } else { "s" };
            cell.on_hover_text(format!(
                "{} of {} seen. {hidden} satellite{plural} hidden: never in the fix over this track",
                count.seen, count.seen_unfiltered
            ));
        }
    }
    count_cell(ui, count.seen, seen_count_color, on);
}

/// The total row: summed fix and seen across every constellation. The bold
/// "Total" label and the separator above set it off; its counts stay the same
/// weight and tier colour as the per-constellation rows, so the numbers read
/// as one column.
fn stats_total_row(ui: &mut egui::Ui, counts: &[EpochCount], has_paren: bool) {
    let (seen, seen_unfiltered, fix) = counts.iter().fold((0, 0, 0), |(s, u, f), c| {
        (s + c.seen, u + c.seen_unfiltered, f + c.fix)
    });
    let total = EpochCount {
        constellation: counts
            .first()
            .map_or(Constellation::Gps, |c| c.constellation),
        seen,
        seen_unfiltered,
        fix,
    };
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(RichText::new("Total").strong());
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            seen_columns(ui, &total, has_paren, true);
            count_cell(ui, fix, fix_count_color, true);
        });
    });
}

/// A count, right-aligned in a fixed-width column and coloured by the given
/// tier function (the same colouring the sticky point popup uses), or greyed
/// when the row is off.
fn count_cell(ui: &mut egui::Ui, count: usize, color: fn(u32, bool) -> egui::Color32, on: bool) {
    let tint = if on {
        color(count as u32, ui.visuals().dark_mode)
    } else {
        ui.visuals().weak_text_color()
    };
    let font = count_font(ui);
    stats_cell(ui, STATS_COUNT_COL_PX, &count.to_string(), font, tint);
}

/// The count font: the monospace family at the body text size, matching the
/// surrounding labels. (`TextStyle::Monospace` is a size of its own, usually
/// smaller, so resolving that instead shrinks the digits.)
fn count_font(ui: &egui::Ui) -> egui::FontId {
    egui::FontId::monospace(egui::TextStyle::Body.resolve(ui.style()).size)
}

/// Reserve a fixed-width cell and paint `text` right-aligned in it.
///
/// Reserving the width with [`egui::Ui::allocate_exact_size`] - rather than a
/// sized label, which centres, or a laid-out sub-`Ui`, which egui collapses to
/// its content in the layout direction - is what keeps the columns lined up
/// across rows whatever their digit count. Returns the cell's response so the
/// caller can attach a hover explanation.
fn stats_cell(
    ui: &mut egui::Ui,
    width: f32,
    text: &str,
    font: egui::FontId,
    color: egui::Color32,
) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(width, ui.spacing().interact_size.y),
        egui::Sense::hover(),
    );
    if !text.is_empty() {
        ui.painter().text(
            rect.right_center() - egui::vec2(STATS_CELL_PAD_PX, 0.0),
            egui::Align2::RIGHT_CENTER,
            text,
            font,
            color,
        );
    }
    response
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
    scrub: &mut f64,
    total: f64,
    dt: f32,
) {
    if !*playing {
        return;
    }
    (*scrub, *playing) = advanced_scrub(*scrub, speed, dt, total);
    // Keep animating next frame.
    ui.ctx().request_repaint();
}

/// Wall-clock seconds since the last frame, clamped to
/// [`MAX_PLAYBACK_FRAME_SECS`].
///
/// Reading it advances the clock, so it must be called exactly once per frame
/// and the result shared with everything that moves the scrubber over time. A
/// second call in the same frame returns zero.
fn frame_dt(ui: &egui::Ui) -> f32 {
    let now = ui.input(|i| i.time);
    let last_id = ui.id().with("playback_last_time");
    let last = ui.data(|d| d.get_temp::<f64>(last_id)).unwrap_or(now);
    ui.data_mut(|d| d.insert_temp(last_id, now));
    ((now - last) as f32).min(MAX_PLAYBACK_FRAME_SECS)
}

/// How fast a held arrow key sweeps the scrubber, in track-seconds per real
/// second.
fn held_seek_rate(total_secs: f64) -> f64 {
    (total_secs * HELD_SEEK_TRACK_FRACTION_PER_SEC).max(MIN_HELD_SEEK_SECS_PER_SEC)
}

/// The next playback speed up or down the preset ladder, saturating at its
/// ends. `steps` is positive to speed up.
fn stepped_speed(speed: f32, steps: i32) -> f32 {
    // The nearest preset, so a speed set before the ladder changed still finds
    // its place on it.
    let current = PLAYBACK_SPEEDS
        .iter()
        .enumerate()
        .min_by(|(_, a), (_, b)| (**a - speed).abs().total_cmp(&(**b - speed).abs()))
        .map_or(0, |(i, _)| i);
    let last = PLAYBACK_SPEEDS.len().saturating_sub(1);
    let next = current.saturating_add_signed(steps as isize).min(last);
    PLAYBACK_SPEEDS.get(next).copied().unwrap_or(speed)
}

/// Whether `key` was pressed by the user this frame, ignoring the operating
/// system's auto-repeat.
///
/// `key_pressed` counts repeats, and a held key produces a stream of them.
/// Counting those as taps made a held arrow stutter along at the OS repeat
/// rate and kept resetting the hold timer, so the continuous sweep engaged for
/// a moment and then never again.
fn tapped(input: &egui::InputState, key: egui::Key) -> bool {
    input.events.iter().any(|event| {
        matches!(
            event,
            egui::Event::Key {
                key: pressed_key,
                pressed: true,
                repeat: false,
                ..
            } if *pressed_key == key
        )
    })
}

/// Arrow-key transport: left/right seek, up/down step the playback speed.
///
/// A tap seeks one report; holding sweeps at [`held_seek_rate`] once the key
/// has been down past [`HOLD_BEFORE_FAST_SEEK_SECS`], so the two do not fight
/// over a quick press. Keys are ignored while a widget holds keyboard focus,
/// since egui gives the slider and the speed selector their own arrow-key
/// handling and both moving at once would double the step.
fn handle_seek_keys(
    ui: &egui::Ui,
    window_hovered: bool,
    total_secs: f64,
    scrub_secs: &mut f64,
    speed: &mut f32,
    dt: f32,
) {
    let press_id = ui.id().with("seek_press_time");
    if !window_hovered || ui.memory(|m| m.focused()).is_some() {
        ui.data_mut(|d| d.remove::<f64>(press_id));
        return;
    }
    let dt = f64::from(dt);
    let (now, left, right, tapped_left, tapped_right, faster, slower) = ui.input(|i| {
        (
            i.time,
            i.key_down(egui::Key::ArrowLeft),
            i.key_down(egui::Key::ArrowRight),
            tapped(i, egui::Key::ArrowLeft),
            tapped(i, egui::Key::ArrowRight),
            tapped(i, egui::Key::ArrowUp),
            tapped(i, egui::Key::ArrowDown),
        )
    });

    if faster != slower {
        *speed = stepped_speed(*speed, if faster { 1 } else { -1 });
    }

    // Both arrows down cancel out rather than picking a winner.
    let direction = f64::from(i8::from(right) - i8::from(left));
    if direction == 0.0 {
        ui.data_mut(|d| d.remove::<f64>(press_id));
        return;
    }
    if tapped_left || tapped_right {
        *scrub_secs += direction * SEEK_TAP_SECS;
        ui.data_mut(|d| d.insert_temp(press_id, now));
    } else if let Some(pressed_at) = ui.data(|d| d.get_temp::<f64>(press_id))
        && now - pressed_at > HOLD_BEFORE_FAST_SEEK_SECS
    {
        *scrub_secs += direction * held_seek_rate(total_secs) * dt;
    }
    *scrub_secs = scrub_secs.clamp(0.0, total_secs);
    // A held key keeps sweeping without further input events.
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
        // Space toggles play/pause the whole time the window is open, wherever
        // the pointer is. Skipped while any widget holds keyboard focus: a
        // focused play button already turns Space into a click (caught by
        // `play.clicked()`, so it is never toggled twice), and a text field
        // elsewhere in the app must keep its own spaces.
        let space =
            ui.memory(|m| m.focused()).is_none() && ui.input(|i| i.key_pressed(egui::Key::Space));
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
        ConstellationSet, DEFAULT_WINDOW_SIZE, MAX_PLAYBACK_FRAME_SECS, MIN_WINDOW_HEIGHT_PX,
        MIN_WINDOW_WIDTH_PX, MapHighlight, SEEK_TAP_SECS, SkyTrails, SkyTrailsRequest,
        SkyTrailsWindow, TrackRef, Window, WindowBody, advanced_scrub, apply_scrub_highlight,
        floor_epoch, offset_time, scrub_offset_of, track_total_secs,
    };

    /// A synthetic track: a few satellites drifting across the sky over eight
    /// report epochs.
    fn demo_trails() -> SkyTrails {
        demo_trails_with(&[])
    }

    /// A demo track spanning several minutes, so a held-key sweep has room to
    /// run without immediately hitting the end.
    fn long_demo_trails() -> SkyTrails {
        let start = DateTime::<Utc>::from_timestamp(1_748_000_000, 0).expect("valid");
        let points = (0..600)
            .map(|i| {
                let f = i as f32 / 599.0;
                let sats = vec![Satellite::new(
                    Constellation::Gps,
                    5,
                    Some(20.0 + 40.0 * f),
                    Some(40.0 + 90.0 * f),
                    Some(40.0),
                    true,
                )];
                let tpv = TimePositionVelocity::builder()
                    .time(GpsTime::from_utc(start + Duration::seconds(i)))
                    .lat(Latitude::new(55.0))
                    .lon(Longitude::new(12.0))
                    .build();
                NavPoint::new(tpv, Some(Satellites::new(None, None, sats)))
            })
            .collect();
        gt_sky::extract_trails(&gt_test_utils::loaded_track_with_points(points))
    }

    /// The demo track with the given satellites tracked but never in the fix,
    /// so the not-in-fix filter has something to hide.
    fn demo_trails_with_tracked_only() -> SkyTrails {
        demo_trails_with(&[(Constellation::Gps, 12)])
    }

    fn demo_trails_with(tracked_only: &[(Constellation, u32)]) -> SkyTrails {
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
                        let in_fix = !tracked_only.contains(&(c, prn));
                        Satellite::new(
                            c,
                            prn,
                            Some(lerp(el, f)),
                            Some(lerp(az, f)),
                            Some(40.0),
                            in_fix,
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

    /// The window must settle at a size and stay there. It used to grow by one
    /// `item_spacing` every frame, without end: the plot is sized to the space
    /// left after the transport, but the reserve measured only the transport's
    /// own height and not the gap egui inserts above it. Content therefore came
    /// out one spacing taller than the space it was sized against, egui's
    /// window grew to fit, and the larger window fed a larger plot on the next
    /// frame. A long track made it obvious, because the heavy plot keeps the UI
    /// repainting, but the creep is there for any track.
    #[test]
    fn the_window_settles_at_a_size_instead_of_growing_every_frame() {
        let trails = demo_trails();
        let mut harness = TestHarness::builder()
            .size(egui::vec2(900.0, 700.0))
            .ui(move |ui| {
                Window::new("Sky trails")
                    .resizable(true)
                    .default_size(DEFAULT_WINDOW_SIZE)
                    .min_width(MIN_WINDOW_WIDTH_PX)
                    .min_height(MIN_WINDOW_HEIGHT_PX)
                    .show(ui.ctx(), |ui| {
                        WindowBody {
                            trails: &trails,
                            scrub_secs: &mut 4.0,
                            playing: &mut false,
                            speed: &mut 60.0,
                            shown: &mut ConstellationSet::all(),
                            show_not_in_fix: &mut true,
                            show_trails: &mut true,
                            in_fix_now: &mut false,
                            show_heatmap: &mut false,
                            trail_opacity_percent: &mut { gt_sky::TRAIL_OPACITY_PERCENT_DEFAULT },
                            track_ref: track_ref(),
                            elevation_mask_deg: 10.0,
                            highlight: &mut MapHighlight::default(),
                        }
                        .ui(ui);
                    });
            });

        let window_size = |h: &TestHarness<'_>| {
            h.inner
                .ctx
                .memory(|m| m.area_rect(egui::Id::new("Sky trails")))
                .map(|r| r.size())
        };

        // A couple of frames to settle: the first sizes against the default,
        // the second against the measured transport.
        harness.run();
        harness.run();
        let settled = window_size(&harness).expect("the window is shown");

        for frame in 0..10 {
            harness.run();
            let now = window_size(&harness).expect("the window is shown");
            assert!(
                (now - settled).length() < 0.5,
                "frame {frame}: window grew from {settled:?} to {now:?}"
            );
        }
    }

    fn track_ref() -> TrackRef {
        TrackRef::new(FileIdx::new(0), TrackIdx::new(0))
    }

    fn body_snapshot(name: &str, trails: SkyTrails) {
        body_snapshot_with(name, trails, true);
    }

    fn body_snapshot_with(name: &str, trails: SkyTrails, mut show_not_in_fix: bool) {
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
                    show_not_in_fix: &mut show_not_in_fix,
                    show_trails: &mut true,
                    in_fix_now: &mut false,
                    show_heatmap: &mut false,
                    trail_opacity_percent: &mut { gt_sky::TRAIL_OPACITY_PERCENT_DEFAULT },
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
                        show_trails: &mut true,
                        in_fix_now: &mut false,
                        show_heatmap: &mut false,
                        trail_opacity_percent: &mut { gt_sky::TRAIL_OPACITY_PERCENT_DEFAULT },
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

    /// Snapshot: with the not-in-fix filter on, the seen column carries the
    /// unfiltered total in parentheses, so the count visibly drops *from*
    /// something rather than silently shrinking.
    #[test]
    fn sky_trails_window_body_filtered() {
        body_snapshot_with(
            "sky_trails_window_body_filtered",
            demo_trails_with_tracked_only(),
            false,
        );
    }

    /// Snapshot: counts that mix single and double digits across rows - the
    /// case that exposed the misalignment. Every fix number must sit under the
    /// Fix heading and every seen number under Seen, whatever their width; a
    /// two-digit seen must not shove its row's fix number out of the column.
    #[test]
    fn sky_trails_window_body_dense() {
        body_snapshot("sky_trails_window_body_dense", dense_trails());
    }

    /// Snapshot: the dense counts with the not-in-fix filter on. The parenthetical
    /// hangs off the rows that hide satellites (GPS, BeiDou) and not the one
    /// that hides none (Galileo, all in fix), while every seen number stays in
    /// its column - the "even worse" case when some rows carry a total and some
    /// do not.
    #[test]
    fn sky_trails_window_body_dense_filtered() {
        body_snapshot_with(
            "sky_trails_window_body_dense_filtered",
            dense_trails(),
            false,
        );
    }

    /// A track whose constellations carry enough satellites to reach two-digit
    /// seen counts beside single-digit ones: 6/12 GPS, 5/11 BeiDou, 8/8
    /// Galileo. Half of GPS and BeiDou are out of the fix so fix and seen
    /// differ.
    fn dense_trails() -> SkyTrails {
        let specs = [
            (Constellation::Gps, 12u32, 6u32),
            (Constellation::Beidou, 11, 5),
            (Constellation::Galileo, 8, 8),
        ];
        let start = DateTime::<Utc>::from_timestamp(1_748_000_000, 0).expect("valid");
        let sats = specs
            .into_iter()
            .flat_map(|(c, seen, fix)| {
                (0..seen).map(move |i| {
                    Satellite::new(
                        c,
                        i + 1,
                        Some(20.0 + f32::from(i as u16) * 4.0),
                        Some(f32::from(i as u16) * 30.0 % 360.0),
                        Some(40.0),
                        i < fix,
                    )
                })
            })
            .collect();
        let tpv = TimePositionVelocity::builder()
            .time(GpsTime::from_utc(start))
            .lat(Latitude::new(55.0))
            .lon(Longitude::new(12.0))
            .build();
        let point = NavPoint::new(tpv, Some(Satellites::new(None, None, sats)));
        gt_sky::extract_trails(&gt_test_utils::loaded_track_with_points(vec![point]))
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

    /// Drive the body with the pointer over the window and return the scrub
    /// position and speed after `keys` are tapped.
    fn body_after_keys(keys: &[egui::Key]) -> (f64, f32) {
        let trails = demo_trails();
        let state = std::rc::Rc::new(std::cell::Cell::new((4.0_f64, 60.0_f32)));
        let seen = state.clone();
        let mut harness = TestHarness::builder()
            .size(egui::vec2(560.0, 440.0))
            .ui(move |ui| {
                let (mut scrub, mut speed) = seen.get();
                WindowBody {
                    trails: &trails,
                    scrub_secs: &mut scrub,
                    playing: &mut false,
                    speed: &mut speed,
                    shown: &mut ConstellationSet::all(),
                    show_not_in_fix: &mut true,
                    show_trails: &mut true,
                    in_fix_now: &mut false,
                    show_heatmap: &mut false,
                    trail_opacity_percent: &mut { gt_sky::TRAIL_OPACITY_PERCENT_DEFAULT },
                    track_ref: track_ref(),
                    elevation_mask_deg: 10.0,
                    highlight: &mut MapHighlight::default(),
                }
                .ui(ui);
                seen.set((scrub, speed));
            });
        // The keys only apply while the window is hovered, like the spacebar.
        harness.inner.hover_at(egui::pos2(280.0, 200.0));
        harness.run();
        for &key in keys {
            harness.inner.key_press(key);
            harness.inner.run_steps(2);
        }
        state.get()
    }

    /// Left and right seek the scrubber a report at a time, and up and down
    /// walk the speed ladder - the arrow keys used to do nothing at all.
    #[test]
    fn the_arrow_keys_drive_the_transport() {
        let (scrub, speed) = body_after_keys(&[]);
        assert!(
            (scrub - 4.0).abs() < f64::EPSILON,
            "unchanged without input"
        );
        assert!((speed - 60.0).abs() < f32::EPSILON);

        let (scrub, _) = body_after_keys(&[egui::Key::ArrowRight]);
        assert!((scrub - 5.0).abs() < f64::EPSILON, "right seeks forward");

        let (scrub, _) = body_after_keys(&[egui::Key::ArrowLeft, egui::Key::ArrowLeft]);
        assert!((scrub - 2.0).abs() < f64::EPSILON, "left seeks back");

        let (_, speed) = body_after_keys(&[egui::Key::ArrowUp]);
        assert!(
            (speed - 120.0).abs() < f32::EPSILON,
            "up speeds playback up"
        );

        let (_, speed) = body_after_keys(&[egui::Key::ArrowDown]);
        assert!((speed - 30.0).abs() < f32::EPSILON, "down slows playback");
    }

    /// Holding an arrow sweeps the scrubber continuously, not just the one
    /// report a tap moves it.
    ///
    /// This drives real frames with wall-clock time elapsing between them,
    /// which a `key_press` (an instant down-up inside one frame) cannot do.
    /// Held seeking once silently did nothing: playback and seeking each read
    /// the per-frame clock, and the first read reset it, so the second always
    /// saw a zero delta.
    #[test]
    fn holding_an_arrow_sweeps_the_scrubber() {
        let trails = long_demo_trails();
        let total = track_total_secs(&trails).expect("has epochs");
        let state = std::rc::Rc::new(std::cell::Cell::new(0.0_f64));
        let seen = state.clone();
        let mut harness = TestHarness::builder()
            .size(egui::vec2(560.0, 440.0))
            .ui(move |ui| {
                let mut scrub = seen.get();
                WindowBody {
                    trails: &trails,
                    scrub_secs: &mut scrub,
                    playing: &mut false,
                    speed: &mut 60.0,
                    shown: &mut ConstellationSet::all(),
                    show_not_in_fix: &mut true,
                    show_trails: &mut true,
                    in_fix_now: &mut false,
                    show_heatmap: &mut false,
                    trail_opacity_percent: &mut { gt_sky::TRAIL_OPACITY_PERCENT_DEFAULT },
                    track_ref: track_ref(),
                    elevation_mask_deg: 10.0,
                    highlight: &mut MapHighlight::default(),
                }
                .ui(ui);
                seen.set(scrub);
            });
        harness.inner.hover_at(egui::pos2(280.0, 200.0));
        harness.inner.step();

        // Press and keep it down. The first frame is the tap; the sweep only
        // starts once the key has been held past the threshold.
        harness.inner.key_down(egui::Key::ArrowRight);
        // The harness advances its own clock a frame at a time, so stepping is
        // all it takes to let real time pass; enough of them to get past the
        // hold threshold and well into the sweep.
        for _ in 0..90 {
            harness.inner.step();
        }
        harness.inner.key_up(egui::Key::ArrowRight);

        let swept = state.get();
        assert!(
            swept > SEEK_TAP_SECS * 2.0,
            "holding right only moved {swept}s, no more than a tap would"
        );
        assert!(swept <= total, "the sweep must stay inside the track");
    }

    /// A held key produces a stream of operating-system auto-repeat presses.
    /// Those must not count as taps: doing so both stuttered the scrubber along
    /// at the repeat rate and kept resetting the hold timer, so the continuous
    /// sweep engaged briefly and then dropped back to repeat speed.
    #[test]
    fn auto_repeat_presses_are_not_taps() {
        let real = egui::Event::Key {
            key: egui::Key::ArrowRight,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        let auto = egui::Event::Key {
            key: egui::Key::ArrowRight,
            physical_key: None,
            pressed: true,
            repeat: true,
            modifiers: egui::Modifiers::NONE,
        };
        let tapped_with = |events: Vec<egui::Event>| {
            let mut input = egui::InputState::default();
            input.events = events;
            super::tapped(&input, egui::Key::ArrowRight)
        };

        assert!(tapped_with(vec![real.clone()]), "a real press is a tap");
        assert!(!tapped_with(vec![auto.clone()]), "auto-repeat is not a tap");
        assert!(!tapped_with(Vec::new()), "no events, no tap");
        // The initial press still registers when repeats arrive alongside it.
        assert!(tapped_with(vec![auto, real]));
    }

    /// The speed ladder steps one preset at a time and stops at its ends
    /// rather than wrapping, so holding a key does not loop from 300x back to
    /// 1x.
    #[rstest::rstest]
    #[case::up_from_default(60.0, 1, 120.0)]
    #[case::down_from_default(60.0, -1, 30.0)]
    #[case::saturates_at_the_top(300.0, 1, 300.0)]
    #[case::saturates_at_the_bottom(1.0, -1, 1.0)]
    // A speed that is not itself a preset snaps to the nearest one first.
    #[case::off_ladder(70.0, 1, 120.0)]
    fn stepped_speed_walks_the_preset_ladder(
        #[case] from: f32,
        #[case] steps: i32,
        #[case] expected: f32,
    ) {
        assert!((super::stepped_speed(from, steps) - expected).abs() < f32::EPSILON);
    }

    /// Held-down seeking crosses the track in about the same wall-clock time
    /// whatever its length, with a floor so a very short track is still quicker
    /// to seek than to play.
    #[test]
    fn held_seek_scales_with_the_track_but_has_a_floor() {
        // An hour-long track: a quarter of it per second, so ~4s end to end.
        assert!((super::held_seek_rate(3600.0) - 900.0).abs() < f64::EPSILON);
        // A ten-second track would scale to 2.5x, which is slower than most
        // playback speeds, so the floor takes over.
        assert!(
            (super::held_seek_rate(10.0) - super::MIN_HELD_SEEK_SECS_PER_SEC).abs() < f64::EPSILON
        );
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

    /// Spacebar toggles play/pause whenever the window is open, wherever the
    /// pointer is - the press below lands with the pointer parked well outside
    /// the window, and it still toggles. A single net toggle per press also
    /// proves it is not double-counted: were both the play button's click path
    /// and the window's space path to fire in one frame, the state would flip
    /// twice and land back where it started.
    #[test]
    fn spacebar_toggles_play_regardless_of_hover() {
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
                        show_trails: &mut true,
                        in_fix_now: &mut false,
                        show_heatmap: &mut false,
                        trail_opacity_percent: &mut { gt_sky::TRAIL_OPACITY_PERCENT_DEFAULT },
                        track_ref: track_ref(),
                        elevation_mask_deg: 10.0,
                        highlight: &mut MapHighlight::default(),
                    }
                    .ui(ui);
                },
                false,
            );
        harness.run();

        // Press space with the pointer parked outside the window, so the toggle
        // cannot be attributed to hover.
        let press_space = |harness: &mut TestHarness<'_, bool>| {
            let key_event = |pressed| egui::Event::Key {
                key: egui::Key::Space,
                physical_key: None,
                pressed,
                repeat: false,
                modifiers: egui::Modifiers::NONE,
            };
            let events = &mut harness.inner.input_mut().events;
            events.push(egui::Event::PointerMoved(egui::pos2(-100.0, -100.0)));
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
                        show_trails: &mut true,
                        in_fix_now: &mut false,
                        show_heatmap: &mut false,
                        trail_opacity_percent: &mut { gt_sky::TRAIL_OPACITY_PERCENT_DEFAULT },
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

        // Reaching the end clamps to the total and stops.
        let (secs, playing) = advanced_scrub(98.0, 60.0, 0.1, 100.0);
        assert!((secs - 100.0).abs() < 1e-4);
        assert!(!playing);
    }

    /// A stall - the window occluded, a breakpoint - must not leap the
    /// scrubber across the track on the frame that follows it. The clamp lives
    /// in `frame_dt`, so this drives the real body with a jump in wall-clock
    /// time rather than calling the arithmetic directly.
    #[test]
    fn a_stalled_frame_does_not_leap_the_scrubber() {
        let trails = demo_trails();
        let state = std::rc::Rc::new(std::cell::Cell::new(0.0_f64));
        let seen = state.clone();
        let mut harness = TestHarness::builder()
            .size(egui::vec2(560.0, 440.0))
            .ui(move |ui| {
                let mut scrub = seen.get();
                WindowBody {
                    trails: &trails,
                    scrub_secs: &mut scrub,
                    playing: &mut true,
                    speed: &mut 60.0,
                    shown: &mut ConstellationSet::all(),
                    show_not_in_fix: &mut true,
                    show_trails: &mut true,
                    in_fix_now: &mut false,
                    show_heatmap: &mut false,
                    trail_opacity_percent: &mut { gt_sky::TRAIL_OPACITY_PERCENT_DEFAULT },
                    track_ref: track_ref(),
                    elevation_mask_deg: 10.0,
                    highlight: &mut MapHighlight::default(),
                }
                .ui(ui);
                seen.set(scrub);
            });
        // Playback requests a repaint every frame, so `run` never settles:
        // step one frame at a time. One to seed the clock, then a ten-second
        // stall before the next.
        harness.inner.step();
        state.set(0.0);
        let stalled_at = harness.inner.input().time.unwrap_or(0.0) + 10.0;
        harness.inner.input_mut().time = Some(stalled_at);
        harness.inner.step();

        // At 60x, an unclamped ten-second stall would advance 600 track-seconds
        // and run off the end of this eight-second track.
        let advanced = state.get();
        let ceiling = f64::from(60.0 * MAX_PLAYBACK_FRAME_SECS);
        assert!(
            advanced <= ceiling + 1e-6,
            "a stalled frame advanced {advanced}s, past the {ceiling}s clamp"
        );
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
    #[case::height_bound(egui::vec2(800.0, 500.0), false, 80.0, 420.0)]
    // Wide but short: the plot is bounded by the leftover height, not width.
    #[case::width_is_slack(egui::vec2(1200.0, 400.0), false, 80.0, 320.0)]
    // Tiny window: the plot holds its legibility floor rather than shrinking.
    #[case::clamped_to_floor(egui::vec2(300.0, 300.0), false, 80.0, 240.0)]
    // The parenthetical column (the not-in-fix filter is on) takes its space
    // from the plot, not from the constellation names.
    #[case::filtered_column(egui::vec2(600.0, 900.0), true, 80.0, 378.0)]
    fn plot_diameter_fills_the_space_above_the_transport(
        #[case] avail: egui::Vec2,
        #[case] has_paren: bool,
        #[case] reserved: f32,
        #[case] expected: f32,
    ) {
        let diameter = super::plot_diameter(avail, super::stats_col_width(has_paren), reserved);
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
