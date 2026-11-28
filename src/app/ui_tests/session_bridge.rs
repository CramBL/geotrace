use std::fs;
use std::io::ErrorKind;
use std::sync::Arc;

use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::Queryable as _;
use gt_instance_lock::DataDirectoryLock;
use gt_map::TileAccess;
use gt_pending_writes::PendingWrites;
use gt_session_bridge::test_util::TestEndpoint;
use gt_session_bridge::{
    ErrorCode, GetRecording, ListRecordings, RecordingReference, Reply, Request,
};
use gt_test_utils::{HarnessInteraction as _, TestHarness};
use rstest::rstest;

use crate::app::test_util::{harness, session};
use crate::app::{App, StartupOptions, Storage, ui_tests};
use crate::settings::Settings;

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

#[test]
fn toggling_start_mcp_on_launch_persists_without_changing_running_state() {
    let (mut harness, config_path) = TestHarness::builder().eframe(harness::build_app);
    let endpoint = Arc::new(TestEndpoint::new().unwrap());
    harness.inner.state_mut().session_bridge_endpoint = Some(Arc::clone(&endpoint));
    self::open_mcp_settings(&mut harness.inner);
    assert!(!harness.inner.state().mcp_start_on_launch);
    assert!(harness.inner.state().session_bridge.server().is_none());

    self::click_start_on_launch_checkbox(&mut harness.inner);
    assert!(harness.inner.state().mcp_start_on_launch);
    assert!(harness.inner.state().session_bridge.server().is_none());
    assert!(
        harness.inner.step_until(|_| {
            fs::read_to_string(&config_path)
                .ok()
                .and_then(|text| toml::from_str::<Settings>(&text).ok())
                .is_some_and(|settings| settings.mcp.start_on_launch)
        }),
        "enabling Start MCP on launch was not persisted"
    );

    self::click_mcp_button(&mut harness.inner, "Start MCP");
    assert!(harness.inner.state().session_bridge.server().is_some());

    self::click_start_on_launch_checkbox(&mut harness.inner);
    assert!(!harness.inner.state().mcp_start_on_launch);
    assert!(harness.inner.state().session_bridge.server().is_some());
    assert!(
        harness.inner.step_until(|_| {
            fs::read_to_string(&config_path)
                .ok()
                .and_then(|text| toml::from_str::<Settings>(&text).ok())
                .is_some_and(|settings| !settings.mcp.start_on_launch)
        }),
        "disabling Start MCP on launch was not persisted"
    );
}

#[derive(Clone, Copy)]
enum StartupTrigger {
    CliFlag,
    Settings,
}

#[rstest]
#[case::from_settings(StartupTrigger::Settings)]
#[case::from_cli_flag(StartupTrigger::CliFlag)]
fn startup_starts_mcp_when_enabled_by_settings_or_flag(#[case] trigger: StartupTrigger) {
    let config_dir = tempfile::tempdir().unwrap();
    let config_path = config_dir.path().join("config.toml");
    let start_on_launch = matches!(trigger, StartupTrigger::Settings);
    let start_session_bridge = matches!(trigger, StartupTrigger::CliFlag);
    fs::write(
        &config_path,
        format!("[mcp]\nstart_on_launch = {start_on_launch}\n"),
    )
    .unwrap();

    let endpoint = Arc::new(TestEndpoint::new().unwrap());
    let endpoint_for_app = Arc::clone(&endpoint);
    let mut harness = Harness::builder()
        .with_context_setup(gt_ui_theme::install_app_style)
        .with_wait_for_pending_images(false)
        .build_eframe(move |cc| {
            App::new_with_config(
                cc,
                &[],
                Some(config_path),
                StartupOptions {
                    fading_enabled: false,
                    offline: true,
                    tile_access: TileAccess::Synthetic,
                    storage: Storage::Disabled,
                    app_version: harness::TEST_APP_VERSION,
                    pending_writes: PendingWrites::default(),
                    instance_lock: DataDirectoryLock::marking_nothing(),
                    start_session_bridge,
                    session_bridge_endpoint: Some(endpoint_for_app),
                },
            )
        });

    assert!(harness.state().session_bridge.server().is_some());
    assert_eq!(harness.state().mcp_start_on_launch, start_on_launch);
    assert!(matches!(
        session::exchange_request(&mut harness, &endpoint, &LIST_RECORDINGS),
        Reply::Recordings { .. }
    ));
    self::open_mcp_settings(&mut harness);
    harness.get_by_label("MCP is running");
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

fn click_start_on_launch_checkbox(harness: &mut Harness<'_, App>) {
    harness
        .get_by_role_and_label(Role::Window, "Settings")
        .get_by_role_and_label(Role::CheckBox, "Start MCP on launch")
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
