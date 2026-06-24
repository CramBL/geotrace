#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
pub mod app;
pub mod settings;
pub mod terms;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Minimum 2D texture dimension to request from wgpu, matching eframe's
/// default: the surface/depth textures must cover 4k+ displays.
const MIN_TEXTURE_DIMENSION_2D: u32 = 8192;

fn main() -> eframe::Result {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();

    // CLI flags that must not open a window. Release smoke tests run
    // `geotrace --version` to confirm the installed binary launches without the
    // GUI. On Windows the release build has no attached console, so the clean
    // exit code is the cross-platform signal and the printed version is checked
    // on Unix.
    if raw_args.iter().any(|a| a == "--version" || a == "-V") {
        println!("geotrace {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if raw_args.iter().any(|a| a == "--help" || a == "-h") {
        println!(
            "GeoTrace {} - GNSS navigation data visualizer\n\n\
             Usage: geotrace [FILES]...\n\n\
             Arguments:\n  \
             [FILES]...  .gtd recordings or .log files to open on startup\n\n\
             Options:\n  \
             -V, --version  Print version and exit\n  \
             -h, --help     Print help and exit",
            env!("CARGO_PKG_VERSION")
        );
        return Ok(());
    }

    // Dependencies whose debug logging floods the output with per-frame or
    // per-request noise, capped at warn so `RUST_LOG=debug` stays readable
    // for GeoTrace's own logs:
    // - winit traces a span for every window call. Through the tracing-log
    //   bridge they arrive under both the module path and `tracing::span`.
    // - The wgpu stack logs every surface reconfiguration, shader
    //   compilation, and resource creation.
    // - walkers and its HTTP stack log every tile decode, request, and
    //   connection-pool event.
    const NOISY_DEPENDENCY_LOGS: &[&str] = &[
        "winit",
        "tracing::span",
        "wgpu_core",
        "wgpu_hal",
        "naga",
        "walkers",
        "reqwest",
        "hyper",
        "hyper_util",
        "h2",
        "rustls",
    ];
    let mut log_builder = env_logger::Builder::from_env(env_logger::Env::default());
    for module in NOISY_DEPENDENCY_LOGS {
        log_builder.filter_module(module, log::LevelFilter::Warn);
    }
    log_builder.init();

    let initial_paths: Vec<std::path::PathBuf> =
        raw_args.iter().map(std::path::PathBuf::from).collect();

    // Safety net for very large recordings: egui packs the whole frame into
    // one vertex buffer, and eframe's default device limits cap buffers at
    // 256 MiB - exceeding that is a fatal wgpu validation error. Request the
    // adapter's actual maximum instead. The renderers also decimate to keep
    // vertex counts low, so this should rarely be needed.
    let mut wgpu_setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
    wgpu_setup.device_descriptor = std::sync::Arc::new(|adapter| {
        let base_limits = if adapter.get_info().backend == eframe::wgpu::Backend::Gl {
            eframe::wgpu::Limits::downlevel_webgl2_defaults()
        } else {
            eframe::wgpu::Limits::default()
        };
        eframe::wgpu::DeviceDescriptor {
            label: Some("egui wgpu device"),
            required_limits: eframe::wgpu::Limits {
                max_texture_dimension_2d: base_limits
                    .max_texture_dimension_2d
                    .max(MIN_TEXTURE_DIMENSION_2D),
                max_buffer_size: adapter.limits().max_buffer_size,
                ..base_limits
            },
            ..Default::default()
        }
    });

    let mut viewport = egui::ViewportBuilder::default()
        .with_inner_size([800.0, 600.0])
        .with_min_inner_size([400.0, 320.0])
        .with_drag_and_drop(true);
    // The window/taskbar icon. The Windows executable also embeds it via winres
    // (build.rs) so Explorer and the installer's shortcuts show it too.
    match eframe::icon_data::from_png_bytes(include_bytes!("../assets/geotrace_icon.png")) {
        Ok(icon) => viewport = viewport.with_icon(icon),
        Err(error) => log::warn!("could not load the app icon: {error}"),
    }

    let native_options = eframe::NativeOptions {
        viewport,
        wgpu_options: eframe::egui_wgpu::WgpuConfiguration {
            wgpu_setup: wgpu_setup.into(),
            ..Default::default()
        },
        ..Default::default()
    };
    eframe::run_native(
        concat!("GeoTrace v", env!("CARGO_PKG_VERSION")),
        native_options,
        Box::new(move |cc| {
            Ok(Box::new(app::App::new_with_files(
                cc,
                &initial_paths,
                app::StartupOptions::default(),
            )))
        }),
    )
}

#[cfg(test)]
mod tests {
    // The window icon is embedded at compile time and decoded at startup, so a
    // corrupt asset would only surface when someone launches the GUI. Decode it
    // here so CI fails loudly instead.
    #[test]
    fn embedded_app_icon_is_a_valid_png() {
        let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/geotrace_icon.png"))
            .expect("embedded app icon (assets/geotrace_icon.png) must be a valid PNG");
        assert!(icon.width > 0 && icon.height > 0);
    }
}
