#![expect(
    clippy::literal_string_with_formatting_args,
    reason = "recording-name templates intentionally contain {token} placeholders, not format args"
)]

use egui::CentralPanel;
use egui_phosphor::regular::CHECK_SQUARE as ICON_CHECK_SQUARE;
use egui_phosphor::regular::EYE_SLASH as ICON_EYE_SLASH;
use egui_phosphor::regular::LINE_SEGMENTS as ICON_LINE_SEGMENTS;
use egui_phosphor::regular::NOTE as ICON_NOTE;
use egui_phosphor::regular::PATH as ICON_PATH;
use egui_phosphor::regular::SQUARE as ICON_SQUARE;
use egui_phosphor::regular::WARNING as ICON_WARNING;
use std::path::PathBuf;

use egui_kittest::Node;
use egui_kittest::kittest::Queryable as _;
use geotrace_sdk_units::Unit;
use gt_filter::GlobalFilter;
use gt_loaded_files::{FileHistory, LoadedFiles, RecordingNames};
use gt_side_panel::{
    FilterPanelState, NodeKey, PanelContext, SnapCostingTarget, SnapPanelView, SnapRowView,
    TreeState, show_side_panel,
};
use gt_test_utils::{By, HarnessInteraction as _, TestHarness};
use gt_types::{
    FileIdx, FixStats, LoadWarning, LoadedFile, NavPoint, PointIdx, TrackIdx, TrackRef,
};
use gt_ui_types::{DisplayCategory, DisplayMask, HighlightScope, MapHighlight, SnapCosting};
use rustc_hash::FxHashMap;

struct State {
    files: LoadedFiles,
    tree: TreeState,
    filter: GlobalFilter,
    filter_state: FilterPanelState,
    highlight: MapHighlight,
    map_center: Option<(f64, f64)>,
    popup_pos: Option<egui::Pos2>,
    zoom_to_visible: bool,
    warnings_request: Option<(String, Vec<LoadWarning>)>,
    clear_query_request: bool,
    display_mask: DisplayMask,
    recording_name_template: String,
    metadata_request: Option<gt_side_panel::RecordingDetails>,
    snap_rows: FxHashMap<TrackRef, SnapRowView>,
    snap_progress: gt_side_panel::SnapProgressView,
    snap_offline: bool,
    snap_consent_pending: bool,
    snap_request: Option<TrackRef>,
    snap_visibility_request: Option<TrackRef>,
    snap_costing_choices: Vec<(SnapCosting, String)>,
    snap_costing_request: Option<(SnapCostingTarget, SnapCosting)>,
    sky_trails_request: Option<gt_ui_types::SkyTrailsRequest>,
}

/// A recording built from `points`, loaded from a path of its own name.
fn build_file(
    name: &str,
    points: &[NavPoint],
    meta: gt_track_builder::FileMeta,
    warnings: Vec<LoadWarning>,
) -> LoadedFile {
    gt_track_builder::build_loaded_file(
        name.to_owned(),
        points,
        &[],
        vec![],
        vec![],
        &[],
        &gt_track_builder::SegmentationConfig::default(),
        gt_types::FileSource::GtdPath(PathBuf::from(name)),
        meta,
        warnings,
    )
}

/// The panel state over `files`, with the tree synced to them, no request
/// pending and every snap view empty.
fn make_state_from_files(files: LoadedFiles) -> State {
    let mut tree = TreeState::new();
    tree.sync_from_loaded_files(files.files());
    State {
        files,
        tree,
        filter: GlobalFilter::default(),
        filter_state: FilterPanelState::default(),
        highlight: MapHighlight::default(),
        map_center: None,
        popup_pos: None,
        zoom_to_visible: false,
        warnings_request: None,
        clear_query_request: false,
        display_mask: DisplayMask::default(),
        recording_name_template: "{filename}".to_owned(),
        metadata_request: None,
        snap_rows: FxHashMap::default(),
        snap_progress: gt_side_panel::SnapProgressView::default(),
        snap_offline: false,
        snap_consent_pending: false,
        snap_request: None,
        snap_visibility_request: None,
        snap_costing_choices: Vec::new(),
        snap_costing_request: None,
        sky_trails_request: None,
    }
}

fn make_state(file_count: usize) -> State {
    make_state_with_warnings_on(file_count, 0, &[])
}

fn make_state_with_warnings_on(
    file_count: usize,
    warned_file: usize,
    warnings: &[LoadWarning],
) -> State {
    let points = gt_test_utils::nav_test_data();
    let mut files = LoadedFiles::new();
    for i in 0..file_count {
        let w = if i == warned_file {
            warnings.to_vec()
        } else {
            vec![]
        };
        let file = build_file(
            &format!("ride_{i}.gtd"),
            &points,
            gt_track_builder::FileMeta::default(),
            w,
        );
        files.push(file, FileHistory::None);
    }
    let mut state = make_state_from_files(files);
    state.snap_costing_choices = vec![
        (SnapCosting::Auto, "Auto".to_owned()),
        (SnapCosting::Bicycle, "Bicycle".to_owned()),
        (SnapCosting::Pedestrian, "Pedestrian".to_owned()),
    ];
    state
}

fn make_harness(state: State) -> TestHarness<'static, State> {
    make_harness_sized(state, egui::vec2(280.0, 600.0))
}

fn make_harness_sized(state: State, size: egui::Vec2) -> TestHarness<'static, State> {
    TestHarness::builder().size(size).ui_state(
        |ui, s: &mut State| {
            let mut ctx = PanelContext {
                loaded_files: s.files.view(),
                tree: &mut s.tree,
                highlight: &mut s.highlight,
                filter: &mut s.filter,
                filter_state: &mut s.filter_state,
                map_center_request: &mut s.map_center,
                popup_pos_request: &mut s.popup_pos,
                query_matches: None,
                zoom_to_visible_request: &mut s.zoom_to_visible,
                warnings_request: &mut s.warnings_request,
                clear_query_request: &mut s.clear_query_request,
                display_mask: s.display_mask,
                recording_names: &RecordingNames::resolve(
                    s.files.view(),
                    &s.recording_name_template,
                ),
                metadata_request: &mut s.metadata_request,
                snap: SnapPanelView {
                    offline: s.snap_offline,
                    consent_pending: s.snap_consent_pending,
                    rows: &s.snap_rows,
                    costing_choices: &s.snap_costing_choices,
                    progress: &s.snap_progress,
                },
                snap_request: &mut s.snap_request,
                snap_visibility_request: &mut s.snap_visibility_request,
                snap_costing_request: &mut s.snap_costing_request,
                sky_trails_request: &mut s.sky_trails_request,
            };
            show_side_panel(ui, &mut ctx);
        },
        state,
    )
}

/// The tree row for a label the Visible section repeats: the tree renders
/// below the section.
fn tree_row<'h>(harness: &'h TestHarness<'static, State>, label: &'h str) -> Node<'h> {
    harness
        .inner
        .bottommost_matching(By::new().label_contains(label))
}

/// A point on the divider between the section and the tree, three points above
/// the first tree row: the panel edge takes a drag within
/// `interaction.resize_grab_radius_side` of it.
fn divider_point(harness: &TestHarness<'static, State>) -> egui::Pos2 {
    let row = tree_row(harness, "ride_0").rect();
    egui::pos2(row.center().x, row.top() - 3.0)
}

/// The Visible section row for a label the tree repeats: the section renders
/// above the tree.
fn section_row<'h>(harness: &'h TestHarness<'static, State>, label: &'h str) -> Node<'h> {
    harness
        .inner
        .topmost_matching(By::new().label_contains(label))
}

/// A panel tall enough for the visible-tracks section to hold several rows:
/// the section takes a fixed share of the panel's height.
fn make_harness_with_a_tall_panel(state: State) -> TestHarness<'static, State> {
    make_harness_sized(state, egui::vec2(280.0, 900.0))
}

#[test]
fn snapshot_collapsed_files() {
    let mut harness = make_harness(make_state(2));
    harness.run();
    harness.snapshot("side_panel_collapsed");
}

#[test]
fn snapshot_one_file_expanded() {
    let mut state = make_state(2);
    state.tree.toggle_expand_file(FileIdx::new(0));
    let mut harness = make_harness(state);
    harness.run();
    harness.snapshot("side_panel_file_expanded");
}

/// The tree's track rows line up in the same columns as the Visible section:
/// a track of over an hour and tens of kilometres above one of half a minute.
#[test]
fn snapshot_tree_track_columns_across_magnitudes() {
    let mut files = LoadedFiles::new();
    files.push(
        build_file(
            "commute.gtd",
            &gt_test_utils::nav_data_with_gap(4_000, 30),
            gt_track_builder::FileMeta::default(),
            vec![],
        ),
        FileHistory::None,
    );
    let mut state = make_state_from_files(files);
    state.tree.toggle_expand_file(FileIdx::new(0));
    let mut harness = make_harness(state);
    harness.run();
    harness.snapshot("side_panel_tree_track_columns");
}

/// One set of column widths over every expanded recording: the second
/// recording's tracks line up under the first's.
#[test]
fn snapshot_tree_track_columns_across_two_expanded_recordings() {
    let mut files = LoadedFiles::new();
    files.push(
        build_file(
            "commute.gtd",
            &gt_test_utils::nav_data_with_gap(4_000, 30),
            gt_track_builder::FileMeta::default(),
            vec![],
        ),
        FileHistory::None,
    );
    files.push(
        build_file(
            "paused.gtd",
            &gt_test_utils::nav_data_with_gap(60, 60),
            gt_track_builder::FileMeta::default(),
            vec![],
        ),
        FileHistory::None,
    );
    let mut state = make_state_from_files(files);
    state.tree.toggle_expand_file(FileIdx::new(0));
    state.tree.toggle_expand_file(FileIdx::new(1));
    let mut harness = make_harness_with_a_tall_panel(state);
    harness.run();
    harness.snapshot("side_panel_tree_track_columns_two_recordings");
}

#[test]
fn snapshot_masked_categories_show_hint() {
    // Categories hidden by the map display toggles get a trailing
    // eye-slash on their tree row - the tree state itself is untouched.
    let mut state = make_state(1);
    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    state.tree.toggle_expand_file(FileIdx::new(0));
    state.tree.toggle_expand_track(track);
    state
        .display_mask
        .set_visible(DisplayCategory::TrackPoints, false);
    state
        .display_mask
        .set_visible(DisplayCategory::GeneratedMarkers, false);
    // Satellite labels are the one row whose label ("Satellite reports")
    // differs from its display category - pin the mapping visually too.
    state
        .display_mask
        .set_visible(DisplayCategory::SatelliteLabels, false);
    let mut harness = make_harness(state);
    harness.run();
    harness.snapshot("side_panel_masked_categories");
}

/// A completed-run row with fixture counts, shown or hidden on the map.
fn done_row(shown: bool) -> SnapRowView {
    SnapRowView::Done {
        snapped: 120,
        interpolated: 340,
        unsnapped: 12,
        confidence_score: Some(0.87),
        shown,
        stale: None,
        partial: false,
        warnings: Vec::new(),
    }
}

/// One track per snap state, exercising every rendering of the trigger: idle
/// (plain icon), unsnappable/queued/in-flight (grayed, per the never-hide
/// rule), failed (amber retry), and done (weak status glyph).
#[test]
fn snapshot_snap_trigger_states() {
    let mut state = make_state(7);
    let track = |i: usize| TrackRef::new(FileIdx::new(i), TrackIdx::new(0));
    for i in 0..7 {
        state.tree.toggle_expand_file(FileIdx::new(i));
    }
    // File 0 stays idle: no entry.
    state.snap_rows.insert(
        track(1),
        SnapRowView::Unsnappable {
            travel_mode: "Boat".to_owned(),
        },
    );
    state.snap_rows.insert(track(2), SnapRowView::Queued);
    state.snap_rows.insert(
        track(3),
        SnapRowView::InFlight {
            completed_chunks: 2,
            total_chunks: 5,
        },
    );
    state.snap_rows.insert(
        track(4),
        SnapRowView::Failed {
            error: "server unreachable".to_owned(),
        },
    );
    state.snap_rows.insert(track(5), done_row(true));
    // A completed run whose snapped track is toggled hidden: the status
    // glyph dims further.
    state.snap_rows.insert(track(6), done_row(false));
    // Taller than the default harness: seven expanded files must all fit.
    // Bounded steps: the in-flight spinner repaints forever, so
    // run-until-idle would never settle.
    let mut harness = make_harness_sized(state, egui::vec2(280.0, 720.0));
    harness.inner.run_steps(3);
    harness.snapshot("side_panel_snap_states");
}

/// The progress strip while a run is in flight with more queued: bar with
/// the current action, queue count beside it, pinned to the panel bottom.
#[test]
fn snapshot_snap_progress_strip_active() {
    let mut state = make_state(3);
    let track = |i: usize| TrackRef::new(FileIdx::new(i), TrackIdx::new(0));
    state.snap_rows.insert(
        track(0),
        SnapRowView::InFlight {
            completed_chunks: 2,
            total_chunks: 5,
        },
    );
    state.snap_rows.insert(track(1), SnapRowView::Queued);
    state.snap_rows.insert(track(2), SnapRowView::Queued);
    state.snap_progress = gt_side_panel::SnapProgressView {
        in_flight: Some(gt_side_panel::SnapInFlightView {
            track: track(0),
            completed_chunks: 2,
            total_chunks: 5,
        }),
        queued: 2,
    };
    // Bounded steps: the in-flight spinner repaints forever.
    let mut harness = make_harness(state);
    harness.inner.run_steps(3);
    harness.snapshot("side_panel_snap_progress_strip");
}

/// Queued work with the app offline: the strip states that the queue is paused
/// and why.
#[test]
fn snapshot_snap_progress_strip_offline_paused() {
    let mut state = make_state(1);
    let track = |i: usize| TrackRef::new(FileIdx::new(i), TrackIdx::new(0));
    state.snap_offline = true;
    state.snap_rows.insert(track(0), SnapRowView::Queued);
    state.snap_progress = gt_side_panel::SnapProgressView {
        in_flight: None,
        queued: 1,
    };
    let mut harness = make_harness(state);
    harness.run();
    harness.snapshot("side_panel_snap_progress_strip_offline");
}

/// A partial run with warnings: the status hover must surface the partial
/// marker and every warning line - anomalies are signal, never hidden. The
/// snapshot captures the opened hover tooltip.
#[test]
fn snapshot_snap_status_hover_with_warnings() {
    let mut state = make_state(1);
    state.tree.toggle_expand_file(FileIdx::new(0));
    state.snap_rows.insert(
        TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        SnapRowView::Done {
            snapped: 120,
            interpolated: 340,
            unsnapped: 12,
            confidence_score: Some(0.87),
            shown: true,
            stale: None,
            partial: true,
            warnings: vec![
                "Chunk 3 failed - its points carry no snap data (HTTP 502)".to_owned(),
                "The map data updated mid-run (OSM changeset 100 to 200)".to_owned(),
            ],
        },
    );
    let mut harness = make_harness_sized(state, egui::vec2(560.0, 480.0));
    harness.run();

    harness.inner.get_by_label(ICON_PATH).hover();
    // Tooltips appear after egui's hover delay; keep stepping until the
    // delay has elapsed and the tooltip laid itself out.
    for _ in 0..60 {
        harness.run();
    }
    harness.snapshot("side_panel_snap_status_warnings");
}

/// A stale run's row for the stale-state tests: the [`done_row`] fixture
/// with one named parameter difference.
fn stale_row() -> SnapRowView {
    SnapRowView::Done {
        snapped: 120,
        interpolated: 340,
        unsnapped: 12,
        confidence_score: Some(0.87),
        shown: true,
        stale: Some(vec![
            "Snapped as Bicycle - would now snap as Auto".to_owned(),
        ]),
        partial: false,
        warnings: Vec::new(),
    }
}

/// A stale completed run: the status glyph turns warning-colored and the
/// re-run trigger appears next to it - the outdated result stays visible
/// but can never pass as current.
#[test]
fn snapshot_snap_stale_run() {
    let mut state = make_state(1);
    state.tree.toggle_expand_file(FileIdx::new(0));
    state.snap_rows.insert(
        TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        stale_row(),
    );
    let mut harness = make_harness(state);
    harness.run();
    harness.snapshot("side_panel_snap_stale");
}

/// Clicking a stale run's re-run trigger requests a snap (not the
/// visibility toggle) - the two controls sit side by side, status glyph
/// first, trigger second.
#[test]
fn clicking_stale_trigger_requests_snap() {
    let mut state = make_state(1);
    state.tree.toggle_expand_file(FileIdx::new(0));
    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    state.snap_rows.insert(track, stale_row());
    let mut harness = make_harness(state);
    harness.run();

    // The status glyph and the re-run trigger use distinct icons.
    harness.inner.get_by_label(ICON_PATH).hover();
    harness.inner.get_by_label(ICON_LINE_SEGMENTS).click();
    harness.run();

    assert_eq!(harness.state().snap_request, Some(track));
    assert_eq!(
        harness.state().snap_visibility_request,
        None,
        "the re-run trigger must not toggle visibility"
    );
}

/// A completed run whose ink the map display toggles hide: the status glyph
/// gains the trailing eye-slash hint, same as masked tree category rows.
#[test]
fn snapshot_snap_glyph_masked_category() {
    let mut state = make_state(1);
    state.tree.toggle_expand_file(FileIdx::new(0));
    state.snap_rows.insert(
        TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        done_row(true),
    );
    state
        .display_mask
        .set_visible(DisplayCategory::SnappedTracks, false);
    let mut harness = make_harness(state);
    harness.run();
    harness.snapshot("side_panel_snap_glyph_masked");
}

/// Offline, every snap trigger is grayed out, never hidden.
#[test]
fn snapshot_snap_trigger_offline() {
    let mut state = make_state(1);
    state.tree.toggle_expand_file(FileIdx::new(0));
    state.snap_offline = true;
    let mut harness = make_harness(state);
    harness.run();
    harness.snapshot("side_panel_snap_offline");
}

#[test]
fn clicking_snap_trigger_requests_snap() {
    let mut state = make_state(1);
    state.tree.toggle_expand_file(FileIdx::new(0));
    let mut harness = make_harness(state);
    harness.run();

    harness.inner.get_by_label(ICON_LINE_SEGMENTS).click();
    harness.run();

    assert_eq!(
        harness.state().snap_request,
        Some(TrackRef::new(FileIdx::new(0), TrackIdx::new(0))),
        "clicking the snap trigger must hand the track to the app"
    );
}

/// While consent is pending a click opens the consent dialog first, so the
/// trigger carries the `…` suffix (and only then, per DESIGN.md).
#[test]
fn snap_trigger_carries_ellipsis_only_while_consent_pending() {
    let mut state = make_state(1);
    state.tree.toggle_expand_file(FileIdx::new(0));
    state.snap_consent_pending = true;
    let mut harness = make_harness(state);
    harness.run();

    let suffixed = format!("{ICON_LINE_SEGMENTS}…");
    harness.inner.get_by_label(&suffixed).click();
    harness.run();
    assert_eq!(
        harness.state().snap_request,
        Some(TrackRef::new(FileIdx::new(0), TrackIdx::new(0))),
        "the suffixed trigger must still hand the track to the app"
    );

    harness.state_mut().snap_consent_pending = false;
    harness.run();
    assert!(
        harness.inner.query_by_label(&suffixed).is_none(),
        "the suffix must disappear once consent is granted"
    );
    assert!(harness.inner.query_by_label(ICON_LINE_SEGMENTS).is_some());
}

/// A grayed trigger (offline here) must not produce a request.
#[test]
fn disabled_snap_trigger_does_not_request() {
    let mut state = make_state(1);
    state.tree.toggle_expand_file(FileIdx::new(0));
    state.snap_offline = true;
    let mut harness = make_harness(state);
    harness.run();

    harness.inner.get_by_label(ICON_LINE_SEGMENTS).click();
    harness.run();

    assert_eq!(harness.state().snap_request, None);
}

/// Clicking a completed run's status glyph requests the snapped-track
/// visibility toggle (and not a snap run).
#[test]
fn clicking_done_glyph_requests_visibility_toggle() {
    let mut state = make_state(1);
    state.tree.toggle_expand_file(FileIdx::new(0));
    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    state.snap_rows.insert(
        track,
        SnapRowView::Done {
            snapped: 10,
            interpolated: 20,
            unsnapped: 0,
            confidence_score: None,
            shown: true,
            stale: None,
            partial: false,
            warnings: Vec::new(),
        },
    );
    let mut harness = make_harness(state);
    harness.run();

    harness.inner.get_by_label(ICON_PATH).click();
    harness.run();

    assert_eq!(harness.state().snap_visibility_request, Some(track));
    assert_eq!(
        harness.state().snap_request,
        None,
        "the status glyph must not queue a snap run"
    );
}

/// The context menu's snapped-track entry mirrors the status glyph: it
/// requests the visibility toggle for the right-clicked track.
#[test]
fn context_menu_toggles_snapped_track_visibility() {
    let mut state = make_state(1);
    state.tree.toggle_expand_file(FileIdx::new(0));
    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    state.snap_rows.insert(
        track,
        SnapRowView::Done {
            snapped: 10,
            interpolated: 20,
            unsnapped: 0,
            confidence_score: None,
            shown: true,
            stale: None,
            partial: false,
            warnings: Vec::new(),
        },
    );
    let mut harness = make_harness(state);
    harness.run();

    tree_row(&harness, "#1  4.6 km").click_secondary();
    harness.run();
    harness.inner.get_by_label("Hide snapped track").click();
    harness.run();

    assert_eq!(harness.state().snap_visibility_request, Some(track));
}

/// The "Snap again as" submenu: hovering it on a completed run's context
/// menu opens the costing choices; clicking one requests the costing
/// re-run (and not a plain snap or a visibility toggle).
#[test]
fn costing_submenu_requests_the_chosen_costing() {
    let mut state = make_state(1);
    state.tree.toggle_expand_file(FileIdx::new(0));
    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    state.snap_rows.insert(track, done_row(true));
    let mut harness = make_harness(state);
    harness.run();

    tree_row(&harness, "#1  4.6 km").click_secondary();
    harness.run();
    harness
        .inner
        .hover_and_settle(By::new().label_contains("Snap again as"), 3);
    harness.inner.get_by_label("Bicycle").click();
    harness.run();

    assert_eq!(
        harness.state().snap_costing_request,
        Some((SnapCostingTarget::Track(track), SnapCosting::Bicycle)),
    );
    assert_eq!(harness.state().snap_request, None);
    assert_eq!(harness.state().snap_visibility_request, None);
}

/// The recording row's context menu carries the same submenu, targeting
/// the whole recording so the app can request a scope.
#[test]
fn recording_context_menu_requests_the_costing_for_the_recording() {
    let state = make_state(1);
    let mut harness = make_harness(state);
    harness.run();

    tree_row(&harness, "ride_0").click_secondary();
    harness.run();
    harness
        .inner
        .hover_and_settle(By::new().label_contains("Snap again as"), 3);
    harness.inner.get_by_label("Pedestrian").click();
    harness.run();

    assert_eq!(
        harness.state().snap_costing_request,
        Some((
            SnapCostingTarget::Recording(FileIdx::new(0)),
            SnapCosting::Pedestrian
        )),
    );
}

/// The status glyph's own context menu offers the same re-run submenu as
/// the row's, so the icon showing the result is also where it is replaced.
#[test]
fn status_glyph_context_menu_requests_the_chosen_costing() {
    let mut state = make_state(1);
    state.tree.toggle_expand_file(FileIdx::new(0));
    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    state.snap_rows.insert(track, done_row(true));
    let mut harness = make_harness(state);
    harness.run();

    harness.inner.get_by_label(ICON_PATH).click_secondary();
    harness.run();
    harness
        .inner
        .hover_and_settle(By::new().label_contains("Snap again as"), 3);
    harness.inner.get_by_label("Bicycle").click();
    harness.run();

    assert_eq!(
        harness.state().snap_costing_request,
        Some((SnapCostingTarget::Track(track), SnapCosting::Bicycle)),
    );
    assert_eq!(harness.state().snap_visibility_request, None);
}

/// A declared road-less mode gets the "Snap as" override submenu - wrong
/// declarations happen, and the submenu is the escape hatch.
#[test]
fn unsnappable_rows_offer_the_costing_override() {
    let mut state = make_state(1);
    state.tree.toggle_expand_file(FileIdx::new(0));
    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    state.snap_rows.insert(
        track,
        SnapRowView::Unsnappable {
            travel_mode: "Boat".to_owned(),
        },
    );
    let mut harness = make_harness(state);
    harness.run();

    tree_row(&harness, "#1  4.6 km").click_secondary();
    harness.run();
    harness
        .inner
        .hover_and_settle(By::new().label_contains("Snap as"), 3);
    harness.inner.get_by_label("Pedestrian").click();
    harness.run();

    assert_eq!(
        harness.state().snap_costing_request,
        Some((SnapCostingTarget::Track(track), SnapCosting::Pedestrian)),
    );
}

/// Snapshot: the open context menu of a completed run, with the costing
/// submenu expanded.
#[test]
fn snapshot_snap_costing_submenu() {
    let mut state = make_state(1);
    state.tree.toggle_expand_file(FileIdx::new(0));
    state.snap_rows.insert(
        TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        done_row(true),
    );
    let mut harness = make_harness_sized(state, egui::vec2(420.0, 600.0));
    harness.run();

    tree_row(&harness, "#1  4.6 km").click_secondary();
    harness.run();
    harness
        .inner
        .hover_and_settle(By::new().label_contains("Snap again as"), 3);
    harness.snapshot("side_panel_snap_costing_submenu");
}

#[test]
fn snapshot_generated_markers_grouped() {
    // `nav_test_data` drops PRNs 9-12 (above the mask) at one epoch, producing a
    // single multi-satellite loss-of-lock slip - so the generated-markers section
    // shows the per-type nesting and the "(4)" satellite count.
    let mut state = make_state(1);
    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    state.tree.toggle_expand_file(FileIdx::new(0));
    state.tree.toggle_expand_track(track);
    state
        .tree
        .toggle_category_expanded(track, gt_types::DataCategory::GeneratedMarker);
    // Expand the slip type group so its individual marker row (with the "(4)"
    // satellite count) is shown beneath the per-type heading.
    state
        .tree
        .toggle_generated_kind_expanded(track, gt_types::GeneratedMarkerKindTag::Slip);
    let mut harness = make_harness(state);
    harness.run();
    harness.snapshot("side_panel_generated_markers");
}

fn first_track() -> TrackRef {
    TrackRef::new(FileIdx::new(0), TrackIdx::new(0))
}

/// A recording of two tracks. Hiding the first leaves the second in the
/// Visible section.
fn make_state_with_a_two_track_recording() -> State {
    let mut files = LoadedFiles::new();
    files.push(
        build_file(
            "paused.gtd",
            &gt_test_utils::nav_data_with_gap(60, 60),
            gt_track_builder::FileMeta::default(),
            vec![],
        ),
        FileHistory::None,
    );
    make_state_from_files(files)
}

/// The Visible section groups the tracks toggled on under their recording's
/// caption: the second recording's hidden track is left out.
#[test]
fn snapshot_visible_section_groups_the_tracks_under_their_recording() {
    let mut files = LoadedFiles::new();
    files.push(
        build_file(
            "ride.gtd",
            &gt_test_utils::nav_test_data(),
            gt_track_builder::FileMeta::default(),
            vec![],
        ),
        FileHistory::None,
    );
    files.push(
        build_file(
            "paused.gtd",
            &gt_test_utils::nav_data_with_gap(60, 60),
            gt_track_builder::FileMeta::default(),
            vec![],
        ),
        FileHistory::None,
    );
    let mut state = make_state_from_files(files);
    state
        .tree
        .toggle_track_check(TrackRef::new(FileIdx::new(1), TrackIdx::new(1)));
    let mut harness = make_harness_with_a_tall_panel(state);
    harness.run();
    harness.snapshot("side_panel_visible_section");
}

/// The section's columns line up across magnitudes: a track of hours and tens
/// of kilometres above two tracks of under a minute and under a kilometre.
#[test]
fn snapshot_visible_section_columns_across_magnitudes() {
    let start = chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
        .and_then(|d| d.and_hms_opt(12, 0, 0))
        .expect("valid date")
        .and_utc();
    let mut files = LoadedFiles::new();
    files.push(
        build_file(
            "commute.gtd",
            &gt_test_utils::nav_points_from(start, 400, 30),
            gt_track_builder::FileMeta::default(),
            vec![],
        ),
        FileHistory::None,
    );
    files.push(
        build_file(
            "paused.gtd",
            &gt_test_utils::nav_data_with_gap(60, 60),
            gt_track_builder::FileMeta::default(),
            vec![],
        ),
        FileHistory::None,
    );
    let mut harness = make_harness_with_a_tall_panel(make_state_from_files(files));
    harness.run();
    harness.snapshot("side_panel_visible_section_columns");
}

/// More tracks than the section's height holds scroll inside it, and the tree
/// keeps the rest of the panel.
#[test]
fn snapshot_visible_section_scrolls_the_tracks_past_its_height() {
    let mut harness = make_harness_with_a_tall_panel(make_state(8));
    harness.run();
    harness.snapshot("side_panel_visible_section_scrolling");
}

#[test]
fn snapshot_visible_section_with_every_track_hidden() {
    let mut state = make_state(2);
    state.tree.set_all_enabled(false);
    let mut harness = make_harness_with_a_tall_panel(state);
    harness.run();
    harness.snapshot("side_panel_visible_section_empty");
}

#[test]
fn showing_a_hidden_recording_leaves_the_tree_rows_in_place() {
    let mut state = make_state(2);
    state.tree.toggle_file_check(FileIdx::new(0));
    let mut harness = make_harness(state);
    harness.run();
    let before = tree_row(&harness, "ride_1").rect();

    harness.inner.get_by_label(ICON_SQUARE).click();
    harness.run();

    assert!(harness.state().tree.visibility().files[0].enabled);
    assert_eq!(tree_row(&harness, "ride_1").rect(), before);
}

/// The section packs its rows tighter than the tree, so its fixed height holds
/// more of them.
#[test]
fn the_visible_section_rows_sit_closer_than_the_tree_rows() {
    let mut state = make_state_with_a_two_track_recording();
    state.tree.toggle_expand_file(FileIdx::new(0));
    let mut harness = make_harness_with_a_tall_panel(state);
    harness.run();

    let section_pitch =
        section_row(&harness, "#2  ").rect().top() - section_row(&harness, "#1  ").rect().top();
    let tree_pitch =
        tree_row(&harness, "#2  ").rect().top() - tree_row(&harness, "#1  ").rect().top();

    assert!(
        section_pitch < tree_pitch * 0.8,
        "section rows are {section_pitch} apart, tree rows {tree_pitch}"
    );
}

#[test]
fn hiding_every_track_leaves_the_tree_rows_in_place() {
    let mut harness = make_harness(make_state(2));
    harness.run();
    let before = tree_row(&harness, "ride_1").rect();

    harness.state_mut().tree.set_all_enabled(false);
    harness.run();

    assert_eq!(tree_row(&harness, "ride_1").rect(), before);
}

/// The section opens at the share the tree state holds, where the app puts the
/// persisted share before the first frame.
#[test]
fn the_visible_section_opens_at_the_stored_share_of_the_region() {
    let mut state = make_state(2);
    state.tree.set_visible_section_fraction(0.5);
    let mut harness = make_harness(state);
    harness.run();

    let share = harness.state().tree.visible_section_fraction();
    assert!(
        (share - 0.5).abs() < 0.02,
        "the section opened at {share} of the region"
    );
}

/// A stored share above the divider's maximum opens the section at that
/// maximum, three quarters of the region.
#[test]
fn a_stored_share_above_the_maximum_opens_the_section_at_the_maximum() {
    let mut state = make_state(2);
    state.tree.set_visible_section_fraction(1.0);
    let mut harness = make_harness(state);
    harness.run();

    let share = harness.state().tree.visible_section_fraction();
    assert!(
        (share - 0.75).abs() < 0.02,
        "the section opened at {share} of the region"
    );
}

/// Dragging the divider down writes the section's larger share back to the
/// tree state, which is what the app persists.
#[test]
fn dragging_the_divider_writes_the_new_share_back() {
    let mut harness = make_harness(make_state(2));
    harness.run();
    let before = harness.state().tree.visible_section_fraction();

    let divider = divider_point(&harness);
    harness
        .inner
        .press_drag_release(divider, egui::vec2(0.0, 100.0), 4);
    harness.run();

    let after = harness.state().tree.visible_section_fraction();
    assert!(
        after > before + 0.2,
        "the section went from {before} to {after} of the region"
    );
}

#[test]
fn the_visible_section_checkbox_hides_one_track_of_a_fully_visible_recording() {
    let mut harness = make_harness(make_state_with_a_two_track_recording());
    harness.run();

    // The highest checkbox on screen is the section's first track row: the
    // tree below opens with its recordings collapsed.
    harness
        .inner
        .topmost_matching(By::new().label(ICON_CHECK_SQUARE))
        .click();
    harness.run();

    let visibility = harness.state().tree.visibility();
    assert!(!visibility.files[0].tracks[0].enabled);
    assert!(visibility.files[0].tracks[1].enabled);
    assert!(
        !harness.state().tree.files[0].expanded,
        "the checkbox takes the click on its own, without the row revealing the track"
    );
    assert!(harness.state().tree.selection.is_empty());
    assert!(
        harness.inner.query_by_label_contains("#1  ").is_none(),
        "the hidden track leaves the section"
    );
    let caption = section_row(&harness, "paused.gtd").rect();
    let remaining_track = section_row(&harness, "#2  ").rect();
    assert!(
        caption.top() < remaining_track.top(),
        "the caption stays above the track the recording has left"
    );
}

#[test]
fn clicking_a_visible_section_row_reveals_the_track_in_the_tree() {
    let mut harness = make_harness(make_state(1));
    harness.run();
    assert!(!harness.state().tree.files[0].expanded);

    section_row(&harness, "#1  4.6 km").click();
    harness.run();

    assert!(harness.state().tree.files[0].expanded);
    assert!(
        harness
            .state()
            .tree
            .selection
            .contains(&NodeKey::Track(first_track()))
    );
}

/// The row is one surface over its whole width: a click past its columns
/// reveals the track, as a click on them does.
#[test]
fn clicking_a_visible_section_row_past_its_columns_reveals_the_track() {
    let mut harness = make_harness(make_state(1));
    harness.run();
    let row = section_row(&harness, "#1  4.6 km").rect();

    harness
        .inner
        .click_at(egui::pos2(row.right() - 2.0, row.center().y));

    assert!(harness.state().tree.files[0].expanded);
}

#[test]
fn the_visible_section_context_menu_hides_the_track_it_was_opened_on() {
    let mut harness = make_harness(make_state(1));
    harness.run();

    section_row(&harness, "#1  4.6 km").click_secondary();
    harness.run();
    harness.inner.get_by_label("Hide").click();
    harness.run();

    assert!(!harness.state().tree.visibility().files[0].tracks[0].enabled);
}

#[test]
fn the_visible_section_context_menu_hides_every_track_of_the_recording() {
    let mut harness = make_harness(make_state_with_a_two_track_recording());
    harness.run();

    section_row(&harness, "#1  ").click_secondary();
    harness.run();
    harness.inner.get_by_label("Hide recording").click();
    harness.run();

    let visibility = harness.state().tree.visibility();
    assert!(!visibility.files[0].enabled);
    assert!(
        visibility.files[0]
            .tracks
            .iter()
            .all(|track| !track.enabled)
    );
}

#[test]
fn the_visible_section_context_menu_shows_only_the_track_it_was_opened_on() {
    let mut harness = make_harness_with_a_tall_panel(make_state(2));
    harness.run();

    // One track row per recording, both the section's: the tree below opens
    // with its recordings collapsed.
    harness
        .inner
        .bottommost_matching(By::new().label_contains("#1  4.6 km"))
        .click_secondary();
    harness.run();
    harness.inner.get_by_label("Show only this track").click();
    harness.run();

    let visibility = harness.state().tree.visibility();
    assert!(!visibility.files[0].enabled);
    assert!(visibility.files[1].tracks[0].enabled);
    assert!(harness.state().zoom_to_visible);
}

#[test]
fn hovering_a_visible_section_row_marks_its_track() {
    let mut harness = make_harness(make_state(1));
    harness.run();

    let row = section_row(&harness, "#1  4.6 km").rect().center();
    harness.inner.hover_at_and_settle(row, 3);

    assert_eq!(
        harness.state().highlight.hover,
        Some(HighlightScope::Track(first_track()))
    );
}

#[test]
fn renders_without_panic() {
    let mut harness = make_harness(make_state(1));
    harness.run();
}

#[test]
fn hiding_file_updates_visibility() {
    let mut harness = make_harness(make_state(1));
    harness.run();
    harness.state_mut().tree.toggle_file_check(FileIdx::new(0));
    harness.run();
    let vis = harness.state().tree.visibility();
    assert!(!vis.files[0].enabled, "file should be hidden after toggle");
}

#[test]
fn hiding_one_track_makes_file_mixed() {
    let mut harness = make_harness(make_state(1));
    harness.run();
    let track_count = harness.state().files[0].tracks.len();
    if track_count < 2 {
        return; // need at least 2 tracks
    }
    harness
        .state_mut()
        .tree
        .toggle_track_check(TrackRef::new(FileIdx::new(0), TrackIdx::new(0)));
    harness.run();
    let check = harness.state().tree.files[0].check;
    assert_eq!(
        check,
        gt_side_panel::CheckState::Mixed,
        "file should be Mixed when one track is hidden"
    );
}

#[test]
fn expand_file_is_reflected_in_tree_state() {
    let mut harness = make_harness(make_state(1));
    harness.run();
    assert!(!harness.state().tree.files[0].expanded);
    harness.state_mut().tree.toggle_expand_file(FileIdx::new(0));
    harness.run();
    assert!(harness.state().tree.files[0].expanded);
}

/// The tree's track row is one surface over its whole width: a click past its
/// columns expands the track.
#[test]
fn clicking_a_tree_track_row_past_its_columns_expands_the_track() {
    let mut state = make_state(1);
    state.tree.toggle_expand_file(FileIdx::new(0));
    let mut harness = make_harness(state);
    harness.run();
    let row = tree_row(&harness, "#1  4.6 km").rect();

    harness
        .inner
        .click_at(egui::pos2(row.right() - 2.0, row.center().y));

    assert!(harness.state().tree.files[0].tracks[0].expanded);
}

#[test]
fn track_without_satellite_reports_falls_back_to_no_data_tooltip() {
    let points = gt_test_utils::stationary_nav_data(10);
    let file = build_file(
        "no_sats.gtd",
        &points,
        gt_track_builder::FileMeta::default(),
        vec![],
    );
    assert_eq!(file.tracks.len(), 1);
    assert!(
        file.tracks[0].metadata.fix_stats.is_none(),
        "track with no satellite reports should have fix_stats == None"
    );

    let mut files = LoadedFiles::new();
    files.push(file, FileHistory::None);
    let mut state = make_state_from_files(files);
    state.tree.toggle_expand_file(FileIdx::new(0));

    // Renders the expanded track row, exercising the `fix_stats == None` fallback
    // ("No satellite data") instead of the colored tooltip.
    let mut harness = make_harness(state);
    harness.run();
}

/// The hover text states the time range and the recorded time apart for a
/// recording that idled between its tracks.
#[test]
fn the_recording_row_hover_states_the_time_range_and_the_recorded_time() {
    // The hover text states both times even for a recording with no fix stats:
    // the two 60-point tracks here lie ten minutes apart and carry no
    // satellite reports.
    let points = gt_test_utils::nav_data_with_gap(60, 60);
    let mut files = LoadedFiles::new();
    files.push(
        build_file(
            "paused.gtd",
            &points,
            gt_track_builder::FileMeta::default(),
            vec![],
        ),
        FileHistory::None,
    );
    let mut harness = make_harness(make_state_from_files(files));
    harness.run();

    let row = tree_row(&harness, "paused.gtd").rect().center();
    harness.inner.hover_at_and_settle(row, 3);

    assert!(
        harness
            .inner
            .query_by_label("2026-01-01 12:00:00 – 12:11:59")
            .is_some(),
        "the hover text must state the range the recording covers"
    );
    assert!(
        harness
            .inner
            .query_by_label("Recorded time 1m58s")
            .is_some(),
        "the hover text must state the recorded time its tracks hold"
    );
}

#[test]
fn the_recording_row_hover_states_an_em_dash_for_a_recording_with_no_fixes() {
    let mut files = LoadedFiles::new();
    files.push(
        build_file(
            "no_fixes.gtd",
            &[],
            gt_track_builder::FileMeta::default(),
            vec![],
        ),
        FileHistory::None,
    );
    let mut harness = make_harness(make_state_from_files(files));
    harness.run();

    let row = tree_row(&harness, "no_fixes.gtd").rect().center();
    harness.inner.hover_at_and_settle(row, 3);

    assert!(
        harness.inner.query_by_label(gt_ui_theme::EM_DASH).is_some(),
        "the hover text must state a recording with no track covers no span"
    );
}

#[test]
fn snapshot_fix_stats_tooltip_content() {
    let stats = FixStats {
        time_with_fix: chrono::Duration::seconds(4800), // 1h20m
        time_without_fix: chrono::Duration::seconds(900), // 15m
        fix_loss_count: 3,
        max_continuous_no_fix: chrono::Duration::seconds(480), // 8m
    };
    let mut h = TestHarness::builder()
        .size(egui::vec2(480.0, 30.0))
        .ui(move |ui| {
            ui.add_space(4.0);
            gt_side_panel::widgets::fix_stats_tooltip_row(ui, stats);
        });
    h.run();
    h.snapshot("fix_stats_tooltip_content");
}

#[test]
fn snapshot_file_with_warnings() {
    let warnings = [
        LoadWarning {
            count: 3,
            issue: "satellite(s) with PRN 0".to_owned(),
            description: "PRN 0 is reserved and undefined in NMEA".to_owned(),
        },
        LoadWarning {
            count: 2,
            issue: "satellite(s) with elevation > 90°".to_owned(),
            description: "above the zenith; valid NMEA elevation range is [0°, 90°]".to_owned(),
        },
    ];
    let state = make_state_with_warnings_on(2, 0, &warnings);
    let mut harness = make_harness(state);
    harness.run();
    harness.snapshot("side_panel_file_with_warnings");
}

/// A track holding fixes with a coordinate out of range, and one of a
/// recording that has no valid position at all, each marked with the warning
/// glyph.
#[test]
fn snapshot_tracks_with_coordinates_out_of_range() {
    let mut files = LoadedFiles::new();
    files.push(
        build_file(
            "out_of_range.gtd",
            &gt_test_utils::nav_points_with_a_latitude_out_of_range(5, PointIdx::new(2)),
            gt_track_builder::FileMeta::default(),
            vec![],
        ),
        FileHistory::None,
    );
    files.push(
        build_file(
            "no_position.gtd",
            &gt_test_utils::nav_points_without_a_valid_position(4),
            gt_track_builder::FileMeta::default(),
            vec![],
        ),
        FileHistory::None,
    );
    let mut state = make_state_from_files(files);
    state.tree.toggle_expand_file(FileIdx::new(0));
    state.tree.toggle_expand_file(FileIdx::new(1));
    let mut harness = make_harness(state);
    harness.run();
    harness.snapshot("side_panel_coordinates_out_of_range");
}

#[test]
fn snapshot_track_channels() {
    // A stationary track (starts 2026-01-01T12:00:00Z, 1 pt/s) plus two channels
    // whose samples fall in its range: a vector accel and a scalar incline.
    let start = chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
        .and_then(|d| d.and_hms_opt(12, 0, 0))
        .expect("valid date")
        .and_utc();
    let points = gt_test_utils::stationary_nav_data(10);
    let accel = gt_types::Channel {
        name: "accel".to_owned(),
        unit: Some(Unit::G.into()),
        period: None,
        description: None,
        components: vec!["x".to_owned(), "y".to_owned(), "z".to_owned()],
        times: vec![start, start + chrono::Duration::seconds(1)],
        values: vec![0.1, 0.2, 0.98, -0.1, 0.3, 1.02],
    };
    let incline = gt_types::Channel {
        name: "incline".to_owned(),
        unit: Some(Unit::DEG.into()),
        period: None,
        description: None,
        components: vec![],
        times: vec![start, start + chrono::Duration::seconds(2)],
        values: vec![1.5, 2.0],
    };
    // A unitless scalar channel exercises the `unit: None` label branch.
    let raw = gt_types::Channel {
        name: "raw".to_owned(),
        unit: None,
        period: None,
        description: None,
        components: vec![],
        times: vec![start],
        values: vec![42.0],
    };
    let file = gt_track_builder::build_loaded_file(
        "sensors.gtd".to_owned(),
        &points,
        &[],
        vec![],
        vec![],
        &[accel, incline, raw],
        &gt_track_builder::SegmentationConfig::default(),
        gt_types::FileSource::GtdPath(PathBuf::from("sensors.gtd")),
        gt_track_builder::FileMeta::default(),
        vec![],
    );

    let mut tree = TreeState::new();
    tree.sync_from_loaded_files(std::slice::from_ref(&file));
    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    tree.toggle_expand_file(FileIdx::new(0));
    tree.toggle_expand_track(track);
    tree.toggle_channels_expanded(track);
    let mut files = LoadedFiles::new();
    files.push(file, FileHistory::None);

    let state = State {
        files,
        tree,
        filter: GlobalFilter::default(),
        filter_state: FilterPanelState::default(),
        highlight: MapHighlight::default(),
        map_center: None,
        popup_pos: None,
        zoom_to_visible: false,
        warnings_request: None,
        clear_query_request: false,
        display_mask: DisplayMask::default(),
        recording_name_template: "{filename}".to_owned(),
        metadata_request: None,
        snap_rows: FxHashMap::default(),
        snap_progress: gt_side_panel::SnapProgressView::default(),
        snap_offline: false,
        snap_consent_pending: false,
        snap_request: None,
        snap_visibility_request: None,
        snap_costing_choices: Vec::new(),
        snap_costing_request: None,
        sky_trails_request: None,
    };
    let mut harness = make_harness(state);
    harness.run();
    harness.snapshot("side_panel_track_channels");
}

/// Two files whose names share the prefix `/home/user/gps/recordings/`.
/// The panel should display only the part after the shared prefix, with the
/// full path shown as hover text.
fn make_state_with_shared_prefix() -> State {
    let points = gt_test_utils::nav_test_data();
    let mut files = LoadedFiles::new();
    for name in [
        "/home/user/gps/recordings/2024-01-15_morning_ride.gtd",
        "/home/user/gps/recordings/2024-01-16_evening_walk.gtd",
    ] {
        let file = build_file(name, &points, gt_track_builder::FileMeta::default(), vec![]);
        files.push(file, FileHistory::None);
    }
    make_state_from_files(files)
}

#[test]
fn snapshot_shared_prefix_stripped() {
    let mut harness = make_harness(make_state_with_shared_prefix());
    harness.run();
    harness.snapshot("side_panel_shared_prefix");
}

fn make_state_with_long_name() -> State {
    let points = gt_test_utils::nav_test_data();
    let mut files = LoadedFiles::new();
    let name = "this_is_an_extremely_long_recording_filename_that_should_be_truncated_at_the_available_panel_width.gtd";
    files.push(
        build_file(name, &points, gt_track_builder::FileMeta::default(), vec![]),
        FileHistory::None,
    );
    make_state_from_files(files)
}

/// Two recordings carrying SDK title/device metadata, for exercising the
/// recording-name template.
fn make_state_with_metadata() -> State {
    let points = gt_test_utils::nav_test_data();
    let mut files = LoadedFiles::new();
    for (name, title, device, notes) in [
        (
            "ride_0.gtd",
            "Morning ride",
            "uBlox F9P",
            "cross-town commute",
        ),
        ("ride_1.gtd", "Evening walk", "uBlox F9P", "along the river"),
    ] {
        let file = build_file(
            name,
            &points,
            gt_track_builder::FileMeta {
                title: Some(title.to_owned()),
                device: Some(device.to_owned()),
                notes: Some(notes.to_owned()),
                travel_mode: None,
            },
            vec![],
        );
        let meta = gt_history_types::RecordingMeta {
            time_range: None,
            nav_point_count: 0,
            sat_report_count: 0,
            marker_count: 0,
            event_marker_count: 0,
            gtd_size_bytes: 0,
        };
        let identity = format!("auto:{title}::{device}");
        files.push(file, FileHistory::recording(identity, meta, None));
    }
    let mut state = make_state_from_files(files);
    state.recording_name_template = "{title} — {device}".to_owned();
    state
}

#[test]
fn snapshot_recording_name_template() {
    // A non-default template renders "{title} — {device}" for each row instead
    // of the filename, and recordings with metadata show the details note icon
    // between the checkbox and the name.
    let mut harness = make_harness(make_state_with_metadata());
    harness.run();
    harness.snapshot("side_panel_name_template");
}

#[test]
fn snapshot_metadata_detail_rows_content() {
    // Directly exercise the grid renderer used by the recording-details dialog,
    // independent of the note-icon click that opens it.
    let mut h = TestHarness::builder()
        .size(egui::vec2(480.0, 150.0))
        .ui(move |ui| {
            ui.add_space(4.0);
            gt_side_panel::widgets::metadata_detail_rows(
                ui,
                &gt_side_panel::widgets::MetadataView {
                    title: Some("Morning ride"),
                    device: Some("uBlox F9P"),
                    travel_mode: Some("Bicycle"),
                    identity: Some("auto:Morning ride::uBlox F9P"),
                    notes: Some("cross-town commute"),
                },
            );
        });
    h.run();
    h.snapshot("metadata_detail_rows_content");
}

/// One drag covers several values: selection runs across labels, and the
/// captions between them stay out of what it copies.
#[test]
fn a_drag_down_the_metadata_grid_copies_every_value_it_covered() {
    let mut h = TestHarness::builder()
        .size(egui::vec2(480.0, 150.0))
        .ui(move |ui| {
            gt_side_panel::widgets::metadata_detail_rows(
                ui,
                &gt_side_panel::widgets::MetadataView {
                    device: Some("uBlox F9P"),
                    notes: Some("cross-town commute"),
                    ..gt_side_panel::widgets::MetadataView::default()
                },
            );
        });
    h.run();

    let device = h.inner.get_by_label("uBlox F9P").rect();
    let notes = h.inner.get_by_label("cross-town commute").rect();
    let from = device.left_center() + egui::vec2(2.0, 0.0);
    let to = notes.right_center() - egui::vec2(2.0, 0.0);
    h.inner.press_drag_release(from, to - from, 4);
    h.inner.input_mut().events.push(egui::Event::Copy);
    h.inner.step();

    let copied = h
        .inner
        .output()
        .platform_output
        .commands
        .iter()
        .find_map(|command| match command {
            egui::OutputCommand::CopyText(text) => Some(text.clone()),
            egui::OutputCommand::OpenUrl(_) | egui::OutputCommand::CopyImage(_) => None,
        })
        .expect("the drag selected text to copy");

    // Blank lines between the values vary with the grid's row spacing: egui
    // spaces copied galleys by how far apart they sat.
    let copied_values: Vec<&str> = copied.lines().filter(|line| !line.is_empty()).collect();
    assert_eq!(copied_values, ["uBlox F9P", "cross-town commute"]);
}

/// The values are the recorder's own strings, which a reader copies out of the
/// details dialog. The captions naming them are not.
#[rstest::rstest]
#[case::caption("Device", egui::CursorIcon::Default)]
#[case::value("uBlox F9P", egui::CursorIcon::Text)]
fn the_metadata_values_select_and_their_captions_do_not(
    #[case] label: &str,
    #[case] expected: egui::CursorIcon,
) {
    let mut h = TestHarness::builder()
        .size(egui::vec2(480.0, 150.0))
        .ui(move |ui| {
            gt_side_panel::widgets::metadata_detail_rows(
                ui,
                &gt_side_panel::widgets::MetadataView {
                    device: Some("uBlox F9P"),
                    ..gt_side_panel::widgets::MetadataView::default()
                },
            );
        });
    h.run();

    let row = h.inner.get_by_label(label).rect().center();
    h.inner.hover_at_and_settle(row, 3);

    assert_eq!(
        h.inner.output().platform_output.cursor_icon,
        expected,
        "hovering {label:?} should request {expected:?}"
    );
}

#[test]
fn clicking_note_icon_requests_recording_details() {
    // Clicking the note icon must populate `metadata_request` with the file's
    // metadata and identity. One file, so the NOTE glyph is unambiguous.
    let points = gt_test_utils::nav_test_data();
    let mut files = LoadedFiles::new();
    let file = build_file(
        "ride.gtd",
        &points,
        gt_track_builder::FileMeta {
            title: Some("Morning ride".to_owned()),
            device: Some("uBlox F9P".to_owned()),
            notes: None,
            travel_mode: None,
        },
        vec![],
    );
    let meta = gt_history_types::RecordingMeta {
        time_range: None,
        nav_point_count: 0,
        sat_report_count: 0,
        marker_count: 0,
        event_marker_count: 0,
        gtd_size_bytes: 0,
    };
    files.push(
        file,
        FileHistory::recording("auto:Morning ride::uBlox F9P".to_owned(), meta, None),
    );
    let mut harness = make_harness(make_state_from_files(files));
    harness.run();
    assert!(harness.state().metadata_request.is_none());
    harness.inner.get_by_label(ICON_NOTE).click();
    harness.run();
    let request = harness
        .state()
        .metadata_request
        .as_ref()
        .expect("clicking the note icon sets the details request");
    assert_eq!(request.metadata.title.as_deref(), Some("Morning ride"));
    assert_eq!(
        request.identity.as_deref(),
        Some("auto:Morning ride::uBlox F9P")
    );
}

#[test]
fn the_warning_icon_shows_the_pointing_hand_and_requests_the_files_warnings() {
    let warnings = [LoadWarning {
        count: 3,
        issue: "satellite(s) with PRN 0".to_owned(),
        description: "PRN 0 is reserved and undefined in NMEA".to_owned(),
    }];
    // One file, so the WARNING glyph is unambiguous.
    let mut harness = make_harness(make_state_with_warnings_on(1, 0, &warnings));
    harness.run();

    let icon = harness.inner.get_by_label(ICON_WARNING).rect().center();
    harness.inner.hover_at_and_settle(icon, 3);
    assert_eq!(
        harness.inner.output().platform_output.cursor_icon,
        egui::CursorIcon::PointingHand
    );

    harness.inner.get_by_label(ICON_WARNING).click();
    harness.run();
    let (filename, requested) = harness
        .state()
        .warnings_request
        .as_ref()
        .expect("clicking the warning icon sets the warnings request");
    assert_eq!(filename, "ride_0.gtd");
    assert_eq!(
        requested
            .iter()
            .map(|w| w.issue.as_str())
            .collect::<Vec<_>>(),
        ["satellite(s) with PRN 0"]
    );
}

/// Render `show_side_panel` inside a resizable [`egui::Panel::left`], the same
/// container the real app uses (`src/app.rs`), in a window wide enough that the
/// panel *could* grow, then return the panel's settled width. A resizable panel
/// grows to fit any child that reports a width wider than the panel.
fn settled_docked_panel_width(state: State) -> f32 {
    let width = std::rc::Rc::new(std::cell::Cell::new(-1.0_f32));
    let width_probe = std::rc::Rc::clone(&width);
    let mut harness = TestHarness::builder()
        .size(egui::vec2(1200.0, 600.0))
        .ui_state(
            move |ui, s: &mut State| {
                CentralPanel::default().show(ui, |ui| {
                    let resp =
                        egui::Panel::left("track_data_panel")
                            .min_size(240.0)
                            .show(ui, |ui| {
                                let mut ctx = PanelContext {
                                    loaded_files: s.files.view(),
                                    tree: &mut s.tree,
                                    highlight: &mut s.highlight,
                                    filter: &mut s.filter,
                                    filter_state: &mut s.filter_state,
                                    map_center_request: &mut s.map_center,
                                    popup_pos_request: &mut s.popup_pos,
                                    query_matches: None,
                                    zoom_to_visible_request: &mut s.zoom_to_visible,
                                    warnings_request: &mut s.warnings_request,
                                    clear_query_request: &mut s.clear_query_request,
                                    display_mask: s.display_mask,
                                    recording_names: &RecordingNames::resolve(
                                        s.files.view(),
                                        &s.recording_name_template,
                                    ),
                                    metadata_request: &mut s.metadata_request,
                                    snap: SnapPanelView {
                                        offline: s.snap_offline,
                                        consent_pending: s.snap_consent_pending,
                                        rows: &s.snap_rows,
                                        costing_choices: &s.snap_costing_choices,
                                        progress: &s.snap_progress,
                                    },
                                    snap_request: &mut s.snap_request,
                                    snap_visibility_request: &mut s.snap_visibility_request,
                                    snap_costing_request: &mut s.snap_costing_request,
                                    sky_trails_request: &mut s.sky_trails_request,
                                };
                                show_side_panel(ui, &mut ctx);
                            });
                    width_probe.set(resp.response.rect.width());
                });
            },
            state,
        );
    // A resizable panel persists its width across frames, so let it settle.
    for _ in 0..6 {
        harness.run();
    }
    width.get()
}

/// A long recording name must not widen the side panel: the file label truncates
/// at the available width (CHANGELOG 0.5.1).
#[test]
fn long_filename_does_not_widen_panel() {
    let short = settled_docked_panel_width(make_state(1));
    let long = settled_docked_panel_width(make_state_with_long_name());
    assert!(
        (long - short).abs() < 1.0,
        "long filename widened the panel: short={short}px, long={long}px"
    );
}

/// Whether the plot cursor sits close enough to the point it hovers for the
/// cross-highlight to activate.
struct PlotHoverSnapped(bool);

/// The rows that repainted when the plot reported a hover on the first track of
/// `ride_0.gtd`, with `ride_1.gtd` as the untouched control.
#[derive(Debug, PartialEq, Eq)]
struct RepaintedRows {
    hovered_file: bool,
    hovered_track: bool,
    other_file: bool,
}

#[expect(
    clippy::expect_used,
    reason = "a harness that cannot render is a fatal test setup failure"
)]
fn rows_repainted_by_plot_hover(PlotHoverSnapped(snapped): PlotHoverSnapped) -> RepaintedRows {
    let mut state = make_state(2);
    state.tree.toggle_expand_file(FileIdx::new(0));
    let mut harness = make_harness(state);
    harness.run();

    let pixels_per_point = harness.inner.ctx.pixels_per_point();
    let hovered_file = tree_row(&harness, "ride_0").rect();
    let hovered_track = tree_row(&harness, "#1  4.6 km").rect();
    let other_file = tree_row(&harness, "ride_1").rect();
    let before = harness.inner.render().expect("the harness renders a frame");

    let highlight = &mut harness.state_mut().highlight;
    highlight.plot_hover_point = Some((FileIdx::new(0), TrackIdx::new(0), PointIdx::new(0)));
    highlight.plot_hover_snapped = snapped;
    harness.inner.run_steps(2);

    let after = harness.inner.render().expect("the harness renders a frame");
    let repainted = |rect| {
        gt_test_utils::snapshot_harness::pixels_differ(&before, &after, rect, pixels_per_point)
    };
    RepaintedRows {
        hovered_file: repainted(hovered_file),
        hovered_track: repainted(hovered_track),
        other_file: repainted(other_file),
    }
}

/// A plot hover marks the same rows a map hover on that point marks: the track
/// row and the row of the recording it belongs to.
#[test]
fn a_snapped_plot_hover_marks_its_track_row_and_its_recording_row() {
    assert_eq!(
        rows_repainted_by_plot_hover(PlotHoverSnapped(true)),
        RepaintedRows {
            hovered_file: true,
            hovered_track: true,
            other_file: false,
        }
    );
}

/// The cursor crossing into the plot area without reaching any data point marks
/// nothing.
#[test]
fn a_plot_hover_that_has_not_snapped_marks_no_row() {
    assert_eq!(
        rows_repainted_by_plot_hover(PlotHoverSnapped(false)),
        RepaintedRows {
            hovered_file: false,
            hovered_track: false,
            other_file: false,
        }
    );
}

/// The time range filter's bar and its start and end labels, for a recording
/// of ten fixes at 10 Hz spanning 900 ms.
#[test]
fn the_time_range_filter_covers_a_recording_shorter_than_a_second() {
    let start = chrono::NaiveDate::from_ymd_opt(2026, 1, 1)
        .and_then(|d| d.and_hms_opt(12, 0, 0))
        .expect("valid date")
        .and_utc();
    let points = gt_test_utils::fixtures::nav_points_from_specs(start, 10, 100, |_| {
        gt_test_utils::fixtures::NavPointSpec::default()
    });
    let mut files = LoadedFiles::new();
    files.push(
        build_file(
            "sprint.gtd",
            &points,
            gt_track_builder::FileMeta::default(),
            vec![],
        ),
        FileHistory::None,
    );
    let mut harness = make_harness(make_state_from_files(files));
    harness.run();

    assert!(
        harness
            .inner
            .query_by_label("01/01 12:00 — 01/01 12:00")
            .is_some(),
        "the time range bar must be drawn for a recording spanning under a second"
    );
}

/// A state whose tree shows one track row with its trailing controls: one
/// recording of one track, its file expanded.
fn state_with_an_expanded_recording(points: &[NavPoint]) -> State {
    let mut files = LoadedFiles::new();
    files.push(
        build_file(
            "ride.gtd",
            points,
            gt_track_builder::FileMeta::default(),
            vec![],
        ),
        FileHistory::None,
    );
    let mut state = make_state_from_files(files);
    state.tree.toggle_expand_file(FileIdx::new(0));
    state
}

/// The track row's snap trigger while its run is in flight: disabled, with
/// the progress hover.
fn state_with_a_snap_run_in_flight() -> State {
    let mut state = state_with_an_expanded_recording(&gt_test_utils::nav_test_data());
    state.snap_rows.insert(
        first_track(),
        SnapRowView::InFlight {
            completed_chunks: 2,
            total_chunks: 5,
        },
    );
    state
}

/// The track row's status glyph alone: a completed run, its snapped track
/// drawn on the map.
fn state_with_a_completed_snap_run() -> State {
    let mut state = state_with_an_expanded_recording(&gt_test_utils::nav_test_data());
    state.snap_rows.insert(first_track(), done_row(true));
    state
}

/// The track row's coordinate warning icon: no fix of the recording holds a
/// position.
fn state_with_a_track_without_a_valid_position() -> State {
    state_with_an_expanded_recording(&gt_test_utils::nav_points_without_a_valid_position(3))
}

/// The track row's eye-slash hint: a completed run whose snapped track is
/// hidden by the map display toggles.
fn state_with_a_hidden_snapped_track() -> State {
    let mut state = state_with_an_expanded_recording(&gt_test_utils::nav_test_data());
    state.snap_rows.insert(first_track(), done_row(true));
    state
        .display_mask
        .set_visible(DisplayCategory::SnappedTracks, false);
    state
}

/// The panel over `state` in a bounded number of steps: a row with a run in
/// flight spins forever and run-until-idle never settles.
fn make_harness_in_bounded_steps(state: State) -> TestHarness<'static, State> {
    let mut harness = make_harness(state);
    harness.inner.run_steps(3);
    harness
}

/// The first line of a track row's stats tooltip for a recording of
/// [`gt_test_utils::nav_test_data`].
const TRACK_STATS_TIME_RANGE: &str = "2026-01-01 12:00:00 – 12:19:59";

/// Frames a hover is held for before its tooltip is read and snapshotted:
/// egui opens a tooltip after its delay, and lays a newly opened one out over
/// the frames that follow.
const TOOLTIP_SETTLE_FRAMES: usize = 5;

/// One digit longer than the duration [`gt_test_utils::nav_test_data`] gives
/// a track ("0:19:59"), which widens the duration column by that digit.
const WIDER_TRACK_DURATION: chrono::TimeDelta = chrono::TimeDelta::seconds(37_199);

/// How many tooltips egui shows: each one is an area of its own in the
/// tooltip layer order.
fn open_tooltip_count(harness: &TestHarness<'static, State>) -> usize {
    harness.inner.ctx.memory(|memory| {
        memory
            .areas()
            .visible_layer_ids()
            .iter()
            .filter(|layer| layer.order == egui::Order::Tooltip)
            .count()
    })
}

/// The three controls of a track row with a tooltip of their own: the snap
/// trigger disabled while its run is in flight, the coordinate warning icon
/// and the eye-slash hint.
#[rstest::rstest]
#[case::disabled_snap_trigger(
    state_with_a_snap_run_in_flight,
    ICON_LINE_SEGMENTS,
    "Snapping - completed 2 of 5 chunks",
    "side_panel_tooltip_over_the_in_flight_trigger"
)]
#[case::coordinate_warning_icon(
    state_with_a_track_without_a_valid_position,
    ICON_WARNING,
    "No fix has a valid coordinate, so the track is not drawn on the map",
    "side_panel_tooltip_over_the_coordinate_warning"
)]
#[case::masked_snapped_track_hint(
    state_with_a_hidden_snapped_track,
    ICON_EYE_SLASH,
    "Hidden by the map display toggles",
    "side_panel_tooltip_over_the_masked_hint"
)]
fn snapshot_hovering_a_track_row_control_opens_that_controls_tooltip_alone(
    #[case] state: fn() -> State,
    #[case] control_label: &str,
    #[case] expected_tooltip: &str,
    #[case] snapshot_name: &str,
) {
    let mut harness = make_harness_in_bounded_steps(state());

    let control = harness.inner.get_by_label(control_label).rect().center();
    harness
        .inner
        .hover_at_and_settle(control, TOOLTIP_SETTLE_FRAMES);

    assert!(
        harness.inner.query_by_label(expected_tooltip).is_some(),
        "hovering {control_label:?} must state {expected_tooltip:?}"
    );
    assert_eq!(
        open_tooltip_count(&harness),
        1,
        "the row's track stats must stay closed while a control's tooltip is open"
    );
    harness.snapshot(snapshot_name);
}

/// The point on the columns a track row paints, midway between its checkbox
/// and its snap trigger, where the row holds no widget of its own.
fn track_row_cells_position(harness: &TestHarness<'static, State>) -> egui::Pos2 {
    let checkbox = harness
        .inner
        .bottommost_matching(By::new().label_contains(ICON_CHECK_SQUARE))
        .rect();
    let trigger = harness.inner.get_by_label(ICON_LINE_SEGMENTS).rect();
    egui::pos2(
        (checkbox.right() + trigger.left()) / 2.0,
        checkbox.center().y,
    )
}

#[test]
fn snapshot_hovering_a_track_rows_cells_opens_the_track_stats_tooltip() {
    let mut harness = make_harness_in_bounded_steps(state_with_a_snap_run_in_flight());

    let cells = track_row_cells_position(&harness);
    harness
        .inner
        .hover_at_and_settle(cells, TOOLTIP_SETTLE_FRAMES);

    assert!(
        harness
            .inner
            .query_by_label(TRACK_STATS_TIME_RANGE)
            .is_some(),
        "the row's tooltip must state the span the track covers"
    );
    assert_eq!(
        open_tooltip_count(&harness),
        1,
        "the snap trigger's tooltip must stay closed while the pointer is off it"
    );
    harness.snapshot("side_panel_tooltip_over_the_row_cells");
}

/// The two places the pointer rests in the handover tests: the columns a
/// track row paints, and the snap trigger it draws after them.
#[derive(Clone, Copy)]
enum RowHoverTarget {
    Cells,
    SnapTrigger,
}

impl RowHoverTarget {
    fn position(self, harness: &TestHarness<'static, State>) -> egui::Pos2 {
        match self {
            Self::Cells => track_row_cells_position(harness),
            Self::SnapTrigger => harness
                .inner
                .get_by_label(ICON_LINE_SEGMENTS)
                .rect()
                .center(),
        }
    }
}

/// The pointer moves from the row's columns onto the in-flight trigger, and
/// back. The frame it arrives on shows neither tooltip: egui allows one
/// tooltip per layer, and reads which widget holds it from the previous
/// frame.
#[rstest::rstest]
#[case::cells_to_trigger(
    RowHoverTarget::Cells,
    RowHoverTarget::SnapTrigger,
    "Snapping - completed 2 of 5 chunks",
    "side_panel_tooltip_handover_cells_to_trigger"
)]
#[case::trigger_to_cells(
    RowHoverTarget::SnapTrigger,
    RowHoverTarget::Cells,
    TRACK_STATS_TIME_RANGE,
    "side_panel_tooltip_handover_trigger_to_cells"
)]
fn snapshot_a_track_row_shows_no_tooltip_on_the_frame_the_pointer_arrives(
    #[case] from: RowHoverTarget,
    #[case] to: RowHoverTarget,
    #[case] expected_tooltip: &str,
    #[case] snapshot_name: &str,
) {
    let mut harness = make_harness_in_bounded_steps(state_with_a_snap_run_in_flight());

    let start = from.position(&harness);
    harness
        .inner
        .hover_at_and_settle(start, TOOLTIP_SETTLE_FRAMES);
    assert_eq!(open_tooltip_count(&harness), 1, "the tooltip hovered first");

    let target = to.position(&harness);
    harness.inner.hover_at(target);
    harness.inner.step();
    assert_eq!(
        open_tooltip_count(&harness),
        0,
        "the frame the pointer arrives on"
    );
    harness.snapshot(&format!("{snapshot_name}_arrival_frame"));

    harness.inner.run_steps(TOOLTIP_SETTLE_FRAMES);
    assert_eq!(open_tooltip_count(&harness), 1, "the settled frame");
    assert!(
        harness.inner.query_by_label(expected_tooltip).is_some(),
        "the settled tooltip must state {expected_tooltip:?}"
    );
    harness.snapshot(&format!("{snapshot_name}_settled"));
}

/// The tooltip counts over the three frames after `change` is applied to the
/// state under a resting pointer.
fn tooltip_counts_over_three_frames(
    harness: &mut TestHarness<'static, State>,
    change: impl FnOnce(&mut State),
) -> [usize; 3] {
    change(harness.state_mut());
    std::array::from_fn(|_| {
        harness.inner.step();
        open_tooltip_count(harness)
    })
}

/// The status glyph of a completed run takes the trigger's place in the row,
/// and its widget id with it. The one-tooltip-per-layer rule reads that id
/// from the previous frame and allows the glyph's tooltip on the first frame
/// the glyph is drawn.
#[test]
fn snapshot_the_status_glyph_opens_its_tooltip_at_once_when_the_run_completes_under_the_pointer() {
    let mut harness = make_harness_in_bounded_steps(state_with_a_snap_run_in_flight());

    let trigger = RowHoverTarget::SnapTrigger.position(&harness);
    harness
        .inner
        .hover_at_and_settle(trigger, TOOLTIP_SETTLE_FRAMES);
    assert_eq!(open_tooltip_count(&harness), 1, "the trigger's tooltip");

    let counts = tooltip_counts_over_three_frames(&mut harness, |state| {
        state.snap_rows.insert(first_track(), done_row(true));
    });

    assert_eq!(counts, [1, 1, 1]);
    assert!(
        harness.inner.query_by_label("Snapped to road").is_some(),
        "the status glyph's tooltip heads the completed run's breakdown"
    );
    harness.snapshot("side_panel_tooltip_after_the_run_completed");
}

/// The trigger's tooltip continues showing the progress text while the
/// pointer rests on it: the trigger keeps its widget id when its run starts.
#[test]
fn snapshot_the_trigger_tooltip_continues_when_its_run_starts_under_the_pointer() {
    let mut harness = make_harness_in_bounded_steps(state_with_an_expanded_recording(
        &gt_test_utils::nav_test_data(),
    ));

    let trigger = RowHoverTarget::SnapTrigger.position(&harness);
    harness
        .inner
        .hover_at_and_settle(trigger, TOOLTIP_SETTLE_FRAMES);
    assert!(
        harness
            .inner
            .query_by_label(
                "Snap to road - match this track against the OpenStreetMap road network"
            )
            .is_some(),
        "the idle trigger's tooltip"
    );

    let counts = tooltip_counts_over_three_frames(&mut harness, |state| {
        state.snap_rows.insert(
            first_track(),
            SnapRowView::InFlight {
                completed_chunks: 2,
                total_chunks: 5,
            },
        );
    });

    assert_eq!(counts, [1, 1, 1]);
    assert!(
        harness
            .inner
            .query_by_label("Snapping - completed 2 of 5 chunks")
            .is_some(),
        "the tooltip must state the progress of the run that started"
    );
    harness.snapshot("side_panel_tooltip_after_the_run_started");
}

/// A longer duration widens the duration column, which moves the trailing
/// trigger right, onto a pointer resting beside it.
#[test]
fn snapshot_the_trigger_takes_the_tooltip_when_wider_columns_shift_it_under_the_pointer() {
    let mut harness = make_harness_in_bounded_steps(state_with_a_snap_run_in_flight());

    let trigger = harness.inner.get_by_label(ICON_LINE_SEGMENTS).rect();
    let beside_the_trigger = egui::pos2(trigger.right() + 3.0, trigger.center().y);
    harness
        .inner
        .hover_at_and_settle(beside_the_trigger, TOOLTIP_SETTLE_FRAMES);
    assert!(
        harness
            .inner
            .query_by_label(TRACK_STATS_TIME_RANGE)
            .is_some(),
        "the row's track stats beside the trigger"
    );

    let counts = tooltip_counts_over_three_frames(&mut harness, |state| {
        if let Some(track) = state
            .files
            .get_mut(0)
            .and_then(|file| file.tracks.first_mut())
        {
            track.metadata.duration = WIDER_TRACK_DURATION;
        }
    });

    let shifted = harness.inner.get_by_label(ICON_LINE_SEGMENTS).rect();
    assert!(
        shifted.contains(beside_the_trigger),
        "the wider columns must move the trigger onto the resting pointer"
    );
    assert_eq!(counts, [1, 0, 1]);

    // The trigger's tooltip opened on the last of those frames: hold the
    // pointer while egui lays it out.
    harness.inner.run_steps(TOOLTIP_SETTLE_FRAMES);
    assert_eq!(open_tooltip_count(&harness), 1);
    assert!(
        harness
            .inner
            .query_by_label("Snapping - completed 2 of 5 chunks")
            .is_some(),
        "the trigger the wider columns moved under the pointer"
    );
    harness.snapshot("side_panel_tooltip_after_the_columns_widened");
}

/// The pointer 3 points from a control, over the row itself. egui hits the
/// row directly, since the row is the click target that contains the pointer,
/// and never reaches for the widgets within `interact_radius`.
#[rstest::rstest]
#[case::beside_an_enabled_status_glyph(
    state_with_a_completed_snap_run,
    ICON_PATH,
    "side_panel_tooltip_beside_the_status_glyph"
)]
#[case::beside_a_disabled_trigger(
    state_with_a_snap_run_in_flight,
    ICON_LINE_SEGMENTS,
    "side_panel_tooltip_beside_the_in_flight_trigger"
)]
fn snapshot_hovering_the_row_beside_a_control_opens_the_track_stats_tooltip(
    #[case] state: fn() -> State,
    #[case] control_label: &str,
    #[case] snapshot_name: &str,
) {
    let mut harness = make_harness_in_bounded_steps(state());

    let control = harness.inner.get_by_label(control_label).rect();
    let beside_it = egui::pos2(control.left() - 3.0, control.center().y);
    harness
        .inner
        .hover_at_and_settle(beside_it, TOOLTIP_SETTLE_FRAMES);

    assert_eq!(open_tooltip_count(&harness), 1);
    assert!(
        harness
            .inner
            .query_by_label(TRACK_STATS_TIME_RANGE)
            .is_some(),
        "hovering 3 points left of {control_label:?} must state the track stats"
    );
    harness.snapshot(snapshot_name);
}
