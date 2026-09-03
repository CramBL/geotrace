//! The warning indicator in the map's top-right corner: a storm glyph whose
//! hover names the tracks the archived environment values could have disturbed
//! and what each metric reached over them, and whose click opens the whole
//! list over the levels each metric warns at.

use egui::scroll_area::ScrollBarVisibility;
use egui::{Align2, Area, CursorIcon, Frame, Id, Response, RichText, ScrollArea, Ui};
use egui_phosphor::regular::BOOK_OPEN_TEXT as ICON_BOOK_OPEN_TEXT;
use egui_phosphor::regular::CLOUD_LIGHTNING as ICON_CLOUD_LIGHTNING;
use gt_ui_types::reference::ReferenceDocument;
use gt_ui_types::{TrackSpaceWeatherWarning, WarningLevelExplanation};

/// Inset from the map's top-right corner, matching the layer picker's inset
/// from the bottom-right one.
const CORNER_INSET_PX: f32 = 8.0;

const IDLE_HOVER_TEXT: &str = "No environment warnings. Warnings appear here when archived \
                               interference, geomagnetic, solar flare or TEC deviation data \
                               crosses a warning level during a loaded recording.";

/// Closes the hover, since a hover cannot be clicked through to the tracks it
/// left out.
const CLICK_FOR_EVERY_TRACK: &str = "Click for every affected track and the warning levels";

const AFFECTED_TRACKS_TITLE: &str = "Affected tracks";

const NO_AFFECTED_TRACKS: &str = "No loaded track reached a warning level";

const LEVELS_TITLE: &str = "Environment warning levels";

/// How many affected tracks the hover names before summarizing the rest, so a
/// session of many disturbed tracks still produces a readable tooltip. The
/// popup a click opens lists every one of them.
const MAX_HOVER_TRACKS: usize = 5;

/// Width the level rows wrap at, which is what sets the popup's width.
const LEVELS_WIDTH_PX: f32 = 420.0;

/// Height the affected-track list scrolls past, which keeps the levels below
/// it on screen.
const AFFECTED_TRACKS_MAX_HEIGHT_PX: f32 = 240.0;

/// Space above each level row, separating it from the row's link above.
const LEVEL_ROW_SPACING_PX: f32 = 8.0;

/// Space above each affected track after the first, separating it from the
/// lines of the track above.
const TRACK_SPACING_PX: f32 = 6.0;

/// What the indicator shows: the tracks its hover names, and the levels its
/// popup explains.
#[derive(Clone, Copy)]
pub struct SpaceWeatherIndicator<'a> {
    /// One entry per loaded track an environment metric reached its
    /// disturbance level over, empty while the archives place none.
    pub track_warnings: &'a [TrackSpaceWeatherWarning],
    /// One row per metric, stating the level at which it raises a warning.
    pub levels: &'a [WarningLevelExplanation],
    /// What a stated TEC grade is measured against, shown once by every
    /// surface that lists a track stating one.
    pub tec_deviation_caveat: &'a str,
}

/// Session-only state of the indicator: whether its popup is open.
#[derive(Default)]
pub(crate) struct SpaceWeatherIndicatorState {
    details_open: bool,
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
                .show(ui, |ui| glyph_ui(ui, state.details_open, indicator))
                .inner
        });
    if glyph.inner.clicked() {
        state.details_open = !state.details_open;
    }
    if !state.details_open {
        return None;
    }

    let glyph_rect = glyph.response.rect;
    let popup = Area::new(Id::new("map_space_weather_details"))
        .fixed_pos(egui::pos2(
            glyph_rect.right(),
            glyph_rect.bottom() + ui.style().spacing.item_spacing.y,
        ))
        .pivot(Align2::RIGHT_TOP)
        .show(ui.ctx(), |ui| {
            Frame::popup(ui.style())
                .show(ui, |ui| details_ui(ui, indicator))
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
        state.details_open = false;
    }
    opened_document
}

/// The glyph itself, in the warning colour once a metric has reached its
/// level, hovering the tracks behind it.
fn glyph_ui(ui: &mut Ui, details_open: bool, indicator: SpaceWeatherIndicator<'_>) -> Response {
    let color = if indicator.track_warnings.is_empty() {
        ui.visuals().weak_text_color()
    } else {
        gt_ui_theme::WARNING.resolve(ui.visuals().dark_mode)
    };
    ui.selectable_label(
        details_open,
        RichText::new(ICON_CLOUD_LIGHTNING).color(color),
    )
    .on_hover_cursor(CursorIcon::PointingHand)
    .on_hover_ui(|ui| hover_ui(ui, indicator))
}

/// The hover body: the first [`MAX_HOVER_TRACKS`] affected tracks with their
/// own values, then how many the popup holds beyond them, and what a stated
/// TEC grade is measured against.
fn hover_ui(ui: &mut Ui, indicator: SpaceWeatherIndicator<'_>) {
    let track_warnings = indicator.track_warnings;
    if track_warnings.is_empty() {
        ui.label(IDLE_HOVER_TEXT);
        return;
    }
    let named = track_warnings
        .get(..MAX_HOVER_TRACKS)
        .unwrap_or(track_warnings);
    for (position, warning) in named.iter().enumerate() {
        if position > 0 {
            ui.add_space(TRACK_SPACING_PX);
        }
        track_warning_ui(ui, warning);
    }
    let left_out = track_warnings.len().saturating_sub(MAX_HOVER_TRACKS);
    if left_out > 0 {
        ui.add_space(TRACK_SPACING_PX);
        ui.label(
            RichText::new(format!(
                "and {left_out} more {}",
                gt_fmt::pluralize(left_out, "track", "tracks")
            ))
            .weak(),
        );
    }
    tec_deviation_caveat_ui(ui, indicator.tec_deviation_caveat, named);
    ui.add_space(TRACK_SPACING_PX);
    ui.label(RichText::new(CLICK_FOR_EVERY_TRACK).weak());
}

/// What a stated TEC grade is measured against, shown once where any of the
/// tracks the surface lists states one.
fn tec_deviation_caveat_ui(ui: &mut Ui, caveat: &str, listed: &[TrackSpaceWeatherWarning]) {
    if !listed.iter().any(|warning| warning.states_tec_deviation) {
        return;
    }
    ui.add_space(TRACK_SPACING_PX);
    ui.label(RichText::new(caveat).weak());
}

/// One track's block: the track as the rest of the app names it, over one line
/// per metric that reached its level on it.
fn track_warning_ui(ui: &mut Ui, warning: &TrackSpaceWeatherWarning) {
    ui.label(RichText::new(&warning.track_label).strong());
    for line in &warning.lines {
        ui.label(line);
    }
}

/// The popup body: every affected track, then the level each metric warns at.
/// Returns the document whose link was clicked.
fn details_ui(ui: &mut Ui, indicator: SpaceWeatherIndicator<'_>) -> Option<ReferenceDocument> {
    ui.set_max_width(LEVELS_WIDTH_PX);
    ui.label(RichText::new(AFFECTED_TRACKS_TITLE).strong());
    if indicator.track_warnings.is_empty() {
        ui.label(RichText::new(NO_AFFECTED_TRACKS).weak());
    } else {
        // A floating bar shows only under the pointer. A fixed one keeps the
        // scrollbar in view, which is what marks the list as scrollable.
        ui.spacing_mut().scroll.floating = false;
        ScrollArea::vertical()
            .max_height(AFFECTED_TRACKS_MAX_HEIGHT_PX)
            .scroll_bar_visibility(ScrollBarVisibility::AlwaysVisible)
            .show(ui, |ui| {
                for (position, warning) in indicator.track_warnings.iter().enumerate() {
                    if position > 0 {
                        ui.add_space(TRACK_SPACING_PX);
                    }
                    track_warning_ui(ui, warning);
                }
            });
        tec_deviation_caveat_ui(ui, indicator.tec_deviation_caveat, indicator.track_warnings);
    }
    ui.add_space(LEVEL_ROW_SPACING_PX);
    ui.separator();
    levels_ui(ui, indicator.levels)
}

/// One row per metric, each stating its level over the link to the material
/// behind it. Returns the document whose link was clicked.
fn levels_ui(ui: &mut Ui, levels: &[WarningLevelExplanation]) -> Option<ReferenceDocument> {
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

    /// One disturbed track, named the way the application names a track of a
    /// recording that split into several.
    fn one_warning() -> Vec<TrackSpaceWeatherWarning> {
        vec![TrackSpaceWeatherWarning {
            track_label: "morning.gtd (track 2)".to_owned(),
            lines: vec!["Geomagnetic storm (≥5): Hp30 7.667, G3".to_owned()],
            states_tec_deviation: false,
        }]
    }

    /// One track whose lines state a TEC deviation, which is what the caveat
    /// closes a surface for.
    fn one_tec_deviation_warning() -> Vec<TrackSpaceWeatherWarning> {
        vec![TrackSpaceWeatherWarning {
            track_label: "morning.gtd (track 2)".to_owned(),
            lines: vec![
                "ΔTEC (<-30%): -51% from the 27-day median, intense ionospheric storm \
                 (W = -4), 8h, no geomagnetic storm in the 48h before"
                    .to_owned(),
            ],
            states_tec_deviation: true,
        }]
    }

    /// `count` disturbed tracks, each with its own peak, as several loaded
    /// recordings produce.
    fn warnings(count: usize) -> Vec<TrackSpaceWeatherWarning> {
        (0..count)
            .map(|index| TrackSpaceWeatherWarning {
                track_label: format!("ride-{index}.gtd"),
                lines: vec![format!(
                    "Aircraft interference (≥2%): up to {}.0% of aircraft in a crossed cell",
                    index + 3
                )],
                states_tec_deviation: false,
            })
            .collect()
    }

    /// Two rows, so the popup is exercised against a list. The application
    /// supplies the wording it lists.
    fn levels() -> Vec<WarningLevelExplanation> {
        vec![
            WarningLevelExplanation {
                trigger: "Aircraft interference: ≥2% of aircraft in a crossed cell.".to_owned(),
                reference: gt_jam::reference::AIRCRAFT_INTERFERENCE,
            },
            WarningLevelExplanation {
                trigger: gt_ionex::text::DEVIATION_WARNING_TRIGGER.clone(),
                reference: gt_ionex::reference::IONOSPHERIC_TEC,
            },
        ]
    }

    fn harness(
        track_warnings: Vec<TrackSpaceWeatherWarning>,
    ) -> gt_test_utils::TestHarness<'static, IndicatorState> {
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
                            track_warnings: &track_warnings,
                            levels: &levels,
                            tec_deviation_caveat: &gt_ionex::text::DEVIATION_REFERENCE_CAVEAT,
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

    fn popup_is_open(harness: &gt_test_utils::TestHarness<'_, IndicatorState>) -> bool {
        harness.inner.query_by_label(LEVELS_TITLE).is_some()
    }

    fn hover_the_glyph(harness: &mut gt_test_utils::TestHarness<'_, IndicatorState>) {
        harness
            .inner
            .hover_and_settle(By::new().label(ICON_CLOUD_LIGHTNING), TOOLTIP_SETTLE_FRAMES);
    }

    /// The glyph is on the map either way. What a metric reaching its
    /// disturbance level changes is the hover behind it, which names the
    /// affected track over the values it reached.
    #[rstest]
    #[case::warned(one_warning(), "morning.gtd (track 2)")]
    #[case::warned_values(one_warning(), "Geomagnetic storm (≥5): Hp30 7.667, G3")]
    #[case::idle(Vec::new(), IDLE_HOVER_TEXT)]
    fn the_hover_names_the_affected_tracks_or_explains_the_idle_glyph(
        #[case] track_warnings: Vec<TrackSpaceWeatherWarning>,
        #[case] expected_hover: &str,
    ) {
        let mut harness = harness(track_warnings);

        assert!(harness.inner.query_by_label(ICON_CLOUD_LIGHTNING).is_some());
        hover_the_glyph(&mut harness);

        assert!(
            harness
                .inner
                .query_by_label_contains(expected_hover)
                .is_some()
        );
    }

    /// The caveat closes a surface that states a TEC deviation, and stays off
    /// one where no listed track does. The hover and the popup both show it.
    #[rstest]
    #[case::hovered_with_a_deviation(one_tec_deviation_warning(), true, true)]
    #[case::hovered_without_one(one_warning(), true, false)]
    #[case::in_the_popup(one_tec_deviation_warning(), false, true)]
    fn the_tec_caveat_closes_a_surface_that_states_a_deviation(
        #[case] track_warnings: Vec<TrackSpaceWeatherWarning>,
        #[case] hovered: bool,
        #[case] expected: bool,
    ) {
        let mut harness = harness(track_warnings);

        if hovered {
            hover_the_glyph(&mut harness);
        } else {
            click_the_glyph(&mut harness);
        }

        assert_eq!(
            harness
                .inner
                .query_by_label_contains(&gt_ionex::text::DEVIATION_REFERENCE_CAVEAT)
                .is_some(),
            expected
        );
    }

    /// A hover cannot be scrolled or clicked, so it names only the first few
    /// tracks and counts the rest, and says where all of them are.
    #[test]
    fn the_hover_counts_the_tracks_it_leaves_out() {
        let left_out = 3;
        let mut harness = harness(warnings(MAX_HOVER_TRACKS + left_out));

        hover_the_glyph(&mut harness);

        assert!(
            harness
                .inner
                .query_by_label_contains(&format!("and {left_out} more tracks"))
                .is_some()
        );
        assert!(
            harness
                .inner
                .query_by_label_contains(CLICK_FOR_EVERY_TRACK)
                .is_some()
        );
    }

    /// The popup lists every affected track, including the ones the hover
    /// left out.
    #[test]
    fn the_popup_lists_every_affected_track() {
        let listed = warnings(MAX_HOVER_TRACKS + 2);
        let mut harness = harness(listed.clone());

        click_the_glyph(&mut harness);

        for warning in listed {
            assert!(
                harness
                    .inner
                    .query_by_label_contains(&warning.track_label)
                    .is_some(),
                "the popup never names {:?}",
                warning.track_label
            );
        }
    }

    /// The glyph requests the pointing-hand cursor in both states.
    #[rstest]
    #[case::warned(one_warning())]
    #[case::idle(Vec::new())]
    fn the_glyph_requests_the_pointing_hand(#[case] track_warnings: Vec<TrackSpaceWeatherWarning>) {
        let mut harness = harness(track_warnings);
        let glyph = harness.inner.get_by_label(ICON_CLOUD_LIGHTNING).rect();

        harness.inner.hover_at_and_settle(glyph.center(), 2);

        assert_eq!(
            harness.inner.output().platform_output.cursor_icon,
            CursorIcon::PointingHand
        );
    }

    /// The same levels are listed whether or not a metric warns: the list
    /// above them states what happened, and stays in place stating that
    /// nothing did.
    #[rstest]
    #[case::warned(one_warning())]
    #[case::idle(Vec::new())]
    fn clicking_the_glyph_lists_every_level(#[case] track_warnings: Vec<TrackSpaceWeatherWarning>) {
        let warned = !track_warnings.is_empty();
        let mut harness = harness(track_warnings);

        click_the_glyph(&mut harness);

        assert!(popup_is_open(&harness));
        assert!(
            harness
                .inner
                .query_by_label(AFFECTED_TRACKS_TITLE)
                .is_some()
        );
        assert_eq!(
            harness.inner.query_by_label(NO_AFFECTED_TRACKS).is_some(),
            !warned
        );
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
        assert!(popup_is_open(&harness));

        dismiss(&mut harness);

        assert!(!popup_is_open(&harness));
    }

    /// Clicking the glyph again closes the popup it opened.
    #[test]
    fn clicking_the_glyph_again_closes_the_popup() {
        let mut harness = harness(one_warning());

        click_the_glyph(&mut harness);
        click_the_glyph(&mut harness);

        assert!(!popup_is_open(&harness));
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
        assert!(!popup_is_open(&harness));
    }
}
