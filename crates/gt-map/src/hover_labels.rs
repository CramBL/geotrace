//! The map's hover labels: the stack every layer under the pointer writes
//! into, the compound label several recorded elements share, candidate
//! resolution, and the rows of the click-disambiguation popup.

use std::cell::RefCell;

use egui::{Frame, PopupAnchor, Style, Tooltip};
use egui_phosphor::regular::ARROWS_SPLIT as ICON_ARROWS_SPLIT;
use egui_phosphor::regular::CROSSHAIR as ICON_CROSSHAIR;
use egui_phosphor::regular::FLAG as ICON_FLAG;
use egui_phosphor::regular::MAP_PIN as ICON_MAP_PIN;
use gt_types::{
    CustomMarker, DataCategory, EventMarker, GeneratedMarker, LoadedFile, LoadedTrack, PlacedPoint,
    PointIdx,
};
use gt_ui_theme::EM_DASH;
use gt_ui_types::{DataPointRef, HoverCandidates, MapHighlight, QueryMatches};

use crate::jamming_renderer::InterferenceCellLabel;
use crate::log_match_renderer::LogHexagonLabel;
use crate::recording_labels::RecordingLabels;
use crate::tec_renderer::TecNodeLabel;
use crate::{
    event_marker_renderer, generated_marker_renderer, marker_renderer, query_match_renderer,
    tpv_renderer,
};

/// The parent widget id the stacked labels share, which is what makes egui
/// place each of them below the ones already shown. It is the stack's own and
/// not the map response's: the snapped edge's label is anchored at that
/// response, and would otherwise push the stack below the map.
const HOVER_LABEL_STACK_ID: &str = "map_hover_label_stack";

/// The map layer a hover label comes from. The declaration order is the
/// order the map registers its plugins in, reversed: the marker pins drawn
/// last are at the top of the stack and the TEC grid drawn first at its
/// bottom.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum HoverLabelLayer {
    Marker,
    LogHexagon,
    Fix,
    InterferenceCell,
    TecNode,
}

/// What the stack reads a recorded element's label out of once the map has
/// drawn.
#[derive(Clone, Copy)]
pub(crate) struct HoverLabelSources<'a> {
    pub(crate) files: &'a [LoadedFile],
    pub(crate) recording_labels: RecordingLabels<'a>,
    pub(crate) query_matches: Option<&'a QueryMatches>,
}

impl HoverLabelSources<'_> {
    /// The header of the match a hovered fix lies in, above that fix's table.
    fn show_match_header(self, ui: &mut egui::Ui, candidate: DataPointRef) {
        let Some(matches) = self.query_matches else {
            return;
        };
        let Some(range) = matches.header_range(candidate.track, candidate.point_index.as_usize())
        else {
            return;
        };
        query_match_renderer::match_header_ui(
            ui,
            self.files,
            candidate.track,
            range,
            matches.stale,
        );
    }
}

/// One layer's label, holding what that layer found under the pointer.
pub(crate) enum HoverLabelEntry {
    RecordedElement(RecordedElementLabel),
    LogHexagon(LogHexagonLabel),
    InterferenceCell(InterferenceCellLabel),
    TecNode(TecNodeLabel),
}

impl HoverLabelEntry {
    fn layer(&self) -> HoverLabelLayer {
        match self {
            Self::RecordedElement(RecordedElementLabel::Compound(_)) => HoverLabelLayer::Marker,
            Self::RecordedElement(RecordedElementLabel::One(candidate)) => {
                match candidate.category {
                    DataCategory::Tpv | DataCategory::SatelliteReport => HoverLabelLayer::Fix,
                    DataCategory::EventMarker
                    | DataCategory::CustomMarker
                    | DataCategory::GeneratedMarker
                    | DataCategory::Track => HoverLabelLayer::Marker,
                }
            }
            Self::LogHexagon(_) => HoverLabelLayer::LogHexagon,
            Self::InterferenceCell(_) => HoverLabelLayer::InterferenceCell,
            Self::TecNode(_) => HoverLabelLayer::TecNode,
        }
    }

    /// The frame the stack draws this entry's tooltip in. The compound label
    /// draws a card per element it stands for, and takes none of its own.
    fn frame(&self, style: &Style) -> Frame {
        match self {
            Self::RecordedElement(RecordedElementLabel::Compound(_)) => Frame::NONE,
            Self::RecordedElement(RecordedElementLabel::One(_))
            | Self::LogHexagon(_)
            | Self::InterferenceCell(_)
            | Self::TecNode(_) => Frame::popup(style),
        }
    }

    fn show(self, ui: &mut egui::Ui, sources: HoverLabelSources<'_>) {
        match self {
            Self::RecordedElement(label) => label.show(ui, sources),
            Self::LogHexagon(label) => label.show(ui),
            Self::InterferenceCell(label) => label.show(ui),
            Self::TecNode(label) => label.show(ui),
        }
    }
}

/// What the recorded elements under the pointer show: the compound label when
/// several of them are there, and the one element's own label otherwise.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum RecordedElementLabel {
    Compound(HoverCandidates),
    One(DataPointRef),
}

impl RecordedElementLabel {
    fn show(self, ui: &mut egui::Ui, sources: HoverLabelSources<'_>) {
        match self {
            Self::Compound(candidates) => {
                draw_multi_hover_label_contents(
                    ui,
                    candidates,
                    sources.files,
                    sources.recording_labels,
                );
            }
            Self::One(candidate) => show_element_label(ui, candidate, sources),
        }
    }
}

/// The label of the one recorded element under the pointer, as its own
/// renderer writes it.
fn show_element_label(ui: &mut egui::Ui, candidate: DataPointRef, sources: HoverLabelSources<'_>) {
    let recording_name = sources
        .recording_labels
        .name_when_several_files_loaded(candidate.track.fi);
    match resolve_candidate(candidate, sources.files) {
        None => {}
        Some(ResolvedCandidate::Tpv {
            point,
            track,
            point_index,
        }) => {
            sources.show_match_header(ui, candidate);
            tpv_renderer::show_hover_table(
                ui,
                point,
                &tpv_renderer::SkySection::resolve(track, point_index),
                recording_name,
            );
        }
        Some(ResolvedCandidate::EventMarker(marker)) => {
            event_marker_renderer::show_hover_label(ui, marker);
        }
        Some(ResolvedCandidate::CustomMarker(marker)) => {
            marker_renderer::show_hover_label(ui, marker);
        }
        Some(ResolvedCandidate::GeneratedMarker { marker, track }) => {
            generated_marker_renderer::show_hover_label(ui, track, marker, recording_name);
        }
    }
}

/// Which popups stand between an element under the pointer and its label.
#[derive(Clone, Copy, Default)]
pub(crate) struct OpenPopups {
    /// The map's own disambiguation popup, which lists the candidates a click
    /// could not resolve.
    pub(crate) disambiguation: bool,
    /// Any egui popup, which the map's context menu is one of.
    pub(crate) egui_popup_was_open_last_frame: bool,
}

impl OpenPopups {
    pub(crate) fn a_popup_owns_the_pointer(self) -> bool {
        self.disambiguation || self.egui_popup_was_open_last_frame
    }
}

/// Which recorded element's label the stack takes this frame, and `None`
/// where none of them shows one.
///
/// An individual label states what [`MapHighlight::hover_candidates`] holds,
/// which is the previous frame's hit test. `current_candidates` are this
/// frame's. The two kinds of label are never drawn together: the compound
/// label replaces the individual ones only once both frames have several
/// elements under the pointer.
pub(crate) fn recorded_element_label(
    highlight: &MapHighlight,
    current_candidates: HoverCandidates,
    popups: OpenPopups,
) -> Option<RecordedElementLabel> {
    if current_candidates.is_ambiguous()
        && highlight.suppress_hover_labels
        && !popups.disambiguation
    {
        return Some(RecordedElementLabel::Compound(current_candidates));
    }
    let candidate = highlight.hover_candidates.primary()?;
    highlight
        .shows_hover_label(candidate, popups.egui_popup_was_open_last_frame)
        .then_some(RecordedElementLabel::One(candidate))
}

/// The labels one frame has under the pointer: the overlay plugins push while
/// they draw, and the map pushes the recorded element's own once its hit test
/// has run.
#[derive(Default)]
pub(crate) struct HoverLabelStack {
    entries: RefCell<Vec<HoverLabelEntry>>,
}

impl HoverLabelStack {
    pub(crate) fn push(&self, entry: HoverLabelEntry) {
        self.entries.borrow_mut().push(entry);
    }

    /// Draws the frame's labels stacked at the pointer, the topmost layer's
    /// first, and empties the stack for the next frame. While a popup owns
    /// the pointer it draws none of them, and empties the stack all the same.
    ///
    /// The call order is the stacking order: egui places each further
    /// tooltip of one parent widget below the bounding rect of the ones
    /// already shown for it. A stack too tall for the room below the pointer
    /// is cut off by the screen.
    pub(crate) fn show_at_the_pointer(
        &self,
        ui: &egui::Ui,
        sources: HoverLabelSources<'_>,
        popups: OpenPopups,
    ) {
        let mut entries = self.entries.take();
        if popups.a_popup_owns_the_pointer() {
            return;
        }
        entries.sort_by_key(HoverLabelEntry::layer);
        for entry in entries {
            let mut tooltip = Tooltip::always_open(
                ui.ctx().clone(),
                ui.layer_id(),
                egui::Id::new(HOVER_LABEL_STACK_ID),
                PopupAnchor::Pointer,
            )
            .gap(TOOLTIP_POINTER_GAP_PX);
            tooltip.popup = tooltip.popup.frame(entry.frame(ui.style()));
            tooltip.show(|ui| entry.show(ui, sources));
        }
    }
}

/// Spacing between an icon and the text following it in labels.
const ICON_GAP: &str = "  ";

/// Gap between the pointer and the first label of the stack, and between one
/// stacked label and the next.
const TOOLTIP_POINTER_GAP_PX: f32 = 12.0;

/// Alpha of the hover band drawn over a sky-plot highlight target, low enough
/// to keep the text underneath legible.
const HOVER_BAND_ALPHA: f32 = 0.3;
/// The band's outward pad and corner rounding around the element's rect.
const HOVER_BAND_PAD_PX: f32 = 2.0;
const HOVER_BAND_ROUNDING_PX: f32 = 3.0;

/// Whether `rect` is hovered, and, when it is, paints a highlight band over
/// the element and sets the pointer cursor.
///
/// The band is painted on top of the element at low alpha. The sticky popup is
/// an [`egui::Window`] (`Order::Middle`), so a band on a background-order layer
/// would render behind the window frame and vanish.
pub(crate) fn hover_affordance(ui: &egui::Ui, rect: egui::Rect) -> bool {
    if !ui.rect_contains_pointer(rect) {
        return false;
    }
    paint_hover_band(ui, rect);
    true
}

/// [`hover_affordance`] for one row of a stacked list, hit-testing a band that
/// covers the gaps above and below so consecutive rows tile without a dead
/// strip between them. The band is painted at the row's own rect, so the rows
/// stay visually separate.
///
/// A full spacing each side, not half: a grid leaves two spacings between
/// consecutive rows, so half would leave the middle uncovered. Where two rows
/// overlap the lower one wins.
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

/// Renders the sections of a multi-hover stacked label into `ui`.
///
/// Each section is wrapped in its own `Frame::popup` so the items appear as
/// distinct opaque cards, which is the containment they need: the caller adds
/// no frame of its own around them.
pub(crate) fn draw_multi_hover_label_contents(
    ui: &mut egui::Ui,
    candidates: HoverCandidates,
    files: &[LoadedFile],
    recording_labels: RecordingLabels<'_>,
) {
    for candidate in candidates.iter() {
        Frame::popup(ui.style()).show(ui, |ui| {
            draw_candidate_section(ui, candidate, files, recording_labels);
        });
    }
}

enum ResolvedCandidate<'a> {
    Tpv {
        point: PlacedPoint<'a>,
        track: &'a LoadedTrack,
        point_index: PointIdx,
    },
    GeneratedMarker {
        marker: &'a GeneratedMarker,
        /// The track the marker was derived from, whose fix at the marker's
        /// instant its label states.
        track: &'a LoadedTrack,
    },
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
            point: track
                .placed_points()?
                .get(candidate.point_index.as_usize())?,
            track,
            point_index: candidate.point_index,
        },
        DataCategory::GeneratedMarker => ResolvedCandidate::GeneratedMarker {
            marker: candidate.point_index.get(&track.generated_markers)?,
            track,
        },
        DataCategory::EventMarker => {
            ResolvedCandidate::EventMarker(candidate.point_index.get(&track.event_markers)?)
        }
        DataCategory::CustomMarker => {
            ResolvedCandidate::CustomMarker(candidate.point_index.get(&track.custom_markers)?)
        }
        DataCategory::Track => return None,
    })
}

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
                point.fix.tpv.time().utc().format("%H:%M:%S")
            ));
            tpv_renderer::show_hover_table(
                ui,
                point,
                &tpv_renderer::SkySection::resolve(track, point_index),
                recording_labels.name_when_several_files_loaded(candidate.track.fi),
            );
        }
        Some(ResolvedCandidate::GeneratedMarker { marker, .. }) => {
            ui.strong(format!(
                "{icon}{ICON_GAP}{}",
                generated_marker_renderer::generated_marker_header(&marker.kind)
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
                point.fix.tpv.time().utc().format("%H:%M:%S")
            )
        }
        Some(ResolvedCandidate::EventMarker(m)) => match &m.annotation {
            Some(note) if !note.is_empty() => {
                format!("{}{ICON_GAP}{EM_DASH}{ICON_GAP}{note}", m.variant_path)
            }
            _ => m.variant_path.clone(),
        },
        Some(ResolvedCandidate::CustomMarker(m)) => m.label.clone(),
        Some(ResolvedCandidate::GeneratedMarker { marker, .. }) => {
            generated_marker_renderer::generated_marker_header(&marker.kind)
        }
    }
}

#[cfg(test)]
mod tests {
    use gt_types::{DataCategory, FileIdx, PointIdx, TrackIdx, TrackRef};
    use gt_ui_types::{DataPointRef, HoverCandidates, MapHighlight};
    use rstest::rstest;

    use super::{OpenPopups, RecordedElementLabel, recorded_element_label};

    const FIX: DataCategory = DataCategory::Tpv;
    const EVENT_MARKER: DataCategory = DataCategory::EventMarker;

    fn candidate(category: DataCategory) -> DataPointRef {
        DataPointRef {
            track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
            category,
            point_index: PointIdx::new(0),
        }
    }

    fn candidates(categories: &[DataCategory]) -> HoverCandidates {
        let mut candidates = HoverCandidates::default();
        for &category in categories {
            candidates.keep_nearest(candidate(category));
        }
        candidates
    }

    /// What a case varies beside the candidates of the two frames.
    #[derive(Default)]
    struct FrameInputs {
        /// Whether several elements were under the pointer on the previous
        /// frame, which is what the map suppresses the individual labels on.
        settled_multi_hover: bool,
        pinned: Option<DataPointRef>,
        popups: OpenPopups,
    }

    #[rstest]
    #[case::nothing_under_the_pointer(&[], &[], FrameInputs::default(), None)]
    #[case::one_element(
        &[FIX],
        &[FIX],
        FrameInputs::default(),
        Some(RecordedElementLabel::One(candidate(FIX)))
    )]
    #[case::the_element_under_the_pointer_is_pinned(
        &[FIX],
        &[FIX],
        FrameInputs { pinned: Some(candidate(FIX)), ..FrameInputs::default() },
        None
    )]
    #[case::another_element_is_pinned(
        &[FIX],
        &[FIX],
        FrameInputs { pinned: Some(candidate(EVENT_MARKER)), ..FrameInputs::default() },
        Some(RecordedElementLabel::One(candidate(FIX)))
    )]
    #[case::the_context_menu_is_open(
        &[FIX],
        &[FIX],
        FrameInputs {
            popups: OpenPopups {
                egui_popup_was_open_last_frame: true,
                ..OpenPopups::default()
            },
            ..FrameInputs::default()
        },
        None
    )]
    #[case::two_elements_on_the_frame_the_pointer_reaches_them(
        &[],
        &[FIX, EVENT_MARKER],
        FrameInputs::default(),
        None
    )]
    #[case::two_elements_once_settled(
        &[FIX, EVENT_MARKER],
        &[FIX, EVENT_MARKER],
        FrameInputs { settled_multi_hover: true, ..FrameInputs::default() },
        Some(RecordedElementLabel::Compound(candidates(&[FIX, EVENT_MARKER])))
    )]
    #[case::two_elements_under_the_disambiguation_popup(
        &[FIX, EVENT_MARKER],
        &[FIX, EVENT_MARKER],
        FrameInputs {
            settled_multi_hover: true,
            popups: OpenPopups { disambiguation: true, ..OpenPopups::default() },
            ..FrameInputs::default()
        },
        None
    )]
    fn the_stack_takes_the_compound_label_only_once_several_candidates_have_settled(
        #[case] previous_categories: &[DataCategory],
        #[case] current_categories: &[DataCategory],
        #[case] frame: FrameInputs,
        #[case] expected: Option<RecordedElementLabel>,
    ) {
        let highlight = MapHighlight {
            hover_candidates: candidates(previous_categories),
            sticky: frame.pinned,
            suppress_hover_labels: frame.settled_multi_hover || frame.popups.disambiguation,
            ..MapHighlight::default()
        };

        assert_eq!(
            recorded_element_label(&highlight, candidates(current_categories), frame.popups),
            expected
        );
    }
}
