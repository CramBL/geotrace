use egui_kittest::{Harness, SnapshotOptions};

/// Re-exported so crates using [`TestHarness`] can query widgets, and read the
/// nodes they match, without each taking a direct `egui_kittest`
/// dev-dependency of their own.
pub use egui_kittest::Node;
pub use egui_kittest::kittest::{By, NodeT, Queryable};
use std::path::{Path, PathBuf};

/// Pixel-count tolerance for [`TestHarness::snapshot`]. Anti-aliased edges
/// differ by a gray level or two between driver versions: on three
/// `gt-side-panel` baselines 769 to 1582 pixels differed, 1 to 2 of them past
/// the 0.6 threshold. The smallest deliberate UI change measured against those
/// baselines, a 0.05 alpha step on one status glyph, put 8 pixels past it.
const STRICT_PIXEL_COUNT_TOLERANCE: usize = 4;

fn snapshot_options() -> SnapshotOptions {
    SnapshotOptions::new()
        .threshold(0.6_f32)
        .max_failed_pixels(STRICT_PIXEL_COUNT_TOLERANCE)
}

/// Per-pixel color tolerance for [`TestHarness::snapshot_loose`], as a squared
/// YIQ distance. 4.0 admits a difference of two gray levels per channel, which
/// is how far apart the macOS and the Linux rasterizer place an anti-aliased
/// edge. A one-pixel error in the History window's height put 10596 pixels
/// past it.
const LOOSE_PIXEL_COLOR_TOLERANCE: f32 = 4.0;

/// Pixel-count tolerance for [`TestHarness::snapshot_loose`]. Live map/plot
/// snapshots differ by a handful of pixels between GPU backends (the baselines
/// are committed from Linux but CI compares them on the macOS runner), so a
/// small allowance keeps those tests stable without masking real regressions.
const LOOSE_PIXEL_COUNT_TOLERANCE: usize = 32;

/// Snapshot comparison runs locally, on macOS CI (Metal), and on Linux CI
/// (Mesa's software rasterizer via `WGPU_BACKEND=gl`, deterministic across
/// runs). Windows CI is the only skip: its D3D output has no baseline
/// coverage and would need a third tolerance budget.
fn on_windows_ci() -> bool {
    std::env::var("CI").is_ok() && cfg!(target_os = "windows")
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
/// Image comparison runs everywhere except Windows CI (see [`on_windows_ci`]).
/// Auto-prefixes snapshot names with `snap_` if not already present.
pub struct TestHarness<'a, State = ()> {
    pub inner: Harness<'a, State>,
    _temp_dir: Option<tempfile::TempDir>,
}

impl<'a> TestHarness<'a, ()> {
    /// Entry point for all snapshot harnesses. Lives on the `State = ()` `impl`
    /// so `TestHarness::builder()` resolves without a turbofish. The builder's
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
    /// then re-run to stabilise.  Use this when the content dimensions aren't
    /// known up front.
    pub fn fit_contents(&mut self) {
        self.inner.fit_contents();
    }

    pub fn snapshot(&mut self, name: &str) {
        if on_windows_ci() {
            return;
        }
        self.inner
            .snapshot_options(snap_name(name), &snapshot_options());
    }

    /// Like [`TestHarness::snapshot`] but at [`LOOSE_PIXEL_COLOR_TOLERANCE`],
    /// with a budget of [`LOOSE_PIXEL_COUNT_TOLERANCE`] pixels.
    ///
    /// Use it for live-rendered content (plots, maps), whose floating-point
    /// layout differs by a few pixels between GPU backends. Use it also for an
    /// image far larger than the 280x600 side-panel baselines
    /// [`STRICT_PIXEL_COUNT_TOLERANCE`] was measured on: a larger image has
    /// more anti-aliased edges.
    pub fn snapshot_loose(&mut self, name: &str) {
        self.snapshot_with_tolerance(
            name,
            LOOSE_PIXEL_COLOR_TOLERANCE,
            LOOSE_PIXEL_COUNT_TOLERANCE,
        );
    }

    pub fn snapshot_with_threshold(&mut self, name: &str, threshold: f32) {
        self.snapshot_with_tolerance(name, threshold, 0);
    }

    pub fn snapshot_with_tolerance(
        &mut self,
        name: &str,
        threshold: f32,
        max_failed_pixels: usize,
    ) {
        if on_windows_ci() {
            return;
        }
        self.inner.snapshot_options(
            snap_name(name),
            &SnapshotOptions::new()
                .threshold(threshold)
                .max_failed_pixels(max_failed_pixels),
        );
    }
}

/// Whether anything inside `rect` was painted differently between two rendered
/// frames.
///
/// `rect` is in points, as a widget reports it, and is scaled by
/// `pixels_per_point` to address the frames.
pub fn pixels_differ(
    before: &image::RgbaImage,
    after: &image::RgbaImage,
    rect: egui::Rect,
    pixels_per_point: f32,
) -> bool {
    #[expect(
        clippy::cast_sign_loss,
        clippy::cast_possible_truncation,
        reason = "a widget rect is inside a canvas of a few hundred points"
    )]
    let bound = |value: f32| value.max(0.0) as u32;
    let pixels = rect * pixels_per_point;
    let columns = bound(pixels.left())..bound(pixels.right()).min(before.width());
    let rows = bound(pixels.top())..bound(pixels.bottom()).min(before.height());
    rows.flat_map(|y| columns.clone().map(move |x| (x, y)))
        .any(|(x, y)| before.get_pixel(x, y) != after.get_pixel(x, y))
}

pub struct TestHarnessBuilder<'a> {
    size: Option<egui::Vec2>,
    fading_enabled: bool,
    dark_mode: Option<bool>,
    step_dt: Option<f32>,
    render_state_hook: Option<RenderStateHook>,
    _marker: std::marker::PhantomData<&'a ()>,
}

/// Extra renderer setup run after the harness is built, with the context and
/// the wgpu render state - e.g. installing gt-map's GPU icon pipeline the
/// way the app does at startup.
pub type RenderStateHook = fn(&egui::Context, &egui_wgpu::RenderState);

impl Default for TestHarnessBuilder<'_> {
    fn default() -> Self {
        Self {
            size: None,
            fading_enabled: false,
            dark_mode: None,
            step_dt: None,
            render_state_hook: None,
            _marker: std::marker::PhantomData,
        }
    }
}

impl<'a> TestHarnessBuilder<'a> {
    pub fn size(mut self, size: egui::Vec2) -> Self {
        self.size = Some(size);
        self
    }

    /// Run `hook` with the harness context and wgpu render state after the
    /// harness is built. Applies to `ui`/`ui_state` harnesses only: `eframe`
    /// harnesses configure their renderer through `CreationContext` like the
    /// app.
    pub fn render_state_hook(mut self, hook: RenderStateHook) -> Self {
        self.render_state_hook = Some(hook);
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
    /// uses egui's default (dark) visuals. `true` selects dark, `false` light.
    pub fn theme(mut self, dark_mode: bool) -> Self {
        self.dark_mode = Some(dark_mode);
        self
    }

    /// Apply the `dark_mode` theme to a freshly built harness and re-run so the
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
        let render_state = egui_kittest::wgpu::create_render_state(
            egui_kittest::wgpu::default_wgpu_setup(),
            egui_wgpu::RendererOptions::PREDICTABLE,
        );
        let renderer =
            egui_kittest::wgpu::WgpuTestRenderer::from_render_state(render_state.clone());
        let mut builder = Harness::builder()
            .with_options(snapshot_options())
            .renderer(renderer);
        if let Some(sz) = self.size {
            builder = builder.with_size(sz);
        }
        if let Some(dt) = self.step_dt {
            builder = builder.with_step_dt(dt);
        }
        let mut inner = builder.build_ui(f);
        gt_ui_theme::install_app_style(&inner.ctx);
        if let Some(hook) = self.render_state_hook {
            hook(&inner.ctx, &render_state);
        }
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
        let render_state = egui_kittest::wgpu::create_render_state(
            egui_kittest::wgpu::default_wgpu_setup(),
            egui_wgpu::RendererOptions::PREDICTABLE,
        );
        let renderer =
            egui_kittest::wgpu::WgpuTestRenderer::from_render_state(render_state.clone());
        let mut builder = Harness::builder()
            .with_options(snapshot_options())
            .renderer(renderer);
        if let Some(sz) = self.size {
            builder = builder.with_size(sz);
        }
        if let Some(dt) = self.step_dt {
            builder = builder.with_step_dt(dt);
        }
        let mut inner = builder.build_ui_state(f, state);
        gt_ui_theme::install_app_style(&inner.ctx);
        if let Some(hook) = self.render_state_hook {
            hook(&inner.ctx, &render_state);
        }
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
