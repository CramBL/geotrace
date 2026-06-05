use egui_kittest::{Harness, SnapshotOptions};
use std::path::{Path, PathBuf};

fn snapshot_options() -> SnapshotOptions {
    SnapshotOptions::new().threshold(0.6)
}

fn on_non_macos_ci() -> bool {
    std::env::var("CI").is_ok() && !cfg!(target_os = "macos")
}

fn snap_name(name: &str) -> String {
    if name.starts_with("snap_") {
        name.to_owned()
    } else {
        format!("snap_{name}")
    }
}

/// Wrapper around [`egui_kittest::Harness`] for UI snapshot tests.
///
/// Skips image comparison on non-macOS CI runners (Linux/Windows use different
/// GPU backends that produce slightly different pixel output vs the Metal baseline).
/// Auto-prefixes snapshot names with `snap_` if not already present.
pub struct TestHarness<'a, State = ()> {
    pub inner: Harness<'a, State>,
    _temp_dir: Option<tempfile::TempDir>,
}

impl<'a> TestHarness<'a> {
    pub fn new_wgpu(size: egui::Vec2, f: impl FnMut(&mut egui::Ui) + 'a) -> Self {
        let inner = Harness::builder()
            .with_size(size)
            .with_options(snapshot_options())
            .wgpu()
            .build_ui(f);
        Self {
            inner,
            _temp_dir: None,
        }
    }
}

impl<'a, State> TestHarness<'a, State> {
    pub fn from_harness(inner: Harness<'a, State>) -> Self {
        Self {
            inner,
            _temp_dir: None,
        }
    }

    /// Builds a new eframe snapshot test harness with a temporary configuration directory.
    #[expect(
        clippy::expect_used,
        reason = "fatal setup failure in test harness should panic"
    )]
    pub fn new_eframe<F>(size: Option<egui::Vec2>, build_app: F) -> (Self, PathBuf)
    where
        F: FnOnce(&eframe::CreationContext<'_>, &Path) -> State,
        State: eframe::App + 'static,
    {
        let temp_dir = tempfile::tempdir().expect("failed to create temp config dir");
        let config_path = temp_dir.path().join("config.toml");
        let config_path_clone = config_path.clone();

        let mut builder = Harness::builder()
            .with_wait_for_pending_images(false)
            .with_options(snapshot_options());
        if let Some(sz) = size {
            builder = builder.with_size(sz);
        }
        let inner = builder.build_eframe(move |cc| build_app(cc, &config_path_clone));

        (
            Self {
                inner,
                _temp_dir: Some(temp_dir),
            },
            config_path,
        )
    }

    pub fn run(&mut self) {
        self.inner.run();
    }

    pub fn snapshot(&mut self, name: &str) {
        if on_non_macos_ci() {
            return;
        }
        self.inner
            .snapshot_options(snap_name(name), &snapshot_options());
    }

    /// Like [`snapshot`] but with a higher pixel-diff tolerance.
    ///
    /// Use this for snapshots that include live-rendered content (plots, maps)
    /// where minor floating-point layout differences produce a small number of
    /// differing pixels across runs.
    pub fn snapshot_loose(&mut self, name: &str) {
        if on_non_macos_ci() {
            return;
        }
        self.inner
            .snapshot_options(snap_name(name), &SnapshotOptions::new().threshold(4.0));
    }
}
