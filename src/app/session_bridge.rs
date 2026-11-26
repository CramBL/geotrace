use std::io;

use gt_session_bridge::{Reply, Server};

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
        self.session_traffic_window = SessionTrafficWindow::default();
    }

    pub(super) fn dispatch_session_request(&mut self, ctx: &egui::Context) {
        self.session_bridge = match std::mem::take(&mut self.session_bridge) {
            SessionBridge::Running(mut server) if server.is_running() => {
                server.dispatch(|request| {
                    self.execute_session_request(request)
                        .unwrap_or_else(Reply::from)
                });
                SessionBridge::Running(server)
            }
            SessionBridge::Running(_) => {
                self.release_session_bridge_state();
                SessionBridge::Disconnected
            }
            status => status,
        };
        // Send the job ID before copying the selected tracks.
        self.prepare_session_query(ctx);
    }
}
