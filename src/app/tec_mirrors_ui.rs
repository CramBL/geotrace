//! The mirror list editor of the settings dialog's "Ionospheric TEC" section.
//!
//! One row per mirror, in the fetch order, with the controls to reorder,
//! remove and add.

use egui::{Button, TextEdit, Ui};
use egui_phosphor::regular::{
    ARROW_DOWN as ICON_ARROW_DOWN, ARROW_UP as ICON_ARROW_UP, PLUS as ICON_PLUS,
    TRASH as ICON_TRASH,
};
use gt_ionex::{MirrorBaseUrl, MirrorList};

const URL_HOVER: &str = "Base URL of a host serving the global ionosphere maps under JPL's \
                         directory layout. Requests contain a date and nothing about your \
                         recordings.";

const MIRROR_FIELD_WIDTH: f32 = 260.0;

enum MirrorEdit {
    Add,
    Replace(usize, MirrorBaseUrl),
    Remove(usize),
    MoveUp(usize),
    MoveDown(usize),
}

/// The editable mirror list. Returns `true` when the list changed.
pub fn show_mirror_list(ui: &mut Ui, mirrors: &mut MirrorList) -> bool {
    let mut edit = None;
    ui.vertical(|ui| {
        let last = mirrors.as_slice().len().saturating_sub(1);
        for (index, mirror) in mirrors.as_slice().iter().enumerate() {
            ui.horizontal(|ui| {
                let mut url = mirror.to_string();
                if ui
                    .add(TextEdit::singleline(&mut url).desired_width(MIRROR_FIELD_WIDTH))
                    .on_hover_text(URL_HOVER)
                    .changed()
                {
                    edit = Some(MirrorEdit::Replace(index, MirrorBaseUrl::new(url)));
                }
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
        if ui
            .button(format!("{ICON_PLUS} Add mirror"))
            .on_hover_text("Add the publishing host as another mirror, tried after the ones above")
            .clicked()
        {
            edit = Some(MirrorEdit::Add);
        }
    });

    match edit {
        Some(MirrorEdit::Add) => mirrors.add(MirrorBaseUrl::new(gt_ionex::DEFAULT_BASE_URL)),
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
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    use egui_kittest::{Harness, Node};
    use rstest::rstest;

    use super::*;

    /// The list under edit, and whether the editor reported a change on any
    /// frame it was run for.
    struct EditorState {
        mirrors: MirrorList,
        changed: bool,
    }

    fn mirrors(hosts: &[&str]) -> MirrorList {
        MirrorList::new(hosts.iter().copied().map(MirrorBaseUrl::new).collect())
            .expect("a named host")
    }

    fn hosts(state: &EditorState) -> Vec<String> {
        state
            .mirrors
            .as_slice()
            .iter()
            .map(MirrorBaseUrl::to_string)
            .collect()
    }

    fn editor(mirrors: MirrorList) -> Harness<'static, EditorState> {
        let mut harness = Harness::new_ui_state(
            |ui, state: &mut EditorState| {
                state.changed |= show_mirror_list(ui, &mut state.mirrors);
            },
            EditorState {
                mirrors,
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
            .get_all_by_label(label)
            .nth(position)
            .expect("the button")
            .click();
        harness.run();
        assert!(harness.state().changed, "the click changed the list");
        hosts(harness.state())
    }

    /// The row of `url`, which is the field holding it.
    fn row<'tree>(harness: &'tree Harness<'_, EditorState>, url: &'tree str) -> Node<'tree> {
        harness.get_by(move |node| {
            node.role() == Role::TextInput && node.value().is_some_and(|value| value == url)
        })
    }

    #[test]
    fn every_mirror_is_listed_in_the_fetch_order() {
        let harness = editor(mirrors(&[
            "https://first.example",
            "https://second.example",
        ]));

        assert_eq!(
            harness
                .get_all_by(|node| node.role() == Role::TextInput)
                .filter_map(|node| node.accesskit_node().value())
                .collect::<Vec<_>>(),
            ["https://first.example", "https://second.example"]
        );
    }

    #[test]
    fn adding_a_mirror_appends_the_publishing_host() {
        let edited = after_clicking(
            mirrors(&["https://first.example"]),
            &format!("{ICON_PLUS} Add mirror"),
            0,
        );

        assert_eq!(
            edited,
            ["https://first.example", gt_ionex::DEFAULT_BASE_URL]
        );
    }

    #[test]
    fn removing_a_mirror_leaves_the_rest_in_order() {
        let edited = after_clicking(
            mirrors(&["https://first.example", "https://second.example"]),
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
            mirrors(&["https://first.example", "https://second.example"]),
            label,
            position,
        );

        assert_eq!(edited, ["https://second.example", "https://first.example"]);
    }

    /// The last mirror stays, so the fetch always has a host to try, and its
    /// remove button stays visible and grayed, per DESIGN.md.
    #[test]
    fn the_only_mirror_cannot_be_removed() {
        let mut harness = editor(mirrors(&["https://first.example"]));

        let remove = harness.get_by_label(ICON_TRASH);
        assert!(remove.accesskit_node().is_disabled());
        remove.click();
        harness.run();

        assert!(!harness.state().changed);
        assert_eq!(hosts(harness.state()), ["https://first.example"]);
    }

    /// Typing in a row rewrites that mirror and leaves its place in the order.
    #[test]
    fn an_edited_row_rewrites_its_mirror() {
        let mut harness = editor(mirrors(&[
            "https://first.example",
            "https://second.example",
        ]));

        let field = row(&harness, "https://first.example");
        field.focus();
        field.type_text("/iono");
        harness.run();

        assert_eq!(
            hosts(harness.state()),
            ["https://first.example/iono", "https://second.example"]
        );
    }
}
