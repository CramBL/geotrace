use std::path::PathBuf;

use gt_filter::GlobalFilter;
use gt_loaded_files::{FileHistory, LoadedFiles};
use gt_side_panel::{FilterPanelState, PanelContext, TreeState, show_side_panel};
use gt_test_utils::TestHarness;
use gt_types::{FileIdx, FixStats, LoadWarning, TrackIdx, TrackRef};
use gt_ui_types::MapHighlight;

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
    }
}

fn make_harness(state: State) -> TestHarness<'static, State> {
    TestHarness::builder()
        .size(egui::vec2(280.0, 600.0))
        .ui_state(
            |ui, s: &mut State| {
                let mut ctx = PanelContext {
                    loaded_files: s.files.view(),
                    tree: &mut s.tree,
                    highlight: &mut s.highlight,
                    filter: &mut s.filter,
                    filter_state: &mut s.filter_state,
                    map_center_request: &mut s.map_center,
                    popup_pos_request: &mut s.popup_pos,
                    zoom_to_visible_request: &mut s.zoom_to_visible,
                    warnings_request: &mut s.warnings_request,
                    clear_query_request: &mut s.clear_query_request,
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
        unit: Some("g".to_owned()),
        period: None,
        description: None,
        components: vec!["x".to_owned(), "y".to_owned(), "z".to_owned()],
        times: vec![start, start + chrono::Duration::seconds(1)],
        values: vec![0.1, 0.2, 0.98, -0.1, 0.3, 1.02],
    };
    let incline = gt_types::Channel {
        name: "incline".to_owned(),
        unit: Some("deg".to_owned()),
        period: None,
        description: None,
        components: vec![],
        times: vec![start, start + chrono::Duration::seconds(2)],
        values: vec![1.5, 2.0],
    };
    let file = gt_track_builder::build_loaded_file(
        "sensors.gtd".to_owned(),
        &points,
        &[],
        vec![],
        vec![],
        &[accel, incline],
        &gt_track_builder::SegmentationConfig::default(),
        gt_types::FileSource::GtdPath(PathBuf::from("sensors.gtd")),
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
    };
    let mut harness = make_harness(state);
    harness.run();
    harness.snapshot("side_panel_track_channels");
}
