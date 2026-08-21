//! The warning indicator in the map's top-right corner: a storm glyph whose
//! hover lists the environment metrics that could have disturbed the loaded
//! recordings, and whose click opens the levels each metric warns at.

use egui::{Align2, Area, CursorIcon, Frame, Id, Response, RichText, Ui};
use egui_phosphor::regular::BOOK_OPEN_TEXT as ICON_BOOK_OPEN_TEXT;
use egui_phosphor::regular::CLOUD_LIGHTNING as ICON_CLOUD_LIGHTNING;
use gt_ui_types::WarningLevelExplanation;
use gt_ui_types::reference::ReferenceDocument;

/// Inset from the map's top-right corner, matching the layer picker's inset
/// from the bottom-right one.
const CORNER_INSET_PX: f32 = 8.0;

const IDLE_HOVER_TEXT: &str = "No environment warnings. Warnings appear here when archived \
                               interference, geomagnetic, solar flare or TEC deviation data \
                               crosses a warning level during a loaded recording.";

const LEVELS_TITLE: &str = "Environment warning levels";

/// Width the level rows wrap at, which is what sets the popup's width.
const LEVELS_WIDTH_PX: f32 = 420.0;

/// Space above each level row, separating it from the row's link above.
const LEVEL_ROW_SPACING_PX: f32 = 8.0;

/// What the indicator shows: the evidence its hover lists, and the levels its
/// popup explains.
#[derive(Clone, Copy)]
pub struct SpaceWeatherIndicator<'a> {
    /// One line per environment metric that reached its disturbance level over
    /// a loaded recording, empty while the archives place none.
    pub warning_lines: &'a [String],
    /// One row per metric, stating the level at which it raises a warning.
    pub levels: &'a [WarningLevelExplanation],
}

/// Session-only state of the indicator: whether its levels popup is open.
#[derive(Default)]
pub(crate) struct SpaceWeatherIndicatorState {
    levels_open: bool,
}

/// Show the indicator over the map's top-right corner, which the map's other
/// floating controls leave free.
///
/// Drawn whether or not `indicator` holds a warning, so the glyph can be found
/// before one is raised. With no warning it renders in the weak text colour
/// and hovers with [`IDLE_HOVER_TEXT`] in place of the list.
///
/// Returns the reference document whose link was clicked this frame, for the
/// caller to open its window on.
pub(crate) fn show_space_weather_warning(
    ui: &Ui,
    map_rect: egui::Rect,
    state: &mut SpaceWeatherIndicatorState,
    indicator: SpaceWeatherIndicator<'_>,
) -> Option<ReferenceDocument> {
    let glyph = Area::new(Id::new("map_space_weather_warning"))
        .fixed_pos(egui::pos2(
            map_rect.right() - CORNER_INSET_PX,
            map_rect.top() + CORNER_INSET_PX,
        ))
        .pivot(Align2::RIGHT_TOP)
        .show(ui.ctx(), |ui| {
            Frame::popup(ui.style())
                .show(ui, |ui| {
                    glyph_ui(ui, state.levels_open, indicator.warning_lines)
                })
                .inner
        });
    if glyph.inner.clicked() {
        state.levels_open = !state.levels_open;
    }
    if !state.levels_open {
        return None;
    }

    let glyph_rect = glyph.response.rect;
    let popup = Area::new(Id::new("map_space_weather_levels"))
        .fixed_pos(egui::pos2(
            glyph_rect.right(),
            glyph_rect.bottom() + ui.style().spacing.item_spacing.y,
        ))
        .pivot(Align2::RIGHT_TOP)
        .show(ui.ctx(), |ui| {
            Frame::popup(ui.style())
                .show(ui, |ui| levels_ui(ui, indicator.levels))
                .inner
        });

    // Clicks on the glyph itself already toggled the popup above, and clicks
    // inside it are its own links.
    let clicked_outside = popup.response.clicked_elsewhere() && glyph.inner.clicked_elsewhere();
    let opened_document = popup.inner;
    if clicked_outside
        || opened_document.is_some()
        || ui.input(|i| i.key_pressed(egui::Key::Escape))
    {
        state.levels_open = false;
    }
    opened_document
}

/// The glyph itself, in the warning colour once a metric has reached its
/// level, hovering the evidence behind it.
fn glyph_ui(ui: &mut Ui, levels_open: bool, warning_lines: &[String]) -> Response {
    let color = if warning_lines.is_empty() {
        ui.visuals().weak_text_color()
    } else {
        gt_ui_theme::WARNING.resolve(ui.visuals().dark_mode)
    };
    ui.selectable_label(
        levels_open,
        RichText::new(ICON_CLOUD_LIGHTNING).color(color),
    )
    .on_hover_cursor(CursorIcon::PointingHand)
    .on_hover_ui(|ui| match warning_lines {
        [] => {
            ui.label(IDLE_HOVER_TEXT);
        }
        lines => {
            for line in lines {
                ui.label(line);
            }
        }
    })
}

/// The popup body: one row per metric, each stating its level over the link
/// to the material behind it. Returns the document whose link was clicked.
fn levels_ui(ui: &mut Ui, levels: &[WarningLevelExplanation]) -> Option<ReferenceDocument> {
    ui.set_max_width(LEVELS_WIDTH_PX);
    ui.label(RichText::new(LEVELS_TITLE).strong());
    let mut opened_document = None;
    for level in levels {
        ui.add_space(LEVEL_ROW_SPACING_PX);
        ui.label(&level.trigger);
        if ui
            .link(format!(
                "{ICON_BOOK_OPEN_TEXT} {}",
                level.reference.link_question
            ))
            .clicked()
        {
            opened_document = Some(level.reference);
        }
    }
    opened_document
}

#[cfg(test)]
mod tests {
    use egui_kittest::kittest::{By, Queryable as _};
    use gt_test_utils::HarnessInteraction as _;
    use rstest::rstest;

    use super::*;

    /// Frames the pointer holds still for, which covers egui's tooltip delay.
    const TOOLTIP_SETTLE_FRAMES: usize = 60;

    /// What the indicator keeps between frames, plus the document its popup
    /// asked for, which the harness reads back after the click.
    #[derive(Default)]
    struct IndicatorState {
        indicator: SpaceWeatherIndicatorState,
        opened_document: Option<ReferenceDocument>,
    }

    fn one_warning() -> Vec<String> {
        vec!["Geomagnetic storm: Hp30 reached 7.667 (G3)".to_owned()]
    }

    /// Two rows, so the popup is exercised against a list. The application
    /// supplies the wording it lists.
    fn levels() -> Vec<WarningLevelExplanation> {
        vec![
            WarningLevelExplanation {
                trigger: "Aircraft interference: 2 % or more of aircraft in a crossed cell."
                    .to_owned(),
                reference: gt_jam::reference::AIRCRAFT_INTERFERENCE,
            },
            WarningLevelExplanation {
                trigger: gt_ionex::text::DEVIATION_WARNING_TRIGGER.clone(),
                reference: gt_ionex::reference::IONOSPHERIC_TEC,
            },
        ]
    }

    fn harness(warning_lines: Vec<String>) -> gt_test_utils::TestHarness<'static, IndicatorState> {
        let levels = levels();
        let mut harness = crate::test_harness::builder()
            .size(egui::vec2(600.0, 400.0))
            .ui_state(
                move |ui, state: &mut IndicatorState| {
                    let opened = show_space_weather_warning(
                        ui,
                        ui.max_rect(),
                        &mut state.indicator,
                        SpaceWeatherIndicator {
                            warning_lines: &warning_lines,
                            levels: &levels,
                        },
                    );
                    state.opened_document = opened.or(state.opened_document);
                },
                IndicatorState::default(),
            );
        harness.run();
        harness
    }

    fn click_the_glyph(harness: &mut gt_test_utils::TestHarness<'_, IndicatorState>) {
        harness.inner.get_by_label(ICON_CLOUD_LIGHTNING).click();
        harness.run();
    }

    /// A click on bare map, away from the indicator and its popup.
    fn click_the_map(harness: &mut gt_test_utils::TestHarness<'_, IndicatorState>) {
        let target = egui::pos2(40.0, 360.0);
        harness.inner.hover_at(target);
        harness.run();
        harness.inner.drag_at(target);
        harness.inner.drop_at(target);
        harness.run();
    }

    fn press_escape(harness: &mut gt_test_utils::TestHarness<'_, IndicatorState>) {
        harness.inner.key_press(egui::Key::Escape);
        harness.run();
    }

    fn levels_popup_is_open(harness: &gt_test_utils::TestHarness<'_, IndicatorState>) -> bool {
        harness.inner.query_by_label(LEVELS_TITLE).is_some()
    }

    /// The glyph is on the map either way. What a metric reaching its
    /// disturbance level changes is the hover behind it.
    #[rstest]
    #[case::warned(one_warning(), "Geomagnetic storm: Hp30 reached 7.667 (G3)")]
    #[case::idle(Vec::new(), IDLE_HOVER_TEXT)]
    fn the_hover_lists_the_warnings_or_explains_the_idle_glyph(
        #[case] lines: Vec<String>,
        #[case] expected_hover: &str,
    ) {
        let mut harness = harness(lines);

        assert!(harness.inner.query_by_label(ICON_CLOUD_LIGHTNING).is_some());
        harness
            .inner
            .hover_and_settle(By::new().label(ICON_CLOUD_LIGHTNING), TOOLTIP_SETTLE_FRAMES);

        assert!(
            harness
                .inner
                .query_by_label_contains(expected_hover)
                .is_some()
        );
    }

    /// The glyph requests the pointing-hand cursor in both states.
    #[rstest]
    #[case::warned(one_warning())]
    #[case::idle(Vec::new())]
    fn the_glyph_requests_the_pointing_hand(#[case] lines: Vec<String>) {
        let mut harness = harness(lines);
        let glyph = harness.inner.get_by_label(ICON_CLOUD_LIGHTNING).rect();

        harness.inner.hover_at_and_settle(glyph.center(), 2);

        assert_eq!(
            harness.inner.output().platform_output.cursor_icon,
            CursorIcon::PointingHand
        );
    }

    /// The same levels are listed whether or not a metric warns: the hover
    /// states what happened, the popup what the levels are.
    #[rstest]
    #[case::warned(one_warning())]
    #[case::idle(Vec::new())]
    fn clicking_the_glyph_lists_every_level(#[case] lines: Vec<String>) {
        let mut harness = harness(lines);

        click_the_glyph(&mut harness);

        assert!(levels_popup_is_open(&harness));
        for level in levels() {
            assert!(
                harness
                    .inner
                    .query_by_label_contains(&level.trigger)
                    .is_some(),
                "the popup never states {:?}",
                level.trigger
            );
        }
    }

    /// Escape and a click on the map close the popup, as they close the map's
    /// other popups.
    #[rstest]
    #[case::escape(press_escape)]
    #[case::click_outside(click_the_map)]
    fn the_popup_closes_on_escape_and_on_a_click_outside(
        #[case] dismiss: fn(&mut gt_test_utils::TestHarness<'_, IndicatorState>),
    ) {
        let mut harness = harness(one_warning());
        click_the_glyph(&mut harness);
        assert!(levels_popup_is_open(&harness));

        dismiss(&mut harness);

        assert!(!levels_popup_is_open(&harness));
    }

    /// Clicking the glyph again closes the popup it opened.
    #[test]
    fn clicking_the_glyph_again_closes_the_popup() {
        let mut harness = harness(one_warning());

        click_the_glyph(&mut harness);
        click_the_glyph(&mut harness);

        assert!(!levels_popup_is_open(&harness));
    }

    /// A row's link reports its document, which is what opens the reference
    /// window, and closes the popup behind it.
    #[test]
    fn a_row_link_reports_its_reference_document() {
        let mut harness = harness(Vec::new());
        click_the_glyph(&mut harness);

        harness
            .inner
            .get_by_label_contains(gt_jam::reference::AIRCRAFT_INTERFERENCE.link_question)
            .click();
        harness.run();

        assert_eq!(
            harness.state().opened_document,
            Some(gt_jam::reference::AIRCRAFT_INTERFERENCE)
        );
        assert!(!levels_popup_is_open(&harness));
    }
}
