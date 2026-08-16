use gt_side_panel::tree::{CheckState, NodeKey, TreeState};
use gt_types::{DataCategory, DataCategorySet, FileIdx, TrackIdx, TrackRef};

fn make_tree(file_count: usize, tracks_per_file: usize) -> TreeState {
    let mut tree = TreeState::new();
    for _ in 0..file_count {
        let file_node = gt_side_panel::FileNode {
            expanded: false,
            check: CheckState::On,
            tracks: (0..tracks_per_file).map(|_| make_track_node()).collect(),
        };
        tree.files.push(file_node);
    }
    tree
}

fn make_track_node() -> gt_side_panel::TrackNode {
    gt_side_panel::TrackNode {
        expanded: false,
        check: CheckState::On,
        categories_expanded: DataCategorySet::default(),
        track_visible: true,
        tpv_visible: true,
        satellites_visible: true,
        custom_markers_visible: true,
        generated_markers_visible: true,
        generated_kinds_expanded: Default::default(),
        generated_kinds_hidden: Default::default(),
        event_paths: Default::default(),
        event_filter: String::new(),
        channels_expanded: false,
    }
}

fn add_event_paths(tree: &mut TreeState, fi: usize, ti: usize, paths: &[&str]) {
    if let Some(track_node) = tree.files.get_mut(fi).and_then(|f| f.tracks.get_mut(ti)) {
        track_node
            .event_paths
            .sync_from_paths(paths.iter().copied());
    }
}

fn track_check(tree: &TreeState, fi: usize, ti: usize) -> CheckState {
    tree.files
        .get(fi)
        .and_then(|f| f.tracks.get(ti))
        .map_or(CheckState::Off, |t| t.check)
}

fn file_check(tree: &TreeState, fi: usize) -> CheckState {
    tree.files.get(fi).map_or(CheckState::Off, |f| f.check)
}

fn event_path_check(tree: &TreeState, fi: usize, ti: usize, path: &str) -> CheckState {
    tree.files
        .get(fi)
        .and_then(|f| f.tracks.get(ti))
        .and_then(|t| t.event_paths.nodes.get(path).copied())
        .unwrap_or(CheckState::On)
}

#[test]
fn toggle_check_file_on_off() {
    let mut tree = make_tree(1, 3);
    tree.toggle_file_check(FileIdx::new(0));
    assert_eq!(file_check(&tree, 0), CheckState::Off);
    assert_eq!(track_check(&tree, 0, 0), CheckState::Off);
    assert_eq!(track_check(&tree, 0, 1), CheckState::Off);
    assert_eq!(track_check(&tree, 0, 2), CheckState::Off);
}

#[test]
fn toggle_check_file_off_to_on() {
    let mut tree = make_tree(1, 2);
    tree.toggle_file_check(FileIdx::new(0)); // → Off
    tree.toggle_file_check(FileIdx::new(0)); // → On
    assert_eq!(file_check(&tree, 0), CheckState::On);
    assert_eq!(track_check(&tree, 0, 0), CheckState::On);
    assert_eq!(track_check(&tree, 0, 1), CheckState::On);
}

#[test]
fn toggle_check_track_partial_makes_file_mixed() {
    let mut tree = make_tree(1, 2);
    tree.toggle_track_check(TrackRef::new(FileIdx::new(0), TrackIdx::new(1))); // track[1] → Off, track[0] stays On
    assert_eq!(track_check(&tree, 0, 0), CheckState::On);
    assert_eq!(track_check(&tree, 0, 1), CheckState::Off);
    assert_eq!(file_check(&tree, 0), CheckState::Mixed);
}

#[test]
fn toggle_check_file_mixed_goes_on() {
    let mut tree = make_tree(1, 2);
    tree.toggle_track_check(TrackRef::new(FileIdx::new(0), TrackIdx::new(1))); // file → Mixed
    assert_eq!(file_check(&tree, 0), CheckState::Mixed);
    tree.toggle_file_check(FileIdx::new(0)); // Mixed → On, all children On
    assert_eq!(file_check(&tree, 0), CheckState::On);
    assert_eq!(track_check(&tree, 0, 0), CheckState::On);
    assert_eq!(track_check(&tree, 0, 1), CheckState::On);
}

#[test]
fn toggle_check_track_enables_parent_file() {
    let mut tree = make_tree(1, 2);
    tree.toggle_file_check(FileIdx::new(0)); // all Off
    tree.toggle_track_check(TrackRef::new(FileIdx::new(0), TrackIdx::new(0))); // track[0] → On
    assert_eq!(track_check(&tree, 0, 0), CheckState::On);
    assert_eq!(track_check(&tree, 0, 1), CheckState::Off);
    assert_eq!(file_check(&tree, 0), CheckState::Mixed);
}

#[test]
fn event_path_toggle_parent_cascades() {
    let mut tree = make_tree(1, 1);
    add_event_paths(&mut tree, 0, 0, &["power/boot", "power/sleep"]);
    // All nodes start On
    assert_eq!(event_path_check(&tree, 0, 0, "power"), CheckState::On);
    assert_eq!(event_path_check(&tree, 0, 0, "power/boot"), CheckState::On);

    tree.toggle_event_path(TrackRef::new(FileIdx::new(0), TrackIdx::new(0)), "power"); // parent Off → all descendants Off
    assert_eq!(event_path_check(&tree, 0, 0, "power"), CheckState::Off);
    assert_eq!(event_path_check(&tree, 0, 0, "power/boot"), CheckState::Off);
    assert_eq!(
        event_path_check(&tree, 0, 0, "power/sleep"),
        CheckState::Off
    );

    tree.toggle_event_path(TrackRef::new(FileIdx::new(0), TrackIdx::new(0)), "power"); // parent Off → On
    assert_eq!(event_path_check(&tree, 0, 0, "power"), CheckState::On);
    assert_eq!(event_path_check(&tree, 0, 0, "power/boot"), CheckState::On);
    assert_eq!(event_path_check(&tree, 0, 0, "power/sleep"), CheckState::On);
}

#[test]
fn event_path_toggle_leaf_recomputes_parent() {
    let mut tree = make_tree(1, 1);
    add_event_paths(&mut tree, 0, 0, &["power/boot", "power/sleep"]);

    tree.toggle_event_path(
        TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        "power/boot",
    ); // boot → Off
    assert_eq!(event_path_check(&tree, 0, 0, "power/boot"), CheckState::Off);
    assert_eq!(event_path_check(&tree, 0, 0, "power/sleep"), CheckState::On);
    assert_eq!(event_path_check(&tree, 0, 0, "power"), CheckState::Mixed);
}

#[test]
fn event_path_grandparent_recomputes() {
    let mut tree = make_tree(1, 1);
    add_event_paths(&mut tree, 0, 0, &["a/b/c", "a/b/d"]);

    tree.toggle_event_path(TrackRef::new(FileIdx::new(0), TrackIdx::new(0)), "a/b/c"); // → Off
    assert_eq!(event_path_check(&tree, 0, 0, "a/b/c"), CheckState::Off);
    assert_eq!(event_path_check(&tree, 0, 0, "a/b/d"), CheckState::On);
    assert_eq!(event_path_check(&tree, 0, 0, "a/b"), CheckState::Mixed);
    assert_eq!(event_path_check(&tree, 0, 0, "a"), CheckState::Mixed);
}

#[test]
fn toggle_category_expanded_flips_independently_and_restores() {
    let mut tree = make_tree(1, 1);
    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    let expanded = |tree: &TreeState, cat| {
        tree.track_node(track)
            .expect("track node")
            .categories_expanded
            .contains(cat)
    };

    // Sections start collapsed.
    assert!(!expanded(&tree, DataCategory::Tpv));

    tree.toggle_category_expanded(track, DataCategory::Tpv);
    assert!(expanded(&tree, DataCategory::Tpv));

    // A second category toggles without disturbing the first.
    tree.toggle_category_expanded(track, DataCategory::EventMarker);
    assert!(expanded(&tree, DataCategory::EventMarker));
    assert!(expanded(&tree, DataCategory::Tpv));

    // Toggling back restores the collapsed state.
    tree.toggle_category_expanded(track, DataCategory::Tpv);
    assert!(!expanded(&tree, DataCategory::Tpv));
    assert!(expanded(&tree, DataCategory::EventMarker));
}

#[test]
fn toggle_expand_file() {
    let mut tree = make_tree(1, 1);
    assert!(!tree.files[0].expanded);
    tree.toggle_expand_file(FileIdx::new(0));
    assert!(tree.files[0].expanded);
    tree.toggle_expand_file(FileIdx::new(0));
    assert!(!tree.files[0].expanded);
}

#[test]
fn toggle_channels_expanded_flips_the_flag() {
    let mut tree = make_tree(1, 1);
    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    assert!(
        !tree
            .track_node(track)
            .expect("track node")
            .channels_expanded
    );
    tree.toggle_channels_expanded(track);
    assert!(
        tree.track_node(track)
            .expect("track node")
            .channels_expanded
    );
    tree.toggle_channels_expanded(track);
    assert!(
        !tree
            .track_node(track)
            .expect("track node")
            .channels_expanded
    );
}

#[test]
fn apply_click_single_clears_previous_selection() {
    let mut tree = make_tree(1, 2);
    tree.files[0].expanded = true;
    let fi = FileIdx::new(0);
    let track0 = NodeKey::Track(TrackRef::new(fi, TrackIdx::new(0)));
    let track1 = NodeKey::Track(TrackRef::new(fi, TrackIdx::new(1)));
    tree.apply_click(track0, false, false);
    assert!(tree.selection.contains(&track0));
    tree.apply_click(track1, false, false);
    assert!(!tree.selection.contains(&track0));
    assert!(tree.selection.contains(&track1));
}

#[test]
fn apply_click_ctrl_adds_to_selection() {
    let mut tree = make_tree(1, 2);
    tree.files[0].expanded = true;
    let fi = FileIdx::new(0);
    let track0 = NodeKey::Track(TrackRef::new(fi, TrackIdx::new(0)));
    let track1 = NodeKey::Track(TrackRef::new(fi, TrackIdx::new(1)));
    tree.apply_click(track0, false, false);
    tree.apply_click(track1, true, false);
    assert!(tree.selection.contains(&track0));
    assert!(tree.selection.contains(&track1));
}

#[test]
fn apply_click_shift_selects_range() {
    let mut tree = make_tree(1, 3);
    tree.files[0].expanded = true;
    let fi = FileIdx::new(0);
    let file_key = NodeKey::File(fi);
    let track0 = NodeKey::Track(TrackRef::new(fi, TrackIdx::new(0)));
    let track2 = NodeKey::Track(TrackRef::new(fi, TrackIdx::new(2)));
    tree.apply_click(file_key, false, false); // anchor = File(0)
    tree.apply_click(track2, false, true); // shift to Track(2)
    // Should select File, Track(0), Track(1), Track(2)
    assert_eq!(tree.selection.len(), 4);
    assert!(tree.selection.contains(&file_key));
    assert!(tree.selection.contains(&track0));
    assert!(tree.selection.contains(&track2));
}

#[test]
fn ordered_visible_keys_excludes_collapsed_children() {
    let mut tree = make_tree(2, 2);
    tree.files[0].expanded = true; // file 0 expanded
    // file 1 collapsed (default)

    let keys = tree.ordered_visible_keys();
    // Expected: File(0), Track(0,0), Track(0,1), File(1)
    assert_eq!(keys.len(), 4);
    assert_eq!(keys[0], NodeKey::File(FileIdx::new(0)));
    assert_eq!(
        keys[1],
        NodeKey::Track(TrackRef::new(FileIdx::new(0), TrackIdx::new(0)))
    );
    assert_eq!(
        keys[2],
        NodeKey::Track(TrackRef::new(FileIdx::new(0), TrackIdx::new(1)))
    );
    assert_eq!(keys[3], NodeKey::File(FileIdx::new(1)));
}

#[test]
fn event_marker_visibility_synced_after_path_toggle() {
    let mut tree = make_tree(1, 1);
    add_event_paths(&mut tree, 0, 0, &["power/boot", "power/sleep"]);

    // Initially all visible
    assert!(tree.event_marker_visibility().is_visible(
        TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        "power/boot"
    ));

    tree.toggle_event_path(TrackRef::new(FileIdx::new(0), TrackIdx::new(0)), "power"); // hide all under power
    assert!(!tree.event_marker_visibility().is_visible(
        TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        "power/boot"
    ));
    assert!(!tree.event_marker_visibility().is_visible(
        TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        "power/sleep"
    ));

    tree.toggle_event_path(
        TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        "power/boot",
    ); // show just boot
    assert!(tree.event_marker_visibility().is_visible(
        TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        "power/boot"
    ));
    assert!(!tree.event_marker_visibility().is_visible(
        TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        "power/sleep"
    ));
}
