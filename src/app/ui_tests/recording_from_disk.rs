use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui_kittest::{Harness, kittest::NodeT as _, kittest::Queryable as _};
use gt_test_utils::{HarnessInteraction as _, TestHarness};
use gt_types::{FileIdx, TrackIdx, TrackRef};
use rstest::rstest;

use crate::app::App;
use crate::app::recording_from_disk::{
    self, LEAVE_SHELVED_TRACKS_OUT_LABEL, LOAD_FROM_DISK_LABEL, NO_SHELVED_TRACK_HOVER,
    OPEN_THE_STORED_VERSION_LABEL, RecordingAlreadyInHistory, RecordingContent, RecordingFromDisk,
    RecordingsAlreadyInHistory,
};
use crate::app::test_util;
use crate::app::test_util::harness::TestDroppedFile;
use crate::app::ui_tests;

/// The app with `ride.gtd` in its history database, the second of its two
/// tracks shelved, and the same file sitting on disk beside the database.
fn app_with_a_stored_recording_on_disk() -> (Harness<'static, App>, tempfile::TempDir, PathBuf) {
    use gt_store::{TrackRange, TrackState};

    app_with_a_stored_recording_of(&[
        TrackRange {
            start: 0,
            end: 10,
            state: TrackState::Live,
        },
        TrackRange {
            start: 10,
            end: 20,
            state: TrackState::Shelved,
        },
    ])
}

/// [`app_with_a_stored_recording_on_disk`] with both of the stored tracks
/// live.
fn app_with_a_stored_recording_of_live_tracks()
-> (Harness<'static, App>, tempfile::TempDir, PathBuf) {
    app_with_a_stored_recording_of(&ui_tests::two_live_track_ranges())
}

/// The app with `ride.gtd` in its history database under `tracks`, and the
/// same file sitting on disk beside the database.
///
/// The temporary directory holds both and is returned so it outlives the
/// harness.
fn app_with_a_stored_recording_of(
    tracks: &[gt_store::TrackRange],
) -> (Harness<'static, App>, tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let bytes = ui_tests::two_track_gtd_bytes();
    let gtd_path = dir.path().join("ride.gtd");
    std::fs::write(&gtd_path, &bytes).expect("write the recording");

    let (mut harness, databases) = ui_tests::app_with_the_databases_still_opening(&[]);
    harness.step();

    // Stored under the settings the app runs with: opening the stored version
    // then reproduces its tracks without the "Track settings differ" prompt.
    let settings =
        crate::app::loader::stored_segmentation_from_config(&harness.state().processing_config);
    ui_tests::insert_recording_into(&store, &bytes, tracks, settings);

    ui_tests::land_the_databases(&mut harness, &databases, &store);
    (harness, dir, gtd_path)
}

/// Drop each of `paths` on the window in one drop, and step once so the app
/// takes them.
fn drop_paths(harness: &mut Harness<'_, App>, paths: &[PathBuf]) {
    for path in paths {
        harness
            .input_mut()
            .dropped_files
            .push(Arc::new(TestDroppedFile::path(path.clone())));
    }
    harness.step();
}

/// How many tracks the one loaded recording has, once its load lands.
fn step_until_one_recording_is_loaded(harness: &mut Harness<'_, App>) -> usize {
    assert!(
        harness.step_until(
            |harness| harness.state().shared.borrow().loaded_files.len() == 1
                && harness.state().loader.loading_jobs.is_empty()
        ),
        "no recording finished loading"
    );
    let state = harness.state();
    let shared = state.shared.borrow();
    shared
        .loaded_files
        .view()
        .get(0)
        .expect("the loaded recording")
        .file()
        .tracks
        .len()
}

#[test]
fn dropping_a_recording_history_holds_raises_a_prompt_before_it_loads() {
    let (mut harness, _dir, gtd_path) = app_with_a_stored_recording_on_disk();

    drop_paths(&mut harness, &[gtd_path]);
    test_util::harness::step_until_the_prompt_over_stored_recordings_is_drawn(&mut harness);

    let state = harness.state();
    let prompt = state
        .pending_recordings_already_in_history
        .as_ref()
        .expect("the prompt is open");
    assert_eq!(
        prompt
            .recordings
            .iter()
            .map(|recording| recording.from_disk.filename.as_str())
            .collect::<Vec<_>>(),
        vec!["ride.gtd"]
    );
    assert_eq!(
        state.shared.borrow().loaded_files.len(),
        0,
        "the recording loaded before the user chose"
    );
}

/// One drop of five recordings history holds costs one decision, not five.
#[test]
fn one_drop_of_several_stored_recordings_raises_one_prompt() {
    let (mut harness, dir, gtd_path) = app_with_a_stored_recording_on_disk();
    let copies: Vec<PathBuf> = (0..4)
        .map(|copy| {
            let path = dir.path().join(format!("ride-copy-{copy}.gtd"));
            std::fs::copy(&gtd_path, &path).expect("copy the recording");
            path
        })
        .collect();

    drop_paths(&mut harness, &[vec![gtd_path], copies].concat());
    test_util::harness::step_until_the_prompt_over_stored_recordings_is_drawn(&mut harness);

    let state = harness.state();
    let prompt = state
        .pending_recordings_already_in_history
        .as_ref()
        .expect("the prompt is open");
    assert_eq!(prompt.recordings.len(), 5);
    assert!(
        harness
            .window_rect(&recording_from_disk::recordings_already_in_history_title(5))
            .is_some(),
        "no prompt stands over all five recordings"
    );
}

/// A drop mixing a stored recording with one history has never seen loads the
/// second straight away and raises the prompt over the first.
#[test]
fn a_drop_loads_the_recording_new_to_history_and_prompts_over_the_stored_one() {
    let (mut harness, dir, gtd_path) = app_with_a_stored_recording_on_disk();
    let fresh_path = dir.path().join("fresh.gtd");
    std::fs::write(&fresh_path, ui_tests::minimal_gtd_bytes()).expect("write the second recording");

    drop_paths(&mut harness, &[gtd_path, fresh_path]);
    step_until_one_recording_is_loaded(&mut harness);

    let state = harness.state();
    let shared = state.shared.borrow();
    assert_eq!(
        shared
            .loaded_files
            .view()
            .get(0)
            .expect("the loaded recording")
            .file()
            .metadata
            .filename,
        "fresh.gtd"
    );
    let prompt = state
        .pending_recordings_already_in_history
        .as_ref()
        .expect("the prompt is open");
    assert_eq!(
        prompt
            .recordings
            .iter()
            .map(|recording| recording.from_disk.filename.as_str())
            .collect::<Vec<_>>(),
        vec!["ride.gtd"]
    );
}

#[test]
fn opening_the_stored_version_reproduces_the_stored_tracks_from_the_database() {
    let (mut harness, _dir, gtd_path) = app_with_a_stored_recording_on_disk();

    drop_paths(&mut harness, &[gtd_path]);
    test_util::harness::step_until_the_prompt_over_stored_recordings_is_drawn(&mut harness);
    harness.get_by_label(OPEN_THE_STORED_VERSION_LABEL).click();

    assert_eq!(
        step_until_one_recording_is_loaded(&mut harness),
        1,
        "the stored version showed the shelved track"
    );
    let state = harness.state();
    let shared = state.shared.borrow();
    let file = shared
        .loaded_files
        .view()
        .get(0)
        .expect("the loaded recording")
        .file();
    assert!(
        matches!(file.source, gt_types::FileSource::GtdBytes(_)),
        "the stored version was read from disk"
    );
}

#[test]
fn loading_from_disk_leaves_the_shelved_track_out_while_the_tickbox_is_ticked() {
    let (mut harness, _dir, gtd_path) = app_with_a_stored_recording_on_disk();

    drop_paths(&mut harness, &[gtd_path]);
    test_util::harness::step_until_the_prompt_over_stored_recordings_is_drawn(&mut harness);
    harness.get_by_label(LOAD_FROM_DISK_LABEL).click();

    assert_eq!(
        step_until_one_recording_is_loaded(&mut harness),
        1,
        "the load from disk showed the shelved track"
    );
    let state = harness.state();
    let shared = state.shared.borrow();
    let file = shared
        .loaded_files
        .view()
        .get(0)
        .expect("the loaded recording")
        .file();
    assert!(
        matches!(file.source, gt_types::FileSource::GtdPath(_)),
        "the load from disk read the stored bytes"
    );
}

#[test]
fn loading_from_disk_shows_every_track_once_the_tickbox_is_cleared() {
    let (mut harness, _dir, gtd_path) = app_with_a_stored_recording_on_disk();

    drop_paths(&mut harness, &[gtd_path]);
    test_util::harness::step_until_the_prompt_over_stored_recordings_is_drawn(&mut harness);
    harness.get_by_label(LEAVE_SHELVED_TRACKS_OUT_LABEL).click();
    harness.step();
    harness.get_by_label(LOAD_FROM_DISK_LABEL).click();

    assert_eq!(
        step_until_one_recording_is_loaded(&mut harness),
        2,
        "the load from disk left the shelved track out with the tickbox cleared"
    );
}

#[test]
fn the_tickbox_is_grayed_out_where_no_stored_recording_has_a_shelved_track() {
    let (mut harness, _dir, gtd_path) = app_with_a_stored_recording_of_live_tracks();

    drop_paths(&mut harness, &[gtd_path]);
    test_util::harness::step_until_the_prompt_over_stored_recordings_is_drawn(&mut harness);

    assert!(
        harness
            .get_by_label(LEAVE_SHELVED_TRACKS_OUT_LABEL)
            .accesskit_node()
            .is_disabled(),
        "the tickbox is live over recordings with every track in the view"
    );
    harness.get_by_label(LEAVE_SHELVED_TRACKS_OUT_LABEL).hover();
    harness.run_steps(3);
    assert!(
        harness
            .query_by_label_contains(NO_SHELVED_TRACK_HOVER)
            .is_some(),
        "the grayed-out tickbox says nothing about why"
    );
}

/// Drop `gtd_path` on the window and open what the history database holds for
/// it. The prompt over a stored recording offers that as one of its choices.
fn open_the_stored_version_of(harness: &mut Harness<'_, App>, gtd_path: &Path) {
    drop_paths(harness, std::slice::from_ref(&gtd_path.to_path_buf()));
    test_util::harness::step_until_the_prompt_over_stored_recordings_is_drawn(harness);
    harness.get_by_label(OPEN_THE_STORED_VERSION_LABEL).click();
}

/// The tracks the map draws, in tree order.
fn visible_tracks(harness: &Harness<'_, App>) -> Vec<TrackRef> {
    let state = harness.state();
    let shared = state.shared.borrow();
    shared
        .tree
        .visible_tracks_by_file()
        .iter()
        .flat_map(|group| group.tracks.iter().copied())
        .collect()
}

/// The app reads the hidden track from the database alone when the recording
/// opens again. Stores `ride.gtd` of two live tracks in history. Hides its
/// second track and stores that with the recording. Takes the recording out of
/// the view and clears the tree.
fn app_that_stored_the_second_track_of_a_recording_as_hidden()
-> (Harness<'static, App>, tempfile::TempDir, PathBuf) {
    let (mut harness, dir, gtd_path) = app_with_a_stored_recording_of_live_tracks();
    open_the_stored_version_of(&mut harness, &gtd_path);
    assert_eq!(step_until_one_recording_is_loaded(&mut harness), 2);

    harness
        .state_mut()
        .shared
        .borrow_mut()
        .tree
        .hide_track(TrackRef::new(FileIdx::new(0), TrackIdx::new(1)));
    harness.step();

    harness.state_mut().shared.borrow_mut().tree.pending_unload =
        Some(vec![gt_side_panel::NodeKey::File(FileIdx::new(0))]);
    harness.step();
    harness.state_mut().shared.borrow_mut().tree = gt_side_panel::TreeState::new();
    (harness, dir, gtd_path)
}

/// A track hidden in a recording that history holds is stored with that
/// recording: opening it again hides that track and leaves the other one
/// shown.
#[test]
fn a_hidden_track_is_stored_with_its_recording_and_hidden_when_it_opens_again() {
    let (mut harness, _dir, gtd_path) = app_that_stored_the_second_track_of_a_recording_as_hidden();

    open_the_stored_version_of(&mut harness, &gtd_path);

    assert_eq!(step_until_one_recording_is_loaded(&mut harness), 2);
    assert_eq!(
        visible_tracks(&harness),
        [TrackRef::new(FileIdx::new(0), TrackIdx::new(0))],
        "the second track opens hidden and the first one shown"
    );
}

/// A recording the user reads from the file on disk hides the tracks its
/// stored UI state holds, which arrives after its tracks reach the view.
#[test]
fn a_recording_loaded_from_disk_hides_the_tracks_stored_with_it_as_hidden() {
    let (mut harness, _dir, gtd_path) = app_that_stored_the_second_track_of_a_recording_as_hidden();

    drop_paths(&mut harness, std::slice::from_ref(&gtd_path));
    test_util::harness::step_until_the_prompt_over_stored_recordings_is_drawn(&mut harness);
    harness.get_by_label(LOAD_FROM_DISK_LABEL).click();

    assert_eq!(step_until_one_recording_is_loaded(&mut harness), 2);
    assert!(
        harness
            .step_until(|harness| visible_tracks(harness)
                == [TrackRef::new(FileIdx::new(0), TrackIdx::new(0))]),
        "the tracks the file on disk holds show whatever the database holds as hidden"
    );
}

/// The recordings one drop brought in that history already holds, as the
/// prompt lists them.
fn recordings_already_in_history(count: usize) -> RecordingsAlreadyInHistory {
    use gt_store::{TrackRange, TrackState};

    RecordingsAlreadyInHistory {
        recordings: (0..count)
            .map(|index| RecordingAlreadyInHistory {
                from_disk: RecordingFromDisk {
                    filename: format!("ride-{index}.gtd"),
                    content: RecordingContent::Path(PathBuf::from(format!(
                        "/recordings/ride-{index}.gtd"
                    ))),
                },
                db_ref: gt_store::DatabaseRef {
                    identity: format!("auto:ride-{index}.gtd"),
                    group_name: format!("2025-05-2{index}T10:00:00Z_a1b2"),
                },
                stored_tracks: vec![
                    TrackRange {
                        start: 0,
                        end: 10,
                        state: TrackState::Live,
                    },
                    TrackRange {
                        start: 10,
                        end: 20,
                        state: TrackState::Shelved,
                    },
                ],
            })
            .collect(),
        leave_shelved_tracks_out: true,
    }
}

fn app_showing_the_prompt_over_stored_recordings(count: usize) -> TestHarness<'static, App> {
    let (mut harness, _config_path) = TestHarness::builder()
        .size(egui::vec2(640.0, 420.0))
        .eframe(test_util::harness::build_app);
    harness.inner.step();
    harness
        .inner
        .state_mut()
        .pending_recordings_already_in_history = Some(recordings_already_in_history(count));
    harness.run();
    harness
}

#[test]
fn snapshot_recordings_already_in_history_dialog() {
    let mut harness = app_showing_the_prompt_over_stored_recordings(1);
    harness.snapshot_loose("recordings_already_in_history_dialog");
}

/// More recordings than the prompt lists one by one: the rest are counted.
#[test]
fn snapshot_recordings_already_in_history_dialog_past_the_listed_recordings() {
    let mut harness = app_showing_the_prompt_over_stored_recordings(14);
    harness.snapshot_loose("recordings_already_in_history_dialog_past_the_listed_recordings");
}

fn stored_segmentation_from_app_with_rules(
    app: &App,
    track_split_rule: gt_store::StoredTrackSplitRule,
    fix_placement_rule: gt_store::StoredFixPlacementRule,
) -> gt_store::StoredSegmentation {
    gt_store::StoredSegmentation {
        track_split_rule,
        fix_placement_rule,
        ..crate::app::loader::stored_segmentation_from_config(&app.processing_config)
    }
}

fn resegment_prompt_for(
    app: &App,
    track_split_rule: gt_store::StoredTrackSplitRule,
    fix_placement_rule: gt_store::StoredFixPlacementRule,
) -> crate::app::ResegmentPrompt {
    crate::app::ResegmentPrompt {
        db_ref: gt_store::DatabaseRef {
            identity: "auto:ride.gtd".to_owned(),
            group_name: "2025-05-23T10:00:00Z_a1b2".to_owned(),
        },
        filename: "ride.gtd".to_owned(),
        bytes: std::sync::Arc::from(ui_tests::minimal_gtd_bytes()),
        stored: stored_segmentation_from_app_with_rules(app, track_split_rule, fix_placement_rule),
        stored_tracks: vec![
            gt_store::TrackRange {
                start: 0,
                end: 30,
                state: gt_store::TrackState::Live,
            },
            gt_store::TrackRange {
                start: 30,
                end: 61,
                state: gt_store::TrackState::Shelved,
            },
        ],
        marker_settings_changed: false,
    }
}

#[rstest]
#[case::the_forward_gap_only_split_rule(
    gt_store::StoredTrackSplitRule::ForwardGapOnly,
    gt_store::StoredFixPlacementRule::MissingHeadingAndNothingInFix,
    true
)]
#[case::a_split_rule_this_build_does_not_implement(
    gt_store::StoredTrackSplitRule::Unrecognized(7),
    gt_store::StoredFixPlacementRule::MissingHeadingAndNothingInFix,
    true
)]
#[case::the_missing_heading_placement_rule(
    gt_store::StoredTrackSplitRule::StepInEitherDirection,
    gt_store::StoredFixPlacementRule::MissingHeading,
    true
)]
#[case::a_placement_rule_this_build_does_not_implement(
    gt_store::StoredTrackSplitRule::StepInEitherDirection,
    gt_store::StoredFixPlacementRule::Unrecognized(7),
    true
)]
#[case::the_current_rules(
    gt_store::StoredTrackSplitRule::StepInEitherDirection,
    gt_store::StoredFixPlacementRule::MissingHeadingAndNothingInFix,
    false
)]
fn opening_a_recording_stored_by_another_rule_raises_the_resegment_prompt(
    #[case] stored_split_rule: gt_store::StoredTrackSplitRule,
    #[case] stored_placement_rule: gt_store::StoredFixPlacementRule,
    #[case] expected_prompt: bool,
) {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    let stored = gt_store::StoredRecording {
        bytes: ui_tests::minimal_gtd_bytes(),
        tracks: vec![
            gt_store::TrackRange {
                start: 0,
                end: 30,
                state: gt_store::TrackState::Live,
            },
            gt_store::TrackRange {
                start: 30,
                end: 61,
                state: gt_store::TrackState::Shelved,
            },
        ],
        segmentation: Some(stored_segmentation_from_app_with_rules(
            harness.state(),
            stored_split_rule,
            stored_placement_rule,
        )),
    };

    harness
        .state_mut()
        .handle_history_response(crate::app::history_db::Response::Opened {
            db_ref: gt_store::DatabaseRef {
                identity: "auto:ride.gtd".to_owned(),
                group_name: "2025-05-23T10:00:00Z_a1b2".to_owned(),
            },
            result: Ok(crate::app::history_db::OpenedRecording {
                stored,
                ui_state: Ok(gt_store::RecordingUiState::default()),
            }),
        });

    assert_eq!(harness.state().pending_resegment.is_some(), expected_prompt);
}

#[rstest]
#[case::the_forward_gap_only_split_rule(
    gt_store::StoredTrackSplitRule::ForwardGapOnly,
    gt_store::StoredFixPlacementRule::MissingHeadingAndNothingInFix,
    false
)]
#[case::a_split_rule_this_build_does_not_implement(
    gt_store::StoredTrackSplitRule::Unrecognized(7),
    gt_store::StoredFixPlacementRule::MissingHeadingAndNothingInFix,
    true
)]
#[case::the_missing_heading_placement_rule(
    gt_store::StoredTrackSplitRule::StepInEitherDirection,
    gt_store::StoredFixPlacementRule::MissingHeading,
    false
)]
#[case::a_placement_rule_this_build_does_not_implement(
    gt_store::StoredTrackSplitRule::StepInEitherDirection,
    gt_store::StoredFixPlacementRule::Unrecognized(7),
    true
)]
fn the_resegment_prompt_offers_the_stored_tracks_only_for_rules_it_implements(
    #[case] stored_split_rule: gt_store::StoredTrackSplitRule,
    #[case] stored_placement_rule: gt_store::StoredFixPlacementRule,
    #[case] expected_disabled: bool,
) {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    let prompt = resegment_prompt_for(harness.state(), stored_split_rule, stored_placement_rule);
    harness.state_mut().pending_resegment = Some(prompt);
    harness.run();

    let button = harness.get_by_role_and_label(egui::accesskit::Role::Button, "Use stored tracks");

    assert_eq!(button.accesskit_node().is_disabled(), expected_disabled);
}
