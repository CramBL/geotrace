use egui_kittest::{Harness, SnapshotOptions};

fn snapshot_options() -> SnapshotOptions {
    SnapshotOptions::new().threshold(0.6)
}

/// Wrapper around [`egui_kittest::Harness`] for gt-map UI snapshot tests.
///
/// Configures wgpu rendering and a consistent snapshot threshold, and skips
/// image comparison on non-macOS CI runners (which use different GPU backends
/// that produce slightly different pixel output).
pub(crate) struct TestHarness<'a> {
    inner: Harness<'a>,
}

impl<'a> TestHarness<'a> {
    pub(crate) fn new_wgpu(size: egui::Vec2, f: impl FnMut(&mut egui::Ui) + 'a) -> Self {
        let inner = Harness::builder()
            .with_size(size)
            .with_options(snapshot_options())
            .wgpu()
            .build_ui(f);
        Self { inner }
    }

    pub(crate) fn run(&mut self) {
        self.inner.run();
    }

    /// Compare the rendered frame against the stored snapshot.
    ///
    /// Skipped on non-macOS CI: GitHub's Linux and Windows runners use different
    /// GPU backends (software GL / DirectX) that produce pixel-level differences
    /// vs. the Metal-rendered macOS baseline.
    pub(crate) fn snapshot(&mut self, name: &str) {
        let on_non_macos_ci = std::env::var("CI").is_ok() && !cfg!(target_os = "macos");
        if on_non_macos_ci {
            return;
        }
        self.inner.snapshot(name);
    }
}
