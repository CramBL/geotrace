//! Hover and disambiguation label UI shared by the map's hit-test layer:
//! compound multi-hover labels, candidate resolution, and row rendering
//! for the click-disambiguation popup.

use egui::Frame;
use egui_phosphor::regular::ARROWS_SPLIT as ICON_ARROWS_SPLIT;
use egui_phosphor::regular::CROSSHAIR as ICON_CROSSHAIR;
use egui_phosphor::regular::FLAG as ICON_FLAG;
use egui_phosphor::regular::MAP_PIN as ICON_MAP_PIN;
use gt_types::{
    CustomMarker, DataCategory, EventMarker, GeneratedMarker, LoadedFile, LoadedTrack, NavPoint,
    PointIdx,
};
use gt_ui_theme::EM_DASH;
use gt_ui_types::DataPointRef;

use crate::recording_labels::RecordingLabels;

/// Spacing between an icon and the text following it in labels.
const ICON_GAP: &str = "  ";

/// Alpha of the hover band drawn over a sky-plot highlight target, low enough
/// to keep the text underneath legible.
const HOVER_BAND_ALPHA: f32 = 0.3;
/// The band's outward pad and corner rounding around the element's rect.
const HOVER_BAND_PAD_PX: f32 = 2.0;
const HOVER_BAND_ROUNDING_PX: f32 = 3.0;

/// Whether `rect` is hovered, and, when it is, applies the interactive
/// affordance: a subtle highlight band over the element plus the pointer
/// cursor, so a satellite-table target that drives the sky-plot highlight
/// reads as interactive rather than plain text. Shared by the sticky point
/// popup and the trails window.
///
/// The band is painted on top of the element (at low alpha, so the text stays
/// legible) rather than behind it. The sticky popup is an [`egui::Window`]
/// (`Order::Middle`), so a band on a background-order layer - the technique
/// `gt_side_panel::paint_map_hover_bg` uses for its panel rows, which are not
/// inside a window - would render behind the window frame and vanish.
pub(crate) fn hover_affordance(ui: &egui::Ui, rect: egui::Rect) -> bool {
    if !ui.rect_contains_pointer(rect) {
        return false;
    }
    paint_hover_band(ui, rect);
    true
}

/// [`hover_affordance`] for one row of a stacked list, hit-testing a band that
/// reaches halfway into the gaps above and below so consecutive rows tile
/// without a dead strip between them.
///
/// Rows are laid out with spacing between them, and a plain rect hit-test
/// leaves that spacing hovering nothing. Dragging the pointer down a satellite
/// table would then drop the sky-plot highlight in every gap, so the plot
/// flashed back to full strength between one satellite and the next. Bridging
/// the gap hands the highlight straight from one row to the next. The band is
/// still painted at the row's own rect, so the rows stay visually separate.
///
/// A full spacing each side, not half: a grid leaves two spacings between
/// consecutive rows, so half would still leave the middle uncovered. Reaching
/// too far is harmless - where two rows overlap the lower one wins, which is
/// still a clean hand-off - whereas reaching too short brings the flicker back.
pub(crate) fn row_hover_affordance(ui: &egui::Ui, rect: egui::Rect) -> bool {
    let bridge = ui.spacing().item_spacing.y;
    if !ui.rect_contains_pointer(rect.expand2(egui::vec2(0.0, bridge))) {
        return false;
    }
    paint_hover_band(ui, rect);
    true
}

/// The shared hover band and pointer cursor.
fn paint_hover_band(ui: &egui::Ui, rect: egui::Rect) {
    let band = ui
        .visuals()
        .selection
        .bg_fill
        .gamma_multiply(HOVER_BAND_ALPHA);
    ui.painter()
        .rect_filled(rect.expand(HOVER_BAND_PAD_PX), HOVER_BAND_ROUNDING_PX, band);
    ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
}

/// Returns `true` when the multi-hover compound label should be drawn.
///
/// `current_multi_hover`, more than one candidate hovered this frame.
/// `disambig_open`, the disambiguation popup is open.
/// `suppress_hover_labels`, set from the previous frame's candidate count.
/// False on the transition frame so the compound label and individual
/// renderer tooltips never appear simultaneously.
#[expect(
    clippy::fn_params_excessive_bools,
    reason = "three independent boolean inputs to the guard"
)]
pub(crate) fn should_show_compound_label(
    current_multi_hover: bool,
    disambig_open: bool,
    suppress_hover_labels: bool,
) -> bool {
    current_multi_hover && !disambig_open && suppress_hover_labels
}

/// Renders the sections of a multi-hover stacked label into `ui`.
///
/// Each section is wrapped in its own `Frame::popup` so the items appear as
/// distinct opaque cards. The caller should NOT wrap this in an outer frame,
/// the popup frames provide all the visual containment needed.
pub(crate) fn draw_multi_hover_label_contents(
    ui: &mut egui::Ui,
    candidates: &[Option<DataPointRef>; 4],
    files: &[LoadedFile],
    recording_labels: RecordingLabels<'_>,
) {
    for candidate in candidates.iter().flatten().copied() {
        Frame::popup(ui.style()).show(ui, |ui| {
            draw_candidate_section(ui, candidate, files, recording_labels);
        });
    }
}

enum ResolvedCandidate<'a> {
    Tpv {
        point: &'a NavPoint,
        track: &'a LoadedTrack,
        point_index: PointIdx,
    },
    GeneratedMarker(&'a GeneratedMarker),
    EventMarker(&'a EventMarker),
    CustomMarker(&'a CustomMarker),
}

fn resolve_candidate<'a>(
    candidate: DataPointRef,
    files: &'a [LoadedFile],
) -> Option<ResolvedCandidate<'a>> {
    let file = candidate.track.fi.get(files)?;
    let track = candidate.track.index.get(&file.tracks)?;
    Some(match candidate.category {
        DataCategory::Tpv | DataCategory::SatelliteReport => ResolvedCandidate::Tpv {
            point: candidate.point_index.get(&track.points)?,
            track,
            point_index: candidate.point_index,
        },
        DataCategory::GeneratedMarker => {
            ResolvedCandidate::GeneratedMarker(candidate.point_index.get(&track.generated_markers)?)
        }
        DataCategory::EventMarker => {
            ResolvedCandidate::EventMarker(candidate.point_index.get(&track.event_markers)?)
        }
        DataCategory::CustomMarker => {
            ResolvedCandidate::CustomMarker(candidate.point_index.get(&track.custom_markers)?)
        }
        DataCategory::Track => return None,
    })
}

/// Renders a single candidate's section inside the multi-hover label.
///
/// Shows a header line (icon + summary) for every type, plus type-specific body
/// content: the full hover table for TPV points, and the duration for GNSS-fix-
/// regained markers.
fn draw_candidate_section(
    ui: &mut egui::Ui,
    candidate: DataPointRef,
    files: &[LoadedFile],
    recording_labels: RecordingLabels<'_>,
) {
    let icon = category_icon(candidate.category);
    match resolve_candidate(candidate, files) {
        None => {
            let fallback = match candidate.category {
                DataCategory::Tpv | DataCategory::SatelliteReport => "GNSS fix",
                DataCategory::EventMarker => "Event marker",
                DataCategory::CustomMarker => "Custom marker",
                DataCategory::GeneratedMarker => "Generated marker",
                DataCategory::Track => "",
            };
            ui.strong(format!("{icon}{ICON_GAP}{fallback}"));
        }
        Some(ResolvedCandidate::Tpv {
            point,
            track,
            point_index,
        }) => {
            ui.strong(format!(
                "{icon}{ICON_GAP}GNSS fix{ICON_GAP}{}",
                point.tpv.time().utc().format("%H:%M:%S")
            ));
            crate::tpv_renderer::show_hover_table(
                ui,
                point,
                &crate::tpv_renderer::SkySection::resolve(track, point_index),
                recording_labels.name_when_several_files_loaded(candidate.track.fi),
            );
        }
        Some(ResolvedCandidate::GeneratedMarker(marker)) => {
            ui.strong(format!(
                "{icon}{ICON_GAP}{}",
                crate::generated_marker_renderer::generated_marker_header(&marker.kind)
            ));
        }
        Some(ResolvedCandidate::EventMarker(m)) => match &m.annotation {
            Some(note) if !note.is_empty() => {
                ui.label(format!(
                    "{icon}{ICON_GAP}{}{ICON_GAP}{EM_DASH}{ICON_GAP}{note}",
                    m.variant_path
                ));
            }
            _ => {
                ui.label(format!("{icon}{ICON_GAP}{}", m.variant_path));
            }
        },
        Some(ResolvedCandidate::CustomMarker(m)) => {
            ui.label(format!("{icon}{ICON_GAP}{}", m.label));
        }
    }
}

/// Renders a single row of the disambiguation popup.
///
/// Returns the response from `selectable_label` so the caller can check `.clicked()`.
pub(crate) fn draw_disambig_row(
    ui: &mut egui::Ui,
    candidate: DataPointRef,
    files: &[LoadedFile],
    is_selected: bool,
) -> egui::Response {
    let icon = category_icon(candidate.category);
    let label = candidate_label(candidate, files);
    let mut job = egui::text::LayoutJob::default();
    let text_color = ui.visuals().text_color();
    job.append(
        icon,
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::proportional(20.0),
            color: text_color,
            valign: egui::Align::Center,
            ..Default::default()
        },
    );
    job.append(
        &format!("{ICON_GAP}{label}"),
        0.0,
        egui::TextFormat {
            font_id: egui::FontId::proportional(13.0),
            color: text_color,
            valign: egui::Align::Center,
            ..Default::default()
        },
    );
    ui.selectable_label(is_selected, job)
}

pub(crate) fn category_icon(cat: DataCategory) -> &'static str {
    match cat {
        DataCategory::Tpv | DataCategory::SatelliteReport => ICON_CROSSHAIR,
        DataCategory::EventMarker => ICON_FLAG,
        DataCategory::CustomMarker => ICON_MAP_PIN,
        DataCategory::GeneratedMarker => ICON_ARROWS_SPLIT,
        DataCategory::Track => "",
    }
}

pub(crate) fn candidate_label(candidate: DataPointRef, files: &[LoadedFile]) -> String {
    match resolve_candidate(candidate, files) {
        None => match candidate.category {
            DataCategory::Tpv | DataCategory::SatelliteReport => "GNSS fix".to_owned(),
            DataCategory::EventMarker => "Event marker".to_owned(),
            DataCategory::CustomMarker => "Custom marker".to_owned(),
            DataCategory::GeneratedMarker => "Generated marker".to_owned(),
            DataCategory::Track => String::new(),
        },
        Some(ResolvedCandidate::Tpv { point, .. }) => {
            format!(
                "GNSS fix{ICON_GAP}{}",
                point.tpv.time().utc().format("%H:%M:%S")
            )
        }
        Some(ResolvedCandidate::EventMarker(m)) => match &m.annotation {
            Some(note) if !note.is_empty() => {
                format!("{}{ICON_GAP}{EM_DASH}{ICON_GAP}{note}", m.variant_path)
            }
            _ => m.variant_path.clone(),
        },
        Some(ResolvedCandidate::CustomMarker(m)) => m.label.clone(),
        Some(ResolvedCandidate::GeneratedMarker(m)) => {
            crate::generated_marker_renderer::generated_marker_header(&m.kind)
        }
    }
}
