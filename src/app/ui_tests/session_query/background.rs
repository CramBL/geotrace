use std::sync::mpsc;
use std::time::{Duration, Instant};

use egui_kittest::Harness;
use gt_fetch::TransportSource;
use gt_session_bridge::QueryJobState;
use gt_snap::request_plan::SnapParams;
use gt_snap::wire::Costing;
use gt_store::{SolarStore, Store};
use gt_types::{FileIdx, TrackIdx, TrackRef};
use rstest::rstest;

use super::{App, ui_tests};
use crate::app::snap::{SnapActivity, SnapPriority, SnapScheduler};
use crate::app::solar::TEST_KP_WORKER_NAME_PREFIX;
use crate::app::test_util;

#[derive(Clone, Copy)]
enum QueryInput {
    Kp,
    Snap,
}

#[rstest]
#[case::geomagnetic_archive(QueryInput::Kp, "points | where kp > 0", 1)]
#[case::snap_run(QueryInput::Snap, "points | where snap_error >= 2 m", 2)]
fn a_minimized_app_uses_new_query_inputs(
    #[case] input: QueryInput,
    #[case] text: &str,
    #[case] matches: usize,
) {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = ui_tests::app_with_a_recording();
    if matches!(input, QueryInput::Kp) {
        let archive = Store::open_in(dir.path())
            .open_or_create_archive::<SolarStore>()
            .unwrap();
        harness
            .state_mut()
            .geomagnetic_indices
            .adopt_store(Some(archive));
    }
    let first = super::start(&mut harness, text);
    self::poll_minimized_until_query_finishes(&mut harness);
    let before = super::status(&mut harness, first);
    let QueryJobState::Completed { queries } = &before else {
        panic!("expected completed query, got {before:?}");
    };
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].matches, 0);

    let (woke, repaint) = mpsc::channel();
    let caller = std::thread::current().id();
    harness.ctx.set_repaint_observer(move |info| {
        let thread = std::thread::current();
        let completion_source = match input {
            QueryInput::Kp => thread
                .name()
                .is_some_and(|name| name.starts_with(TEST_KP_WORKER_NAME_PREFIX)),
            QueryInput::Snap => thread.id() == caller,
        };
        if completion_source && info.delay.is_zero() {
            woke.send(info.viewport_id).ok();
        }
    });
    match input {
        QueryInput::Kp => harness
            .state()
            .geomagnetic_indices
            .archive_kp_from_test_worker(ui_tests::base_time().date_naive()),
        QueryInput::Snap => test_util::snap::report_completed_run(
            &harness,
            TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        ),
    }
    assert_eq!(
        repaint.recv_timeout(Duration::from_secs(5)).unwrap(),
        egui::ViewportId::ROOT
    );
    assert_eq!(super::status(&mut harness, first), before);

    if matches!(input, QueryInput::Kp) {
        test_util::harness::step_minimized_logic(&mut harness);
        assert_eq!(super::status(&mut harness, first), QueryJobState::Stale);
    }
    let next = super::start(&mut harness, text);
    self::poll_minimized_until_query_finishes(&mut harness);
    let after = super::status(&mut harness, next);
    let QueryJobState::Completed { queries } = &after else {
        panic!("expected completed query, got {after:?}");
    };
    assert_eq!(queries.len(), 1);
    assert_eq!(queries[0].matches, matches);
}

fn poll_minimized_until_query_finishes(harness: &mut Harness<'_, App>) {
    let deadline = Instant::now() + Duration::from_secs(5);
    while harness.state().session_query.is_active() {
        test_util::harness::step_minimized_logic(harness);
        assert!(Instant::now() < deadline, "timed out waiting for query");
        std::thread::sleep(Duration::from_millis(5));
    }
}

#[test]
fn a_close_request_prevents_a_queued_archive_fetch_from_starting() {
    let dir = tempfile::tempdir().unwrap();
    let mut harness = ui_tests::app_with_a_recording();
    let archive = Store::open_in(dir.path())
        .open_or_create_archive::<SolarStore>()
        .unwrap();
    let scheduler = &mut harness.state_mut().geomagnetic_indices;
    scheduler.adopt_store(Some(archive));
    scheduler
        .fetch_queue_mut()
        .queue_track_day(ui_tests::base_time().date_naive());
    assert_eq!(scheduler.fetch_queue().queued(), 1);

    ui_tests::request_window_close(&mut harness);
    harness.step();

    let app = harness.state();
    assert!(app.shutdown.has_begun());
    assert_eq!(app.geomagnetic_indices.fetch_queue().queued(), 1);
    assert!(!app.geomagnetic_indices.fetch_queue().is_fetching());
}

#[test]
fn a_minimized_close_adopts_a_snap_result_without_starting_the_queued_run() {
    let mut harness = ui_tests::app_with_a_recording();
    let finished = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    let queued = ui_tests::push_file_with_travel_mode(&mut harness, "queued.gtd", None);
    harness.state_mut().snap =
        SnapScheduler::new(harness.ctx.clone(), TransportSource::Offline, false);
    {
        let app = harness.state_mut();
        let shared = app.shared.borrow();
        let track = queued.resolve(shared.loaded_files.files()).unwrap();
        app.snap.set_run_in_flight_for_test(finished);
        app.snap.request_snap(
            queued,
            track,
            SnapParams::new(Costing::Auto),
            SnapPriority::Manual,
        );
        assert_eq!(app.snap.activity_for(queued), Some(&SnapActivity::Queued));
    }
    test_util::snap::report_completed_run(&harness, finished);

    ui_tests::request_window_close(&mut harness);
    test_util::harness::step_minimized_logic(&mut harness);

    let app = harness.state();
    let shared = app.shared.borrow();
    let track = finished.resolve(shared.loaded_files.files()).unwrap();
    assert!(app.shutdown.has_begun());
    assert!(app.snap.latest_run_for(track).is_some());
    assert_eq!(app.snap.activity_for(queued), Some(&SnapActivity::Queued));
}
