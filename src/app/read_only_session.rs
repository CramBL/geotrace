//! What a read-only session shows: the marker in the window's corner, and the
//! reason the controls that would write give while they are grayed out.
//!
//! A session becomes read-only when the user starts it beside the instance
//! that owns the data directory, as [`super::instance_wait`] describes, and
//! stays read-only until GeoTrace is restarted.

use egui::RichText;
use gt_pending_writes::WriteAccess;
use gt_ui_theme::warning_amber;

use super::App;

pub(in crate::app) const READ_ONLY_MARKER_LABEL: &str = "read only";

/// The first sentence of the marker's hover text, which lists the writes the
/// session skips.
const WRITES_NOTHING: &str = "GeoTrace writes nothing this session: no recording is stored, no \
                              day is downloaded, and no setting is saved.";

/// What the controls that delete archived days or download new ones say.
pub(in crate::app) const READ_ONLY_ARCHIVES_HOVER: &str =
    "This session is read-only: it changes none of the archives";

/// What the controls that store, delete, prune or rename recordings say.
pub(in crate::app) const READ_ONLY_RECORDING_HISTORY_HOVER: &str =
    "This session is read-only: it changes nothing in the recording history";

impl App {
    /// The marker a read-only session keeps in the window's bottom-left
    /// corner, beside the debug-build warning.
    pub(in crate::app) fn show_read_only_marker(&self, ui: &mut egui::Ui) {
        if self.pending_writes.write_access() != WriteAccess::ReadOnly {
            return;
        }
        ui.label(
            RichText::new(READ_ONLY_MARKER_LABEL)
                .small()
                .color(warning_amber(ui.visuals().dark_mode)),
        )
        .on_hover_text(self.read_only_marker_hover_text());
    }

    fn read_only_marker_hover_text(&self) -> String {
        match self.data_directory_owner_process_id {
            Some(process_id) => format!(
                "{WRITES_NOTHING} Another GeoTrace (process {process_id}) owns the data directory."
            ),
            None => WRITES_NOTHING.to_owned(),
        }
    }
}
