use egui_kittest::Harness;
use gt_filter::GlobalFilter;
use gt_side_panel::{FilterPanelState, PanelContext, TreeState, show_side_panel};
use gt_test_utils::TestHarness;
use gt_types::{FileIdx, TrackIdx, TrackRef};
use gt_ui_types::MapHighlight;

struct State {
    files: Vec<gt_types::LoadedFile>,
    tree: TreeState,
    filter: GlobalFilter,
    filter_state: FilterPanelState,
    highlight: MapHighlight,
    map_center: Option<(f64, f64)>,
    popup_pos: Option<egui::Pos2>,
    zoom_to_visible: bool,
    warnings_request: Option<(String, Vec<String>)>,
    unload_request: Option<FileIdx>,
}

fn make_state(file_count: usize) -> State {
    make_state_with_warnings_on(file_count, 0, &[])
}

fn make_state_with_warnings_on(
    file_count: usize,
    warned_file: usize,
    warnings: &[String],
) -> State {
    let points = gt_test_utils::nav_test_data();
    let files = (0..file_count)
        .map(|i| {
            let w = if i == warned_file {
                warnings.to_vec()
            } else {
                vec![]
            };
            gt_track_builder::build_loaded_file(
                format!("ride_{i}.gtd"),
                format!("auto:ride_{i}.gtd"),
                &points,
                &[],
                vec![],
                vec![],
                &gt_track_builder::SegmentationConfig::default(),
                gt_types::FileSource::GtdPath(std::path::PathBuf::from(format!("ride_{i}.gtd"))),
                w,
            )
        })
        .collect();
    let mut tree = TreeState::new();
    let files_ref: &Vec<gt_types::LoadedFile> = &files;
    tree.sync_from_loaded_files(files_ref);
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
        unload_request: None,
    }
}

fn make_harness(state: State) -> Harness<'static, State> {
    Harness::builder()
        .with_size(egui::vec2(280.0, 600.0))
        .wgpu()
        .build_ui_state(
            |ui, s: &mut State| {
                let mut ctx = PanelContext {
                    files: &s.files,
                    tree: &mut s.tree,
                    highlight: &mut s.highlight,
                    filter: &mut s.filter,
                    filter_state: &mut s.filter_state,
                    map_center_request: &mut s.map_center,
                    popup_pos_request: &mut s.popup_pos,
                    zoom_to_visible_request: &mut s.zoom_to_visible,
                    warnings_request: &mut s.warnings_request,
                    unload_request: &mut s.unload_request,
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
    TestHarness::from_harness(harness).snapshot("side_panel_collapsed");
}

#[test]
fn snapshot_one_file_expanded() {
    let mut state = make_state(2);
    state.tree.toggle_expand_file(FileIdx::new(0));
    let mut harness = make_harness(state);
    harness.run();
    TestHarness::from_harness(harness).snapshot("side_panel_file_expanded");
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
fn snapshot_file_with_warnings() {
    let warnings = [
        "3 satellite(s) with PRN 0 - PRN 0 is reserved and undefined in NMEA".to_owned(),
        "2 satellite(s) with elevation > 90° - above the zenith, outside the valid NMEA range [0°, 90°]".to_owned(),
    ];
    let state = make_state_with_warnings_on(2, 0, &warnings);
    let mut harness = make_harness(state);
    harness.run();
    TestHarness::from_harness(harness).snapshot("side_panel_file_with_warnings");
}
