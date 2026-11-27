use std::io;
use std::time::{Duration, Instant};

use egui::{Event, InputState};
use gt_session_bridge::{ErrorCode, Reply, Request, Server, SessionExchange};

use super::App;
use super::session_traffic_window::SessionTrafficWindow;

#[derive(Default)]
pub(super) enum SessionBridge {
    #[default]
    Stopped,
    Running(Server),
    StartFailed(io::Error),
    Disconnected,
}

impl SessionBridge {
    pub(super) fn server(&self) -> Option<&Server> {
        match self {
            Self::Running(server) if server.is_running() => Some(server),
            _ => None,
        }
    }
}

#[derive(Default)]
pub(super) struct SessionNavigation {
    raw_input_pending: Option<bool>,
    waiting: Option<NavigationAfterInput>,
}

impl SessionNavigation {
    pub(super) fn observe_raw_input(&mut self, ctx: &egui::Context, input: &egui::RawInput) {
        self.raw_input_pending = Some(
            self::has_navigation_events(&input.events)
                || !input.dropped_files.is_empty()
                || ctx.input(|input| input.pointer.any_down()),
        );
    }

    fn receive(&mut self, ctx: &egui::Context, server: &mut Server) -> Option<SessionExchange> {
        // eframe retains unprocessed raw events while ticking logic without painting.
        let input_pending = self.raw_input_pending.take().unwrap_or_else(|| {
            ctx.input(|input| {
                input.viewport().visible() != Some(false) && self::navigation_input_pending(input)
            })
        });
        if let Some(waiting) = self.waiting.take()
            && waiting.exchange.client_is_waiting()
            && let Some(waiting) = waiting.reject_if_expired()
        {
            ctx.request_repaint_after(waiting.expires.saturating_duration_since(Instant::now()));
            self.waiting = Some(waiting);
            return None;
        }
        let exchange = server.receive()?;
        if input_pending
            && matches!(
                exchange.request(),
                Request::OpenQuery(_) | Request::OpenSetting(_)
            )
        {
            self.waiting = Some(NavigationAfterInput {
                exchange,
                expires: Instant::now() + NAVIGATION_INPUT_WAIT,
            });
            ctx.request_repaint_after(NAVIGATION_INPUT_WAIT);
            None
        } else {
            Some(exchange)
        }
    }
}

struct NavigationAfterInput {
    exchange: SessionExchange,
    expires: Instant,
}

impl NavigationAfterInput {
    fn reject_if_expired(self) -> Option<Self> {
        if Instant::now() >= self.expires {
            self.exchange.respond(|_| ErrorCode::Busy.into());
            None
        } else {
            Some(self)
        }
    }
}

pub(super) fn navigation_input_pending(input: &InputState) -> bool {
    input.smooth_scroll_delta() != egui::Vec2::ZERO
        || !input.raw.dropped_files.is_empty()
        || input.pointer.any_down()
        || input.pointer.any_released()
        || self::has_navigation_events(&input.events)
}

fn has_navigation_events(events: &[Event]) -> bool {
    events.iter().any(|event| {
        matches!(
            event,
            Event::AccessKitActionRequest(_)
                | Event::Copy
                | Event::Cut
                | Event::Ime(_)
                | Event::Key { pressed: true, .. }
                | Event::MouseWheel { .. }
                | Event::Paste(_)
                | Event::PointerButton { .. }
                | Event::Text(_)
                | Event::Touch { .. }
        )
    })
}

impl App {
    pub(crate) fn start_session_bridge(&mut self) {
        if self.shutdown_requested() || self.session_bridge.server().is_some() {
            return;
        }
        self.stop_session_bridge();
        let ctx = self.ctx.clone();
        let wake = move || ctx.request_repaint();
        #[cfg(test)]
        let started = match &self.session_bridge_endpoint {
            Some(endpoint) => endpoint.start_server(wake),
            None => Server::start(wake),
        };
        #[cfg(not(test))]
        let started = Server::start(wake);
        self.session_bridge = match started {
            Ok(server) => SessionBridge::Running(server),
            Err(error) => {
                log::warn!("Could not start MCP ({error})");
                SessionBridge::StartFailed(error)
            }
        };
    }

    pub(super) fn stop_session_bridge(&mut self) {
        self.session_bridge = SessionBridge::Stopped;
        self.release_session_bridge_state();
    }

    fn release_session_bridge_state(&mut self) {
        self.editor_query.release_active_session_job();
        self.session_navigation = SessionNavigation::default();
        self.session_traffic_window = SessionTrafficWindow::default();
    }

    pub(super) fn dispatch_session_request(&mut self, ctx: &egui::Context) {
        let exchange = match &mut self.session_bridge {
            SessionBridge::Running(server) if server.is_running() => {
                self.session_navigation.receive(ctx, server)
            }
            SessionBridge::Running(_) => {
                self.session_bridge = SessionBridge::Disconnected;
                self.release_session_bridge_state();
                None
            }
            _ => None,
        };
        if let Some(exchange) = exchange {
            self.respond_to_session_exchange(exchange);
        }
        // Send the job ID before copying the selected tracks.
        self.prepare_session_query(ctx);
    }

    pub(super) fn complete_session_navigation_after_input(&mut self, ctx: &egui::Context) {
        if let Some(waiting) = self
            .session_navigation
            .waiting
            .take_if(|_| !ctx.will_discard() && !ctx.input(|input| input.pointer.any_down()))
        {
            if let Some(waiting) = waiting.reject_if_expired() {
                self.respond_to_session_exchange(waiting.exchange);
            }
            ctx.request_repaint();
        }
    }

    fn respond_to_session_exchange(&mut self, exchange: SessionExchange) {
        exchange.respond(|request| {
            self.execute_session_request(request)
                .unwrap_or_else(Reply::from)
        });
    }
}

const NAVIGATION_INPUT_WAIT: Duration = Duration::from_millis(500);

#[cfg(test)]
mod tests {
    use std::time::Instant;

    use egui::{Event, RawInput};
    use egui_kittest::Harness;
    use gt_session_bridge::{ErrorCode, QueryText, Request};
    use rstest::rstest;

    use crate::app::test_util::{harness, session};

    #[rstest]
    #[case::before_logic(true)]
    #[case::during_ui(false)]
    fn expired_navigation_leaves_the_draft_unchanged(#[case] expire_before_logic: bool) {
        let mut harness = Harness::new_eframe(harness::transient_app);
        let reply = session::exchange_queued_request(
            &mut harness,
            &Request::OpenQuery(QueryText {
                text: "points".to_owned(),
            }),
            |harness| {
                let ctx = harness.ctx.clone();
                let mut unprocessed = RawInput {
                    events: vec![Event::Text("draft".to_owned())],
                    ..RawInput::default()
                };
                eframe::App::raw_input_hook(harness.state_mut(), &ctx, &mut unprocessed);
                let _ = harness::step_minimized_logic(harness);
                let app = harness.state_mut();
                assert!(!app.query_window.open);
                app.session_navigation.waiting.as_mut().unwrap().expires = Instant::now();
                if expire_before_logic {
                    eframe::App::raw_input_hook(app, &ctx, &mut unprocessed);
                    let _ = harness::step_minimized_logic(harness);
                } else {
                    app.complete_session_navigation_after_input(&ctx);
                }
                assert!(harness.state().session_navigation.waiting.is_none());
                harness.input_mut().events.extend(unprocessed.events);
                harness.run_steps(2);
            },
        );
        assert_eq!(reply, ErrorCode::Busy.into());
        let app = harness.state();
        assert!(!app.query_window.open);
        assert!(app.editor_query.session().text().is_empty());
    }
}
