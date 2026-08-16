//! The display toggle - the on-map eye button and its popup, where whole
//! element categories are shown or hidden via the [`DisplayMask`].
//!
//! The popup lists every [`DisplayCategory`] with its in-scope count
//! ([`DisplayCounts`]).

use egui::Button;
use egui::{Align2, Area, Frame, Id, RichText, Ui};
use egui_phosphor::regular::CARET_LEFT as ICON_CARET_LEFT;
use egui_phosphor::regular::CARET_RIGHT as ICON_CARET_RIGHT;
use egui_phosphor::regular::EYE as ICON_EYE;
use egui_phosphor::regular::EYE_SLASH as ICON_EYE_SLASH;
use gt_jam::day_selection::{DaySelection, EmptyReason};
use gt_ui_theme::EM_DASH;
use gt_ui_types::{DisplayCategory, DisplayMask, SkyGlyphVariant};
use strum::IntoEnumIterator;

use crate::display_counts::DisplayCounts;

/// Width of the right-aligned count column, sized for the widest expected
/// count (`999,999`) so the eye glyphs stay aligned across rows.
const COUNT_COLUMN_WIDTH_PX: f32 = 48.0;

/// Indent of the interference row's day stepper, marking it as that row's
/// detail.
const STEPPER_INDENT_PX: f32 = 24.0;

/// Disabled hover text while the interference layer is hidden.
const HIDDEN_LAYER_TEXT: &str = "Show the interference layer to step days";

/// The interference row's own state: which day it shows, and why it has
/// nothing to draw.
pub(crate) struct InterferenceRow<'a> {
    pub(crate) day: &'a mut DaySelection,
    pub(crate) empty_reason: Option<EmptyReason>,
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
    }
}

/// Why a category's row is disabled.
fn empty_hover_text(category: DisplayCategory) -> String {
    match category {
        DisplayCategory::JammingHexes => "No interference data archived for this day".to_owned(),
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
        DisplayCategory::JammingHexes => return jamming_hover_text(visible),
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

/// The day stepper on the interference row. Grayed at the ends of the
/// coverage window, per DESIGN.md.
fn day_stepper_ui(ui: &mut Ui, enabled: bool, selection: &mut DaySelection) {
    let back = ui
        .add_enabled(
            enabled && selection.previous().is_some(),
            Button::new(ICON_CARET_LEFT),
        )
        .on_hover_text("Previous day")
        .on_disabled_hover_text(if enabled {
            DaySelection::earliest_day_text()
        } else {
            HIDDEN_LAYER_TEXT.to_owned()
        });
    if back.clicked() {
        selection.step_back();
    }

    let label = selection
        .day()
        .map_or_else(|| EM_DASH.to_owned(), |day| day.to_string());
    ui.add_enabled(enabled, egui::Label::new(RichText::new(label).monospace()));

    let forward = ui
        .add_enabled(
            enabled && selection.next().is_some(),
            Button::new(ICON_CARET_RIGHT),
        )
        .on_hover_text("Next day")
        .on_disabled_hover_text(if enabled {
            DaySelection::latest_day_text()
        } else {
            HIDDEN_LAYER_TEXT.to_owned()
        });
    if forward.clicked() {
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
    mut interference: InterferenceRow<'_>,
    counts: impl FnOnce() -> DisplayCounts,
) {
    let gap = ui.style().spacing.item_spacing.y;
    let button_response = Area::new(Id::new("display_toggle_button"))
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
    let popup_response = Area::new(Id::new("display_toggle_popup"))
        .fixed_pos(egui::pos2(button_rect.right(), button_rect.top() - gap))
        .pivot(Align2::RIGHT_BOTTOM)
        .show(ui.ctx(), |ui| {
            Frame::popup(ui.style()).show(ui, |ui| {
                popup_contents(
                    ui,
                    state,
                    mask,
                    sky_glyph_variant,
                    &mut interference,
                    counts,
                );
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
    interference: &mut InterferenceRow<'_>,
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
            let in_scope = count > 0 || category == DisplayCategory::JammingHexes;
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
            if category == DisplayCategory::JammingHexes
                && let Some(reason) = interference.empty_reason
                && visible
            {
                ui.label(
                    RichText::new(reason.badge())
                        .small()
                        .color(gt_ui_theme::WARNING.resolve(ui.visuals().dark_mode)),
                )
                .on_hover_text(reason.message());
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
                day_stepper_ui(ui, visible, interference.day);
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

    fn mixed_counts() -> DisplayCounts {
        counts_with_interference(1043)
    }

    /// [`mixed_counts`] with the interference count set, so the empty-state
    /// case can show the zero a non-downloaded day produces.
    fn counts_with_interference(jamming_hexes: usize) -> DisplayCounts {
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

    /// Snapshot: mixed state - generated markers hidden (dimmed row with
    /// the eye-slash and the persistent `only` state elsewhere absent),
    /// query highlights at zero (disabled row). The second case hides the
    /// sky glyphs, graying their variant picker.
    #[rstest::rstest]
    #[case::mixed("display_toggle_popup", DisplayCategory::GeneratedMarkers, None, true)]
    #[case::sky_glyphs_hidden(
        "display_toggle_popup_sky_hidden",
        DisplayCategory::SkyGlyphs,
        None,
        true
    )]
    #[case::interference_empty(
        "display_toggle_popup_interference_empty",
        DisplayCategory::GeneratedMarkers,
        Some(EmptyReason::NotFetched),
        true
    )]
    #[case::interference_empty_light(
        "display_toggle_popup_interference_empty_light",
        DisplayCategory::GeneratedMarkers,
        Some(EmptyReason::NotFetched),
        false
    )]
    fn snap_display_toggle_popup(
        #[case] name: &str,
        #[case] hidden: DisplayCategory,
        #[case] empty_reason: Option<EmptyReason>,
        #[case] dark_mode: bool,
    ) {
        let mut state = DisplayToggleState::default();
        let mut mask = DisplayMask::default();
        mask.set_visible(hidden, false);
        let mut variant = SkyGlyphVariant::default();
        let mut day_selection = DaySelection::new(
            chrono::NaiveDate::from_ymd_opt(2026, 7, 20),
            chrono::NaiveDate::from_ymd_opt(2026, 7, 31).unwrap_or_default(),
        );
        let counts = if empty_reason.is_some() {
            counts_with_interference(0)
        } else {
            mixed_counts()
        };

        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(280.0, 300.0))
            .theme(dark_mode)
            .ui(move |ui| {
                Frame::popup(ui.style()).show(ui, |ui| {
                    popup_contents(
                        ui,
                        &mut state,
                        &mut mask,
                        &mut variant,
                        &mut InterferenceRow {
                            day: &mut day_selection,
                            empty_reason,
                        },
                        counts,
                    );
                });
            });
        harness.fit_contents();
        harness.snapshot(name);
    }
}
