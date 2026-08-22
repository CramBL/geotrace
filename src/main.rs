#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
pub mod app;
pub mod settings;
mod termination_signal;
pub mod terms;

use std::time::Duration;
use std::{path::PathBuf, process::ExitCode};

use gt_pending_writes::PendingWrites;

use crate::app::shutdown;
use crate::termination_signal::{TERMINATION_SIGNAL_FLAG, TerminationSignalAction};

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

/// Minimum 2D texture dimension to request from wgpu, matching eframe's
/// default: the surface/depth textures must cover 4k+ displays.
const MIN_TEXTURE_DIMENSION_2D: u32 = 8192;

/// Under this flag GeoTrace sends no request: no map tiles, no downloads, no
/// snapping, no update check.
const OFFLINE_FLAG: &str = "--offline";

/// Extra `--help` line for the `--update` flag, present only in dist builds
/// that carry the updater. Empty otherwise so the flag is never advertised by a
/// build that cannot honor it.
#[cfg(feature = "self-update")]
const SELF_UPDATE_HELP: &str = "\n      --update   Update in place and exit";
#[cfg(not(feature = "self-update"))]
const SELF_UPDATE_HELP: &str = "";

/// The action requested on the command line, resolved before any GUI setup.
///
/// Flags win over files, so `geotrace --version foo.gtd` prints the version and
/// never opens a window. The variants are ordered by that precedence.
#[derive(Debug, PartialEq, Eq)]
enum CliAction {
    /// Print the version and exit (`--version` / `-V`).
    Version,
    /// Print help and exit (`--help` / `-h`).
    Help,
    /// Update in place and exit (`--update`). Recognized in every build so one
    /// without the updater can report it clearly. Only dist builds honor it.
    SelfUpdate,
    /// Launch the GUI, opening the given files on startup.
    Launch {
        paths: Vec<PathBuf>,
        /// [`OFFLINE_FLAG`] was given.
        offline: bool,
    },
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
            Self::Launch {
                paths: args
                    .iter()
                    .filter(|arg| arg.as_str() != OFFLINE_FLAG)
                    .map(PathBuf::from)
                    .collect(),
                offline: has(&[OFFLINE_FLAG]),
            }
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

#[cfg(not(feature = "self-update"))]
fn run_self_update_cli() -> ExitCode {
    eprintln!("updating is not supported by this build");
    ExitCode::FAILURE
}

/// How often the wait for the last writes stops to read the
/// termination-signal flag.
const TERMINATION_SIGNAL_CHECK_INTERVAL: Duration = Duration::from_millis(100);

/// Waits for the writes still running once the window is gone, which is where
/// "Run in background" leaves them, and reports the code to exit with.
fn begin_shutdown_and_wait_for_pending_writes(pending_writes: &PendingWrites) -> ExitCode {
    pending_writes.begin_shutdown();
    for status in pending_writes.snapshot().running {
        log::info!("Waiting for background work to finish: {}", status.label);
    }
    while !pending_writes.wait_until_idle_for(TERMINATION_SIGNAL_CHECK_INTERVAL) {
        if TERMINATION_SIGNAL_FLAG.take_action()
            == TerminationSignalAction::QuitLeavingWritesUnfinished
        {
            shutdown::log_writes_left_unfinished(
                shutdown::SECOND_SIGNAL_QUIT_CAUSE,
                pending_writes,
            );
            return ExitCode::from(shutdown::FORCE_QUIT_EXIT_CODE);
        }
    }
    log::info!("Shutdown complete");
    ExitCode::SUCCESS
}

fn main() -> ExitCode {
    let raw_args: Vec<String> = std::env::args().skip(1).collect();

    // Flags that must not open a window are handled before any GUI/GPU setup.
    // Release smoke tests run `geotrace --version` to confirm the installed
    // binary launches without the GUI. On Windows the release build has no
    // attached console, so the clean exit code is the cross-platform signal and
    // the printed version is checked on Unix.
    let (initial_paths, offline) = match CliAction::parse(&raw_args) {
        CliAction::Version => {
            println!("geotrace {}", env!("CARGO_PKG_VERSION"));
            return ExitCode::SUCCESS;
        }
        CliAction::Help => {
            println!(
                "GeoTrace {} - GNSS navigation data visualizer\n\n\
                 Usage: geotrace [OPTIONS] [FILES]...\n\n\
                 Arguments:\n  \
                 [FILES]...  .gtd recordings or .log files to open on startup\n\n\
                 Options:\n  \
                 -V, --version  Print version and exit\n  \
                 -h, --help     Print help and exit\n      \
                 --offline  Run without network access: no map tiles, downloads, \
                 snapping, or update check{}",
                env!("CARGO_PKG_VERSION"),
                SELF_UPDATE_HELP,
            );
            return ExitCode::SUCCESS;
        }
        CliAction::SelfUpdate => return run_self_update_cli(),
        CliAction::Launch { paths, offline } => (paths, offline),
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

    termination_signal::install_handler();

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
    let pending_writes = PendingWrites::default();
    let app_pending_writes = pending_writes.clone();
    let result = eframe::run_native(
        concat!("GeoTrace v", env!("CARGO_PKG_VERSION")),
        native_options,
        Box::new(move |cc| {
            Ok(Box::new(app::App::new_with_files(
                cc,
                &initial_paths,
                app::StartupOptions {
                    fading_enabled: true,
                    offline,
                    storage: app::Storage::DataDirectory,
                    app_version: env!("CARGO_PKG_VERSION"),
                    pending_writes: app_pending_writes,
                },
            )))
        }),
    );
    let shutdown_exit_code = begin_shutdown_and_wait_for_pending_writes(&pending_writes);
    match result {
        Ok(()) => shutdown_exit_code,
        Err(error) => {
            eprintln!("geotrace exited with an error: {error}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;
    use std::process::ExitCode;
    use std::thread;
    use std::time::Duration;

    use gt_pending_writes::{PendingWrites, WriteKind};
    use rstest::rstest;

    use crate::CliAction;
    use crate::app::shutdown;
    use crate::termination_signal::{TERMINATION_SIGNAL_FLAG, TerminationSignalAction};

    #[test]
    fn a_wait_with_nothing_left_to_finish_exits_clean() {
        assert_eq!(
            crate::begin_shutdown_and_wait_for_pending_writes(&PendingWrites::default()),
            ExitCode::SUCCESS
        );
    }

    /// Longer than [`TERMINATION_SIGNAL_CHECK_INTERVAL`]: the second signal
    /// always ends the wait first. Short enough that a wait that stopped
    /// reading the flag fails instead of hanging.
    const HELD_WRITE_RELEASED_AFTER: Duration = Duration::from_secs(5);

    /// The wait after the window closes reads the flag too: a signal there,
    /// with one already read, abandons the write still running.
    ///
    /// No other test sees the process-global flag this raises: every test
    /// runs in its own process under `cargo nextest`.
    #[test]
    fn a_second_signal_ends_the_wait_with_the_force_quit_code() {
        let pending_writes = PendingWrites::default();
        let compaction = pending_writes
            .try_begin(
                "Compacting the TEC archive",
                WriteKind::ArchiveCompaction {
                    archive: "ionospheric TEC",
                },
            )
            .expect("the registry is running");
        thread::Builder::new()
            .name("pending-write-holder".to_owned())
            .spawn(move || {
                thread::sleep(HELD_WRITE_RELEASED_AFTER);
                drop(compaction);
            })
            .expect("spawn the holding thread");
        TERMINATION_SIGNAL_FLAG.raise();
        assert_eq!(
            TERMINATION_SIGNAL_FLAG.take_action(),
            TerminationSignalAction::BeginShutdown
        );

        TERMINATION_SIGNAL_FLAG.raise();

        assert_eq!(
            crate::begin_shutdown_and_wait_for_pending_writes(&pending_writes),
            ExitCode::from(shutdown::FORCE_QUIT_EXIT_CODE)
        );
    }

    // Decodes independently of app startup so a corrupt embedded icon fails
    // CI instead of only surfacing when someone launches the GUI.
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
        assert_eq!(
            CliAction::parse(&[]),
            CliAction::Launch {
                paths: vec![],
                offline: false
            }
        );
    }

    #[test]
    fn bare_paths_launch_with_those_files() {
        let args = vec!["a.gtd".to_owned(), "b.log".to_owned()];
        assert_eq!(
            CliAction::parse(&args),
            CliAction::Launch {
                paths: vec![PathBuf::from("a.gtd"), PathBuf::from("b.log")],
                offline: false
            }
        );
    }

    #[test]
    fn the_offline_flag_launches_offline_without_becoming_a_file() {
        let args = vec!["--offline".to_owned(), "a.gtd".to_owned()];
        assert_eq!(
            CliAction::parse(&args),
            CliAction::Launch {
                paths: vec![PathBuf::from("a.gtd")],
                offline: true
            }
        );
    }

    #[test]
    fn a_flag_wins_over_files() {
        let args = vec!["--version".to_owned(), "foo.gtd".to_owned()];
        assert_eq!(CliAction::parse(&args), CliAction::Version);
    }
}
