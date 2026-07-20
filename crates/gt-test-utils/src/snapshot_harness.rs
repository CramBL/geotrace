use egui_kittest::{Harness, SnapshotOptions};

/// Re-exported so crates using [`TestHarness`] can query widgets without each
/// taking a direct `egui_kittest` dev-dependency of their own.
pub use egui_kittest::kittest::Queryable;
use std::path::{Path, PathBuf};

fn snapshot_options() -> SnapshotOptions {
    SnapshotOptions::new().threshold(0.6)
}

/// Pixel-count tolerance for [`TestHarness::snapshot_loose`]. Live map/plot
/// snapshots differ by a handful of pixels between GPU backends (the baselines
/// are committed from Linux but CI compares them on the macOS runner), so a
/// small allowance keeps those tests stable without masking real regressions.
const LOOSE_PIXEL_COUNT_TOLERANCE: usize = 32;

/// Installs the Phosphor icon font and image loaders into the test context so
/// snapshots render real glyphs and SVG marker icons instead of fallback boxes.
///
/// Mirrors the production setup in `App::new_with_config`. `register_marker_icons`
/// is deliberately not called here: it lives in `gt-map` and is invoked by that
/// crate's own tests (calling it from here would invert the dependency direction).
fn install_icon_assets(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    egui_phosphor::add_to_fonts(&mut fonts, egui_phosphor::Variant::Regular);
    ctx.set_fonts(fonts);
    egui_extras::install_image_loaders(ctx);
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

impl<'a> TestHarness<'a, ()> {
    /// Entry point for all snapshot harnesses. Lives on the `State = ()` impl so
    /// `TestHarness::builder()` resolves without a turbofish. The builder's
    /// `ui_state`/`eframe` methods then pick the real `State`.
    pub fn builder() -> TestHarnessBuilder<'a> {
        TestHarnessBuilder::default()
    }
}

impl<'a, State> TestHarness<'a, State> {
    pub fn from_harness(inner: Harness<'a, State>) -> Self {
        Self {
            inner,
            _temp_dir: None,
        }
    }

    pub fn run(&mut self) {
        self.inner.run();
    }

    pub fn step(&mut self) {
        self.inner.step();
    }

    pub fn state(&self) -> &State {
        self.inner.state()
    }

    pub fn state_mut(&mut self) -> &mut State {
        self.inner.state_mut()
    }

    /// Resize the harness viewport to exactly fit the rendered content,
    /// then re-run to stabilise.  Use this instead of a hard-coded size
    /// when the content dimensions aren't known up front.
    pub fn fit_contents(&mut self) {
        self.inner.fit_contents();
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
    /// differing pixels across runs and across GPU backends (the committed
    /// baselines are compared against the macOS runner).
    pub fn snapshot_loose(&mut self, name: &str) {
        self.snapshot_with_tolerance(name, 4.0, LOOSE_PIXEL_COUNT_TOLERANCE);
    }

    pub fn snapshot_with_threshold(&mut self, name: &str, threshold: f32) {
        self.snapshot_with_tolerance(name, threshold, 0);
    }

    pub fn snapshot_with_tolerance(
        &mut self,
        name: &str,
        threshold: f32,
        failed_pixel_count_threshold: usize,
    ) {
        if on_non_macos_ci() {
            return;
        }
        self.inner.snapshot_options(
            snap_name(name),
            &SnapshotOptions::new()
                .threshold(threshold)
                .failed_pixel_count_threshold(failed_pixel_count_threshold),
        );
    }
}

pub struct TestHarnessBuilder<'a> {
    size: Option<egui::Vec2>,
    fading_enabled: bool,
    dark_mode: Option<bool>,
    step_dt: Option<f32>,
    _marker: std::marker::PhantomData<&'a ()>,
}

impl Default for TestHarnessBuilder<'_> {
    fn default() -> Self {
        Self {
            size: None,
            fading_enabled: false,
            dark_mode: None,
            step_dt: None,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<'a> TestHarnessBuilder<'a> {
    pub fn size(mut self, size: egui::Vec2) -> Self {
        self.size = Some(size);
        self
    }

    pub fn fading_enabled(mut self, enabled: bool) -> Self {
        self.fading_enabled = enabled;
        self
    }

    /// Simulated time per frame. kittest's default is a coarse 0.25 s (it
    /// saves CPU on animations) - too coarse for interactions inside egui's
    /// 0.3 s double-click window, where each queued event runs one frame.
    pub fn step_dt(mut self, step_dt: f32) -> Self {
        self.step_dt = Some(step_dt);
        self
    }

    /// Force the context into light or dark visuals before rendering, so a
    /// widget can be snapshotted under both themes. Without this the harness
    /// uses egui's default (dark) visuals, which is why light-mode regressions
    /// went uncaught. `true` selects dark, `false` light.
    pub fn theme(mut self, dark_mode: bool) -> Self {
        self.dark_mode = Some(dark_mode);
        self
    }

    /// Apply the requested theme to a freshly built harness and re-run so the
    /// first snapshot reflects it.
    fn apply_theme<State>(&self, inner: &mut Harness<'a, State>) {
        if let Some(dark_mode) = self.dark_mode {
            let visuals = if dark_mode {
                egui::Visuals::dark()
            } else {
                egui::Visuals::light()
            };
            inner.ctx.set_visuals(visuals);
            inner.run();
        }
    }

    /// Build a harness for a simple UI closure.
    pub fn ui<F>(self, f: F) -> TestHarness<'a>
    where
        F: FnMut(&mut egui::Ui) + 'a,
    {
        let mut builder = Harness::builder().with_options(snapshot_options()).wgpu();
        if let Some(sz) = self.size {
            builder = builder.with_size(sz);
        }
        if let Some(dt) = self.step_dt {
            builder = builder.with_step_dt(dt);
        }
        let mut inner = builder.build_ui(f);
        install_icon_assets(&inner.ctx);
        self.apply_theme(&mut inner);
        TestHarness {
            inner,
            _temp_dir: None,
        }
    }

    /// Build a harness for a UI closure with custom state.
    pub fn ui_state<F, State>(self, f: F, state: State) -> TestHarness<'a, State>
    where
        F: FnMut(&mut egui::Ui, &mut State) + 'a,
        State: 'static,
    {
        let mut builder = Harness::builder().with_options(snapshot_options()).wgpu();
        if let Some(sz) = self.size {
            builder = builder.with_size(sz);
        }
        if let Some(dt) = self.step_dt {
            builder = builder.with_step_dt(dt);
        }
        let mut inner = builder.build_ui_state(f, state);
        install_icon_assets(&inner.ctx);
        self.apply_theme(&mut inner);
        TestHarness {
            inner,
            _temp_dir: None,
        }
    }

    /// Build a harness for a full `eframe::App`, creating a temporary configuration directory.
    #[expect(
        clippy::expect_used,
        reason = "fatal setup failure in test harness should panic"
    )]
    pub fn eframe<F, State>(self, build_app: F) -> (TestHarness<'a, State>, PathBuf)
    where
        F: FnOnce(&eframe::CreationContext<'_>, &Path, bool) -> State,
        State: eframe::App + 'static,
    {
        let temp_dir = tempfile::tempdir().expect("failed to create temp config dir");
        let config_path = temp_dir.path().join("config.toml");
        let config_path_clone = config_path.clone();

        let mut builder = Harness::builder()
            .with_wait_for_pending_images(false)
            .with_options(snapshot_options());
        if let Some(sz) = self.size {
            builder = builder.with_size(sz);
        }
        let inner =
            builder.build_eframe(move |cc| build_app(cc, &config_path_clone, self.fading_enabled));

        (
            TestHarness {
                inner,
                _temp_dir: Some(temp_dir),
            },
            config_path,
        )
    }
}
