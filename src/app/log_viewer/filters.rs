//! The live filter over the shown log and the chips added from it: the field
//! with its `.*` toggle and match count, and one chip per added filter.

use std::num::NonZeroUsize;

use egui::{Button, Checkbox, Frame, Label, RichText, StrokeKind, TextEdit};
use egui_phosphor::regular::FUNNEL as ICON_FUNNEL;
use egui_phosphor::regular::PLUS_CIRCLE as ICON_PLUS_CIRCLE;
use egui_phosphor::regular::X as ICON_X;
use gt_log_view::{
    FilterChip, FilterChipId, FilterChipMode, FilterStack, LayerColorSlots, LoadedLogs,
};
use gt_ui_types::LoadedLogId;

use super::LogViewerWindow;

/// Id of the live-filter field. A test puts the keyboard into it by this id,
/// whatever else is on screen.
pub(in crate::app) const LIVE_FILTER_FIELD_ID: &str = "log_viewer_live_filter";

const FIELD_HINT: &str = "Filter lines";

const FIELD_HOVER: &str = "Show the lines whose message holds every term written here";

/// Width of the live-filter field, wide enough for the several terms a filter
/// usually holds.
const FIELD_WIDTH_PX: f32 = 260.0;

pub(super) const REGEX_TOGGLE_LABEL: &str = ".*";

const REGEX_TOGGLE_HOVER: &str = "Read the field as a regular expression instead of a set of terms";

pub(in crate::app) const ADD_FILTER_LABEL: &str = "+ Add filter";

const ADD_FILTER_HOVER: &str = "Keep the live filter as a chip and empty the field";

pub(super) const ADD_FILTER_EMPTY_HOVER: &str = "Write a live filter to add it as a chip";

pub(super) const ADD_FILTER_INVALID_HOVER: &str =
    "The live filter is added once its regular expression compiles";

const CLEAR_LABEL: &str = "Clear";

const CLEAR_HOVER: &str = "Empty the live filter";

const CLEAR_EMPTY_HOVER: &str = "The live filter is empty already";

const MATCH_COUNT_HOVER: &str = "Lines the filters show, of the log's entries";

/// What the viewer says while a scan of the log is still running. U+2026
/// HORIZONTAL ELLIPSIS marks the work in flight.
pub(in crate::app) const PENDING_NOTE: &str = "Filtering…";

/// How long a scan has to run before the viewer says it is running. Below this
/// the note would only flicker.
const PENDING_NOTE_DELAY_SECS: f64 = 0.1;

/// Characters of a chip's filter text the chip shows, the rest on hover.
const CHIP_TEXT_CHARS: NonZeroUsize = match NonZeroUsize::new(28) {
    Some(chars) => chars,
    None => NonZeroUsize::MIN,
};

const CHIP_CORNER_RADIUS: u8 = 10;

const CHIP_INNER_MARGIN: egui::Margin = egui::Margin::symmetric(8, 2);

const CHIP_BORDER_WIDTH_PX: f32 = 1.0;

const CHIP_DASH_LENGTH_PX: f32 = 3.0;

const CHIP_DASH_GAP_PX: f32 = 2.0;

/// Side of the square a layer chip shows its palette colour in.
const CHIP_SWATCH_SIZE_PX: f32 = 10.0;

const CHIP_SWATCH_CORNER_RADIUS: u8 = 1;

/// Width of the ring drawn around the swatch of a chip sharing its colour with
/// another one.
const CHIP_SHARED_SWATCH_RING_PX: f32 = 2.0;

const SHARED_SWATCH_HOVER: &str = "Another filter draws in this colour too";

const LAYER_CHIP_HOVER: &str = "Draw the lines this filter matches";

const REFINE_CHIP_HOVER: &str = "Narrow the table to the lines this filter matches";

const SWITCH_TO_LAYER_HOVER: &str =
    "Switch to layer mode: this filter colours the lines it matches without narrowing the table";

const SWITCH_TO_REFINE_HOVER: &str =
    "Switch to refine mode: only the lines this filter matches stay in the table";

const REMOVE_CHIP_HOVER: &str = "Remove this filter";

/// The edit the filter row or the chip row produced while rendering. It reaches
/// the engine once rendering has finished: every chip of a frame is drawn from
/// one state.
enum FilterEdit {
    WriteLiveFilter(String),
    ReadLiveFilterAsRegex(bool),
    ClearLiveFilter,
    AddLiveFilterAsChip,
    SetChipEnabled {
        chip: FilterChipId,
        enabled: bool,
    },
    SwitchChipMode {
        chip: FilterChipId,
        to: FilterChipMode,
    },
    RemoveChip(FilterChipId),
}

impl LogViewerWindow {
    /// The live filter over the shown log, and the chips added from it.
    pub(super) fn filters_ui(
        &mut self,
        ui: &mut egui::Ui,
        logs: &mut LoadedLogs,
        shown: LoadedLogId,
    ) {
        let Some(log) = logs.get_by_id(shown) else {
            return;
        };
        let filters = log.filters();
        let edit = self
            .filter_row_ui(ui, filters)
            .or_else(|| chip_row_ui(ui, filters, logs.layer_color_slots()));

        let (Some(edit), Some((stack, slots))) = (edit, logs.filter_stack_mut_by_id(shown)) else {
            return;
        };
        match edit {
            FilterEdit::WriteLiveFilter(text) => stack.set_live_filter_text(&text),
            FilterEdit::ReadLiveFilterAsRegex(regex) => stack.set_live_filter_regex(regex),
            FilterEdit::ClearLiveFilter => stack.clear_live_filter(),
            FilterEdit::AddLiveFilterAsChip => {
                stack.add_live_filter_as_chip(slots);
            }
            FilterEdit::SetChipEnabled { chip, enabled } => stack.set_chip_enabled(chip, enabled),
            FilterEdit::SwitchChipMode { chip, to } => match to {
                FilterChipMode::Layer => stack.switch_chip_to_layer_mode(chip, slots),
                FilterChipMode::Refine => stack.switch_chip_to_refine_mode(chip, slots),
            },
            FilterEdit::RemoveChip(chip) => stack.remove_chip(chip, slots),
        }
    }

    /// The field the user filters the log from, what its pattern selected, and
    /// the controls turning it into a chip or emptying it.
    fn filter_row_ui(&mut self, ui: &mut egui::Ui, filters: &FilterStack) -> Option<FilterEdit> {
        let mut text = filters.live_filter_text().to_owned();
        let regex = filters.live_filter_is_regex();
        let match_count = format!(
            "{} of {}",
            gt_fmt::format_count(filters.visible_entries().len()),
            gt_fmt::format_count(filters.entry_count())
        );
        let addable = filters.can_add_live_filter_as_chip();
        let pending_note = self.pending_note(ui, filters);

        let mut edit = None;
        // Wraps onto further rows on a narrow window.
        ui.horizontal_wrapped(|ui| {
            if ui
                .add(
                    TextEdit::singleline(&mut text)
                        .id(egui::Id::new(LIVE_FILTER_FIELD_ID))
                        .hint_text(FIELD_HINT)
                        .desired_width(FIELD_WIDTH_PX),
                )
                .on_hover_text(FIELD_HOVER)
                .changed()
            {
                edit = Some(FilterEdit::WriteLiveFilter(text));
            }
            if ui
                .selectable_label(regex, REGEX_TOGGLE_LABEL)
                .on_hover_text(REGEX_TOGGLE_HOVER)
                .clicked()
            {
                edit = Some(FilterEdit::ReadLiveFilterAsRegex(!regex));
            }
            ui.label(RichText::new(match_count).weak())
                .on_hover_text(MATCH_COUNT_HOVER);
            if ui
                .add_enabled(addable, Button::new(ADD_FILTER_LABEL))
                .on_hover_text(ADD_FILTER_HOVER)
                .on_disabled_hover_text(match filters.live_filter_error() {
                    Some(_) => ADD_FILTER_INVALID_HOVER,
                    None => ADD_FILTER_EMPTY_HOVER,
                })
                .clicked()
            {
                edit = Some(FilterEdit::AddLiveFilterAsChip);
            }
            let written = !filters.live_filter_text().is_empty();
            if ui
                .add_enabled(written, Button::new(CLEAR_LABEL))
                .on_hover_text(CLEAR_HOVER)
                .on_disabled_hover_text(CLEAR_EMPTY_HOVER)
                .clicked()
            {
                edit = Some(FilterEdit::ClearLiveFilter);
            }
            if let Some(note) = pending_note {
                ui.label(RichText::new(note).weak());
            }
        });

        if let Some(error) = filters.live_filter_error() {
            ui.label(
                RichText::new(error.message())
                    .small()
                    .color(gt_ui_theme::error_indicator(ui.visuals().dark_mode)),
            );
        }
        edit
    }

    /// What to say about a scan still running, once it has run long enough for
    /// the note to mean something.
    fn pending_note(&mut self, ui: &egui::Ui, filters: &FilterStack) -> Option<&'static str> {
        if !filters.is_query_pending() {
            self.query_pending_since = None;
            return None;
        }
        // Repaint without waiting for input: the scan finishes on a worker
        // thread.
        ui.ctx().request_repaint();
        let now = ui.input(|input| input.time);
        let since = *self.query_pending_since.get_or_insert(now);
        (now - since >= PENDING_NOTE_DELAY_SECS).then_some(PENDING_NOTE)
    }
}

/// One chip per added filter, wrapping onto further rows when the window is too
/// narrow for them.
fn chip_row_ui(
    ui: &mut egui::Ui,
    filters: &FilterStack,
    slots: &LayerColorSlots,
) -> Option<FilterEdit> {
    if filters.chips().is_empty() {
        return None;
    }
    let mut edit = None;
    ui.horizontal_wrapped(|ui| {
        for chip in filters.chips() {
            if let Some(chip_edit) = chip_ui(ui, chip, slots) {
                edit = Some(chip_edit);
            }
        }
    });
    edit
}

/// One added filter: whether it is applied at all, the colour it draws in, what
/// it matches, the mode it does that in, and its removal.
fn chip_ui(ui: &mut egui::Ui, chip: &FilterChip, slots: &LayerColorSlots) -> Option<FilterEdit> {
    let mode = chip.mode();
    let dark_mode = ui.visuals().dark_mode;
    let color = chip.layer_slot().map_or_else(
        || ui.visuals().weak_text_color(),
        |slot| gt_ui_theme::log_layer_slot_color(slot.index()).resolve(dark_mode),
    );
    let mut edit = None;

    let chip_frame = Frame::new()
        .fill(ui.visuals().widgets.inactive.bg_fill)
        .corner_radius(CHIP_CORNER_RADIUS)
        .inner_margin(CHIP_INNER_MARGIN);
    let drawn = chip_frame.show(ui, |ui| {
        let mut enabled = chip.is_enabled();
        let applied_hover = match mode {
            FilterChipMode::Layer => LAYER_CHIP_HOVER,
            FilterChipMode::Refine => REFINE_CHIP_HOVER,
        };
        if ui
            .add(Checkbox::without_text(&mut enabled))
            .on_hover_text(applied_hover)
            .changed()
        {
            edit = Some(FilterEdit::SetChipEnabled {
                chip: chip.id(),
                enabled,
            });
        }
        if let Some(slot) = chip.layer_slot() {
            swatch_ui(ui, color, slots.is_shared(slot));
        }

        let text = &chip.pattern().text;
        let mode_name = match mode {
            FilterChipMode::Layer => "Layer",
            FilterChipMode::Refine => "Refine",
        };
        ui.add(Label::new(
            RichText::new(gt_fmt::truncate_with_ellipsis(text, CHIP_TEXT_CHARS)).monospace(),
        ))
        .on_hover_text(format!("{text}\n{mode_name} filter"));

        let (glyph, switch_to, switch_hover) = match mode {
            FilterChipMode::Layer => (
                ICON_PLUS_CIRCLE,
                FilterChipMode::Refine,
                SWITCH_TO_REFINE_HOVER,
            ),
            FilterChipMode::Refine => (ICON_FUNNEL, FilterChipMode::Layer, SWITCH_TO_LAYER_HOVER),
        };
        if ui.small_button(glyph).on_hover_text(switch_hover).clicked() {
            edit = Some(FilterEdit::SwitchChipMode {
                chip: chip.id(),
                to: switch_to,
            });
        }
        if ui
            .small_button(ICON_X)
            .on_hover_text(REMOVE_CHIP_HOVER)
            .clicked()
        {
            edit = Some(FilterEdit::RemoveChip(chip.id()));
        }
    });

    let stroke = egui::Stroke::new(CHIP_BORDER_WIDTH_PX, color);
    paint_chip_border(ui.painter(), drawn.response.rect, stroke, mode);
    edit
}

/// The palette colour a layer chip's matches draw in. A colour handed out twice
/// is drawn ringed, the chip's counterpart of the doubled outline the map draws
/// around a shared colour's glyphs.
fn swatch_ui(ui: &mut egui::Ui, color: egui::Color32, shared: bool) {
    let side = match shared {
        true => CHIP_SWATCH_SIZE_PX + 2.0 * CHIP_SHARED_SWATCH_RING_PX,
        false => CHIP_SWATCH_SIZE_PX,
    };
    let (rect, response) = ui.allocate_exact_size(egui::vec2(side, side), egui::Sense::hover());
    if shared {
        ui.painter().rect_stroke(
            rect,
            CHIP_SWATCH_CORNER_RADIUS,
            egui::Stroke::new(CHIP_BORDER_WIDTH_PX, color),
            StrokeKind::Inside,
        );
        response.on_hover_text(SHARED_SWATCH_HOVER);
    }
    ui.painter().rect_filled(
        egui::Rect::from_center_size(
            rect.center(),
            egui::vec2(CHIP_SWATCH_SIZE_PX, CHIP_SWATCH_SIZE_PX),
        ),
        CHIP_SWATCH_CORNER_RADIUS,
        color,
    );
}

/// A layer chip is bounded solidly, like the overlay it draws. A refine chip is
/// bounded in dashes, like the cut it makes into the table.
fn paint_chip_border(
    painter: &egui::Painter,
    rect: egui::Rect,
    stroke: egui::Stroke,
    mode: FilterChipMode,
) {
    match mode {
        FilterChipMode::Layer => {
            painter.rect_stroke(rect, CHIP_CORNER_RADIUS, stroke, StrokeKind::Inside);
        }
        FilterChipMode::Refine => painter.extend(egui::Shape::dashed_line(
            &[
                rect.left_top(),
                rect.right_top(),
                rect.right_bottom(),
                rect.left_bottom(),
                rect.left_top(),
            ],
            stroke,
            CHIP_DASH_LENGTH_PX,
            CHIP_DASH_GAP_PX,
        )),
    }
}
