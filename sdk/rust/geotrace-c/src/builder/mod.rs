//! The opaque handle for a file under construction.

mod finish;
mod ingest;
mod lifecycle;
mod metadata;

use std::time::Duration;

use geotrace_sdk::{MetaStringWithNul, NavFileBuilder, NavRecorder, TravelMode};

use crate::error::{self, GtdStatus};

const METADATA_BEFORE_DATA: &str = "metadata must be set before adding data";

/// Opaque handle for a file-under-construction.
///
/// Created by `gtd_builder_create()`. Freed by `gtd_builder_finish()`, whatever status it
/// returns, or by `gtd_builder_destroy()` for a builder the caller does not finish.
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

    /// When `configure` returns an error, the builder stays unchanged: `configure` receives a
    /// clone.
    fn configure_before_data(
        &mut self,
        call_order_message: &str,
        configure: impl FnOnce(NavFileBuilder) -> Result<NavFileBuilder, MetaStringWithNul>,
    ) -> GtdStatus {
        let Some(builder) = &self.builder else {
            error::set_last_error(call_order_message);
            return GtdStatus::GTD_ERR_CALL_ORDER;
        };
        match configure(builder.clone()) {
            Ok(configured) => {
                self.builder = Some(configured);
                GtdStatus::GTD_OK
            }
            Err(string_with_nul) => {
                error::set_last_error(string_with_nul);
                GtdStatus::GTD_ERR_INVALID_ARGUMENT
            }
        }
    }

    fn set_title(&mut self, title: &str) -> GtdStatus {
        self.configure_before_data(METADATA_BEFORE_DATA, |b| b.with_title(title))
    }

    fn set_device(&mut self, device: &str) -> GtdStatus {
        self.configure_before_data(METADATA_BEFORE_DATA, |b| b.with_device(device))
    }

    fn set_notes(&mut self, notes: &str) -> GtdStatus {
        self.configure_before_data(METADATA_BEFORE_DATA, |b| b.with_notes(notes))
    }

    fn set_identity(&mut self, identity: &str) -> GtdStatus {
        self.configure_before_data(METADATA_BEFORE_DATA, |b| b.with_identity(identity))
    }

    fn set_travel_mode(&mut self, mode: TravelMode) -> GtdStatus {
        self.configure_before_data(METADATA_BEFORE_DATA, |b| b.with_travel_mode(mode))
    }

    fn set_lenient(&mut self) -> GtdStatus {
        self.configure_before_data("lenient mode must be set before adding data", |b| {
            Ok(b.with_lenient_errors())
        })
    }

    fn set_satellite_window(&mut self, window: Duration) -> GtdStatus {
        self.configure_before_data("the satellite window must be set before adding data", |b| {
            Ok(b.with_satellite_window(window))
        })
    }
}
