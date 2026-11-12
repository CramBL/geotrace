use chrono::Duration;
use gt_types::{FileMetadata, TravelMode};

use crate::{DetailRow, DetailsLayout};

#[derive(Clone, Copy, Debug, Default)]
pub struct MetadataView<'a> {
    pub title: Option<&'a str>,
    pub device: Option<&'a str>,
    pub travel_mode: Option<&'a str>,
    /// The caller supplies the identity in its display form.
    pub identity: Option<&'a str>,
    pub notes: Option<&'a str>,
    pub debug_time_repair_threshold: Option<Duration>,
}

impl<'a> MetadataView<'a> {
    pub fn from_file_metadata(metadata: &'a FileMetadata, identity: Option<&'a str>) -> Self {
        Self {
            title: metadata.title.as_deref(),
            device: metadata.device.as_deref(),
            travel_mode: metadata.travel_mode.as_ref().map(TravelMode::display_name),
            identity,
            notes: metadata.notes.as_deref(),
            debug_time_repair_threshold: metadata.debug_time_repair_threshold,
        }
    }

    pub fn has_details(&self) -> bool {
        self.title.is_some()
            || self.device.is_some()
            || self.travel_mode.is_some()
            || self.identity.is_some()
            || self.notes.is_some()
            || self.debug_time_repair_threshold.is_some()
    }

    pub fn show_ui(&self, ui: &mut egui::Ui) -> egui::Response {
        let repair_details = self.debug_time_repair_threshold.map(|threshold| {
            format!(
                "Backward jump threshold {}s. System/boot deltas are computed from system timestamps loaded with debug time repair. Differences between shifted spans include synthetic shifts.",
                threshold.as_seconds_f64(),
            )
        });
        let rows: Vec<_> = [
            ("Title", self.title),
            ("Device", self.device),
            ("Travel mode", self.travel_mode),
            ("Identity", self.identity),
            ("Notes", self.notes),
            ("Debug time repair", repair_details.as_deref()),
        ]
        .into_iter()
        .filter_map(|(caption, value)| value.map(|value| DetailRow { caption, value }))
        .collect();
        DetailsLayout::new(&rows).show_ui(ui)
    }
}
