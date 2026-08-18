//! The Mapbox token editor, rendered by the map's token dialog and by the
//! settings window's Interface page.

use egui::TextEdit;
use gt_map::NavMap;

pub const TOKEN_LABEL: &str = "Mapbox token";

const TOKEN_FIELD_WIDTH: f32 = 260.0;

/// When the field hands its text to the map.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum MapboxTokenCommit {
    /// Enter, or leaving the field: what the settings row applies an edit on,
    /// like every other control on the page.
    OnEnterOrFocusLoss,
    /// Enter alone. The dialog's Cancel and its title-bar close take focus off
    /// the field, and must leave the map's token as it was.
    OnEnter,
}

/// The token as it is being typed. An unfocused field shows the token the map
/// holds.
#[derive(Default)]
pub struct MapboxTokenField {
    text: String,
}

impl MapboxTokenField {
    pub fn show(&mut self, ui: &mut egui::Ui, map: &mut NavMap, commit: MapboxTokenCommit) {
        let id = ui.id().with("mapbox_token_field");
        if !ui.memory(|memory| memory.has_focus(id)) && self.text != map.mapbox_token() {
            self.text = map.mapbox_token().to_owned();
        }
        let response = ui.add(
            TextEdit::singleline(&mut self.text)
                .id(id)
                .desired_width(TOKEN_FIELD_WIDTH),
        );
        if !response.lost_focus() {
            return;
        }
        let entered = ui.input(|input| input.key_pressed(egui::Key::Enter));
        if commit == MapboxTokenCommit::OnEnterOrFocusLoss || entered {
            self.commit(map);
        }
    }

    /// Hands the entered text to the map, which clears the token when the text
    /// is empty.
    pub fn commit(&self, map: &mut NavMap) {
        if self.text != map.mapbox_token() {
            map.set_mapbox_token(self.text.clone());
        }
    }

    pub fn text(&self) -> &str {
        &self.text
    }
}

#[cfg(test)]
mod tests {
    use egui::accesskit::Role;
    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    use gt_map::TileAccess;
    use gt_test_utils::{HarnessInteraction as _, TestHarness};
    use rstest::rstest;

    use super::*;

    /// The field beside another widget to click, which is how both call sites
    /// render it and how focus leaves it without an Enter.
    struct EditorState {
        field: MapboxTokenField,
        map: NavMap,
        commit: MapboxTokenCommit,
    }

    const ELSEWHERE: &str = "Elsewhere";

    fn editor(token: &str, commit: MapboxTokenCommit) -> TestHarness<'static, EditorState> {
        let mut map = NavMap::new(egui::Context::default(), TileAccess::Offline);
        map.set_mapbox_token(token.to_owned());
        let mut harness = TestHarness::builder().ui_state(
            |ui, state: &mut EditorState| {
                state.field.show(ui, &mut state.map, state.commit);
                let _elsewhere = ui.button(ELSEWHERE);
            },
            EditorState {
                field: MapboxTokenField::default(),
                map,
                commit,
            },
        );
        harness.run();
        harness
    }

    fn field_text(harness: &TestHarness<'_, EditorState>) -> String {
        harness
            .inner
            .get_by(|node| node.role() == Role::TextInput)
            .accesskit_node()
            .value()
            .unwrap_or_default()
    }

    #[test]
    fn an_unfocused_field_shows_the_token_the_map_holds() {
        let mut harness = editor("stored", MapboxTokenCommit::OnEnterOrFocusLoss);
        assert_eq!(field_text(&harness), "stored");

        harness
            .state_mut()
            .map
            .set_mapbox_token("replaced".to_owned());
        harness.run();

        assert_eq!(field_text(&harness), "replaced");
    }

    #[test]
    fn the_entered_token_reaches_the_map_on_enter() {
        let mut harness = editor("", MapboxTokenCommit::OnEnter);
        harness.inner.type_into_text_input("entered");
        assert_eq!(
            harness.state().map.mapbox_token(),
            "",
            "the map keeps its token while the field is being typed in"
        );

        harness.inner.key_press(egui::Key::Enter);
        harness.run();

        assert_eq!(harness.state().map.mapbox_token(), "entered");
    }

    /// Clicking away from the field applies the edit only where the call site
    /// asked for it.
    #[rstest]
    #[case::on_focus_loss(MapboxTokenCommit::OnEnterOrFocusLoss, "entered")]
    #[case::on_enter_alone(MapboxTokenCommit::OnEnter, "")]
    fn leaving_the_field_applies_the_edit_under_the_call_site_rule(
        #[case] commit: MapboxTokenCommit,
        #[case] expected: &str,
    ) {
        let mut harness = editor("", commit);
        harness.inner.type_into_text_input("entered");

        harness.inner.get_by_label(ELSEWHERE).click();
        harness.run();

        assert_eq!(harness.state().map.mapbox_token(), expected);
    }
}
