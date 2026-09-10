use std::sync::Arc;

use egui_kittest::{Harness, kittest::Queryable as _};
use egui_phosphor::regular::PUSH_PIN as ICON_PUSH_PIN;
use egui_phosphor::regular::TERMINAL_WINDOW as ICON_TERMINAL_WINDOW;
use egui_phosphor::regular::X as ICON_X;
use geotrace_sdk::ChannelUnit;
use gt_test_utils::TestHarness;
use rustc_hash::FxHashMap;

use crate::app::App;
use crate::app::test_util;
use crate::app::ui_tests;

/// "Reset filters" clears the query filter too, not just the global filter, so
/// the map fully returns to normal.
#[test]
fn reset_filters_clears_the_query_filter() {
    let mut harness = ui_tests::app_with_query_window_open();
    ui_tests::run_query(&mut harness, "points | where velocity > 1 km/h");
    assert!(
        harness.state().query_window.filter_active(),
        "the run produced an active query filter"
    );

    // Close the window so it can't overlap the side panel's reset button.
    harness.state_mut().query_window.open = false;
    harness.run_steps(2);
    harness.get_by_label_contains("Reset filters").click();
    harness.run_steps(3);

    assert!(
        harness.state().query_window.matches().is_none(),
        "Reset filters drops the query results"
    );
    assert!(!harness.state().query_window.filter_active());
}

/// Toggling pin flips the entry's pin and deleting removes it - the two
/// interactive history mutations, driven through the widgets.
#[test]
fn query_history_pin_and_delete_via_ui() {
    let mut harness = ui_tests::app_with_query_window_open();
    ui_tests::run_query(&mut harness, "points | where velocity > 1 km/h");

    harness.get_by_label("Query history").click();
    harness.run_steps(3);

    let revision_before = harness.state().query_window.history_revision();
    harness.get_by_label(ICON_PUSH_PIN).click();
    harness.run_steps(3);
    {
        let window = &harness.state().query_window;
        assert!(window.history()[0].pinned, "clicking pin pins the entry");
        assert!(
            window.history_revision() > revision_before,
            "pinning bumps the revision so settings flush"
        );
    }

    harness.get_by_label(ICON_X).click();
    harness.run_steps(3);
    assert!(
        harness.state().query_window.history().is_empty(),
        "clicking the delete button removes the entry"
    );
}

/// Clicking an example fills the editor with its text and does not run.
#[test]
fn query_example_loads_into_editor_without_running() {
    let mut harness = ui_tests::app_with_query_window_open();

    harness.get_by_label("Examples").click();
    harness.run_steps(3);
    harness.get_by_label("Weak fix").click();
    harness.run_steps(3);

    let window = &harness.state().query_window;
    assert_eq!(window.text(), "points\n| where sats_fix < 6");
    assert!(
        window.matches().is_none() && window.history().is_empty(),
        "loading an example only fills the editor - it never runs"
    );
}

/// Ctrl+Enter (Cmd+Enter) runs the current query, mirroring the Run button.
#[test]
fn query_ctrl_enter_runs() {
    let mut harness = ui_tests::app_with_query_window_open();
    harness
        .state_mut()
        .query_window
        .set_text("points | where velocity > 1 km/h".to_owned());
    harness.run_steps(3);

    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    harness.step();
    test_util::harness::step_until_query_result(&mut harness);
    harness.run_steps(3);

    assert!(
        harness.state().query_window.matches().is_some(),
        "Ctrl+Enter starts a run"
    );
    assert_eq!(
        harness.state().query_window.history().len(),
        1,
        "the Ctrl+Enter run is recorded in history"
    );
}

/// Open the query window with `text`, focus the editor with the caret at the
/// end, and step until the autocomplete popup has candidates.
fn editor_with_popup(text: &str) -> Harness<'static, App> {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    {
        let app = harness.state_mut();
        app.query_window.open = true;
        app.query_window.set_text(text.to_owned());
    }
    harness.run_steps(2);
    ui_tests::focus_query_editor_at_end(&harness, text);
    harness.run_steps(3);
    harness
}

/// Enter accepts the highlighted candidate, replacing the partial word. A
/// stage keyword gets a trailing space so the next token can be typed straight
/// away.
#[test]
fn autocomplete_enter_accepts_top_candidate() {
    let mut harness = editor_with_popup("points | wh");
    assert_eq!(
        harness.state().query_window.autocomplete_names(),
        vec!["where".to_owned(), "with".to_owned()]
    );

    harness
        .input_mut()
        .events
        .push(ui_tests::key_press(egui::Key::Enter));
    harness.run_steps(2);

    assert_eq!(harness.state().query_window.text(), "points | where ");
}

/// Arrow keys move the selection before Enter accepts it.
#[test]
fn autocomplete_arrow_down_then_enter_accepts_second() {
    let mut harness = editor_with_popup("points | wh");

    harness
        .input_mut()
        .events
        .push(ui_tests::key_press(egui::Key::ArrowDown));
    harness.run_steps(1);
    harness
        .input_mut()
        .events
        .push(ui_tests::key_press(egui::Key::Enter));
    harness.run_steps(2);

    assert_eq!(harness.state().query_window.text(), "points | with ");
}

/// Esc dismisses the popup without editing the text or closing the window, and
/// the popup stays closed until the text changes again.
#[test]
fn autocomplete_esc_dismisses_without_editing() {
    let mut harness = editor_with_popup("points | wh");

    harness
        .input_mut()
        .events
        .push(ui_tests::key_press(egui::Key::Escape));
    harness.run_steps(2);

    let window = &harness.state().query_window;
    assert!(
        window.autocomplete_names().is_empty(),
        "Esc closes the popup"
    );
    assert_eq!(
        window.text(),
        "points | wh",
        "Esc leaves the text unchanged"
    );
    assert!(window.open, "Esc dismisses the popup, not the window");
}

/// The blank line after a query stays quiet - no `points` popup before a
/// character is typed - and continuation typing (`| …`) is analyzed in the
/// context of the chunk above, not as a fresh query.
#[test]
fn no_popup_on_the_blank_line_after_a_query() {
    let mut harness = editor_with_popup("points\n");
    assert!(
        harness.state().query_window.autocomplete_names().is_empty(),
        "the empty line after a query must not pop `points`"
    );

    harness
        .input_mut()
        .events
        .push(egui::Event::Text("| d".to_owned()));
    harness.run_steps(3);
    let names = harness.state().query_window.autocomplete_names();
    assert!(
        names.iter().any(|n| n == "draw"),
        "continuation typing completes stage keywords in context: {names:?}"
    );
}

/// An eagerly opened empty-prefix popup (units after a number) is passive:
/// Enter still breaks the line.
#[test]
fn passive_unit_popup_lets_enter_break_the_line() {
    let mut harness = editor_with_popup("points | where velocity > 30");
    let names = harness.state().query_window.autocomplete_names();
    assert_eq!(
        names.first().map(String::as_str),
        Some("km/h"),
        "the unit popup is open on the empty prefix: {names:?}"
    );

    harness
        .input_mut()
        .events
        .push(ui_tests::key_press(egui::Key::Enter));
    harness.run_steps(2);
    assert_eq!(
        harness.state().query_window.text(),
        "points | where velocity > 30\n",
        "Enter breaks the line; the passive popup does not claim it"
    );
}

/// Tab accepts even a passive popup, and a unit accepted directly after a
/// digit gets its separating space.
#[test]
fn tab_accepts_a_passive_unit_with_a_separating_space() {
    let mut harness = editor_with_popup("points | where velocity > 30");
    harness
        .input_mut()
        .events
        .push(ui_tests::key_press(egui::Key::Tab));
    harness.run_steps(2);
    assert_eq!(
        harness.state().query_window.text(),
        "points | where velocity > 30 km/h"
    );
}

/// Accepting a function inserts its parentheses with the caret inside them.
#[test]
fn accepting_a_function_inserts_parentheses() {
    let mut harness = editor_with_popup("points | window 3 | where av");
    harness
        .input_mut()
        .events
        .push(ui_tests::key_press(egui::Key::Enter));
    harness.run_steps(2);
    assert_eq!(
        harness.state().query_window.text(),
        "points | window 3 | where avg()"
    );
    // Typing lands inside the parentheses.
    harness
        .input_mut()
        .events
        .push(egui::Event::Text("velocity".to_owned()));
    harness.run_steps(2);
    assert_eq!(
        harness.state().query_window.text(),
        "points | window 3 | where avg(velocity)"
    );
}

/// Ctrl+Space opens the popup on demand where the automatic path waits for a
/// typed character.
#[test]
fn ctrl_space_opens_the_popup_manually() {
    let mut harness = editor_with_popup("points | ");
    assert!(
        harness.state().query_window.autocomplete_names().is_empty(),
        "a stage position waits for the first character"
    );

    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Space,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    });
    harness.run_steps(3);
    let names = harness.state().query_window.autocomplete_names();
    assert!(
        names.iter().any(|n| n == "where"),
        "Ctrl+Space offers the stage keywords: {names:?}"
    );
}

/// With the editor focused, Esc first unfocuses it. Only a second Esc closes
/// the query window.
#[test]
fn esc_unfocuses_the_editor_before_closing_the_window() {
    let mut harness = editor_with_popup("points | draw");
    assert!(
        harness.state().query_window.autocomplete_names().is_empty(),
        "nothing completes after a display mode"
    );

    harness
        .input_mut()
        .events
        .push(ui_tests::key_press(egui::Key::Escape));
    harness.run_steps(2);
    assert!(
        harness.state().query_window.open,
        "the first Esc only unfocuses the editor"
    );

    harness
        .input_mut()
        .events
        .push(ui_tests::key_press(egui::Key::Escape));
    harness.run_steps(2);
    assert!(
        !harness.state().query_window.open,
        "the second Esc closes the window"
    );
}

/// A standalone comment paragraph between queries is skipped, so it neither
/// errors nor blocks running the real query.
#[test]
fn comment_only_chunk_does_not_block_run() {
    let mut harness = ui_tests::app_with_query_window_open();
    ui_tests::run_query(
        &mut harness,
        "# scratch note between queries\n\npoints | where velocity > 1 km/h",
    );
    assert!(
        harness.state().query_window.matches().is_some(),
        "the comment paragraph must not disable Run"
    );
}

/// A loaded file carrying one scalar channel (no points), for driving the `@`
/// completion path through the app: `schema_from_files` builds the schema the
/// popup offers from.
fn push_file_with_channel(harness: &mut Harness<App>, name: &str, unit: &str) {
    use gt_types::{Channel, FileSource, LoadedFile, LoadedTrack};
    let channel = Channel {
        name: name.to_owned(),
        unit: Some(ChannelUnit::from_file_label(unit)),
        period: None,
        description: None,
        components: vec![],
        times: vec![],
        values: vec![],
    };
    let file = LoadedFile {
        metadata: gt_test_utils::empty_file_metadata(),
        tracks: vec![LoadedTrack {
            channels: vec![channel],
            ..gt_test_utils::loaded_track_with_points(vec![])
        }],
        event_marker_styles: FxHashMap::default(),
        orphaned_event_markers: vec![],
        source: FileSource::GtdBytes(Arc::from(Vec::<u8>::new())),
        load_warnings: vec![],
    };
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .loaded_files
        .push(file, gt_loaded_files::FileHistory::None);
    harness.step();
}

/// The `@` channel popup, driven through the whole app path: the loaded file's
/// channel reaches the schema, the popup offers it, and accepting inserts the
/// `@name` reference.
#[test]
fn channel_popup_offers_and_inserts_a_loaded_channel() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    push_file_with_channel(&mut harness, "accel", "g");
    {
        let app = harness.state_mut();
        app.query_window.open = true;
        app.query_window.set_text("@ac".to_owned());
    }
    harness.run_steps(2);
    ui_tests::focus_query_editor_at_end(&harness, "@ac");
    harness.run_steps(3);

    assert_eq!(
        harness.state().query_window.autocomplete_names(),
        vec!["@accel".to_owned()],
        "the loaded channel is offered for the typed sigil"
    );
    harness
        .input_mut()
        .events
        .push(ui_tests::key_press(egui::Key::Enter));
    harness.run_steps(2);
    assert_eq!(harness.state().query_window.text(), "@accel");
}

/// A channel-source query mixed with a points query cannot run. The editor
/// says why.
#[test]
fn mixed_channel_queries_explain_why_run_is_disabled() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    push_file_with_channel(&mut harness, "accel", "g");
    {
        let app = harness.state_mut();
        app.query_window.open = true;
        app.query_window
            .set_text("points | where velocity > 1 km/h\n\n@accel | where @accel > 1 g".to_owned());
    }
    harness.run_steps(3);

    harness.get_by_label_contains("must be the only query in the editor");
}

/// Clicking a popup row accepts that candidate (the deferred click path, not
/// the keyboard path).
#[test]
fn autocomplete_click_accepts_candidate() {
    let mut harness = editor_with_popup("points | wh");
    // The "with" row is identified by its unique summary text.
    harness
        .get_by_label_contains("set satellite-analysis parameters")
        .click();
    harness.run_steps(2);
    assert_eq!(harness.state().query_window.text(), "points | with ");
}

/// Right-clicking the toolbar query button while a filter is active offers
/// "Clear query filter", which clears it.
#[test]
fn toolbar_context_menu_clears_query_filter() {
    let mut harness = ui_tests::app_with_query_window_open();
    ui_tests::run_query(&mut harness, "points | where velocity > 1 km/h");
    // Close the window so the toolbar shows the active-filter alert.
    harness.state_mut().query_window.open = false;
    harness.run_steps(3);
    assert!(harness.state().query_window.filter_active());

    harness
        .get_by_label_contains(ICON_TERMINAL_WINDOW)
        .click_secondary();
    harness.run_steps(2);
    harness.get_by_label_contains("Clear query filter").click();
    harness.run_steps(3);

    assert!(
        !harness.state().query_window.filter_active(),
        "the context menu cleared the query filter"
    );
}

/// The autocomplete popup: candidates under the caret, the top one
/// highlighted, capped to five rows with a footer noting the rest.
#[test]
fn snapshot_query_autocomplete_popup() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(560.0, 460.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    // A prefix that matches many metrics, so the popup overflows five rows.
    let text = "points | where s";
    {
        let app = harness.inner.state_mut();
        app.query_window.open = true;
        app.query_window.set_text(text.to_owned());
    }
    harness.inner.run_steps(3);
    ui_tests::focus_query_editor_at_end(&harness.inner, text);
    harness.inner.run_steps(3);

    let names = harness.inner.state().query_window.autocomplete_names();
    assert!(
        names.len() > 5,
        "the popup overflows five rows and shows a footer, got {names:?}"
    );
    assert!(
        names.iter().any(|n| n == "sats_fix"),
        "s-metrics are offered: {names:?}"
    );

    harness.snapshot_with_color_tolerance("query_autocomplete_popup");
}

/// A checker error: an error icon and the red problem, then the suggestion as
/// a plain "Hint:" line below.
#[test]
fn snapshot_query_error() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(560.0, 260.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    {
        let app = harness.inner.state_mut();
        app.query_window.open = true;
        // Per-point metric in a window: the message splits into problem + hint.
        app.query_window
            .set_text("points | window 10 | where velocity >= 10 km/h".to_owned());
    }
    // Editor left unfocused so no completion popup covers the error.
    harness.inner.run_steps(3);
    harness.snapshot_with_color_tolerance("query_error");
}

/// Hovering a construct in the editor shows a Rust-doc-style tooltip: name and
/// kind, summary, then the fuller explanation and an example.
#[test]
fn snapshot_query_hover_docs() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(560.0, 460.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();

    {
        let app = harness.inner.state_mut();
        app.query_window.open = true;
        app.query_window
            .set_text("points | window 10 | where avg(velocity) > 30 km/h".to_owned());
    }
    harness.inner.run_steps(3);

    // Hover the `window` token. It starts after "points | " on the first
    // line. The editor's text begins near the top-left of the window content.
    let editor = harness
        .inner
        .get_by_role(egui::accesskit::Role::MultilineTextInput);
    let rect = editor.rect();
    let hover = egui::pos2(rect.left() + 96.0, rect.top() + 10.0);
    harness.inner.run_steps(2);
    harness.inner.hover_at(hover);
    // The hover doc appears only after the pointer has rested, so step past
    // the delay (steps advance the mock clock a frame at a time).
    harness.inner.run_steps(40);

    harness.snapshot_with_color_tolerance("query_hover_docs");
}

/// The hover doc stays up while the pointer moves within its token, hides off
/// any token, and re-arms the rest delay before the next token's doc shows.
#[test]
fn query_hover_doc_sticks_within_its_token() {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(560.0, 460.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    {
        let app = harness.inner.state_mut();
        app.query_window.open = true;
        app.query_window
            .set_text("points | window 10 | where avg(velocity) > 30 km/h".to_owned());
    }
    harness.inner.run_steps(3);

    let editor = harness
        .inner
        .get_by_role(egui::accesskit::Role::MultilineTextInput);
    let rect = editor.rect();
    // Inside the `window` token, as in `snapshot_query_hover_docs`.
    let in_token = egui::pos2(rect.left() + 96.0, rect.top() + 10.0);
    harness.inner.hover_at(in_token);
    harness.inner.run_steps(40);
    assert!(
        harness.inner.state().query_window.hover_doc_shown(),
        "the doc shows once the pointer has rested on the token"
    );

    // Nudge one character to the right, still inside `window`. The frame that
    // processes the movement has a freshly reset rest timer, so only the
    // stickiness keeps the doc up.
    harness.inner.hover_at(in_token + egui::vec2(7.0, 0.0));
    harness.inner.run_steps(1);
    assert!(
        harness.inner.state().query_window.hover_doc_shown(),
        "the doc stays up while the pointer moves within its token"
    );

    // The blank editor space below the text is off any token.
    harness
        .inner
        .hover_at(egui::pos2(rect.left() + 96.0, rect.bottom() - 10.0));
    harness.inner.run_steps(1);
    assert!(
        !harness.inner.state().query_window.hover_doc_shown(),
        "the doc hides once the pointer leaves the token"
    );

    // Back on the token: the delay is armed again, then the doc returns.
    harness.inner.hover_at(in_token);
    harness.inner.run_steps(1);
    assert!(
        !harness.inner.state().query_window.hover_doc_shown(),
        "a token entered just now waits for the pointer to rest"
    );
    harness.inner.run_steps(40);
    assert!(
        harness.inner.state().query_window.hover_doc_shown(),
        "the doc returns after the pointer rests on the token again"
    );
}
