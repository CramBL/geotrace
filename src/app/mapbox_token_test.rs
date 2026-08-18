//! The Mapbox token test: one satellite tile requested with the entered token,
//! reported next to the token field in the settings window's Interface page.

use std::sync::Arc;
use std::thread;

use egui::{Button, RichText};
use gt_fetch::{HttpRequest, Transport, TransportSource};
use parking_lot::Mutex;

const TEST_BUTTON_LABEL: &str = "Test";
const TEST_HOVER: &str = "Fetch one satellite map tile with the entered token";
const RUNNING_HOVER: &str = "The tile request is still running";
const WITHOUT_TOKEN_HOVER: &str = "Enter a token to test it";
const OFFLINE_HOVER: &str = "Testing is disabled in offline mode";
const RUNNING_STATUS: &str = "Testing…";

/// The statuses Mapbox answers with when it does not accept the token.
const TOKEN_REJECTED_STATUSES: [u16; 2] = [401, 403];

/// What the tile request reported.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MapboxTokenTestOutcome {
    TileFetched,
    TokenRejected { status_line: String },
    RequestFailed { detail: String },
}

impl std::fmt::Display for MapboxTokenTestOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TileFetched => write!(f, "Token accepted"),
            Self::TokenRejected { status_line } => {
                write!(f, "Mapbox rejected the token: {status_line}")
            }
            Self::RequestFailed { detail } => write!(f, "Tile request failed: {detail}"),
        }
    }
}

impl MapboxTokenTestOutcome {
    fn color(&self, dark_mode: bool) -> egui::Color32 {
        match self {
            Self::TileFetched => gt_ui_theme::SUCCESS_GREEN,
            Self::TokenRejected { .. } | Self::RequestFailed { .. } => {
                gt_ui_theme::error_indicator(dark_mode)
            }
        }
    }
}

#[derive(Default)]
enum MapboxTokenTestState {
    #[default]
    Idle,
    Running,
    Finished(MapboxTokenTestOutcome),
}

/// Whether a test can start, and what stops it when it cannot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MapboxTokenTestReadiness {
    Ready,
    /// No request may leave the machine: GeoTrace runs offline.
    Offline,
    WithoutToken,
    /// One test at a time: a request is in flight.
    Running,
}

impl MapboxTokenTestReadiness {
    /// Why the test button is grayed.
    fn blocked_hover(self) -> &'static str {
        match self {
            Self::Ready => TEST_HOVER,
            Self::Offline => OFFLINE_HOVER,
            Self::WithoutToken => WITHOUT_TOKEN_HOVER,
            Self::Running => RUNNING_HOVER,
        }
    }
}

/// The test button's state and the outcome it last reported. The worker thread
/// writes the outcome and requests a repaint.
#[derive(Default)]
pub struct MapboxTokenTest {
    state: Arc<Mutex<MapboxTokenTestState>>,
}

impl MapboxTokenTest {
    pub fn show_test_button_and_result(
        &self,
        ui: &mut egui::Ui,
        token: &str,
        transport: TransportSource,
    ) {
        let readiness = self.readiness(token, transport);
        let started = ui
            .add_enabled(
                readiness == MapboxTokenTestReadiness::Ready,
                Button::new(TEST_BUTTON_LABEL),
            )
            .on_hover_text(TEST_HOVER)
            .on_disabled_hover_text(readiness.blocked_hover())
            .clicked();

        match &*self.state.lock() {
            MapboxTokenTestState::Idle => {}
            MapboxTokenTestState::Running => {
                ui.label(RichText::new(RUNNING_STATUS).weak());
            }
            MapboxTokenTestState::Finished(outcome) => {
                ui.label(
                    RichText::new(outcome.to_string()).color(outcome.color(ui.visuals().dark_mode)),
                );
            }
        }

        if started {
            self.start(token.to_owned(), transport, ui.ctx().clone());
        }
    }

    fn readiness(&self, token: &str, transport: TransportSource) -> MapboxTokenTestReadiness {
        if transport == TransportSource::Offline {
            MapboxTokenTestReadiness::Offline
        } else if token.is_empty() {
            MapboxTokenTestReadiness::WithoutToken
        } else if matches!(*self.state.lock(), MapboxTokenTestState::Running) {
            MapboxTokenTestReadiness::Running
        } else {
            MapboxTokenTestReadiness::Ready
        }
    }

    fn start(&self, token: String, transport: TransportSource, ctx: egui::Context) {
        self.spawn(ctx, move || match transport.connect(None) {
            Ok(connection) => fetch_test_tile(&connection, &token),
            Err(err) => MapboxTokenTestOutcome::RequestFailed { detail: err.detail },
        });
    }

    /// Runs `request` off the UI thread and repaints once its outcome lands.
    fn spawn(
        &self,
        ctx: egui::Context,
        request: impl FnOnce() -> MapboxTokenTestOutcome + Send + 'static,
    ) {
        *self.state.lock() = MapboxTokenTestState::Running;
        let state = Arc::clone(&self.state);
        let spawned = thread::Builder::new()
            .name("mapbox-token-test".to_owned())
            .spawn(move || {
                let outcome = request();
                *state.lock() = MapboxTokenTestState::Finished(outcome);
                ctx.request_repaint();
            });
        if let Err(err) = spawned {
            *self.state.lock() =
                MapboxTokenTestState::Finished(MapboxTokenTestOutcome::RequestFailed {
                    detail: format!("{err:#}"),
                });
        }
    }

    #[cfg(test)]
    fn finished_outcome(&self) -> Option<MapboxTokenTestOutcome> {
        match &*self.state.lock() {
            MapboxTokenTestState::Finished(outcome) => Some(outcome.clone()),
            MapboxTokenTestState::Idle | MapboxTokenTestState::Running => None,
        }
    }
}

/// Sends one request for the tile the satellite layer draws at its lowest zoom,
/// over the transport that leaves the PNG body undecoded.
fn fetch_test_tile(transport: &impl Transport<Vec<u8>>, token: &str) -> MapboxTokenTestOutcome {
    let request = HttpRequest::get(gt_map::mapbox_tiles::token_test_tile_url(token));
    let response = match transport.send(&request) {
        Ok(response) => response,
        Err(err) => return MapboxTokenTestOutcome::RequestFailed { detail: err.detail },
    };
    if response.is_success() {
        return MapboxTokenTestOutcome::TileFetched;
    }
    if TOKEN_REJECTED_STATUSES.contains(&response.status) {
        return MapboxTokenTestOutcome::TokenRejected {
            status_line: response.status_line(),
        };
    }
    MapboxTokenTestOutcome::RequestFailed {
        detail: response.status_line(),
    }
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, Instant};

    use egui_kittest::kittest::{NodeT as _, Queryable as _};
    use gt_fetch::{BytesResponse, TransportError};
    use gt_test_utils::{ScriptedTransport, TestHarness, TransportAnswer};
    use rstest::rstest;

    use super::*;

    fn tile(status: u16) -> TransportAnswer<Vec<u8>> {
        Ok(BytesResponse {
            status,
            body: Vec::new(),
        })
    }

    fn unreachable_host() -> TransportAnswer<Vec<u8>> {
        Err(TransportError {
            detail: "connection refused".to_owned(),
        })
    }

    /// Every answer one tile request can come back with, and the line the row
    /// shows for it.
    #[rstest]
    #[case::fetched(tile(200), "Token accepted")]
    #[case::unauthorized(tile(401), "Mapbox rejected the token: 401 Unauthorized")]
    #[case::forbidden(tile(403), "Mapbox rejected the token: 403 Forbidden")]
    #[case::not_found(tile(404), "Tile request failed: 404 Not Found")]
    #[case::server_error(tile(503), "Tile request failed: 503 Service Unavailable")]
    #[case::unreachable(unreachable_host(), "Tile request failed: connection refused")]
    fn the_result_reports_what_the_host_answered(
        #[case] answer: TransportAnswer<Vec<u8>>,
        #[case] expected: &str,
    ) {
        let transport = ScriptedTransport::in_order(vec![answer]);

        let outcome = fetch_test_tile(&transport, "tok");

        assert_eq!(outcome.to_string(), expected);
        assert_eq!(transport.sends(), 1, "one click sends one request");
    }

    #[test]
    fn a_finished_request_leaves_its_outcome_in_the_state() {
        let test = MapboxTokenTest::default();

        test.spawn(egui::Context::default(), || {
            fetch_test_tile(&ScriptedTransport::in_order(vec![tile(200)]), "tok")
        });

        let deadline = Instant::now() + Duration::from_secs(2);
        while test.finished_outcome().is_none() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(
            test.finished_outcome(),
            Some(MapboxTokenTestOutcome::TileFetched)
        );
    }

    fn row(
        test: MapboxTokenTest,
        token: &'static str,
        transport: TransportSource,
    ) -> TestHarness<'static, MapboxTokenTest> {
        let mut harness = TestHarness::builder().ui_state(
            move |ui, state: &mut MapboxTokenTest| {
                ui.horizontal(|ui| {
                    state.show_test_button_and_result(ui, token, transport);
                });
            },
            test,
        );
        harness.run();
        harness
    }

    /// Never hidden, per DESIGN.md: what blocks a test grays the button and
    /// says so on hover.
    #[rstest]
    #[case::offline(TransportSource::Offline, "tok", MapboxTokenTestReadiness::Offline)]
    #[case::without_a_token(TransportSource::Network, "", MapboxTokenTestReadiness::WithoutToken)]
    fn a_blocked_test_leaves_the_button_disabled(
        #[case] transport: TransportSource,
        #[case] token: &'static str,
        #[case] expected: MapboxTokenTestReadiness,
    ) {
        let test = MapboxTokenTest::default();
        assert_eq!(test.readiness(token, transport), expected);

        let harness = row(test, token, transport);

        assert!(
            harness
                .inner
                .get_by_label(TEST_BUTTON_LABEL)
                .accesskit_node()
                .is_disabled()
        );
    }

    #[rstest]
    #[case::ready(
        MapboxTokenTestReadiness::Ready,
        "Fetch one satellite map tile with the entered token"
    )]
    #[case::offline(
        MapboxTokenTestReadiness::Offline,
        "Testing is disabled in offline mode"
    )]
    #[case::without_a_token(MapboxTokenTestReadiness::WithoutToken, "Enter a token to test it")]
    #[case::running(MapboxTokenTestReadiness::Running, "The tile request is still running")]
    fn the_hover_names_what_blocks_the_test(
        #[case] readiness: MapboxTokenTestReadiness,
        #[case] expected: &str,
    ) {
        assert_eq!(readiness.blocked_hover(), expected);
    }

    /// A running test grays its own button and says so in the row.
    #[test]
    fn a_running_test_reports_itself_in_the_row() {
        let test = MapboxTokenTest::default();
        *test.state.lock() = MapboxTokenTestState::Running;

        let harness = row(test, "tok", TransportSource::Network);

        assert!(
            harness
                .inner
                .get_by_label(TEST_BUTTON_LABEL)
                .accesskit_node()
                .is_disabled()
        );
        harness.inner.get_by_label(RUNNING_STATUS);
    }

    #[rstest]
    #[case::fetched(MapboxTokenTestOutcome::TileFetched, "Token accepted")]
    #[case::rejected(
        MapboxTokenTestOutcome::TokenRejected { status_line: "401 Unauthorized".to_owned() },
        "Mapbox rejected the token: 401 Unauthorized"
    )]
    fn a_finished_test_shows_its_result_in_the_row(
        #[case] outcome: MapboxTokenTestOutcome,
        #[case] expected: &str,
    ) {
        let test = MapboxTokenTest::default();
        *test.state.lock() = MapboxTokenTestState::Finished(outcome);

        let harness = row(test, "tok", TransportSource::Network);

        harness.inner.get_by_label(expected);
    }
}
