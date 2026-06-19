//! Startup update check.
//!
//! GeoTrace ships no separate updater binary. Instead the app checks for a newer
//! release on startup (in release builds, when not offline) and prompts the user.
//!
//! Two cases, distinguished by whether a dist install receipt is present:
//!
//! - **Receipt present** (installed via the shell / PowerShell installer):
//!   `axoupdater` can replace the binary in place, so we offer a one-click update.
//! - **No receipt** (Homebrew, MSI, or a manually downloaded build): we still
//!   detect a newer release, but we don't guess how it was installed; we just
//!   point the user at the downloads page and never try to self-replace.
//!
//! In both cases we drive `axoupdater`, which only matches releases that carry
//! the GeoTrace installer assets. SDK releases (tagged `geotrace-sdk-v*`) carry
//! no such assets, so they are ignored here automatically.

use std::{sync::Arc, thread};

use axoupdater::{AxoUpdater, ReleaseSource, ReleaseSourceType};
use parking_lot::Mutex;

/// The app name dist records in the install receipt and uses for installer
/// asset names. Must match the `geotrace` package/binary name.
const APP_NAME: &str = "geotrace";
const REPO_OWNER: &str = "CramBL";
const REPO_NAME: &str = "geotrace";
const RELEASES_URL: &str = "https://github.com/CramBL/geotrace/releases/latest";

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
    /// The new version was installed; the user must restart to apply it.
    Done,
    Failed(String),
}

/// Owns the startup update check and its prompt UI.
pub struct UpdateChecker {
    outcome: Arc<Mutex<Option<CheckOutcome>>>,
    install: Arc<Mutex<InstallStatus>>,
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

    /// Render the prompt when an update is available and not skipped/dismissed.
    /// Returns an event the app should act on (e.g. persist a skipped version).
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
        if skipped_version == Some(version.as_str()) {
            return None;
        }

        let mut event = None;
        let mut later = false;
        let mut skip = false;
        let mut start_install = false;
        let mut quit = false;

        let install_status = self.install.lock();
        egui::Window::new("Update available")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::vec2(0.0, 0.0))
            .show(ctx, |ui| {
                ui.label(format!(
                    "GeoTrace {version} is available (current: {}).",
                    env!("CARGO_PKG_VERSION")
                ));
                ui.add_space(6.0);

                match &*install_status {
                    InstallStatus::Done => {
                        ui.label("Update installed. Restart GeoTrace to use the new version.");
                        ui.add_space(6.0);
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
                            gt_ui_theme::WARNING_AMBER,
                            format!("Update failed: {err}"),
                        );
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.hyperlink_to("Download manually", RELEASES_URL);
                            if ui.button("Later").clicked() {
                                later = true;
                            }
                        });
                    }
                    InstallStatus::Idle => {
                        if self_update {
                            ui.horizontal(|ui| {
                                if ui.button("Update and restart").clicked() {
                                    start_install = true;
                                }
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
                        } else {
                            // No receipt: this build wasn't placed by the
                            // shell/PowerShell installer, so we can't self-replace.
                            // We don't guess at the install method; just point the
                            // user at the downloads page.
                            ui.label(
                                "This build can't update itself (it wasn't installed by the \
                                 GeoTrace installer). Download the new version:",
                            );
                            ui.add_space(6.0);
                            ui.horizontal(|ui| {
                                ui.hyperlink_to("Open downloads", RELEASES_URL);
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
                }
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
/// `axoupdater`'s async API (reqwest needs a reactor; there is none on a bare
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
