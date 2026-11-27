use std::time::Instant;

use egui_kittest::Harness;
use gt_session_bridge::{ErrorCode, OpenPoint, RangeTarget, Reply, Request, ShowOnMap};
use gt_test_utils::recording_fixtures;
use gt_types::{FileIdx, TrackIdx, TrackRef};
use gt_ui_types::MatchRevealTarget;
use rstest::rstest;

use crate::app::test_util::harness::TestDroppedFile;
use crate::app::test_util::{harness, session};

enum WaitEnd {
    DeadlineBeforeLogic,
    DeadlineBeforeLayout,
    ClientDisconnected,
}

#[rstest]
#[case::deadline_before_logic(WaitEnd::DeadlineBeforeLogic)]
#[case::deadline_before_layout(WaitEnd::DeadlineBeforeLayout)]
#[case::disconnected_client(WaitEnd::ClientDisconnected)]
fn an_unapplied_map_request_keeps_the_earlier_desktop_reveal(#[case] end: WaitEnd) {
    let mut harness = Harness::new_eframe(harness::transient_app);
    harness::drop_file_and_wait_for_load(
        &mut harness,
        TestDroppedFile::bytes(recording_fixtures::minimal_gtd_bytes(), "test.gtd"),
    );
    let track = session::first_track_reference(&mut harness);
    let active = Request::ShowOnMap(ShowOnMap {
        target: RangeTarget::Track {
            track,
            start: Some(30),
            end: Some(33),
        },
    });
    assert_eq!(
        session::execute(&mut harness, active),
        Reply::MapRangeFramed {
            track,
            start: 30,
            end: 33
        }
    );
    let before = harness.state().shared.borrow().highlight.presented_match;
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
    let framed = harness.state().map.viewport_geo_bounds().unwrap();
    assert!(latitude < framed.lat_min);
    let mut reply = session::begin(
        &mut harness,
        Request::OpenPoint(OpenPoint { track, point: 20 }),
    );
    if matches!(end, WaitEnd::ClientDisconnected) {
        drop(reply);
        let _ = harness::step_minimized_logic(&mut harness);
    } else {
        let app = harness.state_mut();
        app.shared
            .borrow_mut()
            .pending_session_map
            .as_mut()
            .unwrap()
            .expires = Instant::now();
        if matches!(end, WaitEnd::DeadlineBeforeLayout) {
            let draw =
                app.shared
                    .borrow_mut()
                    .apply_session_map(&mut app.map, None, &app.editor_query);
            assert!(draw.is_none());
        } else {
            let _ = harness::step_minimized_logic(&mut harness);
        }
        assert_eq!(
            reply.try_recv().unwrap(),
            Some(ErrorCode::MapPresentationTimedOut.into())
        );
    }
    assert!(
        harness
            .state()
            .shared
            .borrow()
            .pending_session_map
            .is_none()
    );
    harness.step();
    let restored = harness.state().map.viewport_geo_bounds().unwrap();
    assert!((restored.lat_min..=restored.lat_max).contains(&latitude));
    let shared = harness.state().shared.borrow();
    assert_eq!(shared.highlight.presented_match, before);
    assert_eq!(shared.highlight.sticky(), None);
}
