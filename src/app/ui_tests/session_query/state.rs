use chrono::TimeDelta;
use egui_kittest::Harness;
use gt_filter::TimeWindow;
use gt_map::MapLayer;
use gt_session_bridge::{
    ListRecordings, OpenPoint, QueryJobRequest, Reply, Request, SessionMapLayer, SessionPanel,
    SessionQueryPhase, SessionQueryState, SessionRequest, SessionState, SessionTimeFilter,
};
use gt_types::{FileIdx, TrackIdx, TrackRef};
use gt_ui_types::{DisplayCategory, SkyTrailsRequest};

use crate::app::test_util::{harness, session};
use crate::app::{App, ui_tests};

#[test]
fn an_unpainted_desktop_has_no_map_bounds() {
    let mut context = eframe::CreationContext::_new_kittest(egui::Context::default());
    let mut app = harness::transient_app(&mut context);
    let Reply::SessionState { state } = app
        .execute_session_request(SessionRequest {
            request: Request::GetSessionState,
            session: session::SESSION_ID,
        })
        .unwrap()
    else {
        panic!("expected desktop session state");
    };
    assert_eq!(state.map.bounds, None);
    assert_eq!(state.map.pinned_point, None);
    assert_eq!(state.query, None);
    assert_eq!(state.recording_count, 0);
    assert!(state.recording_preview.recordings.is_empty());
    assert_eq!(state.time_filter, SessionTimeFilter::All);
}

#[test]
fn an_empty_desktop_reports_the_viewport_around_its_default_center() {
    let mut harness = Harness::builder()
        .with_context_setup(gt_ui_theme::install_app_style)
        .with_wait_for_pending_images(false)
        .build_eframe(harness::transient_app);
    let state = self::session_state(&mut harness);
    assert_eq!(state.recording_count, 0);
    let bounds = state.map.bounds.unwrap();
    assert!(
        (bounds.south_deg..=bounds.north_deg).contains(&55.676),
        "{bounds:?}"
    );
    assert!(
        (bounds.west_deg..=bounds.east_deg).contains(&12.565),
        "{bounds:?}"
    );
}

#[test]
fn session_state_reads_panel_and_filter_changes_without_painting() {
    let mut harness = ui_tests::app_with_a_recording();
    let Reply::Recordings { page } = session::execute(
        &mut harness,
        Request::ListRecordings(ListRecordings {
            after: None,
            limit: None,
        }),
    ) else {
        panic!("expected loaded recordings");
    };
    let before = self::session_state(&mut harness);
    assert_eq!(before.recording_count, 1);
    assert_eq!(before.recording_preview, page);
    let app = harness.state_mut();
    let before_toasts = app.toasts.len();
    app.history_window.open = true;
    app.log_viewer.open = true;
    app.settings_open = true;
    app.sky_trails_window.open(SkyTrailsRequest {
        track: TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        at: None,
    });
    app.tiles_tree.set_visible(app.plot_tile_id, false);
    app.map.set_layer(MapLayer::Satellite);
    let end = ui_tests::base_time() + TimeDelta::microseconds(123_456);
    {
        let mut shared = app.shared.borrow_mut();
        shared.display_mask.solo(DisplayCategory::TrackPoints);
        shared.filter.set_time_bounds(None, Some(end));
    }
    let changed = self::session_state(&mut harness);
    assert_eq!(
        changed.open_panels,
        [
            SessionPanel::History,
            SessionPanel::LogViewer,
            SessionPanel::Settings,
            SessionPanel::SkyTrails
        ]
    );
    assert_eq!(
        changed.time_filter,
        SessionTimeFilter::Bounds {
            start: None,
            end: Some(end),
            start_unrepresentable: false,
            end_unrepresentable: false,
        }
    );
    assert_eq!(changed.map.base_layer, SessionMapLayer::Satellite);
    assert_eq!(
        changed.map.visible_categories,
        [DisplayCategory::TrackPoints]
    );
    assert_eq!(changed.map.bounds, before.map.bounds);
    assert_eq!(harness.state().toasts.len(), before_toasts);

    harness
        .state_mut()
        .shared
        .borrow_mut()
        .filter
        .set_time_bounds(Some(end + TimeDelta::seconds(1)), Some(end));
    assert_eq!(
        self::session_state(&mut harness).time_filter,
        SessionTimeFilter::Empty
    );
}

#[test]
fn session_state_reports_applied_map_geometry_and_current_pins() {
    let mut harness = ui_tests::app_with_a_recording();
    let reference = session::first_track_reference(&mut harness);
    let point = OpenPoint {
        track: reference,
        point: 1,
    };
    assert_eq!(
        session::execute(&mut harness, Request::OpenPoint(point)),
        Reply::MapPresentationQueued
    );
    assert_eq!(self::session_state(&mut harness).map.pinned_point, None);
    harness.run_steps(2);
    let displayed = self::session_state(&mut harness);
    assert_eq!(displayed.map.pinned_point, Some(point));
    let bounds = displayed.map.bounds.unwrap();
    let viewport = harness.state().map.viewport_geo_bounds().unwrap();
    assert_eq!(
        (bounds.south_deg, bounds.north_deg),
        (viewport.lat_min, viewport.lat_max)
    );
    assert_eq!(
        (bounds.west_deg, bounds.east_deg),
        (viewport.lon_min, viewport.lon_max)
    );

    let app = harness.state_mut();
    app.tiles_tree.set_visible(app.map_tile_id, false);
    app.shared
        .borrow_mut()
        .filter
        .set_time_window(TimeWindow::Empty);
    let hidden = self::session_state(&mut harness);
    assert_eq!(hidden.map.bounds, None);
    assert_eq!(hidden.map.pinned_point, Some(point));
    harness.state_mut().apply_resegmentation();
    let after_processing = self::session_state(&mut harness);
    assert_eq!(after_processing.map.pinned_point, None);
    assert_eq!(
        after_processing.recording_preview.recordings[0].id,
        point.track.recording
    );
}

#[test]
fn the_summary_reports_shared_query_phase_without_assigning_a_job() {
    let mut harness = ui_tests::app_with_query_window_open();
    ui_tests::run_query(&mut harness, "points | table velocity");
    let desktop = self::session_state(&mut harness);
    assert!(desktop.open_panels.contains(&SessionPanel::Query));
    assert_eq!(
        desktop.query,
        Some(SessionQueryState {
            phase: SessionQueryPhase::Completed,
            job: None
        })
    );

    let job = session::start_query(&mut harness, "points | table velocity");
    assert_eq!(
        self::session_state(&mut harness).query,
        Some(SessionQueryState {
            phase: SessionQueryPhase::Preparing,
            job: Some(job)
        })
    );
    assert!(matches!(
        session::execute(&mut harness, Request::CancelQuery(QueryJobRequest { job })),
        Reply::QueryJob { .. }
    ));
    assert_eq!(
        self::session_state(&mut harness).query,
        Some(SessionQueryState {
            phase: SessionQueryPhase::Cancelled,
            job: Some(job)
        })
    );
}

fn session_state(harness: &mut Harness<'_, App>) -> SessionState {
    let Reply::SessionState { state } = session::execute(harness, Request::GetSessionState) else {
        panic!("expected desktop session state");
    };
    state
}
