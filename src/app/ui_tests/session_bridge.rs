use std::io::ErrorKind;
use std::sync::Arc;

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use gt_session_bridge::test_util::TestEndpoint;
use gt_session_bridge::{
    ErrorCode, GetRecording, ListRecordings, RecordingReference, Reply, Request,
};

use crate::app::test_util::{harness, session};
use crate::app::{App, ui_tests};

#[test]
fn the_mcp_settings_control_starts_stops_and_restarts_the_local_server() {
    let mut harness = ui_tests::app_with_a_recording();
    let endpoint = Arc::new(TestEndpoint::new().unwrap());
    harness.state_mut().session_bridge_endpoint = Some(Arc::clone(&endpoint));
    self::open_mcp_settings(&mut harness);
    harness.get_by_label("MCP is stopped");
    self::assert_mcp_unavailable(&endpoint);

    self::click_mcp_button(&mut harness, "Start MCP");
    harness.get_by_label("MCP is running");
    let capture = harness
        .state()
        .session_bridge
        .server()
        .unwrap()
        .traffic_capture();
    capture.set_enabled(true);
    let original = self::first_recording(&mut harness, &endpoint);
    assert!(!capture.snapshot().is_empty());

    self::click_mcp_button(&mut harness, "Stop MCP");
    harness.get_by_label("MCP is stopped");
    self::assert_mcp_unavailable(&endpoint);
    assert!(capture.snapshot().is_empty());
    assert!(!capture.is_enabled());

    self::click_mcp_button(&mut harness, "Start MCP");
    harness.get_by_label("MCP is running");
    let current = self::first_recording(&mut harness, &endpoint);
    assert_ne!(current.session, original.session);
    assert_eq!(current.number, original.number);
    let fresh_capture = harness
        .state()
        .session_bridge
        .server()
        .unwrap()
        .traffic_capture();
    assert!(fresh_capture.snapshot().is_empty());
    assert!(!fresh_capture.is_enabled());
    assert_eq!(
        session::exchange_request(
            &mut harness,
            &endpoint,
            &Request::GetRecording(GetRecording {
                recording: original,
                after: None,
                stored_track_start: None,
                limit: None,
            }),
        ),
        Reply::from(ErrorCode::StaleSession)
    );
}

#[test]
fn a_failed_mcp_start_preserves_the_other_listener_and_can_be_retried() {
    let (mut owner, endpoint) = session::app_with_bridge();
    let mut contender = Harness::new_eframe(harness::transient_app);
    contender.state_mut().session_bridge_endpoint = Some(Arc::clone(&endpoint));
    self::open_mcp_settings(&mut contender);
    self::click_mcp_button(&mut contender, "Start MCP");
    contender.get_by_label("MCP could not start");
    assert!(contender.state().session_bridge.server().is_none());
    assert!(matches!(
        session::exchange_request(&mut owner, &endpoint, &LIST_RECORDINGS),
        Reply::Recordings { .. }
    ));

    owner.state_mut().begin_shutdown();
    self::assert_mcp_unavailable(&endpoint);
    self::click_mcp_button(&mut contender, "Start MCP");
    contender.get_by_label("MCP is running");
    assert!(matches!(
        session::exchange_request(&mut contender, &endpoint, &LIST_RECORDINGS),
        Reply::Recordings { .. }
    ));
}

fn open_mcp_settings(harness: &mut Harness<'_, App>) {
    harness.state_mut().settings_open = true;
    harness.run_steps(2);
    harness
        .get_by_role_and_label(Role::Window, "Settings")
        .get_by_label_contains("MCP")
        .click();
    harness.run_steps(2);
}

fn click_mcp_button(harness: &mut Harness<'_, App>, label: &str) {
    harness
        .get_by_role_and_label(Role::Window, "Settings")
        .get_by_role_and_label(Role::Button, label)
        .click();
    harness.run_steps(2);
}

fn assert_mcp_unavailable(endpoint: &TestEndpoint) {
    let error = endpoint.exchange_payload(LIST_PAYLOAD).unwrap_err();
    assert!(
        matches!(
            error.kind(),
            ErrorKind::NotFound | ErrorKind::ConnectionRefused
        ),
        "{error}"
    );
}

fn first_recording(harness: &mut Harness<'_, App>, endpoint: &TestEndpoint) -> RecordingReference {
    let Reply::Recordings { page } = session::exchange_request(harness, endpoint, &LIST_RECORDINGS)
    else {
        panic!("expected loaded recordings");
    };
    page.recordings.first().unwrap().id
}

const LIST_RECORDINGS: Request = Request::ListRecordings(ListRecordings {
    after: None,
    limit: None,
});
const LIST_PAYLOAD: &[u8] = br#"{"method":"list_recordings"}"#;
