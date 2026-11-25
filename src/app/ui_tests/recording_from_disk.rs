use std::path::{Path, PathBuf};
use std::sync::Arc;

use egui_kittest::{Harness, kittest::NodeT as _, kittest::Queryable as _};
use geotrace_sdk::{Angle, Channel, NavFile, NavFileBuilder, NavFix, SolutionState};
use gt_loaded_files::LoadedFileId;
use gt_log_view::{LoadedLog, LogAttachmentRef, RecordingKey};
use gt_logfile::ParsedLog;
use gt_side_panel::NodeKey;
use gt_store::{
    DatabaseRef, HistoryAccess, HistoryDatabase as _, LogAttachmentId,
    ReadOnlyHistoryDatabase as _, ReadOnlyRecordings, Recordings, StoredFixPlacementRule,
    StoredLogFilterStack, StoredLogSettings, StoredYearResolution, TrackRange, TrackState,
};
use gt_test_utils::{HarnessInteraction as _, TestHarness};
use gt_track_builder::SegmentationConfig;
use gt_types::{AssociationWindow, FileIdx, Latitude, Longitude, TrackIdx, TrackRef};
use rstest::rstest;

use crate::app::association_batches::{AssociationSubmission, RecordingOperationOrigin};
use crate::app::recording_from_disk::{
    self, LEAVE_SHELVED_TRACKS_OUT_LABEL, LOAD_FROM_DISK_LABEL, NO_SHELVED_TRACK_HOVER,
    OPEN_THE_STORED_VERSION_LABEL, RecordingAlreadyInHistory, RecordingContent, RecordingFromDisk,
    RecordingsAlreadyInHistory,
};
use crate::app::test_util;
use crate::app::test_util::harness::TestDroppedFile;
use crate::app::ui_tests;
use crate::app::{App, ParsedLogOrigin, log_viewer, modals};
use crate::settings::InitialPositionSourcePolicy;

/// The app with `ride.gtd` in its history database, the second of its two
/// tracks shelved, and the same file sitting on disk beside the database.
fn app_with_a_stored_recording_on_disk() -> (Harness<'static, App>, tempfile::TempDir, PathBuf) {
    app_with_a_stored_shelved_track(&ui_tests::two_track_gtd_bytes())
}

fn app_with_a_stored_shelved_track(
    bytes: &[u8],
) -> (Harness<'static, App>, tempfile::TempDir, PathBuf) {
    let [live, mut shelved] = ui_tests::two_live_track_ranges();
    shelved.state = TrackState::Shelved;
    app_with_a_stored_recording_of_bytes(&[live, shelved], bytes)
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
    app_with_a_stored_recording_of_bytes(tracks, &ui_tests::two_track_gtd_bytes())
}

fn app_with_a_stored_recording_of_bytes(
    tracks: &[TrackRange],
    bytes: &[u8],
) -> (Harness<'static, App>, tempfile::TempDir, PathBuf) {
    let dir = tempfile::tempdir().expect("temp dir");
    let store = gt_store::Store::open_in(dir.path());
    let gtd_path = dir.path().join("ride.gtd");
    std::fs::write(&gtd_path, bytes).expect("write the recording");

    let (mut harness, databases) = ui_tests::app_with_the_databases_still_opening(&[]);
    harness.step();

    // Stored under the settings the app runs with: opening the stored version
    // then reproduces its tracks without the "Track settings differ" prompt.
    let settings =
        crate::app::loader::stored_segmentation_from_config(&harness.state().processing_config);
    ui_tests::insert_recording_into(&store, bytes, tracks, settings);

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

#[derive(Clone, Copy)]
enum DiskShelfMode {
    Owner,
    ReadOnly,
    AutoStoreDisabled,
}

fn two_tracks_with_missing_samples() -> Vec<u8> {
    let bytes = ui_tests::two_track_gtd_bytes();
    let file = NavFile::read(bytes.as_slice()).expect("read two tracks");
    let mut builder = NavFileBuilder::new().open();
    for (index, point) in file.nav_points().iter().enumerate() {
        let fix = if index == 4 {
            NavFix::status_only(point.fix.time, SolutionState::NoFix)
        } else {
            point.fix
        };
        builder.add_nav_fix(fix);
    }
    builder.add_channel(
        Channel::builder()
            .name("sensor")
            .times(vec![ui_tests::base_time()])
            .values(vec![f64::NAN])
            .build()
            .expect("channel sample"),
    );
    let file = builder.finish().expect("recording with missing samples");
    let mut bytes = Vec::new();
    file.write(&mut bytes).expect("write recording");
    bytes
}

#[rstest]
#[case::owner(DiskShelfMode::Owner)]
#[case::read_only(DiskShelfMode::ReadOnly)]
#[case::missing_samples_without_auto_store(DiskShelfMode::AutoStoreDisabled)]
fn loading_from_disk_leaves_the_shelved_track_out_while_the_tickbox_is_ticked(
    #[case] mode: DiskShelfMode,
) {
    let (mut harness, _dir, gtd_path) = match mode {
        DiskShelfMode::Owner | DiskShelfMode::ReadOnly => app_with_a_stored_recording_on_disk(),
        DiskShelfMode::AutoStoreDisabled => {
            app_with_a_stored_shelved_track(&two_tracks_with_missing_samples())
        }
    };
    let app = harness.state_mut();
    match mode {
        DiskShelfMode::ReadOnly => app
            .pending_writes
            .become_read_only_for_the_rest_of_the_run(),
        DiskShelfMode::AutoStoreDisabled => {
            app.storage_settings.enabled = false;
            app.sync_db_path();
        }
        DiskShelfMode::Owner => {}
    }

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
    let entry = shared
        .loaded_files
        .view()
        .get(0)
        .expect("the loaded recording");
    assert!(entry.history().track_snapshot().is_some());
    let file = entry.file();
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

#[rstest]
#[case::leave_shelved_out(true)]
#[case::show_all_tracks(false)]
fn a_different_disk_payload_never_uses_the_stored_track_rows(#[case] leave_shelved_out: bool) {
    let (mut harness, _dir, gtd_path) = app_with_a_stored_recording_on_disk();
    drop_paths(&mut harness, std::slice::from_ref(&gtd_path));
    test_util::harness::step_until_the_prompt_over_stored_recordings_is_drawn(&mut harness);

    self::change_recorded_first_latitude(&gtd_path);

    if !leave_shelved_out {
        harness.get_by_label(LEAVE_SHELVED_TRACKS_OUT_LABEL).click();
        harness.step();
    }
    harness.get_by_label(LOAD_FROM_DISK_LABEL).click();
    assert_eq!(step_until_one_recording_is_loaded(&mut harness), 2);
    let shared = harness.state().shared.borrow();
    let entry = shared.loaded_files.view().get(0).expect("loaded recording");
    assert!(entry.history().is_stored());
    assert!(entry.history().track_snapshot().is_none());
    assert_eq!(entry.history().shelved_track_count(), 0);
}

fn change_recorded_first_latitude(gtd_path: &Path) {
    let original = std::fs::read(gtd_path).expect("read original file");
    let mut changed = NavFile::read(original.as_slice()).expect("decode original file");
    changed.nav_points_mut()[0].fix.lat = Angle::degrees(52.0);
    let mut changed_bytes = Vec::new();
    changed
        .write(&mut changed_bytes)
        .expect("encode changed position");
    let old_meta = gt_store::extract_meta(&original).expect("read original metadata");
    let new_meta = gt_store::extract_meta(&changed_bytes).expect("read changed metadata");
    assert!(old_meta.same_recording(&new_meta));
    std::fs::write(gtd_path, &changed_bytes).expect("replace file before load");
}

#[test]
fn a_log_arrival_uses_the_selected_copy_of_a_stored_recording() {
    let (mut harness, _dir, gtd_path) = app_with_a_stored_recording_of_live_tracks();
    drop_paths(&mut harness, std::slice::from_ref(&gtd_path));
    test_util::harness::step_until_the_prompt_over_stored_recordings_is_drawn(&mut harness);
    self::change_recorded_first_latitude(&gtd_path);
    harness.get_by_label(LOAD_FROM_DISK_LABEL).click();
    assert_eq!(step_until_one_recording_is_loaded(&mut harness), 2);
    let recording = harness
        .state()
        .shared
        .borrow()
        .loaded_files
        .view()
        .get(0)
        .unwrap()
        .history()
        .db_ref()
        .unwrap()
        .clone();
    self::open_scoped_history_arrival(harness.state_mut(), &recording);
    assert!(harness.step_until(|harness| {
        harness
            .state()
            .first_log()
            .is_some_and(|log| log.associated_recording().is_some())
    }));

    let app = harness.state_mut();
    let mut files = app.shared.borrow().loaded_files.clone();
    let entry = files.view().get(1).unwrap();
    assert_eq!(entry.history().db_ref(), Some(&recording));
    let stored_id = entry.id();
    self::assert_log_source(app, Some(stored_id), Some(Latitude::new(51.5)));
    for (file_index, count) in [(0, 0), (1, 1)] {
        let removed = modals::removed_recording_keys(
            &[NodeKey::File(FileIdx::new(file_index))],
            files.view(),
        );
        assert_eq!(app.logs.anchored_to(&removed).count(), count);
    }

    files.remove_file(1);
    app.logs.reassociate_all(&files.view());
    self::assert_log_source(app, None, None);
    app.history.open(recording);
    assert!(harness.step_until(|harness| harness.state().shared.borrow().loaded_files.len() == 3));
    let app = harness.state_mut();
    let (reread_file, history) = {
        let shared = app.shared.borrow();
        let reread = shared.loaded_files.view().get(2).expect("History reread");
        (reread.file().clone(), reread.history().clone())
    };
    let mut partial_file = reread_file.clone();
    partial_file.tracks.remove(0);
    files.push(partial_file, history.clone());
    app.logs.reassociate_all(&files.view());
    self::assert_log_source(app, None, None);

    files.replace_file(1, reread_file, history);
    let reopened = files.view().get(1).unwrap().id();
    assert_ne!(reopened, stored_id);
    app.logs.reassociate_all(&files.view());
    self::assert_log_source(app, Some(reopened), Some(Latitude::new(51.5)));
}

#[test]
fn a_log_stays_unassociated_with_a_changed_history_payload() {
    let (mut harness, _dir, _gtd_path) = app_with_a_stored_recording_on_disk();
    let app = harness.state_mut();
    let path = app.history.path().expect("History path").to_owned();
    let recording = self::stored_recording_ref(&path);
    self::open_scoped_history_arrival(app, &recording);
    assert!(harness.step_until(|harness| {
        harness
            .state()
            .first_log()
            .is_some_and(|log| log.associated_recording().is_some())
    }));

    {
        let _access = HistoryAccess::lock();
        let file = hdf5_pure::File::open_rw(&path).expect("edit History");
        let dataset_path = format!(
            "by_identity/{}/{}/nav_points/lat",
            gt_store::identity_group_name(&recording.identity),
            recording.group_name,
        );
        let mut latitude = file.dataset(&dataset_path).expect("latitude dataset");
        let mut samples = latitude.read_f64().expect("latitude samples");
        samples[0] = 52.0;
        latitude.write(&samples).expect("replace latitude");
        file.commit().expect("commit changed payload");
    }

    harness.state().history.open(recording);
    assert!(harness.step_until(|harness| harness.state().shared.borrow().loaded_files.len() == 2));
    let app = harness.state_mut();
    let mut files = app.shared.borrow().loaded_files.clone();
    let first = files.view().get(0).expect("original load");
    let second = files.view().get(1).expect("replacement load");
    let original = first
        .history()
        .track_snapshot()
        .expect("original observation");
    let replacement = second
        .history()
        .track_snapshot()
        .expect("replacement observation");
    assert!(original.is_current() && original.same_observation(replacement));
    assert_eq!(original.rows(), replacement.rows());
    self::assert_log_source(app, Some(first.id()), Some(Latitude::new(51.5)));

    let replacement_id = second.id();
    files.remove_file(0);
    app.logs.reassociate_all(&files.view());
    self::assert_log_source(app, None, None);

    let log_id = app.logs.first_id().expect("loaded log identity");
    app.logs
        .get_mut_by_id(log_id)
        .expect("loaded log")
        .anchor_to_loaded_recording(Some(replacement_id), &files.view());
    self::assert_log_source(app, Some(replacement_id), Some(Latitude::new(52.0)));
}

#[rstest]
#[case::matching_rows(true)]
#[case::different_rows(false)]
fn a_detached_log_waits_for_a_verified_history_source(#[case] matching_rows: bool) {
    let (mut harness, _dir, gtd_path) = if matching_rows {
        app_with_a_stored_recording_on_disk()
    } else {
        let [first, last] = ui_tests::two_live_track_ranges();
        app_with_a_stored_recording_of(&[TrackRange {
            end: last.end,
            ..first
        }])
    };
    let app = harness.state_mut();
    let recording = self::stored_recording_ref(app.history.path().expect("History path"));
    let attachment = LogAttachmentRef {
        recording: recording.clone(),
        id: LogAttachmentId::new_random(),
    };
    {
        let shared = app.shared.borrow();
        let files = shared.loaded_files.view();
        app.logs.restore_attachment(
            LoadedLog::new(
                Some("detached.log".to_owned()),
                self::parsed_position_log(),
                AssociationWindow::ZERO,
            ),
            attachment.clone(),
            StoredLogSettings {
                association_window: None,
                filters: StoredLogFilterStack::default(),
                year_reference: None,
                year_resolution: StoredYearResolution::Infer,
            },
            &files,
        );
        app.logs.forget_attachment(&attachment, &files);
    }
    drop_paths(&mut harness, std::slice::from_ref(&gtd_path));
    test_util::harness::step_until_the_prompt_over_stored_recordings_is_drawn(&mut harness);
    self::change_recorded_first_latitude(&gtd_path);
    harness.get_by_label(LOAD_FROM_DISK_LABEL).click();
    assert_eq!(step_until_one_recording_is_loaded(&mut harness), 2);
    self::assert_log_source(harness.state(), None, None);
    let original_log = harness.state().logs.first_id().expect("detached log");
    harness.state_mut().log_viewer.open_on_log(original_log);
    harness.run_steps(3);
    assert!(
        !harness
            .get_by_label(log_viewer::LOAD_RECORDING_LABEL)
            .accesskit_node()
            .is_disabled()
    );
    harness.state_mut().shared.borrow_mut().tree.pending_unload =
        Some(vec![NodeKey::File(FileIdx::new(0))]);
    harness.run_steps(2);
    assert!(harness.state().shared.borrow().loaded_files.is_empty());
    assert_eq!(harness.state().logs.first_id(), Some(original_log));
    self::assert_log_source(harness.state(), None, None);

    harness.state().history.open(recording);
    assert_eq!(
        step_until_one_recording_is_loaded(&mut harness),
        if matching_rows { 1 } else { 2 }
    );
    if !matching_rows {
        self::assert_log_source(harness.state(), None, None);
        harness.get_by_label("Choose the source in Positions from");
        assert!(
            harness
                .get_by_label(log_viewer::LOAD_RECORDING_LABEL)
                .accesskit_node()
                .is_disabled()
        );
        return;
    }
    let app = harness.state_mut();
    let shared = app.shared.borrow();
    let entry = shared
        .loaded_files
        .view()
        .get(0)
        .expect("verified History load");
    self::assert_log_source(app, Some(entry.id()), Some(Latitude::new(51.5)));
    assert_eq!(
        app.first_log().expect("detached log").anchor_key(),
        Some(RecordingKey::of_loaded_recording(entry))
    );
}

fn stored_recording_ref(path: &Path) -> DatabaseRef {
    let stored = ReadOnlyRecordings::open_existing_read_only(path).expect("open History");
    stored.list_recordings().expect("list History")[0]
        .db_ref
        .clone()
}

fn open_scoped_history_arrival(app: &mut App, recording: &DatabaseRef) {
    app.initial_position_source_policy = InitialPositionSourcePolicy::AutomaticallyUseUnambiguous;
    let batch = app.association_batches.begin_submission(std::iter::empty());
    app.history.open_with_origin(
        recording.clone(),
        RecordingOperationOrigin::Arrival(batch.recording()),
    );
    app.integrate_parsed_log(
        Some("selected.log".to_owned()),
        self::parsed_position_log(),
        ParsedLogOrigin::Loose(&batch.log()),
    );
}

fn parsed_position_log() -> ParsedLog {
    let time = ui_tests::base_time();
    let text = format!(
        "{} selected recording\n",
        time.format(gt_fmt::UTC_SECOND_FORMAT)
    );
    gt_logfile::parse_log(text.into(), time).expect("parse log")
}

fn assert_log_source(app: &App, recording: Option<LoadedFileId>, latitude: Option<Latitude>) {
    let log = app.first_log().expect("loaded log");
    assert_eq!(log.associated_recording(), recording);
    assert_eq!(
        log.entry_placement(0).map(|placement| placement.position.0),
        latitude
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
    harness.run_steps(5);
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
fn recordings_already_in_history(
    submission: &AssociationSubmission,
    count: usize,
) -> RecordingsAlreadyInHistory {
    use gt_store::{TrackRange, TrackState};

    RecordingsAlreadyInHistory {
        recordings: (0..count)
            .map(|index| RecordingAlreadyInHistory {
                from_disk: RecordingFromDisk {
                    arrival: submission.recording(),
                    filename: format!("ride-{index}.gtd"),
                    content: RecordingContent::Path(PathBuf::from(format!(
                        "/recordings/ride-{index}.gtd"
                    ))),
                    mode: crate::app::loader::GtdLoadMode::Regular,
                },
                history_path: PathBuf::new(),
                track_snapshot: test_util::recordings::synthetic_track_snapshot(
                    &gt_store::DatabaseRef {
                        identity: format!("auto:ride-{index}.gtd"),
                        group_name: format!("2025-05-2{index}T10:00:00Z_a1b2"),
                    },
                    &[
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
                ),
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
    let submission = harness.inner.state_mut().recording_submission_for_test();
    harness
        .inner
        .state_mut()
        .pending_recordings_already_in_history =
        Some(recordings_already_in_history(&submission, count));
    drop(submission);
    harness.run();
    harness
}

#[test]
fn snapshot_recordings_already_in_history_dialog() {
    let mut harness = app_showing_the_prompt_over_stored_recordings(1);
    harness.snapshot_with_color_tolerance("recordings_already_in_history_dialog");
}

/// More recordings than the prompt lists one by one: the rest are counted.
#[test]
fn snapshot_recordings_already_in_history_dialog_past_the_listed_recordings() {
    let mut harness = app_showing_the_prompt_over_stored_recordings(14);
    harness.snapshot_with_color_tolerance(
        "recordings_already_in_history_dialog_past_the_listed_recordings",
    );
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
    let db_ref = gt_store::DatabaseRef {
        identity: "auto:ride.gtd".to_owned(),
        group_name: "2025-05-23T10:00:00Z_a1b2".to_owned(),
    };
    crate::app::ResegmentPrompt {
        origin: RecordingOperationOrigin::Independent,
        db_ref: db_ref.clone(),
        filename: "ride.gtd".to_owned(),
        bytes: std::sync::Arc::from(ui_tests::minimal_gtd_bytes()),
        stored: stored_segmentation_from_app_with_rules(app, track_split_rule, fix_placement_rule),
        track_snapshot: test_util::recordings::synthetic_track_snapshot(
            &db_ref,
            &[
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
        ),
        marker_settings_changed: false,
        mode: crate::app::loader::GtdLoadMode::Regular,
        placement: crate::app::loader::LoadedRecordingPlacement::AddAnEntry,
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
#[case::the_previous_default_placement_rule(
    gt_store::StoredTrackSplitRule::StepInEitherDirection,
    gt_store::StoredFixPlacementRule::MissingHeadingAndNothingInFix,
    true
)]
#[case::the_current_rules(
    gt_store::StoredTrackSplitRule::StepInEitherDirection,
    gt_store::StoredFixPlacementRule::SynthesizedOrHeadinglessNoFix,
    false
)]
fn opening_a_recording_stored_by_another_rule_raises_the_resegment_prompt(
    #[case] stored_split_rule: gt_store::StoredTrackSplitRule,
    #[case] stored_placement_rule: gt_store::StoredFixPlacementRule,
    #[case] expected_prompt: bool,
) {
    let mut harness = Harness::builder()
        .with_context_setup(gt_ui_theme::install_app_style)
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
        debug_tag: None,
    };

    let origin = RecordingOperationOrigin::Independent;
    let recording = gt_store::DatabaseRef {
        identity: "auto:ride.gtd".to_owned(),
        group_name: "2025-05-23T10:00:00Z_a1b2".to_owned(),
    };
    harness
        .state_mut()
        .handle_history_response(crate::app::history_db::Response::Opened {
            origin,
            db_ref: recording.clone(),
            placement: crate::app::loader::LoadedRecordingPlacement::AddAnEntry,
            result: Ok(test_util::recordings::opened_recording(&recording, stored)),
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
        .with_context_setup(gt_ui_theme::install_app_style)
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    let prompt = resegment_prompt_for(harness.state(), stored_split_rule, stored_placement_rule);
    harness.state_mut().pending_resegment = Some(prompt);
    harness.run();

    let button = harness.get_by_role_and_label(egui::accesskit::Role::Button, "Use stored tracks");

    assert_eq!(button.accesskit_node().is_disabled(), expected_disabled);
}

#[rstest]
#[case::original(StoredFixPlacementRule::MissingHeading, 0.5)]
#[case::previous_default(StoredFixPlacementRule::MissingHeadingAndNothingInFix, 0.5)]
#[case::navigation_evidence(StoredFixPlacementRule::SynthesizedOrHeadinglessNoFix, 10.0)]
fn history_reopening_reproduces_geometry_and_navigation_evidence(
    #[case] rule: StoredFixPlacementRule,
    #[case] expected_middle_longitude: f64,
) {
    let file =
        gt_test_utils::navigation_evidence_fixtures::headingless_navigation_recording().unwrap();
    let mut bytes = Vec::new();
    file.write(&mut bytes).unwrap();
    let current = SegmentationConfig::default();
    let settings = gt_store::StoredSegmentation {
        fix_placement_rule: rule,
        ..crate::app::loader::stored_segmentation_from_config(&current)
    };
    let config = crate::app::loader::config_from_stored_segmentation(&settings, current);
    let direct = gt_loader::load_bytes_with_progress(
        &bytes,
        "navigation.gtd".to_owned(),
        |_, _| {},
        &config,
    )
    .unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut database = Recordings::open_or_create(&directory.path().join("history.h5")).unwrap();
    let meta = gt_store::extract_meta(&bytes).unwrap();
    let reference = database
        .insert(
            "navigation",
            &meta,
            &[TrackRange {
                start: 0,
                end: 3,
                state: TrackState::Live,
            }],
            settings,
            &bytes,
        )
        .unwrap();
    let stored = database.load(&reference).unwrap();
    let stored_settings = stored.segmentation.unwrap();
    let restored_config =
        crate::app::loader::config_from_stored_segmentation(&stored_settings, current);
    assert_eq!(
        restored_config.fix_placement_rule,
        config.fix_placement_rule
    );
    let reopened = gt_loader::load_bytes_with_progress(
        &stored.bytes,
        "navigation.gtd".to_owned(),
        |_, _| {},
        &restored_config,
    )
    .unwrap();
    assert_eq!(
        NavFile::read(stored.bytes.as_slice()).unwrap().nav_points(),
        file.nav_points()
    );
    let direct_track = &direct.tracks[0];
    let reopened_track = &reopened.tracks[0];
    assert_eq!(direct_track.geometry, reopened_track.geometry);
    assert_eq!(
        direct_track.metadata.fix_stats,
        reopened_track.metadata.fix_stats
    );
    for (before, after) in direct_track.points.iter().zip(&reopened_track.points) {
        assert_eq!(before.navigation, after.navigation);
        assert_eq!(
            before.satellite_evidence.usage,
            after.satellite_evidence.usage
        );
        assert_eq!(
            before.satellite_evidence.observations,
            after.satellite_evidence.observations
        );
        assert_eq!(before.tpv.position(), after.tpv.position());
    }
    assert_eq!(
        reopened_track.points[1].tpv.position(),
        Some((Latitude::new(10.0), Longitude::new(10.0)))
    );
    assert!(
        (reopened_track
            .resolved_position_at(1)
            .unwrap()
            .1
            .as_degrees()
            - expected_middle_longitude)
            .abs()
            < 1e-9
    );
    assert_eq!(reopened_track.points[1].satellite_evidence.usage.len(), 2);
}
