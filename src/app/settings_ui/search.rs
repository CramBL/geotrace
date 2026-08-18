//! The query field above the category rail, and the pages a query reaches.

use egui::TextEdit;
use egui_phosphor::regular::MAGNIFYING_GLASS as ICON_MAGNIFYING_GLASS;
use strum::IntoEnumIterator as _;

use crate::app::settings_ui::SettingsPage;

/// Explicit id the query field registers under, which is how a test focuses it
/// among the text fields the app behind the window renders.
pub(in crate::app) const QUERY_FIELD_ID: &str = "settings_search_query";

const FIELD_HOVER: &str = "Matches page names and the labels on each page. Escape clears the \
                           query, and closes the window once it is empty.";

/// Query the category rail is filtered by. Session state: the window opens
/// with an empty field every run.
#[derive(Default)]
pub(in crate::app) struct SettingsSearch {
    query: String,
}

/// A page the query reaches, with the labels on it that matched. Empty
/// `labels` is a page reached by its own name.
pub(super) struct PageMatch {
    pub(super) page: SettingsPage,
    pub(super) labels: Vec<&'static str>,
}

impl SettingsSearch {
    pub(super) fn show_query_field(&mut self, ui: &mut egui::Ui) {
        ui.add(
            TextEdit::singleline(&mut self.query)
                .id(egui::Id::new(QUERY_FIELD_ID))
                .hint_text(format!("{ICON_MAGNIFYING_GLASS} Search settings"))
                .desired_width(f32::INFINITY),
        )
        .on_hover_text(FIELD_HOVER);
    }

    pub(super) fn is_active(&self) -> bool {
        !self.query.is_empty()
    }

    pub(super) fn clear(&mut self) {
        self.query.clear();
    }

    /// Pages whose name or one of whose labels contains the query, compared
    /// without case, in rail order.
    pub(super) fn page_matches(&self) -> Vec<PageMatch> {
        let query = self.query.to_lowercase();
        SettingsPage::iter()
            .filter_map(|page| {
                let labels: Vec<&'static str> = page
                    .searchable_labels()
                    .iter()
                    .copied()
                    .filter(|label| label.to_lowercase().contains(&query))
                    .collect();
                let name_matches = page.rail_label().to_lowercase().contains(&query);
                (name_matches || !labels.is_empty()).then_some(PageMatch { page, labels })
            })
            .collect()
    }
}
