pub(crate) use gt_test_utils::TestHarness;
use gt_test_utils::TestHarnessBuilder;

/// Install the GPU icon pipeline into a test renderer the way the app does
/// at startup, so snapshots exercise the instanced path. Dithering off,
/// matching kittest's PREDICTABLE renderer options.
fn install_gpu_icons(ctx: &egui::Context, render_state: &egui_wgpu::RenderState) {
    if let Ok(library) = crate::icon_mesh::IconMeshLibrary::embedded() {
        crate::icon_mesh::gpu::install(ctx, render_state, &library, false);
    }
}

/// The harness builder for gt-map snapshot tests: [TestHarness::builder]
/// plus the GPU icon pipeline.
pub(crate) fn builder<'a>() -> TestHarnessBuilder<'a> {
    TestHarness::builder().render_state_hook(install_gpu_icons)
}
