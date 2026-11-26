use std::str;

use egui::OutputCommand;
use egui::accesskit::Role;
use egui_kittest::Harness;
use egui_kittest::kittest::{NodeT as _, Queryable as _};
use gt_session_bridge::TrafficCapture;
use gt_test_utils::HarnessInteraction as _;

use crate::app::App;
use crate::app::test_util::{harness, session};

#[test]
fn the_traffic_menu_requires_an_enabled_session_bridge() {
    let mut harness = Harness::new_eframe(harness::transient_app);
    harness.step();
    harness.get_by_label("File").click();
    harness.run_steps(2);
    assert!(
        harness
            .get_by_label(MENU_LABEL)
            .accesskit_node()
            .is_disabled()
    );
}

#[test]
fn the_traffic_window_captures_and_copies_the_request_and_reply() {
    let (mut harness, endpoint) = session::app_with_bridge();
    let capture = self::capture(&harness);
    session::exchange_payload(&mut harness, &endpoint, LIST_REQUEST);
    self::open_traffic(&mut harness);
    harness.get_by_label("No captured traffic");
    assert!(!capture.is_enabled());

    harness.get_by_label("Capture traffic").click();
    harness.step();
    assert!(capture.is_enabled());
    harness
        .get_by_role_and_label(Role::Window, WINDOW_TITLE)
        .get_by_label("Close window")
        .click();
    harness.run_steps(2);
    assert!(
        harness
            .query_by_role_and_label(Role::Window, WINDOW_TITLE)
            .is_none()
    );

    let reply = session::exchange_payload(&mut harness, &endpoint, LIST_REQUEST);
    assert!(harness.step_until(|_| capture.snapshot().len() == 2));
    self::open_traffic(&mut harness);
    self::select_frame(&mut harness, "#1   Request");
    self::assert_payload_copies(&mut harness, str::from_utf8(LIST_REQUEST).unwrap());
    self::select_frame(&mut harness, "#1   Reply");
    self::assert_payload_copies(&mut harness, str::from_utf8(&reply).unwrap());

    harness.get_by_label("Capture traffic").click();
    harness.step();
    assert!(!capture.is_enabled());
    session::exchange_payload(&mut harness, &endpoint, LIST_REQUEST);
    assert_eq!(capture.snapshot().len(), 2);

    harness.get_by_label("Clear").click();
    harness.step();
    assert!(capture.snapshot().is_empty());
    harness.get_by_label("No captured traffic");
    assert!(
        harness
            .get_by_label("Copy payload")
            .accesskit_node()
            .is_disabled()
    );
    assert!(
        harness
            .query_by_label(str::from_utf8(&reply).unwrap())
            .is_none()
    );
}

#[test]
fn the_traffic_window_copies_hex_and_drops_an_evicted_selection() {
    let (mut harness, endpoint) = session::app_with_bridge();
    let capture = self::capture(&harness);
    capture.set_enabled(true);
    self::open_traffic(&mut harness);
    endpoint.exchange_payload(&[0x00, b'\n', 0xff]).unwrap();
    assert!(harness.step_until(|_| capture.snapshot().len() == 2));
    harness.step();
    self::select_frame(&mut harness, "#1   Request");
    harness.get_by_label("Invalid UTF-8 shown as hex");
    self::assert_payload_copies(&mut harness, "00 0a ff");

    endpoint.exchange_payload(b"").unwrap();
    assert!(harness.step_until(|_| capture.snapshot().len() == 4));
    harness.step();
    self::select_frame(&mut harness, "#2   Request");
    harness.get_by_label("Empty payload");
    assert!(
        harness
            .get_by_label("Copy payload")
            .accesskit_node()
            .is_disabled()
    );

    endpoint.exchange_payload(b"a\0b").unwrap();
    assert!(harness.step_until(|_| capture.snapshot().len() == 6));
    harness.step();
    self::select_frame(&mut harness, "#3   Request");
    harness.get_by_label("NUL bytes shown as hex");
    self::assert_payload_copies(&mut harness, "61 00 62");

    for _ in 0..MAX_EVICTION_EXCHANGES {
        if capture
            .snapshot()
            .iter()
            .all(|frame| frame.exchange_id != 3)
        {
            break;
        }
        endpoint.exchange_payload(b"!").unwrap();
    }
    assert!(
        capture
            .snapshot()
            .iter()
            .all(|frame| frame.exchange_id != 3)
    );
    harness.step();
    assert!(
        harness
            .get_by_label("Copy payload")
            .accesskit_node()
            .is_disabled()
    );
    assert!(harness.query_by_label("NUL bytes shown as hex").is_none());
}

fn capture(harness: &Harness<'_, App>) -> TrafficCapture {
    harness
        .state()
        .session_bridge
        .server()
        .unwrap()
        .traffic_capture()
}

fn open_traffic(harness: &mut Harness<'_, App>) {
    harness.step();
    harness.get_by_label("File").click();
    harness.run_steps(2);
    harness.get_by_label(MENU_LABEL).click();
    harness.run_steps(3);
    harness.get_by_role_and_label(Role::Window, WINDOW_TITLE);
}

fn select_frame(harness: &mut Harness<'_, App>, label: &str) {
    harness.get_by_label_contains(label).click();
    harness.step();
}

fn assert_payload_copies(harness: &mut Harness<'_, App>, expected: &str) {
    harness.get_by_label(expected);
    harness.get_by_label("Copy payload").click();
    harness.step();
    let clipboard = harness
        .output()
        .platform_output
        .commands
        .iter()
        .find_map(|command| match command {
            OutputCommand::CopyText(text) => Some(text.as_str()),
            _ => None,
        });
    assert_eq!(clipboard, Some(expected));
}

const MENU_LABEL: &str = "Session bridge traffic…";
const WINDOW_TITLE: &str = "Session bridge traffic";
const LIST_REQUEST: &[u8] = b" \r\n{ \"limit\": 1, \"method\": \"list_recordings\" } \n";
const MAX_EVICTION_EXCHANGES: usize = 128;
