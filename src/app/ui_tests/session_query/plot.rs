use std::ops::Range;

use chrono::TimeDelta;
use egui::accesskit::Role;
use gt_loaded_files::FileHistory;
use gt_session_bridge::{
    ErrorCode, QueryJobState, QueryMatchPosition, RangeTarget, Reply, Request, ShowOnMap,
    ShowOnPlot, TimeRange, TrackReference,
};
use gt_test_utils::{Queryable as _, fixtures};
use gt_types::{FileIdx, MetricKind};
use rstest::rstest;

use crate::app::SharedAppState;
use crate::app::test_util::{harness, session};
use crate::app::ui_tests;

#[test]
fn the_plot_frames_filtered_fix_times_across_a_clock_reversal() {
    let mut harness = ui_tests::app_with_a_recording();
    let base = ui_tests::base_time();
    let fraction = TimeDelta::nanoseconds(123);
    let points = [23_500, 40_000, 21_250, 24_750, 22_500]
        .into_iter()
        .flat_map(|offset| {
            fixtures::nav_points_from(base + TimeDelta::milliseconds(offset) + fraction, 1, 1)
        })
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
        Reply::PlotFramed {
            track,
            time_range: TimeRange {
                start: base + TimeDelta::milliseconds(21_250) + fraction,
                end: base + TimeDelta::milliseconds(24_750) + fraction,
            },
        }
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
    let base = base.timestamp() as f64 + fraction.as_seconds_f64();
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
    let time = ui_tests::base_time();
    let reply = Reply::PlotFramed {
        track,
        time_range: TimeRange {
            start: time + TimeDelta::seconds(20),
            end: time + TimeDelta::seconds(23),
        },
    };
    assert_eq!(session::execute(&mut harness, frame.clone()), reply);
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

    assert_eq!(session::execute(&mut harness, frame), reply);
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
    let track = session::first_track_reference(&mut harness);
    harness.set_size(egui::vec2(100.0, 60.0));
    let mut reply = session::begin(
        &mut harness,
        Request::ShowOnPlot(ShowOnPlot {
            target,
            metrics: vec!["eph".to_owned()],
        }),
    );
    harness.step();
    assert_eq!(reply.try_recv().unwrap(), None);
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
    let expected = if replace_job {
        ErrorCode::PlotPresentationStale.into()
    } else {
        Reply::PlotFramed {
            track,
            time_range: TimeRange {
                start: point_at,
                end: point_at,
            },
        }
    };
    assert_eq!(reply.try_recv().unwrap(), Some(expected));
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
    assert!(matches!(
        session::execute(
            &mut harness,
            Request::ShowOnPlot(ShowOnPlot {
                target,
                metrics: Vec::new()
            })
        ),
        Reply::PlotFramed { track: framed, .. } if framed == track
    ));
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::Button, "Dismiss plot presentation");
    let hide = harness
        .get_by_role_and_label(Role::Button, "Hide plot")
        .rect()
        .center();
    harness.hover_at(hide);
    harness.step();
    let mut pending = session::begin(&mut harness, self::request(track, 25..28, &["eph"]));
    for pressed in [true, false] {
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: hide,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
        if pressed {
            assert_eq!(pending.try_recv().unwrap(), None);
        }
    }
    assert_eq!(
        pending.try_recv().unwrap(),
        Some(ErrorCode::PlotPresentationSuperseded.into())
    );
    harness.step();
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

#[rstest]
#[case::expired(false)]
#[case::cancelled(true)]
fn an_offscreen_plot_request_leaves_the_previous_view(#[case] cancel: bool) {
    let mut harness = ui_tests::app_with_a_recording();
    let track = session::first_track_reference(&mut harness);
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .plot_state
        .sync_to_map = false;
    assert!(matches!(
        session::execute(&mut harness, self::request(track, 20..24, &[])),
        Reply::PlotFramed { .. }
    ));
    harness.run_steps(2);
    let old_view = harness.state().shared.borrow().plot_state.visible_x_range();
    let app = harness.state_mut();
    app.tiles_tree.tiles.set_visible(app.plot_tile_id, false);
    app.shared
        .borrow_mut()
        .plot_state
        .metric_vis
        .set(MetricKind::Eph, false);
    let mut reply = session::begin(&mut harness, self::request(track, 40..44, &["eph"]));
    assert!(harness.state().plot_is_visible());
    let _ = harness::step_minimized_logic(&mut harness);
    assert_eq!(reply.try_recv().unwrap(), None);
    assert!(matches!(
        session::execute(&mut harness, Request::GetSessionState),
        Reply::SessionState { .. }
    ));
    let receive = if cancel {
        drop(reply);
        None
    } else {
        harness
            .state_mut()
            .shared
            .borrow_mut()
            .pending_session_plot
            .as_mut()
            .unwrap()
            .expire();
        Some(reply)
    };
    let _ = harness::step_minimized_logic(&mut harness);
    if let Some(mut reply) = receive {
        assert_eq!(
            reply.try_recv().unwrap(),
            Some(ErrorCode::PlotPresentationTimedOut.into())
        );
    }
    assert!(
        harness
            .state()
            .shared
            .borrow()
            .pending_session_plot
            .is_none()
    );
    harness.run_steps(2);
    harness.get_by_label_contains("Track 1 / Points 20 to 23");
    let shared = harness.state().shared.borrow();
    assert!(!shared.plot_state.metric_vis.field(MetricKind::Eph));
    assert_eq!(shared.plot_state.visible_x_range(), old_view);
}

#[test]
fn only_the_newer_pending_plot_request_is_framed() {
    let mut harness = ui_tests::app_with_a_recording();
    let track = session::first_track_reference(&mut harness);
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .plot_state
        .metric_vis
        .set(MetricKind::Eph, false);
    let mut first = session::begin(&mut harness, self::request(track, 20..24, &["eph"]));
    let mut second = session::begin(&mut harness, self::request(track, 40..44, &["velocity"]));
    assert_eq!(
        first.try_recv().unwrap(),
        Some(ErrorCode::PlotPresentationSuperseded.into())
    );
    assert_eq!(second.try_recv().unwrap(), None);
    harness.step();
    let time = ui_tests::base_time();
    assert_eq!(
        second.try_recv().unwrap(),
        Some(Reply::PlotFramed {
            track,
            time_range: TimeRange {
                start: time + TimeDelta::seconds(40),
                end: time + TimeDelta::seconds(43),
            },
        })
    );
    let shared = harness.state().shared.borrow();
    assert!(!shared.plot_state.metric_vis.field(MetricKind::Eph));
    assert!(shared.plot_state.metric_vis.field(MetricKind::Velocity));
}

#[derive(Clone, Copy)]
enum CompetingPlotInput {
    AxisDrag,
    MapWheel,
    PlotWheel,
}

#[rstest]
#[case::plot_wheel(CompetingPlotInput::PlotWheel)]
#[case::axis_drag(CompetingPlotInput::AxisDrag)]
#[case::map_wheel(CompetingPlotInput::MapWheel)]
fn plot_navigation_supersedes_framing_with_plot_owned_input(#[case] input: CompetingPlotInput) {
    let mut harness = ui_tests::app_with_a_recording();
    let track = session::first_track_reference(&mut harness);
    {
        let mut shared = harness.state_mut().shared.borrow_mut();
        shared.plot_state.sync_to_map = false;
        shared.plot_state.metric_vis.set(MetricKind::Eph, false);
    }
    assert!(matches!(
        session::execute(&mut harness, self::request(track, 20..24, &[])),
        Reply::PlotFramed { .. }
    ));
    harness.run_steps(2);
    let in_plot = !matches!(input, CompetingPlotInput::MapWheel);
    let app = harness.state();
    let tile = if in_plot {
        app.plot_tile_id
    } else {
        app.map_tile_id
    };
    let pane = app.tiles_tree.tiles.rect(tile).unwrap();
    let y = if matches!(input, CompetingPlotInput::AxisDrag) {
        pane.bottom() - 12.0
    } else {
        egui::lerp(pane.y_range(), 0.75)
    };
    let pos = egui::pos2(pane.center().x, y);
    harness.hover_at(pos);
    harness.step();
    if matches!(input, CompetingPlotInput::AxisDrag) {
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
    }
    let mut reply = session::begin(&mut harness, self::request(track, 40..44, &["eph"]));
    let event = match input {
        CompetingPlotInput::AxisDrag => egui::Event::PointerMoved(pos + egui::vec2(12.0, 0.0)),
        CompetingPlotInput::PlotWheel | CompetingPlotInput::MapWheel => egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            source: egui::MouseWheelSource::Wheel,
            delta: egui::vec2(-60.0, 0.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        },
    };
    harness.input_mut().events.push(event);
    harness.step();
    let time = ui_tests::base_time();
    let expected = if in_plot {
        ErrorCode::PlotPresentationSuperseded.into()
    } else {
        Reply::PlotFramed {
            track,
            time_range: TimeRange {
                start: time + TimeDelta::seconds(40),
                end: time + TimeDelta::seconds(43),
            },
        }
    };
    assert_eq!(reply.try_recv().unwrap(), Some(expected));
    let shared = harness.state().shared.borrow();
    let range = shared.plot_state.visible_x_range().unwrap();
    let start = time.timestamp() as f64;
    if in_plot {
        assert_ne!(range, start + 19.0..=start + 24.0);
        assert!(*range.end() < start + 30.0);
    } else {
        assert_eq!(range, start + 39.0..=start + 44.0);
    }
    assert_eq!(
        shared.plot_state.metric_vis.field(MetricKind::Eph),
        !in_plot
    );
}

fn request(track: TrackReference, points: Range<usize>, metrics: &[&str]) -> Request {
    Request::ShowOnPlot(ShowOnPlot {
        target: RangeTarget::Track {
            track,
            start: Some(points.start),
            end: Some(points.end),
        },
        metrics: metrics.iter().map(|metric| (*metric).to_owned()).collect(),
    })
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
