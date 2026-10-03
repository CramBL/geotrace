//! The live filter over the shown log and the chips added from it: the field
//! with its `.*` toggle and match count, and one chip per added filter.

use std::num::NonZeroUsize;

use egui::{Button, Checkbox, Frame, Label, RichText, StrokeKind, TextEdit};
use egui_phosphor::regular::FUNNEL as ICON_FUNNEL;
use egui_phosphor::regular::PLUS_CIRCLE as ICON_PLUS_CIRCLE;
use egui_phosphor::regular::X as ICON_X;
use gt_log_view::{
    FilterChip, FilterChipId, FilterChipMode, FilterGroup, FilterGroupId, FilterGroupOperator,
    FilterPattern, FilterScope, FilterStack, LayerColorSlots, LoadedLogs,
};
use gt_logfile::{LogLevelKind, ParsedLog};
use gt_ui_types::LoadedLogId;
use strum::IntoEnumIterator as _;

use super::LogViewerWindow;

/// The edit the filter row or the chip row produced while rendering. It reaches
/// the engine once rendering has finished: every chip of a frame is drawn from
/// one state.
enum FilterEdit {
    AddLiveFilterAsChip,
    ClearLiveFilter,
    CreateGroup,
    MoveChipToGroup {
        chip: FilterChipId,
        group: FilterGroupId,
    },
    ReadLiveFilterAsRegex(bool),
    RemoveChip(FilterChipId),
    RemoveGroup(FilterGroupId),
    SelectGroup(FilterGroupId),
    SetChipEnabled {
        chip: FilterChipId,
        enabled: bool,
    },
    SetGroupOperator {
        group: FilterGroupId,
        operator: FilterGroupOperator,
    },
    SwitchChipMode {
        chip: FilterChipId,
        to: FilterChipMode,
    },
    WriteLiveFilter(String),
    SetScope(FilterScope),
    SetLevel(LogLevelKind),
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
        let live_edit = self.filter_row_ui(ui, filters, log.parsed());
        let chip_edit = chip_row_ui(ui, filters, logs.layer_color_slots());
        let edit = live_edit.or(chip_edit);

        let Some(edit) = edit else {
            return;
        };
        let Some((stack, slots)) = logs.filter_stack_mut_by_id(shown) else {
            return;
        };
        match edit {
            FilterEdit::SetScope(scope) => stack.set_live_filter_scope(scope),
            FilterEdit::SetLevel(level) => stack.set_live_filter_level(level),
            FilterEdit::WriteLiveFilter(text) => stack.set_live_filter_text(&text),
            FilterEdit::ReadLiveFilterAsRegex(regex) => stack.set_live_filter_regex(regex),
            FilterEdit::CreateGroup => {
                stack.create_group();
            }
            FilterEdit::MoveChipToGroup { chip, group } => stack.move_chip_to_group(chip, group),
            FilterEdit::RemoveGroup(group) => stack.remove_group(group),
            FilterEdit::SelectGroup(group) => stack.select_group(group),
            FilterEdit::SetGroupOperator { group, operator } => {
                stack.set_group_operator(group, operator)
            }
            FilterEdit::ClearLiveFilter => stack.clear_live_filter(),
            FilterEdit::AddLiveFilterAsChip => {
                stack.add_live_filter_as_chip();
            }
            FilterEdit::SetChipEnabled { chip, enabled } => stack.set_chip_enabled(chip, enabled),
            FilterEdit::SwitchChipMode { chip, to } => match to {
                FilterChipMode::Layer => stack.switch_chip_to_layer_mode(chip, slots),
                FilterChipMode::Refine => stack.switch_chip_to_refine_mode(chip, slots),
            },
            FilterEdit::RemoveChip(chip) => stack.remove_chip(chip, slots),
        }
    }

    pub(super) fn display_options_ui(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(DISPLAY_OPTIONS_LABEL, |ui| {
            ui.checkbox(&mut self.color_services, COLOR_SERVICES_LABEL)
                .on_hover_text(COLOR_SERVICES_HOVER);
            ui.checkbox(&mut self.color_levels, COLOR_LEVELS_LABEL)
                .on_hover_text(COLOR_LEVELS_HOVER);
        })
        .response
        .on_hover_text("Table display options");
    }

    /// The field the user filters the log from, what its pattern selected, and
    /// the controls turning it into a chip or emptying it.
    fn filter_row_ui(
        &mut self,
        ui: &mut egui::Ui,
        filters: &FilterStack,
        log: &ParsedLog,
    ) -> Option<FilterEdit> {
        let mut text = filters.live_filter_text().to_owned();
        let regex = filters.live_filter_is_regex();
        let scope = filters.live_filter_pattern().scope();
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
                .small_button(NEW_GROUP_LABEL)
                .on_hover_text("Create a group and select it for the live filter")
                .clicked()
            {
                edit = Some(FilterEdit::CreateGroup);
            }
            ui.menu_button(FilterScopeUi(scope).glyph(), |ui| {
                for candidate in FilterScope::iter() {
                    if ui
                        .selectable_label(scope == candidate, candidate.to_string())
                        .clicked()
                    {
                        edit = Some(FilterEdit::SetScope(candidate));
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text(format!("Filter {}", scope.to_string().to_lowercase()));
            if let FilterPattern::Level(level) = filters.live_filter_pattern() {
                ui.menu_button(level.to_string(), |ui| {
                    for candidate in LogLevelKind::iter() {
                        if ui
                            .selectable_label(*level == candidate, candidate.to_string())
                            .clicked()
                        {
                            edit = Some(FilterEdit::SetLevel(candidate));
                            ui.close();
                        }
                    }
                })
                .response
                .on_hover_text("Filter recognized level");
            } else {
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
                    edit = Some(FilterEdit::WriteLiveFilter(text.clone()));
                }
                if scope == FilterScope::Service {
                    ui.add_enabled_ui(log.services_by_first_appearance().next().is_some(), |ui| {
                        ui.menu_button(SERVICE_SUGGESTIONS_GLYPH, |ui| {
                            for service in log.services_by_first_appearance() {
                                if ui
                                    .selectable_label(text.eq_ignore_ascii_case(service), service)
                                    .clicked()
                                {
                                    edit = Some(FilterEdit::WriteLiveFilter(service.to_owned()));
                                    ui.close();
                                }
                            }
                        })
                        .response
                        .on_hover_text("Select a recognized service")
                        .on_disabled_hover_text("This log has no recognized services");
                    });
                }
            }
            if scope == FilterScope::Message
                && ui
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

fn group_operator_ui(ui: &mut egui::Ui, group: &FilterGroup) -> Option<FilterEdit> {
    let (glyph, hover, next) = match group.operator() {
        FilterGroupOperator::All => (
            INTERSECTION_SYMBOL,
            ALL_FILTERS_HOVER,
            FilterGroupOperator::Any,
        ),
        FilterGroupOperator::Any => (UNION_SYMBOL, ANY_FILTER_HOVER, FilterGroupOperator::All),
    };
    ui.small_button(glyph)
        .on_hover_text(hover)
        .clicked()
        .then_some(FilterEdit::SetGroupOperator {
            group: group.id(),
            operator: next,
        })
}

/// One chip per added filter, wrapping onto further rows when the window is too
/// narrow for them.
fn chip_row_ui(
    ui: &mut egui::Ui,
    filters: &FilterStack,
    slots: &LayerColorSlots,
) -> Option<FilterEdit> {
    let mut edit = None;
    for (index, group) in filters.groups().iter().enumerate() {
        ui.push_id(group.id(), |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(RichText::new("Table filters").weak());
                if ui
                    .selectable_label(
                        filters.selected_group() == group.id(),
                        (index + 1).to_string(),
                    )
                    .on_hover_text(format!("Use group {} for the live filter", index + 1))
                    .clicked()
                {
                    edit = Some(FilterEdit::SelectGroup(group.id()));
                }
                if let Some(operator_edit) = group_operator_ui(ui, group) {
                    edit = Some(operator_edit);
                }
                if ui
                    .add_enabled(
                        filters.groups().len() > 1,
                        Button::new(REMOVE_GROUP_LABEL).small(),
                    )
                    .on_hover_text(
                        "Remove this group and assign its filters to the first remaining group",
                    )
                    .on_disabled_hover_text("Keep at least one group for table filters")
                    .clicked()
                {
                    edit = Some(FilterEdit::RemoveGroup(group.id()));
                }
                for chip in filters.chips().iter().filter(|chip| {
                    chip.mode() == FilterChipMode::Refine && chip.group() == group.id()
                }) {
                    if let Some(chip_edit) = chip_ui(ui, chip, slots, filters.groups()) {
                        edit = Some(chip_edit);
                    }
                }
            });
        });
    }
    if filters
        .chips()
        .iter()
        .any(|chip| chip.mode() == FilterChipMode::Layer)
    {
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Map highlights").weak());
            for chip in filters
                .chips()
                .iter()
                .filter(|chip| chip.mode() == FilterChipMode::Layer)
            {
                if let Some(chip_edit) = chip_ui(ui, chip, slots, filters.groups()) {
                    edit = Some(chip_edit);
                }
            }
        });
    }
    edit
}

/// One added filter: whether it is applied at all, the colour it draws in, what
/// it matches, the mode it does that in, and its removal.
fn chip_ui(
    ui: &mut egui::Ui,
    chip: &FilterChip,
    slots: &LayerColorSlots,
    groups: &[FilterGroup],
) -> Option<FilterEdit> {
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
        let effect = match mode {
            FilterChipMode::Layer => LAYER_CHIP_HOVER,
            FilterChipMode::Refine => REFINE_CHIP_HOVER,
        };
        if ui
            .add(Checkbox::without_text(&mut enabled))
            .on_hover_text(effect)
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

        ui.label(FilterScopeUi(chip.pattern().scope()).glyph())
            .on_hover_text(chip.pattern().scope().to_string());
        let text = chip.pattern().text();
        ui.add(Label::new(
            RichText::new(gt_fmt::truncate_with_ellipsis(text, CHIP_TEXT_CHARS)).monospace(),
        ))
        .on_hover_text(format!("{text}\n{effect}"));

        let (glyph, switch_to, switch_hover) = match mode {
            FilterChipMode::Layer => (ICON_PLUS_CIRCLE, FilterChipMode::Refine, REFINE_CHIP_HOVER),
            FilterChipMode::Refine => (ICON_FUNNEL, FilterChipMode::Layer, LAYER_CHIP_HOVER),
        };
        if ui.small_button(glyph).on_hover_text(switch_hover).clicked() {
            edit = Some(FilterEdit::SwitchChipMode {
                chip: chip.id(),
                to: switch_to,
            });
        }
        ui.add_enabled_ui(groups.len() > 1, |ui| {
            ui.menu_button(MOVE_FILTER_LABEL, |ui| {
                for (index, group) in groups.iter().enumerate() {
                    if ui
                        .selectable_label(
                            chip.group() == group.id(),
                            format!("Group {}", index + 1),
                        )
                        .clicked()
                    {
                        edit = Some(FilterEdit::MoveChipToGroup {
                            chip: chip.id(),
                            group: group.id(),
                        });
                        ui.close();
                    }
                }
            })
            .response
            .on_hover_text("Select this filter's table group")
            .on_disabled_hover_text("Create another group to assign this filter to it");
        });
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

pub(in crate::app) const ADD_FILTER_LABEL: &str = "+";

const ADD_FILTER_HOVER: &str = "Add this as a table filter";

pub(super) const ADD_FILTER_EMPTY_HOVER: &str = "Write a live filter to add it as a chip";

pub(super) const ADD_FILTER_INVALID_HOVER: &str =
    "The live filter is added once its regular expression compiles";

const CLEAR_LABEL: &str = "Clear";

const CLEAR_HOVER: &str = "Empty the live filter";

const CLEAR_EMPTY_HOVER: &str = "The live filter is empty already";

const MATCH_COUNT_HOVER: &str = "Lines the filters show, of the log's entries";

pub(super) const DISPLAY_OPTIONS_LABEL: &str = "Aa";

pub(super) const COLOR_SERVICES_LABEL: &str = "Colour services";

const COLOR_SERVICES_HOVER: &str = "Draw each service name in a colour of its own";

pub(super) const COLOR_LEVELS_LABEL: &str = "Colour levels";

const COLOR_LEVELS_HOVER: &str =
    "Draw each error, warning and debug level in the colour of its severity";

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

const LAYER_CHIP_HOVER: &str = "Highlight on map";

const REFINE_CHIP_HOVER: &str = "Filter table";

const REMOVE_CHIP_HOVER: &str = "Remove this filter";

pub(in crate::app) const INTERSECTION_SYMBOL: &str = "∩";
pub(in crate::app) const UNION_SYMBOL: &str = "∪";
const ALL_FILTERS_HOVER: &str = "Match all filters in this group. Click to match any";
const ANY_FILTER_HOVER: &str = "Match any filter in this group. Click to match all";

pub(in crate::app) const NEW_GROUP_LABEL: &str = "⊕";
pub(in crate::app) const REMOVE_GROUP_LABEL: &str = "−";
pub(in crate::app) const MOVE_FILTER_LABEL: &str = "→";

struct FilterScopeUi(FilterScope);

impl FilterScopeUi {
    fn glyph(self) -> &'static str {
        match self.0 {
            FilterScope::Hostname => egui_phosphor::regular::DESKTOP,
            FilterScope::Level => egui_phosphor::regular::WARNING,
            FilterScope::Message => egui_phosphor::regular::TEXT_ALIGN_LEFT,
            FilterScope::Service => egui_phosphor::regular::GEAR,
        }
    }
}

const SERVICE_SUGGESTIONS_GLYPH: &str = egui_phosphor::regular::LIST;
