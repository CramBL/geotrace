use std::sync::mpsc::{self, Receiver, TryRecvError};

use gt_filter::GlobalFilter;
use gt_loaded_files::{LoadedFilesView, RecordingData};
use gt_query::ChannelSchema;
use gt_query_run::{
    CheckRefresh, JammingValues, QuerySession, RunInputs, RunOutcome, SnapErrorValues,
};
use gt_types::Generation;
use gt_ui_types::{GeomagneticSeries, JammingSeries, TecSeries, TrackDataVisibility};

use super::{App, background_thread};

#[derive(Default)]
pub(super) struct BackgroundQuery {
    session: QuerySession,
    worker: Option<Receiver<RunOutcome>>,
}

impl BackgroundQuery {
    pub(super) fn with_session(session: QuerySession) -> Self {
        Self {
            session,
            worker: None,
        }
    }

    pub(super) fn start(&mut self, ctx: &egui::Context, inputs: RunInputs<'_>) -> bool {
        let Some(prepared) = self.session.start_run(inputs) else {
            return false;
        };
        let (sender, receiver) = mpsc::channel();
        let ctx = ctx.clone();
        background_thread::spawn_or_panic("query-run", move || {
            sender.send(prepared.execute()).ok();
            ctx.request_repaint();
        });
        self.worker = Some(receiver);
        true
    }

    pub(super) fn update(&mut self, inputs: RunInputs<'_>) -> bool {
        let received = self.receive_outcome();
        self.session.refresh_staleness(inputs);
        received
    }

    pub(super) fn clear_results(&mut self) {
        self.session.cancel_run();
        self.worker = None;
        self.session.clear_results();
    }

    pub(super) fn set_text(&mut self, text: String) {
        self.session.set_text(text);
    }

    pub(super) fn text_mut(&mut self) -> &mut String {
        self.session.text_mut()
    }

    pub(super) fn sync_checks(&mut self, schema: &ChannelSchema) -> CheckRefresh {
        self.session.sync_checks(schema)
    }

    pub(super) fn receive_outcome(&mut self) -> bool {
        let Some(worker) = &self.worker else {
            return false;
        };
        match worker.try_recv() {
            Ok(outcome) => {
                self.worker = None;
                self.session.finish_run(outcome);
                true
            }
            Err(TryRecvError::Empty) => false,
            Err(TryRecvError::Disconnected) => {
                log::error!("Query worker disconnected before sending an outcome");
                self.worker = None;
                self.session.abandon_run();
                false
            }
        }
    }

    pub(super) fn session(&self) -> &QuerySession {
        &self.session
    }
}

impl Drop for BackgroundQuery {
    fn drop(&mut self) {
        self.session.cancel_run();
    }
}

pub(super) struct QuerySeries {
    jamming: JammingSeries,
    geomagnetic: GeomagneticSeries,
    tec: TecSeries,
    snap_errors: SnapErrorValues,
    jamming_values: JammingValues,
    data_generation: Generation<RecordingData>,
}

impl QuerySeries {
    pub(super) fn capture(app: &mut App) -> Self {
        let snap_errors = app.snap_error_values();
        let shared = app.shared.borrow();
        let files = shared.loaded_files.view();
        let jamming = app.jamming.plot_series(files);
        Self {
            jamming,
            geomagnetic: app.geomagnetic_indices.plot_series(files),
            tec: app.tec_maps.plot_series(files),
            snap_errors,
            jamming_values: app.jamming.query_values(),
            data_generation: files.data_generation(),
        }
    }

    pub(super) fn jamming(&self) -> &JammingSeries {
        &self.jamming
    }

    pub(super) fn geomagnetic(&self) -> &GeomagneticSeries {
        &self.geomagnetic
    }

    pub(super) fn tec(&self) -> &TecSeries {
        &self.tec
    }

    pub(super) fn inputs<'a>(
        &'a self,
        loaded_files: LoadedFilesView<'a>,
        visibility: &'a TrackDataVisibility,
        filter: &'a GlobalFilter,
    ) -> Option<RunInputs<'a>> {
        (self.data_generation == loaded_files.data_generation()).then_some(RunInputs {
            loaded_files,
            visibility,
            filter,
            snap_errors: &self.snap_errors,
            jamming: &self.jamming_values,
            geomagnetic: &self.geomagnetic,
            tec: &self.tec,
        })
    }
}
