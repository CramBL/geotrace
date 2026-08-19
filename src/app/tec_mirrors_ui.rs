//! The mirror list editor of the settings dialog's "Ionospheric TEC" section.
//!
//! One row per mirror, in the fetch order, with the controls to reorder,
//! remove and add. A row states the archive layout its host serves, and a row
//! serving one that needs the Earthdata token is badged while no token is set.

use egui::{Button, RichText, TextEdit, Ui};
use egui_phosphor::regular::{
    ARROW_DOWN as ICON_ARROW_DOWN, ARROW_UP as ICON_ARROW_UP, PLUS as ICON_PLUS,
    TRASH as ICON_TRASH, WARNING as ICON_WARNING,
};
use gt_ionex::{Mirror, MirrorBaseUrl, MirrorLayout, MirrorList};
use strum::IntoEnumIterator as _;

const URL_HOVER: &str = "Base URL of a host serving the global ionosphere maps. The layout beside \
                         the field says which archive's directories and file names are expected \
                         under it. Requests contain a date and nothing about your recordings.";

const LAYOUT_HOVER: &str = "Which archive's directories and file names this host serves. Pick it \
                            when adding the mirror.";

const MIRROR_FIELD_WIDTH: f32 = 208.0;

/// Width the layout name is given, so the fields of rows serving different
/// ones line up under each other.
const LAYOUT_COLUMN_WIDTH: f32 = 44.0;

/// Width kept for the badge on every row, so a badged row's buttons sit where
/// the rest of them do.
const BADGE_COLUMN_WIDTH: f32 = 18.0;

/// Whether the Earthdata token setting holds a token, which decides whether
/// the mirrors needing one are fetched from at all.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EarthdataToken {
    Set,
    Missing,
}

enum MirrorEdit {
    Add(MirrorLayout),
    Replace(usize, Mirror),
    Remove(usize),
    MoveUp(usize),
    MoveDown(usize),
}

/// The editable mirror list. Returns `true` when the list changed.
pub fn show_mirror_list(
    ui: &mut Ui,
    mirrors: &mut MirrorList,
    earthdata_token: EarthdataToken,
) -> bool {
    let mut edit = None;
    ui.vertical(|ui| {
        let last = mirrors.as_slice().len().saturating_sub(1);
        for (index, mirror) in mirrors.as_slice().iter().enumerate() {
            ui.horizontal(|ui| {
                ui.scope(|ui| {
                    ui.set_min_width(LAYOUT_COLUMN_WIDTH);
                    ui.label(mirror.layout.to_string())
                        .on_hover_text(LAYOUT_HOVER);
                });
                let mut url = mirror.base_url.to_string();
                if ui
                    .add(TextEdit::singleline(&mut url).desired_width(MIRROR_FIELD_WIDTH))
                    .on_hover_text(URL_HOVER)
                    .changed()
                {
                    edit = Some(MirrorEdit::Replace(
                        index,
                        Mirror::new(MirrorBaseUrl::new(url), mirror.layout),
                    ));
                }
                ui.scope(|ui| {
                    ui.set_min_width(BADGE_COLUMN_WIDTH);
                    if mirror.layout.needs_earthdata_token()
                        && earthdata_token == EarthdataToken::Missing
                    {
                        ui.label(
                            RichText::new(ICON_WARNING)
                                .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)),
                        )
                        .on_hover_text(gt_ionex::text::MISSING_EARTHDATA_TOKEN.as_str());
                    }
                });
                if ui
                    .add_enabled(index > 0, Button::new(ICON_ARROW_UP).small())
                    .on_hover_text("Move this mirror one place earlier in the fetch order")
                    .on_disabled_hover_text("This mirror is already first in the fetch order")
                    .clicked()
                {
                    edit = Some(MirrorEdit::MoveUp(index));
                }
                if ui
                    .add_enabled(index < last, Button::new(ICON_ARROW_DOWN).small())
                    .on_hover_text("Move this mirror one place later in the fetch order")
                    .on_disabled_hover_text("This mirror is already last in the fetch order")
                    .clicked()
                {
                    edit = Some(MirrorEdit::MoveDown(index));
                }
                if ui
                    .add_enabled(last > 0, Button::new(ICON_TRASH).small())
                    .on_hover_text("Remove this mirror")
                    .on_disabled_hover_text("Add another mirror before removing this one")
                    .clicked()
                {
                    edit = Some(MirrorEdit::Remove(index));
                }
            });
        }
        ui.horizontal(|ui| {
            for layout in MirrorLayout::iter() {
                if ui
                    .button(format!("{ICON_PLUS} Add {layout} mirror"))
                    .on_hover_text(format!(
                        "Add the host publishing the {layout} archive, tried after the ones above"
                    ))
                    .clicked()
                {
                    edit = Some(MirrorEdit::Add(layout));
                }
            }
        });
    });

    match edit {
        Some(MirrorEdit::Add(layout)) => mirrors.add(Mirror::publishing(layout)),
        Some(MirrorEdit::Replace(index, mirror)) => mirrors.replace(index, mirror),
        Some(MirrorEdit::Remove(index)) => {
            mirrors.remove(index);
        }
        Some(MirrorEdit::MoveUp(index)) => mirrors.move_up(index),
        Some(MirrorEdit::MoveDown(index)) => mirrors.move_down(index),
        None => return false,
    }
    true
}

#[cfg(test)]
mod tests {
    use egui::accesskit::Role;
    use egui_kittest::Node;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    use gt_test_utils::{By, HarnessInteraction as _, TestHarness};
    use rstest::rstest;

    use super::*;

    /// The list under edit, and whether the editor reported a change on any
    /// frame it was run for.
    struct EditorState {
        mirrors: MirrorList,
        earthdata_token: EarthdataToken,
        changed: bool,
    }

    fn jpl_mirrors(hosts: &[&str]) -> MirrorList {
        MirrorList::new(
            hosts
                .iter()
                .map(|host| Mirror::new(MirrorBaseUrl::new(*host), MirrorLayout::Jpl))
                .collect(),
        )
        .expect("a named host")
    }

    fn hosts(state: &EditorState) -> Vec<String> {
        state
            .mirrors
            .as_slice()
            .iter()
            .map(|mirror| mirror.base_url.to_string())
            .collect()
    }

    fn editor(mirrors: MirrorList) -> TestHarness<'static, EditorState> {
        editor_with_token(mirrors, EarthdataToken::Set)
    }

    fn editor_with_token(
        mirrors: MirrorList,
        earthdata_token: EarthdataToken,
    ) -> TestHarness<'static, EditorState> {
        let mut harness = TestHarness::builder().ui_state(
            |ui, state: &mut EditorState| {
                state.changed |= show_mirror_list(ui, &mut state.mirrors, state.earthdata_token);
            },
            EditorState {
                mirrors,
                earthdata_token,
                changed: false,
            },
        );
        harness.run();
        harness
    }

    /// Click the button at `position` among the ones labeled `label`, and
    /// return the mirrors the click left behind.
    fn after_clicking(mirrors: MirrorList, label: &str, position: usize) -> Vec<String> {
        let mut harness = editor(mirrors);
        harness
            .inner
            .nth_matching(By::new().label(label), position)
            .click();
        harness.run();
        assert!(harness.state().changed, "the click changed the list");
        hosts(harness.state())
    }

    /// The row of `url`, which is the field holding it.
    fn row<'tree>(harness: &'tree TestHarness<'_, EditorState>, url: &'tree str) -> Node<'tree> {
        harness.inner.get_by(move |node| {
            node.role() == Role::TextInput && node.value().is_some_and(|value| value == url)
        })
    }

    #[test]
    fn every_mirror_is_listed_in_the_fetch_order() {
        let harness = editor(jpl_mirrors(&[
            "https://first.example",
            "https://second.example",
        ]));

        assert_eq!(
            harness
                .inner
                .get_all_by(|node| node.role() == Role::TextInput)
                .filter_map(|node| node.accesskit_node().value())
                .collect::<Vec<_>>(),
            ["https://first.example", "https://second.example"]
        );
    }

    /// A mirror states the archive its host serves, so a list holding both
    /// kinds reads as what each row is.
    #[test]
    fn every_mirror_states_the_layout_it_serves() {
        let harness = editor(MirrorList::default());

        for layout in MirrorLayout::iter() {
            assert_eq!(
                harness
                    .inner
                    .query_all_by_label(layout.to_string().as_str())
                    .count(),
                1,
                "{layout}"
            );
        }
    }

    /// Each layout is added from its own button, at the host that publishes
    /// it.
    #[rstest]
    #[case::the_publishing_archive(MirrorLayout::Jpl, gt_ionex::DEFAULT_BASE_URL)]
    #[case::the_authenticated_archive(MirrorLayout::Cddis, gt_ionex::cddis::DEFAULT_BASE_URL)]
    fn adding_a_mirror_appends_the_host_publishing_that_layout(
        #[case] layout: MirrorLayout,
        #[case] expected_host: &str,
    ) {
        let edited = after_clicking(
            jpl_mirrors(&["https://first.example"]),
            &format!("{ICON_PLUS} Add {layout} mirror"),
            0,
        );

        assert_eq!(edited, ["https://first.example", expected_host]);
    }

    /// A mirror needing a token is badged while none is set, and the badge is
    /// gone once one is: the row stays in the list either way, per DESIGN.md.
    #[rstest]
    #[case::without_a_token(EarthdataToken::Missing, 1)]
    #[case::with_a_token(EarthdataToken::Set, 0)]
    fn a_mirror_needing_a_token_is_badged_until_one_is_set(
        #[case] earthdata_token: EarthdataToken,
        #[case] expected_badges: usize,
    ) {
        let harness = editor_with_token(MirrorList::default(), earthdata_token);

        assert_eq!(
            harness.inner.query_all_by_label(ICON_WARNING).count(),
            expected_badges
        );
    }

    #[test]
    fn removing_a_mirror_leaves_the_rest_in_order() {
        let edited = after_clicking(
            jpl_mirrors(&["https://first.example", "https://second.example"]),
            ICON_TRASH,
            0,
        );

        assert_eq!(edited, ["https://second.example"]);
    }

    /// A mirror moves one place at a time, in either direction.
    #[rstest]
    #[case::up_from_the_second(ICON_ARROW_UP, 1)]
    #[case::down_from_the_first(ICON_ARROW_DOWN, 0)]
    fn moving_a_mirror_swaps_it_with_its_neighbor(#[case] label: &str, #[case] position: usize) {
        let edited = after_clicking(
            jpl_mirrors(&["https://first.example", "https://second.example"]),
            label,
            position,
        );

        assert_eq!(edited, ["https://second.example", "https://first.example"]);
    }

    /// The last mirror stays, so the fetch always has a host to try, and its
    /// remove button stays visible and grayed, per DESIGN.md.
    #[test]
    fn the_only_mirror_cannot_be_removed() {
        let mut harness = editor(jpl_mirrors(&["https://first.example"]));

        let remove = harness.inner.get_by_label(ICON_TRASH);
        assert!(remove.accesskit_node().is_disabled());
        remove.click();
        harness.run();

        assert!(!harness.state().changed);
        assert_eq!(hosts(harness.state()), ["https://first.example"]);
    }

    /// Typing in a row rewrites that mirror's host and leaves its place in the
    /// order and the layout it serves.
    #[test]
    fn an_edited_row_rewrites_its_host_and_keeps_its_layout() {
        let mut harness = editor(MirrorList::default());
        let cddis_host = gt_ionex::cddis::DEFAULT_BASE_URL;

        let field = row(&harness, cddis_host);
        field.focus();
        field.type_text("/copy");
        harness.run();

        assert_eq!(
            harness.state().mirrors.as_slice().get(1),
            Some(&Mirror::new(
                MirrorBaseUrl::new(format!("{cddis_host}/copy")),
                MirrorLayout::Cddis
            ))
        );
    }
}
