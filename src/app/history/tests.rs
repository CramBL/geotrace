use egui_kittest::kittest::{NodeT as _, Queryable as _};
use gt_pending_writes::{PendingWrites, WriteAccess};
use gt_store::{HistoryDatabase as _, RecordingsHandle};
use gt_test_utils::window_fit::{
    CRAMPED_VIEWPORT, NARROW_VIEWPORT, OVERSIZED_ROW_COUNT, SHORT_VIEWPORT,
};
use gt_test_utils::{
    AuditedWindow, By, ControlLabel, HarnessInteraction as _, TestHarness, WindowFitAssertions as _,
};

use crate::app::history_db::Response;
use crate::app::read_only_session::READ_ONLY_RECORDING_HISTORY_HOVER;
use crate::app::storage_controls::AUTO_STORE_LABEL;

use egui_phosphor::regular::NOTE as ICON_NOTE;
use gt_store::ChannelSummary;

use super::table::{
    MAX_HOVER_CHANNELS, breakdown_cell_id, channel_title, data_breakdown_ui, track_count_text,
};
use super::{
    DELETE_HIDDEN_WINDOW_TITLE, DatabaseRef, HistorySort, HistoryWindow, HistoryWorker,
    ICON_CARET_DOWN, ICON_CARET_UP, PRUNE_WINDOW_TITLE, RecordingEntry, RecordingMeta, SortColumn,
    SortDirection, identity_display_parts, travel_mode_display,
};
use strum::{EnumCount as _, IntoEnumIterator as _};

/// Harness state for driving the History window: the window, a live (empty)
/// worker so the list branch renders, and the settings toggles `show` needs.
struct HistoryHarness {
    window: HistoryWindow,
    worker: HistoryWorker,
    storage: crate::settings::StorageSettings,
    /// What the app reports while its startup open runs.
    databases_opening: bool,
    /// What the session may write, which is what grays the controls that do.
    write_access: WriteAccess,
    _dir: tempfile::TempDir,
}

fn history_harness(entries: Vec<RecordingEntry>) -> HistoryHarness {
    let dir = tempfile::tempdir().expect("temp dir");
    let db = gt_store::Recordings::open_or_create(&dir.path().join("history.h5")).expect("open db");
    let worker = HistoryWorker::spawn(
        RecordingsHandle::Owner(db),
        egui::Context::default(),
        PendingWrites::default(),
    );
    let mut window = HistoryWindow::new();
    window.open = true;
    // Populate directly so the list renders without a worker round-trip.
    window.set_entries(entries);
    HistoryHarness {
        window,
        worker,
        storage: crate::settings::StorageSettings {
            auto_prune_max_bytes: 0,
            ..crate::settings::StorageSettings::default()
        },
        databases_opening: false,
        write_access: WriteAccess::Owner,
        _dir: dir,
    }
}

fn show_history(ui: &mut egui::Ui, s: &mut HistoryHarness) {
    s.window.show(
        ui.ctx(),
        &s.worker,
        &[],
        &mut s.storage,
        s.databases_opening,
        s.write_access,
    );
}

/// A harness backed by a real database holding one recording, with no
/// pre-seeded entries - the list arrives from the worker (see [`pump_history`]).
fn history_harness_with_recording(identity: &str) -> HistoryHarness {
    use gt_store::{StoredSegmentation, TrackRange};

    let dir = tempfile::tempdir().expect("temp dir");
    let mut db =
        gt_store::Recordings::open_or_create(&dir.path().join("history.h5")).expect("open db");
    let bytes = gt_test_utils::GOLD_BYTES;
    let meta = gt_store::extract_meta(bytes).expect("meta");
    let tracks = [TrackRange {
        start: 0,
        end: meta.nav_point_count,
        hidden: false,
    }];
    let settings = StoredSegmentation {
        track_split_gap_us: 300_000_000,
        detect_clock_discontinuities: true,
        clock_discontinuity_sigmas: 5.0,
    };
    db.insert(identity, &meta, &tracks, settings, bytes)
        .expect("insert recording");
    let worker = HistoryWorker::spawn(
        RecordingsHandle::Owner(db),
        egui::Context::default(),
        PendingWrites::default(),
    );
    let mut window = HistoryWindow::new();
    window.open = true;
    HistoryHarness {
        window,
        worker,
        storage: crate::settings::StorageSettings {
            auto_prune_max_bytes: 0,
            ..crate::settings::StorageSettings::default()
        },
        databases_opening: false,
        write_access: WriteAccess::Owner,
        _dir: dir,
    }
}

/// Drive one frame like the app does: drain the worker's responses into the
/// window (list refresh, mutation acknowledgements) and then render it.
fn pump_history(ui: &mut egui::Ui, s: &mut HistoryHarness) {
    for resp in s.worker.poll() {
        match resp {
            Response::Listed(Ok(entries)) => s.window.set_entries(entries),
            Response::Mutated { result: Ok(()), .. } => s.window.invalidate(),
            _ => {}
        }
    }
    show_history(ui, s);
}

#[test]
fn rename_workflow_updates_the_listed_identity_end_to_end() {
    // Full workflow against a real worker + database: the row lists, the user
    // edits the identity inline, and after the async rename the list shows the
    // new name.
    let harness = history_harness_with_recording("auto:ride.gtd");
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(pump_history, harness);

    // The recording lists under its stripped identity.
    assert!(
        h.inner
            .step_until(|h| h.query_by_label_contains("ride.gtd").is_some()),
        "recording should appear in the History list"
    );

    // Open the inline editor through the identity's context menu.
    // `request_focus` applies the frame after the editor first renders,
    // so settle a couple of frames before typing.
    h.inner.get_by_label_contains("ride.gtd").click_secondary();
    h.step();
    h.inner.get_by_label("Rename").click_accesskit();
    h.step();
    h.step();
    assert!(
        h.inner.query_all_by_value("ride.gtd").next().is_some(),
        "probe: editor not open after Rename click"
    );

    // Append to the seeded name and commit with Enter.
    h.inner.event(egui::Event::Text(" v2".to_owned()));
    h.step();
    h.inner.key_press(egui::Key::Enter);
    h.step();

    // After the worker renames and the window re-lists, the new identity shows.
    assert!(
        h.inner
            .step_until(|h| h.query_by_label_contains("ride.gtd v2").is_some()),
        "the renamed identity should appear in the refreshed list"
    );
}

/// Never hidden, per DESIGN.md: in a read-only session the row's actions that
/// write are grayed and say the recording history is left as it is, while
/// opening a recording - which writes nothing - stays live.
#[test]
fn the_row_actions_that_write_are_grayed_in_a_read_only_session() {
    let mut harness = history_harness(vec![entry_with_identity("auto:ride.gtd")]);
    harness.write_access = WriteAccess::ReadOnly;
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    h.step();

    let delete = h.inner.get_by_label("Delete");
    assert!(delete.accesskit_node().is_disabled());
    let delete_center = delete.rect().center();
    assert!(
        !h.inner.get_by_label("Open").accesskit_node().is_disabled(),
        "opening a stored recording stays live: it writes nothing"
    );
    assert!(
        h.inner
            .get_by_label_contains("Prune…")
            .accesskit_node()
            .is_disabled()
    );

    h.inner.hover_at_and_settle(delete_center, 3);

    h.inner
        .get_by_label_contains(READ_ONLY_RECORDING_HISTORY_HOVER);
}

/// The rename a double click opens is a write too: a read-only session opens
/// no editor, and the context menu's Rename is grayed.
#[test]
fn no_rename_editor_opens_in_a_read_only_session() {
    let mut harness = history_harness(vec![entry_with_identity("auto:ride.gtd")]);
    harness.write_access = WriteAccess::ReadOnly;
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .step_dt(1.0 / 60.0)
        .ui_state(show_history, harness);
    h.run();

    h.inner.get_by_label_contains("ride.gtd").click();
    h.inner.get_by_label_contains("ride.gtd").click();
    h.run();
    h.inner.event(egui::Event::Text(" v2".to_owned()));
    h.step();
    h.step();

    assert!(
        h.inner.query_all_by_value("ride.gtd v2").next().is_none(),
        "the read-only session opened the rename editor and typing reached it"
    );
    h.inner.get_by_label_contains("ride.gtd").click_secondary();
    h.step();
    assert!(
        h.inner
            .get_by_label("Rename")
            .accesskit_node()
            .is_disabled(),
        "the read-only session offers a rename that would be rejected"
    );
}

/// The window opened during startup says the database is opening: it is not
/// unavailable, it is not open yet.
#[test]
fn the_window_reports_the_databases_still_opening() {
    let mut harness = history_harness(vec![entry_with_identity("auto:ride.gtd")]);
    harness.databases_opening = true;
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    // The spinner repaints every frame, so the harness is stepped.
    h.inner.run_steps(2);

    h.inner
        .get_by_label_contains(crate::app::history::OPENING_RECORDINGS_DATABASE);
    assert!(
        h.inner.query_by_label_contains("ride.gtd").is_none(),
        "the list waits for the database"
    );
}

/// The recordings table: identity takes the remaining width (long names
/// get the room), the value columns stay compact, headers carry the
/// resize handles.
#[test]
fn snapshot_history_window_table() {
    let mut harness = history_harness(vec![
        entry_with_identity("auto:ride.gtd"),
        entry_with_identity("a much longer recording identity that needs the room"),
        entry_with_identity("survey_flight_2026_07_15.gtd"),
    ]);
    // The temporary database path differs every run, so keep it out of the
    // image.
    harness.worker.hide_path();
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    // Auto columns measure their content over the first frames, so settle
    // before snapshotting.
    for _ in 0..4 {
        h.run();
    }
    h.snapshot("history_window_table");
}

#[test]
fn double_clicking_identity_opens_inline_editor() {
    let harness = history_harness(vec![entry_with_identity("auto:ride.gtd")]);
    // Frames at 60 fps: kittest's default 0.25 s/frame clock (one frame
    // per queued event) spaces the two clicks beyond egui's 0.3 s
    // double-click window.
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .step_dt(1.0 / 60.0)
        .ui_state(show_history, harness);
    h.run();
    // Two quick clicks on the identity label register as a double click
    // and swap the cell for the inline text editor (seeded with the
    // `auto:`-stripped name).
    h.inner.get_by_label_contains("ride.gtd").click();
    h.inner.get_by_label_contains("ride.gtd").click();
    h.run();
    assert!(
        h.inner.query_all_by_value("ride.gtd").next().is_some(),
        "inline editor should show the stripped identity as its value"
    );
    // The editor holds keyboard focus: typing extends its buffer.
    h.step();
    h.inner.event(egui::Event::Text(" v2".to_owned()));
    h.step();
    h.step();
    assert!(
        h.inner.query_all_by_value("ride.gtd v2").next().is_some(),
        "typed text should reach the freshly opened editor"
    );
}

/// A listing entry for `identity` with no tracks and no SDK metadata, for the
/// identity-cell layout tests.
fn entry_with_identity(identity: &str) -> RecordingEntry {
    RecordingEntry {
        db_ref: DatabaseRef {
            identity: identity.to_owned(),
            group_name: "rec0".to_owned(),
        },
        meta: RecordingMeta {
            start_us: 0,
            end_us: 0,
            nav_point_count: 0,
            sat_report_count: 0,
            marker_count: 0,
            event_marker_count: 0,
            gtd_size_bytes: 0,
        },
        total_tracks: 0,
        hidden_tracks: 0,
        title: None,
        device: None,
        notes: None,
        travel_mode: None,
        channels: Vec::new(),
    }
}

/// A listing entry with the four sortable value columns set, for the
/// ordering tests. `duration_us` is added to `start_us` to give the entry
/// its span.
fn sortable_entry(
    identity: &str,
    start_us: i64,
    duration_us: i64,
    nav_point_count: u64,
    gtd_size_bytes: u64,
) -> RecordingEntry {
    let mut entry = entry_with_identity(identity);
    entry.meta.start_us = start_us;
    entry.meta.end_us = start_us + duration_us;
    entry.meta.nav_point_count = nav_point_count;
    entry.meta.gtd_size_bytes = gtd_size_bytes;
    entry
}

/// Three entries whose columns disagree about the order, so sorting by any
/// one of them produces a different sequence: `beta` is the oldest but the
/// longest and biggest, `alpha` the newest but the shortest.
fn sortable_entries() -> Vec<RecordingEntry> {
    vec![
        sortable_entry("Alpha", 3_000, 10, 5, 50),
        sortable_entry("beta", 1_000, 300, 100, 5_000),
        sortable_entry("Gamma", 2_000, 60, 40, 400),
    ]
}

/// The identities the sort produces, in list order.
fn sorted_identities(sort: HistorySort, entries: &[RecordingEntry]) -> Vec<&str> {
    let mut visible: Vec<&RecordingEntry> = entries.iter().collect();
    sort.apply(&mut visible);
    visible.iter().map(|e| e.db_ref.identity.as_str()).collect()
}

/// Every column orders the list by its own value, in both directions.
/// Identity compares case-insensitively on the displayed name, so `beta`
/// sorts between `Alpha` and `Gamma` rather than after both.
#[rstest::rstest]
#[case(SortColumn::Identity, SortDirection::Ascending, ["Alpha", "beta", "Gamma"])]
#[case(SortColumn::Identity, SortDirection::Descending, ["Gamma", "beta", "Alpha"])]
#[case(SortColumn::Date, SortDirection::Ascending, ["beta", "Gamma", "Alpha"])]
#[case(SortColumn::Date, SortDirection::Descending, ["Alpha", "Gamma", "beta"])]
#[case(SortColumn::Duration, SortDirection::Ascending, ["Alpha", "Gamma", "beta"])]
#[case(SortColumn::Duration, SortDirection::Descending, ["beta", "Gamma", "Alpha"])]
#[case(SortColumn::Points, SortDirection::Ascending, ["Alpha", "Gamma", "beta"])]
#[case(SortColumn::Points, SortDirection::Descending, ["beta", "Gamma", "Alpha"])]
#[case(SortColumn::Size, SortDirection::Ascending, ["Alpha", "Gamma", "beta"])]
#[case(SortColumn::Size, SortDirection::Descending, ["beta", "Gamma", "Alpha"])]
fn sorting_orders_by_the_chosen_column(
    #[case] column: SortColumn,
    #[case] direction: SortDirection,
    #[case] expected: [&str; 3],
) {
    let entries = sortable_entries();
    let sort = HistorySort { column, direction };

    assert_eq!(sorted_identities(sort, &entries), expected.to_vec());
}

/// Entries that tie on the sorted column keep one stable order regardless of
/// direction, so equal rows do not shuffle when the sort is reversed.
#[test]
fn ties_break_stably_and_independently_of_direction() {
    // Same size, different identities - only the tie-break can separate them.
    let entries = vec![
        sortable_entry("charlie", 3_000, 10, 5, 100),
        sortable_entry("alpha", 1_000, 20, 9, 100),
        sortable_entry("bravo", 2_000, 30, 7, 100),
    ];
    let by_size = |direction| {
        sorted_identities(
            HistorySort {
                column: SortColumn::Size,
                direction,
            },
            &entries,
        )
    };

    assert_eq!(
        by_size(SortDirection::Ascending),
        ["alpha", "bravo", "charlie"]
    );
    assert_eq!(
        by_size(SortDirection::Descending),
        ["alpha", "bravo", "charlie"],
        "reversing the direction must not reshuffle rows that tie on the column",
    );
}

/// Clicking the active column reverses it. Clicking another switches to it
/// in that column's own natural direction.
#[test]
fn header_clicks_reverse_then_switch_columns() {
    let mut sort = HistorySort::default();
    assert_eq!(sort.column, SortColumn::Date);
    assert_eq!(sort.direction, SortDirection::Descending);

    sort.clicked(SortColumn::Date);
    assert_eq!(
        sort.direction,
        SortDirection::Ascending,
        "re-click reverses"
    );

    sort.clicked(SortColumn::Identity);
    assert_eq!(
        (sort.column, sort.direction),
        (SortColumn::Identity, SortDirection::Ascending),
        "identity starts A to Z",
    );

    sort.clicked(SortColumn::Size);
    assert_eq!(
        (sort.column, sort.direction),
        (SortColumn::Size, SortDirection::Descending),
        "size starts largest first, not carrying identity's ascending order",
    );
}

/// Every sortable column carries its own header title and a distinct hint
/// per direction, so no variant can be added without describing itself.
#[test]
fn every_sort_column_describes_itself() {
    let columns: Vec<SortColumn> = SortColumn::iter().collect();
    assert_eq!(
        columns.len(),
        SortColumn::COUNT,
        "the iterator must cover every variant",
    );

    let mut titles: Vec<&str> = columns.iter().map(|c| c.title()).collect();
    titles.sort_unstable();
    titles.dedup();
    assert_eq!(
        titles.len(),
        SortColumn::COUNT,
        "column titles must be unique"
    );

    for column in columns {
        assert_ne!(
            column.order_hint(SortDirection::Ascending),
            column.order_hint(SortDirection::Descending),
            "{column:?} must read differently in each direction",
        );
    }
}

/// The DB hands the listing the raw `meta_travel_mode` wire value. The
/// hover must show the human spelling for known modes and the preserved
/// wire value verbatim for unknown ones.
#[rstest::rstest]
#[case("bicycle", "Bicycle")]
#[case("hovercraft", "hovercraft")]
fn travel_mode_display_humanizes_the_wire_value(#[case] wire: &str, #[case] expected: &str) {
    assert_eq!(travel_mode_display(wire), expected);
}

/// A travel mode alone must badge the row with the note icon, proving
/// `identity_cell` feeds the field into the shared metadata presence check.
#[test]
fn travel_mode_alone_shows_the_metadata_note_icon() {
    let mut entry = entry_with_identity("auto:ride.gtd");
    entry.travel_mode = Some("bicycle".to_owned());
    let harness = history_harness(vec![entry]);
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    h.run();
    assert!(
        h.inner.query_by_label(ICON_NOTE).is_some(),
        "the note icon should appear for an entry whose only metadata is a travel mode"
    );
}

/// Settled width of the History window, through the real rendering path
/// ([`HistoryWindow::show`]). A resizable window runs a sizing pass over
/// its content, the path where an un-clipped column would report its
/// full text width and stretch the window.
fn history_window_width(identity: &str) -> f32 {
    let harness = history_harness(vec![entry_with_identity(identity)]);
    let mut h = TestHarness::builder()
        .size(egui::vec2(1600.0, 500.0))
        .ui_state(show_history, harness);
    h.inner
        .settled_window_size("History", 6)
        .expect("the History window is shown")
        .x
}

/// A long recording identity truncates in the History window rather than
/// stretching it: a short, a long, and a much longer identity all settle the
/// resizable window at the same width. Without the truncation the identity
/// column would size to its full text and the window would grow with it.
#[test]
fn long_identity_does_not_widen_history_window() {
    let short = history_window_width("auto:ride.gtd");
    let long = history_window_width(&"a/very/long/recording/identity/".repeat(4));
    let longer = history_window_width(&"a/very/long/recording/identity/".repeat(12));
    assert!(
        (long - short).abs() < 1.0 && (longer - short).abs() < 1.0,
        "identity length changed the history window width: \
         short={short}px long={long}px longer={longer}px",
    );
}

/// The metadata-width measurement is ignored during the table's sizing pass:
/// on the first frame the auto columns have not grown to their content, so
/// the reserve reads far too small and, if cached, would inflate identity and
/// stick the window permanently wide. A freshly opened window must therefore
/// settle to its content width, not a bloated one.
#[test]
fn fresh_window_settles_to_content_width_not_a_bloated_one() {
    // Room to bloat into: the screen is 1600px, the content needs well under
    // half that. A leaked sizing-pass measurement pushed this past 900px.
    let width = history_window_width("auto:ride.gtd");
    assert!(
        width < 750.0,
        "the History window settled far wider than its content ({width:.0}px); \
         the sizing-pass metadata measurement likely leaked into the identity fill",
    );
}

/// The identity filter field fills the toolbar space to the left of the
/// action controls and must yield as the window narrows, never growing into
/// them. Previously the field kept a fixed width and the "Auto-store
/// recordings" checkbox slid left underneath it, overlapping.
#[test]
fn filter_field_does_not_overlap_the_toolbar_controls() {
    let harness = history_harness(vec![entry_with_identity("auto:ride.gtd")]);
    let mut h = TestHarness::builder()
        .size(egui::vec2(1200.0, 500.0))
        .ui_state(show_history, harness);
    for _ in 0..8 {
        h.step();
    }
    // Shrink toward the window's minimum, where the overlap used to appear.
    let w = window_rect(&h);
    h.inner.press_drag_release(
        egui::pos2(w.right() - 1.0, w.bottom() - 1.0),
        egui::vec2(-500.0, 0.0),
        10,
    );
    for _ in 0..3 {
        h.step();
    }

    let checkbox_left = h.inner.get_by_label(AUTO_STORE_LABEL).rect().left();
    // The first text input in the window is the identity filter field.
    let filter_right = h
        .inner
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .map(|n| n.rect())
        .next()
        .expect("identity filter field")
        .right();
    assert!(
        filter_right <= checkbox_left + 1.0,
        "the identity filter field (right edge {filter_right:.0}px) overlaps the \
         Auto-store checkbox (left edge {checkbox_left:.0}px)",
    );
}

/// A History window sized to a wide screen, populated with long identities
/// (they clip in the identity column), settled so the auto columns have
/// measured their content.
fn resize_harness() -> TestHarness<'static, HistoryHarness> {
    let long = "a/very/long/recording/identity/that/needs/lots/of/room/".repeat(2);
    let harness = history_harness(vec![
        entry_with_identity(&long),
        entry_with_identity(&format!("{long}/2")),
        entry_with_identity(&format!("{long}/3")),
    ]);
    let mut h = TestHarness::builder()
        .size(egui::vec2(1400.0, 600.0))
        .ui_state(show_history, harness);
    // Settle the sizing pass and let the window finish auto-positioning.
    for _ in 0..10 {
        h.step();
    }
    h
}

/// The rightmost content (the Delete button) relative to the window's right
/// edge. Identity fills the leftover width, so this "gap" is only the
/// window's frame padding - at every window size.
fn content_gap_to_window_edge(h: &TestHarness<HistoryHarness>) -> f32 {
    let win = window_rect(h);
    let delete = h
        .inner
        .get_all_by_label("Delete")
        .last()
        .expect("delete button")
        .rect();
    win.right() - delete.right()
}

/// Identity fills the window at every size: the metadata columns keep their
/// content width and identity takes the rest. Growing or shrinking the
/// window leaves no gap on the right and traps no content off-screen - the
/// table is always exactly as wide as the window.
#[test]
fn identity_fills_the_window_at_every_size() {
    let mut h = resize_harness();
    let settled_gap = content_gap_to_window_edge(&h);

    // Grow the window from its bottom-right corner.
    let before = window_rect(&h);
    h.inner.press_drag_release(
        egui::pos2(before.right() - 1.0, before.bottom() - 1.0),
        egui::vec2(300.0, 0.0),
        8,
    );
    for _ in 0..3 {
        h.step();
    }
    assert!(
        window_rect(&h).width() > before.width() + 200.0,
        "the window did not grow: {:.0}px -> {:.0}px",
        before.width(),
        window_rect(&h).width(),
    );
    assert!(
        (content_gap_to_window_edge(&h) - settled_gap).abs() < 4.0,
        "growing the window left a gap on the right - identity did not fill it",
    );

    // Shrink it back down. egui clamps the drag at the content's minimum
    // width (measured by a sizing pass when the drag starts), so stay well
    // above that floor: the identity-fill invariant is what matters here.
    let grown = window_rect(&h);
    h.inner.press_drag_release(
        egui::pos2(grown.right() - 1.0, grown.bottom() - 1.0),
        egui::vec2(-80.0, 0.0),
        8,
    );
    for _ in 0..3 {
        h.step();
    }
    assert!(
        window_rect(&h).width() < grown.width() - 40.0,
        "the window did not shrink: {:.0}px -> {:.0}px",
        grown.width(),
        window_rect(&h).width(),
    );
    assert!(
        (content_gap_to_window_edge(&h) - settled_gap).abs() < 4.0,
        "shrinking the window left a gap on the right - identity did not fill it",
    );
}

fn window_rect(h: &TestHarness<HistoryHarness>) -> egui::Rect {
    h.inner
        .window_rect("History")
        .expect("the History window is shown")
}

/// The window can be dragged narrower than its settled width. Identity
/// yields as the window shrinks, so the table follows the window down
/// instead of pinning it at a content minimum that snaps it back to full
/// width (the old "can't shrink the window" bug).
#[test]
fn the_window_can_be_shrunk_narrower() {
    let mut h = resize_harness();
    let before = window_rect(&h);
    // Drag the bottom-right resize corner inward.
    let corner = egui::pos2(before.right() - 1.0, before.bottom() - 1.0);
    h.inner
        .press_drag_release(corner, egui::vec2(-200.0, 0.0), 8);
    for _ in 0..3 {
        h.step();
    }
    let after = window_rect(&h);
    assert!(
        after.width() < before.width() - 50.0,
        "the window did not shrink: {:.1}px -> {:.1}px",
        before.width(),
        after.width(),
    );
}

/// The identities the table currently lists, top to bottom, read off the
/// rendered row positions.
fn listed_order(h: &TestHarness<HistoryHarness>, identities: &[&str]) -> Vec<String> {
    let mut rows: Vec<(f32, String)> = identities
        .iter()
        .map(|identity| {
            let top = h.inner.get_by_label_contains(identity).rect().top();
            (top, (*identity).to_owned())
        })
        .collect();
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    rows.into_iter().map(|(_, identity)| identity).collect()
}

/// Click the table's header for `title`. The toolbar carries an "Identity"
/// label of its own, so match on the lowest node on screen - the header row
/// sits below the toolbar.
fn click_header(h: &TestHarness<HistoryHarness>, title: &str) {
    header_node(h, title).click();
}

/// The table header labelled exactly `title`.
///
/// Takes the lowest matching node: the toolbar and filter row carry labels
/// with the same words ("Identity", "Points") and sit above the table.
fn header_node<'t>(h: &'t TestHarness<HistoryHarness>, title: &'t str) -> egui_kittest::Node<'t> {
    h.inner.bottommost_matching(By::new().label(title))
}

/// Clicking a column header reorders the rendered table, and clicking the
/// same header again reverses it - the sort reaching the actual list, not
/// just the state struct.
#[test]
fn clicking_a_header_reorders_the_rendered_rows() {
    let harness = history_harness(sortable_entries());
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    for _ in 0..4 {
        h.run();
    }

    let identities = ["Alpha", "beta", "Gamma"];
    assert_eq!(
        listed_order(&h, &identities),
        ["Alpha", "Gamma", "beta"],
        "the default order is newest first",
    );

    // Sort by identity: a first click on a new column sorts it A to Z.
    click_header(&h, "Identity");
    h.run();
    assert_eq!(listed_order(&h, &identities), ["Alpha", "beta", "Gamma"]);

    // Clicking the active column reverses it.
    click_header(&h, "Identity");
    h.run();
    assert_eq!(listed_order(&h, &identities), ["Gamma", "beta", "Alpha"]);

    // Switching to Points sorts largest first.
    click_header(&h, "Points");
    h.run();
    assert_eq!(listed_order(&h, &identities), ["beta", "Gamma", "Alpha"]);
}

/// The active column is the only one showing a caret, and the caret follows
/// the direction - so the header always says how the list is ordered.
#[test]
fn only_the_active_column_shows_a_direction_caret() {
    let harness = history_harness(sortable_entries());
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    for _ in 0..4 {
        h.run();
    }

    // Default sort is Date descending: exactly one caret, pointing down.
    assert_eq!(h.inner.query_all_by_label(ICON_CARET_DOWN).count(), 1);
    assert_eq!(h.inner.query_all_by_label(ICON_CARET_UP).count(), 0);

    // Reversing it flips the caret without adding a second one.
    click_header(&h, "Date");
    h.run();
    assert_eq!(h.inner.query_all_by_label(ICON_CARET_DOWN).count(), 0);
    assert_eq!(h.inner.query_all_by_label(ICON_CARET_UP).count(), 1);
}

/// A recording carrying ad-hoc sensor channels: two of them, one vector and
/// one scalar, plus counts for every data kind the breakdown reports.
fn entry_with_channels() -> RecordingEntry {
    let mut entry = sortable_entry(
        "auto:sensors.gtd",
        1_700_000_000_000_000,
        3_600_000_000,
        8_940,
        4_096,
    );
    entry.meta.sat_report_count = 1_234;
    entry.meta.marker_count = 12;
    entry.meta.event_marker_count = 3;
    entry.total_tracks = 4;
    entry.channels = vec![
        ChannelSummary {
            name: "accel".to_owned(),
            unit: Some("g".to_owned()),
            description: Some("Frame IMU".to_owned()),
            components: vec!["x".to_owned(), "y".to_owned(), "z".to_owned()],
            sample_count: 12_000,
        },
        ChannelSummary {
            name: "temperature".to_owned(),
            unit: None,
            description: None,
            components: Vec::new(),
            sample_count: 512,
        },
    ];
    entry
}

/// Park the pointer on the widget labelled `label` and hold it there until
/// the hover turns into a tooltip.
fn hover_widget(h: &mut TestHarness<HistoryHarness>, label: &str) {
    let target = topmost_labelled(h, label);
    h.inner.hover_at_and_settle(target, 4);
}

/// Point at the widget labelled `label` and stop before its tooltip opens.
///
/// For reading the cursor a widget requests. A tooltip is its own layer and a
/// big one lands over the pointer, which takes the hover off the widget
/// underneath and resets the cursor - so the cursor has to be read while
/// the widget is still the thing being pointed at.
fn point_at_widget(h: &mut TestHarness<HistoryHarness>, label: &str) {
    let target = topmost_labelled(h, label);
    h.inner.hover_at_and_settle(target, 1);
}

/// Like [`point_at_widget`] for a table header (see [`header_node`]).
fn point_at_header(h: &mut TestHarness<HistoryHarness>, title: &str) {
    let target = header_node(h, title).rect().center();
    h.inner.hover_at_and_settle(target, 1);
}

/// Centre of the topmost widget whose label contains `label` - the table
/// row rather than the footer summary when both carry the same text.
fn topmost_labelled(h: &TestHarness<HistoryHarness>, label: &str) -> egui::Pos2 {
    h.inner
        .topmost_matching(By::new().label_contains(label))
        .rect()
        .center()
}

/// Snapshot the hover breakdown for `entry`, rendered through the same
/// function the tooltip calls.
///
/// Driven directly rather than through a hover so the image is just the
/// breakdown: what it covers is everything the breakdown itself determines -
/// which rows appear, how the channels lay out, and where it truncates.
/// That the hover actually reaches it is covered separately, by the tests
/// that hover a real row.
fn snapshot_breakdown(entry: &RecordingEntry, name: &str) {
    let mut h = TestHarness::builder()
        .size(egui::vec2(420.0, 560.0))
        .ui(|ui| data_breakdown_ui(ui, entry));
    for _ in 0..3 {
        h.run();
    }
    h.snapshot(name);
}

/// The breakdown of a recording carrying ad-hoc sensor channels: its span,
/// its shape on disk, a count per kind of data, and the channels - vector
/// components, units, and sample counts included.
#[test]
fn snapshot_history_row_breakdown() {
    snapshot_breakdown(&entry_with_channels(), "history_row_breakdown");
}

/// A recording with no channels states that in its breakdown rather than
/// rendering nothing. Its hidden tracks also get the note explaining where
/// they came from.
#[test]
fn snapshot_history_row_breakdown_without_channels() {
    let mut entry = sortable_entry(
        "auto:plain.gtd",
        1_700_000_000_000_000,
        900_000_000,
        42,
        4_096,
    );
    entry.total_tracks = 3;
    entry.hidden_tracks = 1;
    snapshot_breakdown(&entry, "history_row_breakdown_no_channels");
}

/// A recording with more channels than the hover lists shows the first
/// [`MAX_HOVER_CHANNELS`] and counts the rest, so the tooltip cannot grow
/// past the screen.
#[test]
fn snapshot_history_row_breakdown_truncates_long_channel_list() {
    let mut entry = entry_with_channels();
    entry.channels = (0..MAX_HOVER_CHANNELS + 3)
        .map(|i| ChannelSummary {
            // Zero-padded so the name order matches the numeric order.
            name: format!("channel_{i:02}"),
            unit: None,
            description: None,
            components: Vec::new(),
            sample_count: 10,
        })
        .collect();
    snapshot_breakdown(&entry, "history_row_breakdown_many_channels");
}

/// A vector channel shows its component labels. A scalar one is just its
/// name.
#[rstest::rstest]
#[case(&[], "accel")]
#[case(&["x", "y", "z"], "accel (x, y, z)")]
fn channel_title_appends_vector_components(#[case] components: &[&str], #[case] expected: &str) {
    let channel = ChannelSummary {
        name: "accel".to_owned(),
        unit: None,
        description: None,
        components: components.iter().map(|s| (*s).to_owned()).collect(),
        sample_count: 0,
    };

    assert_eq!(channel_title(&channel), expected);
}

/// The cursor the window requests right now.
fn cursor_icon(h: &TestHarness<HistoryHarness>) -> egui::CursorIcon {
    h.inner.output().platform_output.cursor_icon
}

/// Each part of the window requests the cursor that matches what it does:
/// only real text entry shows the I-beam, so a column header that sorts on
/// click never reads as a text field.
#[rstest::rstest]
// Sortable headers act on click.
#[case::points_header(point_at_header, "Points", egui::CursorIcon::PointingHand)]
#[case::identity_header(point_at_header, "Identity", egui::CursorIcon::PointingHand)]
// The identity cell renames on double-click and has a context menu.
#[case::identity_cell(point_at_widget, "sensors.gtd", egui::CursorIcon::PointingHand)]
// The toolbar's "Identity" is a term with an explanation, not a control.
#[case::term_label(point_at_widget, "Identity", egui::CursorIcon::Help)]
// Values and captions do nothing on click.
#[case::date_cell(point_at_widget, "2023-11-14 22:13", egui::CursorIcon::Default)]
#[case::duration_cell(point_at_widget, "1h 00m", egui::CursorIcon::Default)]
#[case::points_cell(point_at_widget, "8.9k", egui::CursorIcon::Default)]
#[case::static_caption(point_at_widget, "GB", egui::CursorIcon::Default)]
#[case::button(point_at_widget, "Prune…", egui::CursorIcon::Default)]
#[case::checkbox(point_at_widget, AUTO_STORE_LABEL, egui::CursorIcon::Default)]
fn elements_request_a_cursor_that_matches_what_they_do(
    #[case] hover: fn(&mut TestHarness<HistoryHarness>, &str),
    #[case] label: &str,
    #[case] expected: egui::CursorIcon,
) {
    let mut h = channel_row_harness();

    hover(&mut h, label);

    assert_eq!(
        cursor_icon(&h),
        expected,
        "hovering {label:?} should request {expected:?}",
    );
}

/// The identity filter is real text entry, so it does get the I-beam - the
/// contrast that makes the cursor meaningful everywhere else.
#[test]
fn the_filter_field_still_shows_a_text_cursor() {
    let mut h = channel_row_harness();
    let field = h
        .inner
        .get_all_by_role(egui::accesskit::Role::TextInput)
        .map(|n| n.rect())
        .next()
        .expect("identity filter field");

    h.inner.hover_at_and_settle(field.center(), 1);

    assert_eq!(cursor_icon(&h), egui::CursorIcon::Text);
}

/// A History window showing one recording that carries channels, settled so
/// the auto columns have measured their content.
fn channel_row_harness() -> TestHarness<'static, HistoryHarness> {
    let harness = history_harness(vec![entry_with_channels()]);
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    for _ in 0..4 {
        h.run();
    }
    h
}

/// Hovering *any* of a row's value cells brings up the breakdown, not just
/// the one column whose value is being pointed at - the cells are wired up
/// individually, so each one has to be checked.
#[rstest::rstest]
#[case::date("2023-11-14 22:13")]
#[case::duration("1h 00m")]
#[case::points("8.9k")]
#[case::size("4.0 KB")]
fn hovering_any_value_cell_reveals_the_breakdown(#[case] cell_text: &str) {
    let mut h = channel_row_harness();
    assert!(
        h.inner.query_by_label_contains("custom channel").is_none(),
        "probe: the breakdown must not be visible before the hover",
    );

    hover_widget(&mut h, cell_text);

    assert!(
        h.inner
            .query_by_label_contains("2 custom channels")
            .is_some(),
        "hovering the {cell_text:?} cell should reveal the row's breakdown",
    );
}

/// The breakdown names the recording's ad-hoc sensor channels - their
/// component labels, units, and sample counts - which no table column
/// shows. This is the whole point of the hover.
#[test]
fn the_breakdown_names_the_recordings_channels() {
    let mut h = channel_row_harness();

    hover_widget(&mut h, "8.9k");

    for expected in [
        "2 custom channels",
        "accel (x, y, z)",
        "Frame IMU",
        "temperature",
        "12,000 samples",
        "512 samples",
        "Satellite reports",
        "1,234",
    ] {
        assert!(
            h.inner.query_by_label_contains(expected).is_some(),
            "the breakdown should mention {expected:?}",
        );
    }
}

/// The identity cell keeps its own metadata hover and gains the breakdown,
/// so the tooltip shows the same content wherever along the row it opens.
#[test]
fn hovering_the_identity_cell_shows_metadata_and_the_breakdown() {
    let mut entry = entry_with_channels();
    entry.title = Some("Morning ride".to_owned());
    let harness = history_harness(vec![entry]);
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    for _ in 0..4 {
        h.run();
    }

    hover_widget(&mut h, "sensors.gtd");

    for expected in [
        "Morning ride",
        "2 custom channels",
        "Double-click to rename",
    ] {
        assert!(
            h.inner.query_by_label_contains(expected).is_some(),
            "the identity hover should mention {expected:?}",
        );
    }
}

/// An identity too long for its column opens one tooltip, not two: egui
/// offers the elided text its own tooltip, and the cell's hover already
/// leads with the full identity.
#[test]
fn hovering_a_truncated_identity_opens_a_single_tooltip() {
    let long = "auto:a-recording-identity-far-too-long-for-the-identity-column.gtd";
    let harness = history_harness(vec![entry_with_identity(long)]);
    let mut h = TestHarness::builder()
        .size(egui::vec2(520.0, 300.0))
        .ui_state(show_history, harness);
    for _ in 0..4 {
        h.run();
    }

    hover_widget(&mut h, "a-recording-identity");

    assert!(
        h.inner
            .query_by_label_contains("Double-click to rename")
            .is_some(),
        "probe: the identity hover should be open",
    );
    assert_eq!(
        visible_tooltips(&h),
        1,
        "a truncated identity should not stack egui's elided-text tooltip \
         on top of the cell's own hover",
    );
}

/// How many tooltip layers are on screen.
fn visible_tooltips(h: &TestHarness<HistoryHarness>) -> usize {
    h.inner.ctx.memory(|m| {
        m.areas()
            .visible_layer_ids()
            .iter()
            .filter(|layer| layer.order == egui::Order::Tooltip)
            .count()
    })
}

/// A recording with no channels says so on hover rather than leaving the
/// question unanswered.
#[test]
fn hovering_a_channel_free_row_says_it_has_none() {
    let harness = history_harness(vec![sortable_entry(
        "auto:plain.gtd",
        1_700_000_000_000_000,
        900_000_000,
        42,
        4_096,
    )]);
    let mut h = TestHarness::builder()
        .size(egui::vec2(900.0, 500.0))
        .ui_state(show_history, harness);
    for _ in 0..4 {
        h.run();
    }

    hover_widget(&mut h, "42");

    assert!(
        h.inner
            .query_by_label_contains("No custom channels")
            .is_some(),
        "the breakdown should state that the recording carries no channels",
    );
}

/// Every value cell of a row gets its own breakdown widget id: one per
/// column, and different between rows. Dropping either part of the salt
/// would silently merge neighbouring cells' interaction state.
#[test]
fn breakdown_cell_ids_are_distinct_per_cell() {
    let entries = sortable_entries();
    let first = entries.first().expect("first entry");
    let second = entries.get(1).expect("second entry");

    let cells: Vec<egui::Id> = SortColumn::iter()
        .flat_map(|column| {
            [
                breakdown_cell_id(first, column),
                breakdown_cell_id(second, column),
            ]
        })
        .collect();
    let unique: std::collections::HashSet<egui::Id> = cells.iter().copied().collect();

    assert_eq!(
        unique.len(),
        cells.len(),
        "two breakdown cells share a widget id: {} cells produced {} ids",
        cells.len(),
        unique.len(),
    );
}

/// The track row calls out hidden tracks, and stays quiet when there are
/// none - it is the only place the breakdown mentions them.
#[rstest::rstest]
#[case(4, 0, "4")]
#[case(4, 1, "4 (1 hidden)")]
#[case(0, 0, "0")]
fn track_count_text_names_hidden_tracks(
    #[case] total_tracks: usize,
    #[case] hidden_tracks: usize,
    #[case] expected: &str,
) {
    let mut entry = entry_with_identity("auto:ride.gtd");
    entry.total_tracks = total_tracks;
    entry.hidden_tracks = hidden_tracks;

    assert_eq!(track_count_text(&entry), expected);
}

#[test]
fn identity_display_keeps_full_manual_identity_visible() {
    let identity = "/example.invalid/history/identity/with/slashes/";

    assert_eq!(identity_display_parts(identity), (identity, false));
}

#[test]
fn identity_display_marks_auto_identity_without_losing_original() {
    let identity = "auto:recording-2026-07-09.gtd";

    assert_eq!(
        identity_display_parts(identity),
        ("recording-2026-07-09.gtd", true)
    );
}

/// A History window filled with more recordings than any screen shows at once,
/// each under a long identity, so both axes overflow.
fn crowded_history_harness() -> HistoryHarness {
    let identity = gt_test_utils::oversized_text('r');
    let entries = (0..OVERSIZED_ROW_COUNT)
        .map(|index| {
            let mut entry = entry_with_identity(&format!("{identity}/{index}"));
            entry.meta.gtd_size_bytes = CROWDED_RECORDING_BYTES;
            entry
        })
        .collect();
    history_harness(entries)
}

/// Size given to each of [`crowded_history_harness`]'s recordings, so the
/// footer states a total rather than a placeholder.
const CROWDED_RECORDING_BYTES: u64 = 1024;

/// The stats line the footer ends on for [`crowded_history_harness`].
const CROWDED_FOOTER_STATS: &str = "200 recordings - 200.0 KB";

/// The History window keeps its footer reachable at any viewport: the listing
/// takes the room that is left and scrolls its own rows, instead of pushing the
/// stats line past the bottom of the screen.
#[rstest::rstest]
fn history_window_fits_every_viewport(
    #[values(CRAMPED_VIEWPORT, NARROW_VIEWPORT, SHORT_VIEWPORT)] viewport: egui::Vec2,
) {
    let mut h = TestHarness::builder()
        .size(viewport)
        .ui_state(show_history, crowded_history_harness());
    for _ in 0..8 {
        h.step();
    }
    h.inner
        .assert_window_fits_the_viewport(AuditedWindow::titled("History"));
    h.inner.assert_control_is_reachable(
        AuditedWindow::titled("History"),
        ControlLabel(CROWDED_FOOTER_STATS),
    );
}

/// The prune dialog with a preview of far more recordings than any viewport
/// lists, each named at length.
fn crowded_prune_harness() -> HistoryHarness {
    let identity = gt_test_utils::oversized_text('r');
    let mut harness = history_harness(Vec::new());
    harness.window.prune.open = true;
    harness.window.set_prune_preview(
        (0..OVERSIZED_ROW_COUNT)
            .map(|index| DatabaseRef {
                identity: format!("{identity}/{index}"),
                group_name: format!("rec{index}"),
            })
            .collect(),
    );
    harness
}

/// The prune dialog keeps its destructive action reachable at any viewport: the
/// preview list scrolls rather than pushing the buttons past the screen edge.
#[rstest::rstest]
fn prune_dialog_fits_every_viewport(
    #[values(CRAMPED_VIEWPORT, NARROW_VIEWPORT, SHORT_VIEWPORT)] viewport: egui::Vec2,
) {
    let mut h = TestHarness::builder()
        .size(viewport)
        .ui_state(show_history, crowded_prune_harness());
    for _ in 0..8 {
        h.step();
    }
    h.inner
        .assert_window_fits_the_viewport(AuditedWindow::titled(PRUNE_WINDOW_TITLE));
    h.inner.assert_control_is_reachable(
        AuditedWindow::titled(PRUNE_WINDOW_TITLE),
        ControlLabel("Cancel"),
    );
}

/// The delete-hidden confirmation stays inside the screen and keeps its
/// buttons reachable however many tracks it names.
#[rstest::rstest]
fn delete_hidden_confirmation_fits_every_viewport(
    #[values(CRAMPED_VIEWPORT, NARROW_VIEWPORT, SHORT_VIEWPORT)] viewport: egui::Vec2,
) {
    let mut entry = entry_with_identity(&gt_test_utils::oversized_text('r'));
    entry.total_tracks = OVERSIZED_ROW_COUNT;
    entry.hidden_tracks = OVERSIZED_ROW_COUNT;
    let mut harness = history_harness(vec![entry]);
    harness.window.delete_hidden_confirm_open = true;
    let mut h = TestHarness::builder()
        .size(viewport)
        .ui_state(show_history, harness);
    for _ in 0..8 {
        h.step();
    }
    h.inner
        .assert_window_fits_the_viewport(AuditedWindow::titled(DELETE_HIDDEN_WINDOW_TITLE));
    h.inner.assert_control_is_reachable(
        AuditedWindow::titled(DELETE_HIDDEN_WINDOW_TITLE),
        ControlLabel("Cancel"),
    );
}

/// Screen the height audit runs against: taller than a handful of recordings
/// need, shorter than [`OVERSIZED_ROW_COUNT`] of them.
const HEIGHT_AUDIT_VIEWPORT: egui::Vec2 = egui::vec2(1000.0, 800.0);

/// Settled height of the History window listing `rows` recordings, through the
/// real rendering path ([`HistoryWindow::show`]).
fn settled_history_window_height(rows: usize) -> f32 {
    let entries = (0..rows)
        .map(|index| entry_with_identity(&format!("auto:ride{index}.gtd")))
        .collect();
    let mut h = TestHarness::builder()
        .size(HEIGHT_AUDIT_VIEWPORT)
        .ui_state(show_history, history_harness(entries));
    h.inner
        .settled_window_size("History", 10)
        .expect("the History window is shown")
        .y
}

/// Three recordings leave the History window at the height of three
/// recordings, well under its 480px default height.
#[test]
fn a_short_list_settles_the_window_at_its_content_height() {
    let height = settled_history_window_height(3);
    assert!(
        height < 350.0,
        "the History window settled at {height:.0}px listing three recordings, far more \
         than three rows and the footer need: it stopped tracking its content",
    );
}

/// More recordings than the screen shows grow the window to the screen edge
/// and no further, the rows scrolling inside it from there on.
#[test]
fn a_list_longer_than_the_screen_stops_the_window_at_the_screen_edge() {
    let height = settled_history_window_height(OVERSIZED_ROW_COUNT);
    assert!(
        (600.0..=HEIGHT_AUDIT_VIEWPORT.y).contains(&height),
        "the History window settled at {height:.0}px listing {OVERSIZED_ROW_COUNT} \
         recordings on a {:.0}px screen: it should take the screen's full height and \
         scroll its rows there",
        HEIGHT_AUDIT_VIEWPORT.y,
    );
}
