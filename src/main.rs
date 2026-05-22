#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
pub mod app;

fn main() -> eframe::Result {
    env_logger::init();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([400.0, 320.0]),
        ..Default::default()
    };
    eframe::run_native(
        "naview",
        native_options,
        Box::new(|cc| Ok(Box::new(app::App::new(cc)))),
    )
}
