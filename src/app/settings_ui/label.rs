use egui::{Label, WidgetInfo, WidgetType};

pub(super) struct SettingsLabel<'a> {
    pub(super) icon: &'a str,
    pub(super) text: &'a str,
}

impl SettingsLabel<'_> {
    pub(super) fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        let response = ui.add(Label::new(format!("{} {}", self.icon, self.text)).wrap());
        response.widget_info(|| {
            WidgetInfo::labeled(
                WidgetType::Label,
                ui.is_enabled(),
                format!("{} {}", self.icon, self.text.replace('\n', " ")),
            )
        });
        response
    }
}
