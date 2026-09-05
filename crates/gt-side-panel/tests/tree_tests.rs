use std::collections::BTreeSet;
use std::path::PathBuf;

use gt_history_types::{DatabaseRef, RecordingMeta};
use gt_loaded_files::{FileHistory, LoadedFiles};
use gt_side_panel::HiddenTracksByRecording;
use gt_side_panel::tree::{CheckState, NodeKey, TreeState};
use gt_types::{
    DataCategory, FileIdx, FileMetadata, FileSource, LoadedFile, LoadedTrack, TrackIdx,
    TrackMetadata, TrackRef,
};
use rstest::rstest;
use rustc_hash::FxHashMap;

/// One empty recording whose tracks are numbered as `track_numbers` lists
/// them.
fn make_loaded_file(filename: String, track_numbers: &[usize]) -> LoadedFile {
    LoadedFile {
        metadata: FileMetadata {
            filename,
            ..gt_test_utils::empty_file_metadata()
        },
        tracks: track_numbers
            .iter()
            .map(|&index| LoadedTrack {
                metadata: TrackMetadata {
                    index,
                    ..gt_test_utils::empty_track_metadata()
                },
                ..gt_test_utils::loaded_track_with_points(Vec::new())
            })
            .collect(),
        event_marker_styles: FxHashMap::default(),
        orphaned_event_markers: Vec::new(),
        source: FileSource::GtdPath(PathBuf::new()),
        load_warnings: Vec::new(),
    }
}

/// `file_count` recordings of `tracks_per_file` tracks each, every track empty
/// and numbered the way the track builder numbers one. None of them is in the
/// history database.
fn make_loaded_files(file_count: usize, tracks_per_file: usize) -> LoadedFiles {
    let mut loaded = LoadedFiles::new();
    for fi in 0..file_count {
        let track_numbers: Vec<usize> = (1..=tracks_per_file).collect();
        loaded.push(
            make_loaded_file(format!("ride-{fi}.gtd"), &track_numbers),
            FileHistory::None,
        );
    }
    loaded
}

fn db_ref() -> DatabaseRef {
    DatabaseRef {
        identity: "dev".to_owned(),
        group_name: "2026-01-01T00:00:00Z_ride".to_owned(),
    }
}

fn stored_history() -> FileHistory {
    let meta = RecordingMeta {
        time_range: None,
        nav_point_count: 0,
        sat_report_count: 0,
        marker_count: 0,
        event_marker_count: 0,
        gtd_size_bytes: 0,
    };
    FileHistory::recording("dev".to_owned(), meta, Some(db_ref()))
}

/// One recording in the history database, its tracks numbered as
/// `track_numbers` lists them.
fn stored_recording(track_numbers: &[usize]) -> LoadedFiles {
    let mut loaded = LoadedFiles::new();
    loaded.push(
        make_loaded_file("ride.gtd".to_owned(), track_numbers),
        stored_history(),
    );
    loaded
}

fn make_tree(file_count: usize, tracks_per_file: usize) -> TreeState {
    let mut tree = TreeState::new();
    tree.sync_from_loaded_files(make_loaded_files(file_count, tracks_per_file).view());
    tree
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

#[test]
fn the_visible_tracks_skip_the_hidden_tracks_and_the_recordings_without_one() {
    let mut tree = make_tree(3, 2);
    tree.toggle_track_check(TrackRef::new(FileIdx::new(0), TrackIdx::new(1)));
    tree.toggle_file_check(FileIdx::new(1));

    let visible: Vec<(FileIdx, Vec<TrackRef>)> = tree
        .visible_tracks_by_file()
        .into_iter()
        .map(|group| (group.file, group.tracks))
        .collect();

    assert_eq!(
        visible,
        vec![
            (
                FileIdx::new(0),
                vec![TrackRef::new(FileIdx::new(0), TrackIdx::new(0))]
            ),
            (
                FileIdx::new(2),
                vec![
                    TrackRef::new(FileIdx::new(2), TrackIdx::new(0)),
                    TrackRef::new(FileIdx::new(2), TrackIdx::new(1)),
                ]
            ),
        ]
    );
}

#[test]
fn hide_file_hides_every_track_of_a_partly_hidden_recording() {
    let mut tree = make_tree(1, 2);
    tree.toggle_track_check(TrackRef::new(FileIdx::new(0), TrackIdx::new(1)));
    assert_eq!(file_check(&tree, 0), CheckState::Mixed);

    tree.hide_file(FileIdx::new(0));

    assert_eq!(file_check(&tree, 0), CheckState::Off);
    assert_eq!(track_check(&tree, 0, 0), CheckState::Off);
    assert!(tree.visible_tracks_by_file().is_empty());
}

#[test]
fn hide_track_hides_one_track_and_leaves_its_recording_mixed() {
    let mut tree = make_tree(1, 2);

    tree.hide_track(TrackRef::new(FileIdx::new(0), TrackIdx::new(0)));

    assert_eq!(track_check(&tree, 0, 0), CheckState::Off);
    assert_eq!(track_check(&tree, 0, 1), CheckState::On);
    assert_eq!(file_check(&tree, 0), CheckState::Mixed);
}

/// A hand-edited settings file cannot put the Visible section's divider
/// outside the region it divides with the tree.
#[rstest]
#[case::above_the_region(1.5)]
#[case::negative(-0.2)]
#[case::not_a_number(f32::NAN)]
fn a_share_outside_the_region_leaves_the_visible_section_where_it_is(#[case] share: f32) {
    let mut tree = make_tree(1, 1);
    tree.set_visible_section_fraction(0.5);

    tree.set_visible_section_fraction(share);

    let kept = tree.visible_section_fraction();
    assert!((kept - 0.5).abs() < f32::EPSILON, "the share became {kept}");
}

/// A track hidden in a recording that history holds is remembered by its
/// stored number, not by its position. A permanently deleted earlier track
/// leaves a gap in that numbering, and the recording opens with the same track
/// hidden.
#[test]
fn a_hidden_track_is_remembered_by_its_stored_number() {
    let mut tree = TreeState::new();
    tree.sync_from_loaded_files(stored_recording(&[1, 2, 4]).view());

    tree.hide_track(TrackRef::new(FileIdx::new(0), TrackIdx::new(2)));
    assert_eq!(tree.hidden_tracks().track_numbers(&db_ref()), [4]);

    let mut reopened = TreeState::new();
    reopened.set_hidden_tracks(tree.hidden_tracks().clone());
    reopened.sync_from_loaded_files(stored_recording(&[1, 2, 4]).view());

    assert_eq!(track_check(&reopened, 0, 0), CheckState::On);
    assert_eq!(track_check(&reopened, 0, 1), CheckState::On);
    assert_eq!(track_check(&reopened, 0, 2), CheckState::Off);
}

#[test]
fn a_recording_outside_the_history_database_remembers_no_hidden_track() {
    let mut tree = make_tree(1, 2);

    tree.hide_track(TrackRef::new(FileIdx::new(0), TrackIdx::new(0)));

    assert!(tree.hidden_tracks().is_empty());
}

#[test]
fn removing_a_recording_from_the_view_keeps_its_hidden_tracks() {
    let mut tree = TreeState::new();
    tree.sync_from_loaded_files(stored_recording(&[1, 2]).view());
    tree.hide_track(TrackRef::new(FileIdx::new(0), TrackIdx::new(1)));

    tree.sync_from_loaded_files(LoadedFiles::new().view());

    assert_eq!(tree.hidden_tracks().track_numbers(&db_ref()), [2]);
}

/// A remembered number the recording no longer holds - from a shelve or a
/// permanent delete - stays remembered. Every track of the recording shows
/// until that number comes back.
#[test]
fn a_remembered_track_the_recording_no_longer_has_stays_remembered() {
    let mut hidden = HiddenTracksByRecording::default();
    hidden.record(&db_ref(), BTreeSet::from([2, 7]));
    let mut tree = TreeState::new();
    tree.set_hidden_tracks(hidden);

    tree.sync_from_loaded_files(stored_recording(&[1, 2, 3]).view());

    assert_eq!(track_check(&tree, 0, 0), CheckState::On);
    assert_eq!(track_check(&tree, 0, 1), CheckState::Off);
    assert_eq!(track_check(&tree, 0, 2), CheckState::On);
    assert_eq!(tree.hidden_tracks().track_numbers(&db_ref()), [2, 7]);
}

/// The remembered numbers address other stretches of a re-segmented
/// recording. Re-segmentation numbers its tracks over new boundaries.
#[test]
fn resegmenting_a_recording_forgets_its_hidden_tracks() {
    let mut tree = TreeState::new();
    tree.sync_from_loaded_files(stored_recording(&[1, 2]).view());
    tree.hide_track(TrackRef::new(FileIdx::new(0), TrackIdx::new(1)));

    tree.reset_for_resegmented_files(stored_recording(&[1, 2, 3]).view());

    assert!(tree.hidden_tracks().is_empty());
}

/// Two loaded files of one recording write one entry, from the later of the
/// two in tree order.
#[test]
fn one_recording_loaded_twice_remembers_the_later_files_hidden_tracks() {
    let mut loaded = stored_recording(&[1, 2]);
    loaded.push(
        make_loaded_file("ride.gtd".to_owned(), &[1, 2]),
        stored_history(),
    );
    let mut tree = TreeState::new();
    tree.sync_from_loaded_files(loaded.view());

    tree.hide_track(TrackRef::new(FileIdx::new(0), TrackIdx::new(0)));
    tree.hide_track(TrackRef::new(FileIdx::new(1), TrackIdx::new(1)));

    assert_eq!(tree.hidden_tracks().track_numbers(&db_ref()), [2]);
}
