use chrono::TimeDelta;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::NodeT as _;
use gt_pending_writes::WriteKind;
use gt_session_bridge::{
    ErrorCode, GetQueryResults, OpenPoint, QueryJobId, QueryJobState, QueryMatchPosition,
    QueryPageRequest, RangeTarget, Reply, Request, ShowOnMap,
};
use gt_test_utils::{
    HarnessInteraction as _, Queryable as _, SyntheticGtdSpec, recording_fixtures,
};
use gt_types::{
    DataCategory, FileIdx, FixRef, Latitude, Longitude, PointIdx, TrackIdx, TrackRef, mercator,
};
use gt_ui_types::{DisplayCategory, MapElementRef, MatchHighlight, MatchRevealTarget};
use rstest::rstest;
use uuid::Uuid;

use crate::app::test_util::{harness, session};
use crate::app::{App, ui_tests};

#[test]
fn session_map_frames_a_range_when_the_map_draws() {
    let mut harness = ui_tests::app_with_a_recording();
    let reference = session::first_track_reference(&mut harness);
    let initial = harness.state().map.viewport_geo_bounds().unwrap();
    let app = harness.state_mut();
    app.tiles_tree.set_visible(app.map_tile_id, false);
    app.shared
        .borrow_mut()
        .filter
        .set_time_bounds(Some(ui_tests::base_time() + TimeDelta::seconds(30)), None);

    let mut reply = session::begin(
        &mut harness,
        Request::ShowOnMap(ShowOnMap {
            target: RangeTarget::Track {
                track: reference,
                start: Some(0),
                end: Some(33),
            },
        }),
    );
    harness.step();
    assert_eq!(reply.try_recv().unwrap(), None);
    assert_eq!(
        harness.state().shared.borrow().highlight.presented_match,
        None
    );
    assert!(harness.query_by_label("Dismiss map presentation").is_none());
    assert!(session::visible_action_notices(&harness).is_empty());

    let app = harness.state_mut();
    app.tiles_tree.set_visible(app.map_tile_id, true);
    harness.run_steps(2);
    assert_eq!(
        reply.try_recv().unwrap(),
        Some(Reply::MapRangeFramed {
            track: reference,
            start: 0,
            end: 33
        })
    );
    let track_ref = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    assert_eq!(
        harness.state().shared.borrow().highlight.presented_match,
        Some(MatchHighlight::new(track_ref, &(0..33)))
    );
    harness.get_by_label("test.gtd / Track 1 / Points 0 to 32");
    assert_eq!(
        session::visible_action_notices(&harness),
        ["MCP framed a range on the map"]
    );
    let framed = harness.state().map.viewport_geo_bounds().unwrap();
    assert!(framed.lat_max - framed.lat_min < initial.lat_max - initial.lat_min);
    let shared = harness.state().shared.borrow();
    let track = track_ref.resolve(shared.loaded_files.files()).unwrap();
    let (latitude, longitude) = track.resolved_position_at(31).unwrap();
    let excluded_latitude = track.resolved_position_at(0).unwrap().0.as_degrees();
    assert!((framed.lat_min..=framed.lat_max).contains(&latitude.as_degrees()));
    assert!((framed.lon_min..=framed.lon_max).contains(&longitude.as_degrees()));
    assert!(excluded_latitude < framed.lat_min);
    drop(shared);

    let dismiss = harness
        .get_by_role_and_label(Role::Button, "Dismiss map presentation")
        .rect();
    harness.click_at(dismiss.center());
    harness.run_steps(2);
    assert_eq!(
        harness.state().shared.borrow().highlight.presented_match,
        None
    );
    assert!(harness.query_by_label("Dismiss map presentation").is_none());
    let after = harness.state().map.viewport_geo_bounds().unwrap();
    assert_eq!(
        (after.lat_min, after.lat_max, after.lon_min, after.lon_max),
        (
            framed.lat_min,
            framed.lat_max,
            framed.lon_min,
            framed.lon_max
        )
    );
}

#[rstest]
#[case::queued_stale(PresentationStage::Queued, MatchInvalidation::StaleInputs)]
#[case::active_stale(PresentationStage::Active, MatchInvalidation::StaleInputs)]
#[case::queued_replaced(PresentationStage::Queued, MatchInvalidation::ReplaceJob)]
#[case::active_replaced(PresentationStage::Active, MatchInvalidation::ReplaceJob)]
fn session_map_discards_a_query_match_without_a_status_request(
    #[case] stage: PresentationStage,
    #[case] invalidation: MatchInvalidation,
) {
    let mut harness = ui_tests::harness_with_three_files_loaded();
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .filter
        .set_time_bounds(Some(ui_tests::base_time() + TimeDelta::seconds(20)), None);
    let job = session::start_query(
        &mut harness,
        "points | where velocity > 300 km/h\n\npoints | where velocity > 1 km/h",
    );
    let position = QueryMatchPosition {
        track_ordinal: 1,
        match_ordinal: 0,
    };
    let request = Request::ShowOnMap(ShowOnMap {
        target: RangeTarget::QueryMatch {
            job,
            query: 1,
            matched: position,
        },
    });
    assert_eq!(
        session::execute(&mut harness, request.clone()),
        ErrorCode::QueryNotComplete.into()
    );
    assert!(matches!(
        session::wait_for_query_to_stop(&mut harness, job),
        QueryJobState::Completed { .. }
    ));
    let (matches, _) = session::match_page(
        session::results(
            &mut harness,
            GetQueryResults {
                query: 1,
                page: QueryPageRequest::Matches {
                    start: Some(position),
                },
                ..session::first_query_match_request(job)
            },
        )
        .page,
    );
    let matched = &matches[0];
    assert_eq!(matched.position, position);
    let track = TrackRef::new(FileIdx::new(1), TrackIdx::new(0));
    assert_eq!(matched.range.start, 20);
    let expected = MatchHighlight::new(track, &matched.range);
    let map_visible = matches!(stage, PresentationStage::Active);
    let app = harness.state_mut();
    app.tiles_tree.set_visible(app.map_tile_id, map_visible);

    let mut reply = session::begin(&mut harness, request.clone());
    harness.run_steps(2);
    assert_eq!(
        reply.try_recv().unwrap(),
        map_visible.then_some(Reply::MapRangeFramed {
            track: matched.track,
            start: matched.range.start,
            end: matched.range.end,
        })
    );
    {
        let shared = harness.state().shared.borrow();
        assert_eq!(
            shared.highlight.presented_match,
            map_visible.then_some(expected)
        );
        assert_eq!(shared.pending_session_map.is_some(), !map_visible);
    }
    assert_eq!(
        harness
            .query_by_label_contains("Track 1 / Query 2 match 1")
            .is_some(),
        map_visible
    );

    let expected_error = match invalidation {
        MatchInvalidation::ReplaceJob => {
            assert_ne!(session::start_query(&mut harness, "points"), job);
            ErrorCode::QueryJobNotFound
        }
        MatchInvalidation::StaleInputs => {
            harness
                .state_mut()
                .shared
                .borrow_mut()
                .tree
                .toggle_file_check(FileIdx::new(0));
            ErrorCode::StaleQueryResults
        }
    };
    harness.step();
    if !map_visible {
        assert_eq!(reply.try_recv().unwrap(), Some(expected_error.into()));
    }
    {
        let shared = harness.state().shared.borrow();
        assert_eq!(shared.highlight.presented_match, None);
        assert!(shared.pending_session_map.is_none());
    }
    assert!(harness.query_by_label("Dismiss map presentation").is_none());
    assert_eq!(
        session::execute(&mut harness, request),
        expected_error.into()
    );
}

enum PresentationStage {
    Active,
    Queued,
}

enum MatchInvalidation {
    ReplaceJob,
    StaleInputs,
}

#[test]
fn session_map_validates_query_match_indices_and_geometry() {
    let mut harness = ui_tests::app_with_a_recording();
    let job = session::start_query(&mut harness, "points | where velocity > 1 km/h");
    assert_eq!(
        session::wait_for_query_to_stop(&mut harness, job),
        session::completed_single_track_query(1)
    );
    let position = QueryMatchPosition::default();
    let other_session = QueryJobId {
        session: Uuid::from_u128(2),
        ..job
    };
    let track = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .tree
        .set_category_visible(track, DataCategory::Track, false);

    for (job, query, matched, error) in [
        (other_session, 0, position, ErrorCode::StaleSession),
        (job, 1, position, ErrorCode::InvalidQueryCursor),
        (
            job,
            0,
            QueryMatchPosition {
                track_ordinal: 1,
                ..position
            },
            ErrorCode::InvalidQueryCursor,
        ),
        (
            job,
            0,
            QueryMatchPosition {
                match_ordinal: 1,
                ..position
            },
            ErrorCode::InvalidQueryCursor,
        ),
        (job, 0, position, ErrorCode::NoVisibleMapGeometry),
    ] {
        let request = Request::ShowOnMap(ShowOnMap {
            target: RangeTarget::QueryMatch {
                job,
                query,
                matched,
            },
        });
        assert_eq!(session::execute(&mut harness, request), error.into());
    }
    assert!(
        harness
            .state()
            .shared
            .borrow()
            .pending_session_map
            .is_none()
    );
}

#[rstest]
#[case::within_projection(recording_fixtures::minimal_gtd_spec().start_lat_deg)]
#[case::north_of_projection(89.0)]
#[case::south_of_projection(-89.0)]
fn session_map_opens_and_keeps_the_same_pin_until_the_filter_changes(#[case] start_lat_deg: f64) {
    let mut harness =
        ui_tests::app_with_recording_bytes(gt_test_utils::synthetic_gtd_bytes(SyntheticGtdSpec {
            start_lat_deg,
            ..recording_fixtures::minimal_gtd_spec()
        }));
    let track = session::first_track_reference(&mut harness);
    let point = 20;
    let request = Request::OpenPoint(OpenPoint { track, point });
    let track_ref = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    let fix = MapElementRef::Fix(FixRef::new(track_ref, PointIdx::new(point)));
    let (latitude, longitude) = {
        let shared = harness.state().shared.borrow();
        track_ref
            .resolve(shared.loaded_files.files())
            .unwrap()
            .resolved_position_at(point)
            .unwrap()
    };
    for _ in 0..2 {
        assert_eq!(
            session::execute(&mut harness, request.clone()),
            Reply::MapPointOpened(OpenPoint { track, point })
        );
        harness.step();
        assert_eq!(
            harness.state().shared.borrow().highlight.sticky(),
            Some(fix)
        );
        let viewport = harness.state().map.viewport_geo_bounds().unwrap();
        let rendered_latitude = latitude.clamped_to_the_mercator_limit().as_degrees();
        assert!((viewport.lat_min..=viewport.lat_max).contains(&rendered_latitude));
        assert!((viewport.lon_min..=viewport.lon_max).contains(&longitude.as_degrees()));
    }
    self::exclude_recording_times(&mut harness);
    harness.step();
    assert_eq!(harness.state().shared.borrow().highlight.sticky(), None);
    assert_eq!(
        session::execute(&mut harness, request),
        ErrorCode::NoVisibleMapGeometry.into()
    );
}

#[rstest]
#[case::range_after_rollback(false)]
#[case::filter_before_rollback(true)]
fn map_double_click_preserves_current_session_pin_ownership(#[case] change_filter: bool) {
    let mut harness = ui_tests::app_with_a_recording();
    harness.set_size(egui::vec2(1400.0, 900.0));
    let track = session::first_track_reference(&mut harness);
    let point = 20;
    assert_eq!(
        session::execute(&mut harness, Request::OpenPoint(OpenPoint { track, point })),
        Reply::MapPointOpened(OpenPoint { track, point }),
    );
    harness.run_steps(2);
    let app = harness.state();
    let pinned = app.shared.borrow().highlight.sticky().unwrap();
    let map_rect = app.tiles_tree.tiles.rect(app.map_tile_id).unwrap();
    let target = map_rect.lerp_inside(egui::vec2(0.45, 0.10));
    let release = |harness: &mut Harness<App>| {
        let now = harness.ctx.input(|input| input.time);
        let input = harness.input_mut();
        input.time = Some(now + 0.05);
        for pressed in [true, false] {
            input.events.push(egui::Event::PointerButton {
                pos: target,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        harness.step();
        harness.input_mut().time = None;
    };
    harness.hover_at(target);
    harness.step();
    let initial = harness.state().map.viewport_geo_bounds().unwrap();
    release(&mut harness);
    assert_eq!(harness.state().shared.borrow().highlight.sticky(), None);

    if change_filter {
        harness
            .state_mut()
            .shared
            .borrow_mut()
            .filter
            .set_time_bounds(Some(ui_tests::base_time()), None);
    }
    release(&mut harness);
    let displayed = harness.state().map.viewport_geo_bounds().unwrap();
    assert_eq!(
        (
            displayed.lat_min,
            displayed.lat_max,
            displayed.lon_min,
            displayed.lon_max
        ),
        (
            initial.lat_min,
            initial.lat_max,
            initial.lon_min,
            initial.lon_max
        ),
    );
    harness.step();
    let zoomed = harness.state().map.viewport_geo_bounds().unwrap();
    assert!(zoomed.lat_max - zoomed.lat_min < initial.lat_max - initial.lat_min);
    assert_eq!(
        harness.state().shared.borrow().highlight.sticky(),
        (!change_filter).then_some(pinned),
    );
    if !change_filter {
        assert_eq!(
            session::execute(
                &mut harness,
                Request::ShowOnMap(ShowOnMap {
                    target: RangeTarget::Track {
                        track,
                        start: Some(30),
                        end: Some(33),
                    },
                }),
            ),
            Reply::MapRangeFramed {
                track,
                start: 30,
                end: 33
            },
        );
        harness.step();
        let shared = harness.state().shared.borrow();
        assert!(shared.highlight.presented_match.is_some());
        assert_eq!(shared.highlight.sticky(), None);
    }
}

#[rstest]
#[case::session_last(CameraOrder::SessionLast)]
#[case::session_last_with_zero_height(CameraOrder::SessionLastWithZeroHeight)]
#[case::gui_last(CameraOrder::GuiLast)]
#[case::gui_last_with_zero_height(CameraOrder::GuiLastWithZeroHeight)]
fn the_latest_camera_request_applies_when_the_map_draws(#[case] order: CameraOrder) {
    let mut harness = ui_tests::app_with_a_recording();
    let track = session::first_track_reference(&mut harness);
    let point = 60;
    let track_ref = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    let (gui_latitude, session_latitude) = {
        let shared = harness.state().shared.borrow();
        let track = track_ref.resolve(shared.loaded_files.files()).unwrap();
        (
            track.resolved_position_at(0).unwrap().0,
            track.resolved_position_at(point).unwrap().0,
        )
    };

    let queue_gui_reveal = |harness: &mut Harness<App>| {
        let mut shared = harness.state_mut().shared.borrow_mut();
        shared.zoom_to_visible_request = true;
        shared.reveal_query_matches_request = Some(MatchRevealTarget::OneMatch {
            track: track_ref,
            points: 0..1,
        });
    };
    queue_gui_reveal(&mut harness);
    harness.step();
    let earlier = harness.state().map.viewport_geo_bounds().unwrap();
    assert!(session_latitude.as_degrees() > earlier.lat_max);

    let session_last = matches!(
        order,
        CameraOrder::SessionLast | CameraOrder::SessionLastWithZeroHeight
    );
    if session_last {
        queue_gui_reveal(&mut harness);
    }
    let target = OpenPoint { track, point };
    let mut reply = session::begin(&mut harness, Request::OpenPoint(target));
    if !session_last {
        queue_gui_reveal(&mut harness);
    }
    if matches!(
        order,
        CameraOrder::SessionLastWithZeroHeight | CameraOrder::GuiLastWithZeroHeight
    ) {
        let app = harness.state_mut();
        app.tiles_tree.set_visible(app.plot_tile_id, true);
        app.set_split_ratio(0.0);
        harness.step();
        let app = harness.state_mut();
        let map_rect = app.tiles_tree.tiles.rect(app.map_tile_id).unwrap();
        assert_eq!(map_rect.height(), 0.0);
        {
            let shared = app.shared.borrow();
            assert_eq!(shared.reveal_query_matches_request.is_some(), !session_last);
            assert_eq!(shared.highlight.sticky(), None);
        }
        app.set_split_ratio(0.6);
    }
    harness.step();
    let framed = harness.state().map.viewport_geo_bounds().unwrap();
    let mercator_y =
        |latitude| mercator::normalize_past_the_projection_limit(latitude, Longitude::new(0.0)).y;
    let center_y = mercator_y(Latitude::new(framed.lat_min))
        .midpoint(mercator_y(Latitude::new(framed.lat_max)));
    assert_eq!(
        (center_y - mercator_y(session_latitude)).abs()
            < (center_y - mercator_y(gui_latitude)).abs(),
        session_last,
    );
    let pinned = MapElementRef::Fix(FixRef::new(track_ref, PointIdx::new(point)));
    let expected = if session_last {
        Reply::MapPointOpened(target)
    } else {
        ErrorCode::MapPresentationSuperseded.into()
    };
    assert_eq!(reply.try_recv().unwrap(), Some(expected));
    assert_eq!(
        harness.state().shared.borrow().highlight.sticky(),
        session_last.then_some(pinned),
    );
}

enum CameraOrder {
    SessionLast,
    SessionLastWithZeroHeight,
    GuiLast,
    GuiLastWithZeroHeight,
}

#[test]
fn session_map_preserves_a_gui_pin_when_opening_the_same_point() {
    let mut harness = ui_tests::app_with_query_window_open();
    ui_tests::run_query(&mut harness, "points | where velocity > 1 km/h");
    let row = harness::topmost_point_row(&harness).0.center();
    harness.press_drag_release(row, egui::Vec2::ZERO, 1);
    harness.run_steps(2);
    let Some(MapElementRef::Fix(fix)) = harness.state().shared.borrow().highlight.sticky() else {
        panic!("the result row pins a fix");
    };
    let track = session::first_track_reference(&mut harness);
    let requests = [
        Request::OpenPoint(OpenPoint {
            track,
            point: fix.point.as_usize(),
        }),
        Request::ShowOnMap(ShowOnMap {
            target: RangeTarget::Track {
                track,
                start: Some(30),
                end: Some(33),
            },
        }),
    ];
    for request in requests {
        let expected = match request {
            Request::OpenPoint(point) => Reply::MapPointOpened(point),
            _ => Reply::MapRangeFramed {
                track,
                start: 30,
                end: 33,
            },
        };
        let is_point = matches!(expected, Reply::MapPointOpened(_));
        assert_eq!(session::execute(&mut harness, request), expected);
        harness.run_steps(2);
        assert_eq!(
            harness.state().shared.borrow().highlight.sticky(),
            Some(MapElementRef::Fix(fix)),
        );
        if is_point {
            harness
                .get_by_role_and_label(Role::Button, "Dismiss map presentation")
                .click();
            harness.run_steps(2);
            assert!(harness.query_by_label("Dismiss map presentation").is_none());
            assert_eq!(
                harness.state().shared.borrow().highlight.sticky(),
                Some(MapElementRef::Fix(fix))
            );
        }
    }
    assert_eq!(
        harness.state().shared.borrow().highlight.presented_match,
        Some(MatchHighlight::new(fix.track, &(30..33))),
    );

    let app = harness.state_mut();
    app.tiles_tree.set_visible(app.map_tile_id, false);
    harness.step();
    let mut reply = session::begin(
        &mut harness,
        Request::OpenPoint(OpenPoint {
            track,
            point: fix.point.as_usize(),
        }),
    );
    let row = harness::topmost_point_row(&harness).0.center();
    harness.press_drag_release(row, egui::Vec2::ZERO, 1);
    assert_eq!(
        reply.try_recv().unwrap(),
        Some(ErrorCode::MapPresentationSuperseded.into())
    );
    assert_eq!(harness.state().shared.borrow().highlight.sticky(), None);

    let app = harness.state_mut();
    app.tiles_tree.set_visible(app.map_tile_id, true);
    harness.step();
    assert_eq!(harness.state().shared.borrow().highlight.sticky(), None);
}

#[test]
fn dismissing_a_presented_point_clears_its_pin() {
    let mut harness = ui_tests::app_with_a_recording();
    let track = session::first_track_reference(&mut harness);
    let point = 20;
    assert_eq!(
        session::execute(&mut harness, Request::OpenPoint(OpenPoint { track, point })),
        Reply::MapPointOpened(OpenPoint { track, point })
    );
    harness.run_steps(2);
    harness.get_by_label("test.gtd / Track 1 / Point 20");
    assert_eq!(
        session::visible_action_notices(&harness),
        ["MCP centered the map on a point"]
    );
    assert!(harness.state().shared.borrow().highlight.sticky().is_some());
    harness
        .get_by_role_and_label(Role::Button, "Dismiss map presentation")
        .click();
    harness.run_steps(2);
    assert!(harness.query_by_label("Dismiss map presentation").is_none());
    assert_eq!(harness.state().shared.borrow().highlight.sticky(), None);
}

#[test]
fn the_map_dismiss_button_is_disabled_during_shutdown() {
    let mut harness = ui_tests::app_with_a_recording();
    let track = session::first_track_reference(&mut harness);
    assert_eq!(
        session::execute(
            &mut harness,
            Request::OpenPoint(OpenPoint { track, point: 20 })
        ),
        Reply::MapPointOpened(OpenPoint { track, point: 20 })
    );
    harness.run_steps(2);
    let _write = harness
        .state()
        .pending_writes
        .try_begin("Saving recording", WriteKind::RecordingDatabase)
        .unwrap();
    ui_tests::request_window_close(&mut harness);
    harness.step();
    assert!(
        harness
            .get_by_role_and_label(Role::Button, "Dismiss map presentation")
            .accesskit_node()
            .is_disabled()
    );
}

#[test]
fn superseded_point_request_preserves_deferred_gui_popup_placement() {
    let mut harness = ui_tests::app_with_query_window_open();
    ui_tests::run_query(&mut harness, "points | where velocity > 1 km/h");
    let app = harness.state_mut();
    app.tiles_tree.set_visible(app.map_tile_id, false);
    harness.step();
    let row = harness::topmost_point_row(&harness).0.center();
    harness.press_drag_release(row, egui::Vec2::ZERO, 1);
    let (gui_pin, gui_placement) = {
        let shared = harness.state().shared.borrow();
        (
            shared.highlight.sticky().unwrap(),
            shared.popup_pos_request.unwrap(),
        )
    };
    let track = session::first_track_reference(&mut harness);
    let point = 60;
    assert_ne!(
        gui_pin,
        MapElementRef::Fix(FixRef::new(
            TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
            PointIdx::new(point),
        )),
    );
    let mut reply = session::begin(&mut harness, Request::OpenPoint(OpenPoint { track, point }));
    let app = harness.state_mut();
    {
        let mut shared = app.shared.borrow_mut();
        assert_eq!(shared.popup_pos_request, Some(gui_placement));
        shared.zoom_to_visible_request = true;
    }
    app.tiles_tree.set_visible(app.map_tile_id, true);
    harness.step();
    assert_eq!(
        reply.try_recv().unwrap(),
        Some(ErrorCode::MapPresentationSuperseded.into())
    );
    let shared = harness.state().shared.borrow();
    assert_eq!(shared.highlight.sticky(), Some(gui_pin));
    assert_eq!(shared.popup_pos_request, None);
}

#[test]
fn session_map_range_keeps_the_query_highlights_solo_restore_available() {
    let mut harness = ui_tests::app_with_a_recording();
    let track = session::first_track_reference(&mut harness);
    let custom_mask = {
        let mut shared = harness.state_mut().shared.borrow_mut();
        shared
            .display_mask
            .set_visible(DisplayCategory::TrackPoints, false);
        shared.display_mask
    };
    assert!(matches!(
        session::execute(
            &mut harness,
            Request::ShowOnMap(ShowOnMap {
                target: RangeTarget::Track {
                    track,
                    start: None,
                    end: None
                },
            }),
        ),
        Reply::MapRangeFramed { .. }
    ));
    harness.run_steps(2);
    assert!(harness.state().editor_query.session().matches().is_none());
    let toggle = harness
        .ctx
        .memory(|memory| memory.area_rect(egui::Id::unique(gt_map::DISPLAY_TOGGLE_BUTTON_AREA_ID)))
        .unwrap();
    harness.click_at(toggle.center());
    harness.run_steps(2);
    let row = harness.get_by_label_contains("Query highlights").rect();
    harness.event(egui::Event::ModifiersChanged(egui::Modifiers::ALT));
    harness.step();
    harness.click_at(row.center());
    harness.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
    harness.run_steps(2);
    let mut solo = custom_mask;
    solo.solo(DisplayCategory::QueryHighlights);
    assert_eq!(harness.state().shared.borrow().display_mask, solo);
    let restore = harness.get_by_role_and_label(Role::Button, "only").rect();
    harness.click_at(restore.center());
    harness.run_steps(2);
    let shared = harness.state().shared.borrow();
    assert_eq!(shared.display_mask, custom_mask);
    assert!(shared.highlight.presented_match.is_some());
}

#[test]
fn session_map_drops_a_queued_range_when_its_inputs_change() {
    let mut harness = ui_tests::app_with_a_recording();
    let track = session::first_track_reference(&mut harness);
    let request = Request::ShowOnMap(ShowOnMap {
        target: RangeTarget::Track {
            track,
            start: None,
            end: None,
        },
    });
    let mut reply = session::begin(&mut harness, request.clone());
    self::exclude_recording_times(&mut harness);
    harness.step();
    assert_eq!(
        reply.try_recv().unwrap(),
        Some(ErrorCode::MapPresentationStale.into())
    );
    assert_eq!(
        harness.state().shared.borrow().highlight.presented_match,
        None
    );
    assert_eq!(
        session::execute(&mut harness, request),
        ErrorCode::NoVisibleMapGeometry.into()
    );
}

#[rstest]
#[case::track(false)]
#[case::query_match(true)]
fn session_map_rejects_ranges_with_highlights_disabled_and_keeps_the_point_request(
    #[case] retained_match: bool,
) {
    let mut harness = ui_tests::app_with_a_recording();
    let track = session::first_track_reference(&mut harness);
    let target = if retained_match {
        let job = session::start_query(&mut harness, "points | where velocity > 1 km/h");
        assert!(matches!(
            session::wait_for_query_to_stop(&mut harness, job),
            QueryJobState::Completed { .. }
        ));
        RangeTarget::QueryMatch {
            job,
            query: 0,
            matched: QueryMatchPosition::default(),
        }
    } else {
        RangeTarget::Track {
            track,
            start: Some(0),
            end: Some(3),
        }
    };
    let request = Request::ShowOnMap(ShowOnMap { target });
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .display_mask
        .set_visible(DisplayCategory::QueryHighlights, false);
    let point = 60;
    let target = OpenPoint { track, point };
    let mut reply = session::begin(&mut harness, Request::OpenPoint(target));
    assert_eq!(
        session::execute(&mut harness, request.clone()),
        ErrorCode::QueryHighlightsDisabled.into()
    );
    assert_eq!(reply.try_recv().unwrap(), None);
    harness.step();
    assert_eq!(
        reply.try_recv().unwrap(),
        Some(Reply::MapPointOpened(target))
    );
    let pinned = MapElementRef::Fix(FixRef::new(
        TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
        PointIdx::new(point),
    ));
    let mut shared = harness.state_mut().shared.borrow_mut();
    assert_eq!(shared.highlight.sticky(), Some(pinned));
    assert_eq!(shared.highlight.presented_match, None);
    assert!(
        !shared
            .display_mask
            .is_visible(DisplayCategory::QueryHighlights)
    );
    shared
        .display_mask
        .set_visible(DisplayCategory::QueryHighlights, true);
    drop(shared);

    assert!(matches!(
        session::execute(&mut harness, request),
        Reply::MapRangeFramed { .. }
    ));
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
}

#[test]
fn session_map_drops_a_queued_range_when_highlights_are_disabled_before_drawing() {
    let mut harness = ui_tests::app_with_a_recording();
    let track = session::first_track_reference(&mut harness);
    let request = Request::ShowOnMap(ShowOnMap {
        target: RangeTarget::Track {
            track,
            start: Some(0),
            end: Some(3),
        },
    });
    let before = harness.state().map.viewport_geo_bounds().unwrap();
    let mut reply = session::begin(&mut harness, request);
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .display_mask
        .set_visible(DisplayCategory::QueryHighlights, false);
    harness.step();
    assert_eq!(
        reply.try_recv().unwrap(),
        Some(ErrorCode::QueryHighlightsDisabled.into())
    );
    let after = harness.state().map.viewport_geo_bounds().unwrap();
    assert_eq!(
        (after.lat_min, after.lat_max, after.lon_min, after.lon_max),
        (
            before.lat_min,
            before.lat_max,
            before.lon_min,
            before.lon_max
        )
    );
    let mut shared = harness.state_mut().shared.borrow_mut();
    assert_eq!(shared.highlight.presented_match, None);
    assert!(shared.pending_session_map.is_none());
    shared
        .display_mask
        .set_visible(DisplayCategory::QueryHighlights, true);
    drop(shared);
    harness.step();
    assert_eq!(
        harness.state().shared.borrow().highlight.presented_match,
        None
    );
    assert!(session::visible_action_notices(&harness).is_empty());
}

#[test]
fn a_new_map_request_supersedes_the_waiter_and_state_stays_readable() {
    let mut harness = ui_tests::app_with_a_recording();
    let track = session::first_track_reference(&mut harness);
    let earlier = Request::ShowOnMap(ShowOnMap {
        target: RangeTarget::Track {
            track,
            start: Some(30),
            end: Some(33),
        },
    });
    let mut old_reply = session::begin(&mut harness, earlier);
    let Reply::SessionState { state } = session::execute(&mut harness, Request::GetSessionState)
    else {
        panic!("expected session state");
    };
    assert_eq!(state.map.pinned_point, None);
    assert_eq!(old_reply.try_recv().unwrap(), None);

    let point = OpenPoint { track, point: 20 };
    let mut new_reply = session::begin(&mut harness, Request::OpenPoint(point));
    assert_eq!(
        old_reply.try_recv().unwrap(),
        Some(ErrorCode::MapPresentationSuperseded.into())
    );
    assert_eq!(new_reply.try_recv().unwrap(), None);
    harness.run_steps(2);
    assert_eq!(
        new_reply.try_recv().unwrap(),
        Some(Reply::MapPointOpened(point))
    );
    let shared = harness.state().shared.borrow();
    assert_eq!(shared.highlight.presented_match, None);
    assert_eq!(
        session::visible_action_notices(&harness),
        ["MCP centered the map on a point"]
    );
}

#[rstest]
#[case::invalid_point(usize::MAX, false, ErrorCode::InvalidPoint)]
#[case::stale_track(0, true, ErrorCode::StaleTrack)]
fn session_map_rejects_invalid_point_references(
    #[case] point: usize,
    #[case] stale: bool,
    #[case] expected: ErrorCode,
) {
    let mut harness = ui_tests::app_with_a_recording();
    let mut track = session::first_track_reference(&mut harness);
    if stale {
        track.revision += 1;
    }
    assert_eq!(
        session::execute(&mut harness, Request::OpenPoint(OpenPoint { track, point })),
        expected.into()
    );
}

enum CompetingMapInput {
    DragOutsideMap,
    Escape,
    Zoom,
}

#[rstest]
#[case::escape(CompetingMapInput::Escape)]
#[case::zoom(CompetingMapInput::Zoom)]
#[case::drag_outside_map(CompetingMapInput::DragOutsideMap)]
fn later_map_input_preserves_the_range_and_supersedes_a_point(#[case] input: CompetingMapInput) {
    let mut harness = ui_tests::app_with_a_recording();
    let track = session::first_track_reference(&mut harness);
    let range = Request::ShowOnMap(ShowOnMap {
        target: RangeTarget::Track {
            track,
            start: Some(30),
            end: Some(33),
        },
    });
    assert_eq!(
        session::execute(&mut harness, range),
        Reply::MapRangeFramed {
            track,
            start: 30,
            end: 33
        }
    );
    harness.step();
    let before = harness.state().map.viewport_geo_bounds().unwrap();
    let highlight = harness.state().shared.borrow().highlight.presented_match;
    let app = harness.state();
    let map_rect = app.tiles_tree.tiles.rect(app.map_tile_id).unwrap();
    let map_center = map_rect.center();
    harness.hover_at(map_center);
    harness.step();
    if matches!(input, CompetingMapInput::DragOutsideMap) {
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: map_center,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        harness.step();
    }
    let mut reply = session::begin(
        &mut harness,
        Request::OpenPoint(OpenPoint { track, point: 1 }),
    );
    if matches!(input, CompetingMapInput::DragOutsideMap) {
        harness.step();
        assert!(
            harness
                .state()
                .shared
                .borrow()
                .pending_session_map
                .is_some()
        );
    }
    let event = match input {
        CompetingMapInput::DragOutsideMap => {
            egui::Event::PointerMoved(egui::pos2(map_rect.left() - 20.0, map_center.y))
        }
        CompetingMapInput::Escape => egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        },
        CompetingMapInput::Zoom => egui::Event::MouseWheel {
            unit: egui::MouseWheelUnit::Point,
            source: egui::MouseWheelSource::Wheel,
            delta: egui::vec2(0.0, -90.0),
            phase: egui::TouchPhase::Move,
            modifiers: egui::Modifiers::NONE,
        },
    };
    harness.input_mut().events.push(event);
    harness.step();
    assert_eq!(
        reply.try_recv().unwrap(),
        Some(ErrorCode::MapPresentationSuperseded.into())
    );
    let after = harness.state().map.viewport_geo_bounds().unwrap();
    assert_eq!(
        (after.lat_min, after.lon_min, after.lat_max, after.lon_max)
            == (
                before.lat_min,
                before.lon_min,
                before.lat_max,
                before.lon_max
            ),
        matches!(input, CompetingMapInput::Escape)
    );
    let shared = harness.state().shared.borrow();
    assert_eq!(shared.highlight.presented_match, highlight);
    assert_eq!(shared.highlight.sticky(), None);
    assert_eq!(
        session::visible_action_notices(&harness),
        ["MCP framed a range on the map"]
    );
}

#[rstest]
#[case::one_request(false)]
#[case::replacement(true)]
fn an_unavailable_point_keeps_the_earlier_desktop_reveal(#[case] replace: bool) {
    let mut harness = ui_tests::app_with_a_recording();
    let track = session::first_track_reference(&mut harness);
    let range = Request::ShowOnMap(ShowOnMap {
        target: RangeTarget::Track {
            track,
            start: Some(30),
            end: Some(33),
        },
    });
    assert_eq!(
        session::execute(&mut harness, range),
        Reply::MapRangeFramed {
            track,
            start: 30,
            end: 33
        }
    );
    harness.step();
    let active = harness.state().shared.borrow().highlight.presented_match;
    let reference = TrackRef::new(FileIdx::new(0), TrackIdx::new(0));
    let latitude = {
        let mut shared = harness.state_mut().shared.borrow_mut();
        shared.reveal_query_matches_request = Some(MatchRevealTarget::OneMatch {
            track: reference,
            points: 0..1,
        });
        reference
            .resolve(shared.loaded_files.files())
            .unwrap()
            .resolved_position_at(0)
            .unwrap()
            .0
            .as_degrees()
    };
    assert!(latitude < harness.state().map.viewport_geo_bounds().unwrap().lat_min);
    let point = |point| Request::OpenPoint(OpenPoint { track, point });
    let first_reply = session::begin(&mut harness, point(1));
    let mut reply = if replace {
        let replacement = session::begin(&mut harness, point(2));
        let mut first_reply = first_reply;
        assert_eq!(
            first_reply.try_recv().unwrap(),
            Some(ErrorCode::MapPresentationSuperseded.into())
        );
        replacement
    } else {
        first_reply
    };
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .display_mask
        .set_visible(DisplayCategory::TrackPoints, false);
    harness.step();
    assert_eq!(
        reply.try_recv().unwrap(),
        Some(ErrorCode::NoVisibleMapGeometry.into())
    );
    let restored = harness.state().map.viewport_geo_bounds().unwrap();
    assert!((restored.lat_min..=restored.lat_max).contains(&latitude));
    let shared = harness.state().shared.borrow();
    assert!(shared.pending_session_map.is_none());
    assert_eq!(shared.highlight.sticky(), None);
    assert_eq!(shared.highlight.presented_match, active);
}

#[test]
fn dragging_in_the_query_editor_keeps_map_presentations_available() {
    let mut harness = ui_tests::app_with_query_window_open();
    harness
        .state_mut()
        .editor_query
        .session_mut()
        .set_text("points | where velocity > 1 km/h".to_owned());
    harness.run_steps(2);
    let track = session::first_track_reference(&mut harness);
    let app = harness.state();
    let viewport = app.tiles_tree.tiles.rect(app.map_tile_id).unwrap();
    let editor = harness
        .get_by_role(Role::MultilineTextInput)
        .rect()
        .intersect(viewport);
    assert!(editor.is_positive());
    let start = editor.lerp_inside(egui::vec2(0.2, 0.5));
    let end = editor.lerp_inside(egui::vec2(0.8, 0.5));
    harness.hover_at(start);
    harness.step();
    harness.input_mut().events.push(egui::Event::PointerButton {
        pos: start,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::NONE,
    });
    harness.step();

    let mut reply = session::begin(
        &mut harness,
        Request::ShowOnMap(ShowOnMap {
            target: RangeTarget::Track {
                track,
                start: Some(30),
                end: Some(33),
            },
        }),
    );
    harness
        .input_mut()
        .events
        .push(egui::Event::PointerMoved(end));
    harness.step();
    assert_eq!(
        reply.try_recv().unwrap(),
        Some(Reply::MapRangeFramed {
            track,
            start: 30,
            end: 33
        })
    );
    assert!(
        harness
            .ctx
            .input(|input| input.pointer.any_down() && input.pointer.delta() != egui::Vec2::ZERO)
    );
    let shared = harness.state().shared.borrow();
    assert_eq!(
        shared.highlight.presented_match,
        Some(MatchHighlight::new(
            TrackRef::new(FileIdx::new(0), TrackIdx::new(0)),
            &(30..33),
        ))
    );
}

#[test]
fn a_repeated_desktop_fit_takes_precedence_over_a_queued_point() {
    let mut harness = ui_tests::app_with_a_recording();
    let track = session::first_track_reference(&mut harness);
    harness
        .state_mut()
        .shared
        .borrow_mut()
        .zoom_to_visible_request = true;
    harness.step();
    let app = harness.state();
    let center = app.tiles_tree.tiles.rect(app.map_tile_id).unwrap().center();
    harness.hover_at(center);
    harness.step();
    let before = harness.state().map.viewport_geo_bounds().unwrap();
    let mut reply = session::begin(
        &mut harness,
        Request::OpenPoint(OpenPoint { track, point: 1 }),
    );
    for pressed in [true, false] {
        harness.input_mut().events.push(egui::Event::PointerButton {
            pos: center,
            button: egui::PointerButton::Middle,
            pressed,
            modifiers: egui::Modifiers::NONE,
        });
    }
    harness.run_steps(2);
    assert_eq!(
        reply.try_recv().unwrap(),
        Some(ErrorCode::MapPresentationSuperseded.into())
    );
    let after = harness.state().map.viewport_geo_bounds().unwrap();
    assert_eq!(
        (after.lat_min, after.lat_max, after.lon_min, after.lon_max),
        (
            before.lat_min,
            before.lat_max,
            before.lon_min,
            before.lon_max
        )
    );
    let shared = harness.state().shared.borrow();
    assert!(shared.pending_session_map.is_none());
    assert_eq!(shared.highlight.sticky(), None);
    assert!(session::visible_action_notices(&harness).is_empty());
}

fn exclude_recording_times(harness: &mut Harness<'_, App>) {
    let mut shared = harness.state_mut().shared.borrow_mut();
    shared
        .filter
        .set_time_bounds(Some(ui_tests::base_time() + TimeDelta::minutes(2)), None);
}
