use egui::Grid;
use egui::{Color32, Pos2, Response, Stroke, Ui};
use egui_phosphor::regular::ARROW_RIGHT as ICON_ARROW_RIGHT;
use gt_filter::GlobalFilter;
use gt_types::{DataCategory, LoadedFile, PointIdx, SpatialPoint};
use gt_ui_types::{
    DataPointRef, GeneratedMarkerVisibility, HighlightScope, MapHighlight, TrackDataVisibility,
};
use walkers::{MapMemory, Plugin, Projector};

use crate::track_renderer;

pub struct GeneratedMarkerRenderer<'a> {
    files: &'a [LoadedFile],
    visibility: &'a TrackDataVisibility,
    highlight: &'a MapHighlight,
    filter: &'a GlobalFilter,
    generated_vis: &'a GeneratedMarkerVisibility,
    visible_generated: Vec<SpatialPoint>,
}

impl<'a> GeneratedMarkerRenderer<'a> {
    pub fn new(
        files: &'a [LoadedFile],
        visibility: &'a TrackDataVisibility,
        highlight: &'a MapHighlight,
        filter: &'a GlobalFilter,
        generated_vis: &'a GeneratedMarkerVisibility,
        visible_generated: Vec<SpatialPoint>,
    ) -> Self {
        Self {
            files,
            visibility,
            highlight,
            filter,
            generated_vis,
            visible_generated,
        }
    }

    fn is_point_highlighted(&self, point_ref: DataPointRef) -> bool {
        if self.highlight.sticky.is_some_and(|r| r == point_ref) {
            return true;
        }
        match self.highlight.hover {
            Some(HighlightScope::Point(r)) => r == point_ref,
            Some(HighlightScope::Track(track)) => track == point_ref.track,
            Some(HighlightScope::TrackCategory { track, category }) => {
                track == point_ref.track && category == DataCategory::GeneratedMarker
            }
            _ => false,
        }
    }

    fn show_tooltip(&self, ui: &Ui, point_ref: DataPointRef, pos: Pos2) {
        let Some(file) = point_ref.track.fi.get(self.files) else {
            return;
        };
        let Some(track) = point_ref.track.index.get(&file.tracks) else {
            return;
        };
        let Some(marker) = point_ref.point_index.get(&track.generated_markers) else {
            return;
        };
        let hit_rect = egui::Rect::from_center_size(pos, egui::vec2(20.0, 20.0));
        let response = ui.interact(
            hit_rect,
            ui.id()
                .with("gen_marker_hover")
                .with(point_ref.track)
                .with(point_ref.point_index),
            egui::Sense::hover(),
        );
        response.show_tooltip_ui(|ui| {
            ui.strong(generated_marker_header(&marker.kind));
            match &marker.kind {
                // A slip groups the satellites that lost lock at this epoch, so
                // show what changed for each (geometry and signal, before/after)
                // rather than the whole-epoch point table.
                gt_types::GeneratedMarkerKind::Slip(event) => {
                    ui.separator();
                    show_slip_table(ui, event);
                }
                // Fix-lost and clock-discontinuity markers sit on a specific
                // anomalous sample, so also show that point's data.
                gt_types::GeneratedMarkerKind::GnssFixLost
                | gt_types::GeneratedMarkerKind::ClockDiscontinuity { .. } => {
                    if let Some(index) = track
                        .points
                        .iter()
                        .position(|p| p.tpv.time().utc() == marker.time)
                        && let Some(point) = track.points.get(index)
                    {
                        ui.separator();
                        crate::tpv_renderer::show_hover_table(
                            ui,
                            point,
                            &crate::tpv_renderer::SkySection::resolve(track, PointIdx::new(index)),
                        );
                    }
                }
                // Fix-regained is a transition with no single underlying point.
                gt_types::GeneratedMarkerKind::GnssFixRegained { .. } => {}
            }
        });
    }
}

impl Plugin for GeneratedMarkerRenderer<'_> {
    fn run(
        self: Box<Self>,
        ui: &mut Ui,
        _response: &Response,
        projector: &Projector,
        map_memory: &MapMemory,
    ) {
        let transform =
            crate::transform::MercTransform::new(projector, map_memory, ui.max_rect().center());

        for sp in &self.visible_generated {
            let Some(track) = crate::scope::category_in_scope(
                self.files,
                self.visibility,
                self.filter,
                sp.track_ref(),
                DataCategory::GeneratedMarker,
            ) else {
                continue;
            };
            let Some(marker) = sp.point_index.get(&track.generated_markers) else {
                continue;
            };
            // Per-event-type show/hide (refines the category-level toggle).
            if !self
                .generated_vis
                .is_visible(sp.track_ref(), marker.kind.tag())
            {
                continue;
            }
            if !gt_filter::point_passes_time_filter(marker.time, self.filter) {
                continue;
            }
            let point_ref = DataPointRef {
                track: sp.track_ref(),
                category: DataCategory::GeneratedMarker,
                point_index: sp.point_index,
            };
            let screen_pos = transform.to_screen(sp.merc);
            let highlighted = self.is_point_highlighted(point_ref);
            let fade =
                track_renderer::track_fade_alpha(self.highlight, sp.file_index, sp.track_index);
            draw_generated_marker(ui, screen_pos, &marker.kind, highlighted, fade);
        }

        // Show tooltip for the hovered generated marker. Suppressed when the primary
        // hover is already a TPV point - the TPV tooltip covers the same data and
        // showing both would produce two overlapping labels at the same map position.
        let primary_is_tpv = matches!(
            self.highlight.hover,
            Some(HighlightScope::Point(r)) if r.category == DataCategory::Tpv
        );
        if !primary_is_tpv
            && let Some(r) = self.highlight.hover_candidates[3]
            && self.highlight.sticky != Some(r)
            && !ui.ctx().any_popup_open()
            && !self.highlight.suppress_hover_labels
            && let Some(file) = r.track.fi.get(self.files)
            && let Some(track) = r.track.index.get(&file.tracks)
            && let Some(marker) = r.point_index.get(&track.generated_markers)
        {
            let pos = transform.to_screen(marker.merc);
            self.show_tooltip(ui, r, pos);
        }
    }
}

/// Returns the header string for a generated-marker tooltip or hover section.
///
/// Centralises the "GNSS fix regained after Xs" formatting so both the live
/// tooltip (`show_tooltip`) and the multi-hover compound label
/// (`draw_candidate_section`) always produce identical text.
pub(crate) fn generated_marker_header(kind: &gt_types::GeneratedMarkerKind) -> String {
    match kind {
        gt_types::GeneratedMarkerKind::GnssFixRegained { fix_lost_duration } => format!(
            "{kind} after {}",
            format_fix_duration(fix_lost_duration.num_milliseconds())
        ),
        gt_types::GeneratedMarkerKind::ClockDiscontinuity { step } => {
            let ms = step.num_milliseconds();
            let sign = if ms < 0 { "-" } else { "+" };
            // saturating_abs, not abs: avoids the i64::MIN panic surface, matching
            // the structural .abs() avoidance in the detector.
            format!(
                "{kind} ({sign}{})",
                format_fix_duration(ms.saturating_abs())
            )
        }
        // One satellite: name it inline. Several: the per-satellite detail is in
        // the table below, so the header just gives the count.
        gt_types::GeneratedMarkerKind::Slip(event) => match event.slips.as_slice() {
            [slip] => format!(
                "{kind}: {} {:02} ({})",
                slip.constellation.display_name(),
                slip.prn,
                slip.cause.label(),
            ),
            slips => format!("{kind} ({})", slips.len()),
        },
        gt_types::GeneratedMarkerKind::GnssFixLost => kind.to_string(),
    }
}

/// Render a table of the satellites that slipped at one epoch, one row each,
/// showing the before/after change in elevation, azimuth, and SNR.  Shared by
/// the hover tooltip and the sticky (clicked) detail window.
///
/// Rows are ordered by constellation then PRN so the table is stable, and the
/// satellite cell is tinted with the constellation's canonical color.
pub(crate) fn show_slip_table(ui: &mut Ui, event: &gt_types::satellites::SlipEvent) {
    use gt_types::satellites::Snr;

    let mut slips = event.slips.clone();
    // `Constellation` derives `Ord` in variant-declaration order (GPS, GLONASS,
    // Galileo, BeiDou, NavIC, QZSS), which is the grouping we want here.
    slips.sort_by_key(|s| (s.constellation, s.prn.value()));

    Grid::new("slip_detail_grid")
        .num_columns(5)
        .striped(true)
        .show(ui, |ui| {
            for heading in ["Satellite", "Cause", "Elevation", "Azimuth", "SNR (dB-Hz)"] {
                ui.strong(heading);
            }
            ui.end_row();
            let dark_mode = ui.visuals().dark_mode;
            for slip in &slips {
                ui.colored_label(
                    gt_ui_theme::constellation_color(slip.constellation, dark_mode),
                    format!("{} {:02}", slip.constellation.display_name(), slip.prn),
                );
                ui.label(slip.cause.label());
                // `to` is `None` for a lost-lock slip (the satellite dropped out).
                let to = slip.to;
                ui.label(slip_change(
                    slip.from.elevation,
                    to.map(|t| t.elevation),
                    "°",
                ));
                ui.label(slip_change(slip.from.azimuth, to.map(|t| t.azimuth), "°"));
                ui.label(slip_change(
                    slip.from.snr.map(Snr::value),
                    to.map(|t| t.snr.map(Snr::value)),
                    "",
                ));
                ui.end_row();
            }
        });
}

/// Format one before/after table cell.
///
/// `to` is `None` when the satellite dropped out (lost lock), so only the
/// last-known `from` value is shown.  Otherwise `from -> to`, collapsing to a
/// single value when the two render identically (unchanged, or both unknown -
/// which shows a lone dash rather than `- -> -`).
fn slip_change(from: Option<f32>, to: Option<Option<f32>>, unit: &str) -> String {
    let fmt = |v: Option<f32>| v.map_or_else(|| "-".to_owned(), |x| format!("{x:.1}{unit}"));
    // Compare the rendered text, not the floats, to sidestep `float_cmp`.
    let before = fmt(from);
    match to {
        // Satellite gone: there is no "after", so just the last-known value.
        None => before,
        Some(after) => {
            let after = fmt(after);
            if after == before {
                before
            } else {
                format!("{before} {ICON_ARROW_RIGHT} {after}")
            }
        }
    }
}

/// Formats `total_ms` milliseconds as a human-readable duration string (e.g. `"12.3s"`, `"1m30s"`).
/// Negative values are clamped to zero.
fn format_fix_duration(total_ms: i64) -> String {
    let total_ms = total_ms.max(0);
    let secs = total_ms / 1000;
    let frac_cs = (total_ms % 1000) / 10;
    if secs < 60 {
        match frac_cs {
            0 => format!("{secs}s"),
            cs if cs % 10 == 0 => format!("{secs}.{}s", cs / 10),
            cs => format!("{secs}.{cs:02}s"),
        }
    } else {
        let minutes = secs / 60;
        let remaining_secs = secs % 60;
        if remaining_secs == 0 {
            format!("{minutes}m")
        } else {
            format!("{minutes}m{remaining_secs}s")
        }
    }
}

fn draw_generated_marker(
    ui: &Ui,
    center: Pos2,
    kind: &gt_types::GeneratedMarkerKind,
    highlighted: bool,
    fade: f32,
) {
    let painter = ui.painter();
    let (bg, stroke_color) = match kind {
        gt_types::GeneratedMarkerKind::GnssFixLost => {
            (Color32::from_rgb(219, 68, 55), Color32::WHITE)
        }
        gt_types::GeneratedMarkerKind::GnssFixRegained { .. } => {
            (Color32::from_rgb(15, 157, 88), Color32::WHITE)
        }
        gt_types::GeneratedMarkerKind::ClockDiscontinuity { .. } => {
            (Color32::from_rgb(255, 149, 0), Color32::WHITE)
        }
        // Orchid: distinct from the fix-lost red and clock-discontinuity orange.
        gt_types::GeneratedMarkerKind::Slip(_) => (Color32::from_rgb(186, 85, 211), Color32::WHITE),
    };
    let faded_bg = track_renderer::apply_fade_alpha(bg, fade);
    let faded_stroke = track_renderer::apply_fade_alpha(stroke_color, fade);
    let radius = if highlighted { 11.0 } else { 8.0 };
    painter.circle_filled(center, radius, faded_bg);
    painter.circle_stroke(center, radius, Stroke::new(1.5, faded_stroke));
    if highlighted {
        painter.circle_stroke(
            center,
            radius + 3.5,
            Stroke::new(1.5, Color32::from_rgb(100, 200, 255)),
        );
    }
    let s = 4.0;
    match kind {
        gt_types::GeneratedMarkerKind::GnssFixLost => {
            let st = Stroke::new(2.0, faded_stroke);
            painter.line_segment([center - egui::vec2(s, s), center + egui::vec2(s, s)], st);
            painter.line_segment([center + egui::vec2(-s, s), center + egui::vec2(s, -s)], st);
        }
        gt_types::GeneratedMarkerKind::GnssFixRegained { .. } => {
            let st = Stroke::new(2.0, faded_stroke);
            painter.line_segment(
                [
                    center + egui::vec2(-s, 0.0),
                    center + egui::vec2(-s * 0.3, s),
                ],
                st,
            );
            painter.line_segment(
                [center + egui::vec2(-s * 0.3, s), center + egui::vec2(s, -s)],
                st,
            );
        }
        gt_types::GeneratedMarkerKind::ClockDiscontinuity { .. } => {
            // Exclamation mark: an anomaly to inspect.
            let st = Stroke::new(2.0, faded_stroke);
            painter.line_segment(
                [
                    center - egui::vec2(0.0, s),
                    center + egui::vec2(0.0, s * 0.2),
                ],
                st,
            );
            painter.circle_filled(center + egui::vec2(0.0, s * 0.85), 1.3, faded_stroke);
        }
        gt_types::GeneratedMarkerKind::Slip(_) => {
            // Broken chain link: a lost connection (loss of lock). Drawn from a
            // cached SVG texture (one image quad), not per-frame line
            // tessellation, so a dense cluster of slip markers stays cheap. Sized
            // to nearly fill the disc - the chain detail needs more room than the
            // simple line glyphs above to read.
            let icon_extent = if highlighted { 4.0 * s } else { 3.4 * s };
            let icon = egui::Rect::from_center_size(center, egui::Vec2::splat(icon_extent));
            crate::icons::draw_cached_icon(
                ui,
                crate::icons::ICON_URI_CONNECTION_LOST,
                icon,
                faded_stroke,
            );
        }
    }
}

#[cfg(test)]
mod snapshot_tests {
    use super::show_slip_table;
    use crate::test_harness::TestHarness;
    use gt_types::satellites::{Constellation, SatSample, Slip, SlipCause, SlipEvent, Snr};

    fn sample(elevation: Option<f32>, azimuth: Option<f32>, snr: Option<f32>) -> SatSample {
        SatSample {
            elevation,
            azimuth,
            snr: snr.map(Snr::new),
        }
    }

    /// A slip event mixing both causes, constellations out of order, an
    /// unchanged azimuth, and an all-unknown row, to exercise sorting, the
    /// constellation tint, the before/after collapse, and the missing-value dash.
    fn mixed_slip_event() -> SlipEvent {
        SlipEvent {
            slips: vec![
                Slip {
                    constellation: Constellation::Beidou,
                    prn: gt_types::satellites::Prn::new(14),
                    cause: SlipCause::SnrDrop,
                    from: sample(Some(31.0), Some(120.0), Some(46.0)),
                    to: Some(sample(Some(30.0), Some(122.0), Some(29.0))),
                },
                Slip {
                    constellation: Constellation::Gps,
                    prn: gt_types::satellites::Prn::new(7),
                    cause: SlipCause::LostLock,
                    from: sample(Some(40.0), Some(205.0), Some(48.0)),
                    to: None,
                },
                Slip {
                    constellation: Constellation::Gps,
                    prn: gt_types::satellites::Prn::new(2),
                    cause: SlipCause::SnrDrop,
                    from: sample(Some(22.0), Some(88.0), Some(41.0)),
                    // Elevation and azimuth unchanged -> collapse to a single value.
                    to: Some(sample(Some(22.0), Some(88.0), Some(27.0))),
                },
                Slip {
                    constellation: Constellation::Galileo,
                    prn: gt_types::satellites::Prn::new(3),
                    cause: SlipCause::LostLock,
                    // Everything unknown -> every cell a lone dash, no arrows.
                    from: sample(None, None, None),
                    to: None,
                },
            ],
        }
    }

    #[test]
    fn slip_table_dark() {
        let event = mixed_slip_event();
        let mut harness = TestHarness::builder()
            .size(egui::vec2(420.0, 200.0))
            .theme(true)
            .ui(move |ui| {
                show_slip_table(ui, &event);
            });
        harness.run();
        harness.snapshot("slip_detail_table_dark");
    }

    #[test]
    fn slip_table_light() {
        let event = mixed_slip_event();
        let mut harness = TestHarness::builder()
            .size(egui::vec2(420.0, 200.0))
            .theme(false)
            .ui(move |ui| {
                show_slip_table(ui, &event);
            });
        harness.run();
        harness.snapshot("slip_detail_table_light");
    }
}
