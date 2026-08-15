#![expect(
    clippy::literal_string_with_formatting_args,
    reason = "recording-name templates intentionally contain {token} placeholders, not format args"
)]

use egui::CentralPanel;
use egui_phosphor::regular::LINE_SEGMENTS as ICON_LINE_SEGMENTS;
use egui_phosphor::regular::NOTE as ICON_NOTE;
use egui_phosphor::regular::PATH as ICON_PATH;
use std::collections::HashMap;
use std::path::PathBuf;

use egui_kittest::kittest::Queryable as _;
use geotrace_sdk_units::Unit;
use gt_filter::GlobalFilter;
use gt_loaded_files::{FileHistory, LoadedFiles, RecordingNames};
use gt_side_panel::{
    FilterPanelState, PanelContext, SnapPanelView, SnapRowView, TreeState, show_side_panel,
};
use gt_test_utils::TestHarness;
use gt_types::{FileIdx, FixStats, LoadWarning, TrackIdx, TrackRef};
use gt_ui_types::{DisplayCategory, DisplayMask, MapHighlight, SnapCosting};

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
    snap_rows: HashMap<TrackRef, SnapRowView>,
    snap_progress: gt_side_panel::SnapProgressView,
    snap_offline: bool,
    snap_consent_pending: bool,
    snap_request: Option<TrackRef>,
    snap_visibility_request: Option<TrackRef>,
    snap_costing_choices: Vec<(SnapCosting, String)>,
    snap_costing_request: Option<(TrackRef, SnapCosting)>,
    sky_trails_request: Option<gt_ui_types::SkyTrailsRequest>,
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
        let file = gt_track_builder::build_loaded_file(
            format!("ride_{i}.gtd"),
            &points,
            &[],
            vec![],
            vec![],
            &[],
            &gt_track_builder::SegmentationConfig::default(),
            gt_types::FileSource::GtdPath(PathBuf::from(format!("ride_{i}.gtd"))),
            gt_track_builder::FileMeta::default(),
            w,
        );
        files.push(file, FileHistory::None);
    }
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
        snap_rows: HashMap::new(),
        snap_progress: gt_side_panel::SnapProgressView::default(),
        snap_offline: false,
        snap_consent_pending: false,
        snap_request: None,
        snap_visibility_request: None,
        snap_costing_choices: vec![
            (SnapCosting::Auto, "Auto".to_owned()),
            (SnapCosting::Bicycle, "Bicycle".to_owned()),
            (SnapCosting::Pedestrian, "Pedestrian".to_owned()),
        ],
        snap_costing_request: None,
        sky_trails_request: None,
    }
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

/// Queued work with the app offline: the strip says the queue is paused
/// and why, instead of sitting silent.
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

    // The status glyph (routed path) and the re-run trigger (raw
    // segments) wear distinct icons now.
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

/// Under `GEOTRACE_OFFLINE` every snap trigger is grayed out, never hidden.
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

    harness
        .inner
        .get_by_label_contains("#1  4.6 km")
        .click_secondary();
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

    harness
        .inner
        .get_by_label_contains("#1  4.6 km")
        .click_secondary();
    harness.run();
    harness.inner.get_by_label_contains("Snap again as").hover();
    harness.inner.run_steps(3);
    harness.inner.get_by_label("Bicycle").click();
    harness.run();

    assert_eq!(
        harness.state().snap_costing_request,
        Some((track, SnapCosting::Bicycle)),
    );
    assert_eq!(harness.state().snap_request, None);
    assert_eq!(harness.state().snap_visibility_request, None);
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
    harness.inner.get_by_label_contains("Snap again as").hover();
    harness.inner.run_steps(3);
    harness.inner.get_by_label("Bicycle").click();
    harness.run();

    assert_eq!(
        harness.state().snap_costing_request,
        Some((track, SnapCosting::Bicycle)),
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

    harness
        .inner
        .get_by_label_contains("#1  4.6 km")
        .click_secondary();
    harness.run();
    harness.inner.get_by_label_contains("Snap as").hover();
    harness.inner.run_steps(3);
    harness.inner.get_by_label("Pedestrian").click();
    harness.run();

    assert_eq!(
        harness.state().snap_costing_request,
        Some((track, SnapCosting::Pedestrian)),
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

    harness
        .inner
        .get_by_label_contains("#1  4.6 km")
        .click_secondary();
    harness.run();
    harness.inner.get_by_label_contains("Snap again as").hover();
    harness.inner.run_steps(3);
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

#[test]
fn track_without_satellite_reports_falls_back_to_no_data_tooltip() {
    let points = gt_test_utils::stationary_nav_data(10);
    let file = gt_track_builder::build_loaded_file(
        "no_sats.gtd".to_owned(),
        &points,
        &[],
        vec![],
        vec![],
        &[],
        &gt_track_builder::SegmentationConfig::default(),
        gt_types::FileSource::GtdPath(PathBuf::from("no_sats.gtd")),
        gt_track_builder::FileMeta::default(),
        vec![],
    );
    assert_eq!(file.tracks.len(), 1);
    assert!(
        file.tracks[0].metadata.fix_stats.is_none(),
        "track with no satellite reports should have fix_stats == None"
    );

    let mut tree = TreeState::new();
    tree.sync_from_loaded_files(std::slice::from_ref(&file));
    tree.toggle_expand_file(FileIdx::new(0));
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
        snap_rows: HashMap::new(),
        snap_progress: gt_side_panel::SnapProgressView::default(),
        snap_offline: false,
        snap_consent_pending: false,
        snap_request: None,
        snap_visibility_request: None,
        snap_costing_choices: Vec::new(),
        snap_costing_request: None,
        sky_trails_request: None,
    };
    // Renders the expanded track row, exercising the `fix_stats == None` fallback
    // ("No satellite data") instead of the colored tooltip.
    let mut harness = make_harness(state);
    harness.run();
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
        snap_rows: HashMap::new(),
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
        let file = gt_track_builder::build_loaded_file(
            name.to_owned(),
            &points,
            &[],
            vec![],
            vec![],
            &[],
            &gt_track_builder::SegmentationConfig::default(),
            gt_types::FileSource::GtdPath(PathBuf::from(name)),
            gt_track_builder::FileMeta::default(),
            vec![],
        );
        files.push(file, FileHistory::None);
    }
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
        snap_rows: HashMap::new(),
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
    let file = gt_track_builder::build_loaded_file(
        name.to_owned(),
        &points,
        &[],
        vec![],
        vec![],
        &[],
        &gt_track_builder::SegmentationConfig::default(),
        gt_types::FileSource::GtdPath(PathBuf::from(name)),
        gt_track_builder::FileMeta::default(),
        vec![],
    );
    files.push(file, FileHistory::None);
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
        snap_rows: HashMap::new(),
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
        let file = gt_track_builder::build_loaded_file(
            name.to_owned(),
            &points,
            &[],
            vec![],
            vec![],
            &[],
            &gt_track_builder::SegmentationConfig::default(),
            gt_types::FileSource::GtdPath(PathBuf::from(name)),
            gt_track_builder::FileMeta {
                title: Some(title.to_owned()),
                device: Some(device.to_owned()),
                notes: Some(notes.to_owned()),
                travel_mode: None,
            },
            vec![],
        );
        let meta = gt_history_types::RecordingMeta {
            start_us: 0,
            end_us: 0,
            nav_point_count: 0,
            sat_report_count: 0,
            marker_count: 0,
            event_marker_count: 0,
            gtd_size_bytes: 0,
        };
        let identity = format!("auto:{title}::{device}");
        files.push(file, FileHistory::recording(identity, meta, None));
    }
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
        recording_name_template: "{title} — {device}".to_owned(),
        metadata_request: None,
        snap_rows: HashMap::new(),
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

#[test]
fn clicking_note_icon_requests_recording_details() {
    // The note icon's click is the whole point of the feature: it must populate
    // `metadata_request` with the file's metadata and identity for the app to
    // open the details dialog. One file, so the NOTE glyph is unambiguous.
    let points = gt_test_utils::nav_test_data();
    let mut files = LoadedFiles::new();
    let file = gt_track_builder::build_loaded_file(
        "ride.gtd".to_owned(),
        &points,
        &[],
        vec![],
        vec![],
        &[],
        &gt_track_builder::SegmentationConfig::default(),
        gt_types::FileSource::GtdPath(PathBuf::from("ride.gtd")),
        gt_track_builder::FileMeta {
            title: Some("Morning ride".to_owned()),
            device: Some("uBlox F9P".to_owned()),
            notes: None,
            travel_mode: None,
        },
        vec![],
    );
    let meta = gt_history_types::RecordingMeta {
        start_us: 0,
        end_us: 0,
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
    let mut tree = TreeState::new();
    tree.sync_from_loaded_files(files.files());
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
        snap_rows: HashMap::new(),
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

/// Render `show_side_panel` inside a resizable [`egui::Panel::left`] - the same
/// container the real app uses (`src/app.rs`) - in a window wide enough that the
/// panel *could* grow, then return the panel's settled width. A resizable panel
/// grows to fit any child that reports a width wider than the panel, so a file
/// label that requests its full natural text width (the pre-fix behaviour) drags
/// the whole panel wider.
fn settled_docked_panel_width(state: State) -> f32 {
    let width = std::rc::Rc::new(std::cell::Cell::new(-1.0_f32));
    let width_probe = std::rc::Rc::clone(&width);
    let mut harness = gt_test_utils::TestHarness::builder()
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
/// at the available width instead of forcing the panel to grow (CHANGELOG 0.5.1).
/// Before the `Button::selectable(..).truncate()` fix the label requested its full
/// natural text width, so this same panel settled ~500px wider for the long name.
#[test]
fn long_filename_does_not_widen_panel() {
    let short = settled_docked_panel_width(make_state(1));
    let long = settled_docked_panel_width(make_state_with_long_name());
    assert!(
        (long - short).abs() < 1.0,
        "long filename widened the panel: short={short}px, long={long}px"
    );
}
