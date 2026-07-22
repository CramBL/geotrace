//! The whole-track sky trails plot: every satellite's path across the sky
//! over a track, drawn as a polyline that brightens around the scrubbed
//! instant, with a scrubber marker and a per-constellation filter/focus.

use egui::{Color32, Pos2, Sense, Stroke, Vec2};

use gt_types::GpsTime;
use gt_types::satellites::{Constellation, ConstellationSet, Satellite};

use crate::style;
use crate::trails::{SkyTrail, SkyTrails, SlipMark, TrailSample};
use crate::{grid, heatmap, plot_common, projection};

/// The per-frame plot geometry shared by the trail and marker painting.
#[derive(Clone, Copy)]
struct Frame {
    center: Pos2,
    radius: f32,
}

impl Frame {
    fn project(self, azimuth: f32, elevation: f32) -> Pos2 {
        self.center + projection::unit_disc_position(azimuth, elevation) * self.radius
    }
}

/// The whole-track trails plot. Reuses the point plot's grid and projection,
/// so a satellite lands in the same place here as on the per-report plot.
pub struct SkyTrailsPlot<'a> {
    trails: &'a SkyTrails,
    diameter: f32,
    /// Which constellations' trails to draw (the window's checkboxes).
    shown: ConstellationSet,
    /// The focused constellation, if any: its trails stay at full strength
    /// and the rest dim (the window's hover).
    focus: Option<Constellation>,
    /// The scrubbed time: a marker is dropped on each shown trail at this
    /// instant. `None` draws the trails without markers.
    scrub: Option<GpsTime>,
    /// When false, satellites never in the fix over the track are hidden, so
    /// only the ones that contributed a fix are drawn.
    show_not_in_fix: bool,
    /// When false, the whole-track trail polylines (and slip marks) are hidden,
    /// leaving just the current-instant markers - the immediate snapshot.
    show_trails: bool,
    /// When true, each trail is trimmed to the stretches where the satellite
    /// was in the fix - so a rarely-used satellite shows only the short arcs it
    /// contributed - and a slip on a satellite never in the fix goes with it.
    /// The trimming reads whole-track fix state, so it applies with or without a
    /// [`Self::scrub`]; the scrub only decides the marker, which is hidden for a
    /// satellite merely tracked at that instant.
    in_fix_now: bool,
    /// When true, a smooth signal-strength heat field is drawn beneath the
    /// trails from the current-instant in-fix satellites. Inert without a
    /// [`Self::scrub`].
    show_heatmap: bool,
    /// Multiplier on every trail's alpha, tail and ghost alike, so a busy sky
    /// can be quietened or turned up without changing what is drawn. `1.0` is
    /// the tuned look. Markers and slip marks keep their own strength - the
    /// control is about the paths, not the satellites. See
    /// [`style::trail_opacity_multiplier`] for the mapping from the window's
    /// percentage.
    trail_opacity: f32,
    elevation_mask_deg: Option<f32>,
}

impl<'a> SkyTrailsPlot<'a> {
    pub fn new(trails: &'a SkyTrails, diameter: f32) -> Self {
        Self {
            trails,
            diameter,
            shown: ConstellationSet::all(),
            focus: None,
            scrub: None,
            show_not_in_fix: true,
            show_trails: true,
            in_fix_now: false,
            show_heatmap: false,
            trail_opacity: 1.0,
            elevation_mask_deg: None,
        }
    }

    /// Scale the trails' alpha, `1.0` being the tuned look. Defaults to `1.0`.
    /// The window derives this from its opacity percentage with
    /// [`style::trail_opacity_multiplier`], which clamps the percentage, so a
    /// stray value can never blank the trails or overdrive them.
    pub fn trail_opacity(self, trail_opacity: f32) -> Self {
        Self {
            trail_opacity,
            ..self
        }
    }

    /// Restrict to the given constellations. Defaults to all.
    pub fn shown(self, shown: ConstellationSet) -> Self {
        Self { shown, ..self }
    }

    /// Focus one constellation, dimming the rest.
    pub fn focus(self, focus: Option<Constellation>) -> Self {
        Self { focus, ..self }
    }

    /// Drop a marker on each trail at the given time.
    pub fn scrub(self, scrub: Option<GpsTime>) -> Self {
        Self { scrub, ..self }
    }

    /// Whether to draw satellites never in the fix over the track. Defaults to
    /// true; false hides them to focus on the ones that contributed a fix.
    pub fn show_not_in_fix(self, show_not_in_fix: bool) -> Self {
        Self {
            show_not_in_fix,
            ..self
        }
    }

    /// Whether to draw the whole-track trail polylines and slip marks. Defaults
    /// to true; false leaves only the current-instant markers - the immediate
    /// snapshot of where the satellites are now.
    pub fn show_trails(self, show_trails: bool) -> Self {
        Self {
            show_trails,
            ..self
        }
    }

    /// Whether to trim each trail to the stretches where the satellite was in
    /// the fix, and hide the current-instant marker of one merely tracked right
    /// now. Defaults to false. The trimming applies with or without a
    /// [`Self::scrub`]; only the marker needs one.
    pub fn in_fix_now(self, in_fix_now: bool) -> Self {
        Self { in_fix_now, ..self }
    }

    /// Whether to draw a signal-strength heat field beneath the trails from the
    /// current-instant in-fix satellites. Defaults to false. Inert without a
    /// [`Self::scrub`] to place the satellites at.
    pub fn show_heatmap(self, show_heatmap: bool) -> Self {
        Self {
            show_heatmap,
            ..self
        }
    }

    /// Draw the elevation mask as a dashed ring.
    pub fn with_elevation_mask_deg(self, mask_deg: f32) -> Self {
        Self {
            elevation_mask_deg: Some(mask_deg),
            ..self
        }
    }

    pub fn ui(&self, ui: &mut egui::Ui) -> egui::Response {
        let (rect, response) = ui.allocate_exact_size(Vec2::splat(self.diameter), Sense::hover());
        if !ui.is_rect_visible(rect) {
            return response;
        }
        let center = rect.center();
        let radius = self.diameter / 2.0 - style::FULL_RIM_MARGIN_PX;
        // The trails plot is always full size, so the grid is fully labelled.
        grid::draw_grid(ui, center, radius, true);
        // The mask angle when the pointer is on its ring, else `None` - one
        // value that answers both "is it hovered" and "which angle to label".
        let hovered_mask_deg = self
            .elevation_mask_deg
            .zip(response.hover_pos())
            .filter(|&(mask_deg, pointer)| grid::mask_ring_hit(center, radius, mask_deg, pointer))
            .map(|(mask_deg, _)| mask_deg);
        if let Some(mask_deg) = self.elevation_mask_deg {
            grid::draw_mask_ring(ui, center, radius, mask_deg, hovered_mask_deg.is_some());
        }

        // No reports means no trails to draw. The mask ring is still drawn
        // above (context), so honor its hover even with nothing else.
        if self.trails.time_range.is_none() {
            if let Some(mask_deg) = hovered_mask_deg {
                mask_tooltip(ui, &response, mask_deg);
            }
            return response;
        }
        let frame = Frame { center, radius };
        let dark_mode = ui.visuals().dark_mode;
        let panel = ui.visuals().panel_fill;

        // The signal-strength field, beneath the trails and markers so they
        // read over it. From the current-instant in-fix satellites, so it moves
        // as the scrubber does.
        if self.show_heatmap {
            let sources = self.heat_sources(frame);
            heatmap::paint_signal_field(ui.painter(), center, radius, &sources);
        }

        // The scrub markers drawn this frame, kept for hover: each satellite at
        // the scrubbed instant, with its position matching the drawn dot.
        let mut markers: Vec<(MarkerHover, Pos2)> = Vec::new();
        for trail in &self.trails.trails {
            if !self.shown.contains(trail.constellation) {
                continue;
            }
            // Hide satellites that were never in the fix over the track, when
            // asked to. This is a whole-track judgement, so it hides the trail;
            // the current-instant "in fix only" filter below hides only the
            // marker, leaving the path in place.
            if !self.show_not_in_fix && !trail.ever_in_fix() {
                continue;
            }
            let base = gt_ui_theme::constellation_color(trail.constellation, dark_mode);
            let focus_factor = self.focus_factor(trail.constellation);
            // The trail is drawn independently of whether the satellite is in
            // the fix at the current instant. "In fix only" instead trims the
            // polyline to the stretches where the satellite was in the fix
            // (paint_trail's `in_fix_only`), so a rarely-used satellite shows
            // only the short arcs it contributed rather than a full distracting
            // sweep, without the whole trail blinking as playback crosses it.
            if self.show_trails {
                paint_trail(
                    ui,
                    frame,
                    trail,
                    TrailPaint {
                        color: base,
                        focus_factor,
                        opacity: self.trail_opacity,
                        scrub: self.scrub,
                        in_fix_only: self.in_fix_now,
                    },
                );
            }

            // The report in effect at the scrubbed instant places the marker.
            let Some((azimuth, elevation, report)) =
                self.scrub.and_then(|scrub| marker_at(trail, scrub))
            else {
                continue;
            };
            // "In fix only" drops the current-instant marker for a satellite
            // merely tracked right now, leaving its whole-track trail in place.
            if self.in_fix_now && !report.in_fix {
                continue;
            }
            let pos = frame.project(azimuth, elevation);
            paint_marker(ui, pos, base.gamma_multiply(focus_factor), panel, report);
            // Every drawn marker is a hover target. It used to be one only
            // while the scrubber sat exactly on a report, so hover died the
            // moment playback moved the scrubber off one and stayed dead
            // until something put it back exactly on a report.
            markers.push((
                MarkerHover {
                    satellite: satellite_from_sample(trail, report),
                    at: report.time,
                },
                pos,
            ));
        }

        // Slip marks on top of the trails they annotate. A slip on a hidden
        // (never-in-fix) satellite is hidden too, so the toggle never leaves an
        // orphan mark with no trail behind it.
        for slip in &self.trails.slips {
            if !self.slip_visible(slip) {
                continue;
            }
            let color = gt_ui_theme::constellation_color(slip.constellation, dark_mode)
                .gamma_multiply(self.focus_factor(slip.constellation));
            paint_slip_mark(ui, frame.project(slip.azimuth, slip.elevation), color);
        }
        // Hover precedence, most to least specific: a satellite's scrub marker
        // (the moving current-instant dot, the window's primary target), then a
        // slip mark, then the mask ring. At most one tooltip shows, so they
        // never collide.
        let pointer = response.hover_pos();
        let hovered_marker = pointer.and_then(|pointer| {
            let candidates = markers.iter().map(|(hover, pos)| (hover, *pos));
            plot_common::nearest_within(candidates, pointer, style::MARK_HOVER_RADIUS_PX)
        });
        let hovered_slip = pointer.and_then(|pointer| {
            nearest_slip(&self.trails.slips, frame, pointer, |slip| {
                self.slip_visible(slip)
            })
        });
        if let Some(satellite) = hovered_marker {
            show_marker_tooltip(ui, &response, satellite);
        } else if let Some(slip) = hovered_slip {
            show_slip_tooltip(ui, &response, slip);
        } else if let Some(mask_deg) = hovered_mask_deg {
            mask_tooltip(ui, &response, mask_deg);
        }

        response
    }

    /// The heat sources for the signal field: each shown satellite in the fix
    /// at the scrubbed instant, at its projected position, weighted by SNR.
    /// Empty without a scrub, or when nothing is in the fix right now.
    fn heat_sources(&self, frame: Frame) -> Vec<(Pos2, f32)> {
        let Some(scrub) = self.scrub else {
            return Vec::new();
        };
        self.trails
            .trails
            .iter()
            .filter(|trail| self.shown.contains(trail.constellation))
            .filter_map(|trail| {
                let (azimuth, elevation, report) = marker_at(trail, scrub)?;
                report.in_fix.then(|| {
                    (
                        frame.project(azimuth, elevation),
                        heatmap::snr_weight(report.snr),
                    )
                })
            })
            .collect()
    }

    /// The alpha for a constellation given the current focus: full when it is
    /// focused or nothing is, dimmed otherwise.
    fn focus_factor(&self, constellation: Constellation) -> f32 {
        match self.focus {
            Some(focused) if focused != constellation => style::TRAIL_DIMMED_ALPHA,
            _ => 1.0,
        }
    }

    /// Whether a slip mark is drawn (and thus hoverable). Slips annotate the
    /// whole-track trails, so they follow the trail's visibility: the trails
    /// must be shown at all, the constellation must be shown, and the satellite
    /// must have contributed a fix whenever a filter has trimmed the never-in-
    /// fix trails away - either the whole-track "not in fix" filter is hiding
    /// them, or "in fix only" has trimmed the trail down to its in-fix stretches
    /// and a never-in-fix satellite has none - so a slip never floats with no
    /// trail behind it. The mark is not faded or blinked by the scrubbed
    /// instant: a slip is a fixed whole-track event, so it stays put as playback
    /// moves.
    fn slip_visible(&self, slip: &SlipMark) -> bool {
        let trails_trimmed_to_in_fix = !self.show_not_in_fix || self.in_fix_now;
        self.show_trails
            && self.shown.contains(slip.constellation)
            && (!trails_trimmed_to_in_fix || self.slip_satellite_ever_in_fix(slip))
    }

    /// Whether the satellite behind `slip` was ever in the fix over the track.
    fn slip_satellite_ever_in_fix(&self, slip: &SlipMark) -> bool {
        self.slip_trail(slip).is_some_and(SkyTrail::ever_in_fix)
    }

    /// The trail of the satellite a slip belongs to, if it is among the trails.
    fn slip_trail(&self, slip: &SlipMark) -> Option<&SkyTrail> {
        self.trails
            .trails
            .iter()
            .find(|trail| trail.constellation == slip.constellation && trail.prn == slip.prn)
    }
}

/// How one trail is painted: its colour and the three factors that scale it -
/// the constellation focus, the user's opacity setting, and the tail fade from
/// the scrubbed instant - plus whether to trim it to its in-fix stretches.
/// Grouped so the painting helpers keep a readable signature instead of a row
/// of bare floats and bools.
#[derive(Clone, Copy)]
struct TrailPaint {
    color: Color32,
    focus_factor: f32,
    opacity: f32,
    scrub: Option<GpsTime>,
    in_fix_only: bool,
}

/// Paint one trail: its gap-free runs, each drawn as connected polylines that
/// fade into a tail behind the satellite's position at the scrubbed instant, so
/// the direction of travel reads at a glance. Broken wherever an epoch falls
/// between two samples (the satellite dropped out there), and - when
/// `in_fix_only` is set - wherever the satellite was tracked but not in the fix,
/// trimming the path to the stretches it actually contributed to.
fn paint_trail(ui: &egui::Ui, frame: Frame, trail: &SkyTrail, paint: TrailPaint) {
    let painter = ui.painter();
    for run in trail_runs(trail, frame, paint.in_fix_only) {
        paint_run(painter, &run, paint);
    }
}

/// Paint one gap-free run of a trail. The run is split into maximal stretches
/// of one quantized fade step, each drawn as a single connected polyline: a
/// polyline tessellates as one stroke with proper joins, so - unlike a string
/// of separate translucent segments - overlapping parts never stack opacity
/// where the path is dense or doubles back, and a faint stretch stays uniformly
/// faint. Adjacent steps share their boundary vertex so the line is unbroken.
fn paint_run(painter: &egui::Painter, run: &[TrailVertex], paint: TrailPaint) {
    for (points, step) in fade_stretches(run, paint.scrub) {
        let alpha = trail_stroke_alpha(step, paint.focus_factor, paint.opacity);
        painter.add(egui::Shape::line(
            points,
            Stroke::new(style::TRAIL_WIDTH_PX, paint.color.gamma_multiply(alpha)),
        ));
    }
}

/// The alpha a trail stretch draws at: its quantized fade step's alpha, scaled
/// by the constellation focus factor and the user's opacity multiplier.
///
/// Saturated at fully opaque: an opacity above the default lifts the faint parts
/// of the tail (which stay below 1 even so) while the head, already near opaque,
/// simply tops out. Letting the factor exceed 1 would instead scale the colour's
/// premultiplied channels past saturation in [`egui::Color32::gamma_multiply`]
/// and shift its hue toward white.
fn trail_stroke_alpha(step: i32, focus_factor: f32, opacity: f32) -> f32 {
    (fade_step_alpha(step) * focus_factor * opacity).min(1.0)
}

/// Split a run into maximal stretches of one quantized fade step, each a
/// polyline to draw at that step's alpha.
///
/// A segment spans two vertices that may fall on different steps, and a polyline
/// takes one stroke, so a segment has to be shaded by one of them. It takes its
/// *later* end - the one nearer the head as the tail is walked forwards.
///
/// Neither other choice works. Shading by the earlier end drags the tail one
/// segment into the future: the segment leaving the head takes the head's full
/// brightness while the ones arriving at it stay dim, which reads as the
/// emphasis sitting ahead of the satellite - the visual opposite of a tail.
/// Taking the brighter end has the same flaw, since the head is also the
/// brighter end of the segment just after it. Taking the later end leaves the
/// segment straddling the head at the floor, a gap of about
/// [`MIN_SEGMENT_PX`] that the marker dot covers.
///
/// Adjacent stretches share their boundary vertex - it ends one and begins the
/// next - so the drawn line is unbroken across a step change. A stretch left
/// with a single vertex draws no line and is dropped, so the last vertex of a
/// run never emits a zero-length polyline of its own.
fn fade_stretches(run: &[TrailVertex], scrub: Option<GpsTime>) -> Vec<(Vec<Pos2>, i32)> {
    let mut stretches = Vec::new();
    let mut points: Vec<Pos2> = Vec::new();
    let mut step = 0i32;
    for pair in run.windows(2) {
        let [a, b] = pair else {
            continue;
        };
        let segment_step = fade_step(b.time, scrub);
        if points.is_empty() {
            points.push(a.pos);
        } else if segment_step != step {
            // Close the current stretch at the shared boundary vertex, then
            // start the next one from it so the line stays continuous.
            stretches.push((std::mem::take(&mut points), step));
            points.push(a.pos);
        }
        points.push(b.pos);
        step = segment_step;
    }
    if points.len() >= 2 {
        stretches.push((points, step));
    }
    stretches
}

/// The quantized fade step for a point in time, `0` at the faint floor up to
/// [`style::TRAIL_FADE_STEPS`] at the scrubber. Grouping the trail by step and
/// drawing each as one polyline is what keeps the fade from stacking opacity;
/// [`fade_step_alpha`] turns a step back into its alpha. Signed to match the
/// float-to-int idiom used elsewhere; the value is always in `0..=STEPS`.
fn fade_step(time: GpsTime, scrub: Option<GpsTime>) -> i32 {
    (trail_fade(time, scrub) * style::TRAIL_FADE_STEPS as f32).round() as i32
}

/// The alpha for a fade step, the inverse of the quantization in [`fade_step`]:
/// [`style::TRAIL_MIN_ALPHA`] at step `0`, [`style::TRAIL_MAX_ALPHA`] at the
/// top step.
fn fade_step_alpha(step: i32) -> f32 {
    let fraction = step as f32 / style::TRAIL_FADE_STEPS as f32;
    style::TRAIL_MIN_ALPHA + (style::TRAIL_MAX_ALPHA - style::TRAIL_MIN_ALPHA) * fraction
}

/// How brightly a point in time draws, in `0..=1`, shaping the trail into a
/// comet's tail: `1` at the scrubbed instant, easing back to `0` over
/// [`style::TRAIL_TAIL_SECS`] of past travel, and flat `0` anywhere ahead of the
/// scrubber - the path the satellite has not reached yet.
///
/// Brightening only behind the satellite is what makes the tail read as a tail:
/// the bright end is the head, so the direction of travel is obvious at a glance
/// without arrows. Fading both ways would look like a tail pointing the wrong
/// way half the time.
///
/// The ease is quadratic, so the fade is gentle just behind the head and
/// steepens toward the end of the tail. Without a scrub there is no instant to
/// measure against, so the whole trail draws at full strength.
fn trail_fade(time: GpsTime, scrub: Option<GpsTime>) -> f32 {
    let Some(scrub) = scrub else {
        return 1.0;
    };
    let secs_behind = scrub.signed_duration_since(time).num_milliseconds() as f32 / 1000.0;
    if secs_behind < 0.0 {
        // Ahead of the satellite: not travelled yet, so it stays a ghost.
        return 0.0;
    }
    1.0 - (secs_behind / style::TRAIL_TAIL_SECS).min(1.0).powi(2)
}

/// How far a sample must project from the last kept one before it earns a
/// vertex of its own. Below a pixel there is nothing left to resolve, so the
/// samples in between are collapsed into the run.
const MIN_SEGMENT_PX: f32 = 1.0;

/// One kept vertex of a trail run: where it projects and when, so the fade can
/// alpha it by distance from the scrubbed instant.
#[derive(Clone, Copy, Debug, PartialEq)]
struct TrailVertex {
    pos: Pos2,
    time: GpsTime,
}

/// The gap-free runs to draw for `trail`, each a polyline of kept vertices,
/// collapsing samples that project less than [`MIN_SEGMENT_PX`] from the run's
/// current anchor. When `in_fix_only` is set, only the samples where the
/// satellite was in the fix are drawn through, so the trail is trimmed to the
/// stretches it contributed to. Runs of a single vertex are dropped - a lone
/// point draws no line.
///
/// A trail carries one sample per epoch, so a long recording produces tens of
/// thousands of them per satellite while the disc it is drawn on is only a few
/// hundred pixels across. Drawing every sample is work that scales with how
/// long the receiver ran rather than with what the plot can show, which is what
/// made the window unusable on a long track. Collapsing bounds the vertices by
/// the trail's length in pixels instead.
///
/// Gaps survive the collapse: the pair bracketing an absence always ends the
/// run, so a satellite that dropped out still shows a break rather than a line
/// drawn straight across the time it was gone. Dropping the not-in-fix samples
/// leaves the same kind of break - their epochs are skipped, so the surviving
/// neighbours are no longer consecutive - so a tracked-but-unused stretch reads
/// as a gap too. The final drawn sample always ends the last run, so a trail
/// never stops short of where it really ended.
fn trail_runs(trail: &SkyTrail, frame: Frame, in_fix_only: bool) -> Vec<Vec<TrailVertex>> {
    let vertex = |sample: &TrailSample| TrailVertex {
        pos: frame.project(sample.azimuth, sample.elevation),
        time: sample.time,
    };
    let mut runs = Vec::new();
    let mut samples = trail
        .samples
        .iter()
        .filter(|sample| !in_fix_only || sample.in_fix);
    let Some(first) = samples.next() else {
        return runs;
    };
    let mut run = vec![vertex(first)];
    // The last kept vertex, against which the next sample's distance is judged.
    let mut anchor = vertex(first);
    // The last drawn-through sample, so a gap can close the run at it and the
    // trailing collapse can reach it, even when it was collapsed sub-pixel.
    let mut prev = first;
    let mut pending = false;
    for sample in samples {
        let sample_vertex = vertex(sample);
        if !sample.epoch.follows(prev.epoch) {
            // An absence (or a trimmed not-in-fix stretch) sits between these
            // two samples. Close the run at `prev`, then restart at `sample`.
            if pending {
                run.push(vertex(prev));
            }
            close_run(&mut run, &mut runs);
            run.push(sample_vertex);
            anchor = sample_vertex;
            pending = false;
        } else if anchor.pos.distance(sample_vertex.pos) >= MIN_SEGMENT_PX {
            run.push(sample_vertex);
            anchor = sample_vertex;
            pending = false;
        } else {
            pending = true;
        }
        prev = sample;
    }
    // Whatever was collapsed at the end still has to reach the last sample.
    if pending {
        run.push(vertex(prev));
    }
    close_run(&mut run, &mut runs);
    runs
}

/// Finish the run under construction: keep it if it has a line to draw, and
/// leave it empty either way so the next run starts clean. A single vertex
/// draws nothing, so it is dropped rather than emitted as a stray point.
fn close_run(run: &mut Vec<TrailVertex>, runs: &mut Vec<Vec<TrailVertex>>) {
    if run.len() >= 2 {
        runs.push(std::mem::take(run));
    } else {
        run.clear();
    }
}

/// Whether a scrub marker draws hollow: the report in effect has the satellite
/// tracked but not contributing to the fix.
///
/// Between reports this is the last report received, matching what the stats
/// column counts at the same instant. Filling the marker there instead would
/// claim the satellite rejoined the fix on no evidence, and would disagree with
/// the counts beside it.
fn marker_is_hollow(report: &TrailSample) -> bool {
    !report.in_fix
}

/// Paint the scrub marker for one satellite. Filled when it is in the fix at
/// the scrubbed instant, hollow (an outline ring) when only tracked, so the
/// live fix state reads at a glance.
fn paint_marker(ui: &egui::Ui, pos: Pos2, color: Color32, panel: Color32, report: &TrailSample) {
    let painter = ui.painter();
    let radius = style::TRAIL_MARKER_RADIUS_PX;
    if marker_is_hollow(report) {
        // Tracked but not in the fix: a hollow ring, its centre punched out to
        // the panel colour so it reads over the trail beneath.
        painter.circle_filled(pos, radius, panel);
        painter.circle_stroke(
            pos,
            radius,
            Stroke::new(style::TRAIL_MARKER_HOLLOW_EDGE_PX, color),
        );
    } else {
        painter.circle(
            pos,
            radius,
            color,
            Stroke::new(style::TRAIL_MARKER_EDGE_PX, panel),
        );
    }
}

/// Paint a slip mark as an "×" centered on `pos`, distinct from the round
/// scrub markers and the trail lines.
fn paint_slip_mark(ui: &egui::Ui, pos: Pos2, color: Color32) {
    let r = style::SLIP_MARK_RADIUS_PX;
    let stroke = Stroke::new(style::SLIP_MARK_WIDTH_PX, color);
    let painter = ui.painter();
    painter.line_segment([pos + Vec2::new(-r, -r), pos + Vec2::new(r, r)], stroke);
    painter.line_segment([pos + Vec2::new(-r, r), pos + Vec2::new(r, -r)], stroke);
}

/// The satellite behind a scrub marker, for its hover tooltip - a report's
/// sample carries everything the tooltip needs.
fn satellite_from_sample(trail: &SkyTrail, sample: &TrailSample) -> Satellite {
    Satellite::new(
        trail.constellation,
        trail.prn.value(),
        Some(sample.elevation),
        Some(sample.azimuth),
        sample.snr.map(|snr| snr.value()),
        sample.in_fix,
    )
}

/// A scrub marker's hover payload: the satellite as of the report in effect,
/// and when that report was. The time is carried because the scrubber sits
/// between reports for most of its travel, so the tooltip names the report the
/// values actually came from.
struct MarkerHover {
    satellite: Satellite,
    at: GpsTime,
}

/// Show the hover tooltip for a satellite's scrub marker: the per-report
/// plot's tooltip, plus the report the values were read from.
fn show_marker_tooltip(ui: &egui::Ui, response: &egui::Response, hover: &MarkerHover) {
    let satellite = &hover.satellite;
    egui::Tooltip::always_open(
        ui.ctx().clone(),
        ui.layer_id(),
        response
            .id
            .with(("marker", satellite.constellation(), satellite.prn())),
        egui::PopupAnchor::Pointer,
    )
    .show(|ui| plot_common::satellite_tooltip(ui, satellite, Some(hover.at)));
}

/// Show the hover tooltip for a slip mark.
fn show_slip_tooltip(ui: &egui::Ui, response: &egui::Response, slip: &SlipMark) {
    egui::Tooltip::always_open(
        ui.ctx().clone(),
        ui.layer_id(),
        response.id.with(("slip", slip.constellation, slip.prn)),
        egui::PopupAnchor::Pointer,
    )
    .show(|ui| {
        ui.label(egui::RichText::new(slip_label(slip)).strong());
        let degree = gt_ui_theme::DEGREE_SIGN;
        ui.label(format!("Elevation {:.0}{degree}", slip.elevation));
        ui.label(format!("Azimuth {:.0}{degree}", slip.azimuth));
    });
}

/// Show the elevation-mask ring's hover tooltip, naming it and its angle so it
/// reads as a labelled element rather than a mystery circle.
fn mask_tooltip(ui: &egui::Ui, response: &egui::Response, mask_deg: f32) {
    egui::Tooltip::always_open(
        ui.ctx().clone(),
        ui.layer_id(),
        response.id.with("mask_ring"),
        egui::PopupAnchor::Pointer,
    )
    .show(|ui| {
        ui.label(egui::RichText::new("Elevation mask").strong());
        ui.label(format!("{mask_deg:.0}{}", gt_ui_theme::DEGREE_SIGN));
    });
}

/// The visible slip mark nearest to `pointer`, within
/// [`style::SLIP_MARK_HOVER_RADIUS_PX`]. `visible` is the same predicate the
/// draw loop uses, so a hidden slip is never a hover target.
fn nearest_slip(
    slips: &[SlipMark],
    frame: Frame,
    pointer: Pos2,
    visible: impl Fn(&SlipMark) -> bool,
) -> Option<&SlipMark> {
    let candidates = slips
        .iter()
        .filter(|slip| visible(slip))
        .map(|slip| (slip, frame.project(slip.azimuth, slip.elevation)));
    plot_common::nearest_within(candidates, pointer, style::SLIP_MARK_HOVER_RADIUS_PX)
}

/// "G05 GPS - lost lock" for a slip mark's tooltip header.
fn slip_label(slip: &SlipMark) -> String {
    format!(
        "{} - {}",
        plot_common::satellite_designator(slip.constellation, slip.prn),
        slip.cause.label(),
    )
}

/// The satellite's `(azimuth, elevation)` at `time`, plus the report in effect
/// there - the last one received at or before `time`, which is what the
/// receiver was acting on at that instant and what the stats column counts.
///
/// The position is interpolated between reports so playback animates smoothly,
/// but the report is always a real one: scrubbing to 12:00:00.4 reads the
/// 12:00:00 report rather than inventing values for an instant no receiver ever
/// described. `None` when `time` is outside the trail or inside one of its
/// gaps, where the satellite is not drawn at all.
fn marker_at(trail: &SkyTrail, time: GpsTime) -> Option<(f32, f32, &TrailSample)> {
    let samples = &trail.samples;
    let idx = samples.partition_point(|s| s.time < time);
    // Exact hit on a sample.
    if let Some(s) = samples.get(idx)
        && s.time == time
    {
        return Some((s.azimuth, s.elevation, s));
    }
    // Otherwise interpolate between the bracketing samples, unless the pair
    // spans a gap or `time` is outside the trail.
    let (Some(a), Some(b)) = (
        idx.checked_sub(1).and_then(|i| samples.get(i)),
        samples.get(idx),
    ) else {
        return None;
    };
    if !b.epoch.follows(a.epoch) {
        return None;
    }
    let span = b.time.signed_duration_since(a.time).num_milliseconds();
    if span <= 0 {
        return Some((a.azimuth, a.elevation, a));
    }
    let f = time.signed_duration_since(a.time).num_milliseconds() as f32 / span as f32;
    Some((
        a.azimuth + (b.azimuth - a.azimuth) * f,
        a.elevation + (b.elevation - a.elevation) * f,
        a,
    ))
}

#[cfg(test)]
mod tests {
    use chrono::{DateTime, Duration, Utc};
    use rstest::rstest;

    use gt_test_utils::{Queryable as _, TestHarness};
    use gt_types::satellites::{Constellation, ConstellationSet, Satellite, Satellites};
    use gt_types::{GpsTime, Latitude, Longitude, NavPoint, PointIdx, TimePositionVelocity};

    use super::{SkyTrail, SkyTrailsPlot, SlipMark, marker_at};
    use crate::extract_trails;
    use crate::trails::{EpochIdx, SkyTrails, TrailEpoch, TrailSample};

    /// A frame big enough that the test's coordinates are unambiguous.
    fn test_frame() -> super::Frame {
        super::Frame {
            center: egui::pos2(200.0, 200.0),
            radius: 180.0,
        }
    }

    /// Samples that project less than a pixel apart collapse into one run, but
    /// the run still reaches the last sample rather than stopping at the last
    /// one that happened to clear the threshold.
    #[test]
    fn collapsed_samples_still_reach_the_end_of_the_trail() {
        let frame = test_frame();
        // Four samples a hair apart in elevation: sub-pixel on a 180px radius.
        let trail = trail_of(&[
            trail_sample(0, 90.0, 45.0),
            trail_sample(1, 90.0, 45.01),
            trail_sample(2, 90.0, 45.02),
            trail_sample(3, 90.0, 45.03),
        ]);

        let runs = super::trail_runs(&trail, frame, false);
        assert_eq!(runs.len(), 1, "sub-pixel samples collapse into one run");
        let last = trail.samples.last().expect("has samples");
        assert_eq!(
            runs[0].last().expect("run has vertices").pos,
            frame.project(last.azimuth, last.elevation),
            "the run must reach the trail's final sample"
        );
    }

    /// The collapse must never span an absence: a satellite that dropped out
    /// still shows a break, rather than a line drawn straight across the time
    /// it was gone.
    #[test]
    fn a_gap_ends_the_run_even_when_the_samples_are_sub_pixel_apart() {
        let frame = test_frame();
        let trail = trail_of(&[
            trail_sample(0, 90.0, 45.0),
            trail_sample(1, 90.0, 45.01),
            // Epoch 2 has no sample for this satellite: it was absent.
            trail_sample(3, 90.0, 45.02),
            trail_sample(4, 90.0, 45.03),
        ]);

        let runs = super::trail_runs(&trail, frame, false);
        // One run each side of the gap, and nothing bridging it.
        assert_eq!(runs.len(), 2, "the gap splits the trail in two");
        let before_gap = frame.project(90.0, 45.01);
        let after_gap = frame.project(90.0, 45.02);
        assert_eq!(runs[0].last().expect("run has vertices").pos, before_gap);
        assert_eq!(runs[1].first().expect("run has vertices").pos, after_gap);
    }

    /// Hovering a scrub marker must work wherever the scrubber is parked, not
    /// only when it happens to sit exactly on a report. Playback leaves the
    /// scrubber on a fractional second, so requiring an exact hit meant hover
    /// died as soon as you pressed play and stayed dead after pausing.
    #[test]
    fn a_marker_is_hoverable_between_reports() {
        // One satellite parked at a fixed sky position, so its marker sits in
        // the same place whether or not the instant is interpolated.
        let trail = trail_of(&[
            trail_sample_from_epoch(0, 0, 90.0, 45.0),
            trail_sample_from_epoch(2, 1, 90.0, 45.0),
        ]);
        let trails = SkyTrails {
            trails: vec![trail],
            epochs: vec![epoch(0), epoch(2)],
            slips: Vec::new(),
            time_range: Some(gt_types::GpsTimeRange::new(at(0), at(2))),
        };

        // Half a second past the first report: mid-interpolation, exactly
        // where playback leaves the scrubber.
        let between = GpsTime::from_utc(start() + Duration::milliseconds(500));
        let rect = std::rc::Rc::new(std::cell::Cell::new(egui::Rect::ZERO));
        let seen = rect.clone();
        let mut harness = TestHarness::builder()
            .size(egui::vec2(PLOT_DIAMETER_PX + 40.0, PLOT_DIAMETER_PX + 40.0))
            .ui(move |ui| {
                let response = SkyTrailsPlot::new(&trails, PLOT_DIAMETER_PX)
                    .shown(ConstellationSet::all())
                    .show_not_in_fix(true)
                    .scrub(Some(between))
                    .ui(ui);
                seen.set(response.rect);
            });
        harness.run();

        // Where the plot puts a satellite at 90 deg azimuth, 45 deg elevation.
        let plot = rect.get();
        let radius = plot.width() / 2.0 - crate::style::FULL_RIM_MARGIN_PX;
        let marker = plot.center() + crate::unit_disc_position(90.0, 45.0) * radius;

        assert!(
            harness.inner.query_by_label("In fix").is_none(),
            "the tooltip must not be showing before the marker is hovered"
        );
        harness.inner.hover_at(marker);
        harness.inner.run_steps(2);
        assert!(
            harness.inner.query_by_label("In fix").is_some(),
            "hovering the marker between reports must still show its tooltip"
        );
    }

    /// A synthetic track: `epochs` reports one second apart, each carrying
    /// `sats` satellites sweeping across the sky.
    fn long_trails(epochs: usize, sats: usize) -> SkyTrails {
        let points = (0..epochs)
            .map(|i| {
                let f = i as f32 / (epochs - 1) as f32;
                let list = (0..sats)
                    .map(|s| {
                        let base = s as f32 * 11.0;
                        Satellite::new(
                            Constellation::Gps,
                            (s as u32 % 32) + 1,
                            Some(15.0 + 60.0 * (base + f * 180.0).to_radians().sin().abs()),
                            Some((base + f * 90.0) % 360.0),
                            Some(40.0),
                            true,
                        )
                    })
                    .collect();
                let tpv = TimePositionVelocity::builder()
                    .time(GpsTime::from_utc(start() + Duration::seconds(i as i64)))
                    .lat(Latitude::new(55.0))
                    .lon(Longitude::new(12.0))
                    .build();
                NavPoint::new(tpv, Some(Satellites::new(None, None, list)))
            })
            .collect();
        extract_trails(&gt_test_utils::loaded_track_with_points(points))
    }

    /// Shapes emitted for one frame of the plot at `PLOT_DIAMETER_PX`.
    fn painted_shapes(trails: SkyTrails) -> usize {
        let mut harness = TestHarness::builder()
            .size(egui::vec2(PLOT_DIAMETER_PX + 40.0, PLOT_DIAMETER_PX + 40.0))
            .ui(move |ui| {
                SkyTrailsPlot::new(&trails, PLOT_DIAMETER_PX)
                    .shown(ConstellationSet::all())
                    .show_not_in_fix(true)
                    .ui(ui);
            });
        harness.run();
        harness.inner.output().shapes.len()
    }

    const PLOT_DIAMETER_PX: f32 = 400.0;

    /// The plot emits one line segment per sample pair, per satellite, every
    /// frame - so its cost is linear in the track's length with no ceiling. A
    /// three-hour track at 1 Hz with 30 satellites is ~324k shapes per frame,
    /// which is what made the window unusable on a long recording.
    ///
    /// Adjacent samples on a long track project sub-pixel apart, so nearly all
    /// of that work is invisible: the segment count must be bounded by what the
    /// plot can actually resolve, not by how long the recording ran.
    #[test]
    fn the_trail_paint_cost_is_bounded_by_the_plot_not_the_track_length() {
        let short = painted_shapes(long_trails(600, 12));
        let long = painted_shapes(long_trails(4800, 12));

        // Eight times the epochs must not cost eight times the shapes: past the
        // point where samples land on the same pixel there is nothing more to
        // draw.
        assert!(
            long < short * 2,
            "8x the track length cost {long} shapes against {short} - \
             the paint cost is tracking the recording, not the plot"
        );
    }

    fn start() -> DateTime<Utc> {
        DateTime::<Utc>::from_timestamp(1_748_000_000, 0).expect("valid")
    }

    fn at(secs: i64) -> GpsTime {
        GpsTime::from_utc(start() + Duration::seconds(secs))
    }

    // The point index is irrelevant to the time-only helpers under test.
    fn epoch(secs: i64) -> TrailEpoch {
        TrailEpoch {
            time: at(secs),
            point_index: PointIdx::new(0),
        }
    }

    /// A sample at second `secs`, taken from the epoch of the same number.
    /// These fixtures report at 1 Hz, so a skipped second is a skipped epoch
    /// and reads as a gap. Use [`trail_sample_from_epoch`] where the two must
    /// come apart.
    fn trail_sample(secs: i64, azimuth: f32, elevation: f32) -> TrailSample {
        trail_sample_from_epoch(secs, secs.unsigned_abs() as usize, azimuth, elevation)
    }

    /// A sample at second `secs` taken from epoch `epoch`, for fixtures whose
    /// reports are not one second apart.
    fn trail_sample_from_epoch(
        secs: i64,
        epoch: usize,
        azimuth: f32,
        elevation: f32,
    ) -> TrailSample {
        TrailSample {
            time: at(secs),
            epoch: EpochIdx::new(epoch),
            point_index: PointIdx::new(0),
            azimuth,
            elevation,
            snr: None,
            in_fix: true,
        }
    }

    /// A trail carrying exactly `samples`.
    fn trail_of(samples: &[TrailSample]) -> SkyTrail {
        SkyTrail {
            constellation: Constellation::Gps,
            prn: gt_types::satellites::Prn::new(5),
            samples: samples.to_vec(),
        }
    }

    /// A trail with samples at t0 and t2 - the satellite is absent at the t1
    /// epoch, so its two samples come from epochs 0 and 2, not back-to-back
    /// reports, and the trail breaks between them.
    fn gapped_trail() -> SkyTrail {
        SkyTrail {
            constellation: Constellation::Gps,
            prn: gt_types::satellites::Prn::new(5),
            samples: vec![trail_sample(0, 40.0, 60.0), trail_sample(2, 60.0, 40.0)],
        }
    }

    #[rstest]
    #[case::exact_hit(at(0), Some((40.0, 60.0)))]
    #[case::in_gap(at(1), None)]
    #[case::before_first_sample(at(-1), None)]
    #[case::after_last_sample(at(5), None)]
    fn marker_at_respects_trail_bounds_and_gaps(
        #[case] time: GpsTime,
        #[case] expected: Option<(f32, f32)>,
    ) {
        let position = marker_at(&gapped_trail(), time).map(|(az, el, _)| (az, el));
        assert_eq!(position, expected);
    }

    /// Two back-to-back reports two seconds apart - no skipped epoch between
    /// them, so the trail runs straight through and interpolates.
    fn unbroken_trail() -> SkyTrail {
        trail_of(&[
            trail_sample_from_epoch(0, 0, 40.0, 60.0),
            trail_sample_from_epoch(2, 1, 60.0, 40.0),
        ])
    }

    /// Between reports the marker interpolates its position but still reports
    /// the report in effect - the last one received. Hover used to go dead here
    /// (there was no sample to show), which killed it for the whole of playback
    /// and everything after it.
    #[test]
    fn marker_at_carries_the_report_in_effect_between_reports() {
        let trail = unbroken_trail();
        // Exactly on a report: that report.
        let (_, _, report) = marker_at(&trail, at(0)).expect("hit");
        assert_eq!(report.time, at(0));
        // Between reports: interpolated position, earlier report still in
        // effect.
        let (az, el, report) = marker_at(&trail, at(1)).expect("interpolated");
        assert_eq!((az, el), (50.0, 50.0));
        assert_eq!(report.time, at(0), "the last report received still stands");
    }

    struct Spec {
        c: Constellation,
        prn: u32,
        az: (f32, f32),
        el: (f32, f32),
        absent: &'static [usize],
    }

    /// A synthetic track: several satellites drifting across the sky over ten
    /// report epochs, one dropping out mid-track to leave a gap.
    fn demo_trails() -> SkyTrails {
        const EPOCHS: usize = 10;
        let specs = [
            Spec {
                c: Constellation::Gps,
                prn: 5,
                az: (40.0, 95.0),
                el: (58.0, 71.0),
                absent: &[],
            },
            Spec {
                c: Constellation::Gps,
                prn: 12,
                az: (85.0, 130.0),
                el: (20.0, 47.0),
                absent: &[],
            },
            Spec {
                c: Constellation::Galileo,
                prn: 3,
                az: (60.0, 30.0),
                el: (52.0, 40.0),
                absent: &[],
            },
            Spec {
                c: Constellation::Glonass,
                prn: 9,
                az: (170.0, 205.0),
                el: (48.0, 28.0),
                absent: &[],
            },
            Spec {
                c: Constellation::Beidou,
                prn: 14,
                az: (250.0, 230.0),
                el: (66.0, 51.0),
                absent: &[4, 5],
            },
        ];
        let lerp = |(a, b): (f32, f32), f: f32| a + (b - a) * f;
        let points = (0..EPOCHS)
            .map(|i| {
                let f = i as f32 / (EPOCHS - 1) as f32;
                let sats: Vec<Satellite> = specs
                    .iter()
                    .filter(|s| !s.absent.contains(&i))
                    .map(|s| {
                        Satellite::new(
                            s.c,
                            s.prn,
                            Some(lerp(s.el, f)),
                            Some(lerp(s.az, f)),
                            Some(40.0),
                            true,
                        )
                    })
                    .collect();
                let tpv = TimePositionVelocity::builder()
                    .time(at(i as i64))
                    .lat(Latitude::new(55.0))
                    .lon(Longitude::new(12.0))
                    .build();
                NavPoint::new(tpv, Some(Satellites::new(None, None, sats)))
            })
            .collect();
        extract_trails(&gt_test_utils::loaded_track_with_points(points))
    }

    fn snapshot(
        name: &str,
        shown: ConstellationSet,
        focus: Option<Constellation>,
        scrub: Option<GpsTime>,
    ) {
        let trails = demo_trails();
        let mut harness = TestHarness::builder()
            .size(egui::vec2(320.0, 320.0))
            .theme(true)
            .ui(move |ui| {
                SkyTrailsPlot::new(&trails, 300.0)
                    .shown(shown)
                    .focus(focus)
                    .scrub(scrub)
                    .with_elevation_mask_deg(10.0)
                    .ui(ui);
            });
        harness.run();
        harness.snapshot(name);
    }

    #[test]
    fn sky_trails_full() {
        // Every trail, time-ramped, with the BeiDou gap.
        snapshot("sky_trails_full", ConstellationSet::all(), None, None);
    }

    #[test]
    fn sky_trails_focused() {
        // GPS focused: its trails stay bright, the rest dim.
        snapshot(
            "sky_trails_focused",
            ConstellationSet::all(),
            Some(Constellation::Gps),
            None,
        );
    }

    #[test]
    fn sky_trails_filtered() {
        // Only GPS and Galileo shown.
        let shown = ConstellationSet::single(Constellation::Gps).with(Constellation::Galileo);
        snapshot("sky_trails_filtered", shown, None, None);
    }

    #[test]
    fn sky_trails_scrubbed() {
        // A marker on each trail at a mid-track instant.
        snapshot(
            "sky_trails_scrubbed",
            ConstellationSet::all(),
            None,
            Some(at(4)),
        );
    }

    /// Snapshot: on a long track each trail draws as a comet's tail - brightest
    /// at the satellite's current position, fading back over the ten minutes it
    /// already travelled, with the rest of the path a faint ghost. Twenty
    /// minutes at 1 Hz scrubbed to fifteen minutes in, so the bounded bright
    /// tail sits between a ghost of the earlier path behind it and a ghost of
    /// the path still ahead - the direction of travel reads without any arrow.
    #[test]
    fn sky_trails_tail_points_the_way_the_satellite_is_moving() {
        let trails = long_trails(1200, 3);
        let mut harness = TestHarness::builder()
            .size(egui::vec2(320.0, 320.0))
            .theme(true)
            .ui(move |ui| {
                SkyTrailsPlot::new(&trails, 300.0)
                    .scrub(Some(at(900)))
                    .ui(ui);
            });
        harness.run();
        harness.snapshot("sky_trails_tail");
    }

    #[test]
    fn sky_trails_with_slips() {
        use gt_types::satellites::{Prn, SlipCause};

        let mut trails = demo_trails();
        // Two slip marks (an "×" each), on distinct constellations.
        trails.slips = vec![
            SlipMark {
                constellation: Constellation::Gps,
                prn: Prn::new(5),
                azimuth: 70.0,
                elevation: 64.0,
                cause: SlipCause::LostLock,
            },
            SlipMark {
                constellation: Constellation::Galileo,
                prn: Prn::new(3),
                azimuth: 45.0,
                elevation: 46.0,
                cause: SlipCause::SnrDrop,
            },
        ];
        let mut harness = TestHarness::builder()
            .size(egui::vec2(320.0, 320.0))
            .theme(true)
            .ui(move |ui| {
                SkyTrailsPlot::new(&trails, 300.0)
                    .with_elevation_mask_deg(10.0)
                    .ui(ui);
            });
        harness.run();
        harness.snapshot("sky_trails_with_slips");
    }

    #[test]
    fn sky_trails_mask_ring_hover_lights_up_and_labels() {
        use std::cell::Cell;
        use std::rc::Rc;

        let trails = demo_trails();
        // The plot's rendered centre, captured so the hover point can be placed
        // on the ring regardless of the harness's layout margins.
        let center = Rc::new(Cell::new(egui::Pos2::ZERO));
        let sink = Rc::clone(&center);
        let mut harness = TestHarness::builder()
            .size(egui::vec2(320.0, 360.0))
            .theme(true)
            .ui(move |ui| {
                let response = SkyTrailsPlot::new(&trails, 300.0)
                    .with_elevation_mask_deg(10.0)
                    .ui(ui);
                sink.set(response.rect.center());
            });
        harness.run();

        // Hover a point on the mask ring (its east side).
        let radius = 300.0 / 2.0 - super::style::FULL_RIM_MARGIN_PX;
        let ring_radius = radius * super::projection::unit_disc_radius(10.0);
        let on_ring = center.get() + egui::vec2(ring_radius, 0.0);
        harness.inner.hover_at(on_ring);
        // Tooltips appear after egui's hover delay; step until it elapses.
        for _ in 0..60 {
            harness.run();
        }
        harness.snapshot_loose("sky_trails_mask_ring_hover");
    }

    #[test]
    fn sky_trails_marker_hover_shows_the_satellite() {
        use std::cell::Cell;
        use std::rc::Rc;

        let trails = demo_trails();
        // The GPS-5 satellite's position at the scrubbed epoch, so the hover
        // point can land on its marker.
        let scrub = at(4);
        let gps5 = trails
            .trails
            .iter()
            .find(|t| t.constellation == Constellation::Gps && t.prn.value() == 5)
            .expect("gps-5 trail");
        let sample = gps5.sample_exactly_at(scrub).expect("sample at the epoch");
        let offset = super::projection::unit_disc_position(sample.azimuth, sample.elevation);

        let center = Rc::new(Cell::new(egui::Pos2::ZERO));
        let sink = Rc::clone(&center);
        let trails_for_ui = trails.clone();
        let mut harness = TestHarness::builder()
            .size(egui::vec2(320.0, 360.0))
            .theme(true)
            .ui(move |ui| {
                let response = SkyTrailsPlot::new(&trails_for_ui, 300.0)
                    .scrub(Some(scrub))
                    .ui(ui);
                sink.set(response.rect.center());
            });
        harness.run();

        let radius = 300.0 / 2.0 - super::style::FULL_RIM_MARGIN_PX;
        harness.inner.hover_at(center.get() + offset * radius);
        for _ in 0..60 {
            harness.run();
        }
        harness.snapshot_loose("sky_trails_marker_hover");
    }

    #[test]
    fn satellite_from_sample_carries_the_reports_facts() {
        let trail = SkyTrail {
            constellation: Constellation::Gps,
            prn: gt_types::satellites::Prn::new(5),
            samples: vec![],
        };
        let sample = TrailSample {
            time: at(0),
            epoch: EpochIdx::new(0),
            point_index: PointIdx::new(0),
            azimuth: 40.0,
            elevation: 60.0,
            snr: Some(gt_types::satellites::Snr::new(42.0)),
            in_fix: true,
        };

        let satellite = super::satellite_from_sample(&trail, &sample);
        assert_eq!(satellite.constellation(), Constellation::Gps);
        assert_eq!(satellite.prn().value(), 5);
        assert_eq!(satellite.azimuth(), Some(40.0));
        assert_eq!(satellite.elevation(), Some(60.0));
        assert_eq!(satellite.snr().map(|s| s.value()), Some(42.0));
        assert!(satellite.in_fix());
    }

    /// Trails exercising fix state: one always in fix, one tracked-but-not-in-
    /// fix at the scrubbed epoch (still shown, hollow marker), one never in fix
    /// over the track (hidden when not-in-fix is off).
    fn fix_state_trails() -> SkyTrails {
        let times = [at(0), at(1), at(2)];
        let mk = |c: Constellation, prn: u32, fixes: [bool; 3], az0: f32, az1: f32, el: f32| {
            let samples = fixes
                .into_iter()
                .zip(times)
                .enumerate()
                .map(|(i, (in_fix, time))| TrailSample {
                    time,
                    epoch: EpochIdx::new(i),
                    point_index: PointIdx::new(i),
                    azimuth: az0 + (az1 - az0) * i as f32 / 2.0,
                    elevation: el,
                    snr: Some(gt_types::satellites::Snr::new(40.0)),
                    in_fix,
                })
                .collect();
            SkyTrail {
                constellation: c,
                prn: gt_types::satellites::Prn::new(prn),
                samples,
            }
        };
        SkyTrails {
            trails: vec![
                mk(Constellation::Gps, 5, [true, true, true], 40.0, 70.0, 62.0),
                mk(
                    Constellation::Gps,
                    12,
                    [true, false, true],
                    120.0,
                    150.0,
                    40.0,
                ),
                mk(
                    Constellation::Galileo,
                    3,
                    [false, false, false],
                    200.0,
                    230.0,
                    30.0,
                ),
            ],
            epochs: vec![epoch(0), epoch(1), epoch(2)],
            time_range: Some(gt_types::GpsTimeRange::new(at(0), at(2))),
            // A slip on the never-in-fix Galileo-3: hidden along with its trail
            // when not-in-fix is off, so no orphan mark is left behind.
            slips: vec![SlipMark {
                constellation: Constellation::Galileo,
                prn: gt_types::satellites::Prn::new(3),
                azimuth: 215.0,
                elevation: 30.0,
                cause: gt_types::satellites::SlipCause::LostLock,
            }],
        }
    }

    #[rstest]
    // Shown: all three trails, GPS-5 a filled marker, GPS-12 and the never-in-
    // fix Galileo-3 hollow (both tracked-only at this instant).
    #[case::shown("sky_trails_not_in_fix_shown", true)]
    // Hidden: the never-in-fix Galileo-3 trail is gone entirely.
    #[case::hidden("sky_trails_not_in_fix_hidden", false)]
    fn not_in_fix_toggle_hides_trails_and_hollows_markers(
        #[case] name: &str,
        #[case] show_not_in_fix: bool,
    ) {
        let trails = fix_state_trails();
        let mut harness = TestHarness::builder()
            .size(egui::vec2(320.0, 320.0))
            .theme(true)
            .ui(move |ui| {
                SkyTrailsPlot::new(&trails, 300.0)
                    .scrub(Some(at(1)))
                    .show_not_in_fix(show_not_in_fix)
                    .ui(ui);
            });
        harness.run();
        harness.snapshot(name);
    }

    /// Snapshot: with the trails hidden, only the current-instant markers
    /// remain - the immediate snapshot of where the satellites are now, with no
    /// whole-track polylines behind them.
    #[test]
    fn sky_trails_trails_hidden_leaves_only_markers() {
        let trails = demo_trails();
        let mut harness = TestHarness::builder()
            .size(egui::vec2(320.0, 320.0))
            .theme(true)
            .ui(move |ui| {
                SkyTrailsPlot::new(&trails, 300.0)
                    .scrub(Some(at(4)))
                    .show_trails(false)
                    .with_elevation_mask_deg(10.0)
                    .ui(ui);
            });
        harness.run();
        harness.snapshot("sky_trails_trails_hidden");
    }

    /// Snapshot: "in fix only" trims each trail to its in-fix stretches and
    /// drops the markers of satellites not in the fix right now. At epoch 1 of
    /// `fix_state_trails` GPS-5 is in the fix throughout (full trail, filled
    /// marker), GPS-12 is tracked-only at this epoch (its trail breaks there, no
    /// marker), and Galileo-3 is never in the fix (no trail at all, and its slip
    /// mark goes with it). The trimming is by whole-track fix state, so it does
    /// not blink as playback crosses the epochs - only the marker follows the
    /// instant.
    #[test]
    fn sky_trails_in_fix_now_keeps_only_the_current_fix() {
        let trails = fix_state_trails();
        let mut harness = TestHarness::builder()
            .size(egui::vec2(320.0, 320.0))
            .theme(true)
            .ui(move |ui| {
                SkyTrailsPlot::new(&trails, 300.0)
                    .scrub(Some(at(1)))
                    .in_fix_now(true)
                    .ui(ui);
            });
        harness.run();
        harness.snapshot("sky_trails_in_fix_now");
    }

    /// The trail polylines and the round markers, counted for a single frame at
    /// `at(1)` of `fix_state_trails`, where GPS-12 is tracked-but-not-in-fix.
    /// Trails draw as connected polylines (`egui::Shape::Path`), markers as
    /// circles.
    fn trail_path_and_marker_shapes(in_fix_now: bool) -> (usize, usize) {
        let trails = fix_state_trails();
        let mut harness = TestHarness::builder()
            .size(egui::vec2(320.0, 320.0))
            .ui(move |ui| {
                SkyTrailsPlot::new(&trails, 300.0)
                    .scrub(Some(at(1)))
                    .in_fix_now(in_fix_now)
                    .ui(ui);
            });
        harness.run();
        let shapes = &harness.inner.output().shapes;
        let paths = shapes
            .iter()
            .filter(|s| matches!(s.shape, egui::Shape::Path(_)))
            .count();
        let circles = shapes
            .iter()
            .filter(|s| matches!(s.shape, egui::Shape::Circle(_)))
            .count();
        (paths, circles)
    }

    /// "In fix only" trims each trail to its in-fix stretches and drops the
    /// markers of satellites not in the fix right now, so both the trail
    /// geometry and the marker count shrink when it is on. At at(1) of
    /// `fix_state_trails` GPS-12 is tracked-only (its trail breaks) and Galileo-3
    /// is never in the fix (its trail vanishes).
    #[test]
    fn in_fix_only_trims_trails_and_drops_markers() {
        let (paths_on, markers_on) = trail_path_and_marker_shapes(true);
        let (paths_off, markers_off) = trail_path_and_marker_shapes(false);

        assert!(
            paths_on < paths_off,
            "in fix only must trim the not-in-fix trail geometry \
             ({paths_on} with the filter vs {paths_off} without)"
        );
        assert!(
            markers_on < markers_off,
            "in fix only must drop the markers of not-in-fix satellites \
             ({markers_on} with the filter vs {markers_off} without)"
        );
    }

    /// With `in_fix_only`, a trail is drawn only through the samples where the
    /// satellite was in the fix: a not-in-fix sample breaks the run exactly as a
    /// tracking gap would, so no line bridges a tracked-but-unused stretch, and
    /// a satellite never in the fix draws nothing at all.
    #[test]
    fn in_fix_only_trims_the_trail_to_its_in_fix_stretches() {
        let frame = test_frame();
        let sample = |secs: i64, elevation: f32, in_fix: bool| TrailSample {
            time: at(secs),
            epoch: EpochIdx::new(secs.unsigned_abs() as usize),
            point_index: PointIdx::new(0),
            azimuth: 90.0,
            elevation,
            snr: None,
            in_fix,
        };
        let trail = SkyTrail {
            constellation: Constellation::Gps,
            prn: gt_types::satellites::Prn::new(5),
            samples: vec![
                sample(0, 20.0, true),
                sample(1, 40.0, true),
                sample(2, 60.0, false),
                sample(3, 30.0, true),
            ],
        };

        // Untrimmed: one run through all four samples.
        let full = super::trail_runs(&trail, frame, false);
        assert_eq!(full.len(), 1);
        assert_eq!(full[0].len(), 4);

        // Trimmed: the not-in-fix sample at epoch 2 is dropped, so epoch 3 no
        // longer follows the run and only the 0->1 in-fix stretch survives.
        let trimmed = super::trail_runs(&trail, frame, true);
        assert_eq!(
            trimmed.len(),
            1,
            "no line may bridge the not-in-fix stretch"
        );
        assert_eq!(trimmed[0].len(), 2);

        // Never in the fix: nothing to draw once trimmed.
        let never = SkyTrail {
            samples: vec![sample(0, 20.0, false), sample(1, 40.0, false)],
            ..trail
        };
        assert!(super::trail_runs(&never, frame, true).is_empty());
    }

    /// Splitting a run into fade stretches must leave a continuous line: each
    /// stretch is a real polyline (two or more distinct vertices), consecutive
    /// stretches share their boundary vertex, and no vertex is ever repeated
    /// within a stretch. A duplicated boundary vertex would draw a zero-length
    /// segment, and a trailing single-vertex stretch would draw a stray dot at
    /// the end of every trail.
    #[test]
    fn fade_stretches_stay_continuous_without_duplicated_vertices() {
        // Vertices half a minute apart, with the satellite's current position on
        // the last of them, so the whole run is tail and the fade steps change
        // repeatedly along it.
        let run: Vec<super::TrailVertex> = (0..=10)
            .map(|i| super::TrailVertex {
                pos: egui::pos2(i as f32 * 10.0, 0.0),
                time: at(i * 30),
            })
            .collect();
        let stretches = super::fade_stretches(&run, Some(at(300)));

        assert!(stretches.len() > 1, "the fade must change along the run");
        for (points, _) in &stretches {
            assert!(points.len() >= 2, "a stretch must draw a real line");
            for pair in points.windows(2) {
                let [a, b] = pair else { continue };
                assert_ne!(a, b, "a stretch must not repeat a vertex");
            }
        }
        // Consecutive stretches meet at exactly one shared vertex.
        for pair in stretches.windows(2) {
            let [(before, _), (after, _)] = pair else {
                continue;
            };
            assert_eq!(
                before.last(),
                after.first(),
                "stretches must share their boundary vertex"
            );
        }
        // Every vertex of the run is covered: the stretches, de-duplicated at
        // the shared boundaries, walk the whole polyline.
        let mut walked: Vec<egui::Pos2> = Vec::new();
        for (points, _) in &stretches {
            let skip = usize::from(walked.last() == points.first());
            walked.extend(points.iter().skip(skip));
        }
        let expected: Vec<egui::Pos2> = run.iter().map(|v| v.pos).collect();
        assert_eq!(walked, expected, "the stretches must cover the whole run");
    }

    /// The fade shapes the trail into a tail: brightest at the satellite's
    /// current position, fading back over the tail's length into the path it
    /// already travelled, and flat at the floor over the path ahead of it. The
    /// asymmetry is the point - it is what makes the bright end read as the head
    /// and so shows the direction of travel.
    #[test]
    fn trail_fade_makes_a_tail_behind_the_satellite() {
        use crate::style::{TRAIL_FADE_STEPS, TRAIL_MAX_ALPHA, TRAIL_MIN_ALPHA};

        let approx = |a: f32, b: f32| (a - b).abs() < 1e-6;
        let scrub = Some(at(600));
        // At the satellite: full strength - the head of the tail.
        assert!(approx(super::trail_fade(at(600), scrub), 1.0));
        // Behind it, the tail fades with distance travelled but stays lit.
        let just_behind = super::trail_fade(at(540), scrub);
        let further_behind = super::trail_fade(at(420), scrub);
        assert!(just_behind < 1.0 && just_behind > 0.0);
        assert!(
            further_behind < just_behind,
            "the tail must keep fading the further back it goes"
        );
        // Ahead of it - not travelled yet - is flat floor, however near.
        assert!(approx(super::trail_fade(at(660), scrub), 0.0));
        assert!(approx(super::trail_fade(at(601), scrub), 0.0));
        assert!(
            super::trail_fade(at(599), scrub) > super::trail_fade(at(601), scrub),
            "the emphasis must sit behind the satellite, not ahead of it"
        );
        // A full tail behind (ten minutes), and beyond: the floor.
        assert!(approx(super::trail_fade(at(0), scrub), 0.0));
        assert!(approx(super::trail_fade(at(-120), scrub), 0.0));
        // No scrub: full strength everywhere.
        assert!(approx(super::trail_fade(at(0), None), 1.0));

        // Quantized: the head sits at the top step, a full tail back at the
        // floor step, and the step alphas span the range.
        assert_eq!(super::fade_step(at(600), scrub), TRAIL_FADE_STEPS as i32);
        assert_eq!(super::fade_step(at(0), scrub), 0);
        assert!(approx(super::fade_step_alpha(0), TRAIL_MIN_ALPHA));
        assert!(approx(
            super::fade_step_alpha(TRAIL_FADE_STEPS as i32),
            TRAIL_MAX_ALPHA
        ));
    }

    /// The opacity multiplier scales the whole trail, but the stroke alpha
    /// saturates at fully opaque: a high opacity lifts the faint tail while the
    /// bright head simply tops out rather than overflowing past 1.0 (which would
    /// blow out the colour in `gamma_multiply`).
    #[test]
    fn trail_stroke_alpha_scales_the_tail_and_saturates_the_head() {
        use crate::style::{TRAIL_FADE_STEPS, TRAIL_MAX_ALPHA, TRAIL_MIN_ALPHA};

        let approx = |a: f32, b: f32| (a - b).abs() < 1e-6;
        let top = TRAIL_FADE_STEPS as i32;
        // The maximum opacity the field allows (100 %): 2.5x the tuned look.
        let max_opacity =
            crate::style::trail_opacity_multiplier(crate::style::TRAIL_OPACITY_PERCENT_MAX);

        // At the tuned default the alpha is the fade step's own alpha.
        assert!(approx(
            super::trail_stroke_alpha(top, 1.0, 1.0),
            TRAIL_MAX_ALPHA
        ));
        assert!(approx(
            super::trail_stroke_alpha(0, 1.0, 1.0),
            TRAIL_MIN_ALPHA
        ));

        // Turned up: the head saturates at 1.0, the floor lifts but stays below.
        assert!(approx(
            super::trail_stroke_alpha(top, 1.0, max_opacity),
            1.0
        ));
        let lifted = super::trail_stroke_alpha(0, 1.0, max_opacity);
        assert!(
            lifted > TRAIL_MIN_ALPHA && lifted < 1.0,
            "the ghost floor must lift with opacity but not saturate: {lifted}"
        );

        // The focus factor dims independently of opacity.
        assert!(
            super::trail_stroke_alpha(top, crate::style::TRAIL_DIMMED_ALPHA, 1.0)
                < super::trail_stroke_alpha(top, 1.0, 1.0)
        );
    }

    /// A segment takes its *later* end's step. Shading by the earlier end
    /// dragged the whole tail one segment toward the future: the segment leaving
    /// the head took the head's brightness while the ones arriving at it stayed
    /// dim, so the emphasis appeared to sit ahead of the satellite - the visual
    /// opposite of a tail.
    #[test]
    fn a_segment_takes_its_later_end_so_the_tail_sits_behind_the_head() {
        // Vertices a minute apart, with the satellite's current position - the
        // head - on the middle one.
        let run: Vec<super::TrailVertex> = (0..5)
            .map(|i| super::TrailVertex {
                pos: egui::pos2(i as f32 * 10.0, 0.0),
                time: at(i * 60),
            })
            .collect();
        let head = at(120);
        let stretches = super::fade_stretches(&run, Some(head));

        // The brightest stretch must end at the head, not start from it.
        let brightest = stretches
            .iter()
            .max_by_key(|(_, step)| *step)
            .expect("stretches drawn");
        let head_pos = run[2].pos;
        assert_eq!(
            brightest.0.last(),
            Some(&head_pos),
            "the brightest stretch must arrive at the satellite, not leave it"
        );
        // Everything drawn ahead of the head sits at the floor step.
        for (points, step) in &stretches {
            if points.iter().all(|p| p.x > head_pos.x) {
                assert_eq!(*step, 0, "the path ahead of the satellite stays a ghost");
            }
        }
    }

    /// Snapshot: the signal heat field glows beneath the trails at the
    /// current-instant in-fix satellites, brighter where the signal is stronger
    /// and where satellites cluster.
    #[test]
    fn sky_trails_heatmap_glows_under_the_fix_satellites() {
        let trails = demo_trails();
        let mut harness = TestHarness::builder()
            .size(egui::vec2(320.0, 320.0))
            .theme(true)
            .ui(move |ui| {
                SkyTrailsPlot::new(&trails, 300.0)
                    .scrub(Some(at(4)))
                    .show_heatmap(true)
                    .with_elevation_mask_deg(10.0)
                    .ui(ui);
            });
        harness.run();
        harness.snapshot("sky_trails_heatmap");
    }

    #[test]
    fn marker_is_hollow_only_for_a_tracked_not_in_fix_report() {
        let sample = |in_fix| TrailSample {
            time: at(0),
            epoch: EpochIdx::new(0),
            point_index: PointIdx::new(0),
            azimuth: 0.0,
            elevation: 0.0,
            snr: None,
            in_fix,
        };
        // In the fix per the report in effect: filled.
        assert!(!super::marker_is_hollow(&sample(true)));
        // Tracked but not in the fix per the report in effect: hollow.
        assert!(super::marker_is_hollow(&sample(false)));
    }

    #[test]
    fn slip_label_names_the_satellite_and_cause() {
        use gt_types::satellites::{Prn, SlipCause};
        let slip = SlipMark {
            constellation: Constellation::Gps,
            prn: Prn::new(5),
            azimuth: 70.0,
            elevation: 64.0,
            cause: SlipCause::LostLock,
        };
        assert_eq!(super::slip_label(&slip), "G05 GPS - lost lock");
    }

    #[rstest]
    // A slip at azimuth 90 / elevation 0 sits on the east rim (radius = the
    // frame radius from the center). Pointer near it hits; far misses; a
    // hidden constellation is never selected.
    #[case::hits(egui::pos2(102.0, 100.0), ConstellationSet::all(), true)]
    #[case::beyond_radius(egui::pos2(130.0, 100.0), ConstellationSet::all(), false)]
    #[case::constellation_hidden(egui::pos2(102.0, 100.0), ConstellationSet::empty(), false)]
    fn nearest_slip_respects_radius_and_filter(
        #[case] pointer: egui::Pos2,
        #[case] shown: ConstellationSet,
        #[case] expected: bool,
    ) {
        use gt_types::satellites::{Prn, SlipCause};
        let frame = super::Frame {
            center: egui::pos2(100.0, 100.0),
            radius: 1.0,
        };
        // Placed on the east rim: unit-disc (1, 0) -> center + (1, 0).
        let slips = [SlipMark {
            constellation: Constellation::Gps,
            prn: Prn::new(5),
            azimuth: 90.0,
            elevation: 0.0,
            cause: SlipCause::LostLock,
        }];
        let hit = super::nearest_slip(&slips, frame, pointer, |slip| {
            shown.contains(slip.constellation)
        })
        .is_some();
        assert_eq!(hit, expected);
    }
}
