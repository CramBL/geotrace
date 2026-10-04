//! The live filter over the shown log and the chips added from it: the field
//! with its `.*` toggle and match count, and one chip per added filter.

use std::mem;
use std::num::NonZeroUsize;

use egui::widget_style::{Classes, WidgetState};
use egui::{
    Button, Checkbox, Frame, Label, RichText, StrokeKind, TextEdit, TextStyle, TextWrapMode,
    WidgetText,
};
use egui_phosphor::regular::{EYE as ICON_EYE, EYE_SLASH as ICON_EYE_SLASH};
use gt_log_view::{
    FilterChip, FilterChipId, FilterEffect, FilterGroup, FilterGroupId, FilterGroupOperator,
    FilterScope, FilterStack, LayerColorSlots, LiveFilterDraft, LoadedLogs,
};
use gt_logfile::{LogLevelKind, ParsedLog};
use gt_ui_types::LoadedLogId;
use strum::IntoEnumIterator as _;

use super::LogViewerWindow;

/// The edit the filter row or the chip row produced while rendering. It reaches
/// the engine once rendering has finished: every chip of a frame is drawn from
/// one state.
enum FilterEdit {
    AddChipEffect {
        chip: FilterChipId,
        effect: FilterEffect,
    },
    AddLiveFilterAsChip,
    CancelLiveFilter,
    ClearLiveFilter,
    CreateGroup,
    MoveChipToGroup {
        chip: FilterChipId,
        group: FilterGroupId,
    },
    ReadLiveFilterAsRegex(bool),
    RemoveChipEffect {
        chip: FilterChipId,
        effect: FilterEffect,
    },
    RemoveGroup(FilterGroupId),
    SelectGroup(FilterGroupId),
    SelectMapHighlights,
    SetChipEffectEnabled {
        chip: FilterChipId,
        effect: FilterEffect,
        enabled: bool,
    },
    SetGroupOperator {
        group: FilterGroupId,
        operator: FilterGroupOperator,
    },
    SetLevel(LogLevelKind),
    SetScope(FilterScope),
    WriteLiveFilter(String),
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
        if !filters.live_filter_text().is_empty() {
            self.filter_editor_log = Some(shown);
        }
        let filter_width = ui.available_width().min(ui.clip_rect().width());
        let edit = ui
            .scope(|ui| {
                ui.set_max_width(filter_width);
                self.filter_groups_ui(ui, filters, log.parsed(), logs.layer_color_slots(), shown)
            })
            .inner;

        let Some(edit) = edit else {
            return;
        };
        if matches!(
            edit,
            FilterEdit::SelectGroup(_) | FilterEdit::SelectMapHighlights
        ) {
            self.filter_editor_log = Some(shown);
            self.filter_editor_focus = true;
        }
        if let FilterEdit::SelectGroup(group) = edit
            && filters.selected_group() == group
            && filters.live_filter_effect() == FilterEffect::Table
        {
            return;
        }
        if matches!(
            edit,
            FilterEdit::AddLiveFilterAsChip | FilterEdit::CancelLiveFilter
        ) {
            ui.memory_mut(|memory| {
                if let Some(focused) = memory.focused() {
                    memory.surrender_focus(focused);
                }
            });
        }
        if matches!(edit, FilterEdit::CancelLiveFilter) {
            self.filter_editor_log = None;
            if filters.live_filter_text().is_empty() {
                return;
            }
        }
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
                stack.set_live_filter_effect(FilterEffect::Table);
                self.filter_editor_log = Some(shown);
                self.filter_editor_focus = true;
            }
            FilterEdit::MoveChipToGroup { chip, group } => stack.move_chip_to_group(chip, group),
            FilterEdit::RemoveGroup(group) => stack.remove_group(group),
            FilterEdit::SelectMapHighlights => stack.set_live_filter_effect(FilterEffect::Map),
            FilterEdit::SelectGroup(group) => {
                stack.set_live_filter_effect(FilterEffect::Table);
                stack.select_group(group);
            }
            FilterEdit::SetGroupOperator { group, operator } => {
                stack.set_group_operator(group, operator)
            }
            FilterEdit::ClearLiveFilter | FilterEdit::CancelLiveFilter => stack.clear_live_filter(),
            FilterEdit::AddLiveFilterAsChip => {
                match stack.live_filter_effect() {
                    FilterEffect::Table => {
                        stack.add_live_filter_as_chip();
                    }
                    FilterEffect::Map => {
                        stack.add_live_filter_as_map_highlight(slots);
                    }
                }
                self.filter_editor_log = None;
            }
            FilterEdit::SetChipEffectEnabled {
                chip,
                effect,
                enabled,
            } => stack.set_chip_effect_enabled(chip, effect, enabled),
            FilterEdit::AddChipEffect { chip, effect } => {
                stack.add_chip_effect(chip, effect, slots)
            }
            FilterEdit::RemoveChipEffect { chip, effect } => {
                stack.remove_chip_effect(chip, effect, slots)
            }
        }
    }

    pub(super) fn display_options_ui(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(DISPLAY_OPTIONS_LABEL, |ui| {
            ui.checkbox(&mut self.show_structural_lines, "Show structural lines")
                .on_hover_text("Show recognized source lines between log entries");
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
        let draft = filters.live_filter_draft();
        let scope = draft.scope();
        let addable = filters.can_add_live_filter_as_chip();
        let focus_editor = mem::take(&mut self.filter_editor_focus);

        let mut edit = None;
        // Wraps onto further rows on a narrow window.
        ui.horizontal_wrapped(|ui| {
            let scope_response = ui
                .menu_button(FilterScopeUi(scope).glyph(), |ui| {
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
            scope_response
                .ctx
                .accesskit_node_builder(scope_response.id, |node| {
                    node.set_author_id(LIVE_FILTER_SCOPE_ID)
                });
            if let LiveFilterDraft::Level(level) = draft {
                let field = ui
                    .menu_button(
                        level.map_or_else(|| "Choose level…".to_owned(), |level| level.to_string()),
                        |ui| {
                            for candidate in LogLevelKind::iter() {
                                if ui
                                    .selectable_label(
                                        *level == Some(candidate),
                                        candidate.to_string(),
                                    )
                                    .clicked()
                                {
                                    edit = Some(FilterEdit::SetLevel(candidate));
                                    ui.close();
                                }
                            }
                        },
                    )
                    .response
                    .on_hover_text(FilterScopeUi(scope).help(regex));
                identify_editor(&field, filters);
                if focus_editor {
                    field.request_focus();
                }
            } else {
                let field = ui
                    .add(
                        TextEdit::singleline(&mut text)
                            .id(egui::Id::new(LIVE_FILTER_FIELD_ID))
                            .hint_text(FilterScopeUi(scope).hint())
                            .desired_width(FIELD_WIDTH_PX),
                    )
                    .on_hover_text(FilterScopeUi(scope).help(regex));
                identify_editor(&field, filters);
                if focus_editor {
                    field.request_focus();
                }
                if field.changed() {
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
            if ui
                .add_enabled(addable, Button::new(ADD_FILTER_LABEL))
                .on_hover_text(match filters.live_filter_effect() {
                    FilterEffect::Table => ADD_FILTER_HOVER,
                    FilterEffect::Map => "Add this as a map highlight",
                })
                .on_disabled_hover_text(match filters.live_filter_error() {
                    Some(_) => ADD_FILTER_INVALID_HOVER,
                    None if scope == FilterScope::Level => "Choose a level to add a condition",
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
            if ui
                .small_button("Cancel")
                .on_hover_text("Clear this condition and close the editor")
                .clicked()
            {
                edit = Some(FilterEdit::CancelLiveFilter);
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

impl LogViewerWindow {
    fn filter_groups_ui(
        &mut self,
        ui: &mut egui::Ui,
        filters: &FilterStack,
        log: &ParsedLog,
        slots: &LayerColorSlots,
        shown: LoadedLogId,
    ) -> Option<FilterEdit> {
        let mut edit = None;
        let editor_open =
            self.filter_editor_log == Some(shown) || !filters.live_filter_text().is_empty();
        let pending_note = self.pending_note(ui, filters);
        for (index, group) in filters.groups().iter().enumerate() {
            ui.push_id(group.id(), |ui| {
                ui.ctx().accesskit_node_builder(ui.unique_id(), |node| {
                    node.set_author_id(GroupControl::Row.identity(group.id()));
                });
                ui.horizontal_wrapped(|ui| {
                    if index == 0 {
                        ui.label(RichText::new("Table filters").strong());
                    } else {
                        ui.label("AND");
                    }
                    if let Some(operator_edit) = group_operator_ui(ui, group) {
                        edit = Some(operator_edit);
                    }
                    for chip in filters.chips().iter().filter(|chip| {
                        chip.has_effect(FilterEffect::Table) && chip.group() == group.id()
                    }) {
                        if let Some(chip_edit) = chip_ui(ui, chip, FilterEffect::Table, slots, filters.groups()) {
                            edit = Some(chip_edit);
                        }
                    }
                    let add = ui.small_button(ADD_CONDITION_LABEL)
                        .on_hover_text("Add condition to this group");
                    GroupControl::Add.identify(&add, group.id());
                    if add.clicked() {
                        edit = Some(FilterEdit::SelectGroup(group.id()));
                    }
                    if filters.groups().len() > 1 {
                        let overflow = ui.menu_button(OVERFLOW_LABEL, |ui| {
                            let remove = ui.button(REMOVE_GROUP_LABEL)
                                .on_hover_text("Remove this group and assign its filters to the first remaining group");
                            GroupControl::Remove.identify(&remove, group.id());
                            if remove.clicked() {
                                edit = Some(FilterEdit::RemoveGroup(group.id()));
                                ui.close();
                            }
                        });
                        GroupControl::Overflow.identify(&overflow.response, group.id());
                    }
                });
                if editor_open && filters.live_filter_effect() == FilterEffect::Table && filters.selected_group() == group.id()
                    && let Some(live_edit) = self.filter_row_ui(ui, filters, log)
                {
                    edit = Some(live_edit);
                }
            });
        }
        ui.horizontal_wrapped(|ui| {
            if ui
                .small_button(NEW_GROUP_LABEL)
                .on_hover_text("Create another table filter group")
                .clicked()
            {
                edit = Some(FilterEdit::CreateGroup);
            }
            let count = ui
                .label(
                    RichText::new(format!(
                        "{} of {}",
                        gt_fmt::format_count(filters.visible_entries().len()),
                        gt_fmt::format_count(filters.entry_count())
                    ))
                    .weak(),
                )
                .on_hover_text(MATCH_COUNT_HOVER);
            count
                .ctx
                .accesskit_node_builder(count.id, |node| node.set_author_id(MATCH_COUNT_ID));
            if let Some(note) = pending_note {
                ui.label(RichText::new(note).weak());
            }
        });
        ui.horizontal_wrapped(|ui| {
            ui.label(RichText::new("Map highlights").strong());
            for chip in filters
                .chips()
                .iter()
                .filter(|chip| chip.has_effect(FilterEffect::Map))
            {
                if let Some(chip_edit) =
                    chip_ui(ui, chip, FilterEffect::Map, slots, filters.groups())
                {
                    edit = Some(chip_edit);
                }
            }
            let add = ui
                .small_button(ADD_CONDITION_LABEL)
                .on_hover_text("Add a map highlight");
            add.ctx.accesskit_node_builder(add.id, |node| {
                node.set_author_id(MAP_HIGHLIGHT_ADD_ID);
                node.set_description("Add a map highlight");
            });
            if add.clicked() {
                edit = Some(FilterEdit::SelectMapHighlights);
            }
        });
        if editor_open
            && filters.live_filter_effect() == FilterEffect::Map
            && let Some(live_edit) = self.filter_row_ui(ui, filters, log)
        {
            edit = Some(live_edit);
        }
        edit
    }
}

fn identify_editor(response: &egui::Response, filters: &FilterStack) {
    match filters.live_filter_effect() {
        FilterEffect::Table => GroupControl::Editor.identify(response, filters.selected_group()),
        FilterEffect::Map => {
            response.ctx.accesskit_node_builder(response.id, |node| {
                node.set_author_id(MAP_HIGHLIGHT_EDITOR_ID);
            });
        }
    }
}

fn group_operator_ui(ui: &mut egui::Ui, group: &FilterGroup) -> Option<FilterEdit> {
    let operator = match group.operator() {
        FilterGroupOperator::All => FilterGroupOperator::Any,
        FilterGroupOperator::Any => FilterGroupOperator::All,
    };
    let hover = match group.operator() {
        FilterGroupOperator::All => "Match all conditions. Click to match any condition.",
        FilterGroupOperator::Any => "Match any condition. Click to match all conditions.",
    };
    let response = ui
        .small_button(group.operator().to_string())
        .on_hover_text(hover);
    response
        .ctx
        .accesskit_node_builder(response.id, |node| node.set_description(hover));
    GroupControl::Operator.identify(&response, group.id());
    response.clicked().then_some(FilterEdit::SetGroupOperator {
        group: group.id(),
        operator,
    })
}

fn chip_ui(
    ui: &mut egui::Ui,
    chip: &FilterChip,
    effect: FilterEffect,
    slots: &LayerColorSlots,
    groups: &[FilterGroup],
) -> Option<FilterEdit> {
    let dark_mode = ui.visuals().dark_mode;
    let map_slot = (effect == FilterEffect::Map)
        .then(|| chip.layer_slot())
        .flatten();
    let enabled = chip.is_enabled(effect);
    let color = if !enabled {
        ui.visuals().widgets.noninteractive.bg_stroke.color
    } else {
        map_slot.map_or_else(
            || ui.visuals().weak_text_color(),
            |slot| gt_ui_theme::log_layer_slot_color(slot.index()).resolve(dark_mode),
        )
    };
    let mut edit = None;

    let chip_text = gt_fmt::truncate_with_ellipsis(chip.pattern().text(), CHIP_TEXT_CHARS);
    let value = WidgetText::from(
        RichText::new(chip_text.as_ref())
            .monospace()
            .color(if enabled {
                ui.visuals().text_color()
            } else {
                ui.visuals().weak_text_color()
            }),
    )
    .into_galley(
        ui,
        Some(TextWrapMode::Extend),
        f32::INFINITY,
        TextStyle::Body,
    );
    let scope = WidgetText::from(FilterScopeUi(chip.pattern().scope()).glyph()).into_galley(
        ui,
        Some(TextWrapMode::Extend),
        f32::INFINITY,
        TextStyle::Body,
    );
    let overflow = WidgetText::from(OVERFLOW_LABEL).into_galley(
        ui,
        Some(TextWrapMode::Extend),
        f32::INFINITY,
        TextStyle::Button,
    );
    let classes = Classes::default();
    let checkbox = ui.style().checkbox_style(&classes, WidgetState::default());
    let button = ui.style().button_style(&classes, WidgetState::default());
    let checkbox_width = (checkbox.checkbox_size + checkbox.frame.total_margin().sum().x)
        .max(ui.spacing().interact_size.y);
    let swatch_width = map_slot.map_or(0.0, |slot| {
        CHIP_SWATCH_SIZE_PX
            + ui.spacing().item_spacing.x
            + if slots.is_shared(slot) {
                2.0 * CHIP_SHARED_SWATCH_RING_PX
            } else {
                0.0
            }
    });
    let chip_width = value.size().x
        + scope.size().x
        + overflow.size().x
        + checkbox_width
        + button.frame.total_margin().sum().x
        + CHIP_INNER_MARGIN.sum().x
        + CHIP_CONTROL_GAPS * ui.spacing().item_spacing.x
        + swatch_width;
    if ui.available_size_before_wrap().x < chip_width {
        ui.end_row();
    }

    let chip_frame = Frame::new()
        .fill(if enabled {
            ui.visuals().widgets.inactive.bg_fill
        } else {
            ui.visuals().faint_bg_color
        })
        .corner_radius(CHIP_CORNER_RADIUS)
        .inner_margin(CHIP_INNER_MARGIN);
    let drawn = ui
        .push_id((chip.id(), effect), |ui| {
            chip_frame.show(ui, |ui| {
                if !enabled {
                    ui.visuals_mut().override_text_color = Some(ui.visuals().weak_text_color());
                }
                let mut enabled = enabled;
                let effect_hover = match effect {
                    FilterEffect::Map => LAYER_CHIP_HOVER,
                    FilterEffect::Table => REFINE_CHIP_HOVER,
                };
                let enabled_response = match effect {
                    FilterEffect::Table => ui
                        .add(Checkbox::without_text(&mut enabled))
                        .on_hover_text(effect_hover),
                    FilterEffect::Map => {
                        let response = ui
                            .small_button(if enabled { ICON_EYE } else { ICON_EYE_SLASH })
                            .on_hover_text(if enabled {
                                "Hide map highlight"
                            } else {
                                "Show map highlight"
                            });
                        response.widget_info(|| {
                            egui::WidgetInfo::selected(
                                egui::WidgetType::Checkbox,
                                true,
                                enabled,
                                "Map highlight visibility",
                            )
                        });
                        if response.clicked() {
                            enabled = !enabled;
                        }
                        response
                    }
                };
                ChipControl::Enable.identify(&enabled_response, chip.id(), effect);
                if enabled != chip.is_enabled(effect) {
                    edit = Some(FilterEdit::SetChipEffectEnabled {
                        chip: chip.id(),
                        effect,
                        enabled,
                    });
                }
                if let Some(slot) = map_slot {
                    swatch_ui(ui, color, slots.is_shared(slot));
                }

                ui.label(FilterScopeUi(chip.pattern().scope()).glyph())
                    .on_hover_text(chip.pattern().scope().to_string());
                let text = chip.pattern().text();
                let value_response = ui
                    .add(Label::new(value))
                    .on_hover_text(format!("{text}\n{effect_hover}"));
                ChipControl::Value.identify(&value_response, chip.id(), effect);

                if let Some(action) = chip_actions_ui(ui, chip, effect, groups) {
                    edit = Some(action);
                }
            })
        })
        .inner;

    let stroke = egui::Stroke::new(CHIP_BORDER_WIDTH_PX, color);
    paint_chip_border(ui.painter(), drawn.response.rect, stroke, effect);
    edit
}

fn chip_actions_ui(
    ui: &mut egui::Ui,
    chip: &FilterChip,
    effect: FilterEffect,
    groups: &[FilterGroup],
) -> Option<FilterEdit> {
    let mut edit = None;
    let overflow = ui.menu_button(OVERFLOW_LABEL, |ui| {
        let (other, add_label, remove_other_label) = match effect {
            FilterEffect::Map => (
                FilterEffect::Table,
                "Also filter table",
                "Stop filtering table",
            ),
            FilterEffect::Table => (
                FilterEffect::Map,
                "Also highlight on map",
                "Remove map highlight",
            ),
        };
        let present = chip.has_effect(other);
        let action = ui.button(if present {
            remove_other_label
        } else {
            add_label
        });
        ChipControl::OtherEffect.identify(&action, chip.id(), effect);
        if action.clicked() {
            edit = Some(if present {
                FilterEdit::RemoveChipEffect {
                    chip: chip.id(),
                    effect: other,
                }
            } else {
                FilterEdit::AddChipEffect {
                    chip: chip.id(),
                    effect: other,
                }
            });
            ui.close();
        }
        if effect == FilterEffect::Table && groups.len() > 1 {
            let movement = ui.menu_button(MOVE_FILTER_LABEL, |ui| {
                for (index, group) in groups.iter().enumerate() {
                    let choice = ui.selectable_label(
                        chip.group() == group.id(),
                        format!("Group {}", index + 1),
                    );
                    ChipControl::Destination(group.id()).identify(&choice, chip.id(), effect);
                    if choice.clicked() {
                        edit = Some(FilterEdit::MoveChipToGroup {
                            chip: chip.id(),
                            group: group.id(),
                        });
                        ui.close();
                    }
                }
            });
            ChipControl::MoveToGroup.identify(&movement.response, chip.id(), effect);
        }
        let label = match effect {
            FilterEffect::Map => "Remove map highlight",
            FilterEffect::Table => "Stop filtering table",
        };
        let remove = ui.button(label);
        ChipControl::Remove.identify(&remove, chip.id(), effect);
        if remove.clicked() {
            edit = Some(FilterEdit::RemoveChipEffect {
                chip: chip.id(),
                effect,
            });
            ui.close();
        }
    });
    ChipControl::Overflow.identify(&overflow.response, chip.id(), effect);
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
    effect: FilterEffect,
) {
    match effect {
        FilterEffect::Map => {
            painter.rect_stroke(rect, CHIP_CORNER_RADIUS, stroke, StrokeKind::Inside);
        }
        FilterEffect::Table => painter.extend(egui::Shape::dashed_line(
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

pub(in crate::app) const LIVE_FILTER_SCOPE_ID: &str = "log_viewer_live_filter_scope";

pub(in crate::app) const LIVE_FILTER_FIELD_ID: &str = "log_viewer_live_filter";

/// Width of the live-filter field, wide enough for the several terms a filter
/// usually holds.
const FIELD_WIDTH_PX: f32 = 260.0;

pub(super) const REGEX_TOGGLE_LABEL: &str = ".*";

const REGEX_TOGGLE_HOVER: &str = "Read the field as a regular expression instead of a set of terms";

pub(in crate::app) const ADD_FILTER_LABEL: &str = "Add";

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

const CHIP_CONTROL_GAPS: f32 = 3.0;

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

pub(in crate::app) const ADD_CONDITION_LABEL: &str = "+";
pub(in crate::app) const NEW_GROUP_LABEL: &str = "+ group";
pub(in crate::app) const REMOVE_GROUP_LABEL: &str = "Remove group";
pub(in crate::app) const MOVE_FILTER_LABEL: &str = "Move to group…";

#[derive(Clone, Copy, Debug)]
pub(in crate::app) enum ChipControl {
    Destination(FilterGroupId),
    Enable,
    MoveToGroup,
    OtherEffect,
    Overflow,
    Remove,
    Value,
}

impl ChipControl {
    pub(in crate::app) fn identity(self, chip: FilterChipId, effect: FilterEffect) -> String {
        match self {
            Self::Destination(group) => {
                format!("log-filter-{chip:?}-{effect:?}-Destination-{group:?}")
            }
            _ => format!("log-filter-{chip:?}-{effect:?}-{self:?}"),
        }
    }

    fn identify(self, response: &egui::Response, chip: FilterChipId, effect: FilterEffect) {
        response.ctx.accesskit_node_builder(response.id, |node| {
            node.set_author_id(self.identity(chip, effect));
        });
    }
}

const OVERFLOW_LABEL: &str = "⋯";
pub(in crate::app) const MAP_HIGHLIGHT_ADD_ID: &str = "log-map-highlight-add";
pub(in crate::app) const MAP_HIGHLIGHT_EDITOR_ID: &str = "log-map-highlight-editor";
pub(in crate::app) const MATCH_COUNT_ID: &str = "log-filter-match-count";

#[derive(Clone, Copy, Debug)]
pub(in crate::app) enum GroupControl {
    Add,
    Editor,
    Operator,
    Overflow,
    Remove,
    Row,
}

impl GroupControl {
    pub(in crate::app) fn identity(self, group: FilterGroupId) -> String {
        format!("log-filter-group-{group:?}-{self:?}")
    }

    fn identify(self, response: &egui::Response, group: FilterGroupId) {
        response.ctx.accesskit_node_builder(response.id, |node| {
            node.set_author_id(self.identity(group));
        });
    }
}

struct FilterScopeUi(FilterScope);

impl FilterScopeUi {
    fn hint(&self) -> &'static str {
        match self.0 {
            FilterScope::Hostname => "Filter hostnames",
            FilterScope::Level => "Choose level…",
            FilterScope::Message => "Filter messages",
            FilterScope::Service => "Filter services",
        }
    }

    fn help(&self, regex: bool) -> &'static str {
        match self.0 {
            FilterScope::Hostname => "Match the recognized hostname exactly, ignoring case",
            FilterScope::Level => "Match the recognized severity level",
            FilterScope::Message if regex => "Match messages with this regular expression",
            FilterScope::Message => "Match messages containing every term, ignoring case",
            FilterScope::Service => "Match the recognized service identity exactly, ignoring case",
        }
    }

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
