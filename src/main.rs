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
    // Dependencies whose debug logging floods the output with per-frame or
    // per-request noise, capped at warn so `RUST_LOG=debug` stays readable
    // for GeoTrace's own logs:
    // - winit traces a span for every window call; through the tracing-log
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

    let initial_paths: Vec<std::path::PathBuf> = std::env::args()
        .skip(1)
        .map(std::path::PathBuf::from)
        .collect();

    // Safety net for very large recordings: egui packs the whole frame into
    // one vertex buffer, and eframe's default device limits cap buffers at
    // 256 MiB - exceeding that is a fatal wgpu validation error. Request the
    // adapter's actual maximum instead; the renderers also decimate to keep
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

    let native_options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size([800.0, 600.0])
            .with_min_inner_size([400.0, 320.0])
            .with_drag_and_drop(true),
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
