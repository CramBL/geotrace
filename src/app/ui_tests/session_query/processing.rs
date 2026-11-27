use chrono::Duration;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use geotrace_sdk::{Angle, NavFileBuilder, NavFix, NavFixTime};
use gt_loaded_files::FileHistory;
use gt_pending_writes::WriteKind;
use gt_session_bridge::{
    ApplyProcessing, ErrorCode, GetRecording, GetTrack, OpenPoint, ProcessingSettingKey,
    QueryJobState, RangeTarget, RecordingReference, Reply, Request, SetSetting, SettingInput,
    SettingKey, SettingSelection, ShowOnMap, ShowOnPlot, TrackReference,
};
use gt_side_panel::{CheckState, TreeState};
use gt_store::{
    DatabaseRef, HistoryAccess, HistoryDatabase as _, ReadOnlyHistoryDatabase as _,
    RecordingUiState, Store, UiStateVersionReporter,
};
use gt_test_utils::{HarnessInteraction as _, recording_fixtures};
use gt_types::{FileIdx, FixRef, PointIdx, TrackIdx, TrackRef};
use gt_ui_types::MapElementRef;
use rstest::rstest;

use crate::app::history_db::Response;
use crate::app::test_util::harness::{self, TestDroppedFile};
use crate::app::test_util::{recordings, session};
use crate::app::{App, ui_tests};

#[test]
fn applying_processing_rebuilds_tracks_and_expires_old_query_and_view_requests() {
    let mut harness = ui_tests::app_with_recording_bytes(recording_fixtures::two_track_gtd_bytes());
    let old_track = session::first_track_reference(&mut harness);
    let job = session::start_query(&mut harness, "points");
    assert!(matches!(
        session::wait_for_query_to_stop(&mut harness, job),
        QueryJobState::Completed { .. }
    ));
    let target = RangeTarget::Track {
        track: old_track,
        start: None,
        end: None,
    };
    assert_eq!(
        session::execute(&mut harness, Request::ShowOnMap(ShowOnMap { target })),
        Reply::MapPresentationQueued
    );
    harness.step();
    assert!(
        harness
            .state()
            .shared
            .borrow()
            .highlight
            .presented_match
            .is_some()
    );
    assert_eq!(
        session::execute(
            &mut harness,
            Request::ShowOnPlot(ShowOnPlot {
                target,
                metrics: Vec::new(),
            }),
        ),
        Reply::PlotPresentationQueued
    );
    self::set_track_split_gap(&mut harness, Duration::hours(2));
    assert_eq!(
        harness.state().shared.borrow().loaded_files[0].tracks.len(),
        2
    );
    assert_eq!(
        session::execute(&mut harness, Request::ApplyProcessing(ApplyProcessing {})),
        Reply::ProcessingApplied
    );
    assert_eq!(
        session::execute(
            &mut harness,
            Request::GetTrack(GetTrack { track: old_track })
        ),
        Reply::Error {
            code: ErrorCode::StaleTrack
        }
    );
    assert_eq!(
        session::query_status(&mut harness, job),
        QueryJobState::Stale
    );
    assert_eq!(
        session::execute(
            &mut harness,
            Request::GetQueryResults(session::first_query_match_request(job))
        ),
        Reply::Error {
            code: ErrorCode::StaleQueryResults
        }
    );

    let tracks = self::recording_tracks(&mut harness, old_track.recording);
    let [merged] = tracks.as_slice() else {
        panic!("expected one merged track");
    };
    let Reply::Track { details } =
        session::execute(&mut harness, Request::GetTrack(GetTrack { track: *merged }))
    else {
        panic!("expected the fresh track");
    };
    assert_eq!(details.point_count, 20);

    harness.step();
    {
        let shared = harness.state().shared.borrow();
        assert!(shared.highlight.presented_match.is_none());
        assert!(shared.pending_session_plot.is_none());
        assert!(shared.plot_state.is_track_visible(
            shared.loaded_files.files(),
            shared.tree.visibility(),
            &shared.filter,
            TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        ));
    }
}

#[test]
fn fresh_map_requests_use_the_rebuilt_tracks_before_the_next_frame() {
    let mut harness = ui_tests::app_with_recording_bytes(self::recording_with_three_fix_groups());
    harness.state_mut().query_window.open = true;
    ui_tests::run_query(&mut harness, "points | where eph > 10 m | hide");
    let first = session::first_track_reference(&mut harness);
    let old = self::recording_tracks(&mut harness, first.recording);
    assert_eq!(old.len(), 2);
    assert_eq!(
        session::execute(
            &mut harness,
            Request::OpenPoint(OpenPoint {
                track: old[1],
                point: 0
            }),
        ),
        Reply::Error {
            code: ErrorCode::NoVisibleMapGeometry
        }
    );

    self::set_track_split_gap(&mut harness, Duration::seconds(30));
    assert_eq!(
        session::execute(&mut harness, Request::ApplyProcessing(ApplyProcessing {})),
        Reply::ProcessingApplied
    );
    let fresh = self::recording_tracks(&mut harness, first.recording);
    assert_eq!(fresh.len(), 3);
    assert_eq!(
        session::execute(
            &mut harness,
            Request::OpenPoint(OpenPoint {
                track: fresh[1],
                point: 0
            }),
        ),
        Reply::MapPresentationQueued
    );
    harness.step();
    assert!(
        session::visible_action_notices(&harness)
            .contains(&"MCP reprocessed the loaded recordings")
    );
    assert!(
        harness
            .state()
            .editor_query
            .session()
            .matches()
            .unwrap()
            .stale
    );
    let middle_track = TrackRef::new(FileIdx::new(0), TrackIdx::new(1));
    let fresh_point = MapElementRef::Fix(FixRef::new(middle_track, PointIdx::new(0)));
    assert!(
        harness
            .state()
            .shared
            .borrow()
            .highlight
            .is_sticky(fresh_point)
    );
}

#[test]
fn reprocessing_dismisses_a_gui_pin_before_the_track_index_addresses_another_fix() {
    let mut harness = ui_tests::app_with_recording_bytes(self::recording_with_three_fix_groups());
    harness.state_mut().query_window.open = true;
    ui_tests::run_query(&mut harness, "points | where eph > 10 m");
    let row = harness::topmost_point_row(&harness).0.center();
    harness.press_drag_release(row, egui::Vec2::ZERO, 1);
    harness.run_steps(2);
    let previous_track = TrackRef::new(FileIdx::new(0), TrackIdx::new(1));
    let pin = MapElementRef::Fix(FixRef::new(previous_track, PointIdx::new(0)));
    assert_eq!(
        harness.state().shared.borrow().highlight.sticky(),
        Some(pin)
    );

    self::set_track_split_gap(&mut harness, Duration::seconds(30));
    assert_eq!(
        session::execute(&mut harness, Request::ApplyProcessing(ApplyProcessing {})),
        Reply::ProcessingApplied
    );
    assert_eq!(harness.state().shared.borrow().highlight.sticky(), None);
    harness.step();
    assert_eq!(harness.state().shared.borrow().highlight.sticky(), None);
}

#[test]
fn a_later_settings_input_takes_effect_on_the_next_apply() {
    let mut harness = ui_tests::app_with_recording_bytes(recording_fixtures::two_track_gtd_bytes());
    let key = self::set_track_split_gap(&mut harness, Duration::hours(2));
    assert_eq!(
        session::execute(&mut harness, Request::OpenSetting(SettingSelection { key })),
        Reply::SettingOpened { key }
    );
    harness.run_steps(5);
    harness
        .query_all_by_label("h")
        .find(|node| node.accesskit_node().numeric_value() == Some(2.0))
        .unwrap()
        .set_value_accesskit(0.0);
    assert_eq!(
        session::execute(&mut harness, Request::ApplyProcessing(ApplyProcessing {})),
        Reply::ProcessingApplied
    );
    harness.run_steps(2);
    assert_eq!(
        harness
            .state()
            .processing_config
            .track_layout
            .track_split_gap,
        Duration::seconds(30)
    );
    assert_eq!(
        harness.state().shared.borrow().loaded_files[0].tracks.len(),
        1
    );

    harness
        .get_by_label_contains("Apply to loaded data")
        .click();
    harness.run_steps(2);
    assert_eq!(
        harness.state().shared.borrow().loaded_files[0].tracks.len(),
        2
    );
}

#[test]
fn applying_processing_requires_nav_points_and_preserves_empty_tracks() {
    let mut harness = ui_tests::app_with_a_recording();
    {
        let mut shared = harness.state().shared.borrow_mut();
        let file =
            gt_test_utils::loaded_file_with_tracks(vec![gt_test_utils::loaded_track_with_points(
                Vec::new(),
            )]);
        shared.loaded_files.replace_file(0, file, FileHistory::None);
    }
    let generation = harness
        .state()
        .shared
        .borrow()
        .loaded_files
        .data_generation();
    assert_eq!(
        session::execute(&mut harness, Request::ApplyProcessing(ApplyProcessing {})),
        Reply::Error {
            code: ErrorCode::NoLoadedNavPoints
        }
    );
    assert_eq!(
        harness
            .state()
            .shared
            .borrow()
            .loaded_files
            .data_generation(),
        generation
    );
}

#[test]
fn a_window_close_request_takes_precedence_over_applying_processing() {
    let mut harness = ui_tests::app_with_a_recording();
    let generation = harness
        .state()
        .shared
        .borrow()
        .loaded_files
        .data_generation();
    let mut input = egui::RawInput::default();
    input
        .viewports
        .get_mut(&egui::ViewportId::ROOT)
        .unwrap()
        .events
        .push(egui::ViewportEvent::Close);
    let _output = harness.ctx.clone().run_logic(&input, |_| {
        assert_eq!(
            session::execute(&mut harness, Request::ApplyProcessing(ApplyProcessing {})),
            Reply::Error {
                code: ErrorCode::ShuttingDown
            }
        );
    });
    assert_eq!(
        harness
            .state()
            .shared
            .borrow()
            .loaded_files
            .data_generation(),
        generation
    );
}

#[test]
fn applying_processing_to_a_stored_recording_keeps_its_history_track_table() {
    let dir = tempfile::tempdir().unwrap();
    let store = Store::open_in(dir.path());
    let bytes = recording_fixtures::two_track_gtd_bytes();
    let (mut harness, databases) = harness::app_with_the_databases_still_opening(&[]);
    harness::send_open_storage_and_step(&mut harness, &databases, &store);
    harness::drop_file_and_wait_for_load(&mut harness, TestDroppedFile::bytes(bytes, "ride.gtd"));
    let db_ref = harness
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
    let read_history = || {
        let db = store.open_recordings_read_only().unwrap().unwrap();
        db.load(&db_ref).unwrap()
    };
    let before = read_history();
    assert_eq!(before.track_rows(), ui_tests::two_live_track_ranges());
    self::set_track_split_gap(&mut harness, Duration::hours(2));
    assert_eq!(
        session::execute(&mut harness, Request::ApplyProcessing(ApplyProcessing {})),
        Reply::ProcessingApplied
    );
    harness.run_steps(3);
    let after = read_history();
    assert_eq!(after.track_rows(), before.track_rows());
    assert_eq!(after.segmentation, before.segmentation);
    let shared = harness.state().shared.borrow();
    assert_eq!(shared.loaded_files[0].tracks.len(), 1);
    assert_eq!(
        shared
            .loaded_files
            .view()
            .get(0)
            .unwrap()
            .history()
            .db_ref(),
        Some(&db_ref)
    );
}

#[derive(Clone, Copy)]
enum StoredVisibilityBeforeProcessing {
    MissingProof,
    Observed,
    PendingRead,
    StaleProof,
}

impl StoredVisibilityBeforeProcessing {
    fn prepare(self, app: &App, store: &Store, recording: &DatabaseRef) {
        match self {
            Self::Observed => {}
            Self::PendingRead => {
                let mut shared = app.shared.borrow_mut();
                shared.tree = TreeState::new();
                shared.sync_tree_from_loaded_files();
            }
            Self::StaleProof => {
                let db = store.open_recordings_read_only().unwrap().unwrap();
                HistoryAccess::lock()
                    .invalidate_recording_snapshots(&db, std::slice::from_ref(recording))
                    .unwrap();
            }
            Self::MissingProof => {
                let mut shared = app.shared.borrow_mut();
                let entry = shared.loaded_files.view().get(0).unwrap();
                let file = entry.file().clone();
                let history = entry.history().clone().with_track_snapshot(None);
                shared.loaded_files.replace_file(0, file, history);
                shared.sync_tree_from_loaded_files();
            }
        }
    }
}

#[rstest]
#[case::observed(StoredVisibilityBeforeProcessing::Observed, &[])]
#[case::pending_read(StoredVisibilityBeforeProcessing::PendingRead, &[2])]
#[case::stale_proof(StoredVisibilityBeforeProcessing::StaleProof, &[1, 7])]
#[case::missing_proof(StoredVisibilityBeforeProcessing::MissingProof, &[1, 7])]
fn processing_visibility_persists_through_shutdown_without_painting(
    #[case] visibility: StoredVisibilityBeforeProcessing,
    #[case] expected_hidden: &[u64],
) {
    let stored_rows = ui_tests::two_live_track_ranges();
    let (mut harness, dir, recording) = recordings::app_with_stored_tracks(&stored_rows);
    let store = Store::open_in(dir.path());
    let saved = RecordingUiState::with_hidden_track_numbers([1, 7]);
    store
        .open_recordings()
        .unwrap()
        .set_recording_ui_state(&recording, &saved, &UiStateVersionReporter::default())
        .unwrap();
    let read_recording = || {
        store
            .open_recordings_read_only()
            .unwrap()
            .unwrap()
            .load(&recording)
            .unwrap()
    };
    let before = read_recording();
    let first = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    let second = TrackRef::new(first.fi, TrackIdx::new(1));
    harness.state().history.open(recording.clone());
    assert!(harness.step_until(|harness| {
        harness
            .state()
            .shared
            .borrow()
            .tree
            .track_node(first)
            .is_some_and(|node| node.check == CheckState::Off)
    }));
    let proof = recordings::worker_track_snapshot(&harness.state().history, &recording);
    visibility.prepare(harness.state(), &store, &recording);
    let old_read = matches!(visibility, StoredVisibilityBeforeProcessing::PendingRead).then(|| {
        harness
            .state()
            .history
            .load_recording_ui_state(recording.clone());
        recordings::next_visibility_response(&mut harness)
    });

    let old_markers = harness
        .state()
        .processing_config
        .generated_markers
        .detect_clock_discontinuities;
    assert!(matches!(
        session::execute(
            &mut harness,
            Request::SetSetting(SetSetting {
                key: SettingKey::Processing(ProcessingSettingKey::DetectClockDiscontinuities),
                value: SettingInput::Boolean(!old_markers),
            })
        ),
        Reply::Setting { .. }
    ));
    let eligible = matches!(
        visibility,
        StoredVisibilityBeforeProcessing::Observed | StoredVisibilityBeforeProcessing::PendingRead
    );
    let admitted_visibility_writes = {
        let _access = HistoryAccess::lock();
        harness
            .state()
            .history
            .load_recording_ui_state(recording.clone());
        assert_eq!(
            session::execute(&mut harness, Request::ApplyProcessing(ApplyProcessing {})),
            Reply::ProcessingApplied
        );
        let app = harness.state_mut();
        let admitted = app
            .pending_writes
            .snapshot()
            .running
            .iter()
            .filter(|write| write.kind == WriteKind::RecordingDatabase)
            .count();
        if let Some(old_read) = old_read {
            assert!(
                matches!(&old_read, Response::RecordingUiStateLoaded(Ok((layout, state))) if layout.same_observation(proof.layout()) && *state == saved)
            );
            app.handle_history_response(old_read);
            app.shared.borrow_mut().tree.hide_track(second);
        }
        {
            let shared = app.shared.borrow();
            let entry = shared.loaded_files.view().get(0).unwrap();
            assert_eq!(entry.file().tracks.len(), 2);
            assert_eq!(
                entry
                    .history()
                    .track_snapshot()
                    .is_some_and(|snapshot| snapshot.same_layout_and_payload(&proof)
                        && snapshot.is_current()),
                eligible
            );
            assert_eq!(shared.tree.track_node(first).unwrap().check, CheckState::On);
        }
        app.begin_shutdown();
        admitted
    };
    harness::finish_shutdown_without_painting(&mut harness);
    drop(harness);
    let after = read_recording();
    assert_eq!(after.track_rows(), before.track_rows());
    assert_eq!(after.segmentation, before.segmentation);
    assert_eq!(after.bytes, before.bytes);

    let (mut reopened, databases) = harness::app_with_the_databases_still_opening(&[]);
    reopened.step();
    harness::send_open_storage_and_step(&mut reopened, &databases, &store);
    reopened.state().history.open(recording.clone());
    assert!(
        reopened.step_until(|harness| !harness.state().shared.borrow().loaded_files.is_empty())
    );
    let db = store.open_recordings_read_only().unwrap().unwrap();
    assert_eq!(
        db.recording_ui_state(&recording, &UiStateVersionReporter::default())
            .unwrap(),
        RecordingUiState::with_hidden_track_numbers(expected_hidden.iter().copied())
    );
    assert_eq!(admitted_visibility_writes, usize::from(eligible));
    let shared = reopened.state().shared.borrow();
    for (number, track) in [(1, first), (2, second)] {
        let check = if expected_hidden.contains(&number) {
            CheckState::Off
        } else {
            CheckState::On
        };
        assert_eq!(shared.tree.track_node(track).unwrap().check, check);
    }
}

fn set_track_split_gap(harness: &mut Harness<'_, App>, gap: Duration) -> SettingKey {
    let key = SettingKey::Processing(ProcessingSettingKey::TrackSplitGap);
    assert!(matches!(
        session::execute(
            harness,
            Request::SetSetting(SetSetting {
                key,
                value: SettingInput::Number(gap.as_seconds_f64()),
            }),
        ),
        Reply::Setting { .. }
    ));
    key
}

fn recording_tracks(
    harness: &mut Harness<'_, App>,
    recording: RecordingReference,
) -> Vec<TrackReference> {
    let Reply::Recording { details } = session::execute(
        harness,
        Request::GetRecording(GetRecording {
            recording,
            after: None,
            stored_track_start: None,
            limit: None,
        }),
    ) else {
        panic!("expected the recording details");
    };
    details.tracks
}

fn recording_with_three_fix_groups() -> Vec<u8> {
    let mut recorder = NavFileBuilder::new().open();
    for (index, seconds) in [0, 1, 60, 61, 600, 601].into_iter().enumerate() {
        let time = ui_tests::base_time() + Duration::seconds(seconds);
        recorder.add_nav_fix(
            NavFix::builder()
                .time(NavFixTime::Receiver(time))
                .lat(Angle::degrees(51.5 + index as f64 * 0.0002))
                .lon(Angle::degrees(-0.1))
                .eph_m(if seconds >= 600 { 20.0 } else { 2.4 })
                .build(),
        );
    }
    let mut bytes = Vec::new();
    recorder.finish().unwrap().write(&mut bytes).unwrap();
    bytes
}
