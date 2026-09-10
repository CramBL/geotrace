use std::sync::Arc;

use egui_kittest::{Harness, kittest::Queryable as _};
use gt_store::{ReadOnlyHistoryDatabase as _, RecordingsHandle};
use gt_test_utils::HarnessInteraction as _;
use gt_test_utils::fixtures::FixCountsAroundAGap;

use crate::app::App;
use crate::app::test_util;
use crate::app::ui_tests;

/// Push a two-track recording (the tracks split at a 10 minute gap),
/// returning its file index.
fn push_two_track_file(harness: &mut Harness<'_, App>, name: &str) -> gt_types::FileIdx {
    let points = gt_test_utils::fixtures::nav_data_with_gap(FixCountsAroundAGap {
        before: 30,
        after: 30,
    });
    let fi = ui_tests::push_points_as(
        harness,
        name,
        &points,
        None,
        gt_loaded_files::FileHistory::None,
    );
    let track_count = {
        let state = harness.state();
        let shared = state.shared.borrow();
        fi.get(shared.loaded_files.files()).map(|f| f.tracks.len())
    };
    assert_eq!(
        track_count,
        Some(2),
        "the gap must split the recording in two"
    );
    fi
}

/// A boat-declared file's track resolves to the unsnappable row (the hover
/// states the mode), while an undeclared file stays idle (no entry).
#[test]
fn snap_row_views_marks_declared_roadless_modes_unsnappable() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    let boat = ui_tests::push_file_with_travel_mode(
        &mut harness,
        "boat.gtd",
        Some(gt_types::TravelMode::Boat),
    );
    let plain = ui_tests::push_file_with_travel_mode(&mut harness, "plain.gtd", None);

    let rows = harness.state().snap_row_views();

    assert_eq!(
        rows.get(&boat),
        Some(&gt_side_panel::SnapRowView::Unsnappable {
            travel_mode: "Boat".to_owned()
        })
    );
    assert_eq!(rows.get(&plain), None, "an undeclared file stays idle");
}

/// The full consent round trip for a snap trigger: the request parks on
/// `pending_snap` and raises the dialog, agreeing takes it (the run is
/// queued - a no-op in the offline test app, so only the take is
/// observable), declining drops it.
#[test]
fn snap_request_parks_on_consent_and_agree_takes_it() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    let track = ui_tests::push_file_with_travel_mode(&mut harness, "ride.gtd", None);

    harness.state_mut().handle_snap_request(vec![track]);
    assert!(harness.state().snap_consent_prompt);
    assert_eq!(harness.state().pending_snap.track_refs, vec![track]);
    harness.step();

    // First synthetic click settles the startup map-layer popup (see
    // `snap_consent_agree_persists_the_server_host`), the second lands.
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Agree - snap automatically")
        .click();
    harness.run_steps(3);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Agree - snap automatically")
        .click();
    harness.run_steps(3);

    assert!(!harness.state().snap_consent_prompt);
    assert!(
        harness.state().pending_snap.track_refs.is_empty(),
        "agreeing must take the parked request and queue it"
    );
    assert!(harness.state().snap_settings.consent_granted());
    assert_eq!(harness.state().snap_settings.auto_snap, Some(true));
}

#[test]
fn snap_request_parked_on_consent_is_dropped_on_decline() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    let track = ui_tests::push_file_with_travel_mode(&mut harness, "ride.gtd", None);

    harness.state_mut().handle_snap_request(vec![track]);
    harness.step();

    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();

    assert!(!harness.state().snap_consent_prompt);
    assert!(
        harness.state().pending_snap.track_refs.is_empty(),
        "declining must drop the parked request"
    );
    assert!(!harness.state().snap_settings.consent_granted());
    assert_eq!(
        harness.state().snap.activity_for(track),
        None,
        "nothing may be queued without consent"
    );
}

/// The snap error series is built once per run: consecutive frames hand
/// out the same `Arc` (the plot's mipmap cache keys off it), and a new run
/// for the track produces a new one.
#[test]
fn snap_error_series_is_stable_across_frames() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    let track = ui_tests::push_file_with_travel_mode(&mut harness, "ride.gtd", None);
    test_util::snap::inject_completed_run(&mut harness, track);

    let first = harness.state_mut().snap_error_view();
    let second = harness.state_mut().snap_error_view();
    let (a, b) = (
        first.points_by_track.get(&track).expect("series present"),
        second.points_by_track.get(&track).expect("series present"),
    );
    assert!(
        Arc::ptr_eq(a, b),
        "consecutive frames must reuse the same series allocation"
    );

    // A new run for the same track invalidates: fresh points, fresh Arc.
    test_util::snap::inject_completed_run(&mut harness, track);
    let third = harness.state_mut().snap_error_view();
    let c = third.points_by_track.get(&track).expect("series present");
    assert!(
        !Arc::ptr_eq(a, c),
        "a new run must produce a new series allocation"
    );
}

/// The costing override flow: a "Snap again as" choice beats the declared
/// travel mode - a boat-declared (unsnappable) track becomes snappable
/// under the chosen costing, and clearing state on index changes does not
/// lose the content-keyed override.
#[test]
fn costing_override_beats_the_declared_mode() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    let track = ui_tests::push_file_with_travel_mode(
        &mut harness,
        "boat.gtd",
        Some(gt_types::TravelMode::Boat),
    );
    harness.run_steps(2);

    {
        let state = harness.state();
        let shared = state.shared.borrow();
        let files = shared.loaded_files.files();
        let file = track.fi.get(files).expect("file");
        let loaded = track.resolve(files).expect("track");
        assert_eq!(
            state.effective_costing(file, loaded),
            None,
            "boat: unsnappable"
        );
    }

    // Consent is pending in a fresh app, so the choice parks on the
    // consent dialog and changes nothing yet.
    let request_pedestrian = |harness: &mut Harness<'_, App>| {
        harness.state_mut().handle_snap_costing_request(
            gt_side_panel::SnapCostingTarget::Track(track),
            gt_ui_types::SnapCosting::Pedestrian,
        );
    };
    let resolved_costing = |harness: &Harness<'_, App>| {
        let state = harness.state();
        let shared = state.shared.borrow();
        let files = shared.loaded_files.files();
        let file = track.fi.get(files)?;
        let loaded = track.resolve(files)?;
        state.effective_costing(file, loaded)
    };
    request_pedestrian(&mut harness);
    assert!(harness.state().snap_consent_prompt);
    assert_eq!(harness.state().pending_snap.track_refs, vec![track]);
    assert_eq!(
        resolved_costing(&harness),
        None,
        "parked: still unsnappable"
    );

    harness.state_mut().snap_settings.acknowledge_consent();
    request_pedestrian(&mut harness);
    assert_eq!(
        resolved_costing(&harness),
        Some(gt_snap::wire::Costing::Pedestrian),
        "the override beats the road-less declaration"
    );
}

/// With consent already granted, a "Snap again as" choice must dispatch
/// under the chosen costing - not the plainly resolved one. Discriminated
/// via the cache: with runs cached under both costings (auto being the
/// displayed one), confirming the bicycle choice replaces the bicycle
/// entry and leaves auto's alone only when the dispatch actually resolves
/// the override (regression test for the dispatch ignoring it and hitting
/// the auto entry instead).
#[test]
fn costing_override_reaches_the_dispatched_run() {
    use crate::app::snap::{SnapCacheKey, SnapRun};
    use gt_snap::merge::{self, SnapWarningReporter};
    use gt_snap::request_plan::SnapParams;
    use gt_snap::wire::Costing;

    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    harness.state_mut().snap_settings.acknowledge_consent();
    let track = ui_tests::push_file_with_travel_mode(&mut harness, "ride.gtd", None);
    harness.step();

    let seeded = |harness: &Harness<'_, App>, costing: Costing| {
        let state = harness.state();
        let shared = state.shared.borrow();
        let loaded = track.resolve(shared.loaded_files.files()).expect("track");
        let params = SnapParams::new(costing);
        let key = SnapCacheKey::new(
            loaded,
            params,
            gt_snap::server_host(gt_snap::DEFAULT_SERVER_URL),
        );
        let run = SnapRun::new(
            merge::merge(
                &gt_snap::request_plan::plan(gt_types::PlacedPoints::default()),
                params,
                &[],
                &SnapWarningReporter::default(),
            ),
            Vec::new(),
            gt_snap::server_host(gt_snap::DEFAULT_SERVER_URL),
        );
        (key, run)
    };
    // Bicycle first, then auto: auto is the displayed run, bicycle sits
    // only in the dedupe cache.
    let (key, run) = seeded(&harness, Costing::Bicycle);
    harness.state_mut().snap.insert_run(key, run);
    let (key, run) = seeded(&harness, Costing::Auto);
    harness.state_mut().snap.insert_run(key, run);

    harness.state_mut().handle_snap_costing_request(
        gt_side_panel::SnapCostingTarget::Track(track),
        gt_ui_types::SnapCosting::Bicycle,
    );
    harness.run_steps(3);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Snap again")
        .click();
    harness.run_steps(3);

    let state = harness.state();
    assert!(
        !state.snap_consent_prompt,
        "consent was granted - the request must not re-prompt"
    );
    let shared = state.shared.borrow();
    let loaded = track.resolve(shared.loaded_files.files()).expect("track");
    let cached = |costing| {
        state
            .snap
            .has_cached_run(loaded, state.snap_settings.params(costing))
    };
    assert!(
        !cached(Costing::Bicycle),
        "the dispatch resolved the override, replacing the bicycle entry"
    );
    assert!(cached(Costing::Auto), "the other costing's run is kept");
}

/// A harness whose single track already has a cached auto-costing run,
/// with the auto choice requested and its dialog on screen.
fn harness_prompting_to_replace_the_auto_run<'a>() -> (Harness<'a, App>, gt_types::TrackRef) {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    harness.state_mut().snap_settings.acknowledge_consent();
    let track = ui_tests::push_file_with_travel_mode(&mut harness, "ride.gtd", None);
    harness.step();
    test_util::snap::inject_completed_run(&mut harness, track);
    harness.state_mut().handle_snap_costing_request(
        gt_side_panel::SnapCostingTarget::Track(track),
        gt_ui_types::SnapCosting::Auto,
    );
    harness.run_steps(3);
    (harness, track)
}

/// A "Snap again as" choice for a costing the track already has a run for
/// prompts before replacing it, and cancelling keeps that run.
#[test]
fn costing_choice_with_a_cached_run_prompts_before_replacing_it() {
    let (mut harness, track) = harness_prompting_to_replace_the_auto_run();

    assert_eq!(
        harness.state().snap_replace_prompt.map(|p| p.choice),
        Some(gt_ui_types::SnapCosting::Auto)
    );
    assert_eq!(
        harness.state().snap.activity_for(track),
        None,
        "nothing runs while the dialog is open"
    );

    harness.input_mut().events.push(egui::Event::Key {
        key: egui::Key::Escape,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();

    assert!(harness.state().snap_replace_prompt.is_none());
    assert!(
        test_util::snap::has_cached_auto_run(&harness, track),
        "cancelling keeps the stored run"
    );
}

/// Confirming the dialog forgets the cached run, so the choice reaches the server.
#[test]
fn confirming_the_replace_prompt_discards_the_cached_run() {
    let (mut harness, track) = harness_prompting_to_replace_the_auto_run();

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Snap again")
        .click();
    harness.run_steps(3);

    assert!(harness.state().snap_replace_prompt.is_none());
    assert!(
        !test_util::snap::has_cached_auto_run(&harness, track),
        "the confirmed choice must reach the server, not the cache"
    );
}

/// The scope dialog's counts separate the recording's selected tracks from
/// all of them, and report how many already have data for the costing.
#[test]
fn recording_scope_counts_separate_selected_from_all() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    let fi = push_two_track_file(&mut harness, "tour.gtd");
    harness.step();
    let second = gt_types::TrackRef::new(fi, gt_types::TrackIdx::new(1));
    test_util::snap::inject_completed_run(
        &mut harness,
        gt_types::TrackRef::new(fi, gt_types::TrackIdx::new(0)),
    );
    let prompt = crate::app::SnapScopePrompt {
        fi,
        choice: gt_ui_types::SnapCosting::Auto,
    };

    let unselected = harness.state().snap_scope_counts(prompt);
    assert_eq!(
        unselected.selected,
        crate::app::modals::SnapScopeCount::default()
    );

    test_util::snap::select_track(&mut harness, second);
    let counts = harness.state().snap_scope_counts(prompt);
    assert_eq!(
        counts.selected,
        crate::app::modals::SnapScopeCount {
            tracks: 1,
            already_snapped: 0
        }
    );
    assert_eq!(
        counts.all,
        crate::app::modals::SnapScopeCount {
            tracks: 2,
            already_snapped: 1
        }
    );
}

/// Each scope button runs exactly its own tracks: their cached runs for
/// the chosen costing are replaced and they take the override, while the
/// tracks outside the scope keep theirs.
#[rstest::rstest]
#[case::selected_scope("Snap selected tracks", &[1])]
#[case::all_scope("Snap all tracks", &[0, 1])]
#[case::cancel("Cancel", &[])]
fn recording_scope_dialog_snaps_the_chosen_scope(#[case] button: &str, #[case] expected: &[usize]) {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    harness.state_mut().snap_settings.acknowledge_consent();
    let fi = push_two_track_file(&mut harness, "tour.gtd");
    harness.step();
    let track = |ti| gt_types::TrackRef::new(fi, gt_types::TrackIdx::new(ti));
    test_util::snap::inject_completed_run(&mut harness, track(0));
    test_util::snap::inject_completed_run(&mut harness, track(1));
    test_util::snap::select_track(&mut harness, track(1));

    harness.state_mut().handle_snap_costing_request(
        gt_side_panel::SnapCostingTarget::Recording(fi),
        gt_ui_types::SnapCosting::Auto,
    );
    harness.run_steps(3);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, button)
        .click();
    harness.run_steps(3);

    assert!(harness.state().snap_scope_prompt.is_none());
    for ti in 0..2 {
        let in_scope = expected.contains(&ti);
        assert_eq!(
            test_util::snap::costing_override(&harness, track(ti)),
            in_scope.then_some(gt_snap::wire::Costing::Auto),
            "track {ti} override"
        );
        assert_eq!(
            test_util::snap::has_cached_auto_run(&harness, track(ti)),
            !in_scope,
            "track {ti} cached run"
        );
    }
}

/// A bulk choice on an app without consent parks the whole batch on one
/// dialog and touches nothing until it is accepted: the tracks keep their
/// runs while the question is open, and agreeing releases the batch.
#[test]
fn recording_scope_parks_the_whole_batch_on_one_consent_dialog() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    let fi = push_two_track_file(&mut harness, "tour.gtd");
    harness.step();
    let track = |ti| gt_types::TrackRef::new(fi, gt_types::TrackIdx::new(ti));
    test_util::snap::inject_completed_run(&mut harness, track(0));
    test_util::snap::inject_completed_run(&mut harness, track(1));

    harness.state_mut().handle_snap_costing_request(
        gt_side_panel::SnapCostingTarget::Recording(fi),
        gt_ui_types::SnapCosting::Auto,
    );
    harness.run_steps(3);
    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Snap all tracks")
        .click();
    harness.run_steps(3);

    assert!(harness.state().snap_consent_prompt);
    assert_eq!(harness.state().pending_snap.track_refs.len(), 2);
    for ti in 0..2 {
        assert!(
            test_util::snap::has_cached_auto_run(&harness, track(ti)),
            "track {ti} keeps its run until consent"
        );
    }

    harness
        .get_by_role_and_label(egui::accesskit::Role::Button, "Agree - snap automatically")
        .click();
    harness.run_steps(3);

    assert!(harness.state().pending_snap.track_refs.is_empty());
    for ti in 0..2 {
        assert!(
            !test_util::snap::has_cached_auto_run(&harness, track(ti)),
            "track {ti} runs once the batch is released"
        );
    }
}

/// The persistence integration end to end, against a real temporary database:
/// a completed run of a history-stored file is written into the
/// recording's snap blob via the worker, and feeding the stored blob back
/// through the response handler seeds a fresh scheduler's stores.
#[test]
fn snap_runs_persist_and_restore_through_the_app() {
    use geotrace_sdk::{
        Angle, DateTime, Duration as SdkDuration, NavFileBuilder, NavFix, NavFixTime,
    };
    use gt_store::{HistoryDatabase, Recordings};

    // One real recording so the blob has a valid group to live in.
    let t0 = DateTime::from_timestamp(1_000, 0).expect("valid timestamp");
    let mut recorder = NavFileBuilder::new().open();
    for i in 0..10i64 {
        recorder.add_nav_fix(
            NavFix::builder()
                .time(NavFixTime::Receiver(t0 + SdkDuration::seconds(i)))
                .lat(Angle::degrees(55.68))
                .lon(Angle::degrees(12.56))
                .heading(Angle::degrees(0.0))
                .build(),
        );
    }
    let nav_file = recorder.finish().expect("valid nav file");
    let mut bytes = Vec::new();
    nav_file.write(&mut bytes).expect("write bytes");

    let dir = tempfile::tempdir().expect("temp dir");
    let db_path = dir.path().join("geotrace.h5");
    let mut db = Recordings::open_or_create(&db_path).expect("open");
    let meta = gt_store::extract_meta(&bytes).expect("meta");
    let db_ref =
        test_util::recordings::insert_recording_as_one_whole_file_track(&mut db, "dev", &bytes);

    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    harness.state_mut().history = crate::app::history_db::HistoryWorker::spawn(
        RecordingsHandle::Owner(Recordings::open_or_create(&db_path).expect("reopen")),
        egui::Context::default(),
        gt_pending_writes::PendingWrites::default(),
    );

    // A loaded file associated with the stored recording, with a completed
    // run in the session stores.
    let track = ui_tests::push_file_with(
        &mut harness,
        "ride.gtd",
        None,
        gt_loaded_files::FileHistory::recording("dev".to_owned(), meta, Some(db_ref.clone())),
    );
    test_util::snap::inject_completed_run(&mut harness, track);

    // Persist leg: the worker writes the recording's blob.
    let content = {
        let state = harness.state();
        let shared = state.shared.borrow();
        let loaded = track
            .resolve(shared.loaded_files.files())
            .expect("track present");
        crate::app::snap::TrackContentKey::new(loaded)
    };
    harness.state().persist_snap_runs(&[content]);
    let blob = harness
        .step_until_some(|_| {
            Recordings::open_or_create(&db_path)
                .ok()
                .and_then(|db| db.snap_blob(&db_ref).ok())
                .flatten()
        })
        .expect("the history worker stored the snap blob");

    // Restore leg: a fresh scheduler seeded through the response handler.
    harness.state_mut().snap = crate::app::snap::SnapScheduler::new(
        egui::Context::default(),
        gt_fetch::TransportSource::Offline,
        true,
    );
    harness
        .state_mut()
        .handle_history_response(crate::app::history_db::Response::SnapRunsLoaded {
            db_ref,
            blob: Ok(Some(blob)),
        });
    let state = harness.state();
    let shared = state.shared.borrow();
    let loaded = track
        .resolve(shared.loaded_files.files())
        .expect("track present");
    let restored = state
        .snap
        .latest_run_for(loaded)
        .expect("stored run restored into the fresh session");
    assert_eq!(restored.result.points.len(), 60);
}

/// The map's snapped-track geometry follows the completed run's toggle and
/// the track's tree visibility - hidden either way means no entry.
#[test]
fn snapped_tracks_view_respects_toggle_and_tree_visibility() {
    let mut harness = Harness::builder()
        .with_wait_for_pending_images(false)
        .build_eframe(test_util::harness::transient_app);
    harness.step();
    let track = ui_tests::push_file_with_travel_mode(&mut harness, "ride.gtd", None);
    test_util::snap::inject_completed_run(&mut harness, track);

    let view = harness.state().snapped_tracks_view();
    let geometry = view
        .get(track)
        .expect("a shown completed run must reach the map");
    assert_eq!(
        geometry.segments.len(),
        1,
        "one snapped segment was injected"
    );
    assert_eq!(
        geometry.segments.first().map(|s| s.points.len()),
        Some(2),
        "both positions must be projected"
    );

    // Toggled hidden: gone from the map view.
    harness.state_mut().hidden_snapped.insert(track);
    assert!(harness.state().snapped_tracks_view().is_empty());
    harness.state_mut().hidden_snapped.remove(&track);

    // Track unchecked in the tree: gone as well.
    harness
        .state()
        .shared
        .borrow_mut()
        .tree
        .toggle_track_check(track);
    assert!(harness.state().snapped_tracks_view().is_empty());
}
