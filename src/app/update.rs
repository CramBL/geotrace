//! Startup update check.
//!
//! Compiled only under the `self-update` feature, which dist enables for the
//! released binaries. Builds from source or via package managers omit it and
//! carry no updater code at all.
//!
//! The app checks for a newer release on startup (in release builds, when not
//! offline). What it does with a found update depends on whether a dist install
//! receipt is present:
//!
//! - **Receipt present** (installed via the shell / PowerShell installer):
//!   `axoupdater` can replace the binary in place, so we show a prompt offering a
//!   one-click "Update and restart".
//! - **No receipt** (Homebrew, MSI, or a manually downloaded build): self-replace
//!   would fight the package manager, so we never pop a dialog. Instead the app
//!   shows a subtle "update available" hint next to the version (see
//!   [`UpdateChecker::badge_version`]). The user updates the normal way.
//!
//! Either way we drive `axoupdater`, which only matches releases that carry the
//! GeoTrace installer assets. SDK releases (tagged `geotrace-sdk-v*`) carry no
//! such assets, so they are ignored here automatically.

use egui::{Button, RichText, Window};
use std::{sync::Arc, thread};

use axoupdater::{AxoUpdater, ReleaseSource, ReleaseSourceType};
use parking_lot::Mutex;

/// The app name dist records in the install receipt and uses for installer
/// asset names. Must match the `geotrace` package/binary name.
const APP_NAME: &str = "geotrace";
const REPO_OWNER: &str = "CramBL";
const REPO_NAME: &str = "geotrace";
pub const RELEASES_URL: &str = "https://github.com/CramBL/geotrace/releases/latest";

/// Result of the background version check.
enum CheckOutcome {
    /// Already on the newest release.
    UpToDate,
    /// A newer release exists. `self_update` is `true` when an install receipt
    /// was found and `axoupdater` can replace the binary in place.
    Available { version: String, self_update: bool },
    /// The check could not complete (offline, rate-limited, no releases yet, …).
    /// Treated as "no update" and never surfaced to the user.
    Failed,
}

/// Progress of an in-place self-update.
enum InstallStatus {
    Idle,
    Running,
    /// The new version was installed. The user must restart to apply it.
    Done,
    Failed(String),
}

/// Owns the startup update check and its prompt UI.
pub struct UpdateChecker {
    outcome: Arc<Mutex<Option<CheckOutcome>>>,
    install: Arc<Mutex<InstallStatus>>,
    /// The running version, shown in the prompt as "(current: …)". Held as a
    /// field rather than read from `CARGO_PKG_VERSION` at render time so tests
    /// can pin it to a fixed value; otherwise the prompt snapshot would diff on
    /// every release as the version bumps.
    current_version: String,
    /// Whether the background check has been spawned this session.
    started: bool,
    /// Whether the user dismissed the prompt for this session ("Later").
    dismissed: bool,
}

/// Something the prompt needs the app to persist.
pub enum UpdateEvent {
    /// Remember this version as skipped so the prompt stays hidden for it.
    Skip(String),
}

impl UpdateChecker {
    pub fn new() -> Self {
        Self {
            outcome: Arc::new(Mutex::new(None)),
            install: Arc::new(Mutex::new(InstallStatus::Idle)),
            current_version: env!("CARGO_PKG_VERSION").to_owned(),
            started: false,
            dismissed: false,
        }
    }

    /// Test-only: a checker already showing an available update, so the prompt
    /// can be rendered and snapshotted without any network access. `started` is
    /// set so the real background check never runs and overwrites the state.
    #[cfg(test)]
    pub fn available_for_test(version: &str, self_update: bool) -> Self {
        let mut checker = Self::new();
        checker.started = true;
        // Pin the displayed current version so the prompt snapshot stays stable
        // across releases instead of tracking the live `CARGO_PKG_VERSION`.
        checker.current_version = "0.1.0".to_owned();
        *checker.outcome.lock() = Some(CheckOutcome::Available {
            version: version.to_owned(),
            self_update,
        });
        checker
    }

    /// Spawn the background check exactly once. Cheap to call every frame.
    pub fn start(&mut self, ctx: &egui::Context) {
        if self.started {
            return;
        }
        self.started = true;
        let outcome = Arc::clone(&self.outcome);
        let ctx = ctx.clone();
        let spawned = thread::Builder::new()
            .name("update-check".to_owned())
            .spawn(move || {
                let result = check_for_update();
                *outcome.lock() = Some(result);
                ctx.request_repaint();
            });
        if let Err(e) = spawned {
            log::warn!("could not spawn update-check thread: {e}");
        }
    }

    /// Render the self-update prompt when an in-place update is available and not
    /// skipped/dismissed. Only installs with a receipt (shell/PowerShell) reach
    /// the dialog. Package-manager and manual builds use [`Self::badge_version`]
    /// instead. Returns an event the app should act on (e.g. persist a skip).
    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        skipped_version: Option<&str>,
    ) -> Option<UpdateEvent> {
        if self.dismissed {
            return None;
        }
        // Hold the version/self_update locally so the lock is released before we
        // build the (potentially long-lived) UI closure.
        let (version, self_update) = {
            let guard = self.outcome.lock();
            match guard.as_ref() {
                Some(CheckOutcome::Available {
                    version,
                    self_update,
                }) => (version.clone(), *self_update),
                _ => return None,
            }
        };
        // Builds that can't self-replace never get a dialog (it would just fight
        // the package manager), they show a subtle badge instead.
        if !self_update {
            return None;
        }
        if skipped_version == Some(version.as_str()) {
            return None;
        }

        let mut event = None;
        let mut later = false;
        let mut skip = false;
        let mut start_install = false;
        let mut quit = false;

        let current_version = self.current_version.as_str();
        let install_status = self.install.lock();
        Window::new("Update available")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label(format!(
                        "GeoTrace {version} is available (current: {current_version})."
                    ));
                    ui.add_space(10.0);

                    match &*install_status {
                        InstallStatus::Done => {
                            ui.label("Update installed. Restart GeoTrace to use the new version.");
                            ui.add_space(8.0);
                            if ui.button("Quit now").clicked() {
                                quit = true;
                            }
                        }
                        InstallStatus::Running => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label("Downloading and installing…");
                            });
                        }
                        InstallStatus::Failed(err) => {
                            ui.colored_label(
                                gt_ui_theme::warning_amber(ui.visuals().dark_mode),
                                format!("Update failed: {err}"),
                            );
                            ui.add_space(8.0);
                            ui.horizontal(|ui| {
                                ui.hyperlink_to("Download manually", RELEASES_URL);
                                if ui.button("Later").clicked() {
                                    later = true;
                                }
                            });
                        }
                        InstallStatus::Idle => {
                            // Primary action: prominent, green, and the obvious default.
                            let update = Button::new(
                                RichText::new("Update and restart")
                                    .color(egui::Color32::WHITE)
                                    .strong(),
                            )
                            .fill(gt_ui_theme::SUCCESS_GREEN)
                            .min_size(egui::vec2(200.0, 30.0));
                            if ui.add(update).clicked() {
                                start_install = true;
                            }
                            ui.add_space(8.0);
                            // Lower-key "not right now" choices.
                            ui.horizontal(|ui| {
                                if ui.button("Later").clicked() {
                                    later = true;
                                }
                                if ui
                                    .button("Skip this version")
                                    .on_hover_text("Don't prompt again for this version")
                                    .clicked()
                                {
                                    skip = true;
                                }
                            });
                        }
                    }
                });
            });
        drop(install_status);

        if start_install {
            self.spawn_install(ctx);
        }
        if later {
            self.dismissed = true;
        }
        if skip {
            self.dismissed = true;
            event = Some(UpdateEvent::Skip(version));
        }
        if quit {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
        }
        event
    }

    /// Version of an available update that this build cannot self-install (no
    /// install receipt), for the subtle menu-bar "update available" hint.
    /// `None` when up to date, still checking, or a self-update prompt applies.
    pub fn badge_version(&self) -> Option<String> {
        match self.outcome.lock().as_ref() {
            Some(CheckOutcome::Available {
                version,
                self_update: false,
            }) => Some(version.clone()),
            _ => None,
        }
    }

    fn spawn_install(&self, ctx: &egui::Context) {
        *self.install.lock() = InstallStatus::Running;
        let install = Arc::clone(&self.install);
        let ctx = ctx.clone();
        let spawned = thread::Builder::new()
            .name("update-install".to_owned())
            .spawn(move || {
                let status = match run_self_update() {
                    Ok(()) => InstallStatus::Done,
                    Err(e) => InstallStatus::Failed(e),
                };
                *install.lock() = status;
                ctx.request_repaint();
            });
        if let Err(e) = spawned {
            log::warn!("could not spawn update-install thread: {e}");
            *self.install.lock() = InstallStatus::Failed("could not start updater".to_owned());
        }
    }
}

/// Build the current-thread runtime the background threads use to drive
/// `axoupdater`'s async API (reqwest needs a reactor, there is none on a bare
/// `std::thread`).
fn runtime() -> Result<tokio::runtime::Runtime, String> {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| e.to_string())
}

/// Version check, run on a background thread.
fn check_for_update() -> CheckOutcome {
    let rt = match runtime() {
        Ok(rt) => rt,
        Err(e) => {
            log::debug!("update check: could not build runtime: {e}");
            return CheckOutcome::Failed;
        }
    };
    rt.block_on(async {
        let mut updater = AxoUpdater::new_for(APP_NAME);
        let has_receipt = updater.load_receipt().is_ok();

        if !has_receipt {
            // Without a receipt we don't know the install method or the current
            // version, so supply both explicitly. axoupdater still only matches
            // releases carrying GeoTrace's installer assets, so SDK releases are
            // ignored.
            updater.set_release_source(ReleaseSource {
                release_type: ReleaseSourceType::GitHub,
                owner: REPO_OWNER.to_owned(),
                name: REPO_NAME.to_owned(),
                app_name: APP_NAME.to_owned(),
            });
            let Ok(current) = semver::Version::parse(env!("CARGO_PKG_VERSION")) else {
                return CheckOutcome::Failed;
            };
            if updater.set_current_version(current).is_err() {
                return CheckOutcome::Failed;
            }
        }

        match updater.is_update_needed().await {
            Ok(false) => CheckOutcome::UpToDate,
            Ok(true) => {
                let version = updater
                    .query_new_version()
                    .await
                    .ok()
                    .flatten()
                    .map(ToString::to_string)
                    .unwrap_or_default();
                CheckOutcome::Available {
                    version,
                    self_update: has_receipt,
                }
            }
            Err(e) => {
                log::debug!("update check did not complete: {e}");
                CheckOutcome::Failed
            }
        }
    })
}

/// In-place self-update via the install receipt, run on a background thread.
fn run_self_update() -> Result<(), String> {
    let rt = runtime()?;
    rt.block_on(async {
        let mut updater = AxoUpdater::new_for(APP_NAME);
        updater.load_receipt().map_err(|e| e.to_string())?;
        updater.run().await.map_err(|e| e.to_string())?;
        Ok(())
    })
}
