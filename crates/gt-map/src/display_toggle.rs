//! The display toggle - the on-map eye button and its popup, where whole
//! element categories are shown or hidden via the [`DisplayMask`].
//!
//! The popup lists every [`DisplayCategory`] with its in-scope count
//! ([`DisplayCounts`]).

use chrono::{DateTime, Utc};
use egui::Button;
use egui::{Align2, Area, Frame, Id, Mesh, RichText, Sense, Shape, Ui};
use egui_phosphor::regular::CARET_LEFT as ICON_CARET_LEFT;
use egui_phosphor::regular::CARET_RIGHT as ICON_CARET_RIGHT;
use egui_phosphor::regular::EYE as ICON_EYE;
use egui_phosphor::regular::EYE_SLASH as ICON_EYE_SLASH;
use gt_fmt::UTC_MINUTE_FORMAT;
use gt_ionex::instant_selection::{TecEmptyReason, TecInstantSelection};
use gt_jam::day_selection::{DaySelection, EmptyReason};
use gt_ui_theme::EM_DASH;
use gt_ui_types::{DisplayCategory, DisplayMask, SkyGlyphVariant};
use strum::IntoEnumIterator;

use crate::display_counts::DisplayCounts;

/// Id of the [`egui::Area`] the eye button is drawn in.
pub const DISPLAY_TOGGLE_BUTTON_AREA_ID: &str = "display_toggle_button";

/// Id of the [`egui::Area`] the category popup is drawn in while it is open.
pub const DISPLAY_TOGGLE_POPUP_AREA_ID: &str = "display_toggle_popup";

/// Width of the right-aligned count column, sized for the widest expected
/// count (`999,999`) so the eye glyphs stay aligned across rows.
const COUNT_COLUMN_WIDTH_PX: f32 = 48.0;

/// Indent of the interference row's day stepper, marking it as that row's
/// detail.
const STEPPER_INDENT_PX: f32 = 24.0;

/// Disabled hover text while the interference layer is hidden.
const HIDDEN_LAYER_TEXT: &str = "Show the interference layer to step days";

/// Width and height of the TEC legend's colour strip.
const LEGEND_WIDTH_PX: f32 = 168.0;
const LEGEND_STRIP_HEIGHT_PX: f32 = 10.0;

/// Gap between the legend's strip and its tick labels.
const LEGEND_LABEL_GAP_PX: f32 = 2.0;

/// Columns the legend's gradient is painted in. The ramp is piecewise linear,
/// so this many columns of vertex-interpolated colour render it smoothly.
const LEGEND_COLUMNS: usize = 48;

/// Alpha the legend and its labels draw at while the layer is hidden.
const DISABLED_ALPHA: f32 = 0.4;

/// The interference row's own state: which day it shows, and why it has
/// nothing to draw.
pub(crate) struct InterferenceRow<'a> {
    pub(crate) day: &'a mut DaySelection,
    pub(crate) empty_reason: Option<EmptyReason>,
}

/// The TEC row's own state: which instant it shows, how strongly it draws, and
/// why it has nothing to draw.
pub(crate) struct TecRow<'a> {
    pub(crate) instant: &'a mut TecInstantSelection,
    pub(crate) opacity_percent: &'a mut f32,
    pub(crate) empty_reason: Option<TecEmptyReason>,
}

/// The archive-backed layers' rows, which carry their own controls under the
/// category row.
pub(crate) struct LayerRows<'a> {
    pub(crate) interference: InterferenceRow<'a>,
    pub(crate) tec: TecRow<'a>,
}

/// Session-only UI state of the display toggle.
#[derive(Default)]
pub(crate) struct DisplayToggleState {
    open: bool,
    /// The mask as it was before the last `only` solo, restored by
    /// pressing `only` again on the soloed category.
    solo_restore: Option<DisplayMask>,
}

fn popup_label(category: DisplayCategory) -> &'static str {
    match category {
        DisplayCategory::Tracks => "Tracks",
        DisplayCategory::TrackPoints => "Track points",
        DisplayCategory::SatelliteLabels => "Satellite labels",
        DisplayCategory::CustomMarkers => "Custom markers",
        DisplayCategory::GeneratedMarkers => "Generated markers",
        DisplayCategory::EventMarkers => "Event markers",
        DisplayCategory::QueryHighlights => "Query highlights",
        DisplayCategory::SnappedTracks => "Snapped tracks",
        DisplayCategory::SkyGlyphs => "Sky glyphs",
        DisplayCategory::JammingHexes => gt_jam::text::LAYER_LABEL,
        DisplayCategory::TecHeatmap => gt_ionex::text::LAYER_LABEL,
        DisplayCategory::LogMatches => "Log matches",
    }
}

/// Why a category's row is disabled.
fn empty_hover_text(category: DisplayCategory) -> String {
    match category {
        DisplayCategory::JammingHexes => "No interference data archived for this day".to_owned(),
        DisplayCategory::TecHeatmap => "No TEC maps archived for this instant".to_owned(),
        DisplayCategory::LogMatches => {
            "No log filter is selecting lines with a position".to_owned()
        }
        _ => format!(
            "No {} in the loaded recordings",
            popup_label(category).to_lowercase()
        ),
    }
}

fn row_hover_text(category: DisplayCategory, visible: bool) -> String {
    let verb = if visible { "Hide" } else { "Show" };
    let description = match category {
        DisplayCategory::SkyGlyphs => {
            " Sky glyphs show the directions of the satellites used in each fix."
        }
        DisplayCategory::LogMatches => {
            " Log matches are the lines a log's filters selected, at the position they were \
             recorded at."
        }
        DisplayCategory::JammingHexes => return jamming_hover_text(visible),
        DisplayCategory::TecHeatmap => return tec_hover_text(visible),
        _ => "",
    };
    format!(
        "{verb} all {} on the map. Does not affect filters or the track list. \
         Alt-click to show only this category.{description}",
        popup_label(category).to_lowercase()
    )
}

/// The interference row's hover text, built from the wording every
/// interference surface shares.
fn jamming_hover_text(visible: bool) -> String {
    let verb = if visible { "Hide" } else { "Show" };
    format!(
        "{verb} the aircraft interference layer. {} {}",
        gt_jam::text::LAYER_SUMMARY,
        gt_jam::text::SOURCE_CAVEAT
    )
}

/// The TEC row's hover text, built from the wording every TEC surface shares.
fn tec_hover_text(visible: bool) -> String {
    let verb = if visible { "Hide" } else { "Show" };
    format!(
        "{verb} the ionospheric TEC heatmap. {} {}",
        gt_ionex::text::LAYER_SUMMARY,
        gt_ionex::text::SOURCE_CAVEAT
    )
}

/// One arrow of a stepper, and what it says when it cannot move.
struct StepperArrow<'a> {
    icon: &'a str,
    hover_text: &'a str,
    /// Whether the stepper accepts input at all.
    enabled: bool,
    /// Whether there is a step left in this arrow's direction.
    has_step: bool,
    /// Shown when the stepper accepts input but has reached its bound.
    at_bound_text: String,
    /// Shown when the stepper accepts no input.
    disabled_text: String,
}

/// Draw one stepper arrow, reporting whether it was clicked. Grayed at the
/// ends of the coverage window, per DESIGN.md.
fn stepper_arrow_ui(ui: &mut Ui, arrow: StepperArrow<'_>) -> bool {
    ui.add_enabled(arrow.enabled && arrow.has_step, Button::new(arrow.icon))
        .on_hover_text(arrow.hover_text)
        .on_disabled_hover_text(if arrow.enabled {
            arrow.at_bound_text
        } else {
            arrow.disabled_text
        })
        .clicked()
}

/// The instant stepper on the TEC row, grayed while the layer is hidden and
/// while a hovered or selected fix drives the instant.
fn instant_stepper_ui(ui: &mut Ui, visible: bool, tec: &mut TecRow<'_>) {
    let following = tec
        .instant
        .shown()
        .is_some_and(gt_ionex::ShownInstant::is_followed);
    let enabled = visible && !following;
    let disabled_text = || {
        if visible {
            gt_ionex::text::FOLLOWING_A_FIX.to_owned()
        } else {
            gt_ionex::text::HIDDEN_LAYER_STEPPER.to_owned()
        }
    };

    if stepper_arrow_ui(
        ui,
        StepperArrow {
            icon: ICON_CARET_LEFT,
            hover_text: "Previous map epoch",
            enabled,
            has_step: tec.instant.previous().is_some(),
            at_bound_text: TecInstantSelection::earliest_instant_text(),
            disabled_text: disabled_text(),
        },
    ) {
        tec.instant.step_back();
    }

    let label = tec
        .instant
        .instant()
        .map_or_else(|| EM_DASH.to_owned(), format_instant);
    ui.add_enabled(visible, egui::Label::new(RichText::new(label).monospace()))
        .on_hover_text(if following {
            gt_ionex::text::FOLLOWING_A_FIX
        } else {
            gt_ionex::text::INSTANT_HOVER
        });

    if stepper_arrow_ui(
        ui,
        StepperArrow {
            icon: ICON_CARET_RIGHT,
            hover_text: "Next map epoch",
            enabled,
            has_step: tec.instant.next().is_some(),
            at_bound_text: TecInstantSelection::latest_instant_text(),
            disabled_text: disabled_text(),
        },
    ) {
        tec.instant.step_forward();
    }

    ui.add_enabled(
        visible,
        egui::DragValue::new(tec.opacity_percent)
            .range(gt_ui_theme::TEC_OPACITY_PERCENT_MIN..=gt_ui_theme::TEC_OPACITY_PERCENT_MAX)
            .speed(0.5)
            .fixed_decimals(0)
            .suffix("%"),
    )
    .on_hover_text(gt_ionex::text::OPACITY_HOVER)
    .on_disabled_hover_text(gt_ionex::text::HIDDEN_LAYER_OPACITY);
}

fn format_instant(instant: DateTime<Utc>) -> String {
    instant.format(UTC_MINUTE_FORMAT).to_string()
}

/// The TEC colour scale, with the TEC unit values its stops sit at. Drawn
/// faint while the layer is hidden, per DESIGN.md.
fn legend_ui(ui: &mut Ui, visible: bool) {
    let unit = RichText::new(gt_ionex::text::LEGEND_UNIT).small().weak();
    ui.add_enabled(visible, egui::Label::new(unit));
    let label_height = ui.text_style_height(&egui::TextStyle::Small);
    let (rect, response) = ui.allocate_exact_size(
        egui::vec2(
            LEGEND_WIDTH_PX,
            LEGEND_STRIP_HEIGHT_PX + LEGEND_LABEL_GAP_PX + label_height,
        ),
        Sense::hover(),
    );
    let opacity = if visible { 1.0 } else { DISABLED_ALPHA };
    let dark_mode = ui.visuals().dark_mode;
    let strip =
        egui::Rect::from_min_size(rect.min, egui::vec2(rect.width(), LEGEND_STRIP_HEIGHT_PX));

    let mut mesh = Mesh::default();
    let column_width = strip.width() / LEGEND_COLUMNS as f32;
    for column in 0..LEGEND_COLUMNS {
        let left = strip.left() + column_width * column as f32;
        let color_at = |position: f32| {
            gt_ui_theme::tec_color_at_position(position)
                .resolve(dark_mode)
                .gamma_multiply(opacity)
        };
        let left_color = color_at(column as f32 / LEGEND_COLUMNS as f32);
        let right_color = color_at(column.saturating_add(1) as f32 / LEGEND_COLUMNS as f32);
        let quad = egui::Rect::from_min_size(
            egui::pos2(left, strip.top()),
            egui::vec2(column_width, strip.height()),
        );
        let base = u32::try_from(mesh.vertices.len()).unwrap_or_default();
        for (corner, color) in [
            (quad.left_top(), left_color),
            (quad.right_top(), right_color),
            (quad.right_bottom(), right_color),
            (quad.left_bottom(), left_color),
        ] {
            mesh.colored_vertex(corner, color);
        }
        mesh.add_triangle(base, base.saturating_add(1), base.saturating_add(2));
        mesh.add_triangle(base, base.saturating_add(2), base.saturating_add(3));
    }
    ui.painter().add(Shape::mesh(mesh));

    let text_color = ui.visuals().weak_text_color().gamma_multiply(opacity);
    let font = egui::TextStyle::Small.resolve(ui.style());
    let ticks = gt_ui_theme::TEC_LEGEND_TICKS_TECU;
    let label_top = strip.bottom() + LEGEND_LABEL_GAP_PX;
    for (index, tick) in ticks.iter().enumerate() {
        let x = strip.left() + strip.width() * gt_ui_theme::tec_scale_position(*tick);
        let anchor = if index == 0 {
            Align2::LEFT_TOP
        } else if index.saturating_add(1) == ticks.len() {
            Align2::RIGHT_TOP
        } else {
            Align2::CENTER_TOP
        };
        ui.painter().text(
            egui::pos2(x, label_top),
            anchor,
            format!("{tick:.0}"),
            font.clone(),
            text_color,
        );
    }
    response.on_hover_text(&*gt_ionex::text::SCALE_CAVEAT);
}

/// The day stepper on the interference row, grayed while the layer is hidden.
fn day_stepper_ui(ui: &mut Ui, enabled: bool, selection: &mut DaySelection) {
    if stepper_arrow_ui(
        ui,
        StepperArrow {
            icon: ICON_CARET_LEFT,
            hover_text: "Previous day",
            enabled,
            has_step: selection.previous().is_some(),
            at_bound_text: DaySelection::earliest_day_text(),
            disabled_text: HIDDEN_LAYER_TEXT.to_owned(),
        },
    ) {
        selection.step_back();
    }

    let label = selection
        .day()
        .map_or_else(|| EM_DASH.to_owned(), |day| day.to_string());
    ui.add_enabled(enabled, egui::Label::new(RichText::new(label).monospace()));

    if stepper_arrow_ui(
        ui,
        StepperArrow {
            icon: ICON_CARET_RIGHT,
            hover_text: "Next day",
            enabled,
            has_step: selection.next().is_some(),
            at_bound_text: DaySelection::latest_day_text(),
            disabled_text: HIDDEN_LAYER_TEXT.to_owned(),
        },
    ) {
        selection.step_forward();
    }
}

/// The Ring | Disc picker on the sky-glyphs row. Grayed while the category
/// is hidden (never removed, per DESIGN.md).
fn variant_picker_ui(ui: &mut Ui, visible: bool, variant: &mut SkyGlyphVariant) {
    for candidate in SkyGlyphVariant::iter() {
        let response = ui
            .add_enabled(
                visible,
                Button::selectable(*variant == candidate, candidate.label()),
            )
            .on_hover_text(variant_hover_text(candidate))
            .on_disabled_hover_text("Show sky glyphs to choose a variant");
        if response.clicked() {
            *variant = candidate;
        }
    }
}

fn variant_hover_text(variant: SkyGlyphVariant) -> &'static str {
    match variant {
        SkyGlyphVariant::Ring => "Minimal: one bead per fix satellite at its azimuth",
        SkyGlyphVariant::Disc => "Detailed: a miniature sky plot with azimuth and elevation",
    }
}

/// `true` when `mask` shows exactly `category` - the state `only` creates.
fn is_soloed(mask: DisplayMask, category: DisplayCategory) -> bool {
    DisplayCategory::iter().all(|c| mask.is_visible(c) == (c == category))
}

/// Solo the category, or restore the pre-solo mask when it is already
/// soloed (pressing `only` twice returns to where the user came from).
fn solo_or_restore(
    mask: &mut DisplayMask,
    state: &mut DisplayToggleState,
    category: DisplayCategory,
) {
    if is_soloed(*mask, category) {
        if let Some(previous) = state.solo_restore.take() {
            *mask = previous;
        }
    } else {
        // Chained solos keep the original restore point: hopping between
        // `only` rows and pressing `only` again returns to the mask the
        // user started from, not to an intermediate solo.
        let chained_solo = DisplayCategory::iter().any(|c| is_soloed(*mask, c));
        if state.solo_restore.is_none() || !chained_solo {
            state.solo_restore = Some(*mask);
        }
        mask.solo(category);
    }
}

/// Show the eye button anchored above `below_rect` (the map-layer toggle)
/// and, while open, the category popup above it. `counts` is computed
/// lazily - only an open popup pays for the full scan behind it.
pub(crate) fn show_display_toggle(
    ui: &Ui,
    below_rect: egui::Rect,
    state: &mut DisplayToggleState,
    mask: &mut DisplayMask,
    sky_glyph_variant: &mut SkyGlyphVariant,
    mut layers: LayerRows<'_>,
    counts: impl FnOnce() -> DisplayCounts,
) {
    let gap = ui.style().spacing.item_spacing.y;
    let button_response = Area::new(Id::new(DISPLAY_TOGGLE_BUTTON_AREA_ID))
        .fixed_pos(egui::pos2(below_rect.right(), below_rect.top() - gap))
        .pivot(Align2::RIGHT_BOTTOM)
        .show(ui.ctx(), |ui| {
            Frame::popup(ui.style()).show(ui, |ui| {
                let any_hidden = mask.any_hidden();
                let glyph = if any_hidden { ICON_EYE_SLASH } else { ICON_EYE };
                let text = if any_hidden {
                    RichText::new(glyph).color(ui.visuals().warn_fg_color)
                } else {
                    RichText::new(glyph)
                };
                let hover = match mask.hidden_count() {
                    0 => "Show or hide map elements".to_owned(),
                    1 => "1 element kind hidden - click to review".to_owned(),
                    n => format!("{n} element kinds hidden - click to review"),
                };
                let response = ui.selectable_label(state.open, text).on_hover_text(hover);
                if response.clicked() {
                    state.open = !state.open;
                }
                response.context_menu(|ui| {
                    if ui
                        .add_enabled(mask.any_hidden(), Button::new("Show all"))
                        .on_disabled_hover_text("Everything is already shown")
                        .clicked()
                    {
                        mask.show_all();
                        ui.close();
                    }
                    if ui.button("Hide all markers").clicked() {
                        mask.set_visible(DisplayCategory::CustomMarkers, false);
                        mask.set_visible(DisplayCategory::GeneratedMarkers, false);
                        // Direct mask edits invalidate the solo restore
                        // point, same as the footer buttons.
                        state.solo_restore = None;
                        ui.close();
                    }
                });
                response
            })
        });

    if !state.open {
        return;
    }
    let button_rect = button_response.response.rect;
    let counts = counts();
    let popup_response = Area::new(Id::new(DISPLAY_TOGGLE_POPUP_AREA_ID))
        .fixed_pos(egui::pos2(button_rect.right(), button_rect.top() - gap))
        .pivot(Align2::RIGHT_BOTTOM)
        .show(ui.ctx(), |ui| {
            Frame::popup(ui.style()).show(ui, |ui| {
                popup_contents(ui, state, mask, sky_glyph_variant, &mut layers, counts);
            });
        });

    // Close on click-outside. Clicks on the eye button itself already toggled
    // `open` above, and clicks inside the popup keep it open so multi-toggling
    // is one gesture.
    if popup_response.response.clicked_elsewhere()
        && button_response.inner.inner.clicked_elsewhere()
    {
        state.open = false;
    }
}

/// The popup body: one row per category plus the footer. Extracted from
/// the `Area` wiring so tests can snapshot it directly.
pub(crate) fn popup_contents(
    ui: &mut Ui,
    state: &mut DisplayToggleState,
    mask: &mut DisplayMask,
    sky_glyph_variant: &mut SkyGlyphVariant,
    layers: &mut LayerRows<'_>,
    counts: DisplayCounts,
) {
    for category in DisplayCategory::iter() {
        let count = counts.get(category);
        let visible = mask.is_visible(category);
        ui.horizontal(|ui| {
            let glyph = if visible { ICON_EYE } else { ICON_EYE_SLASH };
            let text = if visible {
                RichText::new(format!("{glyph} {}", popup_label(category)))
            } else {
                RichText::new(format!("{glyph} {}", popup_label(category))).weak()
            };
            let in_scope = count > 0
                || matches!(
                    category,
                    DisplayCategory::JammingHexes | DisplayCategory::TecHeatmap
                );
            let row = ui
                .add_enabled(in_scope, Button::selectable(false, text))
                .on_hover_text(row_hover_text(category, visible))
                .on_disabled_hover_text(empty_hover_text(category));
            let alt_held = ui.input(|i| i.modifiers.alt);
            if row.clicked() {
                if alt_held {
                    solo_or_restore(mask, state, category);
                } else {
                    mask.toggle(category);
                }
            }
            if category == DisplayCategory::SkyGlyphs {
                variant_picker_ui(ui, visible && in_scope, sky_glyph_variant);
            }
            let empty_badge = match category {
                DisplayCategory::JammingHexes => layers
                    .interference
                    .empty_reason
                    .map(|reason| (reason.badge(), reason.message())),
                DisplayCategory::TecHeatmap => layers
                    .tec
                    .empty_reason
                    .map(|reason| (reason.badge(), reason.message())),
                _ => None,
            };
            if let Some((badge, message)) = empty_badge
                && visible
            {
                ui.label(
                    RichText::new(badge)
                        .small()
                        .color(gt_ui_theme::WARNING.resolve(ui.visuals().dark_mode)),
                )
                .on_hover_text(message);
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                // Fixed-width count column so the eye glyphs align.
                ui.allocate_ui(egui::vec2(COUNT_COLUMN_WIDTH_PX, row.rect.height()), |ui| {
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(RichText::new(gt_fmt::format_count(count)).weak());
                    });
                });
                // The solo button shows on row hover and stays while the
                // category is soloed so it can be un-soloed.
                if in_scope && (row.hovered() || is_soloed(*mask, category)) {
                    let only = ui
                        .small_button("only")
                        .on_hover_text("Show only this category. Press again to restore");
                    if only.clicked() {
                        solo_or_restore(mask, state, category);
                    }
                }
            });
        });
        if category == DisplayCategory::JammingHexes {
            ui.horizontal(|ui| {
                ui.add_space(STEPPER_INDENT_PX);
                day_stepper_ui(ui, visible, layers.interference.day);
            });
        }
        if category == DisplayCategory::TecHeatmap {
            ui.horizontal(|ui| {
                ui.add_space(STEPPER_INDENT_PX);
                instant_stepper_ui(ui, visible, &mut layers.tec);
            });
            ui.horizontal(|ui| {
                ui.add_space(STEPPER_INDENT_PX);
                legend_ui(ui, visible);
            });
        }
    }
    ui.separator();
    ui.horizontal(|ui| {
        if ui
            .add_enabled(mask.any_hidden(), Button::new("Show all"))
            .on_disabled_hover_text("Everything is already shown")
            .clicked()
        {
            mask.show_all();
            state.solo_restore = None;
        }
        if ui
            .add_enabled(
                mask.hidden_count() < DisplayCategory::iter().count(),
                Button::new("Hide all"),
            )
            .on_disabled_hover_text("Everything is already hidden")
            .clicked()
        {
            mask.hide_all();
            state.solo_restore = None;
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The counts a loaded recording and a downloaded day produce. The
    /// archive-supplied ones are given per case, so an empty-state case can
    /// show the zero a day nothing was downloaded for produces.
    fn counts_with_archived(jamming_hexes: usize, tec_nodes: usize) -> DisplayCounts {
        DisplayCounts::from_fn(|category| match category {
            DisplayCategory::Tracks => 12,
            DisplayCategory::TrackPoints => 8940,
            DisplayCategory::SatelliteLabels => 214,
            DisplayCategory::CustomMarkers => 37,
            DisplayCategory::GeneratedMarkers => 1482,
            DisplayCategory::EventMarkers => 96,
            DisplayCategory::QueryHighlights => 0,
            DisplayCategory::SnappedTracks => 3,
            DisplayCategory::SkyGlyphs => 187,
            DisplayCategory::JammingHexes => jamming_hexes,
            DisplayCategory::TecHeatmap => tec_nodes,
            DisplayCategory::LogMatches => 42,
        })
    }

    #[test]
    fn solo_and_restore_round_trip() {
        let mut state = DisplayToggleState::default();
        let mut mask = DisplayMask::default();
        mask.set_visible(DisplayCategory::EventMarkers, false);
        let before = mask;

        solo_or_restore(&mut mask, &mut state, DisplayCategory::Tracks);
        assert!(is_soloed(mask, DisplayCategory::Tracks));

        // Soloing another category re-solos without touching the restore
        // point, so `only` still returns to the original mask.
        solo_or_restore(&mut mask, &mut state, DisplayCategory::CustomMarkers);
        assert!(is_soloed(mask, DisplayCategory::CustomMarkers));

        solo_or_restore(&mut mask, &mut state, DisplayCategory::CustomMarkers);
        assert_eq!(mask, before);
    }

    /// One popup snapshot's inputs.
    struct PopupCase {
        name: &'static str,
        /// The category hidden on top of the default mask.
        hidden: DisplayCategory,
        /// Whether the TEC row is shown, which enables its own controls.
        tec_shown: bool,
        interference_empty: Option<EmptyReason>,
        tec_empty: Option<TecEmptyReason>,
        /// Whether a hovered or selected fix drives the TEC instant, which
        /// grays the stepper.
        tec_follows_a_fix: bool,
        dark_mode: bool,
    }

    impl PopupCase {
        fn new(name: &'static str) -> Self {
            Self {
                name,
                hidden: DisplayCategory::GeneratedMarkers,
                tec_shown: false,
                interference_empty: None,
                tec_empty: None,
                tec_follows_a_fix: false,
                dark_mode: true,
            }
        }
    }

    /// Snapshot: mixed state - generated markers hidden (dimmed row with
    /// the eye-slash and the persistent `only` state elsewhere absent),
    /// query highlights at zero (disabled row), and both archive-backed
    /// layers at their default-hidden state with their controls grayed.
    #[rstest::rstest]
    #[case::mixed(PopupCase::new("display_toggle_popup"))]
    #[case::sky_glyphs_hidden(PopupCase {
        hidden: DisplayCategory::SkyGlyphs,
        ..PopupCase::new("display_toggle_popup_sky_hidden")
    })]
    #[case::interference_empty(PopupCase {
        interference_empty: Some(EmptyReason::NotFetched),
        ..PopupCase::new("display_toggle_popup_interference_empty")
    })]
    #[case::interference_empty_light(PopupCase {
        interference_empty: Some(EmptyReason::NotFetched),
        dark_mode: false,
        ..PopupCase::new("display_toggle_popup_interference_empty_light")
    })]
    #[case::tec_shown(PopupCase {
        tec_shown: true,
        ..PopupCase::new("display_toggle_popup_tec_shown")
    })]
    #[case::tec_shown_light(PopupCase {
        tec_shown: true,
        dark_mode: false,
        ..PopupCase::new("display_toggle_popup_tec_shown_light")
    })]
    #[case::tec_empty(PopupCase {
        tec_shown: true,
        tec_empty: Some(TecEmptyReason::NotArchived),
        ..PopupCase::new("display_toggle_popup_tec_empty")
    })]
    #[case::tec_follows_a_fix(PopupCase {
        tec_shown: true,
        tec_follows_a_fix: true,
        ..PopupCase::new("display_toggle_popup_tec_following")
    })]
    fn snap_display_toggle_popup(#[case] case: PopupCase) {
        let mut state = DisplayToggleState::default();
        let mut mask = DisplayMask::default();
        mask.set_visible(case.hidden, false);
        mask.set_visible(DisplayCategory::TecHeatmap, case.tec_shown);
        let mut variant = SkyGlyphVariant::default();
        let mut day_selection = DaySelection::new(
            chrono::NaiveDate::from_ymd_opt(2026, 7, 20),
            chrono::NaiveDate::from_ymd_opt(2026, 7, 31).unwrap_or_default(),
        );
        let mut instant = TecInstantSelection::new(
            chrono::NaiveDate::from_ymd_opt(2024, 5, 10)
                .and_then(|day| day.and_hms_opt(18, 0, 0))
                .map(|naive| naive.and_utc()),
            chrono::NaiveDate::from_ymd_opt(2026, 7, 31).unwrap_or_default(),
        );
        if case.tec_follows_a_fix {
            instant.follow(
                chrono::NaiveDate::from_ymd_opt(2024, 5, 10)
                    .and_then(|day| day.and_hms_opt(6, 34, 0))
                    .map(|naive| naive.and_utc()),
            );
        }
        let mut opacity_percent = gt_ui_theme::TEC_OPACITY_PERCENT_DEFAULT;
        let counts = counts_with_archived(
            if case.interference_empty.is_some() {
                0
            } else {
                1043
            },
            if case.tec_empty.is_some() { 0 } else { 5183 },
        );

        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(320.0, 360.0))
            .theme(case.dark_mode)
            .ui(move |ui| {
                Frame::popup(ui.style()).show(ui, |ui| {
                    popup_contents(
                        ui,
                        &mut state,
                        &mut mask,
                        &mut variant,
                        &mut LayerRows {
                            interference: InterferenceRow {
                                day: &mut day_selection,
                                empty_reason: case.interference_empty,
                            },
                            tec: TecRow {
                                instant: &mut instant,
                                opacity_percent: &mut opacity_percent,
                                empty_reason: case.tec_empty,
                            },
                        },
                        counts,
                    );
                });
            });
        harness.fit_contents();
        harness.snapshot(case.name);
    }
}
