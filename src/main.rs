#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
pub mod app;
pub mod settings;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

fn main() -> eframe::Result {
    env_logger::init();

    let initial_paths: Vec<std::path::PathBuf> = std::env::args()
        .skip(1)
        .map(std::path::PathBuf::from)
        .collect();

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([400.0, 320.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    eframe::run_native(
        "GeoTrace",
        native_options,
        Box::new(move |cc| Ok(Box::new(app::App::new_with_files(cc, &initial_paths)))),
    )
}
