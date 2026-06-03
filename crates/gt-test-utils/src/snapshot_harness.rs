use egui_kittest::{Harness, SnapshotOptions};

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
    inner: Harness<'a, State>,
}

impl<'a> TestHarness<'a> {
    pub fn new_wgpu(size: egui::Vec2, f: impl FnMut(&mut egui::Ui) + 'a) -> Self {
        let inner = Harness::builder()
            .with_size(size)
            .with_options(snapshot_options())
            .wgpu()
            .build_ui(f);
        Self { inner }
    }
}

impl<'a, State> TestHarness<'a, State> {
    pub fn from_harness(inner: Harness<'a, State>) -> Self {
        Self { inner }
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
}
