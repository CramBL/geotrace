use chrono::TimeDelta;
use egui::accesskit::Role;
use gt_loaded_files::FileHistory;
use gt_session_bridge::{
    ErrorCode, QueryJobState, QueryMatchPosition, RangeTarget, Reply, Request, ShowOnMap,
    ShowOnPlot,
};
use gt_test_utils::{Queryable as _, fixtures};
use gt_types::{FileIdx, MetricKind};
use rstest::rstest;

use crate::app::SharedAppState;
use crate::app::test_util::session;
use crate::app::ui_tests;

#[test]
fn the_plot_frames_filtered_fix_times_across_a_clock_reversal() {
    let mut harness = ui_tests::app_with_a_recording();
    let base = ui_tests::base_time();
    let points = [23_500, 40_000, 21_250, 24_750, 22_500]
        .into_iter()
        .flat_map(|offset| fixtures::nav_points_from(base + TimeDelta::milliseconds(offset), 1, 1))
        .collect();
    let mut track = gt_test_utils::loaded_track_with_points(points);
    track.metadata.index = 1;
    let mut recording = gt_test_utils::loaded_file_with_tracks(vec![track]);
    recording.metadata.filename = "reversal.gtd".to_owned();
    let app = harness.state_mut();
    app.tiles_tree.tiles.set_visible(app.plot_tile_id, false);
    {
        let mut shared = app.shared.borrow_mut();
        shared
            .loaded_files
            .replace_file(0, recording, FileHistory::None);
        shared.filter.set_time_bounds(
            Some(base + TimeDelta::seconds(21)),
            Some(base + TimeDelta::seconds(30)),
        );
        let SharedAppState {
            loaded_files,
            plot_state,
            ..
        } = &mut *shared;
        plot_state.rebuild_all(loaded_files.files());
        plot_state.sync_to_map = true;
        plot_state.metric_vis.set(MetricKind::Eph, false);
        plot_state.metric_vis.set(MetricKind::Velocity, false);
    }
    let track = session::first_track_reference(&mut harness);
    let target = RangeTarget::Track {
        track,
        start: Some(0),
        end: Some(5),
    };
    let request = Request::ShowOnPlot(ShowOnPlot {
        target,
        metrics: vec!["eph".to_owned(), "util_all".to_owned()],
    });
    assert_eq!(
        session::execute(&mut harness, request),
        Reply::PlotPresentationQueued
    );
    assert!(harness.state().plot_is_visible());
    harness.run_steps(3);
    harness.get_by_label("reversal.gtd / Track 1 / Points 0 to 4");
    harness
        .get_by_role_and_label(Role::Button, "Dismiss plot presentation")
        .click();
    harness.run_steps(2);
    assert!(
        harness
            .query_by_label("Dismiss plot presentation")
            .is_none()
    );
    let shared = harness.state().shared.borrow();
    let state = &shared.plot_state;
    let base = base.timestamp() as f64;
    assert_eq!(state.visible_x_range(), Some(base + 20.25..=base + 25.75));
    assert!(state.sync_to_map);
    assert!(state.show_advanced_metrics);
    assert!(state.metric_vis.field(MetricKind::Eph));
    assert!(!state.metric_vis.field(MetricKind::Velocity));
    assert!(shared.pending_session_plot.is_none());
}

#[test]
fn the_plot_syncs_again_when_the_map_time_range_changes() {
    let mut harness = ui_tests::app_with_a_recording();
    let track = session::first_track_reference(&mut harness);
    let target = RangeTarget::Track {
        track,
        start: Some(20),
        end: Some(24),
    };
    let frame = Request::ShowOnPlot(ShowOnPlot {
        target,
        metrics: Vec::new(),
    });
    assert_eq!(
        session::execute(&mut harness, frame.clone()),
        Reply::PlotPresentationQueued
    );
    harness.run_steps(3);
    let base = ui_tests::base_time().timestamp() as f64;
    let framed = base + 19.0..=base + 24.0;
    assert_eq!(
        harness.state().shared.borrow().plot_state.visible_x_range(),
        Some(framed.clone())
    );
    harness.get_by_label_contains("Track 1 / Points 20 to 23");

    let target = RangeTarget::Track {
        track,
        start: Some(40),
        end: Some(45),
    };
    let map = Request::ShowOnMap(ShowOnMap { target });
    assert_eq!(
        session::execute(&mut harness, map.clone()),
        Reply::MapRangeFramed {
            track,
            start: 40,
            end: 45
        }
    );
    harness.run_steps(3);
    let shared = harness.state().shared.borrow();
    let (start, end) = shared.map_synced_plot_range.scanned_range().unwrap();
    let map_range = start..=end;
    assert_ne!(map_range, framed);
    assert_eq!(shared.plot_state.visible_x_range(), Some(map_range.clone()));
    assert!(shared.plot_state.sync_to_map);
    drop(shared);
    assert!(
        harness
            .query_by_label("Dismiss plot presentation")
            .is_none()
    );

    assert_eq!(
        session::execute(&mut harness, frame),
        Reply::PlotPresentationQueued
    );
    harness.step();
    harness.state().shared.borrow_mut().map_center_request = Some((0.0, 0.0));
    harness.run_steps(3);
    {
        let shared = harness.state().shared.borrow();
        assert_eq!(shared.map_synced_plot_range.scanned_range(), None);
        assert_eq!(shared.plot_state.visible_x_range(), Some(framed));
    }

    assert_eq!(
        session::execute(&mut harness, map),
        Reply::MapRangeFramed {
            track,
            start: 40,
            end: 45
        }
    );
    harness.run_steps(3);
    let shared = harness.state().shared.borrow();
    assert_eq!(
        shared.map_synced_plot_range.scanned_range(),
        Some((start, end))
    );
    assert_eq!(shared.plot_state.visible_x_range(), Some(map_range));
}

#[rstest]
#[case::current_job(false)]
#[case::replaced_job(true)]
fn a_query_plot_request_waits_for_a_drawable_pane(#[case] replace_job: bool) {
    let mut harness = ui_tests::app_with_a_recording();
    let point_at = ui_tests::base_time() + TimeDelta::seconds(20);
    {
        let mut shared = harness.state_mut().shared.borrow_mut();
        shared
            .filter
            .set_time_bounds(Some(point_at), Some(point_at));
        shared.plot_state.metric_vis.set(MetricKind::Eph, false);
    }
    let job = session::start_query(&mut harness, "points | where velocity > 1 km/h");
    assert!(matches!(
        session::wait_for_query_to_stop(&mut harness, job),
        QueryJobState::Completed { .. }
    ));
    let target = RangeTarget::QueryMatch {
        job,
        query: 0,
        matched: QueryMatchPosition::default(),
    };
    harness.set_size(egui::vec2(100.0, 60.0));
    assert_eq!(
        session::execute(
            &mut harness,
            Request::ShowOnPlot(ShowOnPlot {
                target,
                metrics: vec!["eph".to_owned()]
            })
        ),
        Reply::PlotPresentationQueued
    );
    harness.step();
    assert!(!session::visible_action_notices(&harness).contains(&"MCP framed the plot"));
    assert!(
        harness
            .query_by_label("Dismiss plot presentation")
            .is_none()
    );
    assert!(
        harness
            .state()
            .shared
            .borrow()
            .pending_session_plot
            .is_some()
    );
    if replace_job {
        assert_ne!(session::start_query(&mut harness, "points"), job);
        harness.step();
        assert!(
            harness
                .state()
                .shared
                .borrow()
                .pending_session_plot
                .is_none()
        );
    }
    harness.set_size(egui::vec2(1024.0, 700.0));
    harness.run_steps(3);
    assert_eq!(
        session::visible_action_notices(&harness).contains(&"MCP framed the plot"),
        !replace_job
    );
    assert_eq!(
        harness
            .query_by_label_contains("Track 1 / Query 1 match 1")
            .is_some(),
        !replace_job
    );
    let shared = harness.state().shared.borrow();
    let point = point_at.timestamp() as f64;
    let extent = point - 1.0..=point + 1.0;
    assert_eq!(
        shared.plot_state.metric_vis.field(MetricKind::Eph),
        !replace_job
    );
    if !replace_job {
        assert_eq!(shared.plot_state.visible_x_range(), Some(extent));
    }
    assert!(shared.pending_session_plot.is_none());
    drop(shared);
    if !replace_job {
        assert_ne!(session::start_query(&mut harness, "points"), job);
        harness.run_steps(2);
        assert!(
            harness
                .query_by_label("Dismiss plot presentation")
                .is_none()
        );
    }
}

#[test]
fn hiding_the_plot_clears_its_applied_presentation() {
    let mut harness = ui_tests::app_with_a_recording();
    let track = session::first_track_reference(&mut harness);
    let target = RangeTarget::Track {
        track,
        start: Some(20),
        end: Some(24),
    };
    assert_eq!(
        session::execute(
            &mut harness,
            Request::ShowOnPlot(ShowOnPlot {
                target,
                metrics: Vec::new()
            })
        ),
        Reply::PlotPresentationQueued,
    );
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::Button, "Dismiss plot presentation");
    harness
        .get_by_role_and_label(Role::Button, "Hide plot")
        .click();
    harness.run_steps(2);
    assert!(!harness.state().plot_is_visible());
    assert!(
        harness
            .state()
            .shared
            .borrow()
            .presented_session_plot
            .is_none()
    );
    assert!(
        harness
            .query_by_label("Dismiss plot presentation")
            .is_none()
    );
}

#[test]
fn a_plot_request_requires_plot_metrics_and_visible_times() {
    let mut harness = ui_tests::app_with_a_recording();
    let track = session::first_track_reference(&mut harness);
    let target = RangeTarget::Track {
        track,
        start: Some(3),
        end: Some(4),
    };
    let app = harness.state_mut();
    app.tiles_tree.tiles.set_visible(app.plot_tile_id, false);
    assert_eq!(
        session::execute(
            &mut harness,
            Request::ShowOnPlot(ShowOnPlot {
                target,
                metrics: vec!["time".to_owned()]
            }),
        ),
        ErrorCode::InvalidPlotMetric.into()
    );
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .tree
        .toggle_file_check(FileIdx::new(0));
    assert_eq!(
        session::execute(
            &mut harness,
            Request::ShowOnPlot(ShowOnPlot {
                target,
                metrics: Vec::new()
            }),
        ),
        ErrorCode::NoVisiblePlotTime.into()
    );
    assert!(!harness.state().plot_is_visible());
    assert!(
        harness
            .state()
            .shared
            .borrow()
            .pending_session_plot
            .is_none()
    );
}
