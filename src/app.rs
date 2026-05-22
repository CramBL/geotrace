use nav_map::NavMap;

pub struct App {
    map: NavMap,
    load_error: Option<String>,
}

impl App {
    /// Called once before the first frame.
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        Self {
            map: NavMap::new(cc.egui_ctx.clone()),
            load_error: None,
        }
    }

    fn load_file(&mut self, path: &std::path::Path) {
        match nav_io::load(path) {
            Ok((points, markers)) => {
                self.map.add_points(points);
                self.map.add_markers(markers);
                self.load_error = None;
            }
            Err(e) => {
                log::error!("Failed to load {path:?}: {e}");
                self.load_error = Some(e.to_string());
            }
        }
    }
}

impl eframe::App for App {
    fn save(&mut self, _storage: &mut dyn eframe::Storage) {}

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Consume any files dropped onto the window this frame.
        let dropped = ui.ctx().input(|i| i.raw.dropped_files.clone());
        for file in dropped {
            if let Some(path) = &file.path
                && path
                    .extension()
                    .is_some_and(|ext| ext.eq_ignore_ascii_case("nvd"))
            {
                self.load_file(path);
            }
        }

        egui::Panel::top("top_panel").show_inside(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("Open…").clicked() {
                        ui.close();
                        if let Some(path) = rfd::FileDialog::new()
                            .add_filter("NaView Data", &["nvd"])
                            .pick_file()
                        {
                            self.load_file(&path);
                        }
                    }
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        ui.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.add_space(16.0);
                egui::widgets::global_theme_preference_buttons(ui);
            });
        });

        egui::CentralPanel::default().show_inside(ui, |ui| {
            ui.heading("naview");
            self.map.draw(ui);

            let mut dismiss = false;
            ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                egui::warn_if_debug_build(ui);
                if let Some(error) = &self.load_error {
                    ui.horizontal(|ui| {
                        ui.colored_label(
                            egui::Color32::from_rgb(220, 70, 50),
                            format!("⚠ {error}"),
                        );
                        dismiss = ui.small_button("✕").clicked();
                    });
                }
            });
            if dismiss {
                self.load_error = None;
            }
        });
    }
}
