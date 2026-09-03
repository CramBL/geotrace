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
//!
//! [`run_self_update`] is also reachable headlessly via the `--update` CLI flag
//! (see `main.rs`): the same in-place update, without a window. It gives users
//! on a wedged install a manual path and lets CI exercise the updater end to
//! end.

use egui::{Button, Label, RichText};
use std::{sync::Arc, thread};

use axoupdater::{AxoUpdater, ReleaseSource, ReleaseSourceType};
use parking_lot::Mutex;

use crate::app::anchored_dialog::{AnchoredDialog, AnchoredDialogKind, HeldBodyLines};
use crate::app::modals::{DialogActionRow, DialogBody};

pub(in crate::app) const UPDATE_DIALOG_TITLE: &str = "Update available";

/// The region of the prompt's body that shows what the install reports. The
/// prompt opens before the user starts an install, and shows the progress and
/// the outcome of that install in the same window.
const INSTALL_STATUS_REGION: &str = "install_status";

/// Lines [`INSTALL_STATUS_REGION`] holds from the frame the prompt opens.
/// Each of the statements the install reports takes one line at this width.
const INSTALL_STATUS_LEAST_LINES: u8 = 1;

/// Lines [`INSTALL_STATUS_REGION`] holds at most: the one it reserves plus
/// three for the reason a failed install gives. A longer reason scrolls
/// inside that room.
const INSTALL_STATUS_MOST_LINES: u8 = 4;

/// The app name dist records in the install receipt and uses for installer
/// asset names. Must match the `geotrace` package/binary name.
const APP_NAME: &str = "geotrace";
const REPO_OWNER: &str = "CramBL";
const REPO_NAME: &str = "geotrace";
pub const RELEASES_URL: &str = "https://github.com/CramBL/geotrace/releases/latest";

/// Environment variables consulted for a GitHub token, in precedence order.
/// The first non-empty one authenticates release queries (see
/// [`apply_github_token`]).
const GITHUB_TOKEN_ENV_VARS: [&str; 2] = ["GITHUB_TOKEN", "GH_TOKEN"];

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
    /// The running version, shown in the prompt as "(current: …)". Injected
    /// from the app's single `app_version` so it is fixed in tests and never
    /// diffs a snapshot on a release bump.
    current_version: &'static str,
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
    pub fn new(app_version: &'static str) -> Self {
        Self {
            outcome: Arc::new(Mutex::new(None)),
            install: Arc::new(Mutex::new(InstallStatus::Idle)),
            current_version: app_version,
            started: false,
            dismissed: false,
        }
    }

    /// Test-only: a checker already showing an available update, so the prompt
    /// can be rendered and snapshotted without any network access. `started` is
    /// set so the real background check never runs and overwrites the state.
    #[cfg(test)]
    pub fn available_for_test(version: &str, self_update: bool) -> Self {
        // The same fixed placeholder the app injects in tests, so the prompt
        // snapshot stays stable across releases.
        let mut checker = Self::new(crate::app::TEST_APP_VERSION);
        checker.started = true;
        *checker.outcome.lock() = Some(CheckOutcome::Available {
            version: version.to_owned(),
            self_update,
        });
        checker
    }

    /// Test-only: the install the prompt started reports `error`, as the
    /// install thread does when it cannot replace the binary.
    #[cfg(test)]
    pub fn report_a_failed_install_for_test(&self, error: &str) {
        *self.install.lock() = InstallStatus::Failed(error.to_owned());
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

        let current_version = self.current_version;
        let install_status = self.install.lock();
        let dialog = AnchoredDialog::new(AnchoredDialogKind::UpdateAvailable, UPDATE_DIALOG_TITLE);
        let regions = dialog.regions();
        dialog.show(
            ctx,
            DialogBody::new(|ui| {
                ui.add(
                    Label::new(format!(
                        "GeoTrace {version} is available (current: {current_version})."
                    ))
                    .wrap(),
                );
                regions.frozen_at_open(
                    ui,
                    INSTALL_STATUS_REGION,
                    HeldBodyLines::at_least(INSTALL_STATUS_LEAST_LINES)
                        .and_at_most(INSTALL_STATUS_MOST_LINES),
                    |ui| match &*install_status {
                        InstallStatus::Idle => {}
                        InstallStatus::Running => {
                            ui.horizontal(|ui| {
                                ui.spinner();
                                ui.label(RichText::new("Downloading and installing…").weak());
                            });
                        }
                        InstallStatus::Done => {
                            ui.add(
                                Label::new(
                                    "Update installed. Restart GeoTrace to use the new version.",
                                )
                                .wrap(),
                            );
                        }
                        InstallStatus::Failed(err) => {
                            ui.add(
                                Label::new(
                                    RichText::new(format!("Update failed: {err}"))
                                        .color(gt_ui_theme::warning_amber(ui.visuals().dark_mode)),
                                )
                                .wrap(),
                            );
                        }
                    },
                );
            }),
            DialogActionRow::buttons(|ui| match &*install_status {
                // The row is empty while an install runs: nothing can stop
                // one that has started.
                InstallStatus::Running => {}
                InstallStatus::Done => {
                    if ui.button("Quit now").clicked() {
                        quit = true;
                    }
                }
                InstallStatus::Failed(_) => {
                    ui.hyperlink_to("Download manually", RELEASES_URL);
                    if ui.button("Later").clicked() {
                        later = true;
                    }
                }
                InstallStatus::Idle => {
                    let update = Button::new(
                        RichText::new("Update and restart")
                            .color(egui::Color32::WHITE)
                            .strong(),
                    )
                    .fill(gt_ui_theme::SUCCESS_GREEN);
                    if ui.add(update).clicked() {
                        start_install = true;
                    }
                    if ui
                        .button("Skip this version")
                        .on_hover_text("Don't prompt again for this version")
                        .clicked()
                    {
                        skip = true;
                    }
                    if ui.button("Later").clicked() {
                        later = true;
                    }
                }
            }),
        );
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
        apply_github_token(&mut updater);
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
            // The genuine version the updater compares against the latest
            // release - update logic, never a rendered string, so it must be
            // the real crate version, not the injectable display placeholder.
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

/// In-place self-update via the install receipt.
///
/// Runs on a background thread from the GUI prompt, and synchronously from the
/// `--update` CLI flag (see `main.rs`), which is how CI exercises the updater
/// end to end.
pub fn run_self_update() -> Result<(), String> {
    let rt = runtime()?;
    rt.block_on(async {
        let mut updater = AxoUpdater::new_for(APP_NAME);
        apply_github_token(&mut updater);
        updater.load_receipt().map_err(|e| e.to_string())?;
        updater.run().await.map_err(|e| e.to_string())?;
        Ok(())
    })
}

/// Authenticate GitHub release queries when a token is present in the
/// environment. Unauthenticated requests are the default, but they are heavily
/// rate-limited - CI passes `GITHUB_TOKEN`, and users behind a proxy can set it
/// too. No token is ever embedded in the binary.
fn apply_github_token(updater: &mut AxoUpdater) {
    if let Some(token) = select_github_token(|var| std::env::var(var).ok()) {
        updater.set_github_token(&token);
    }
}

/// The first non-empty token from [`GITHUB_TOKEN_ENV_VARS`], honoring their
/// precedence. Split from [`apply_github_token`] so the selection rule is pure
/// and unit-testable without touching the process environment.
fn select_github_token(lookup: impl Fn(&str) -> Option<String>) -> Option<String> {
    GITHUB_TOKEN_ENV_VARS
        .into_iter()
        .filter_map(lookup)
        .find(|token| !token.is_empty())
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::select_github_token;

    #[rstest]
    // GITHUB_TOKEN wins over GH_TOKEN when both are set.
    #[case(&[("GITHUB_TOKEN", "primary"), ("GH_TOKEN", "fallback")], Some("primary"))]
    // Falls back to GH_TOKEN when the primary is absent.
    #[case(&[("GH_TOKEN", "fallback")], Some("fallback"))]
    // An empty primary is skipped in favor of the next non-empty var.
    #[case(&[("GITHUB_TOKEN", ""), ("GH_TOKEN", "fallback")], Some("fallback"))]
    // No token when nothing is set.
    #[case(&[], None)]
    // No token when the only value present is empty.
    #[case(&[("GITHUB_TOKEN", "")], None)]
    fn selects_first_non_empty_token(#[case] env: &[(&str, &str)], #[case] expected: Option<&str>) {
        let selected = select_github_token(|var| {
            env.iter()
                .find(|(name, _)| *name == var)
                .map(|(_, value)| (*value).to_owned())
        });
        assert_eq!(selected.as_deref(), expected);
    }
}
