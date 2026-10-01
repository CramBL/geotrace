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
}

impl<'a> MetadataView<'a> {
    pub fn from_file_metadata(metadata: &'a FileMetadata, identity: Option<&'a str>) -> Self {
        Self {
            title: metadata.title.as_deref(),
            device: metadata.device.as_deref(),
            travel_mode: metadata.travel_mode.as_ref().map(TravelMode::display_name),
            identity,
            notes: metadata.notes.as_deref(),
        }
    }

    pub fn has_details(&self) -> bool {
        self.title.is_some()
            || self.device.is_some()
            || self.travel_mode.is_some()
            || self.identity.is_some()
            || self.notes.is_some()
    }

    pub fn show_ui(&self, ui: &mut egui::Ui) -> egui::Response {
        let rows: Vec<_> = [
            ("Title", self.title),
            ("Device", self.device),
            ("Travel mode", self.travel_mode),
            ("Identity", self.identity),
            ("Notes", self.notes),
        ]
        .into_iter()
        .filter_map(|(caption, value)| value.map(|value| DetailRow { caption, value }))
        .collect();
        DetailsLayout::new(&rows).show_ui(ui)
    }
}
