#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
pub mod app;
pub mod settings;
pub mod terms;

use std::{path::PathBuf, process::ExitCode};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Minimum 2D texture dimension to request from wgpu, matching eframe's
/// default: the surface/depth textures must cover 4k+ displays.
const MIN_TEXTURE_DIMENSION_2D: u32 = 8192;

/// Extra `--help` line for the `--update` flag, present only in dist builds
/// that carry the updater. Empty otherwise so the flag is never advertised by a
/// build that cannot honor it.
#[cfg(feature = "self-update")]
const SELF_UPDATE_HELP: &str = "\n      --update   Update in place and exit";
#[cfg(not(feature = "self-update"))]
const SELF_UPDATE_HELP: &str = "";

/// What the command line asks the binary to do, resolved before any GUI setup.
///
/// Flags win over files, so `geotrace --version foo.gtd` prints the version and
/// never opens a window. The variants are ordered by that precedence.
#[derive(Debug, PartialEq, Eq)]
enum CliAction {
    /// Print the version and exit (`--version` / `-V`).
    Version,
    /// Print help and exit (`--help` / `-h`).
    Help,
    /// Update in place and exit (`--update`). Recognized in every build so a
    /// build without the updater can report it clearly rather than mistaking
    /// the flag for a file path; only dist builds can actually honor it.
    SelfUpdate,
    /// Launch the GUI, opening the given files on startup.
    Launch(Vec<PathBuf>),
}

impl CliAction {
    fn parse(args: &[String]) -> Self {
        let has = |flags: &[&str]| args.iter().any(|a| flags.contains(&a.as_str()));
        if has(&["--version", "-V"]) {
            Self::Version
        } else if has(&["--help", "-h"]) {
            Self::Help
        } else if has(&["--update"]) {
            Self::SelfUpdate
        } else {
            Self::Launch(args.iter().map(PathBuf::from).collect())
        }
    }
}

/// Run the headless in-place update and return the process exit code. This is
/// the CLI counterpart of the GUI prompt: a manual path for users on a wedged
/// install, and the entry point CI drives to exercise the updater end to end.
#[cfg(feature = "self-update")]
fn run_self_update_cli() -> ExitCode {
    match app::update::run_self_update() {
        Ok(()) => {
            println!("geotrace updated - restart to use the new version");
            ExitCode::SUCCESS
        }
        Err(error) => {
            eprintln!("update failed: {error}");
            ExitCode::FAILURE
        }
    }
}

/// Builds without the updater still recognize `--update` so they can report it
/// plainly instead of treating the flag as a file to open.
#[cfg(not(feature = "self-update"))]
fn run_self_update_cli() -> ExitCode {
    eprintln!("updating is not supported by this build");
    ExitCode::FAILURE
}

fn main() -> ExitCode {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();

    // Flags that must not open a window are handled before any GUI/GPU setup.
    // Release smoke tests run `geotrace --version` to confirm the installed
    // binary launches without the GUI. On Windows the release build has no
    // attached console, so the clean exit code is the cross-platform signal and
    // the printed version is checked on Unix.
    let initial_paths: Vec<PathBuf> = match CliAction::parse(&raw_args) {
        CliAction::Version => {
            println!("geotrace {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        CliAction::Help => {
            println!(
                "GeoTrace {} - GNSS navigation data visualizer\n\n\
                 Usage: geotrace [FILES]...\n\n\
                 Arguments:\n  \
                 [FILES]...  .gtd recordings or .log files to open on startup\n\n\
                 Options:\n  \
                 -V, --version  Print version and exit\n  \
                 -h, --help     Print help and exit{}",
                env!("CARGO_PKG_VERSION"),
                SELF_UPDATE_HELP,
            );
            return ExitCode::SUCCESS;
        }
        CliAction::SelfUpdate => return run_self_update_cli(),
        CliAction::Launch(paths) => paths,
    };

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
    let result = eframe::run_native(
        concat!("GeoTrace v", env!("CARGO_PKG_VERSION")),
        native_options,
        Box::new(move |cc| {
            Ok(Box::new(app::App::new_with_files(
                cc,
                &initial_paths,
                app::StartupOptions {
                    fading_enabled: true,
                    offline: gt_types::env::offline(),
                    storage: app::Storage::Default,
                    app_version: env!("CARGO_PKG_VERSION"),
                },
            )))
        }),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("geotrace exited with an error: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use rstest::rstest;

    use crate::CliAction;

    // The window icon is embedded at compile time and decoded at startup, so a
    // corrupt asset would only surface when someone launches the GUI. Decode it
    // here so CI fails loudly instead.
    #[test]
    fn embedded_app_icon_is_a_valid_png() {
        let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/geotrace_icon.png"))
            .expect("embedded app icon (assets/geotrace_icon.png) must be a valid PNG");
        assert!(icon.width > 0 && icon.height > 0);
    }

    #[rstest]
    #[case(&["--version"], CliAction::Version)]
    #[case(&["-V"], CliAction::Version)]
    #[case(&["--help"], CliAction::Help)]
    #[case(&["-h"], CliAction::Help)]
    #[case(&["--update"], CliAction::SelfUpdate)]
    fn flags_map_to_actions(#[case] args: &[&str], #[case] expected: CliAction) {
        let args: Vec<String> = args.iter().map(ToString::to_string).collect();
        assert_eq!(CliAction::parse(&args), expected);
    }

    #[test]
    fn no_args_launches_with_no_files() {
        assert_eq!(CliAction::parse(&[]), CliAction::Launch(vec![]));
    }

    #[test]
    fn bare_paths_launch_with_those_files() {
        let args = vec!["a.gtd".to_owned(), "b.log".to_owned()];
        assert_eq!(
            CliAction::parse(&args),
            CliAction::Launch(vec![PathBuf::from("a.gtd"), PathBuf::from("b.log")])
        );
    }

    // A flag beats any files on the line: `geotrace --version foo.gtd` prints the
    // version and never opens a window.
    #[test]
    fn a_flag_wins_over_files() {
        let args = vec!["--version".to_owned(), "foo.gtd".to_owned()];
        assert_eq!(CliAction::parse(&args), CliAction::Version);
    }
}
