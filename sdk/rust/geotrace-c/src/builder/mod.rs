//! The opaque handle for a file under construction.

mod finish;
mod ingest;
mod lifecycle;
mod metadata;

use geotrace_sdk::{NavFileBuilder, NavRecorder, TravelMode};

use crate::error::{self, GtdStatus};

/// Opaque handle for a file-under-construction.
///
/// Created by `gtd_builder_create()`. Freed either by `gtd_builder_destroy()`
/// (on error paths) or consumed by `gtd_builder_finish()` (on success).
pub struct GtdFileBuilder {
    builder: Option<NavFileBuilder>,
    recorder: Option<NavRecorder>,
}

impl GtdFileBuilder {
    fn new() -> Self {
        Self {
            builder: Some(NavFileBuilder::new()),
            recorder: None,
        }
    }

    /// Transition from configuring to open, lazily on first data add.
    fn ensure_open(&mut self) {
        if self.recorder.is_none() {
            let b = self.builder.take().unwrap_or_default();
            self.recorder = Some(b.open());
        }
    }

    #[expect(clippy::panic, reason = "ensure_open guarantees recorder is Some")]
    fn recorder_mut(&mut self) -> &mut NavRecorder {
        self.ensure_open();
        match &mut self.recorder {
            Some(s) => s,
            None => panic!("geotrace-c: recorder is None after ensure_open - this is a bug"),
        }
    }

    pub(crate) fn into_recorder(mut self) -> NavRecorder {
        self.ensure_open();
        match self.recorder {
            Some(s) => s,
            None => NavFileBuilder::new().open(),
        }
    }

    fn set_title(&mut self, title: &str) -> GtdStatus {
        match self.builder.take() {
            Some(b) => {
                self.builder = Some(b.with_title(title));
                GtdStatus::GTD_OK
            }
            None => {
                error::set_last_error("metadata must be set before adding data");
                GtdStatus::GTD_ERR_INTERNAL
            }
        }
    }

    fn set_device(&mut self, device: &str) -> GtdStatus {
        match self.builder.take() {
            Some(b) => {
                self.builder = Some(b.with_device(device));
                GtdStatus::GTD_OK
            }
            None => {
                error::set_last_error("metadata must be set before adding data");
                GtdStatus::GTD_ERR_INTERNAL
            }
        }
    }

    fn set_notes(&mut self, notes: &str) -> GtdStatus {
        match self.builder.take() {
            Some(b) => {
                self.builder = Some(b.with_notes(notes));
                GtdStatus::GTD_OK
            }
            None => {
                error::set_last_error("metadata must be set before adding data");
                GtdStatus::GTD_ERR_INTERNAL
            }
        }
    }

    fn set_identity(&mut self, identity: &str) -> GtdStatus {
        match self.builder.take() {
            Some(b) => {
                self.builder = Some(b.with_identity(identity));
                GtdStatus::GTD_OK
            }
            None => {
                error::set_last_error("metadata must be set before adding data");
                GtdStatus::GTD_ERR_INTERNAL
            }
        }
    }

    fn set_travel_mode(&mut self, mode: TravelMode) -> GtdStatus {
        match self.builder.take() {
            Some(b) => {
                self.builder = Some(b.with_travel_mode(mode));
                GtdStatus::GTD_OK
            }
            None => {
                error::set_last_error("metadata must be set before adding data");
                GtdStatus::GTD_ERR_INTERNAL
            }
        }
    }

    fn set_lenient(&mut self) {
        match self.builder.take() {
            Some(b) => {
                self.builder = Some(b.with_lenient_errors());
            }
            None => {
                error::set_last_error("lenient mode must be set before adding data");
            }
        }
    }
}
